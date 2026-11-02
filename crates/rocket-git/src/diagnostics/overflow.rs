//! A bounded disk queue used only when the shared rotation lock is busy.
//! create_new claims one of 256 fixed slots without waiting on another process.
use super::*;
const SLOTS: usize = 256;
const RECORD_BYTES: usize = 16 * 1024;
static NEXT_SLOT: AtomicU64 = AtomicU64::new(0);

pub(super) fn paths(directory: &Path) -> impl Iterator<Item = PathBuf> + '_ {
    (0..SLOTS).map(|i| directory.join("rocket-overflow").join(format!("{i}.jsonl")))
}

pub(super) fn directory(root: &Path, create: bool) -> io::Result<Option<PathBuf>> {
    let path = root.join("rocket-overflow");
    if create {
        let builder = fs::DirBuilder::new();
        #[cfg(unix)]
        let mut builder = builder;
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        match builder.create(&path) {
            Ok(()) => (),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => (),
            Err(e) => return Err(e),
        }
    }
    match fs::symlink_metadata(&path) {
        Ok(meta) if meta.is_dir() && !crate::local_files::redirected(&meta) => Ok(Some(path)),
        Ok(_) => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "diagnostic overflow directory is redirected",
        )),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

pub(super) fn save(root: &Path, record: &[u8]) -> io::Result<()> {
    if record.len() > RECORD_BYTES || !record.ends_with(b"\n") {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "diagnostic overflow record exceeds its bound",
        ));
    }
    let start = NEXT_SLOT.fetch_add(1, Ordering::Relaxed) as usize % SLOTS;
    for retry in 0..2 {
        let Some(path) = directory(root, true)? else {
            continue;
        };
        for offset in 0..SLOTS {
            let slot = path.join(format!("{}.jsonl", (start + offset) % SLOTS));
            match options().create_new(true).open(slot) {
                Ok(mut file) => {
                    // Readers ignore incomplete final lines. A killed writer's
                    // partial record remains visible and is never replayed as valid.
                    file.write_all(record)?;
                    return file.sync_data();
                }
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(e) if e.kind() == io::ErrorKind::NotFound && retry == 0 => break,
                Err(e) => return Err(e),
            }
        }
    }
    Err(io::Error::new(
        io::ErrorKind::WouldBlock,
        "diagnostic overflow capacity exhausted",
    ))
}

pub(super) fn drain(root: &Path, max_bytes: u64) -> io::Result<()> {
    drain_with_sync(root, max_bytes, fs::File::sync_data)
}

