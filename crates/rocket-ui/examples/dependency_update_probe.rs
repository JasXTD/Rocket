//! Explicit public-data acceptance probe. No repository inventory is accepted.
use rocket_git::{CancellationToken, osv_file_digest};
use rocket_ui::dependency_security::scan;
use std::{fs, path::PathBuf};

fn main() {
    let mut args = std::env::args_os().skip(1);
    let executable = PathBuf::from(args.next().expect("approved OSV executable"));
    let output = PathBuf::from(args.next().expect("disposable evidence directory"));
    assert!(
        output.is_absolute() && !output.exists(),
        "use a new absolute evidence directory"
    );
    fs::create_dir_all(&output).unwrap();
    let cache = output.join("data");
    let work = output.join("work");
    let archive = cache.join("osv-scalibr/crates.io/all.zip");
    fs::create_dir_all(archive.parent().unwrap()).unwrap();
    fs::write(&archive, b"previous cache sentinel").unwrap();
    let cancel = CancellationToken::new();
    cancel.cancel();
    assert!(scan::update(&executable, &cache, &work, &["crates.io".into()], &cancel).is_err());
    assert_eq!(fs::read(&archive).unwrap(), b"previous cache sentinel");
    assert!(
        scan::update(
            &std::env::current_exe().unwrap(),
            &cache,
            &work,
            &["crates.io".into()],
            &CancellationToken::new()
        )
        .is_err()
    );
    assert_eq!(fs::read(&archive).unwrap(), b"previous cache sentinel");
    scan::update(
        &executable,
        &cache,
        &work,
        &["crates.io".into()],
        &CancellationToken::new(),
    )
    .unwrap();
    assert!(fs::metadata(&archive).unwrap().len() > 1000);
    assert_eq!(fs::read_dir(work).unwrap().count(), 0);
    let digest = osv_file_digest(&archive, 512 * 1024 * 1024, &CancellationToken::new()).unwrap();
    fs::write(output.join("result.json"), serde_json::to_vec_pretty(&serde_json::json!({
        "passed":true,"ecosystems":["crates.io"],"bytes":fs::metadata(&archive).unwrap().len(),"sha256":digest,
        "checks":["cancel preserves old cache","wrong executable preserves old cache","actual public update publishes archive","temporary data cleaned"]
    })).unwrap()).unwrap();
    println!("Public advisory update: four checks passed");
}
