use super::*;
fn line() -> String {
    format!(
        "ROCKET-CRASH-1\t{}\tdesktop\trust-panic\t1793750000000\t7\t8\tapp.rs\t91\t4\t0\t0:0\t1:123,2:456\twindows\tx86_64\n",
        env!("CARGO_PKG_VERSION")
    )
}
fn scratch() -> PathBuf {
    let p = std::env::temp_dir().join(format!(
        "rocket-crash-unit-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir(&p).unwrap();
    p
}
#[test]
fn projected_report_rejects_payload_injection_and_preserves_numeric_location() {
    let text = line();
    let r = parse(text.trim_end(), "crash-1-2-3-1".into()).unwrap();
    assert!(r.json().contains("\"line\":91"));
    assert!(r.json().contains("\"offset\":123"));
    for bad in [
        text.replace("app.rs", "C:\\PRIVATE\\app.rs"),
        text.replace("app.rs", "x\".rs"),
        text.replace("1:123", "7:123"),
        text.replace("1:123", "0:123"),
        text.replace("1:123", "1:4294967296"),
        text.replace("rust-panic", "PRIVATE"),
        format!("{}\tPRIVATE", text.trim_end()),
    ] {
        assert!(parse(bad.trim_end(), "safe".into()).is_err());
    }
    assert_eq!(source_file("C:\\PRIVATE\\source\\app.rs"), "app.rs");
}
#[test]
fn local_reader_bounds_corruption_and_cancellation() {
    let root = scratch();
    let file = root.join("crash-1-2-3.rcr");
    fs::write(&file, format!("{}broken\npartial", line())).unwrap();
    let s = read(&root, &CancellationToken::new()).unwrap();
    assert_eq!(s.reports.len(), 1);
    assert_eq!(s.damaged, 2);
    fs::write(&file, vec![b'x'; MAX_BYTES as usize + 1]).unwrap();
    let s = read(&root, &CancellationToken::new()).unwrap();
    assert!(s.reports.is_empty());
    assert_eq!(s.damaged, 1);
    let cancel = CancellationToken::new();
    cancel.cancel();
    assert!(matches!(read(&root, &cancel), Err(Error::Cancelled)));
    fs::remove_dir_all(root).unwrap();
}
#[test]
fn report_copy_never_replaces_existing_files_and_cancels_before_publication() {
    let root = scratch();
    let destination = root.join("报告 文.json");
    let report = parse(line().trim_end(), "crash-1-2-3-1".into()).unwrap();
    fs::write(&destination, b"existing user data").unwrap();
    assert!(export(&report, &destination, &CancellationToken::new()).is_err());
    assert_eq!(fs::read(&destination).unwrap(), b"existing user data");
    fs::remove_file(&destination).unwrap();
    let counter = std::cell::Cell::new(0);
    let cancelled = disk::export(report.json().as_bytes(), &destination, || {
        counter.set(counter.get() + 1);
        if counter.get() == 2 {
            Err(Error::Cancelled)
        } else {
            Ok(())
        }
    });
    assert!(matches!(cancelled, Err(Error::Cancelled)));
    assert!(!destination.exists());
    assert_eq!(fs::read_dir(&root).unwrap().count(), 0);
    export(&report, &destination, &CancellationToken::new()).unwrap();
    assert_eq!(fs::read_to_string(&destination).unwrap(), report.json());
    fs::remove_dir_all(root).unwrap();
}
#[test]
fn retention_only_manages_owned_report_names_and_preserves_live_sink() {
    let root = scratch();
    fs::write(root.join("user-notes.md"), b"keep").unwrap();
    for i in 0..MAX_FILES {
        fs::write(root.join(format!("crash-{i:032x}-1-1.rcr")), line()).unwrap();
    }
    let (_, first) = disk::prepare(&root).unwrap();
    assert_eq!(disk::entries(&root).unwrap().len(), MAX_FILES);
    let (_, second) = disk::prepare(&root).unwrap();
    assert_eq!(disk::entries(&root).unwrap().len(), MAX_FILES);
    (&first).write_all(line().as_bytes()).unwrap();
    (&second).write_all(line().as_bytes()).unwrap();
    assert_eq!(fs::read(root.join("user-notes.md")).unwrap(), b"keep");
    drop(first);
    drop(second);
    fs::remove_dir_all(root).unwrap();
}
