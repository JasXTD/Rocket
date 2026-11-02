#![cfg(windows)]
mod common;
use common::{Fixture, isolated};
use rocket_git::*;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};
fn case(name: &str, f: fn()) {
    isolated("ROCKET_LFS_CASE", name, f)
}
fn entry(f: &Fixture, name: &str, content: &[u8]) -> (RepoPath, PathBuf, Vec<u8>) {
    let payload = f.root.join("payload.bin");
    fs::write(&payload, content).unwrap();
    let pointer = f.run(&["lfs", "pointer", &format!("--file={}", payload.display())]);
    let text = String::from_utf8(pointer.clone()).unwrap();
    let oid = text
        .lines()
        .find_map(|s| s.strip_prefix("oid sha256:"))
        .unwrap();
    let object = f
        .repository
        .join(".git/lfs/objects")
        .join(&oid[..2])
        .join(&oid[2..4])
        .join(oid);
    fs::create_dir_all(object.parent().unwrap()).unwrap();
    fs::write(&object, content).unwrap();
    f.write(
        ".gitattributes",
        "*.dat filter=lfs diff=lfs merge=lfs -text\n",
    );
    f.write(name, &text);
    f.commit("LFS fixture");
    (
        RepoPath::new(name.as_bytes().to_vec()).unwrap(),
        object,
        pointer,
    )
}
fn snapshot(f: &Fixture) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    (
        f.run(&["rev-parse", "HEAD"]),
        fs::read(f.repository.join(".git/index")).unwrap(),
        fs::read(f.repository.join(".git/config")).unwrap(),
    )
}
#[test]
fn network_scope_matches_exact_sources_and_stale_reviews_refuse_execution() {
    case(
        "network_scope_matches_exact_sources_and_stale_reviews_refuse_execution",
        || {
            let f = Fixture::new();
            entry(&f, "asset.dat", b"synthetic");
            let allowed = "https://example.invalid/approved.git";
            f.run(&["remote", "add", "origin", allowed]);
            let scope = LfsSourceScope {
                root: f.root.clone(),
                network_sources: vec![allowed.into()],
            };
            let c = CancellationToken::new();
            let repo = f.repo();
            assert!(
                repo.preview_lfs(LfsAction::Fetch("origin".into()), Some(&f.root), &c)
                    .is_err()
            );
            let preview = repo
                .preview_lfs_scoped(LfsAction::Fetch("origin".into()), Some(&scope), &c)
                .unwrap();
            assert_eq!(preview.source(), Some(allowed));
            for other in [
                "https://example.invalid/approved.git/extra",
                "https://example.invalid/other.git",
                "https://example.invalid.evil/approved.git",
                "https://user@example.invalid/approved.git",
            ] {
                f.run(&["remote", "set-url", "origin", other]);
                assert!(
                    repo.preview_lfs_scoped(LfsAction::Fetch("origin".into()), Some(&scope), &c)
                        .is_err()
                );
            }
            let before = snapshot(&f);
            let result = repo.apply_lfs(&preview, &c).unwrap();
            assert!(matches!(result.result, Err(Error::Lfs(LfsFailure::Stale))));
            assert_eq!(snapshot(&f), before);
        },
    );
}
fn success(r: LfsWriteReport) {
    assert!(r.result.is_ok(), "{:?}", r.result);
    assert!(r.verification.is_ok(), "{:?}", r.verification);
    assert!(r.observed.is_ok(), "{:?}", r.observed);
}
#[test]
fn normal_authentication_metadata_does_not_block_lfs_inspection() {
    case(
        "normal_authentication_metadata_does_not_block_lfs_inspection",
        || {
            let f = Fixture::new();
            entry(&f, "asset.dat", b"synthetic");
            let key = "lfs.https://example.invalid/owner/repo.git/info/lfs.access";
            f.run(&["config", key, "basic"]);
            let before = snapshot(&f);
            let c = CancellationToken::new();
            assert_eq!(f.repo().lfs_status(&c).unwrap().entries.len(), 1);
            assert_eq!(snapshot(&f), before);
            f.run(&["config", key, "unknown"]);
            assert!(matches!(
                f.repo().lfs_status(&c),
                Err(Error::Lfs(LfsFailure::Configuration))
            ));
        },
    );
}
#[test]
fn unicode_checkout_preserves_index_head_edits_and_retains_pointer() {
    case(
        "unicode_checkout_preserves_index_head_edits_and_retains_pointer",
        || {
            let f = Fixture::new();
            let (path, _, pointer) = entry(&f, "素材/背景 文.dat", b"binary\0LFS content\n");
            f.write("untracked.txt", "keep");
            let before = snapshot(&f);
            let c = CancellationToken::new();
            let repo = f.repo();
            let status = repo.lfs_status(&c).unwrap();
            assert_eq!(status.entries.len(), 1);
            assert_eq!(status.entries[0].state, LfsFileState::Pointer);
            assert!(status.entries[0].cached);
            let p = repo
                .preview_lfs(LfsAction::Checkout(path), None, &c)
                .unwrap();
            let r = repo.apply_lfs(&p, &c).unwrap();
            let recovery = r.recovery_directory.clone().unwrap();
            success(r);
            assert_eq!(fs::read(recovery.join("before.pointer")).unwrap(), pointer);
            assert_eq!(
                fs::read(recovery.join("original.pointer")).unwrap(),
                pointer
            );
            assert!(recovery.join("RECOVERY.md").exists());
            assert_eq!(
                fs::read(f.repository.join("素材/背景 文.dat")).unwrap(),
                b"binary\0LFS content\n"
            );
            assert_eq!(snapshot(&f), before);
            assert_eq!(
                fs::read(f.repository.join("untracked.txt")).unwrap(),
                b"keep"
            );
        },
    )
}
#[test]
fn modified_stale_corrupt_and_hidden_files_are_refused() {
    case(
        "modified_stale_corrupt_and_hidden_files_are_refused",
        || {
            let f = Fixture::new();
            let (path, object, pointer) = entry(&f, "file.dat", b"payload");
            let c = CancellationToken::new();
            let repo = f.repo();
            let before = snapshot(&f);
            let p = repo
                .preview_lfs(LfsAction::Checkout(path.clone()), None, &c)
                .unwrap();
            f.write("file.dat", "local edit");
            let r = repo.apply_lfs(&p, &c).unwrap();
            assert!(matches!(r.result, Err(Error::Lfs(LfsFailure::Stale))));
            assert_eq!(
                fs::read(f.repository.join("file.dat")).unwrap(),
                b"local edit"
            );
            assert!(matches!(
                repo.preview_lfs(LfsAction::Checkout(path.clone()), None, &c),
                Err(Error::Lfs(LfsFailure::Modified))
            ));
            fs::write(f.repository.join("file.dat"), &pointer).unwrap();
            fs::write(&object, b"corrupt").unwrap();
            assert!(matches!(
                repo.preview_lfs(LfsAction::Checkout(path.clone()), None, &c),
                Err(Error::Lfs(LfsFailure::InvalidObject))
            ));
            fs::write(&object, b"payload").unwrap();
            f.run(&["update-index", "--skip-worktree", "file.dat"]);
            assert!(matches!(
                repo.preview_lfs(LfsAction::Checkout(path), None, &c),
                Err(Error::Lfs(LfsFailure::Layout))
            ));
            assert_eq!(f.run(&["rev-parse", "HEAD"]), before.0);
        },
    )
}
#[test]
fn missing_cache_config_and_active_operations_have_clear_failures() {
    case(
        "missing_cache_config_and_active_operations_have_clear_failures",
        || {
            let f = Fixture::new();
            let (path, object, _) = entry(&f, "file.dat", b"payload");
            let c = CancellationToken::new();
            let repo = f.repo();
            fs::remove_file(object).unwrap();
            assert!(matches!(
                repo.preview_lfs(LfsAction::Checkout(path.clone()), None, &c),
                Err(Error::Lfs(LfsFailure::MissingObject))
            ));
            f.run(&["config", "lfs.storage", "elsewhere"]);
            assert!(matches!(
                repo.lfs_status(&c),
                Err(Error::Lfs(LfsFailure::Configuration))
            ));
            f.run(&["config", "--unset", "lfs.storage"]);
            f.write(".lfsconfig", "[lfs]\nurl=https://example.invalid/lfs\n");
            assert!(matches!(
                repo.lfs_status(&c),
                Err(Error::Lfs(LfsFailure::Configuration))
            ));
            fs::remove_file(f.repository.join(".lfsconfig")).unwrap();
            fs::create_dir(f.repository.join(".git/rebase-merge")).unwrap();
            assert!(matches!(
                repo.preview_lfs(LfsAction::Checkout(path), None, &c),
                Err(Error::Lfs(LfsFailure::Busy))
            ));
        },
    )
}
#[test]
fn local_fetch_is_pinned_and_does_not_checkout_or_change_staging() {
    case(
        "local_fetch_is_pinned_and_does_not_checkout_or_change_staging",
        || {
            let source = Fixture::new();
            let (_, _, _) = entry(&source, "文件.dat", b"download payload");
            let f = Fixture::new();
            f.run(&[
                "remote",
                "add",
                "origin",
                source.repository.to_str().unwrap(),
            ]);
            f.run(&["fetch", "origin"]);
            f.run(&["reset", "--hard", "origin/main"]);
            f.write("untracked.txt", "preserve");
            let before = snapshot(&f);
            let pointer = fs::read(f.repository.join("文件.dat")).unwrap();
            let c = CancellationToken::new();
            let repo = f.repo();
            assert!(!repo.lfs_status(&c).unwrap().entries[0].cached);
            let p = repo
                .preview_lfs(LfsAction::Fetch("origin".into()), None, &c)
                .unwrap();
            assert_eq!(p.entries().len(), 1);
            let expected = f.root.join("expected.config");
            fs::write(&expected, &before.2).unwrap();
            f.run(&[
                "config",
                "--file",
                expected.to_str().unwrap(),
                "lfs.repositoryformatversion",
                "0",
            ]);
            success(repo.apply_lfs(&p, &c).unwrap());
            assert!(repo.lfs_status(&c).unwrap().entries[0].cached);
            assert_eq!(
                snapshot(&f),
                (before.0, before.1, fs::read(expected).unwrap())
            );
            assert_eq!(fs::read(f.repository.join("文件.dat")).unwrap(), pointer);
            let p = repo
                .preview_lfs(LfsAction::Fetch("origin".into()), None, &c)
                .unwrap();
            f.run(&["config", "lfs.fetchinclude", "unreviewed"]);
            assert!(matches!(
                repo.apply_lfs(&p, &c).unwrap().result,
                Err(Error::Lfs(LfsFailure::Stale))
            ));
        },
    )
}
#[test]
fn checkout_rechecks_cache_and_cancellation_before_publication() {
    case(
        "checkout_rechecks_cache_and_cancellation_before_publication",
        || {
            let f = Fixture::new();
            let (path, object, pointer) = entry(&f, "file.dat", b"payload");
            let c = CancellationToken::new();
            let repo = f.repo();
            let p = repo
                .preview_lfs(LfsAction::Checkout(path), None, &c)
                .unwrap();
            let cancelled = CancellationToken::new();
            cancelled.cancel();
            assert!(matches!(
                repo.apply_lfs(&p, &cancelled),
                Err(Error::Cancelled)
            ));
            assert!(!f.repository.join(".git/rocket-lfs-checkouts").exists());
            fs::write(object, b"corrupt").unwrap();
            let r = repo.apply_lfs(&p, &c).unwrap();
            assert!(matches!(
                r.result,
                Err(Error::Lfs(LfsFailure::InvalidObject))
            ));
            assert_eq!(r.completed_steps, 0);
            assert_eq!(fs::read(f.repository.join("file.dat")).unwrap(), pointer);
        },
    )
}
#[test]
fn working_hardlinks_preserve_external_files() {
    case("working_hardlinks_preserve_external_files", || {
        let f = Fixture::new();
        let (path, object, pointer) = entry(&f, "file.dat", b"payload");
        let outside = f.root.join("outside.pointer");
        fs::hard_link(f.repository.join("file.dat"), &outside).unwrap();
        let c = CancellationToken::new();
        let repo = f.repo();
        let p = repo
            .preview_lfs(LfsAction::Checkout(path), None, &c)
            .unwrap();
        success(repo.apply_lfs(&p, &c).unwrap());
        assert_eq!(fs::read(outside).unwrap(), pointer);
        assert_eq!(fs::read(object).unwrap(), b"payload");
    })
}
#[test]
fn wrong_repository_and_changed_index_are_refused() {
    case("wrong_repository_and_changed_index_are_refused", || {
        let f = Fixture::new();
        let (path, _, _) = entry(&f, "file.dat", b"payload");
        let c = CancellationToken::new();
        let repo = f.repo();
        let p = repo
            .preview_lfs(LfsAction::Checkout(path), None, &c)
            .unwrap();
        let other = Fixture::new();
        entry(&other, "file.dat", b"payload");
        assert!(matches!(
            other.repo().apply_lfs(&p, &c).unwrap().result,
            Err(Error::Lfs(LfsFailure::Stale))
        ));
        f.write("new.txt", "staged");
        f.run(&["add", "new.txt"]);
        assert!(matches!(
            repo.apply_lfs(&p, &c).unwrap().result,
            Err(Error::Lfs(LfsFailure::Stale))
        ));
    })
}
#[test]
fn mcp_root_rejects_remote_sources_before_download() {
    case("mcp_root_rejects_remote_sources_before_download", || {
        let f = Fixture::new();
        entry(&f, "file.dat", b"payload");
        f.run(&[
            "remote",
            "add",
            "origin",
            "https://example.invalid/private.git",
        ]);
        let c = CancellationToken::new();
        assert!(matches!(
            f.repo()
                .preview_lfs(LfsAction::Fetch("origin".into()), Some(&f.root), &c),
            Err(Error::InvalidInput(_))
        ));
        let other = Fixture::new();
        f.run(&[
            "remote",
            "set-url",
            "origin",
            other.repository.to_str().unwrap(),
        ]);
        assert!(matches!(
            f.repo()
                .preview_lfs(LfsAction::Fetch("origin".into()), Some(&f.root), &c),
            Err(Error::InvalidInput(_))
        ));
    })
}
#[test]
fn empty_and_malformed_pointers_have_explicit_states() {
    case("empty_and_malformed_pointers_have_explicit_states", || {
        let f = Fixture::new();
        let c = CancellationToken::new();
        assert!(f.repo().lfs_status(&c).unwrap().entries.is_empty());
        f.write(".gitattributes", "*.dat filter=lfs\n");
        f.write(
            "bad.dat",
            "version https://git-lfs.github.com/spec/v1\noid sha256:invalid\nsize 7\n",
        );
        f.commit("invalid LFS fixture");
        assert!(matches!(
            f.repo().lfs_status(&c),
            Err(Error::Lfs(LfsFailure::InvalidObject))
        ));
    })
}
#[test]
fn noncanonical_pointers_cannot_escape_the_fetch_review() {
    case(
        "noncanonical_pointers_cannot_escape_the_fetch_review",
        || {
            let f = Fixture::new();
            let (_, _, pointer) = entry(&f, "edge.dat", b"review every object");
            let pointer = String::from_utf8(pointer).unwrap();
            let oid = pointer
                .lines()
                .find_map(|s| s.strip_prefix("oid "))
                .unwrap();
            for text in [
                format!("ext-0-example {oid}\n{pointer}"),
                format!("\u{2003}{pointer}"),
                pointer.replace(
                    "https://git-lfs.github.com/spec/v1",
                    "http://git-media.io/v/2",
                ),
            ] {
                f.write("edge.dat", &text);
                // Git LFS accepts these noncanonical pointers. Rocket must either
                // review them or explicitly refuse them before invoking fetch.
                f.run(&["lfs", "pointer", "--check", "--file=edge.dat"]);
                f.commit("noncanonical pointer");
                let before = snapshot(&f);
                assert!(matches!(
                    f.repo().lfs_status(&CancellationToken::new()),
                    Err(Error::Lfs(LfsFailure::InvalidObject))
                ));
                assert_eq!(snapshot(&f), before);
            }
        },
    )
}
#[test]
fn url_rewrites_cannot_redirect_a_reviewed_lfs_endpoint() {
    case(
        "url_rewrites_cannot_redirect_a_reviewed_lfs_endpoint",
        || {
            let f = Fixture::new();
            entry(&f, "file.dat", b"payload");
            f.run(&["remote", "add", "origin", f.repository.to_str().unwrap()]);
            f.run(&[
                "config",
                "url.https://example.invalid/.insteadOf",
                "file:///",
            ]);
            // `env` resolves the endpoint but does not contact it. The real helper
            // applies URL aliases even to an explicitly supplied local LFS URL.
            let resolved = f.run(&["-c", "lfs.url=file:///C:/reviewed-fixture", "lfs", "env"]);
            assert!(
                String::from_utf8_lossy(&resolved)
                    .contains("https://example.invalid/C:/reviewed-fixture")
            );
            let before = snapshot(&f);
            assert!(matches!(
                f.repo().preview_lfs(
                    LfsAction::Fetch("origin".into()),
                    None,
                    &CancellationToken::new()
                ),
                Err(Error::Lfs(LfsFailure::Configuration))
            ));
            assert_eq!(snapshot(&f), before);
        },
    )
}
fn relay(f: &Fixture) -> PathBuf {
    let dir = f.root.join("helper");
    fs::create_dir(&dir).unwrap();
    let exe = dir.join("git.exe");
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/lfs_fixture_helper.rs");
    let result = Command::new("rustc")
        .args(["--edition=2024"])
        .arg(source)
        .arg("-o")
        .arg(&exe)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    fs::copy(&exe, dir.join("git-lfs.exe")).unwrap();
    exe
}
#[test]
fn running_fetch_can_be_cancelled_and_logs_omit_private_payloads() {
    case(
        "running_fetch_can_be_cancelled_and_logs_omit_private_payloads",
        || {
            let source = Fixture::new();
            entry(&source, "PRIVATE_文.dat", b"payload");
            let f = Fixture::new();
            f.run(&[
                "remote",
                "add",
                "origin",
                source.repository.to_str().unwrap(),
            ]);
            f.run(&["fetch", "origin"]);
            f.run(&["reset", "--hard", "origin/main"]);
            let exe = relay(&f);
            let marker = f.root.join("started");
            let gate = f.root.join("gate");
            // This test is already isolated into a single-test subprocess.
            unsafe {
                std::env::set_var("ROCKET_LFS_REAL_GIT", f.git.executable());
                std::env::set_var(
                    "ROCKET_LFS_REAL_HELPER",
                    f.git.executable().parent().unwrap().join("git-lfs.exe"),
                );
                std::env::set_var("ROCKET_LFS_STARTED", &marker);
                std::env::set_var("ROCKET_LFS_GATE", &gate);
            }
            let log = DiagnosticLog::open(f.root.join("logs")).unwrap();
            let client = GitClient::new_with_log(
                exe,
                CommandLimits {
                    timeout: Duration::from_secs(20),
                    ..Default::default()
                },
                log.clone(),
            )
            .unwrap();
            let c = CancellationToken::new();
            let repo = client.open(&f.repository, &c).unwrap();
            let p = repo
                .preview_lfs(LfsAction::Fetch("origin".into()), None, &c)
                .unwrap();
            let before = snapshot(&f);
            let task = WriteTask::spawn(move |cancel| repo.apply_lfs(&p, &cancel)).unwrap();
            let deadline = std::time::Instant::now() + Duration::from_secs(120);
            while !marker.exists() {
                assert!(std::time::Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(20));
            }
            task.cancel();
            let r = task.wait().unwrap();
            assert!(matches!(r.result, Err(Error::Cancelled)));
            assert_eq!(snapshot(&f), before);
            let text = fs::read_to_string(f.root.join("logs/rocket.jsonl")).unwrap();
            assert!(text.contains("download_lfs") && text.contains("cancelled"));
            assert!(
                !text.contains("PRIVATE")
                    && !text.contains("payload")
                    && !text.contains("Program Files")
            );
            assert_eq!(log.health().dropped_events, 0);
        },
    )
}
#[test]
fn redirected_cache_is_refused_without_touching_external_objects() {
    case(
        "redirected_cache_is_refused_without_touching_external_objects",
        || {
            let f = Fixture::new();
            let (_, object, pointer) = entry(&f, "file.dat", b"payload");
            let parent = object.parent().unwrap().canonicalize().unwrap();
            let outside = f.root.join("outside-cache");
            fs::rename(&parent, &outside).unwrap();
            let created = Command::new("cmd.exe")
                .args(["/d", "/c", "mklink", "/J"])
                .arg(&parent)
                .arg(outside.canonicalize().unwrap())
                .output()
                .unwrap();
            assert!(
                created.status.success(),
                "{}",
                String::from_utf8_lossy(&created.stderr)
            );
            let result = f.repo().lfs_status(&CancellationToken::new());
            // Remove this exact fixture junction, never its target directory.
            fs::remove_dir(&parent).unwrap();
            assert!(matches!(result, Err(Error::Lfs(LfsFailure::Layout))));
            assert_eq!(
                fs::read(outside.join(object.file_name().unwrap())).unwrap(),
                b"payload"
            );
            assert_eq!(fs::read(f.repository.join("file.dat")).unwrap(), pointer);
        },
    )
}

