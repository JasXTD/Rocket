use super::*;

fn scratch() -> scan::Scratch {
    scan::Scratch::new(&std::env::temp_dir().join("rocket-osv-setup-tests")).unwrap()
}

#[test]
fn fresh_profile_needs_setup_without_creating_settings_or_data() {
    let root = scratch();
    let directory = root.path.join("new-profile/dependency-security");
    let loaded = load(&directory, None, &CancellationToken::new()).unwrap();
    assert!(!loaded.ready);
    assert!(loaded.program.is_none());
    assert!(loaded.bytes.is_none());
    assert!(loaded.error.is_none());
    assert!(!directory.exists());
}

#[test]
fn selection_round_trips_unicode_and_rejects_unbounded_or_relative_values() {
    let root = scratch();
    let path = root.path.join("tools with spaces/日本語/osv-scanner.exe");
    assert_eq!(decode(Some(&encode(&path).unwrap())).unwrap(), Some(path));
    for data in [
        b"bad-format".as_slice(),
        b"ROCKET-OSV-1\n",
        b"ROCKET-OSV-1\nrelative.exe",
        b"ROCKET-OSV-1\n\xff",
    ] {
        assert!(decode(Some(data)).is_err());
    }
    assert!(encode(&root.path.join("bad\npath.exe")).is_err());
    assert!(encode(&root.path.join("a".repeat(LIMIT as usize))).is_err());
}

#[test]
fn malformed_saved_choice_does_not_fall_back_to_another_scanner() {
    let root = scratch();
    fs::write(root.path.join(FILE), b"bad-format").unwrap();
    let loaded = load(
        &root.path,
        Some(std::env::current_exe().unwrap()),
        &CancellationToken::new(),
    )
    .unwrap();
    assert!(!loaded.ready);
    assert!(loaded.program.is_none());
    assert!(loaded.error.is_some());
    assert_eq!(loaded.bytes.as_deref(), Some(b"bad-format".as_slice()));
    assert_eq!(fs::read(root.path.join(FILE)).unwrap(), b"bad-format");
}

#[test]
fn invalid_or_cancelled_selection_preserves_the_previous_choice_and_data() {
    let root = scratch();
    let previous = encode(&root.path.join("previous-osv.exe")).unwrap();
    fs::write(root.path.join(FILE), &previous).unwrap();
    fs::create_dir(root.path.join("data")).unwrap();
    fs::write(root.path.join("data/keep.zip"), b"existing data").unwrap();
    let wrong = root.path.join("unreviewed.exe");
    fs::write(&wrong, b"not a reviewed executable").unwrap();
    assert!(
        save(
            &root.path,
            Some(&previous),
            &wrong,
            &CancellationToken::new()
        )
        .is_err()
    );
    let cancel = CancellationToken::new();
    cancel.cancel();
    assert!(matches!(
        save(&root.path, Some(&previous), &wrong, &cancel),
        Err(Error::Cancelled)
    ));
    assert_eq!(fs::read(root.path.join(FILE)).unwrap(), previous);
    assert_eq!(
        fs::read(root.path.join("data/keep.zip")).unwrap(),
        b"existing data"
    );
    assert!(!root.path.join("scanner-selection.lock").exists());
}

#[test]
fn concurrent_profile_choices_and_cancelled_publication_do_not_overwrite_settings() {
    let root = scratch();
    let one = encode(&root.path.join("one.exe")).unwrap();
    let two = encode(&root.path.join("two.exe")).unwrap();
    publish(&root.path, None, &one, &CancellationToken::new()).unwrap();
    assert!(publish(&root.path, None, &two, &CancellationToken::new()).is_err());
    assert_eq!(fs::read(root.path.join(FILE)).unwrap(), one);
    assert!(!root.path.join("scanner-selection.lock").exists());
    let cancel = CancellationToken::new();
    cancel.cancel();
    assert!(matches!(
        publish(&root.path, Some(&one), &two, &cancel),
        Err(Error::Cancelled)
    ));
    assert_eq!(fs::read(root.path.join(FILE)).unwrap(), one);
    publish(&root.path, Some(&one), &two, &CancellationToken::new()).unwrap();
    assert_eq!(fs::read(root.path.join(FILE)).unwrap(), two);
}

#[test]
fn oversized_and_non_file_settings_fail_without_fallback() {
    let root = scratch();
    fs::File::create(root.path.join(FILE))
        .unwrap()
        .set_len(LIMIT + 1)
        .unwrap();
    assert!(load(&root.path, None, &CancellationToken::new()).is_err());
    fs::remove_file(root.path.join(FILE)).unwrap();
    fs::create_dir(root.path.join(FILE)).unwrap();
    assert!(load(&root.path, None, &CancellationToken::new()).is_err());
}

#[test]
#[ignore = "requires explicitly approved local OSV binary via ROCKET_OSV_TEST_EXE"]
fn real_scanner_selection_persists_per_profile_and_revalidates_changed_bytes() {
    let approved =
        PathBuf::from(std::env::var_os("ROCKET_OSV_TEST_EXE").expect("reviewed local OSV"));
    let root = scratch();
    let selected = root.path.join("工具 with spaces.exe");
    fs::copy(&approved, &selected).unwrap();
    let profile_a = root.path.join("profile-a");
    let profile_b = root.path.join("profile-b");
    let first = save(&profile_a, None, &selected, &CancellationToken::new()).unwrap();
    assert!(first.ready);
    let restarted = load(&profile_a, None, &CancellationToken::new()).unwrap();
    assert!(restarted.ready);
    assert_eq!(restarted.program, Some(selected.clone()));
    assert_eq!(restarted.bytes, first.bytes);
    assert!(
        !load(&profile_b, None, &CancellationToken::new())
            .unwrap()
            .ready
    );
    assert!(!profile_b.exists());
    // A changed selected file must fail, even if a valid automatic copy exists.
    let mut file = fs::OpenOptions::new().write(true).open(&selected).unwrap();
    file.write_all(b"NOT-OSV").unwrap();
    drop(file);
    let changed = load(&profile_a, Some(approved), &CancellationToken::new()).unwrap();
    assert!(!changed.ready);
    assert_eq!(changed.program, Some(selected));
    assert!(changed.error.is_some());
    assert_eq!(changed.bytes, first.bytes);
}

#[test]
fn rejected_and_cancelled_advisory_downloads_preserve_existing_data() {
    let root = scratch();
    let cache = root.path.join("data");
    let work = root.path.join("work");
    let database = cache.join("osv-scalibr/crates.io/all.zip");
    fs::create_dir_all(database.parent().unwrap()).unwrap();
    fs::write(&database, b"previous data").unwrap();
    let wrong = root.path.join("unreviewed.exe");
    fs::write(&wrong, b"not OSV").unwrap();
    assert!(
        scan::update(
            &wrong,
            &cache,
            &work,
            &["crates.io".into()],
            &CancellationToken::new()
        )
        .is_err()
    );
    let cancel = CancellationToken::new();
    cancel.cancel();
    assert!(matches!(
        scan::update(&wrong, &cache, &work, &["crates.io".into()], &cancel),
        Err(Error::Cancelled)
    ));
    assert_eq!(fs::read(&database).unwrap(), b"previous data");
    assert_eq!(fs::read_dir(&work).unwrap().count(), 0);
}