fn drain_with_sync(
    root: &Path,
    max_bytes: u64,
    mut sync: impl FnMut(&fs::File) -> io::Result<()>,
) -> io::Result<()> {
    let Some(directory) = directory(root, false)? else {
        return Ok(());
    };
    if fs::read_dir(&directory)?.next().is_none() {
        return Ok(());
    }
    let mut recovered = Vec::new();
    for path in paths(root) {
        let meta = match fs::symlink_metadata(&path) {
            Ok(meta) => meta,
            Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e),
        };
        if !meta.is_file()
            || crate::local_files::redirected(&meta)
            || meta.len() > RECORD_BYTES as u64
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid diagnostic overflow slot",
            ));
        }
        let mut record = Vec::new();
        fs::File::open(&path)?
            .take((RECORD_BYTES + 1) as u64)
            .read_to_end(&mut record)?;
        if record.is_empty() || !record.ends_with(b"\n") {
            continue;
        }
        if record.len() > RECORD_BYTES || record.len() as u64 > max_bytes {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "diagnostic overflow record exceeds its bound",
            ));
        }
        append_record(root, &record, max_bytes)?;
        recovered.push(path);
    }
    if recovered.is_empty() {
        return Ok(());
    }
    // Flush each retained destination once, including files rotated while
    // replaying. Keep every source slot until all flushes succeed. A crash or
    // error can replay a duplicate sequence, but cannot lose its recovery copy.
    for index in 0..FILES {
        match options().create(false).open(super::path(root, index)) {
            Ok(file) => sync(&file)?,
            Err(error) if index > 0 && error.kind() == io::ErrorKind::NotFound => (),
            Err(error) => return Err(error),
        }
    }
    for path in recovered {
        fs::remove_file(path)?;
    }
    // Keep the empty directory. Removing it races a producer between claiming
    // the directory and creating its slot, and can turn contention into loss.
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recovery_flushes_rotated_destinations_before_removing_any_sources() {
        let root = super::super::tests::Directory::new();
        for n in 0..12 {
            save(&root.0, format!("{{\"n\":{n},\"padding\":\"0123456789\"}}\n").as_bytes()).unwrap();
        }
        let mut flushes = 0;
        drain_with_sync(&root.0, 128, |file| {
            // Even the last flush must still have every recovery source.
            assert_eq!(fs::read_dir(root.0.join("rocket-overflow")).unwrap().count(), 12);
            flushes += 1;
            file.sync_data()
        }).unwrap();
        assert!((2..=FILES).contains(&flushes), "flush once per retained destination: {flushes}");
        let records = (0..FILES).filter_map(|i| fs::read_to_string(super::super::path(&root.0, i)).ok()).collect::<String>();
        assert_eq!(records.lines().count(), 12);
        for n in 0..12 {
            assert!(records.contains(&format!("\"n\":{n},")));
        }
        assert_eq!(fs::read_dir(root.0.join("rocket-overflow")).unwrap().count(), 0);
    }

    #[test]
    fn failed_recovery_flush_keeps_sources_for_retry() {
        let root = super::super::tests::Directory::new();
        for n in 0..12 {
            save(&root.0, format!("{{\"n\":{n},\"padding\":\"0123456789\"}}\n").as_bytes()).unwrap();
        }
        let mut flushes = 0;
        let error = drain_with_sync(&root.0, 128, |file| {
            flushes += 1;
            if flushes == 2 {
                Err(io::Error::other("injected durability failure"))
            } else {
                file.sync_data()
            }
        }).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::Other);
        assert_eq!(fs::read_dir(root.0.join("rocket-overflow")).unwrap().count(), 12);
        drain(&root.0, 128).unwrap();
        let records = (0..FILES).filter_map(|i| fs::read_to_string(super::super::path(&root.0, i)).ok()).collect::<String>();
        for n in 0..12 {
            assert!(records.contains(&format!("\"n\":{n},")), "retry lost record {n}");
        }
        assert_eq!(fs::read_dir(root.0.join("rocket-overflow")).unwrap().count(), 0);
    }

    #[test]
    fn producers_can_save_while_recovery_drains_without_losing_records() {
        let root = super::super::tests::Directory::new();
        let finished = std::sync::atomic::AtomicBool::new(false);
        thread::scope(|scope| {
            let consumer = scope.spawn(|| {
                while !finished.load(Ordering::Acquire) {
                    drain(&root.0, MAX_BYTES).unwrap();
                    thread::yield_now();
                }
            });
            let workers: Vec<_> = (0..8)
                .map(|_| {
                    scope.spawn(|| {
                        for _ in 0..16 {
                            save(&root.0, b"{}\n").unwrap();
                        }
                    })
                })
                .collect();
            let results: Vec<_> = workers.into_iter().map(|worker| worker.join()).collect();
            finished.store(true, Ordering::Release);
            consumer.join().unwrap();
            assert!(results.into_iter().all(|result| result.is_ok()));
        });
        drain(&root.0, MAX_BYTES).unwrap();
        assert_eq!(
            fs::read_to_string(super::super::path(&root.0, 0))
                .unwrap()
                .lines()
                .count(),
            128
        );
    }
    #[test]
    fn overflow_is_bounded_and_recovery_retains_complete_records() {
        let root = super::super::tests::Directory::new();
        for _ in 0..SLOTS {
            save(&root.0, b"{}\n").unwrap();
        }
        assert_eq!(
            save(&root.0, b"{}\n").unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
        assert_eq!(paths(&root.0).filter(|p| p.exists()).count(), SLOTS);
        drain(&root.0, MAX_BYTES).unwrap();
        assert_eq!(
            fs::read_to_string(super::super::path(&root.0, 0))
                .unwrap()
                .lines()
                .count(),
            SLOTS
        );
        assert_eq!(
            fs::read_dir(root.0.join("rocket-overflow"))
                .unwrap()
                .count(),
            0
        );
    }
}
