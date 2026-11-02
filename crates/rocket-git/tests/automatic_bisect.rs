#[allow(dead_code)]
mod common;
use common::{Fixture, isolated};
use rocket_git::*;
use std::{
    fs,
    path::PathBuf,
    thread,
    time::{Duration, Instant},
};

fn token() -> CancellationToken {
    CancellationToken::new()
}
fn fixture() -> (Fixture, Vec<ObjectId>) {
    let f = Fixture::new();
    f.write(".gitignore", "ignored/\n");
    let ids = (0..7)
        .map(|n| {
            f.write("版本 文.txt", format!("{n}\n"));
            f.commit("PRIVATE_AUTO");
            f.repo().resolve_commit("HEAD", &token()).unwrap()
        })
        .collect();
    (f, ids)
}
fn command(args: &[&str], seconds: u64) -> BisectTestCommand {
    BisectTestCommand {
        program: PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tools/helpers/bin/auto-bisect/test-runner.exe"),
        arguments: args.iter().map(|s| s.to_string()).collect(),
        timeout_seconds: seconds,
    }
}
fn run(f: &Fixture, ids: &[ObjectId], cmd: BisectTestCommand) -> BisectAutoReport {
    let repo = f.repo();
    let p = repo
        .preview_bisect_auto(ids[0].as_str(), ids[6].as_str(), &cmd, &token())
        .unwrap();
    repo.run_bisect_auto(&p, &f.root.join("output"), None, &token())
        .unwrap()
}
fn restore(f: &Fixture, original: &ObjectId) {
    let repo = f.repo();
    let p = repo
        .preview_bisect(BisectAction::Restore, "", "", &token())
        .unwrap();
    let r = repo.apply_bisect(&p, &token()).unwrap();
    r.result.unwrap();
    assert!(r.session.unwrap().is_none());
    assert_eq!(repo.resolve_commit("HEAD", &token()).unwrap(), *original);
    assert_eq!(f.run(&["symbolic-ref", "--short", "HEAD"]), b"main\n");
}
fn wait(path: &std::path::Path) {
    let start = Instant::now();
    while !path.exists() {
        assert!(start.elapsed() < Duration::from_secs(60));
        thread::sleep(Duration::from_millis(10));
    }
}
#[cfg(windows)]
fn stopped(path: &std::path::Path) {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut std::ffi::c_void;
        fn WaitForSingleObject(handle: *mut std::ffi::c_void, millis: u32) -> u32;
        fn CloseHandle(handle: *mut std::ffi::c_void) -> i32;
        fn GetLastError() -> u32;
    }
    let pid = fs::read_to_string(path).unwrap().parse::<u32>().unwrap();
    unsafe {
        let h = OpenProcess(0x00100000, 0, pid);
        if !h.is_null() {
            assert_eq!(WaitForSingleObject(h, 0), 0, "process {pid} still running");
            CloseHandle(h);
        } else {
            assert_eq!(GetLastError(), 87);
        }
    }
}
#[cfg(not(windows))]
fn stopped(_path: &std::path::Path) {}

