mod common;
use common::{Fixture, isolated};
use rocket_git::{CancellationToken, Error};
use std::fs;

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
