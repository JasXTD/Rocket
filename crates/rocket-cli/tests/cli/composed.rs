use super::*;

fn pin(p: &Value) -> &str {
    p["data"]["preview"]["review_sha256"].as_str().unwrap()
}
fn staged(f: &Fixture) {
    fs::write(f.repo.join("資料.txt"), "synthetic staged content\n").unwrap();
    f.g(&["add", "資料.txt"]);
}
const MESSAGE: &str = "PRIVATE_COMPOSED_MESSAGE";
fn preview(f: &Fixture) -> Value {
    f.cli(
        &["commit-push", "origin", "main", "-m", MESSAGE, "--preview"],
        0,
    )
}
fn apply(f: &Fixture, p: &Value, exit: i32) -> Value {
    f.cli(
        &[
            "commit-push",
            "origin",
            "main",
            "-m",
            MESSAGE,
            "--expect-review",
            pin(p),
        ],
        exit,
    )
}
fn private_logs(f: &Fixture) {
    let log = fs::read_to_string(f.root.join("logs/rocket.jsonl")).unwrap();
    for private in [
        MESSAGE,
        "synthetic staged content",
        "PRIVATE_COMPOSED_HOOK",
        "LOCAL_NOT_STAGED",
    ] {
        assert!(!log.contains(private), "log leaked {private}");
    }
    assert!(
        log.lines()
            .map(|l| serde_json::from_str::<Value>(l).unwrap())
            .all(|r| r["dropped_before"] == 0)
    );
}

#[test]
fn composed_commit_push_pins_review_and_keeps_unstaged_files() {
    let f = Fixture::new();
    f.seed();
    let bare = advanced_remote(&f);
    staged(&f);
    let original = f.g(&["rev-parse", "HEAD"]);
    fs::write(f.repo.join("base.rs"), "LOCAL_NOT_STAGED\n").unwrap();
    f.cli(&["commit-push", "origin", "main", "-m", MESSAGE], 2);
    let p = preview(&f);
    assert_eq!(p["data"]["operation_started"], false);
    assert_eq!(f.g(&["rev-parse", "HEAD"]), original);
    f.cli(
        &[
            "commit-push",
            "origin",
            "main",
            "-m",
            MESSAGE,
            "--preview",
            "--expect-review",
            pin(&p),
        ],
        2,
    );
    let wrong = f.cli(
        &[
            "commit-push",
            "origin",
            "main",
            "-m",
            "changed",
            "--expect-review",
            pin(&p),
        ],
        1,
    );
    assert_eq!(wrong["error"]["code"], "workflow_review_stale");
    fs::write(f.repo.join("資料.txt"), "changed staging\n").unwrap();
    f.g(&["add", "資料.txt"]);
    assert_eq!(apply(&f, &p, 1)["error"]["code"], "workflow_review_stale");
    staged(&f);
    f.g(&["config", "rocket.fixture", "changed"]);
    assert_eq!(apply(&f, &p, 1)["error"]["code"], "workflow_review_stale");
    let p = preview(&f);
    f.cli(
        &[
            "--cancel-after-ms",
            "0",
            "commit-push",
            "origin",
            "main",
            "-m",
            MESSAGE,
            "--expect-review",
            pin(&p),
        ],
        130,
    );
    assert_eq!(f.g(&["rev-parse", "HEAD"]), original);
    let r = apply(&f, &p, 0);
    let id = r["data"]["commit"].as_str().unwrap();
    assert_eq!(r["data"]["reviewed_commit_confirmed"], true);
    assert_eq!(
        r["data"]["push"]["data"]["remote_outcome"],
        "confirmed_success"
    );
    assert_eq!(bare_git(&f, &bare, &["rev-parse", "main"]), id);
    assert_eq!(
        fs::read_to_string(f.repo.join("base.rs")).unwrap(),
        "LOCAL_NOT_STAGED\n"
    );
    assert_eq!(f.g(&["diff", "--cached", "--name-only"]), "");
    private_logs(&f);
    let logs = fs::read_to_string(f.root.join("logs/rocket.jsonl")).unwrap();
    assert!(logs.contains("\"operation\":\"commit-push\""));
    assert!(logs.contains("\"failure_code\":\"workflow_review_stale\""));
}

#[test]
fn composed_push_rejection_retains_one_commit_and_reports_remote() {
    let f = Fixture::new();
    f.seed();
    let bare = advanced_remote(&f);
    staged(&f);
    let original = f.g(&["rev-parse", "HEAD"]);
    fs::write(
        bare.join("hooks/pre-receive"),
        "#!/bin/sh\necho PRIVATE_COMPOSED_HOOK >&2\nexit 1\n",
    )
    .unwrap();
    let p = preview(&f);
    let r = apply(&f, &p, 1);
    assert_eq!(r["data"]["reviewed_commit_confirmed"], true);
    assert_eq!(r["data"]["recovery"]["local_commit_retained"], true);
    assert_eq!(r["data"]["commit"], f.g(&["rev-parse", "HEAD"]));
    assert_eq!(r["data"]["push"]["data"]["remote_outcome"], "rejected");
    assert!(!r.to_string().contains("PRIVATE_COMPOSED_HOOK"));
    assert_eq!(bare_git(&f, &bare, &["rev-parse", "main"]), original);
    assert_eq!(f.g(&["rev-list", "--count", "HEAD"]), "2");
    private_logs(&f);
}

