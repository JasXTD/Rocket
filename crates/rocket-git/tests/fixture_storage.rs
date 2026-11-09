#[cfg(unix)]
mod common;

#[cfg(unix)]
#[test]
fn native_default_fixture_root_accepts_guarded_storage() {
    // Parent selects a clean environment rather than mutating this test process's.
    const CHILD: &str = "ROCKET_NATIVE_FIXTURE_ROOT_CASE";
    if std::env::var_os(CHILD).is_none() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "native_default_fixture_root_accepts_guarded_storage", "--nocapture"])
            .env(CHILD, "1")
            .env_remove("ROCKET_TEST_REPOSITORIES")
            .output()
            .unwrap();
        assert!(output.status.success(), "{}\n{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
        return;
    }
    let f = common::Fixture::new();
    assert!(!f.root.components().any(|c| matches!(c, std::path::Component::ParentDir)));
    let private = f.root.join("private storage");
    rocket_git::local_storage::create_private(&private).unwrap();
    rocket_git::local_storage::check_private(&private).unwrap();
    assert!(f.repo().status(&rocket_git::CancellationToken::new()).is_ok());
}
