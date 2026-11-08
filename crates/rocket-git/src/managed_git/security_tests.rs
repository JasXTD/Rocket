use super::change_journal::{FULL_READ_EVERY, local_ntfs, names_directory, reference};
use super::*;

fn cached_fixture() -> (PathBuf, Arc<ManagedGitRuntime>) {
    // Tests start together, and two in one clock tick must not share a folder.
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../work/managed-git-security-tests")
        .join(format!(
            "{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
    fs::create_dir_all(directory.join("cmd")).unwrap();
    fs::create_dir_all(directory.join("empty")).unwrap();
    let bytes = b"owned fixture, never executed";
    fs::write(directory.join("cmd/git.exe"), bytes).unwrap();
    let root = directory.canonicalize().unwrap();
    let text = format!(
        "ROCKET-MANAGED-GIT-1\t{MANAGED_GIT_VERSION}\n{}\t{}\tcmd/git.exe\n",
        crate::sha256_bytes(bytes).unwrap(),
        bytes.len()
    );
    let runtime = Arc::new(
        verify_inventory(
            &root,
            &inventory(&text).unwrap(),
            &CancellationToken::new(),
            &ManagedGitProgress::default(),
        )
        .unwrap(),
    );
    leases().lock().unwrap().push(Arc::downgrade(&runtime));
    (root, runtime)
}

fn cached(root: &Path) -> Result<Arc<ManagedGitRuntime>> {
    verify_root(
        root,
        &CancellationToken::new(),
        &ManagedGitProgress::default(),
        true,
        None,
    )
}

#[test]
fn cached_lease_rejects_files_added_after_verification() {
    let (root, runtime) = cached_fixture();
    assert!(Arc::ptr_eq(&cached(&root).unwrap(), &runtime));
    let extra = root.join("cmd/added.dll");
    fs::write(&extra, b"unlisted inert fixture").unwrap();
    assert!(
        matches!(cached(&root), Err(Error::InvalidInput(_))),
        "cached verification accepted an added DLL"
    );
    fs::remove_file(extra).unwrap();
    assert!(Arc::ptr_eq(&cached(&root).unwrap(), &runtime));
}

#[test]
fn cached_lease_rejects_directories_added_after_verification() {
    let (root, _runtime) = cached_fixture();
    let extra = root.join("cmd/new-helpers");
    fs::create_dir(&extra).unwrap();
    assert!(
        matches!(cached(&root), Err(Error::InvalidInput(_))),
        "cached verification accepted an added directory"
    );
    fs::remove_dir(extra).unwrap();
    assert!(cached(&root).is_ok());
}

#[test]
fn existing_runner_clones_refuse_new_runtime_files_before_starting_a_process() {
    use crate::runner::{Runner, args};
    use std::sync::atomic::AtomicBool;
    let (root, runtime) = cached_fixture();
    let runner = Runner {
        executable: runtime.executable(),
        limits: Default::default(),
        log: None,
        transport_scope: None,
        secret_protection: Arc::new(AtomicBool::new(false)),
        guard_ai_api_keys: Arc::new(AtomicBool::new(false)),
        secret_scan_bytes: Arc::new(std::sync::atomic::AtomicUsize::new(crate::DEFAULT_SECRET_SCAN_MIB as usize * 1024 * 1024)),
        push_check: Default::default(),
        git_http_allowed: true,
        managed_git: Some(runtime.clone()),
    };
    let mut logged = runner.clone();
    let log = crate::DiagnosticLog::open(root.with_extension("logs")).unwrap();
    logged.log = Some(log.clone());
    fs::write(root.join("cmd/new-helper.exe"), b"unlisted inert fixture").unwrap();
    for runner in [runner, logged] {
        let started = AtomicBool::new(false);
        let result = runner.tracked_write(
            &root,
            &args(&["--version"]),
            &CancellationToken::new(),
            &started,
        );
        assert!(
            matches!(result, Err(Error::InvalidInput(_))),
            "new runtime file was not rejected before launch"
        );
        assert!(!started.load(Ordering::Acquire));
    }
    let text = fs::read_to_string(log.directory().join("rocket.jsonl")).unwrap();
    let ended: Vec<_> = text.lines().filter(|line| line.contains("\"event\":\"git_end\"")).collect();
    assert_eq!(ended.len(), 1);
    assert!(ended[0].contains("\"verify_us\":"), "failed verification must be timed");
    assert!(!ended[0].contains("\"spawn_us\":"), "verification failure must prevent spawn");
    assert!(!text.contains("new-helper.exe"));
    assert!(!text.contains(&root.to_string_lossy().to_string()));
    let cancel = CancellationToken::new();
    cancel.cancel();
    assert!(matches!(runtime.revalidate(&cancel), Err(Error::Cancelled)));
    assert!(fs::write(root.join("cmd/git.exe"), b"replacement").is_err());
}

fn full_reads(runtime: &ManagedGitRuntime) -> usize {
    runtime.full_reads.load(Ordering::Relaxed)
}

// Records for the fixture's own files can trail it, each costing at most one read.
fn settle(runtime: &ManagedGitRuntime) {
    for _ in 0..8 {
        let before = full_reads(runtime);
        runtime.revalidate(&CancellationToken::new()).unwrap();
        if full_reads(runtime) == before {
            return;
        }
    }
    panic!("records never stopped");
}

// Without a journal on this volume every launch just reads, so these have nothing to show.
fn journal_fixture() -> Option<(PathBuf, Arc<ManagedGitRuntime>)> {
    let fixture = cached_fixture();
    let live = fixture.1.journal.lock().unwrap().is_live();
    if !live {
        eprintln!("no change journal on this volume");
    }
    live.then_some(fixture)
}

fn refused(runtime: &ManagedGitRuntime) -> bool {
    matches!(
        runtime.revalidate(&CancellationToken::new()),
        Err(Error::InvalidInput(_))
    )
}

#[test]
fn launches_read_no_directory_until_the_journal_names_the_runtime() {
    let Some((root, runtime)) = journal_fixture() else {
        return;
    };
    settle(&runtime);
    let settled = full_reads(&runtime);
    runtime.revalidate(&CancellationToken::new()).unwrap();
    assert_eq!(full_reads(&runtime), settled);
    let extra = root.join("cmd/added.dll");
    fs::write(&extra, b"unlisted inert fixture").unwrap();
    for count in 1..=2 {
        assert!(refused(&runtime));
        assert_eq!(full_reads(&runtime), settled + count);
    }
    fs::remove_file(extra).unwrap();
    settle(&runtime);
    let settled = full_reads(&runtime);
    runtime.revalidate(&CancellationToken::new()).unwrap();
    assert_eq!(full_reads(&runtime), settled);
}

#[test]
fn a_change_during_a_read_is_not_cleared_by_it() {
    for another_launch_polls in [false, true] {
        let Some((root, runtime)) = journal_fixture() else {
            return;
        };
        let scratch = root.join("cmd/scratch.tmp");
        fs::write(&scratch, b"removed before the read").unwrap();
        fs::remove_file(&scratch).unwrap();
        let stale = runtime.journal.lock().unwrap().stale().unwrap();
        // The read has passed cmd when this lands, so it finishes clean.
        fs::write(root.join("cmd/added.dll"), b"unlisted inert fixture").unwrap();
        if another_launch_polls {
            runtime.journal.lock().unwrap().stale();
        }
        runtime.journal.lock().unwrap().read(stale);
        assert!(refused(&runtime));
    }
}

#[test]
fn an_earlier_read_cannot_answer_for_records_that_were_skipped() {
    let Some((root, runtime)) = journal_fixture() else {
        return;
    };
    let scratch = root.join("cmd/scratch.tmp");
    fs::write(&scratch, b"removed before the read").unwrap();
    fs::remove_file(&scratch).unwrap();
    let stale = runtime.journal.lock().unwrap().stale().unwrap();
    // This lands behind that read, and then the journal moves past its record.
    fs::write(root.join("cmd/added.dll"), b"unlisted inert fixture").unwrap();
    runtime.journal.lock().unwrap().age(FULL_READ_EVERY);
    runtime.journal.lock().unwrap().stale().unwrap();
    runtime.journal.lock().unwrap().read(stale);
    assert!(refused(&runtime));
}

#[test]
fn parallel_launches_never_pass_while_an_addition_stands() {
    let (root, runtime) = cached_fixture();
    // Odd while the file stands. The same odd value on both sides of a check means
    // it stood throughout, so that check must have refused.
    let cycle = AtomicUsize::new(0);
    let checked = AtomicUsize::new(0);
    let finished = std::sync::atomic::AtomicBool::new(false);
    std::thread::scope(|scope| {
        for _ in 0..4 {
            scope.spawn(|| {
                while !finished.load(Ordering::Acquire) {
                    let before = cycle.load(Ordering::Acquire);
                    let refused = refused(&runtime);
                    if before % 2 == 1 && cycle.load(Ordering::Acquire) == before {
                        assert!(refused, "a launch passed beside an addition");
                        checked.fetch_add(1, Ordering::Relaxed);
                    }
                }
            });
        }
        let extra = root.join("cmd/added.dll");
        for _ in 0..100 {
            fs::write(&extra, b"unlisted inert fixture").unwrap();
            cycle.fetch_add(1, Ordering::AcqRel);
            std::thread::sleep(std::time::Duration::from_millis(2));
            cycle.fetch_add(1, Ordering::AcqRel);
            fs::remove_file(&extra).unwrap();
        }
        finished.store(true, Ordering::Release);
    });
    assert!(
        checked.load(Ordering::Relaxed) > 0,
        "no check saw the addition"
    );
    settle(&runtime);
}

#[test]
fn the_directories_are_read_again_on_schedule_whatever_the_journal_says() {
    let Some((_root, runtime)) = journal_fixture() else {
        return;
    };
    settle(&runtime);
    let settled = full_reads(&runtime);
    runtime.journal.lock().unwrap().age(FULL_READ_EVERY);
    runtime.revalidate(&CancellationToken::new()).unwrap();
    assert_eq!(full_reads(&runtime), settled + 1);
    runtime.revalidate(&CancellationToken::new()).unwrap();
    assert_eq!(full_reads(&runtime), settled + 1);
}

#[test]
fn a_lost_position_sends_the_launch_to_the_full_read() {
    let Some((root, runtime)) = journal_fixture() else {
        return;
    };
    settle(&runtime);
    fs::write(root.join("cmd/added.dll"), b"unlisted inert fixture").unwrap();
    runtime.journal.lock().unwrap().lose_position();
    assert!(refused(&runtime));
}

#[test]
fn too_much_to_catch_up_on_sends_the_launch_to_the_full_read() {
    let Some((root, runtime)) = journal_fixture() else {
        return;
    };
    settle(&runtime);
    runtime.journal.lock().unwrap().limit_catch_up(0);
    // Not a runtime directory, so only the amount of activity can force the read.
    let noise = root.with_extension("noise");
    fs::write(&noise, b"elsewhere on the volume").unwrap();
    fs::remove_file(noise).unwrap();
    let settled = full_reads(&runtime);
    runtime.revalidate(&CancellationToken::new()).unwrap();
    assert_eq!(full_reads(&runtime), settled + 1);
}

#[test]
fn without_a_journal_every_launch_reads_the_directories() {
    let (root, runtime) = cached_fixture();
    *runtime.journal.lock().unwrap() = ChangeJournal::default();
    let cancel = CancellationToken::new();
    runtime.revalidate(&cancel).unwrap();
    runtime.revalidate(&cancel).unwrap();
    assert_eq!(full_reads(&runtime), 2);
    fs::write(root.join("cmd/added.dll"), b"unlisted inert fixture").unwrap();
    assert!(refused(&runtime));
}

#[test]
fn only_a_fixed_ntfs_volume_may_skip_the_read() {
    let name = |text: &str| {
        let mut name = [0u16; 16];
        for (slot, unit) in name.iter_mut().zip(text.encode_utf16()) {
            *slot = unit;
        }
        name
    };
    assert!(local_ntfs(3, &name("NTFS")));
    for (drive, system) in [
        (4, "NTFS"),
        (2, "NTFS"),
        (3, "exFAT"),
        (3, "FAT32"),
        (3, "ReFS"),
        (3, ""),
    ] {
        assert!(!local_ntfs(drive, &name(system)), "{drive} {system}");
    }
}

#[test]
fn a_journal_record_that_cannot_be_understood_counts_as_a_change() {
    let record = |length: u32, version: u16, parent: u64| {
        let mut bytes = vec![0u8; 24];
        bytes[0..4].copy_from_slice(&length.to_ne_bytes());
        bytes[4..6].copy_from_slice(&version.to_ne_bytes());
        bytes[16..24].copy_from_slice(&parent.to_ne_bytes());
        bytes
    };
    let directories = BTreeSet::from([7u64]);
    assert!(!names_directory(&[], &directories));
    assert!(!names_directory(&record(24, 2, 5), &directories));
    assert!(names_directory(&record(24, 2, 7), &directories));
    assert!(names_directory(
        &[record(24, 2, 5), record(24, 2, 7)].concat(),
        &directories
    ));
    assert!(names_directory(&record(24, 3, 5), &directories));
    assert!(names_directory(&record(0, 2, 5), &directories));
    assert!(names_directory(&record(25, 2, 5), &directories));
    assert!(names_directory(&record(24, 2, 5)[..23], &directories));
}

// Mount point data: tag, sizes, then the target and an empty display name. The handle
// stays with the caller, because what a listing shows depends on when it closes.
fn make_junction(directory: &Path, target: &Path) -> File {
    use std::os::windows::{ffi::OsStrExt, fs::OpenOptionsExt, io::AsRawHandle};
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn DeviceIoControl(
            handle: *mut std::ffi::c_void,
            code: u32,
            input: *const std::ffi::c_void,
            input_size: u32,
            output: *mut std::ffi::c_void,
            output_size: u32,
            returned: *mut u32,
            overlapped: *mut std::ffi::c_void,
        ) -> i32;
    }
    let target = target.to_str().unwrap().trim_start_matches(r"\\?\");
    let path: Vec<u16> = std::ffi::OsStr::new(r"\??\")
        .encode_wide()
        .chain(std::ffi::OsStr::new(target).encode_wide())
        .collect();
    let bytes = path.len() * 2;
    let mut data: Vec<u8> = Vec::new();
    data.extend(0xA000_0003u32.to_ne_bytes());
    data.extend(((bytes + 12) as u16).to_ne_bytes());
    data.extend([0u8; 4]);
    data.extend((bytes as u16).to_ne_bytes());
    data.extend(((bytes + 2) as u16).to_ne_bytes());
    data.extend([0u8; 2]);
    path.iter().for_each(|unit| data.extend(unit.to_ne_bytes()));
    data.extend([0u8; 4]);
    let file = OpenOptions::new()
        .write(true)
        .share_mode(7)
        .custom_flags(0x0220_0000)
        .open(directory)
        .unwrap();
    let mut returned = 0;
    // SAFETY: a live handle, a complete buffer of the stated size and no output buffer.
    let ok = unsafe {
        DeviceIoControl(
            file.as_raw_handle(),
            0x0009_00A4, // FSCTL_SET_REPARSE_POINT
            data.as_ptr().cast(),
            data.len() as u32,
            std::ptr::null_mut(),
            0,
            &mut returned,
            std::ptr::null_mut(),
        )
    };
    assert_ne!(ok, 0, "{}", std::io::Error::last_os_error());
    file
}

#[test]
fn a_junction_is_refused_even_while_the_handle_that_set_it_stays_open() {
    let (root, runtime) = cached_fixture();
    runtime.revalidate(&CancellationToken::new()).unwrap();
    // An empty target lists exactly as the verified directory did.
    let target = root.with_extension("target");
    fs::create_dir_all(&target).unwrap();
    let setter = make_junction(&root.join("empty"), &target);
    assert!(refused(&runtime), "refused only after the handle closed");
    drop(setter);
    assert!(refused(&runtime));
}

/// Opens `directory` by file ID and makes `name` through that handle. Windows sends no
/// directory change report for this, however the watch is set up.
fn add_through_handle_by_id(directory: &Path, name: &str, folder: bool) -> File {
    use std::{
        ffi::c_void,
        os::windows::{
            fs::OpenOptionsExt,
            io::{AsRawHandle, FromRawHandle},
        },
        ptr::{null, null_mut},
    };
    #[repr(C)]
    struct FileIdDescriptor {
        size: u32,
        kind: u32,
        id: [u64; 2],
    }
    #[repr(C)]
    struct UnicodeString {
        length: u16,
        maximum: u16,
        buffer: *const u16,
    }
    #[repr(C)]
    struct ObjectAttributes {
        length: u32,
        root: *mut c_void,
        name: *const UnicodeString,
        attributes: u32,
        security: *mut c_void,
        quality: *mut c_void,
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn OpenFileById(
            hint: *mut c_void,
            id: *const FileIdDescriptor,
            access: u32,
            share: u32,
            security: *mut c_void,
            flags: u32,
        ) -> *mut c_void;
    }
    #[link(name = "ntdll")]
    unsafe extern "system" {
        fn NtCreateFile(
            handle: *mut *mut c_void,
            access: u32,
            attributes: *const ObjectAttributes,
            status: *mut [usize; 2],
            allocation: *const i64,
            file_attributes: u32,
            share: u32,
            disposition: u32,
            options: u32,
            extended: *mut c_void,
            extended_length: u32,
        ) -> i32;
    }
    let by_path = OpenOptions::new()
        .read(true)
        .share_mode(7)
        .custom_flags(0x0200_0000)
        .open(directory)
        .unwrap();
    let id = FileIdDescriptor {
        size: size_of::<FileIdDescriptor>() as u32,
        kind: 0,
        id: [reference(&by_path).unwrap(), 0],
    };
    // SAFETY: a live handle as the volume hint, and a complete descriptor.
    let parent = unsafe {
        OpenFileById(
            by_path.as_raw_handle(),
            &id,
            0x0012_019F,
            7,
            null_mut(),
            0x0200_0000,
        )
    };
    assert_ne!(parent as isize, -1, "{}", std::io::Error::last_os_error());
    // SAFETY: just opened and owned from here on.
    let parent = unsafe { File::from_raw_handle(parent) };
    let units: Vec<u16> = name.encode_utf16().collect();
    let name = UnicodeString {
        length: (units.len() * 2) as u16,
        maximum: (units.len() * 2) as u16,
        buffer: units.as_ptr(),
    };
    let attributes = ObjectAttributes {
        length: size_of::<ObjectAttributes>() as u32,
        root: parent.as_raw_handle(),
        name: &name,
        attributes: 0x40,
        security: null_mut(),
        quality: null_mut(),
    };
    let (mut made, mut status) = (null_mut(), [0usize; 2]);
    // SAFETY: every pointer is to a local that outlives the call.
    let code = unsafe {
        NtCreateFile(
            &mut made,
            0x0012_019F,
            &attributes,
            &mut status,
            null(),
            0x80,
            7,
            2,
            if folder { 0x21 } else { 0x60 },
            null_mut(),
            0,
        )
    };
    assert!(code >= 0, "{code:#x}");
    // SAFETY: just created and owned from here on.
    unsafe { File::from_raw_handle(made) }
}

#[test]
fn changes_through_a_handle_by_id_block_the_next_launch() {
    for folder in [false, true] {
        let (root, runtime) = cached_fixture();
        settle(&runtime);
        let made = add_through_handle_by_id(&root.join("cmd"), "added", folder);
        assert!(refused(&runtime), "refused only after the handle closed");
        drop(made);
        assert!(refused(&runtime));
    }
}

#[test]
fn additions_that_arrive_from_elsewhere_on_the_volume_block_the_next_launch() {
    let (root, runtime) = cached_fixture();
    settle(&runtime);
    let outside = root.with_extension("outside");
    fs::create_dir_all(&outside).unwrap();
    let moved = outside.join("moved.dll");
    fs::write(&moved, b"unlisted inert fixture").unwrap();
    fs::rename(&moved, root.join("cmd/moved.dll")).unwrap();
    assert!(refused(&runtime));
    fs::rename(root.join("cmd/moved.dll"), &moved).unwrap();
    settle(&runtime);
    fs::hard_link(&moved, root.join("cmd/linked.dll")).unwrap();
    assert!(refused(&runtime));
    fs::remove_file(root.join("cmd/linked.dll")).unwrap();
    settle(&runtime);
    let carried = outside.join("carried");
    fs::create_dir_all(&carried).unwrap();
    fs::write(carried.join("inner.dll"), b"unlisted inert fixture").unwrap();
    fs::rename(&carried, root.join("cmd/carried")).unwrap();
    assert!(refused(&runtime));
}

#[test]
#[ignore = "requires the reviewed complete local runtime"]
fn reviewed_runtime_membership_rechecks_remain_cancellable_without_rehashing() {
    let root = PathBuf::from(
        std::env::var_os("ROCKET_TEST_MANAGED_GIT_ROOT").expect("set reviewed runtime path"),
    );
    let cancel = CancellationToken::new();
    let progress = ManagedGitProgress::default();
    let runtime = verify_root(&root, &cancel, &progress, false, None).unwrap();
    let before = progress.snapshot();
    let mut times = Vec::new();
    for _ in 0..10 {
        let started = std::time::Instant::now();
        runtime.revalidate(&cancel).unwrap();
        times.push(started.elapsed());
        assert_eq!(progress.snapshot(), before);
    }
    times.sort();
    println!(
        "{} files, {} directories: membership median {:?}, max {:?}",
        runtime.files.len(),
        runtime.directory_entries.len(),
        times[times.len() / 2],
        times.last().unwrap()
    );
    cancel.cancel();
    assert!(matches!(runtime.revalidate(&cancel), Err(Error::Cancelled)));
}