#[test]
fn composed_saved_switch_preserves_staging_and_rejects_external_changes() {
    let f = Fixture::new();
    f.seed();
    f.g(&["branch", "topic/界"]);
    fs::write(f.repo.join("base.rs"), "staged\n").unwrap();
    f.g(&["add", "base.rs"]);
    fs::write(f.repo.join("base.rs"), "staged\nunstaged\n").unwrap();
    fs::write(f.repo.join("untracked.txt"), "keep\n").unwrap();
    f.cli(&["branch", "switch-saved", "topic/界"], 2);
    let p = f.cli(&["branch", "switch-saved", "topic/界", "--preview"], 0);
    fs::write(f.repo.join("base.rs"), "external edit\n").unwrap();
    let r = f.cli(
        &[
            "branch",
            "switch-saved",
            "topic/界",
            "--expect-review",
            pin(&p),
        ],
        1,
    );
    assert_eq!(r["error"]["code"], "workflow_review_stale");
    assert_eq!(f.g(&["stash", "list"]), "");
    fs::write(f.repo.join("base.rs"), "staged\nunstaged\n").unwrap();
    let r = f.cli(
        &[
            "branch",
            "switch-saved",
            "topic/界",
            "--expect-review",
            pin(&p),
        ],
        0,
    );
    assert_eq!(f.g(&["branch", "--show-current"]), "topic/界");
    assert_eq!(r["data"]["phase"], "reapplied");
    let stash = r["data"]["recovery"]["stash"].as_str().unwrap();
    assert_eq!(stash, f.g(&["rev-parse", "refs/stash"]));
    assert_eq!(f.g(&["show", ":base.rs"]), "staged");
    assert_eq!(
        fs::read_to_string(f.repo.join("base.rs")).unwrap(),
        "staged\nunstaged\n"
    );
    assert_eq!(
        fs::read_to_string(f.repo.join("untracked.txt")).unwrap(),
        "keep\n"
    );
    assert!(r["data"]["recovery"]["journal"].is_object());
    private_logs(&f);
}

#[test]
fn composed_saved_switch_conflict_keeps_recovery_stash() {
    let f = Fixture::new();
    f.seed();
    f.g(&["switch", "-c", "topic"]);
    f.commit_file("base.rs", "other branch\n", "Other branch");
    f.g(&["switch", "main"]);
    fs::write(f.repo.join("base.rs"), "local edits\n").unwrap();
    let p = f.cli(&["branch", "switch-saved", "topic", "--preview"], 0);
    let r = f.cli(
        &[
            "branch",
            "switch-saved",
            "topic",
            "--expect-review",
            pin(&p),
        ],
        1,
    );
    assert_eq!(r["data"]["phase"], "reapplying");
    assert_eq!(f.g(&["branch", "--show-current"]), "topic");
    let stash = r["data"]["recovery"]["stash"].as_str().unwrap();
    assert_eq!(f.g(&["show", &format!("{stash}:base.rs")]), "local edits");
    assert!(!f.g(&["ls-files", "-u"]).is_empty());
    assert_eq!(r["data"]["status"]["ok"], true);
}

#[test]
fn composed_cancellation_after_commit_keeps_commit_without_publishing() {
    use std::os::windows::process::CommandExt;
    let f = Fixture::new();
    f.seed();
    let bare = advanced_remote(&f);
    staged(&f);
    let original = f.g(&["rev-parse", "HEAD"]);
    fs::create_dir_all(f.repo.join(".git/hooks")).unwrap();
    fs::write(
        f.repo.join(".git/hooks/pre-push"),
        "#!/bin/sh\nprintf ready > ../control-ready\nsleep 20\n",
    )
    .unwrap();
    let p = preview(&f);
    let args = vec![
        "commit-push",
        "origin",
        "main",
        "-m",
        MESSAGE,
        "--expect-review",
        pin(&p),
    ];
    let mut helper = Command::new(std::env::current_exe().unwrap());
    f.env(&mut helper);
    helper
        .args([
            "--ignored",
            "--exact",
            "console_interrupt_bridge",
            "--nocapture",
        ])
        .env("ROCKET_CLI_CONSOLE_ROOT", &f.root)
        .env("ROCKET_CLI_CONSOLE_EVENT", "1")
        .env(
            "ROCKET_CLI_CONSOLE_ARGS",
            serde_json::to_string(&args).unwrap(),
        )
        .creation_flags(0x08000000);
    let out = helper.output().unwrap();
    assert!(
        out.status.success(),
        "{} {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let r: Value =
        serde_json::from_slice(&fs::read(f.root.join("control-result.json")).unwrap()).unwrap();
    assert_eq!(r["error"]["code"], "cancelled");
    assert_eq!(r["data"]["reviewed_commit_confirmed"], true);
    assert_eq!(r["data"]["commit"], f.g(&["rev-parse", "HEAD"]));
    assert_ne!(r["data"]["commit"], original);
    assert_eq!(bare_git(&f, &bare, &["rev-parse", "main"]), original);
    assert_eq!(r["data"]["push"]["data"]["remote_outcome"], "unknown");
    private_logs(&f);
}
