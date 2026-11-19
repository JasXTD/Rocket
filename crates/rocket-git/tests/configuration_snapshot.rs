mod common;
use common::{Fixture, isolated};
use rocket_git::{CancellationToken, Error};
use std::fs;

#[test]
fn isolated_settings_are_ready_before_any_repository_setup() {
    const TEST: &str = "isolated_settings_are_ready_before_any_repository_setup";
    const CASE: &str = "ROCKET_ISOLATION_ORDER_CASE";
    const LAUNCH: &str = "ROCKET_ISOLATION_ORDER_LAUNCH";
    if std::env::var_os(LAUNCH).is_none() {
        // The launcher only prepares settings. It must work without Git on
        // PATH and discard GIT_DIR even when it points through a regular file.
        let exe = std::env::current_exe().unwrap();
        let output = std::process::Command::new(&exe)
            .args(["--exact", TEST, "--nocapture"])
            .env(LAUNCH, "1")
            .env_remove(CASE)
            .env("PATH", "")
            .env("GIT_DIR", exe.join("invalid-repository"))
            .output()
            .unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        return;
    }
    isolated(CASE, TEST, || {
        assert!(std::env::var_os("GIT_DIR").is_none());
        assert_eq!(std::env::var("GIT_ALLOW_PROTOCOL").unwrap(), "file");
        let config = std::env::var_os("GIT_CONFIG_GLOBAL").unwrap();
        assert_eq!(Some(config.clone()), std::env::var_os("GIT_CONFIG_SYSTEM"));
        assert!(fs::read(config).unwrap().is_empty());
        let template = std::env::var_os("GIT_TEMPLATE_DIR").unwrap();
        assert_eq!(fs::read_dir(template).unwrap().count(), 0);
    });
}

#[test]
fn effective_configuration_tracks_includes_and_preserves_worktree() {
    isolated(
        "ROCKET_CONFIGURATION_TEST",
        "effective_configuration_tracks_includes_and_preserves_worktree",
        check_effective_configuration,
    );
}
fn check_effective_configuration() {
    let f = Fixture::new();
    f.write("文件.txt", "original");
    f.commit("fixture");
    f.write("文件.txt", "unstaged");
    let include = f.root.join("included.config");
    fs::write(&include, "[remote \"origin\"]\nurl = PRIVATE_before\n").unwrap();
    f.run(&["config", "include.path", include.to_str().unwrap()]);
    let repo = f.repo();
    let token = CancellationToken::new();
    let index = fs::read(f.repository.join(".git/index")).unwrap();
    let config = fs::read(f.repository.join(".git/config")).unwrap();
    let first = repo.configuration_snapshot(&token).unwrap();
    assert!(first == repo.configuration_snapshot(&token).unwrap());
    fs::write(&include, "[remote \"origin\"]\nurl = PRIVATE_after\n").unwrap();
    assert!(first != repo.configuration_snapshot(&token).unwrap());
    assert_eq!(fs::read(f.repository.join(".git/index")).unwrap(), index);
    assert_eq!(fs::read(f.repository.join(".git/config")).unwrap(), config);
    assert_eq!(
        fs::read(f.repository.join("文件.txt")).unwrap(),
        b"unstaged"
    );
    token.cancel();
    assert!(matches!(
        repo.configuration_snapshot(&token),
        Err(Error::Cancelled)
    ));
}
