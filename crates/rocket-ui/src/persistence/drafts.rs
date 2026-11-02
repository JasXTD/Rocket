//! Stream user drafts independently of the small settings/cache file. The
//! per-message bound matches the commit writer; there is no retained-draft quota.
use super::*;
use std::io::{BufReader, BufWriter};

const MAGIC: &[u8] = b"ROCKET-DRAFTS\0\x01";
const PATH_BYTES: usize = 128 * 1024;

pub(super) fn archive_name(name: &str) -> bool {
    name.strip_prefix("drafts-")
        .and_then(|s| s.strip_suffix(".bin"))
        .and_then(|s| s.split_once('-'))
        .is_some_and(|(pid, stamp)| {
            !pid.is_empty()
                && !stamp.is_empty()
                && pid.bytes().all(|b| b.is_ascii_digit())
                && stamp.bytes().all(|b| b.is_ascii_digit())
        })
        && name.len() <= 96
}

pub(super) fn validate(drafts: &BTreeMap<PathBuf, Arc<str>>) -> io::Result<()> {
    u32::try_from(drafts.len()).map_err(|_| invalid())?;
    for (path, text) in drafts {
        let path = path_bytes(path);
        if path.len() > PATH_BYTES || text.len() > rocket_git::MAX_COMMIT_MESSAGE_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Draft cannot be saved: commit messages must be at most 1 MiB",
            ));
        }
        let mut record = (path.len() as u32).to_le_bytes().to_vec();
        record.extend(path);
        Reader(&record).path()?;
    }
    Ok(())
}

pub(super) fn write(path: &Path, drafts: &BTreeMap<PathBuf, Arc<str>>) -> io::Result<()> {
    let file = OpenOptions::new().write(true).create_new(true).open(path)?;
    let result = (|| {
        let mut file = BufWriter::new(file);
        file.write_all(MAGIC)?;
        file.write_all(&(drafts.len() as u32).to_le_bytes())?;
        for (path, text) in drafts {
            let path = path_bytes(path);
            for value in [path.as_slice(), text.as_bytes()] {
                file.write_all(&(value.len() as u32).to_le_bytes())?;
                file.write_all(value)?;
            }
        }
        file.flush()?;
        file.get_ref().sync_all()
    })();
    if result.is_err() {
        let _ = fs::remove_file(path);
    }
    result
}