#[test]
fn sha256_unborn_readonly_and_size_limits_are_explicit() {
    case(
        "sha256_unborn_readonly_and_size_limits_are_explicit",
        || {
            let f = Fixture::with_format("sha256");
            let (path, _, _) = entry(&f, "file.dat", b"payload");
            let c = CancellationToken::new();
            let repo = f.repo();
            let before = snapshot(&f);
            let preview = repo
                .preview_lfs(LfsAction::Checkout(path), None, &c)
                .unwrap();
            let target = f.repository.join("file.dat");
            let original = fs::metadata(&target).unwrap().permissions();
            let mut readonly = original.clone();
            readonly.set_readonly(true);
            fs::set_permissions(&target, readonly).unwrap();
            let result = repo.apply_lfs(&preview, &c).unwrap();
            fs::set_permissions(&target, original).unwrap();
            assert!(matches!(result.result, Err(Error::Lfs(LfsFailure::Stale))));
            assert_eq!(result.completed_steps, 0);
            assert_eq!(snapshot(&f), before);
            let big = format!(
                "version https://git-lfs.github.com/spec/v1\noid sha256:{}\nsize {}\n",
                "a".repeat(64),
                LFS_FILE_LIMIT + 1
            );
            f.write("large.dat", &big);
            f.commit("oversize pointer");
            f.run(&["remote", "add", "origin", f.repository.to_str().unwrap()]);
            assert!(matches!(
                repo.preview_lfs(LfsAction::Fetch("origin".into()), None, &c),
                Err(Error::Lfs(LfsFailure::Limit))
            ));
            let empty = Fixture::new();
            let status = empty.repo().lfs_status(&c).unwrap();
            assert!(matches!(status.head, Head::Unborn { .. }));
            assert!(status.entries.is_empty());
        },
    )
}
