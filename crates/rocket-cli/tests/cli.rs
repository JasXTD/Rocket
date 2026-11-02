#![cfg(windows)]
use serde_json::Value;
use std::{
    ffi::OsStr,
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

static NEXT: AtomicU64 = AtomicU64::new(0);

#[test]
fn advanced_tags_and_worktrees_preserve_files_and_return_recovery() {
    let f=Fixture::new(); f.seed();
    let head=f.g(&["rev-parse","HEAD"]);
    fs::write(f.repo.join("untracked 私.txt"),"PRIVATE_ADVANCED_PAYLOAD").unwrap();
    let before=f.g(&["status","--porcelain=v2"]);
    let p=f.cli(&["tag","create","版本/一","HEAD","--message","PRIVATE_TAG_MESSAGE","--preview"],0);
    assert_eq!(p["data"]["operation_started"],false);
    assert_eq!(f.g(&["tag","--list"]),"");
    f.cli(&["tag","create","版本/一","HEAD","--message","PRIVATE_TAG_MESSAGE"],0);
    let detail=f.cli(&["tag","inspect","版本/一"],0);
    let tag_id=detail["data"]["entry"]["id"].as_str().unwrap();
    assert_ne!(tag_id,head); assert_eq!(detail["data"]["target"],head);
    f.cli(&["tag","inspect","版本"],1); // Never mistake a prefix for its child.
    assert_eq!(f.cli(&["tag","delete","版本/一","--expect",&head],1)["error"]["code"],"tag_stale_preview");
    f.cli(&["--cancel-after-ms","0","tag","delete","版本/一","--expect",tag_id],130);
    assert_eq!(f.g(&["rev-parse","refs/tags/版本/一"]),tag_id);
    let deleted=f.cli(&["tag","delete","版本/一","--expect",tag_id],0);
    let recovery=deleted["data"]["recovery_reference"].as_str().unwrap();
    assert_eq!(f.g(&["rev-parse",recovery]),tag_id);
    assert_eq!(f.cli(&["tag","list"],0)["data"]["entries"].as_array().unwrap().len(),0);
    let destination=f.root.join("linked 日本語"); let dest=destination.to_str().unwrap();
    f.cli(&["worktree","create",dest,"--branch","feature/一","--preview"],0); assert!(!destination.exists());
    f.cli(&["worktree","create",dest,"--branch","feature/一"],0);
    assert_eq!(f.cli(&["worktree","list"],0)["data"]["entries"].as_array().unwrap().len(),2);
    let sentinel=destination.join("keep.txt"); fs::write(&sentinel,"keep").unwrap();
    assert_eq!(f.cli(&["worktree","remove",dest,"--expect",&head],1)["error"]["code"],"worktree_dirty");
    assert_eq!(fs::read_to_string(&sentinel).unwrap(),"keep"); fs::remove_file(&sentinel).unwrap();
    f.cli(&["worktree","remove",dest,"--expect",&"f".repeat(40)],1);
    f.cli(&["worktree","remove",dest,"--expect",&head,"--preview"],0); assert!(destination.exists());
    let removed=f.cli(&["worktree","remove",dest,"--expect",&head],0);
    assert!(!destination.exists()); assert!(removed["data"]["recovery_reference"].is_string());
    let existing=f.root.join("existing"); fs::create_dir(&existing).unwrap(); fs::write(existing.join("keep"),"keep").unwrap();
    assert_eq!(f.cli(&["worktree","create",existing.to_str().unwrap(),"--branch","other"],1)["error"]["code"],"worktree_destination_exists");
    assert_eq!(fs::read_to_string(existing.join("keep")).unwrap(),"keep");
    assert_eq!(f.g(&["status","--porcelain=v2"]),before); assert_eq!(f.g(&["rev-parse","HEAD"]),head);
    let logs=fs::read_to_string(f.root.join("logs/rocket.jsonl")).unwrap();
    assert!(!logs.contains("PRIVATE_TAG_MESSAGE") && !logs.contains("PRIVATE_ADVANCED_PAYLOAD"));
    for row in logs.lines().map(|s|serde_json::from_str::<Value>(s).unwrap()) { assert_eq!(row["dropped_before"],0); }
}

fn advanced_remote(f:&Fixture)->PathBuf {
    let bare=f.root.join("remote PRIVATE.git");
    f.git(&f.root,&[OsStr::new("clone"),OsStr::new("--bare"),f.repo.as_os_str(),bare.as_os_str()]);
    f.g(&["remote","add","origin",bare.to_str().unwrap()]); bare
}
fn bare_git(f:&Fixture,bare:&Path,args:&[&str])->String { f.git(bare,&args.iter().map(OsStr::new).collect::<Vec<_>>()) }

#[test]
fn advanced_lease_pins_both_tips_and_preserves_local_changes() {
    let f=Fixture::new(); f.seed();
    let base=f.g(&["rev-parse","HEAD"]);
    f.commit_file("old.rs","old\n","old remote"); let expected=f.g(&["rev-parse","HEAD"]);
    let bare=advanced_remote(&f);
    f.g(&["reset","--hard",&base]); f.commit_file("new.rs","new\n","replacement");
    let source=f.g(&["rev-parse","HEAD"]);
    fs::write(f.repo.join("base.rs"),"local edit\n").unwrap();
    let status=f.g(&["status","--porcelain=v2"]);
    let p=f.cli(&["push-lease","origin","main","main","--preview"],0);
    assert_eq!(p["data"]["preview"]["source"],source); assert_eq!(p["data"]["preview"]["expected"],expected);
    assert_eq!(p["data"]["preview"]["removed_count"],1);
    assert_eq!(f.cli(&["push-lease","origin","main","main","--source",&base,"--expect",&expected],1)["error"]["code"],"force_push_stale_preview");
    bare_git(&f,&bare,&["update-ref","refs/heads/main",&base,&expected]);
    f.cli(&["push-lease","origin","main","main","--source",&source,"--expect",&expected],1);
    assert_eq!(bare_git(&f,&bare,&["rev-parse","HEAD"]),base);
    bare_git(&f,&bare,&["update-ref","refs/heads/main",&expected,&base]);
    let pushed=f.cli(&["push-lease","origin","main","main","--source",&source,"--expect",&expected],0);
    assert_eq!(pushed["data"]["remote_outcome"],"replaced");
    assert_eq!(bare_git(&f,&bare,&["rev-parse","HEAD"]),source);
    assert_eq!(f.g(&["status","--porcelain=v2"]),status);
}

#[test]
fn advanced_atomic_push_is_all_or_nothing_and_rejects_changed_remote() {
    let f=Fixture::new(); f.seed(); let base=f.g(&["rev-parse","HEAD"]); let bare=advanced_remote(&f);
    f.commit_file("advance.rs","next\n","advance"); let source=f.g(&["rev-parse","HEAD"]);
    let a=format!("main:a:{source}:missing"); let b=format!("main:b:{source}:missing");
    let p=f.cli(&["push-atomic","origin","--preview","--","main:a","main:b"],0);
    assert_eq!(p["data"]["preview"]["branches"].as_array().unwrap().len(),2);
    bare_git(&f,&bare,&["config","receive.advertiseAtomic","false"]);
    let unsupported=f.cli(&["push-atomic","origin","--",&a,&b],1);
    assert_eq!(unsupported["error"]["code"],"atomic_push_unsupported");
    assert_eq!(bare_git(&f,&bare,&["for-each-ref","--format=%(refname)","refs/heads/a","refs/heads/b"]),"");
    bare_git(&f,&bare,&["config","--unset","receive.advertiseAtomic"]);
    fs::write(bare.join("hooks/update"),"#!/bin/sh\nif [ \"$1\" = refs/heads/b ]; then echo PRIVATE_ADVANCED_HOOK >&2; exit 1; fi\n").unwrap();
    let rejected=f.cli(&["push-atomic","origin","--",&a,&b],1);
    assert_eq!(rejected["data"]["remote_outcome"],"unchanged");
    assert_eq!(bare_git(&f,&bare,&["for-each-ref","--format=%(refname)","refs/heads/a","refs/heads/b"]),"");
    assert!(!rejected.to_string().contains("PRIVATE_ADVANCED_HOOK"));
    fs::remove_file(bare.join("hooks/update")).unwrap();
    let v=f.cli(&["push-atomic","origin","--",&a,&b],0); assert_eq!(v["data"]["remote_outcome"],"published");
    let a=format!("main:a:{source}:{source}"); let b=format!("main:b:{source}:{source}");
    bare_git(&f,&bare,&["update-ref","refs/heads/a",&base,&source]);
    assert_eq!(f.cli(&["push-atomic","origin","--",&a,&b],1)["error"]["code"],"atomic_push_stale_preview");
    assert_eq!(bare_git(&f,&bare,&["rev-parse","refs/heads/a"]),base);
    assert_eq!(bare_git(&f,&bare,&["rev-parse","refs/heads/b"]),source);
    let logs=fs::read_to_string(f.root.join("logs/rocket.jsonl")).unwrap(); assert!(!logs.contains("PRIVATE_ADVANCED_HOOK"));
}

#[test]
fn advanced_push_cancellation_observes_remote_and_stops_helpers() {
    use std::os::windows::process::CommandExt;
    for atomic in [false,true] {
        let f=Fixture::new(); f.seed(); let base=f.g(&["rev-parse","HEAD"]);
        f.commit_file("old.rs","old\n","remote"); let expected=f.g(&["rev-parse","HEAD"]); let bare=advanced_remote(&f);
        f.g(&["reset","--hard",&base]); f.commit_file("new.rs","new\n","replacement"); let source=f.g(&["rev-parse","HEAD"]);
        fs::create_dir_all(f.repo.join(".git/hooks")).unwrap();
        fs::write(f.repo.join(".git/hooks/pre-push"),"#!/bin/sh\nprintf ready > ../control-ready\nsleep 20\n").unwrap();
        let args=if atomic {vec!["push-atomic".to_owned(),"origin".into(),"--".into(),format!("main:a:{source}:missing"),format!("main:b:{source}:missing")]}
            else {vec!["push-lease".into(),"origin".into(),"main".into(),"main".into(),"--source".into(),source.clone(),"--expect".into(),expected.clone()]};
        let mut helper=Command::new(std::env::current_exe().unwrap()); f.env(&mut helper);
        helper.args(["--ignored","--exact","console_interrupt_bridge","--nocapture"])
            .env("ROCKET_CLI_CONSOLE_ROOT",&f.root).env("ROCKET_CLI_CONSOLE_EVENT","1")
            .env("ROCKET_CLI_CONSOLE_ARGS",serde_json::to_string(&args).unwrap()).creation_flags(0x08000000);
        let output=helper.output().unwrap(); assert!(output.status.success(),"{} {}",String::from_utf8_lossy(&output.stdout),String::from_utf8_lossy(&output.stderr));
        let v:Value=serde_json::from_slice(&fs::read(f.root.join("control-result.json")).unwrap()).unwrap();
        assert_eq!(v["error"]["code"],"cancelled"); assert_eq!(v["data"]["remote_outcome"],"unchanged");
        assert_eq!(v["data"]["observed"]["ok"],true); assert_eq!(bare_git(&f,&bare,&["rev-parse","HEAD"]),expected);
        assert_eq!(f.g(&["rev-parse","HEAD"]),source); assert_eq!(f.g(&["status","--porcelain"]),"");
        let records=fs::read_to_string(f.root.join("logs/rocket.jsonl")).unwrap();
        let rows:Vec<Value>=records.lines().map(|l|serde_json::from_str(l).unwrap()).collect();
        let stopped=rows.iter().find(|r|r["event"]=="git_end"&&r["outcome"]=="cancelled").unwrap();
        assert!(stopped["cancel_to_stop_us"].as_u64().unwrap()<5_000_000);
        assert!(rows.iter().all(|r|r["dropped_before"]==0));
        println!("atomic={atomic} stop_us={}",stopped["cancel_to_stop_us"]);
    }
}
struct Fixture {
    root: PathBuf,
    repo: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let base = std::env::var_os("ROCKET_CLI_TEST_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(|| std::env::temp_dir().join("rocket-cli-tests"));
        let root = base.join(format!(
            "{}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("global.gitconfig"),
            b"[protocol]\n\tallow = never\n[protocol \"file\"]\n\tallow = always\n",
        )
        .unwrap();
        let f = Self {
            repo: root.join("repo 性能"),
            root,
        };
        println!("retained fixture: {}", f.root.display());
        f
    }
    fn env(&self, cmd: &mut Command) {
        // Child-only isolation; no real identity, credentials, hooks or Git config.
        cmd.current_dir(&self.root)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_SYSTEM", self.root.join("absent-system"))
            .env("GIT_CONFIG_GLOBAL", self.root.join("global.gitconfig"))
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GCM_INTERACTIVE", "never")
            .env("GIT_ALLOW_PROTOCOL", "file")
            .env("GIT_CONFIG_COUNT", "0")
            .env("ROCKET_LOG_DIR", self.root.join("logs"))
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .env_remove("GIT_OBJECT_DIRECTORY")
            .env_remove("GIT_ALTERNATE_OBJECT_DIRECTORIES")
            .env_remove("GIT_CONFIG_PARAMETERS")
            .env_remove("ROCKET_LOG");
    }
    fn command(&self, repo: &Path, args: &[&str]) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_rocket-cli"));
        self.env(&mut cmd);
        cmd.arg("--json").arg("--repo").arg(repo).args(args);
        cmd
    }
    fn invoke(&self, repo: &Path, args: &[&str], exit: i32) -> Value {
        let output = self.command(repo, args).output().unwrap();
        self.check(output, args, exit)
    }
    fn check(&self, output: Output, args: &[&str], exit: i32) -> Value {
        assert_eq!(
            output.status.code(),
            Some(exit),
            "args={args:?}\nstdout={}\nstderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let value: Value =
            serde_json::from_slice(&output.stdout).expect("one complete JSON result on stdout");
        assert_eq!(value["schema_version"], 1);
        assert_eq!(value["ok"], exit == 0);
        assert!(value["warnings"].is_array());
        assert_eq!(value["error"].is_null(), exit == 0);
        value
    }
    fn cli(&self, args: &[&str], exit: i32) -> Value {
        self.invoke(&self.repo, args, exit)
    }
    fn git(&self, repo: &Path, args: &[&OsStr]) -> String {
        let mut cmd = Command::new("git");
        self.env(&mut cmd);
        cmd.arg("-C").arg(repo).args(args);
        let out = cmd.output().unwrap();
        assert!(
            out.status.success(),
            "Git fixture setup failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap().trim_end().to_owned()
    }
    fn g(&self, args: &[&str]) -> String {
        self.git(&self.repo, &args.iter().map(OsStr::new).collect::<Vec<_>>())
    }
    fn init(&self) {
        let v = self.cli(&["init", self.repo.to_str().unwrap()], 0);
        assert_eq!(v["data"]["published"], true);
        self.g(&["config", "--local", "user.name", "Rocket CLI Fixture"]);
        self.g(&["config", "--local", "user.email", "cli@example.invalid"]);
        self.g(&["config", "--local", "commit.gpgSign", "false"]);
        self.g(&["config", "--local", "core.autocrlf", "false"]);
    }
    fn seed(&self) {
        self.init();
        fs::write(self.repo.join("base.rs"), "base\n").unwrap();
        self.cli(&["stage", "--all"], 0);
        self.cli(&["commit", "-m", "Base commit"], 0);
    }
    fn commit_file(&self, file: &str, text: &str, message: &str) {
        fs::write(self.repo.join(file), text).unwrap();
        self.cli(&["stage", "--", file], 0);
        self.cli(&["commit", "-m", message], 0);
    }
}

#[test]
fn help_version_and_usage_are_json_without_git_or_repository_side_effects() {
    let f = Fixture::new();
    for args in [vec!["--help"], vec!["--version"], vec!["branch", "--help"]] {
        let mut cmd = f.command(&f.repo, &args);
        cmd.arg("--git").arg(f.root.join("missing.exe"));
        let v = f.check(cmd.output().unwrap(), &args, 0);
        assert!(v["diagnostics"].is_null());
    }
    for args in [
        vec!["stage"],
        vec!["commit", "-m", "x", "--force"],
        vec!["branch", "delete", "main", "--force"],
        vec!["pull", "--rebase", "--merge"],
        vec!["push", "origin", "main", "main", "--force"],
        vec!["history", "--limit", "1001"],
    ] {
        let v = f.cli(&args, 2);
        assert_eq!(v["error"]["code"], "usage");
        assert!(v["diagnostics"].is_null());
    }
    assert!(!f.repo.exists());
    assert!(!f.root.join("logs").exists());
}

#[test]
fn review_history_and_commit_urls_keep_json_errors_cancellation_and_files_safe() {
    let f=Fixture::new();f.seed();f.commit_file("資料.rs","sample\n","Unicode file");
    let head=f.g(&["rev-parse","HEAD"]);let status=f.g(&["status","--porcelain=v2"]);
    for order in ["topo","commit-date","author-date"] {
        let value=f.cli(&["history","--all","--order",order,"--limit","1"],0);
        assert_eq!(value["data"]["order"],order);assert_eq!(value["data"]["commits"][0]["id"],head);
        assert_eq!(value["data"]["has_more"],true);
    }
    f.cli(&["history","--order","timestamp"],2);
    f.cli(&["commit-url","HEAD"],1);
    f.g(&["remote","add","origin","git@github.com:sample/demo.git"]);
    let value=f.cli(&["commit-url","HEAD"],0);
    assert_eq!(value["data"]["url"],format!("https://github.com/sample/demo/commit/{head}"));
    assert_eq!(value["data"]["remote_presence"],"not_checked");
    f.g(&["remote","add","upstream","https://gitlab.com/group/demo.git"]);
    f.cli(&["commit-url","HEAD"],1);
    let value=f.cli(&["commit-url","HEAD","--remote","upstream"],0);
    assert_eq!(value["data"]["url"],format!("https://gitlab.com/group/demo/-/commit/{head}"));
    f.g(&["remote","add","selfhost","https://code.example.test/root/team/%E8%B3%87%E6%96%99.git"]);
    let value=f.cli(&["commit-url","HEAD","--remote","selfhost","--provider","gitea","--server","https://code.example.test/root"],0);
    assert_eq!(value["data"]["url"],format!("https://code.example.test/root/team/%E8%B3%87%E6%96%99/commit/{head}"));
    f.cli(&["commit-url","HEAD","--provider","gitea"],2);
    f.cli(&["review","file","HEAD","--path","../outside"],1);
    f.cli(&["review","commit","not-a-revision"],1);
    f.cli(&["review","file","HEAD"],2);
    let cancelled=f.cli(&["--cancel-after-ms","0","history","--order","author-date"],130);
    assert_eq!(cancelled["error"]["code"],"cancelled");
    assert_eq!(f.g(&["rev-parse","HEAD"]),head);assert_eq!(f.g(&["status","--porcelain=v2"]),status);
    assert_eq!(fs::read_to_string(f.repo.join("資料.rs")).unwrap(),"sample\n");
}

#[test]
fn human_output_escapes_controls_and_broken_pipe_never_retries_commit() {
    let f = Fixture::new();
    f.seed();
    f.commit_file("base.rs", "changed\n", "subject\u{1b}[2J");
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rocket-cli"));
    f.env(&mut cmd);
    cmd.arg("--repo").arg(&f.repo).arg("history");
    let history = cmd.output().unwrap();
    assert!(history.status.success());
    assert!(!history.stdout.contains(&27));
    assert!(String::from_utf8_lossy(&history.stdout).contains("\\u{1b}[2J"));
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_rocket-cli"));
    f.env(&mut cmd);
    cmd.arg("--repo").arg(&f.repo).args(["stage", "--invalid"]);
    let invalid = cmd.output().unwrap();
    assert_eq!(invalid.status.code(), Some(2));
    assert!(invalid.stdout.is_empty());
    assert!(String::from_utf8_lossy(&invalid.stderr).contains("usage:"));
    fs::write(f.repo.join("base.rs"), "pipe commit\n").unwrap();
    f.cli(&["stage", "--all"], 0);
    let count = f
        .g(&["rev-list", "--count", "HEAD"])
        .parse::<u64>()
        .unwrap();
    let mut cmd = f.command(&f.repo, &["commit", "-m", "Commit despite closed output"]);
    cmd.stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let mut child = cmd.spawn().unwrap();
    drop(child.stdout.take());
    let output = child.wait_with_output().unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("Result output could not be written"));
    assert_eq!(
        f.g(&["rev-list", "--count", "HEAD"])
            .parse::<u64>()
            .unwrap(),
        count + 1
    );
    assert_eq!(
        f.g(&["log", "-1", "--format=%s"]),
        "Commit despite closed output"
    );
}

#[test]
fn init_and_unicode_literal_paths_stage_commit_diff_and_external_changes() {
    let f = Fixture::new();
    fs::create_dir(&f.repo).unwrap();
    fs::create_dir(f.repo.join("src")).unwrap();
    let name = "src/性能 🚀 é.rs";
    fs::write(f.repo.join(name), "保留 original\n").unwrap();
    fs::write(f.repo.join("--json"), b"literal\n").unwrap();
    f.init();
    assert_eq!(
        fs::read_to_string(f.repo.join(name)).unwrap(),
        "保留 original\n"
    );
    let s = f.cli(&["status"], 0);
    assert_eq!(s["data"]["head"]["kind"], "unborn");
    assert_eq!(s["data"]["entries"].as_array().unwrap().len(), 2);
    f.cli(&["stage", "--", name], 0);
    assert_eq!(
        f.g(&["diff", "--cached", "--name-only", "-z"]),
        format!("{name}\0")
    );
    f.cli(&["unstage", "--", name], 0);
    assert_eq!(f.g(&["ls-files", "-z"]), "");
    f.cli(&["stage", "--all"], 0);
    f.cli(&["commit", "-m", "Initial Unicode files"], 0);
    let first = f.g(&["rev-parse", "HEAD"]);
    fs::write(f.repo.join(name), "次 external\n").unwrap();
    assert!(
        f.cli(&["diff", "--", name], 0)["data"]["patch"]["text"]
            .as_str()
            .unwrap()
            .contains("+次 external")
    );
    assert_eq!(f.cli(&["diff", "--staged"], 0)["data"]["byte_count"], 0);
    f.cli(&["stage", "--", name], 0);
    assert!(
        f.cli(&["diff", "--staged", "--", name], 0)["data"]["patch"]["text"]
            .as_str()
            .unwrap()
            .contains("+次 external")
    );
    fs::write(f.repo.join(name), "third external\n").unwrap();
    assert!(
        f.cli(&["diff", "--", name], 0)["data"]["patch"]["text"]
            .as_str()
            .unwrap()
            .contains("+third external")
    );
    f.cli(&["unstage", "--all"], 0);
    assert_eq!(f.cli(&["diff", "--staged"], 0)["data"]["byte_count"], 0);
    fs::write(f.repo.join("--json"), "changed literal\n").unwrap();
    f.cli(&["stage", "--", "--json"], 0);
    f.cli(&["commit", "--message", "--json"], 0);
    assert_eq!(f.g(&["log", "-1", "--format=%s"]), "--json");
    f.cli(&["stage", "--", name], 0);
    f.cli(&["commit", "-m", "Second Unicode edit"], 0);
    assert!(
        f.cli(
            &["diff", "--base", &first, "--target", "HEAD", "--", name],
            0
        )["data"]["patch"]["text"]
            .as_str()
            .unwrap()
            .contains("+third external")
    );
    assert!(
        f.cli(&["status"], 0)["data"]["entries"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        f.cli(&["commit", "-m", "Nothing staged"], 1)["error"]["code"],
        "nothing_to_commit"
    );
    let config = fs::read(f.repo.join(".git/config")).unwrap();
    f.cli(&["init", f.repo.to_str().unwrap()], 1);
    assert_eq!(fs::read(f.repo.join(".git/config")).unwrap(), config);
    assert_eq!(
        fs::read_to_string(f.repo.join(name)).unwrap(),
        "third external\n"
    );
}

#[test]
fn branches_and_all_branch_history_preserve_dirty_work_and_reject_unsafe_delete() {
    let f = Fixture::new();
    f.seed();
    let base = f.g(&["rev-parse", "HEAD"]);
    f.cli(&["branch", "create", "feature/性能", "--start", &base], 0);
    f.cli(&["branch", "switch", "feature/性能"], 0);
    f.commit_file("base.rs", "feature\n", "Feature only");
    f.cli(&["branch", "rename", "feature/性能", "feature/renamed"], 0);
    f.cli(&["branch", "switch", "main"], 0);
    f.commit_file("main.rs", "main\n", "Main only");
    let page = f.cli(&["history", "--all", "--limit", "3"], 0);
    assert_eq!(page["data"]["commits"].as_array().unwrap().len(), 3);
    assert_eq!(page["data"]["has_more"], false);
    let page = f.cli(&["history", "--limit", "1"], 0);
    assert_eq!(page["data"]["has_more"], true);
    let page = f.cli(&["history", "--revision", "feature/renamed"], 0);
    assert_eq!(page["data"]["commits"].as_array().unwrap().len(), 2);
    let rejected = f.cli(&["branch", "delete", "feature/renamed"], 1);
    assert_eq!(rejected["error"]["code"], "branch_not_merged");
    fs::write(f.repo.join("base.rs"), "PRIVATE local dirty\n").unwrap();
    f.cli(&["branch", "switch", "feature/renamed"], 1);
    assert_eq!(
        fs::read_to_string(f.repo.join("base.rs")).unwrap(),
        "PRIVATE local dirty\n"
    );
    assert_eq!(f.g(&["symbolic-ref", "--short", "HEAD"]), "main");
    f.cli(&["branch", "create", "safe-delete", "--start", &base], 0);
    f.cli(&["branch", "delete", "safe-delete"], 0);
    assert_eq!(
        f.cli(&["branch", "list"], 0)["data"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn stash_save_apply_and_expected_id_drop_preserve_untracked_data() {
    let f = Fixture::new();
    f.seed();
    fs::write(f.repo.join("base.rs"), "stashed tracked\n").unwrap();
    fs::write(f.repo.join("new 文.rs"), "stashed untracked\n").unwrap();
    f.cli(&["stash", "save", "-u", "-m", "CLI stash"], 0);
    assert!(!f.repo.join("new 文.rs").exists());
    let list = f.cli(&["stash", "list"], 0);
    let id = list["data"][0]["id"].as_str().unwrap();
    f.cli(
        &[
            "stash",
            "drop",
            "0",
            "--expect",
            "1111111111111111111111111111111111111111",
        ],
        1,
    );
    f.cli(&["stash", "apply", id, "--index"], 0);
    assert_eq!(
        fs::read_to_string(f.repo.join("new 文.rs")).unwrap(),
        "stashed untracked\n"
    );
    f.cli(&["stash", "drop", "0", "--expect", id], 0);
    assert_eq!(
        f.cli(&["stash", "list"], 0)["data"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
}

#[test]
fn merge_conflicts_are_structured_and_continue_and_abort_restore_expected_state() {
    let f = Fixture::new();
    f.seed();
    f.cli(&["branch", "create", "side"], 0);
    f.cli(&["branch", "switch", "side"], 0);
    f.commit_file("base.rs", "side\n", "side");
    f.cli(&["branch", "switch", "main"], 0);
    f.commit_file("base.rs", "main\n", "main");
    let before = f.g(&["rev-parse", "HEAD"]);
    let conflict = f.cli(&["merge", "start", "side"], 1);
    assert_eq!(conflict["data"]["merge_failure"]["code"], "merge_conflicts");
    assert_eq!(f.cli(&["operation"], 0)["data"]["idle"], false);
    f.cli(&["merge", "abort"], 0);
    assert_eq!(f.g(&["rev-parse", "HEAD"]), before);
    assert_eq!(
        fs::read_to_string(f.repo.join("base.rs")).unwrap(),
        "main\n"
    );
    f.cli(&["merge", "start", "side"], 1);
    fs::write(f.repo.join("base.rs"), "resolved\n").unwrap();
    f.cli(&["stage", "--", "base.rs"], 0);
    f.cli(&["merge", "continue", "-m", "Resolved CLI merge"], 0);
    assert_eq!(
        f.g(&["rev-list", "--parents", "-n", "1", "HEAD"])
            .split_whitespace()
            .count(),
        3
    );
    assert_eq!(f.cli(&["operation"], 0)["data"]["idle"], true);
}

#[test]
fn local_remote_clone_fetch_pull_push_and_rejections_have_authoritative_reports() {
    let f = Fixture::new();
    f.seed();
    let bare = f.root.join("remote.git");
    f.git(
        &f.root,
        &[
            OsStr::new("init"),
            OsStr::new("--bare"),
            OsStr::new("--initial-branch=main"),
            bare.as_os_str(),
        ],
    );
    let bare_s = bare.to_str().unwrap();
    f.cli(&["remote", "add", "origin", bare_s], 0);
    let p = f.cli(&["push", "origin", "main", "main", "--set-upstream"], 0);
    assert_eq!(p["data"]["remote_outcome"], "confirmed_success");
    let peer = f.root.join("clone 文");
    let peer_s = peer.to_str().unwrap();
    let c = f.cli(&["clone", "--local", bare_s, peer_s], 0);
    assert_eq!(c["data"]["published"], true);
    f.git(
        &peer,
        &[
            OsStr::new("config"),
            OsStr::new("user.name"),
            OsStr::new("Fixture"),
        ],
    );
    f.git(
        &peer,
        &[
            OsStr::new("config"),
            OsStr::new("user.email"),
            OsStr::new("fixture@example.invalid"),
        ],
    );
    fs::write(peer.join("peer.rs"), "peer\n").unwrap();
    f.invoke(&peer, &["stage", "--all"], 0);
    f.invoke(&peer, &["commit", "-m", "Peer"], 0);
    f.invoke(&peer, &["push", "origin", "main", "main"], 0);
    let before = f.g(&["rev-parse", "HEAD"]);
    f.cli(&["fetch", "origin"], 0);
    assert_eq!(f.g(&["rev-parse", "HEAD"]), before);
    let pulled = f.cli(&["pull"], 0);
    assert_eq!(pulled["data"]["update"]["data"]["kind"], "fast_forward");
    assert!(f.repo.join("peer.rs").exists());
    f.commit_file("ours.rs", "ours\n", "Our diverged commit");
    fs::write(peer.join("remote.rs"), "remote\n").unwrap();
    f.invoke(&peer, &["stage", "--all"], 0);
    f.invoke(&peer, &["commit", "-m", "Remote diverged commit"], 0);
    f.invoke(&peer, &["push", "origin", "main", "main"], 0);
    let before = f.g(&["rev-parse", "HEAD"]);
    let rejected = f.cli(&["push", "origin", "main", "main"], 1);
    assert_eq!(rejected["data"]["remote_outcome"], "rejected");
    f.cli(&["pull"], 1);
    assert_eq!(f.g(&["rev-parse", "HEAD"]), before);
    f.cli(&["remote", "set-url", "origin", bare_s], 0);
    f.cli(&["remote", "set-url", "origin", bare_s, "--push"], 0);
    let list = f.cli(&["remote", "list"], 0);
    assert_eq!(list["data"][0]["urls_omitted"], true);
    assert!(!list.to_string().contains(bare_s));
    f.cli(&["remote", "remove", "origin"], 0);
    assert!(bare.join("HEAD").exists());
    assert!(
        f.cli(&["remote", "list"], 0)["data"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    // A clone never replaces an existing directory, including an empty one.
    let safe = f.root.join("safe");
    fs::create_dir(&safe).unwrap();
    fs::write(safe.join("keep"), "PRIVATE existing").unwrap();
    f.cli(&["clone", "--local", bare_s, safe.to_str().unwrap()], 1);
    assert_eq!(
        fs::read_to_string(safe.join("keep")).unwrap(),
        "PRIVATE existing"
    );
}

#[test]
fn rejected_hooks_logs_and_pre_cancelled_writes_keep_payloads_private() {
    let f = Fixture::new();
    f.seed();
    fs::write(f.repo.join("base.rs"), "PRIVATE_CONTENT\n").unwrap();
    f.cli(&["stage", "--all"], 0);
    let before = f.g(&["rev-parse", "HEAD"]);
    let index = fs::read(f.repo.join(".git/index")).unwrap();
    let staged = f.g(&["ls-files", "--stage", "-z"]);
    let c = f.cli(
        &["--cancel-after-ms", "0", "commit", "-m", "PRIVATE_MESSAGE"],
        130,
    );
    assert_eq!(c["error"]["code"], "cancelled");
    assert_eq!(f.g(&["rev-parse", "HEAD"]), before);
    assert_eq!(fs::read(f.repo.join(".git/index")).unwrap(), index);
    fs::create_dir_all(f.repo.join(".git/hooks")).unwrap();
    let password = "ROCKET_FIXTURE_PASSWORD_ONLY";
    fs::write(f.repo.join(".git/hooks/pre-commit"),format!("#!/bin/sh\nprintf '\\033[2JPRIVATE_TOKEN=https://user:{password}@example.invalid\\n' >&2\nexit 1\n")).unwrap();
    let failed = f.cli(&["commit", "-m", "PRIVATE_MESSAGE"], 1);
    assert_eq!(failed["error"]["code"], "git_failed");
    assert!(!failed.to_string().contains("PRIVATE"));
    assert!(!failed.to_string().contains(password));
    assert_eq!(f.g(&["rev-parse", "HEAD"]), before);
    // An attempted Git commit may populate its TREE cache, even when a hook
    // rejects it. Verify every staged mode/object/path, not cache-extension bytes.
    assert_eq!(f.g(&["ls-files", "--stage", "-z"]), staged);
    let logs = fs::read_to_string(f.root.join("logs/rocket.jsonl")).unwrap();
    assert!(!logs.contains("PRIVATE"));
    assert!(!logs.contains(password));
    for row in logs.lines() {
        let row: Value = serde_json::from_str(row).unwrap();
        assert_eq!(row["dropped_before"], 0);
    }
}

#[test]
fn missing_repository_logging_failure_and_identity_failure_are_json() {
    let f = Fixture::new();
    assert_eq!(f.cli(&["status"], 1)["error"]["code"], "io_error");
    let badlog = f.root.join("not-a-directory");
    fs::write(&badlog, b"preserve").unwrap();
    f.init();
    let result = f.cli(&["--log-dir", badlog.to_str().unwrap(), "status"], 0);
    assert_eq!(result["warnings"][0]["code"], "diagnostics_unavailable");
    assert_eq!(fs::read(&badlog).unwrap(), b"preserve");
    f.g(&["config", "--unset", "user.email"]);
    f.g(&["config", "user.useConfigOnly", "true"]);
    fs::write(f.repo.join("a"), "data").unwrap();
    f.cli(&["stage", "--all"], 0);
    assert_eq!(
        f.cli(&["commit", "-m", "No identity"], 1)["error"]["code"],
        "identity_required"
    );
}

#[test]
fn timeout_during_hook_stops_the_owned_process_and_preserves_staged_changes() {
    let f = Fixture::new();
    f.seed();
    fs::write(f.repo.join("base.rs"), "pending\n").unwrap();
    f.cli(&["stage", "--all"], 0);
    let head = f.g(&["rev-parse", "HEAD"]);
    let staged = f.g(&["ls-files", "--stage", "-z"]);
    fs::create_dir_all(f.repo.join(".git/hooks")).unwrap();
    fs::write(
        f.repo.join(".git/hooks/pre-commit"),
        "#!/bin/sh\nprintf ready > ../timeout-ready\nsleep 20\n",
    )
    .unwrap();
    let started = std::time::Instant::now();
    // Leave the real safety scan time to finish before testing hook teardown.
    // A one-second budget can expire in preflight without ever starting Git
    // commit. Keep the existing ten-second total bound and twenty-second hook.
    let value = f.cli(&["--timeout-secs", "5", "commit", "-m", "timeout"], 124);
    assert_eq!(value["error"]["code"], "timed_out");
    assert!(f.root.join("timeout-ready").exists());
    assert!(started.elapsed() < std::time::Duration::from_secs(10));
    let records = fs::read_to_string(f.root.join("logs/rocket.jsonl")).unwrap();
    assert!(records.lines().any(|line| {
        let record: Value = serde_json::from_str(line).unwrap();
        record["event"] == "git_end"
            && record["operation"] == "commit"
            && record["outcome"] == "timed_out"
    }), "the timeout must stop the hook's Git process, not just preflight");
    assert_eq!(f.g(&["rev-parse", "HEAD"]), head);
    assert_eq!(f.g(&["ls-files", "--stage", "-z"]), staged);
    assert_eq!(
        fs::read_to_string(f.repo.join("base.rs")).unwrap(),
        "pending\n"
    );
}

#[test]
fn real_ctrl_c_and_ctrl_break_preserve_completed_write_outcomes() {
    use std::os::windows::process::CommandExt;
    for (hook, event) in [("pre-commit", "0"), ("post-commit", "1")] {
        let f = Fixture::new();
        f.seed();
        fs::write(f.repo.join("base.rs"), "pending\n").unwrap();
        f.cli(&["stage", "--all"], 0);
        fs::create_dir_all(f.repo.join(".git/hooks")).unwrap();
        fs::write(
            f.repo.join(format!(".git/hooks/{hook}")),
            "#!/bin/sh\nprintf ready > ../control-ready\nsleep 20\n",
        )
        .unwrap();
        let head = f.g(&["rev-parse", "HEAD"]);
        let staged = f.g(&["ls-files", "--stage", "-z"]);
        // The helper starts without a console, allocates/hides its own, then
        // broadcasts only inside that private console. No user's shell is targeted.
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
            .env("ROCKET_CLI_CONSOLE_EVENT", event)
            .creation_flags(0x08000000);
        let output = helper.output().unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let result: Value =
            serde_json::from_slice(&fs::read(f.root.join("control-result.json")).unwrap()).unwrap();
        assert_eq!(result["error"]["code"], "cancelled");
        assert_eq!(result["data"]["status"]["ok"], true);
        if hook == "pre-commit" {
            assert_eq!(f.g(&["rev-parse", "HEAD"]), head);
            assert_eq!(f.g(&["ls-files", "--stage", "-z"]), staged);
        } else {
            assert_ne!(f.g(&["rev-parse", "HEAD"]), head);
            assert!(
                result["data"]["status"]["data"]["entries"]
                    .as_array()
                    .unwrap()
                    .is_empty()
            );
        }
        assert_eq!(
            fs::read_to_string(f.repo.join("base.rs")).unwrap(),
            "pending\n"
        );
        let logs = fs::read_to_string(f.root.join("logs/rocket.jsonl")).unwrap();
        let stopped = logs
            .lines()
            .map(|l| serde_json::from_str::<Value>(l).unwrap())
            .find(|r| r["event"] == "git_end" && r["outcome"] == "cancelled")
            .expect("recorded actual Git cancellation");
        assert!(stopped["cancel_to_stop_us"].as_u64().unwrap() < 5_000_000);
        println!("{} stop_us={}", hook, stopped["cancel_to_stop_us"]);
        // The same repository is immediately usable after each interrupted CLI.
        f.cli(&["status"], 0);
        f.cli(&["branch", "create", "after-interrupt"], 0);
    }
}

#[test]
#[ignore = "private console subprocess invoked by real_ctrl_c_and_ctrl_break"]
fn console_interrupt_bridge() {
    use std::ffi::c_void;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn AllocConsole() -> i32;
        fn FreeConsole() -> i32;
        fn GetConsoleWindow() -> *mut c_void;
        fn SetConsoleCtrlHandler(
            handler: Option<unsafe extern "system" fn(u32) -> i32>,
            add: i32,
        ) -> i32;
        fn GenerateConsoleCtrlEvent(event: u32, group: u32) -> i32;
    }
    #[link(name = "user32")]
    unsafe extern "system" {
        fn ShowWindow(window: *mut c_void, command: i32) -> i32;
    }
    unsafe extern "system" fn ignore(event: u32) -> i32 {
        i32::from(event == 0 || event == 1)
    }
    let root = PathBuf::from(
        std::env::var_os("ROCKET_CLI_CONSOLE_ROOT").expect("only invoked by parent test"),
    );
    let f = Fixture {
        repo: root.join("repo 性能"),
        root,
    };
    let event = std::env::var("ROCKET_CLI_CONSOLE_EVENT")
        .unwrap()
        .parse::<u32>()
        .unwrap();
    // SAFETY: detach only this dedicated fixture process if Windows supplied a
    // hidden console, then allocate its own isolated console for the broadcast.
    unsafe {
        FreeConsole();
        assert_ne!(AllocConsole(), 0, "{}", std::io::Error::last_os_error());
        ShowWindow(GetConsoleWindow(), 0);
        assert_ne!(SetConsoleCtrlHandler(Some(ignore), 1), 0);
    }
    let pull_mode = std::env::var("ROCKET_CLI_CONSOLE_PULL").ok();
    let advanced_args: Option<Vec<String>> = std::env::var("ROCKET_CLI_CONSOLE_ARGS").ok().map(|s|serde_json::from_str(&s).unwrap());
    let args: Vec<&str> = if let Some(args) = &advanced_args {
        args.iter().map(String::as_str).collect()
    } else if let Some(mode) = pull_mode.as_deref() {
        vec!["pull", mode]
    } else {
        vec!["commit", "-m", "interrupt fixture"]
    };
    let mut command = f.command(&f.repo, &args);
    command
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let mut child = command.spawn().unwrap();
    let started = std::time::Instant::now();
    while !f.root.join("control-ready").exists() {
        if let Some(status) = child.try_wait().unwrap() {
            panic!("CLI exited before hook readiness: {status}");
        }
        if started.elapsed()
            > std::time::Duration::from_secs(if pull_mode.is_some() || advanced_args.is_some() { 60 } else { 12 })
        {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("hook did not start");
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    // SAFETY: group zero targets this private console only; the fixture handles
    // its own event while the real CLI callback requests backend cancellation.
    unsafe {
        assert_ne!(GenerateConsoleCtrlEvent(event, 0), 0);
    }
    let output = child.wait_with_output().unwrap();
    let value = f.check(output, &["real console interruption"], 130);
    fs::write(
        f.root.join("control-result.json"),
        serde_json::to_vec_pretty(&value).unwrap(),
    )
    .unwrap();
    unsafe {
        SetConsoleCtrlHandler(Some(ignore), 0);
        FreeConsole();
    }
}

fn pull_strategy_fixture(conflict: bool) -> (Fixture, String, String) {
    let f = Fixture::new();
    f.seed();
    let bare = f.root.join("remote PRIVATE.git");
    let peer = f.root.join("peer 日本語");
    f.git(
        &f.root,
        &[
            OsStr::new("clone"),
            OsStr::new("--bare"),
            f.repo.as_os_str(),
            bare.as_os_str(),
        ],
    );
    f.g(&["remote", "add", "origin", bare.to_str().unwrap()]);
    f.g(&["fetch", "origin"]);
    f.g(&["branch", "--set-upstream-to=origin/main", "main"]);
    f.git(
        &f.root,
        &[OsStr::new("clone"), bare.as_os_str(), peer.as_os_str()],
    );
    let g = |a: &[&str]| f.git(&peer, &a.iter().map(OsStr::new).collect::<Vec<_>>());
    g(&["config", "user.name", "Fixture"]);
    g(&["config", "user.email", "fixture@example.invalid"]);
    g(&["config", "commit.gpgsign", "false"]);
    g(&["config", "core.autocrlf", "false"]);
    f.commit_file(
        if conflict { "base.rs" } else { "local 文.txt" },
        "PRIVATE local\n",
        "PRIVATE local commit",
    );
    fs::write(
        peer.join(if conflict {
            "base.rs"
        } else {
            "incoming 🚀.txt"
        }),
        "PRIVATE incoming\n",
    )
    .unwrap();
    g(&["add", "."]);
    g(&["commit", "-m", "PRIVATE incoming commit"]);
    g(&["push", "origin", "main"]);
    let before = f.g(&["rev-parse", "HEAD"]);
    let target = g(&["rev-parse", "HEAD"]);
    (f, before, target)
}
#[test]
fn pull_strategy_preview_default_refusal_and_integration_preserve_history() {
    for strategy in ["--merge", "--rebase"] {
        let (f, before, target) = pull_strategy_fixture(false);
        fs::write(f.repo.join("keep 未追跡.txt"), "PRIVATE keep").unwrap();
        let index = fs::read(f.repo.join(".git/index")).unwrap();
        f.cli(&["pull"], 1);
        assert_eq!(f.g(&["rev-parse", "HEAD"]), before);
        let p = f.cli(&["pull", strategy, "--preview"], 0);
        assert_eq!(p["data"]["preview_only"], true);
        assert_eq!(p["data"]["preview"]["original"], before);
        assert_eq!(p["data"]["preview"]["target"], target);
        assert_eq!(p["data"]["preview"]["local_only"], 1);
        assert_eq!(p["data"]["preview"]["incoming"], 1);
        assert!(p["data"]["integration"].is_null());
        assert_eq!(f.g(&["rev-parse", "HEAD"]), before);
        assert_eq!(fs::read(f.repo.join(".git/index")).unwrap(), index);
        let done = f.cli(&["pull", strategy], 0);
        assert_eq!(done["data"]["status"]["ok"], true);
        assert_eq!(done["data"]["operation"]["data"]["idle"], true);
        if strategy == "--merge" {
            assert_eq!(f.g(&["rev-parse", "HEAD^1"]), before);
            assert_eq!(f.g(&["rev-parse", "HEAD^2"]), target);
        } else {
            assert_eq!(f.g(&["rev-parse", "HEAD^1"]), target);
            let recovery = done["data"]["integration"]["recovery_reference"]
                .as_str()
                .unwrap();
            assert_eq!(f.g(&["rev-parse", recovery]), before);
        }
        assert_eq!(
            fs::read(f.repo.join("keep 未追跡.txt")).unwrap(),
            b"PRIVATE keep"
        );
        assert_eq!(
            fs::read(f.repo.join("incoming 🚀.txt")).unwrap(),
            b"PRIVATE incoming\n"
        );
        f.cli(&["pull", strategy], 0);
    }
}
#[test]
fn pull_strategy_conflicts_have_merge_and_rebase_recovery() {
    let (f, before, _) = pull_strategy_fixture(true);
    let v = f.cli(&["pull", "--merge"], 1);
    assert_eq!(
        v["data"]["integration"]["merge_failure"]["code"],
        "merge_conflicts"
    );
    f.cli(&["merge", "abort"], 0);
    assert_eq!(f.g(&["rev-parse", "HEAD"]), before);
    f.cli(&["pull", "--merge"], 1);
    fs::write(f.repo.join("base.rs"), "resolved merge\n").unwrap();
    f.cli(&["stage", "--", "base.rs"], 0);
    f.cli(&["merge", "continue", "-m", "Reviewed merge resolution"], 0);
    for recovery in ["abort", "skip", "continue"] {
        let (f, before, target) = pull_strategy_fixture(true);
        fs::write(f.repo.join("untracked.txt"), "keep").unwrap();
        let v = f.cli(&["pull", "--rebase"], 1);
        assert_eq!(v["data"]["operation"]["data"]["rebase"], true);
        assert!(v["error"]["guidance"].as_str().unwrap().len() > 10);
        if recovery == "continue" {
            fs::write(f.repo.join("base.rs"), "resolved rebase\n").unwrap();
            f.cli(&["stage", "--", "base.rs"], 0);
        }
        let head = f.g(&["rev-parse", "HEAD"]);
        let index = fs::read(f.repo.join(".git/index")).unwrap();
        let p = f.cli(&["rebase", recovery, "--preview"], 0);
        assert_eq!(p["data"]["preview"]["original"], before);
        assert_eq!(f.g(&["rev-parse", "HEAD"]), head);
        assert_eq!(fs::read(f.repo.join(".git/index")).unwrap(), index);
        let done = f.cli(&["rebase", recovery], 0);
        assert_eq!(done["data"]["operation"]["data"]["idle"], true);
        match recovery {
            "abort" => assert_eq!(f.g(&["rev-parse", "HEAD"]), before),
            "skip" => assert_eq!(f.g(&["rev-parse", "HEAD"]), target),
            _ => assert_eq!(
                fs::read(f.repo.join("base.rs")).unwrap(),
                b"resolved rebase\n"
            ),
        }
        assert_eq!(fs::read(f.repo.join("untracked.txt")).unwrap(), b"keep");
    }
}
#[test]
fn pull_strategy_dirty_refusal_and_missing_upstream_preserve_changes() {
    for strategy in ["--merge", "--rebase"] {
        let (f, before, _) = pull_strategy_fixture(false);
        fs::write(f.repo.join("base.rs"), "PRIVATE staged\n").unwrap();
        f.cli(&["stage", "--", "base.rs"], 0);
        fs::write(f.repo.join("base.rs"), "PRIVATE unstaged\n").unwrap();
        let index = fs::read(f.repo.join(".git/index")).unwrap();
        let v = f.cli(&["pull", strategy], 1);
        assert_eq!(v["error"]["code"], "pull_dirty_worktree");
        assert_eq!(f.g(&["rev-parse", "HEAD"]), before);
        assert_eq!(fs::read(f.repo.join(".git/index")).unwrap(), index);
        assert_eq!(
            fs::read(f.repo.join("base.rs")).unwrap(),
            b"PRIVATE unstaged\n"
        );
    }
    let f = Fixture::new();
    f.seed();
    let head = f.g(&["rev-parse", "HEAD"]);
    let v = f.cli(&["pull", "--merge"], 1);
    assert!(!v["error"]["code"].is_null());
    assert_eq!(f.g(&["rev-parse", "HEAD"]), head);
}
#[test]
fn pull_strategy_real_cancellation_returns_fresh_observations_and_private_logs() {
    use std::os::windows::process::CommandExt;
    for (strategy, hook) in [("--merge", "pre-merge-commit"), ("--rebase", "pre-rebase")] {
        let (f, before, _) = pull_strategy_fixture(false);
        fs::create_dir_all(f.repo.join(".git/hooks")).unwrap();
        fs::write(
            f.repo.join(format!(".git/hooks/{hook}")),
            "#!/bin/sh\nprintf ready > ../control-ready\nsleep 20\n",
        )
        .unwrap();
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
            .env("ROCKET_CLI_CONSOLE_PULL", strategy)
            .creation_flags(0x08000000);
        let output = helper.output().unwrap();
        assert!(
            output.status.success(),
            "{} {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let v: Value =
            serde_json::from_slice(&fs::read(f.root.join("control-result.json")).unwrap()).unwrap();
        assert_eq!(v["error"]["code"], "cancelled");
        assert_eq!(v["data"]["status"]["ok"], true);
        assert_eq!(v["data"]["operation"]["ok"], true);
        assert_eq!(v["data"]["branches"]["ok"], true);
        assert_eq!(f.g(&["rev-parse", "HEAD"]), before);
        let records = fs::read_to_string(f.root.join("logs/rocket.jsonl")).unwrap();
        assert!(!records.contains("PRIVATE"));
        assert!(!records.contains("fixture@example.invalid"));
        let records: Vec<Value> = records
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert!(
            records
                .iter()
                .all(|r| r["dropped_before"].as_u64().unwrap_or(0) == 0)
        );
        let stopped = records
            .iter()
            .find(|r| r["event"] == "git_end" && r["outcome"] == "cancelled")
            .unwrap();
        assert!(stopped["cancel_to_stop_us"].as_u64().unwrap() < 5_000_000);
        println!("{strategy} stop_us={}", stopped["cancel_to_stop_us"]);
        f.cli(&["status"], 0);
    }
}

// A real Git hook holds a CLI write open without holding a branch ref lock.
// A second branch write used to succeed here, across Rocket processes.
struct HeldCommit {
    child: Option<std::process::Child>,
    release: PathBuf,
}
impl HeldCommit {
    fn start(f: &Fixture) -> Self {
        fs::create_dir_all(f.repo.join(".git/hooks")).unwrap();
        fs::write(f.repo.join(".git/hooks/pre-commit"),
            "#!/bin/sh\nprintf ready > .git/write-ready\nwhile ! test -f .git/write-release; do sleep 0.02; done\n").unwrap();
        let child = f
            .command(
                &f.repo,
                &["--timeout-secs", "180", "commit", "-m", "guard owner"],
            )
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let mut held = Self {
            child: Some(child),
            release: f.repo.join(".git/write-release"),
        };
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        while !f.repo.join(".git/write-ready").exists() {
            assert!(
                held.child.as_mut().unwrap().try_wait().unwrap().is_none(),
                "owner exited before hook"
            );
            assert!(
                std::time::Instant::now() < deadline,
                "owner did not reach hook"
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        held
    }
    fn finish(mut self, f: &Fixture) {
        fs::write(&self.release, "release").unwrap();
        let output = self.child.take().unwrap().wait_with_output().unwrap();
        f.check(output, &["commit owner"], 0);
    }
}
impl Drop for HeldCommit {
    fn drop(&mut self) {
        if let Some(child) = &mut self.child {
            let _ = fs::write(&self.release, "release");
            // Only the owned fixture process is stopped, including on an assertion failure.
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

#[test]
fn cross_process_writes_refuse_busy_repositories_but_allow_reads_and_independent_repositories() {
    let f = Fixture::new();
    f.seed();
    let linked = f.root.join("linked 文");
    f.git(
        &f.repo,
        &[
            OsStr::new("worktree"),
            OsStr::new("add"),
            OsStr::new("-b"),
            OsStr::new("linked"),
            linked.as_os_str(),
        ],
    );
    fs::write(f.repo.join("base.rs"), "owner content\n").unwrap();
    f.cli(&["stage", "--all"], 0);
    fs::write(f.repo.join("preserved 文.txt"), "keep untracked\n").unwrap();
    let staged = f.g(&["ls-files", "--stage", "-z"]);
    let head = f.g(&["rev-parse", "HEAD"]);
    let config = fs::read(f.repo.join(".git/config")).unwrap();
    let mut held = HeldCommit::start(&f);
    // Git may refresh cache-tree metadata before entering pre-commit. Snapshot
    // the owner's ready state so exact-byte checks measure only the contenders.
    assert_eq!(f.g(&["ls-files", "--stage", "-z"]), staged);
    let index = fs::read(f.repo.join(".git/index")).unwrap();
    for at in [&f.repo, &linked, &f.repo.join(".")] {
        let started = std::time::Instant::now();
        let v = f.invoke(at, &["branch", "create", "must-not-exist"], 1);
        assert_eq!(v["error"]["code"], "repository_busy");
        assert!(v["error"]["guidance"].as_str().unwrap().contains("refresh"));
        // Discovery can stall before reaching the gate. Prove the contender
        // returned while the owner is still held, then measure the guard's own
        // diagnostic span below instead of conflating it with Git startup.
        assert!(held.child.as_mut().unwrap().try_wait().unwrap().is_none());
        assert!(!held.release.exists());
        println!("busy CLI wall time: {:?}", started.elapsed());
    }
    assert_eq!(
        f.cli(&["stage", "--all"], 1)["error"]["code"],
        "repository_busy"
    );
    assert_eq!(
        f.cli(&["unstage", "--all"], 1)["error"]["code"],
        "repository_busy"
    );
    f.cli(&["status"], 0);
    f.cli(&["history", "--limit", "1"], 0);
    assert_eq!(f.g(&["rev-parse", "HEAD"]), head);
    assert_eq!(fs::read(f.repo.join(".git/index")).unwrap(), index);
    assert_eq!(fs::read(f.repo.join(".git/config")).unwrap(), config);
    assert_eq!(f.g(&["branch", "--list", "must-not-exist"]), "");
    let independent = Fixture::new();
    independent.seed();
    independent.cli(&["branch", "create", "independent-write"], 0);
    held.finish(&f);
    f.cli(&["branch", "create", "retry-after-owner"], 0);
    assert_ne!(f.g(&["rev-parse", "HEAD"]), head);
    assert_eq!(f.g(&["show", "HEAD:base.rs"]), "owner content");
    assert_eq!(
        fs::read_to_string(f.repo.join("preserved 文.txt")).unwrap(),
        "keep untracked\n"
    );
    let records = fs::read_to_string(f.root.join("logs/rocket.jsonl")).unwrap();
    let busy: Vec<Value> = records
        .lines()
        .map(|s| serde_json::from_str::<Value>(s).unwrap())
        .filter(|r| r["outcome"] == "repository_busy")
        .collect();
    assert!(
        busy.iter().any(|r| r["event"] == "write_queue"),
        "busy write guard recorded"
    );
    let guard_records: Vec<_> = busy
        .iter()
        .filter(|r| r["event"] == "write_queue")
        .collect();
    assert_eq!(guard_records.len(), 5);
    for record in guard_records {
        assert!(
            record["elapsed_ms"].as_u64().unwrap() < 500,
            "busy guard must not wait for owner: {record}"
        );
        println!("busy guard ms: {}", record["elapsed_ms"]);
    }
    assert!(
        !records.contains("must-not-exist")
            && !records.contains("preserved 文")
            && !records.contains("owner content")
    );
}

#[test]
fn pull_rebase_prediction_is_visible_in_json_and_text_without_integration() {
    for conflict in [false, true] {
        let (f, head, _) = pull_strategy_fixture(conflict);
        let index = fs::read(f.repo.join(".git/index")).unwrap();
        let value = f.cli(&["pull", "--rebase", "--preview"], 0);
        let p = &value["data"]["preview"]["prediction"];
        let expected = if conflict { "conflicts" } else { "clean" };
        assert_eq!(p["outcome"], expected);
        assert_eq!(p["checked"], 1);
        assert_eq!(p["total"], 1);
        if conflict {
            assert_eq!(p["stopped_at"], 0);
            assert_eq!(p["paths"][0]["text"], "base.rs");
        }
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_rocket-cli"));
        f.env(&mut cmd);
        let output = cmd
            .arg("--repo")
            .arg(&f.repo)
            .args(["pull", "--rebase", "--preview"])
            .output()
            .unwrap();
        assert!(output.status.success());
        let text = String::from_utf8(output.stdout).unwrap();
        assert!(
            text.contains(&format!("Content prediction: {expected}")),
            "{text}"
        );
        if conflict {
            assert!(text.contains("base.rs"));
        }
        assert_eq!(f.g(&["rev-parse", "HEAD"]), head);
        assert_eq!(fs::read(f.repo.join(".git/index")).unwrap(), index);
        assert!(
            f.cli(&["pull", "--merge", "--preview"], 0)["data"]["preview"]["prediction"].is_null()
        );
    }
}

fn line_id(review: &Value, sign: &str, text: &str) -> String {
    let lines = review["data"]["changed_lines"].as_array().unwrap();
    let line = lines.iter().find(|l| l["sign"] == sign && l["text"]["text"] == text).unwrap();
    line["id"].as_u64().unwrap().to_string()
}

#[test]
fn line_selection_stages_reviewed_lines_and_refuses_a_changed_patch() {
    let f = Fixture::new();
    f.seed();
    f.commit_file("lines.rs", "one\ntwo\nthree\n", "lines");
    fs::write(f.repo.join("lines.rs"), "one\nTWO\nthree\nfour\n").unwrap();
    let review = f.cli(&["hunks", "--", "lines.rs"], 0);
    assert_eq!(review["data"]["hunks"].as_array().unwrap().len(), 1);
    let sha = review["data"]["patch_sha256"].as_str().unwrap().to_owned();
    let staged = f.cli(&["stage", "--lines", &line_id(&review, "+", "four"), "--expect-patch", &sha, "--", "lines.rs"], 0);
    assert_eq!(staged["command"], "stage-selection");
    assert_eq!(f.g(&["show", ":lines.rs"]), "one\ntwo\nthree\nfour");
    assert_eq!(fs::read_to_string(f.repo.join("lines.rs")).unwrap(), "one\nTWO\nthree\nfour\n");
    // The unstaged patch changed with that write, so the old pin is refused.
    let stale = f.cli(&["stage", "--lines", &line_id(&review, "+", "TWO"), "--expect-patch", &sha, "--", "lines.rs"], 1);
    assert_eq!(stale["error"]["code"], "selection_stale_preview");
    assert_eq!(f.g(&["show", ":lines.rs"]), "one\ntwo\nthree\nfour");
    let index = f.cli(&["hunks", "--staged", "--", "lines.rs"], 0);
    let sha = index["data"]["patch_sha256"].as_str().unwrap();
    f.cli(&["unstage", "--lines", &line_id(&index, "+", "four"), "--expect-patch", sha, "--", "lines.rs"], 0);
    assert_eq!(f.g(&["diff", "--cached", "--name-only"]), "");
    // A new file: one selected line becomes the staged file; the working file stays.
    fs::write(f.repo.join("new 文.txt"), "keep\nskip\n").unwrap();
    let new = f.cli(&["hunks", "--", "new 文.txt"], 0);
    assert_eq!(new["data"]["untracked"], true);
    let sha = new["data"]["patch_sha256"].as_str().unwrap();
    f.cli(&["stage", "--lines", &line_id(&new, "+", "keep"), "--expect-patch", sha, "--", "new 文.txt"], 0);
    assert_eq!(f.g(&["show", ":new 文.txt"]), "keep");
    assert_eq!(fs::read_to_string(f.repo.join("new 文.txt")).unwrap(), "keep\nskip\n");
}

#[test]
fn conflict_commands_pin_the_file_choose_blocks_and_keep_whole_versions() {
    let f = Fixture::new();
    f.seed();
    let files = [("text.rs", "top\n{1}\nc\nd\ne\nf\n{2}\nbottom\n"), ("whole.txt", "{1} whole\n"), ("manual.txt", "{1} manual\n")];
    let commit = |label: &str, message: &str| {
        for (name, text) in files {
            fs::write(f.repo.join(name), text.replace("{1}", &format!("{label}1")).replace("{2}", &format!("{label}2"))).unwrap();
        }
        f.cli(&["stage", "--all"], 0);
        f.cli(&["commit", "-m", message], 0);
    };
    commit("base", "base versions");
    f.cli(&["branch", "create", "other"], 0);
    f.cli(&["branch", "switch", "other"], 0);
    commit("theirs", "their versions");
    f.cli(&["branch", "switch", "main"], 0);
    commit("ours", "our versions");
    assert_eq!(f.cli(&["merge", "start", "other"], 1)["data"]["merge_failure"]["code"], "merge_conflicts");

    let shown = f.cli(&["conflict", "show", "text.rs"], 0);
    let text = &shown["data"]["text"];
    assert_eq!(text["conflicts"].as_array().unwrap().len(), 2);
    assert_eq!(text["conflicts"][0]["ours"]["text"], "ours1\n");
    assert_eq!(text["conflicts"][1]["theirs"]["text"], "theirs2\n");
    let sha = text["sha256"].as_str().unwrap().to_owned();
    let before = fs::read(f.repo.join("text.rs")).unwrap();
    let wrong = "0".repeat(64);
    let stale = f.cli(&["conflict", "resolve", "text.rs", "--expect-sha256", &wrong, "--choose", "all=ours"], 1);
    assert_eq!(stale["error"]["code"], "conflict_file_changed");
    let partial = f.cli(&["conflict", "resolve", "text.rs", "--expect-sha256", &sha, "--choose", "1=ours", "--stage"], 1);
    assert_eq!(partial["error"]["code"], "conflict_markers_remain");
    let preview = f.cli(&["conflict", "resolve", "text.rs", "--expect-sha256", &sha, "--choose", "2=theirs,1=both", "--preview"], 0);
    let resolved = "top\nours1\ntheirs1\nc\nd\ne\nf\ntheirs2\nbottom\n";
    assert_eq!(preview["data"]["preview"]["result"]["text"], resolved);
    assert_eq!(preview["data"]["preview"]["remaining_conflicts"], 0);
    assert_eq!(fs::read(f.repo.join("text.rs")).unwrap(), before);
    let saved = f.cli(&["conflict", "resolve", "text.rs", "--expect-sha256", &sha, "--choose", "2=theirs,1=both", "--stage"], 0);
    assert_eq!(saved["command"], "conflict-save");
    assert_eq!(saved["data"]["staged"], true);
    assert!(saved["data"]["recovery_directory"]["text"].is_string());
    assert_eq!(fs::read_to_string(f.repo.join("text.rs")).unwrap(), resolved);

    let whole = f.cli(&["conflict", "show", "whole.txt"], 0);
    let ours = whole["data"]["versions"]["ours"]["id"].as_str().unwrap().to_owned();
    let theirs = whole["data"]["versions"]["theirs"]["id"].as_str().unwrap().to_owned();
    let stale = f.cli(&["conflict", "keep", "whole.txt", "--side", "theirs", "--expect", &ours], 1);
    assert_eq!(stale["error"]["code"], "conflict_review_stale");
    assert_eq!(f.cli(&["conflict", "keep", "whole.txt", "--side", "theirs", "--expect", &theirs], 0)["data"]["staged"], true);
    assert_eq!(fs::read_to_string(f.repo.join("whole.txt")).unwrap(), "theirs1 whole\n");

    // An externally edited file is staged only once its markers are gone.
    let edited = fs::read_to_string(f.repo.join("manual.txt")).unwrap();
    assert!(edited.contains("<<<<<<<"));
    assert_eq!(f.cli(&["conflict", "mark-resolved", "--", "manual.txt"], 1)["error"]["code"], "invalid_input");
    fs::write(f.repo.join("manual.txt"), "edited manual\n").unwrap();
    f.cli(&["conflict", "mark-resolved", "--", "manual.txt"], 0);
    f.cli(&["merge", "continue", "-m", "Resolved with the CLI"], 0);
    assert_eq!(f.g(&["show", "HEAD:whole.txt"]), "theirs1 whole");
    assert_eq!(f.cli(&["operation"], 0)["data"]["idle"], true);
}

/// Commits c0..c4 where `state.txt` turns bad in c2; returns their IDs.
fn bisect_history(f: &Fixture) -> Vec<String> {
    f.seed();
    ["good", "good", "bad", "bad", "bad"].iter().enumerate().map(|(i, state)| {
        fs::write(f.repo.join("state.txt"), format!("{state}\n")).unwrap();
        fs::write(f.repo.join("step.txt"), format!("{i}\n")).unwrap();
        f.cli(&["stage", "--all"], 0);
        f.cli(&["commit", "-m", &format!("step {i}")], 0);
        f.g(&["rev-parse", "HEAD"])
    }).collect()
}
/// Test output goes to a fixture folder, not to the real local application data.
fn bisect_cli(f: &Fixture, args: &[&str], exit: i32) -> Value {
    let mut cmd = f.command(&f.repo, args);
    cmd.env("LOCALAPPDATA", f.root.join("local"));
    f.check(cmd.output().unwrap(), args, exit)
}

#[test]
fn bisect_steps_and_reviewed_runs_find_the_first_bad_commit_and_restore() {
    use std::os::windows::process::CommandExt;
    let f = Fixture::new();
    let ids = bisect_history(&f);
    let (good, culprit, bad) = (ids[0].as_str(), ids[2].as_str(), ids[4].as_str());
    assert_eq!(f.cli(&["bisect", "start", good, bad, "--preview"], 0)["data"]["operation_started"], false);
    assert!(!f.repo.join(".git/BISECT_START").exists());
    f.cli(&["bisect", "start", good, bad], 0);
    assert_eq!(f.cli(&["bisect", "good", "--expect", good], 1)["error"]["code"], "bisect_stale_preview");
    let found = loop {
        let session = f.cli(&["bisect", "status"], 0)["data"]["session"].clone();
        if session["phase"] == "found" {
            break session;
        }
        assert_eq!(session["phase"], "testing");
        let verdict = if fs::read_to_string(f.repo.join("state.txt")).unwrap() == "good\n" { "good" } else { "bad" };
        f.cli(&["bisect", verdict, "--expect", session["current"]["target"].as_str().unwrap()], 0);
    };
    assert_eq!(found["first_bad"], culprit);
    f.cli(&["bisect", "restore"], 0);
    assert_eq!(f.g(&["symbolic-ref", "HEAD"]), "refs/heads/main");
    assert_eq!(f.cli(&["bisect", "status"], 0)["data"]["active"], false);

    // An automatic run tests each candidate with a reviewed executable. (findstr /x
    // never matches a line of an LF-only file, so the plain substring test is used.)
    let findstr = r"C:\Windows\System32\findstr.exe";
    let test = ["--", findstr, "/c:good", "state.txt"];
    let run = |extra: &[&str], exit: i32| {
        let mut args = vec!["bisect", "run", good, bad, "--test-timeout", "60"];
        args.extend(extra);
        args.extend(test);
        bisect_cli(&f, &args, exit)
    };
    let review = run(&["--preview"], 0);
    assert_eq!(review["command"], "bisect-auto-preview");
    let digest = review["data"]["preview"]["program"]["sha256"].as_str().unwrap().to_owned();
    let wrong = "0".repeat(64);
    assert_eq!(run(&["--expect-sha256", &wrong], 1)["error"]["code"], "bisect_program_changed");
    assert!(!f.repo.join(".git/BISECT_START").exists());
    let result = run(&["--expect-sha256", &digest], 0);
    assert_eq!(result["data"]["result"], "found");
    assert_eq!(result["data"]["session"]["data"]["first_bad"], culprit);
    let tests = result["data"]["tests"].as_array().unwrap();
    assert!(tests.iter().all(|t| t["recorded"] == true && (t["outcome"] == "good" || t["outcome"] == "bad")), "{tests:?}");
    let saved = f.cli(&["bisect", "status"], 0);
    assert_eq!(saved["data"]["tests"]["data"].as_array().unwrap().len(), tests.len());
    assert_eq!(run(&["--preview"], 1)["error"]["code"], "bisect_active_operation");
    f.cli(&["bisect", "restore"], 0);

    // A test past its limit is not classified; the search stays for --continue or restore.
    let ping = r"C:\Windows\System32\PING.EXE";
    let slow = ["--", ping, "-n", "6", "127.0.0.1"];
    let mut args = vec!["bisect", "run", good, bad, "--test-timeout", "1", "--preview"];
    args.extend(slow);
    let digest = bisect_cli(&f, &args, 0)["data"]["preview"]["program"]["sha256"].as_str().unwrap().to_owned();
    let mut args = vec!["bisect", "run", good, bad, "--test-timeout", "1", "--expect-sha256", &digest];
    args.extend(slow);
    let timed = bisect_cli(&f, &args, 124);
    assert_eq!(timed["error"]["code"], "bisect_test_timed_out");
    assert_eq!(timed["data"]["tests"][0]["outcome"], "timeout");
    assert_eq!(timed["data"]["tests"][0]["recorded"], false);
    assert_eq!(f.cli(&["bisect", "status"], 0)["data"]["session"]["phase"], "testing");

    // Ctrl+Break during a test stops its process tree and keeps the search.
    let cmd = r"C:\Windows\System32\cmd.exe";
    let script = r"type nul > ..\control-ready & ping -n 30 127.0.0.1 > nul";
    let review = bisect_cli(&f, &["bisect", "run", "--continue", "--test-timeout", "120", "--preview", "--", cmd, "/d", "/c", script], 0);
    let digest = review["data"]["preview"]["program"]["sha256"].as_str().unwrap();
    let args = ["bisect", "run", "--continue", "--test-timeout", "120", "--expect-sha256", digest, "--", cmd, "/d", "/c", script];
    let mut helper = Command::new(std::env::current_exe().unwrap());
    f.env(&mut helper);
    helper
        .args(["--ignored", "--exact", "console_interrupt_bridge", "--nocapture"])
        .env("ROCKET_CLI_CONSOLE_ROOT", &f.root)
        .env("ROCKET_CLI_CONSOLE_EVENT", "1")
        .env("ROCKET_CLI_CONSOLE_ARGS", serde_json::to_string(&args).unwrap())
        .env("LOCALAPPDATA", f.root.join("local"))
        .creation_flags(0x08000000);
    let output = helper.output().unwrap();
    assert!(output.status.success(), "{} {}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
    let cancelled: Value = serde_json::from_slice(&fs::read(f.root.join("control-result.json")).unwrap()).unwrap();
    assert_eq!(cancelled["error"]["code"], "cancelled");
    assert_eq!(cancelled["data"]["tests"].as_array().unwrap().last().unwrap()["outcome"], "cancelled");
    assert_eq!(f.cli(&["bisect", "status"], 0)["data"]["session"]["phase"], "testing");
    f.cli(&["bisect", "restore"], 0);
    assert_eq!(f.g(&["rev-parse", "HEAD"]), bad);
    assert_eq!(f.cli(&["operation"], 0)["data"]["bisect"], false);
    let logs = fs::read_to_string(f.root.join("logs/rocket.jsonl")).unwrap();
    assert!(!logs.contains("control-ready") && !logs.contains("127.0.0.1"));
}

#[test]
fn remote_tags_publish_and_delete_only_the_reviewed_objects() {
    let f = Fixture::new();
    f.seed();
    let bare = advanced_remote(&f);
    f.cli(&["tag", "create", "v1", "HEAD", "--message", "Release one"], 0);
    let tag = f.cli(&["tag", "inspect", "v1"], 0)["data"]["entry"]["id"].as_str().unwrap().to_owned();
    let relation = |f: &Fixture| {
        let listed = f.cli(&["tag", "remote", "origin"], 0);
        listed["data"]["entries"].as_array().unwrap().iter().find(|e| e["name"]["text"] == "v1").unwrap()["relation"].clone()
    };
    assert_eq!(relation(&f), "local_only");
    let head = f.g(&["rev-parse", "HEAD"]);
    let stale = f.cli(&["tag", "publish", "v1", "--remote", "origin", "--source", &head], 1);
    assert_eq!(stale["error"]["code"], "tag_stale_preview");
    let preview = f.cli(&["tag", "publish", "v1", "--remote", "origin", "--source", &tag, "--preview"], 0);
    assert_eq!(preview["data"]["operation_started"], false);
    assert_eq!(bare_git(&f, &bare, &["tag", "--list"]), "");
    let published = f.cli(&["tag", "publish", "v1", "--remote", "origin", "--source", &tag], 0);
    assert_eq!(published["data"]["remote_outcome"], "published");
    assert_eq!(bare_git(&f, &bare, &["rev-parse", "refs/tags/v1"]), tag);
    assert_eq!(relation(&f), "published");
    let again = f.cli(&["tag", "publish", "v1", "--remote", "origin", "--source", &tag], 1);
    assert_eq!(again["error"]["code"], "remote_tag_exists");
    // A remote tag that moved after review is never deleted by the old lease.
    bare_git(&f, &bare, &["tag", "-f", "v1", "HEAD"]);
    let moved = bare_git(&f, &bare, &["rev-parse", "refs/tags/v1"]);
    let refused = f.cli(&["tag", "delete-remote", "v1", "--remote", "origin", "--expect", &tag], 1);
    assert_eq!(refused["error"]["code"], "remote_tag_changed");
    assert_eq!(bare_git(&f, &bare, &["rev-parse", "refs/tags/v1"]), moved);
    let deleted = f.cli(&["tag", "delete-remote", "v1", "--remote", "origin", "--expect", &moved], 0);
    assert_eq!(deleted["data"]["remote_outcome"], "deleted");
    assert_eq!(bare_git(&f, &bare, &["tag", "--list"]), "");
    assert_eq!(f.g(&["rev-parse", "refs/tags/v1"]), tag);
}

#[test]
fn stash_paths_parts_lines_and_pop_keep_other_changes() {
    let f = Fixture::new();
    f.seed();
    f.commit_file("a.rs", "a1\na2\na3\n", "a");
    f.commit_file("b.rs", "b1\nb2\nb3\n", "b");
    fs::write(f.repo.join("a.rs"), "a1\nA2\na3\nA4\n").unwrap();
    fs::write(f.repo.join("b.rs"), "b1\nB2\nb3\n").unwrap();
    f.cli(&["stash", "save", "-m", "only a", "--", "a.rs"], 0);
    assert_eq!(fs::read_to_string(f.repo.join("a.rs")).unwrap(), "a1\na2\na3\n");
    assert_eq!(fs::read_to_string(f.repo.join("b.rs")).unwrap(), "b1\nB2\nb3\n");
    let id = f.cli(&["stash", "list"], 0)["data"][0]["id"].as_str().unwrap().to_owned();

    // Apply only the saved A4 line; the index and the stash stay as they are.
    let shown = f.cli(&["stash", "show", &id], 0);
    let file = &shown["data"]["files"][0];
    assert_eq!(file["path"]["text"], "a.rs");
    let a4 = file["patch"]["text"].as_str().unwrap().split('\n').position(|l| l == "+A4").unwrap().to_string();
    let sha = file["patch_sha256"].as_str().unwrap().to_owned();
    let wrong = "0".repeat(64);
    let stale = f.cli(&["stash", "apply", &id, "--lines", &a4, "--expect-patch", &wrong, "--", "a.rs"], 1);
    assert_eq!(stale["error"]["code"], "selection_stale_preview");
    let preview = f.cli(&["stash", "apply", &id, "--lines", &a4, "--expect-patch", &sha, "--preview", "--", "a.rs"], 0);
    assert_eq!(preview["data"]["operation_started"], false);
    assert_eq!(fs::read_to_string(f.repo.join("a.rs")).unwrap(), "a1\na2\na3\n");
    let applied = f.cli(&["stash", "apply", &id, "--lines", &a4, "--expect-patch", &sha, "--", "a.rs"], 0);
    assert_eq!(applied["data"]["verification"]["ok"], true);
    assert_eq!(fs::read_to_string(f.repo.join("a.rs")).unwrap(), "a1\na2\na3\nA4\n");
    assert_eq!(f.g(&["diff", "--cached", "--name-only"]), "");
    assert_eq!(f.cli(&["stash", "list"], 0)["data"].as_array().unwrap().len(), 1);

    // Save the selected lines of b.rs as their own stash; the file loses only them.
    let review = f.cli(&["hunks", "--", "b.rs"], 0);
    let lines = format!("{},{}", line_id(&review, "-", "b2"), line_id(&review, "+", "B2"));
    let sha = review["data"]["patch_sha256"].as_str().unwrap();
    let saved = f.cli(&["stash", "save", "-m", "B2 only", "--lines", &lines, "--expect-patch", sha, "--", "b.rs"], 0);
    assert_eq!(fs::read_to_string(f.repo.join("b.rs")).unwrap(), "b1\nb2\nb3\n");
    let line_stash = saved["data"]["stash"].as_str().unwrap().to_owned();
    assert_eq!(f.g(&["rev-parse", saved["data"]["recovery_reference"].as_str().unwrap()]), line_stash);
    assert_eq!(f.cli(&["stash", "list"], 0)["data"][0]["id"], line_stash.as_str());

    // Pop needs a clean working tree, then restores the stash and keeps a backup ref.
    assert_eq!(f.cli(&["stash", "pop", "0", "--expect", &line_stash], 1)["error"]["code"], "invalid_input");
    f.g(&["checkout", "--", "a.rs"]);
    let popped = f.cli(&["stash", "pop", "0", "--expect", &line_stash], 0);
    assert_eq!(popped["data"]["removed"], true);
    assert_eq!(fs::read_to_string(f.repo.join("b.rs")).unwrap(), "b1\nB2\nb3\n");
    assert_eq!(f.g(&["rev-parse", popped["data"]["recovery_reference"].as_str().unwrap()]), line_stash);
    assert_eq!(f.cli(&["stash", "list"], 0)["data"][0]["id"], id.as_str());
}

#[test]
fn commit_undo_values_undo_only_that_unpublished_commit_and_keep_its_changes() {
    let f = Fixture::new();
    f.seed();
    let base = f.g(&["rev-parse", "HEAD"]);
    fs::write(f.repo.join("undo.rs"), "undo me\n").unwrap();
    f.cli(&["stage", "--", "undo.rs"], 0);
    let undo = f.cli(&["commit", "-m", "To be undone"], 0)["data"]["undo"].clone();
    let head = f.g(&["rev-parse", "HEAD"]);
    assert_eq!(undo["commit"], head.as_str());
    assert_eq!(undo["previous"], base.as_str());
    assert_eq!(undo["branch"]["text"], "main");
    let sha = undo["index_sha256"].as_str().unwrap().to_owned();
    let args = ["undo-commit", "--branch", "main", "--expect", head.as_str(), "--index-sha256", sha.as_str()];
    // Staging after the commit, and a remote-tracking branch with the commit, block Undo.
    fs::write(f.repo.join("later.rs"), "later\n").unwrap();
    f.cli(&["stage", "--", "later.rs"], 0);
    assert_eq!(f.cli(&args, 1)["error"]["code"], "undo_index_changed");
    f.cli(&["unstage", "--", "later.rs"], 0);
    f.g(&["update-ref", "refs/remotes/origin/main", &head]);
    assert_eq!(f.cli(&args, 1)["error"]["code"], "undo_shared_commit");
    f.g(&["update-ref", "-d", "refs/remotes/origin/main"]);
    let mut preview = args.to_vec();
    preview.push("--preview");
    assert_eq!(f.cli(&preview, 0)["data"]["operation_started"], false);
    assert_eq!(f.g(&["rev-parse", "HEAD"]), head);
    let undone = f.cli(&args, 0);
    assert_eq!(undone["command"], "undo-commit");
    assert_eq!(undone["data"]["verification"]["ok"], true);
    assert_eq!(f.g(&["rev-parse", "HEAD"]), base);
    assert_eq!(f.g(&["diff", "--cached", "--name-only"]), "undo.rs");
    assert_eq!(fs::read_to_string(f.repo.join("undo.rs")).unwrap(), "undo me\n");
    assert_eq!(f.g(&["rev-parse", undone["data"]["recovery_reference"].as_str().unwrap()]), head);
    assert_eq!(f.cli(&args, 1)["error"]["code"], "undo_head_changed");
}

#[test]
fn cherry_pick_revert_and_amend_apply_only_the_reviewed_commits() {
    let f = Fixture::new();
    f.seed();
    f.cli(&["branch", "create", "side"], 0);
    f.cli(&["branch", "switch", "side"], 0);
    f.commit_file("pick.rs", "picked\n", "Pick me");
    let pick = f.g(&["rev-parse", "HEAD"]);
    f.commit_file("base.rs", "side base\n", "Conflicting change");
    let conflicting = f.g(&["rev-parse", "HEAD"]);
    f.cli(&["branch", "switch", "main"], 0);
    f.commit_file("base.rs", "main base\n", "Main change");
    let head = f.g(&["rev-parse", "HEAD"]);
    let preview = f.cli(&["cherry-pick", &pick, "--preview"], 0);
    assert_eq!(preview["command"], "commit-preview");
    assert_eq!(preview["data"]["preview"]["commits"][0]["id"], pick.as_str());
    assert_eq!(f.cli(&["cherry-pick", &pick, "--expect", &pick], 1)["error"]["code"], "stale_preview");
    assert_eq!(f.g(&["rev-parse", "HEAD"]), head);
    assert_eq!(f.cli(&["cherry-pick", &pick, "--expect", &head], 0)["command"], "cherry-pick");
    assert_eq!(fs::read_to_string(f.repo.join("pick.rs")).unwrap(), "picked\n");
    let picked = f.g(&["rev-parse", "HEAD"]);
    f.cli(&["revert", &picked], 0);
    assert!(!f.repo.join("pick.rs").exists());
    // Amend replaces only the reviewed commit and keeps it under a recovery ref.
    let reverted = f.g(&["rev-parse", "HEAD"]);
    assert_eq!(f.cli(&["amend", "-m", "Reworded", "--expect", &head], 1)["error"]["code"], "stale_preview");
    let amended = f.cli(&["amend", "-m", "Reworded revert", "--expect", &reverted], 0);
    assert_eq!(f.g(&["log", "-1", "--format=%s"]), "Reworded revert");
    assert_eq!(f.g(&["rev-parse", amended["data"]["recovery_reference"].as_str().unwrap()]), reverted);
    // A conflicting pick stops for resolution; abort returns to where it started.
    let before = f.g(&["rev-parse", "HEAD"]);
    let stopped = f.cli(&["cherry-pick", &conflicting], 1);
    assert_eq!(stopped["data"]["operation"]["data"]["cherry_pick"], true);
    f.cli(&["cherry-pick", "abort"], 0);
    assert_eq!(f.g(&["rev-parse", "HEAD"]), before);
    assert_eq!(fs::read_to_string(f.repo.join("base.rs")).unwrap(), "main base\n");
    assert_eq!(f.cli(&["operation"], 0)["data"]["idle"], true);
}

#[test]
fn rebase_start_and_data_plans_rewrite_only_the_reviewed_range() {
    let f = Fixture::new();
    f.seed();
    f.cli(&["branch", "create", "topic"], 0);
    f.cli(&["branch", "switch", "topic"], 0);
    for name in ["one", "two", "three"] {
        f.commit_file(&format!("{name}.rs"), &format!("{name}\n"), &format!("Add {name}"));
    }
    f.cli(&["branch", "switch", "main"], 0);
    f.commit_file("main.rs", "main\n", "Main moves");
    let main = f.g(&["rev-parse", "HEAD"]);
    f.cli(&["branch", "switch", "topic"], 0);
    let original = f.g(&["rev-parse", "HEAD"]);
    let review = f.cli(&["rebase", "start", "main", "--preview"], 0);
    assert_eq!(review["data"]["preview"]["original"], original.as_str());
    assert_eq!(review["data"]["preview"]["onto"], main.as_str());
    assert_eq!(review["data"]["preview"]["commits"].as_array().unwrap().len(), 3);
    let stale = f.cli(&["rebase", "start", "main", "--expect-original", &main, "--expect-onto", &main], 1);
    assert_eq!(stale["error"]["code"], "stale_preview");
    assert_eq!(f.g(&["rev-parse", "HEAD"]), original);
    let rebased = f.cli(&["rebase", "start", "main", "--expect-original", &original, "--expect-onto", &main], 0);
    assert_eq!(rebased["command"], "rebase");
    assert_eq!(f.g(&["merge-base", "HEAD", "main"]), main);
    assert_eq!(f.g(&["rev-parse", rebased["data"]["recovery_reference"].as_str().unwrap()]), original);

    // An interactive review lists the range as picks even when the branch is up to date.
    // A data-only plan then rewords the first commit, folds in the second, drops the third.
    let tip = f.g(&["rev-parse", "HEAD"]);
    assert_eq!(f.cli(&["rebase", "start", "main", "--preview"], 0)["data"]["preview"]["up_to_date"], true);
    let review = f.cli(&["rebase", "start", "main", "--interactive", "--preview"], 0);
    let rows = review["data"]["preview"]["plan"].as_array().unwrap();
    assert!(rows.iter().all(|r| r["instruction"] == "pick"), "{rows:?}");
    let commits: Vec<String> = rows.iter().map(|r| r["commit"].as_str().unwrap().to_owned()).collect();
    assert_eq!(commits.len(), 3);
    let plan = f.root.join("plan.json");
    fs::write(&plan, serde_json::to_vec(&serde_json::json!([
        {"commit": commits[0], "instruction": "reword", "message": "One and two"},
        {"commit": commits[1], "instruction": "fixup"},
        {"commit": commits[2], "instruction": "drop"},
    ])).unwrap()).unwrap();
    let commands = f.root.join("commands.json");
    fs::write(&commands, serde_json::to_vec(&serde_json::json!([
        {"commit": commits[0], "instruction": "pick", "exec": "echo PRIVATE_COMMAND"},
        {"commit": commits[1], "instruction": "pick"},
        {"commit": commits[2], "instruction": "pick"},
    ])).unwrap()).unwrap();
    let refused = f.cli(&["rebase", "start", "main", "--plan", commands.to_str().unwrap(), "--preview"], 1);
    assert_eq!(refused["error"]["code"], "invalid_input");
    let plan = plan.to_str().unwrap();
    let planned = f.cli(&["rebase", "start", "main", "--plan", plan, "--preview"], 0);
    assert_eq!(planned["data"]["preview"]["plan"][1]["instruction"], "fixup");
    let applied = f.cli(&["rebase", "start", "main", "--plan", plan, "--expect-original", &tip, "--expect-onto", &main], 0);
    assert_eq!(applied["command"], "interactive-rebase");
    assert_eq!(f.g(&["log", "--format=%s", &format!("{main}..HEAD")]), "One and two");
    assert!(f.repo.join("one.rs").exists() && f.repo.join("two.rs").exists() && !f.repo.join("three.rs").exists());
    assert_eq!(f.cli(&["operation"], 0)["data"]["idle"], true);
    let logs = fs::read_to_string(f.root.join("logs/rocket.jsonl")).unwrap();
    assert!(!logs.contains("One and two") && !logs.contains("PRIVATE_COMMAND"));
}

#[test]
fn reflog_reset_and_signature_reads_follow_the_reviewed_head() {
    let f = Fixture::new();
    f.seed();
    let first = f.g(&["rev-parse", "HEAD"]);
    f.commit_file("later.rs", "later\n", "Later");
    let later = f.g(&["rev-parse", "HEAD"]);
    let log = f.cli(&["reflog", "--limit", "5"], 0);
    assert!(log["data"]["entries"].as_array().unwrap().iter().any(|e| e["id"] == later.as_str()));
    // A hard reset refuses tracked edits instead of discarding them.
    fs::write(f.repo.join("base.rs"), "edited\n").unwrap();
    assert_eq!(f.cli(&["reset", "--hard", &first, "--expect", &later], 1)["error"]["code"], "dirty_worktree");
    assert_eq!(fs::read_to_string(f.repo.join("base.rs")).unwrap(), "edited\n");
    f.g(&["checkout", "--", "base.rs"]);
    assert_eq!(f.cli(&["reset", "--soft", &first, "--expect", &first], 1)["error"]["code"], "stale_preview");
    assert_eq!(f.cli(&["reset", "--soft", &first, "--expect", &later, "--preview"], 0)["data"]["operation_started"], false);
    assert_eq!(f.g(&["rev-parse", "HEAD"]), later);
    let reset = f.cli(&["reset", "--soft", &first, "--expect", &later], 0);
    assert_eq!(reset["command"], "reset-soft");
    assert_eq!(f.g(&["rev-parse", "HEAD"]), first);
    assert_eq!(f.g(&["diff", "--cached", "--name-only"]), "later.rs");
    assert_eq!(f.g(&["rev-parse", reset["data"]["recovery_reference"].as_str().unwrap()]), later);
    // An unsigned commit verifies as unsigned; --require-valid turns that into a failure.
    assert_eq!(f.cli(&["verify", "commit", "HEAD"], 0)["data"]["status"], "unsigned");
    assert_eq!(f.cli(&["verify", "commit", "HEAD", "--require-valid"], 1)["error"]["code"], "signature_not_valid");
    f.cli(&["tag", "create", "v1", "HEAD", "--message", "Release"], 0);
    let tag = f.cli(&["verify", "tag", "v1"], 0);
    assert_eq!((tag["data"]["kind"].as_str(), tag["data"]["status"].as_str()), (Some("tag"), Some("unsigned")));
    assert_eq!(f.cli(&["signing", "show"], 0)["data"]["format"], "openpgp");
}

#[test]
fn history_reads_blame_follow_renames_search_and_compare_without_writes() {
    let f = Fixture::new();
    f.seed();
    let lines: String = (1..=9).map(|n| format!("line {n}\n")).collect();
    f.commit_file("old name.rs", &lines, "Add file");
    let added = f.g(&["rev-parse", "HEAD"]);
    f.g(&["mv", "old name.rs", "new 名.rs"]);
    f.cli(&["commit", "-m", "Rename file"], 0);
    f.commit_file("new 名.rs", &lines.replace("line 5\n", "changed needle\n"), "Change needle");
    let head = f.g(&["rev-parse", "HEAD"]);
    let index = f.g(&["ls-files", "--stage"]);
    let blame = f.cli(&["blame", "HEAD", "--path", "new 名.rs"], 0);
    let blamed = blame["data"]["lines"].as_array().unwrap();
    assert_eq!(blamed.len(), 9);
    assert_eq!(blamed[0]["commit"], added.as_str());
    assert_eq!(blamed[4]["commit"], head.as_str());
    assert_eq!(blamed[4]["content"]["text"], "changed needle");
    let history = f.cli(&["file-history", "new 名.rs"], 0);
    let entries = history["data"]["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 3);
    assert_eq!(entries[1]["previous_path"]["text"], "old name.rs");
    let text = f.cli(&["search", "needle"], 0);
    assert_eq!(text["data"]["commits"][0]["id"], head.as_str());
    assert_eq!(text["data"]["complete"], true);
    let content = f.cli(&["search", "changed needle", "--kind", "content"], 0);
    assert_eq!(content["data"]["commits"].as_array().unwrap().len(), 1);
    let path = f.cli(&["search", "old name", "--kind", "path"], 0);
    assert!(path["data"]["commits"].as_array().unwrap().iter().any(|c| c["id"] == added.as_str()));
    let compared = f.cli(&["compare", &added, &head], 0);
    let file = &compared["data"]["files"][0];
    assert_eq!((file["change"].as_str(), file["original_path"]["text"].as_str()), (Some("renamed"), Some("old name.rs")));
    fs::write(f.repo.join("base.rs"), "worktree edit\n").unwrap();
    let local = f.cli(&["compare", "HEAD", "--worktree"], 0);
    assert_eq!(local["data"]["local"], "worktree");
    assert!(local["data"]["target"].is_null());
    assert_eq!(local["data"]["files"][0]["path"]["text"], "base.rs");
    assert_eq!(f.g(&["rev-parse", "HEAD"]), head);
    assert_eq!(f.g(&["ls-files", "--stage"]), index);
}

#[test]
fn remote_maintenance_prunes_deletes_renames_fast_forwards_and_fetches_tags() {
    let f = Fixture::new();
    f.seed();
    let bare = advanced_remote(&f);
    for branch in ["gone", "keep", "side"] {
        f.cli(&["branch", "create", branch], 0);
        f.cli(&["push", "origin", branch, branch, "--set-upstream"], 0);
    }
    // A server branch deleted elsewhere leaves an obsolete tracking ref to prune.
    bare_git(&f, &bare, &["update-ref", "-d", "refs/heads/gone"]);
    let review = f.cli(&["remote", "prune", "origin", "--preview"], 0);
    let obsolete = review["data"]["preview"]["obsolete"].as_array().unwrap().clone();
    assert!(obsolete.iter().any(|r| r["reference"]["text"] == "refs/remotes/origin/gone"), "{obsolete:?}");
    f.cli(&["remote", "prune", "origin", "--", "refs/remotes/origin/gone"], 0);
    assert_eq!(f.g(&["for-each-ref", "refs/remotes/origin/gone"]), "");
    // Deleting a server branch needs its advertised object as the lease.
    let keep = bare_git(&f, &bare, &["rev-parse", "refs/heads/keep"]);
    let wrong = "0".repeat(40);
    assert_eq!(f.cli(&["branch", "delete-remote", "origin", "keep", "--expect", &wrong], 1)["error"]["code"], "remote_branch_changed");
    assert_eq!(bare_git(&f, &bare, &["rev-parse", "refs/heads/keep"]), keep);
    assert_eq!(f.cli(&["branch", "delete-remote", "origin", "keep", "--expect", &keep], 0)["data"]["remote_outcome"], "deleted");
    assert_eq!(bare_git(&f, &bare, &["for-each-ref", "refs/heads/keep"]), "");
    f.cli(&["remote", "rename", "origin", "upstream", "--preview"], 0);
    assert_eq!(f.g(&["remote"]), "origin");
    f.cli(&["remote", "rename", "origin", "upstream"], 0);
    assert_eq!(f.g(&["remote"]), "upstream");
    assert_ne!(f.g(&["for-each-ref", "refs/remotes/upstream/side"]), "");
    // Another clone advances side and publishes a tag; fast-forward moves only side.
    let other = f.root.join("other");
    f.git(&f.root, &[OsStr::new("clone"), bare.as_os_str(), other.as_os_str()]);
    let og = |args: &[&str]| f.git(&other, &args.iter().map(OsStr::new).collect::<Vec<_>>());
    og(&["config", "user.name", "Other"]);
    og(&["config", "user.email", "other@example.invalid"]);
    og(&["switch", "side"]);
    fs::write(other.join("side.rs"), "side\n").unwrap();
    og(&["add", "side.rs"]);
    og(&["commit", "-m", "Advance side"]);
    og(&["tag", "-a", "v9", "-m", "Release"]);
    og(&["push", "origin", "side", "v9"]);
    let advanced = og(&["rev-parse", "HEAD"]);
    f.cli(&["fetch", "upstream"], 0);
    assert_eq!(f.g(&["tag", "--list"]), "");
    let review = f.cli(&["branch", "fast-forward", "--preview"], 0);
    let rows = review["data"]["preview"]["branches"].as_array().unwrap().clone();
    let side = rows.iter().find(|r| r["branch"]["text"] == "refs/heads/side").unwrap();
    assert_eq!(side["after"], advanced.as_str());
    assert!(side["blocked"].is_null(), "{side:?}");
    f.cli(&["branch", "fast-forward", "--", "side"], 0);
    assert_eq!(f.g(&["rev-parse", "refs/heads/side"]), advanced);
    f.cli(&["fetch", "upstream", "--tags"], 0);
    assert_eq!(f.g(&["tag", "--list"]), "v9");
}

#[test]
fn worktree_maintenance_moves_repairs_and_prunes_registrations() {
    let f = Fixture::new();
    f.seed();
    let head = f.g(&["rev-parse", "HEAD"]);
    let first = f.root.join("linked one");
    let moved = f.root.join("moved one");
    let (first_s, moved_s) = (first.to_str().unwrap(), moved.to_str().unwrap());
    f.cli(&["worktree", "create", first_s, "--branch", "one"], 0);
    let wrong = "0".repeat(40);
    assert_eq!(f.cli(&["worktree", "move", first_s, moved_s, "--expect", &wrong], 1)["error"]["code"], "worktree_stale_preview");
    f.cli(&["worktree", "move", first_s, moved_s, "--expect", &head, "--preview"], 0);
    assert!(first.exists() && !moved.exists());
    assert_eq!(f.cli(&["worktree", "move", first_s, moved_s, "--expect", &head], 0)["command"], "worktree-move");
    assert!(!first.exists() && moved.join("base.rs").exists());
    // A folder moved outside Git is repaired from where it now is.
    let elsewhere = f.root.join("elsewhere one");
    fs::rename(&moved, &elsewhere).unwrap();
    f.cli(&["worktree", "repair", moved_s, "--at", elsewhere.to_str().unwrap(), "--expect", &head], 0);
    let listed = |f: &Fixture| f.cli(&["worktree", "list"], 0)["data"]["entries"].as_array().unwrap().iter()
        .map(|e| e["path"]["text"].as_str().unwrap().to_owned()).collect::<Vec<_>>();
    assert!(listed(&f).iter().any(|p| p.ends_with("elsewhere one")), "{:?}", listed(&f));
    assert_eq!(f.g(&["-C", elsewhere.to_str().unwrap(), "rev-parse", "HEAD"]), head);
    // A deleted folder's registration is pruned; a recovery ref keeps its HEAD.
    let second = f.root.join("linked two");
    let second_s = second.to_str().unwrap();
    f.cli(&["worktree", "create", second_s, "--branch", "two"], 0);
    fs::remove_dir_all(&second).unwrap();
    let pruned = f.cli(&["worktree", "prune", second_s, "--expect", &head], 0);
    assert_eq!(listed(&f).len(), 2);
    assert_eq!(f.g(&["rev-parse", pruned["data"]["recovery_reference"].as_str().unwrap()]), head);
}

#[test]
fn identity_signed_tags_and_tag_order_use_repository_settings() {
    let f = Fixture::new();
    f.seed();
    assert_eq!(f.cli(&["identity", "show"], 0)["data"]["local"]["name"], "Rocket CLI Fixture");
    assert_eq!(f.cli(&["identity", "set", "--name", "Renamed Fixture", "--preview"], 0)["data"]["operation_started"], false);
    assert_eq!(f.g(&["config", "--local", "user.name"]), "Rocket CLI Fixture");
    f.cli(&["identity", "set", "--name", "Renamed Fixture"], 0);
    assert_eq!(f.g(&["config", "--local", "user.name"]), "Renamed Fixture");
    assert_eq!(f.g(&["config", "--local", "user.email"]), "cli@example.invalid");
    // An older annotated tag, then a newer tag signed with a disposable SSH key.
    let mut old = Command::new("git");
    f.env(&mut old);
    old.arg("-C").arg(&f.repo).args(["tag", "-a", "a-old", "-m", "Old"]).env("GIT_COMMITTER_DATE", "2001-01-01T00:00:00Z");
    assert!(old.output().unwrap().status.success());
    let key = f.root.join("signing");
    let keygen = Command::new(r"C:\Windows\System32\OpenSSH\ssh-keygen.exe")
        .args(["-q", "-t", "ed25519", "-N", "", "-C", "fixture", "-f"]).arg(&key).output().unwrap();
    assert!(keygen.status.success(), "{}", String::from_utf8_lossy(&keygen.stderr));
    let public = key.with_extension("pub");
    let allowed = f.root.join("allowed_signers");
    fs::write(&allowed, format!("cli@example.invalid {}", fs::read_to_string(&public).unwrap())).unwrap();
    f.g(&["config", "--local", "gpg.format", "ssh"]);
    f.g(&["config", "--local", "user.signingkey", public.to_str().unwrap()]);
    f.g(&["config", "--local", "gpg.ssh.allowedSignersFile", allowed.to_str().unwrap()]);
    assert_eq!(f.cli(&["tag", "create", "v1", "HEAD", "--sign"], 2)["error"]["code"], "usage");
    let signed = f.cli(&["tag", "create", "v1", "HEAD", "--message", "Signed release", "--sign"], 0);
    assert_eq!(signed["command"], "tag-sign");
    assert_eq!(f.cli(&["verify", "tag", "v1"], 0)["data"]["status"], "valid");
    let names = |order: &str| f.cli(&["tag", "list", "--order", order], 0)["data"]["entries"].as_array().unwrap().iter()
        .map(|e| e["name"]["text"].as_str().unwrap().to_owned()).collect::<Vec<_>>();
    assert_eq!(names("name"), ["a-old", "v1"]);
    assert_eq!(names("newest"), ["v1", "a-old"]);
}

#[test]
fn lfs_tracks_pushes_fetches_and_checks_out_through_a_local_remote() {
    use std::io::Write as _;
    let f = Fixture::new();
    fs::OpenOptions::new().append(true).open(f.root.join("global.gitconfig")).unwrap()
        .write_all(b"[filter \"lfs\"]\n\tclean = git-lfs clean -- %f\n\tsmudge = git-lfs smudge -- %f\n\tprocess = git-lfs filter-process\n\trequired = true\n")
        .unwrap();
    f.seed();
    let preview = f.cli(&["lfs", "track", "*.bin", "--preview"], 0);
    assert!(preview["data"]["preview"]["attributes"]["added"].as_str().unwrap().contains("*.bin"), "{preview}");
    assert!(!f.repo.join(".gitattributes").exists());
    assert_eq!(f.cli(&["lfs", "track", "*.bin"], 0)["command"], "lfs-track");
    let tracking = f.cli(&["lfs", "status"], 0)["data"]["tracking"].clone();
    // Rocket quotes a non-literal pattern: "*.bin" filter=lfs ...
    let rule = tracking.as_array().unwrap().iter().find(|r| r["text"].as_str().unwrap().contains("*.bin")).unwrap().clone();
    let asset = b"\x00binary asset\x01".to_vec();
    fs::write(f.repo.join("asset.bin"), &asset).unwrap();
    f.cli(&["stage", "--all"], 0);
    f.cli(&["commit", "-m", "Add asset"], 0);
    let status = f.cli(&["lfs", "status"], 0);
    let file = status["data"]["files"].as_array().unwrap().iter().find(|e| e["path"]["text"] == "asset.bin").unwrap().clone();
    assert_eq!(file["cached"], true);
    let oid = file["oid"].as_str().unwrap().to_owned();
    // Push to a local bare remote, then fetch and check out in a clone made without content.
    let bare = advanced_remote(&f);
    assert_eq!(f.cli(&["lfs", "push", "origin"], 0)["data"]["upload_outcome"], "confirmed_by_helper");
    assert!(bare.join("lfs/objects").join(&oid[0..2]).join(&oid[2..4]).join(&oid).exists());
    let clone = f.root.join("clone");
    let mut cloning = Command::new("git");
    f.env(&mut cloning);
    cloning.arg("clone").arg(&bare).arg(&clone).env("GIT_LFS_SKIP_SMUDGE", "1");
    assert!(cloning.output().unwrap().status.success());
    let pointer = f.invoke(&clone, &["lfs", "status"], 0);
    let entry = pointer["data"]["files"].as_array().unwrap().iter().find(|e| e["path"]["text"] == "asset.bin").unwrap().clone();
    assert_eq!((entry["state"].as_str(), entry["cached"].as_bool()), (Some("pointer"), Some(false)));
    assert_eq!(f.invoke(&clone, &["lfs", "fetch", "origin"], 0)["data"]["verification"]["ok"], true);
    f.invoke(&clone, &["lfs", "checkout", "--", "asset.bin"], 0);
    assert_eq!(fs::read(clone.join("asset.bin")).unwrap(), asset);
    // Locks need an LFS server; a local remote is refused before anything is sent.
    assert_eq!(f.cli(&["lfs", "locks", "origin"], 1)["error"]["code"], "lfs_lock_unsupported");
    assert_eq!(f.cli(&["lfs", "lock", "asset.bin", "--remote", "origin"], 1)["error"]["code"], "lfs_lock_unsupported");
    // Untrack removes exactly the reviewed rule.
    let line = rule["line"].to_string();
    f.cli(&["lfs", "untrack", "--line", &line, "--expect", rule["text"].as_str().unwrap()], 0);
    assert!(!fs::read_to_string(f.repo.join(".gitattributes")).unwrap().contains("*.bin"));
}

#[test]
fn ssh_keys_check_use_and_restore_read_only_the_fixture_home() {
    let f = Fixture::new();
    f.seed();
    let home = f.root.join("home");
    let ssh = home.join(".ssh");
    fs::create_dir_all(&ssh).unwrap();
    let key = ssh.join("id_fixture");
    let keygen = Command::new(r"C:\Windows\System32\OpenSSH\ssh-keygen.exe")
        .args(["-q", "-t", "ed25519", "-N", "", "-C", "fixture", "-f"]).arg(&key).output().unwrap();
    assert!(keygen.status.success(), "{}", String::from_utf8_lossy(&keygen.stderr));
    // The user's own .ssh folder is never read here. Rocket always asks the Windows
    // ssh-agent service which keys are unlocked (it ignores SSH_AUTH_SOCK), read only.
    let run = |args: &[&str], exit: i32| {
        let mut cmd = f.command(&f.repo, args);
        cmd.env("USERPROFILE", &home).env("HOME", &home)
            .env_remove("GIT_SSH").env_remove("GIT_SSH_COMMAND").env_remove("GIT_SSH_VARIANT");
        f.check(cmd.output().unwrap(), args, exit)
    };
    let keys = run(&["ssh", "keys"], 0);
    let found = keys["data"]["keys"].as_array().unwrap().clone();
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(found[0]["public_key"].as_str().unwrap().starts_with("ssh-ed25519 "));
    assert_eq!(found[0]["unlocked"], false);
    assert!(keys["data"]["agent_running"].is_boolean());
    assert!(keys["data"]["profiles"].as_array().unwrap().is_empty());
    // A Rocket profile for the existing key, made as the desktop's SSH keys page does.
    let cancel = rocket_git::CancellationToken::new();
    rocket_git::SshManager::new(ssh.join("rocket-keys"), None).unwrap()
        .prepare("rocket-work", "example.invalid", "git", 22, Some(&key), &cancel).unwrap()
        .finish(&cancel).unwrap();
    assert_eq!(run(&["ssh", "keys"], 0)["data"]["profiles"][0]["alias"], "rocket-work");
    assert!(run(&["ssh", "check", "rocket-work"], 0)["data"]["effective_config"].as_str().unwrap().contains("example.invalid"));
    let config = fs::read(f.repo.join(".git/config")).unwrap();
    let wrong = "0".repeat(64);
    assert_eq!(run(&["ssh", "use", "rocket-work", "--expect-config", &wrong], 1)["error"]["code"], "ssh_config_changed");
    let review = run(&["ssh", "use", "rocket-work", "--preview"], 0);
    let sha = review["data"]["preview"]["config_sha256"].as_str().unwrap().to_owned();
    assert_eq!(fs::read(f.repo.join(".git/config")).unwrap(), config);
    assert_eq!(run(&["ssh", "use", "rocket-work", "--expect-config", &sha], 0)["command"], "ssh-binding-save");
    assert!(f.g(&["config", "--local", "core.sshCommand"]).contains("rocket-keys"));
    let restore = run(&["ssh", "restore", "--preview"], 0);
    assert_eq!(restore["data"]["preview"]["restoring"], true);
    run(&["ssh", "restore", "--expect-config", restore["data"]["preview"]["config_sha256"].as_str().unwrap()], 0);
    assert_eq!(fs::read(f.repo.join(".git/config")).unwrap(), config);
}