#[test]
fn finds_culprit_preserves_local_files_and_reopens_results() {
    isolated(
        "ROCKET_AUTO_TEST",
        "finds_culprit_preserves_local_files_and_reopens_results",
        || {
            let (f, ids) = fixture();
            f.write("untracked 文", "PRIVATE_UNTRACKED");
            f.write("ignored/file", "PRIVATE_IGNORED");
            let original_index = f.run(&["ls-files", "--stage"]);
            let r = run(&f, &ids, command(&["version"], 20));
            r.result.unwrap();
            assert_eq!(r.session.unwrap().unwrap().target, Some(ids[4].clone()));
            assert!(r.tests.iter().all(|t| t.recorded && !t.stdout.is_empty()));
            let recovered = f.repo().bisect_test_history(&token()).unwrap();
            assert_eq!(recovered.len(), r.tests.len());
            assert!(recovered.iter().all(|t| t.recorded && t.stdout.is_empty()));
            restore(&f, &ids[6]);
            assert_eq!(f.run(&["ls-files", "--stage"]), original_index);
            assert_eq!(
                fs::read(f.repository.join("untracked 文")).unwrap(),
                b"PRIVATE_UNTRACKED"
            );
            assert_eq!(
                fs::read(f.repository.join("ignored/file")).unwrap(),
                b"PRIVATE_IGNORED"
            );
            assert_eq!(fs::read_dir(f.root.join("output")).unwrap().count(), 0);
        },
    );
}
#[test]
fn skips_and_bounds_output_without_claiming_a_culprit() {
    isolated(
        "ROCKET_AUTO_TEST",
        "skips_and_bounds_output_without_claiming_a_culprit",
        || {
            for mode in ["skip", "output", "generated"] {
                let (f, ids) = fixture();
                let r = run(&f, &ids, command(&[mode], 20));
                r.result.unwrap();
                assert_eq!(r.session.unwrap().unwrap().phase, BisectPhase::Inconclusive);
                assert!(
                    r.tests
                        .iter()
                        .all(|t| t.outcome == BisectTestOutcome::Skip && t.recorded)
                );
                if mode == "output" {
                    assert!(
                        r.tests
                            .iter()
                            .all(|t| t.stdout.len() == 8192 && t.truncated)
                    );
                }
                restore(&f, &ids[6]);
                if mode == "generated" {
                    assert_eq!(
                        fs::read(f.repository.join("generated 文.txt")).unwrap(),
                        b"PRIVATE_GENERATED"
                    );
                }
            }
        },
    );
}
#[test]
fn command_errors_timeout_and_output_flood_are_unclassified() {
    isolated(
        "ROCKET_AUTO_TEST",
        "command_errors_timeout_and_output_flood_are_unclassified",
        || {
            for args in [
                vec!["exit", "126"],
                vec!["exit", "127"],
                vec!["exit", "130"],
                vec!["flood"],
            ] {
                let (f, ids) = fixture();
                let r = run(&f, &ids, command(&args, 20));
                assert!(r.result.is_err());
                assert_eq!(r.tests.len(), 1);
                assert!(!r.tests[0].recorded);
                assert_eq!(r.tests[0].outcome, BisectTestOutcome::Failed);
                restore(&f, &ids[6]);
            }
            let (f, ids) = fixture();
            let marker = f.root.join("child.pid");
            let r = run(&f, &ids, command(&["hold", marker.to_str().unwrap()], 1));
            assert!(matches!(r.result, Err(Error::TimedOut)));
            assert_eq!(r.tests[0].outcome, BisectTestOutcome::TimedOut);
            assert!(!r.tests[0].recorded);
            stopped(&marker);
            stopped(&marker.with_extension("pid.parent"));
            restore(&f, &ids[6]);
        },
    );
}
#[test]
fn cancelling_and_success_both_stop_owned_descendants() {
    isolated(
        "ROCKET_AUTO_TEST",
        "cancelling_and_success_both_stop_owned_descendants",
        || {
            let (f, ids) = fixture();
            let marker = f.root.join("child.pid");
            let repo = f.repo();
            let c = token();
            let p = repo
                .preview_bisect_auto(
                    ids[0].as_str(),
                    ids[6].as_str(),
                    &command(&["hold", marker.to_str().unwrap()], 30),
                    &c,
                )
                .unwrap();
            thread::scope(|s| {
                s.spawn(|| {
                    wait(&marker.with_extension("pid.parent"));
                    c.cancel();
                });
                let r = repo
                    .run_bisect_auto(&p, &f.root.join("output"), None, &c)
                    .unwrap();
                assert!(matches!(r.result, Err(Error::Cancelled)));
                assert_eq!(r.tests[0].outcome, BisectTestOutcome::Cancelled);
                assert!(!r.tests[0].recorded);
            });
            stopped(&marker);
            stopped(&marker.with_extension("pid.parent"));
            restore(&f, &ids[6]);
            fs::remove_file(&marker).unwrap();
            fs::remove_file(marker.with_extension("pid.parent")).unwrap();
            let r = run(&f, &ids, command(&["orphan", marker.to_str().unwrap()], 20));
            r.result.unwrap();
            stopped(&marker);
            restore(&f, &ids[6]);
        },
    );
}
#[test]
fn external_edits_and_stale_reviews_preserve_data_and_do_not_classify() {
    isolated(
        "ROCKET_AUTO_TEST",
        "external_edits_and_stale_reviews_preserve_data_and_do_not_classify",
        || {
            let (f, ids) = fixture();
            let repo = f.repo();
            let p = repo
                .preview_bisect_auto(
                    ids[0].as_str(),
                    ids[6].as_str(),
                    &command(&["version"], 20),
                    &token(),
                )
                .unwrap();
            f.write("版本 文.txt", "PRIVATE_EDIT");
            let r = repo
                .run_bisect_auto(&p, &f.root.join("output"), None, &token())
                .unwrap();
            assert!(r.result.is_err());
            assert!(r.tests.is_empty());
            assert!(!f.repository.join(".git/rocket-bisect").exists());
            f.run(&["restore", "--", "版本 文.txt"]);
            let r = run(&f, &ids, command(&["dirty"], 20));
            assert!(r.result.is_err());
            assert_eq!(r.tests[0].outcome, BisectTestOutcome::StateChanged);
            assert!(!r.tests[0].recorded);
            assert_eq!(
                fs::read(f.repository.join("版本 文.txt")).unwrap(),
                b"PRIVATE_EXTERNAL_EDIT"
            );
            assert!(
                repo.preview_bisect(BisectAction::Restore, "", "", &token())
                    .is_err()
            );
            f.run(&["restore", "--", "版本 文.txt"]);
            restore(&f, &ids[6]);
        },
    );
}
#[test]
fn changed_executable_and_history_are_refused_but_manual_recovery_remains() {
    isolated(
        "ROCKET_AUTO_TEST",
        "changed_executable_and_history_are_refused_but_manual_recovery_remains",
        || {
            let (f, ids) = fixture();
            let mut cmd = command(&["version"], 20);
            let exe = f.root.join("test.exe");
            fs::copy(&cmd.program, &exe).unwrap();
            cmd.program = exe.clone();
            let repo = f.repo();
            let p = repo
                .preview_bisect_auto(ids[0].as_str(), ids[6].as_str(), &cmd, &token())
                .unwrap();
            fs::write(&exe, b"changed").unwrap();
            assert!(
                repo.run_bisect_auto(&p, &f.root.join("output"), None, &token())
                    .unwrap()
                    .result
                    .is_err()
            );
            assert!(!f.repository.join(".git/rocket-bisect").exists());
            let r = run(&f, &ids, command(&["exit", "127"], 20));
            assert!(r.result.is_err());
            let log = fs::read_dir(f.repository.join(".git"))
                .unwrap()
                .filter_map(|x| x.ok())
                .find(|x| {
                    x.file_name()
                        .to_string_lossy()
                        .starts_with("rocket-bisect-tests-")
                })
                .unwrap()
                .path();
            fs::write(&log, b"unfamiliar result data").unwrap();
            assert!(repo.bisect_test_history(&token()).is_err());
            assert!(
                repo.preview_bisect_auto("", "", &command(&["version"], 20), &token())
                    .is_err()
            );
            restore(&f, &ids[6]);
            assert_eq!(fs::read(log).unwrap(), b"unfamiliar result data");
        },
    );
}