pub(super) fn read(path: &Path) -> io::Result<BTreeMap<PathBuf, Arc<str>>> {
    let file = File::open(path)?;
    let length = file.metadata()?.len();
    let mut file = BufReader::new(file);
    let mut magic = [0; MAGIC.len()];
    file.read_exact(&mut magic)?;
    if magic != MAGIC {
        return Err(invalid());
    }
    let number = |file: &mut BufReader<File>| -> io::Result<u32> {
        let mut bytes = [0; 4];
        file.read_exact(&mut bytes)?;
        Ok(u32::from_le_bytes(bytes))
    };
    let count = number(&mut file)?;
    // Reject fabricated counts before allocation; read each bounded record only
    // as it is encountered, without buffering the entire archive a second time.
    if u64::from(count) > length.saturating_sub(MAGIC.len() as u64 + 4) / 8 {
        return Err(invalid());
    }
    let mut drafts = BTreeMap::new();
    for _ in 0..count {
        let size = number(&mut file)? as usize;
        if size > PATH_BYTES {
            return Err(invalid());
        }
        let mut bytes = Vec::with_capacity(4 + size);
        bytes.extend((size as u32).to_le_bytes());
        bytes.resize(4 + size, 0);
        file.read_exact(&mut bytes[4..])?;
        let path = Reader(&bytes).path()?;
        let size = number(&mut file)? as usize;
        if size > rocket_git::MAX_COMMIT_MESSAGE_BYTES {
            return Err(invalid());
        }
        let mut text = vec![0; size];
        file.read_exact(&mut text)?;
        let text = String::from_utf8(text).map_err(|_| invalid())?;
        if drafts.insert(path, text.into()).is_some() {
            return Err(invalid());
        }
    }
    let mut extra = [0];
    if file.read(&mut extra)? != 0 {
        return Err(invalid());
    }
    Ok(drafts)
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    fn directory(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "rocket-drafts-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&dir).unwrap();
        dir
    }
    #[test]
    fn migrates_legacy_and_round_trips_large_many_drafts_without_blocking_settings() {
        let dir = directory("migration");
        let mut state = SavedState::default();
        state
            .drafts
            .insert(dir.join("legacy"), "Keep my original draft 文".into());
        fs::write(dir.join("state.bin"), encode(&state).unwrap()).unwrap();
        let (store, mut loaded) = Store::open(&dir).unwrap();
        assert_eq!(loaded, state);
        for i in 0..70 {
            // Also exceed the old 8 MiB aggregate bound with valid commit text.
            loaded.drafts.insert(
                dir.join(format!("repo-{i}")),
                if i < 9 {
                    "文".repeat(rocket_git::MAX_COMMIT_MESSAGE_BYTES / 3).into()
                } else {
                    "small retained draft".into()
                },
            );
        }
        loaded.preferences.sidebar = false;
        store.save(&loaded).unwrap();
        assert!(fs::metadata(dir.join("state.bin")).unwrap().len() < 4096);
        drop(store);
        let (store, restored) = Store::open(&dir).unwrap();
        assert_eq!(restored, loaded);
        loaded.drafts.remove(&dir.join("repo-69"));
        loaded.preferences.details = false;
        store.save(&loaded).unwrap();
        drop(store);
        let (store, restored) = Store::open(&dir).unwrap();
        assert_eq!(restored, loaded);
        assert_eq!(
            fs::read_dir(&dir)
                .unwrap()
                .filter_map(Result::ok)
                .filter(|e| archive_name(&e.file_name().to_string_lossy()))
                .count(),
            1
        );
        drop(store);
        assert_eq!(dir.parent(), Some(std::env::temp_dir().as_path()));
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn failed_pointer_commit_preserves_both_files_and_retry_keeps_full_draft() {
        use std::os::windows::fs::OpenOptionsExt;
        let dir = directory("atomic");
        let (store, mut state) = Store::open(&dir).unwrap();
        state.drafts.insert(dir.join("repo"), "old draft".into());
        store.save(&state).unwrap();
        let old = fs::read(dir.join("state.bin")).unwrap();
        let old_archive = store.archive.lock().unwrap().clone().unwrap();
        let lock = OpenOptions::new()
            .read(true)
            .share_mode(1)
            .open(dir.join("state.bin"))
            .unwrap();
        state.drafts.insert(dir.join("repo"), "x".repeat(131_073).into());
        assert!(store.save(&state).is_err());
        assert_eq!(fs::read(dir.join("state.bin")).unwrap(), old);
        assert_eq!(
            read(&dir.join(&old_archive)).unwrap()[&dir.join("repo")].as_ref(),
            "old draft"
        );
        assert_eq!(
            fs::read_dir(&dir)
                .unwrap()
                .filter_map(Result::ok)
                .filter(|e| archive_name(&e.file_name().to_string_lossy()))
                .count(),
            1
        );
        drop(lock);
        store.save(&state).unwrap();
        assert!(!dir.join(old_archive).exists());
        drop(store);
        let (store, loaded) = Store::open(&dir).unwrap();
        assert_eq!(loaded, state);
        let archive = store.archive.lock().unwrap().clone().unwrap();
        drop(store);
        // A damaged external draft archive must never be interpreted as empty.
        fs::write(dir.join(archive), MAGIC).unwrap();
        let pointer = fs::read(dir.join("state.bin")).unwrap();
        assert!(Store::open(&dir).is_err());
        assert_eq!(fs::read(dir.join("state.bin")).unwrap(), pointer);
        assert!(!archive_name("../drafts-1-2.bin"));
        assert!(!archive_name("drafts-1-2.bin/other"));
        assert_eq!(dir.parent(), Some(std::env::temp_dir().as_path()));
        fs::remove_dir_all(dir).unwrap();
    }
}
