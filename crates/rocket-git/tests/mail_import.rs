mod common;
use common::{Fixture, isolated};
use rocket_git::*;
use std::{fs, path::PathBuf};
fn oid(f: &Fixture) -> String {
    String::from_utf8(f.run(&["rev-parse", "HEAD"]))
        .unwrap()
        .trim()
        .into()
}
fn setup(conflict: bool) -> (Fixture, PathBuf, String) {
    let f = Fixture::new();
    f.write("資料.txt", "base\n");
    f.commit("base");
    let base = oid(&f);
    f.write("資料.txt", "series\n");
    f.commit("Series 文");
    f.write("later.txt", "second\n");
    f.commit("Second");
    let source = f.root.canonicalize().unwrap().join("シリーズ");
    let p = f
        .repo()
        .preview_patch_series(&base, "HEAD", &source, &CancellationToken::new())
        .unwrap();
    f.repo()
        .export_patch_series(&p, &CancellationToken::new())
        .unwrap();
    f.run(&["reset", "--hard", &base]);
    if conflict {
        f.write("資料.txt", "other\n");
        f.commit("Diverged");
    }
    let original = oid(&f);
    (f, source, original)
}

#[test]
fn security_display_bounds_oversized_mail_subject() {
    isolated("ROCKET_MAIL_IMPORT", "security_display_bounds_oversized_mail_subject", || {
        let (f, folder, _) = setup(false);
        let c = CancellationToken::new();
        let initial = f.repo().preview_mail_import(&folder, &c).unwrap();
        let source = folder.join(&initial.entries()[0].filename);
        let raw = fs::read_to_string(&source).unwrap();
        let header = raw.lines().find(|l| l.starts_with("Subject: ")).unwrap();
        fs::write(&source, raw.replacen(header, &format!("Subject: {}", "x".repeat(1024 * 1024)), 1)).unwrap();
        let mut preview = f.repo().preview_mail_import(&folder, &c).unwrap();
        assert!(preview.entries()[0].subject.len() <= 512, "subject must have a fixed display budget");
        assert!(preview.requires_truncation_acknowledgement());
        assert!(!preview.truncation_acknowledged());
        assert!(f.repo().apply_mail_import(&preview, &c).unwrap_err().to_string().contains("series_review_incomplete"));
        assert!(!f.repository.join(".git/rocket-mail-import").exists());
        let reviewed_bytes = fs::read(&source).unwrap();
        preview.acknowledge_truncation(true);
        fs::write(&source, [reviewed_bytes.as_slice(), b"\n"].concat()).unwrap();
        assert!(f.repo().apply_mail_import(&preview, &c).unwrap_err().to_string().contains("series_source_changed"));
        fs::write(&source, &reviewed_bytes).unwrap();
        let refreshed = f.repo().preview_mail_import(&folder, &c).unwrap();
        assert!(!refreshed.truncation_acknowledged());
        preview.acknowledge_truncation(false);
        assert!(f.repo().apply_mail_import(&preview, &c).unwrap_err().to_string().contains("series_review_incomplete"));
        preview.acknowledge_truncation(true);
        let report = f.repo().apply_mail_import(&preview, &c).unwrap();
        assert!(report.write.result.is_ok(), "{report:?}");
        let imported = f.run(&["log", "-1", "--format=%s", "HEAD~1"]);
        assert_eq!(imported.len(), 1024 * 1024 + 1, "the original subject is imported unchanged");
    });
}

#[test]
fn security_display_escapes_preserve_real_mail_and_paths() {
    isolated("ROCKET_MAIL_IMPORT", "security_display_escapes_preserve_real_mail_and_paths", || {
        let f = Fixture::new();
        let c = CancellationToken::new();
        f.write("base", "base\n");
        f.commit("Base");
        let base = oid(&f);
        let name = "safe\u{202e}name.txt";
        let subject = "Keep 文 \u{2066}visible\u{2069}";
        f.write(name, "+content\t\u{61c}hidden\r\n");
        f.run(&["add", "--", name]);
        f.run(&["commit", "-m", subject, "--author=Author \u{202e}name <fixture@example.invalid>"]);
        let expected = f.run(&["log", "-1", "--format=%an%x00%B"]);
        let folder = f.root.canonicalize().unwrap().join("escaped-mail");
        let export = f.repo().preview_patch_series(&base, "HEAD", &folder, &c).unwrap();
        f.repo().export_patch_series(&export, &c).unwrap();
        f.run(&["reset", "--hard", &base]);
        let preview = f.repo().preview_mail_import(&folder, &c).unwrap();
        let entry = &preview.entries()[0];
        assert!(entry.subject.contains("\\u{2066}"));
        assert!(entry.author.contains("\\u{202e}"));
        assert!(entry.display.paths[0].contains("\\u{202e}"));
        assert!(entry.display.patch.contains("\\u{61c}"));
        assert!(entry.display.escaped && !entry.display.limited);
        assert!(!preview.requires_truncation_acknowledgement());
        let report = f.repo().apply_mail_import(&preview, &c).unwrap();
        assert!(report.write.result.is_ok(), "{report:?}");
        assert_eq!(f.run(&["log", "-1", "--format=%an%x00%B"]), expected);
        assert!(f.repository.join(name).is_file());
    });
}

#[test]
fn security_display_hidden_patch_content_requires_review_but_secrets_still_refuse() {
    isolated("ROCKET_MAIL_IMPORT", "security_display_hidden_patch_content_requires_review_but_secrets_still_refuse", || {
        let f = Fixture::new();
        let c = CancellationToken::new();
        f.write("base", "base\n");
        f.commit("Base");
        let base = oid(&f);
        f.write("many-lines", &("ordinary line\n".repeat(200) + "tail-only-review-marker\n"));
        f.write("long-line", "z".repeat(128 * 1024));
        f.commit("Long patch");
        let folder = f.root.canonicalize().unwrap().join("limited-mail");
        let export = f.repo().preview_patch_series(&base, "HEAD", &folder, &c).unwrap();
        f.repo().export_patch_series(&export, &c).unwrap();
        f.run(&["reset", "--hard", &base]);
        let mut preview = f.repo().preview_mail_import(&folder, &c).unwrap();
        let entry = &preview.entries()[0];
        assert!(entry.display.limited);
        assert!(entry.display.patch.len() <= 16 * 1024);
        assert!(entry.display.shown_lines <= 160);
        assert!(entry.display.total_lines > 160);
        assert!(preview.requires_truncation_acknowledgement());
        assert!(f.repo().apply_mail_import(&preview, &c).unwrap_err().to_string().contains("series_review_incomplete"));
        preview.acknowledge_truncation(true);
        let report = f.repo().apply_mail_import(&preview, &c).unwrap();
        assert!(report.write.result.is_ok(), "{report:?}");
        assert!(fs::read_to_string(f.repository.join("many-lines")).unwrap().ends_with("tail-only-review-marker\n"));
        f.run(&["reset", "--hard", &base]);
        let source = folder.join(&preview.entries()[0].filename);
        let raw = fs::read_to_string(&source).unwrap();
        let header = raw.lines().find(|l| l.starts_with("Subject: ")).unwrap();
        let token = format!("{}{}_{}", "g", "hp", "Ab1Cd2".repeat(6));
        fs::write(&source, raw.replacen(header, &format!("Subject: {} {token}", "x".repeat(2048)), 1)).unwrap();
        assert!(matches!(f.repo().preview_mail_import(&folder, &c), Err(Error::SecretProtection(_))), "raw metadata beyond the display cap must still be scanned");
        assert_eq!(oid(&f), base);
    });
}

#[test]
fn security_display_recovery_needs_fresh_acknowledgement_but_abort_stays_available() {
    isolated("ROCKET_MAIL_IMPORT", "security_display_recovery_needs_fresh_acknowledgement_but_abort_stays_available", || {
        let (f, folder, original) = setup(true);
        let c = CancellationToken::new();
        let initial = f.repo().preview_mail_import(&folder, &c).unwrap();
        let source = folder.join(&initial.entries()[0].filename);
        let raw = fs::read_to_string(&source).unwrap();
        let header = raw.lines().find(|l| l.starts_with("Subject: ")).unwrap();
        fs::write(&source, raw.replacen(header, &format!("Subject: {}", "x".repeat(2048)), 1)).unwrap();
        let mut preview = f.repo().preview_mail_import(&folder, &c).unwrap();
        preview.acknowledge_truncation(true);
        let report = f.repo().apply_mail_import(&preview, &c).unwrap();
        assert!(report.state.unwrap().mail_import);
        f.write("資料.txt", "resolved\n");
        f.run(&["add", "資料.txt"]);
        let recovery = f.repo().preview_mail_recovery(MailImportAction::Continue, &c).unwrap();
        assert!(recovery.requires_truncation_acknowledgement());
        assert!(!recovery.truncation_acknowledged());
        assert!(f.repo().apply_mail_import(&recovery, &c).unwrap_err().to_string().contains("series_review_incomplete"));
        let abort = f.repo().preview_mail_recovery(MailImportAction::Abort, &c).unwrap();
        assert!(!abort.requires_truncation_acknowledgement());
        assert!(f.repo().apply_mail_import(&abort, &c).unwrap().write.result.is_ok());
        assert_eq!(oid(&f), original);
    });
}
#[test]
fn import_preserves_metadata_unicode_and_refuses_stale_input_or_dirty_checkout() {
    isolated(
        "ROCKET_MAIL_IMPORT",
        "import_preserves_metadata_unicode_and_refuses_stale_input_or_dirty_checkout",
        || {
            let c = CancellationToken::new();
            let (f, folder, original) = setup(false);
            let p = f.repo().preview_mail_import(&folder, &c).unwrap();
            assert_eq!(p.entries().len(), 2);
            assert!(p.entries()[0].subject.contains('文'));
            let source = folder.join(&p.entries()[0].filename);
            let bytes = fs::read(&source).unwrap();
            let mut changed = bytes.clone();
            changed.extend(b"\n");
            fs::write(&source, &changed).unwrap();
            assert!(
                f.repo()
                    .apply_mail_import(&p, &c)
                    .unwrap_err()
                    .to_string()
                    .contains("series_source_changed")
            );
            assert_eq!(oid(&f), original);
            fs::write(source, bytes).unwrap();
            f.write("資料.txt", "local\n");
            assert!(
                f.repo()
                    .preview_mail_import(&folder, &c)
                    .unwrap_err()
                    .to_string()
                    .contains("series_dirty")
            );
            f.run(&["restore", "--", "資料.txt"]);
            let p = f.repo().preview_mail_import(&folder, &c).unwrap();
            let stopped = CancellationToken::new();
            stopped.cancel();
            assert!(f.repo().apply_mail_import(&p, &stopped).is_err());
            assert_eq!(oid(&f), original);
            f.write("untracked.txt", "keep\n");
            let p = f.repo().preview_mail_import(&folder, &c).unwrap();
            let r = f.repo().apply_mail_import(&p, &c).unwrap();
            assert!(r.write.result.is_ok(), "{r:?}");
            assert!(r.journal_result.is_ok(), "{r:?}");
            assert!(r.state.unwrap().is_idle());
            assert_ne!(oid(&f), original);
            assert_eq!(
                fs::read(f.repository.join("資料.txt")).unwrap(),
                b"series\n"
            );
            assert_eq!(
                fs::read(f.repository.join("untracked.txt")).unwrap(),
                b"keep\n"
            );
            assert_eq!(
                String::from_utf8(f.run(&["rev-list", "--count", &format!("{original}..HEAD")]))
                    .unwrap()
                    .trim(),
                "2"
            );
        },
    );
}
#[test]
fn conflict_continue_skip_abort_protect_resolutions_and_recovery_state() {
    isolated(
        "ROCKET_MAIL_IMPORT",
        "conflict_continue_skip_abort_protect_resolutions_and_recovery_state",
        || {
            for action in [
                MailImportAction::Continue,
                MailImportAction::Skip,
                MailImportAction::Abort,
            ] {
                let c = CancellationToken::new();
                let (f, folder, original) = setup(true);
                let p = f.repo().preview_mail_import(&folder, &c).unwrap();
                let r = f.repo().apply_mail_import(&p, &c).unwrap();
                assert!(r.write.result.is_err(), "{r:?}");
                assert!(r.journal_result.is_ok(), "{r:?}");
                let state = r.state.unwrap();
                assert!(state.mail_import);
                assert!(!state.rebase);
                assert!(!state.is_idle());
                if action == MailImportAction::Continue {
                    f.write("資料.txt", "resolved 文\n");
                    f.run(&["add", "--", "資料.txt"]);
                }
                let metadata = f.repository.join(".git/rebase-apply/apply-opt");
                let old = fs::read(&metadata).ok();
                fs::write(&metadata, b"--unsafe-paths\n").unwrap();
                assert!(
                    f.repo()
                        .preview_mail_recovery(action, &c)
                        .unwrap_err()
                        .to_string()
                        .contains("series_state_changed")
                );
                if let Some(old) = old {
                    fs::write(&metadata, old).unwrap();
                } else {
                    fs::remove_file(&metadata).unwrap();
                }
                let p = f.repo().preview_mail_recovery(action, &c).unwrap();
                assert_eq!(p.current(), 1);
                if action == MailImportAction::Abort {
                    f.write("資料.txt", "late edit\n");
                    assert!(
                        f.repo()
                            .apply_mail_import(&p, &c)
                            .unwrap_err()
                            .to_string()
                            .contains("series_stale")
                    );
                }
                let p = f.repo().preview_mail_recovery(action, &c).unwrap();
                let r = f.repo().apply_mail_import(&p, &c).unwrap();
                assert!(r.write.result.is_ok(), "{r:?}");
                assert!(r.journal_result.is_ok(), "{r:?}");
                assert!(r.state.unwrap().is_idle());
                if action == MailImportAction::Abort {
                    assert_eq!(oid(&f), original);
                    let backup = r.backup.unwrap();
                    assert!(backup.join("manifest.log").is_file());
                    assert!(fs::read_dir(&backup).unwrap().any(|e| {
                        fs::read(e.unwrap().path()).ok().as_deref() == Some(b"late edit\n")
                    }));
                } else {
                    assert!(f.repository.join("later.txt").is_file());
                }
            }
        },
    );
}

#[test]
fn empty_commits_rename_modes_and_author_metadata_round_trip() {
    isolated(
        "ROCKET_MAIL_IMPORT",
        "empty_commits_rename_modes_and_author_metadata_round_trip",
        || {
            let f = Fixture::new();
            let c = CancellationToken::new();
            f.write("旧名.txt", "one\n");
            f.commit("base");
            let base = oid(&f);
            f.run(&["mv", "旧名.txt", "新名 🚀.txt"]);
            f.run(&["update-index", "--chmod=+x", "新名 🚀.txt"]);
            f.run(&[
                "commit",
                "-m",
                "Rename 文\n\nDetailed body.",
                "--author=作者 <synthetic@example.invalid>",
            ]);
            f.run(&["commit", "--allow-empty", "-m", "Empty 文"]);
            let expected = f.run(&[
                "log",
                "--reverse",
                "--format=%an%x00%ae%x00%aI%x00%B%x00",
                &format!("{base}..HEAD"),
            ]);
            let tree = f.run(&["rev-parse", "HEAD^{tree}"]);
            let folder = f.root.canonicalize().unwrap().join("mail");
            let export = f
                .repo()
                .preview_patch_series(&base, "HEAD", &folder, &c)
                .unwrap();
            f.repo().export_patch_series(&export, &c).unwrap();
            f.run(&["reset", "--hard", &base]);
            let p = f.repo().preview_mail_import(&folder, &c).unwrap();
            assert_eq!(p.entries().len(), 2);
            assert!(p.entries()[1].paths.is_empty());
            let r = f.repo().apply_mail_import(&p, &c).unwrap();
            assert!(r.write.result.is_ok(), "{r:?}");
            assert!(r.journal_result.is_ok());
            assert_eq!(
                f.run(&[
                    "log",
                    "--reverse",
                    "--format=%an%x00%ae%x00%aI%x00%B%x00",
                    &format!("{base}..HEAD")
                ]),
                expected
            );
            assert_eq!(f.run(&["rev-parse", "HEAD^{tree}"]), tree);
            assert!(f.repository.join("新名 🚀.txt").is_file());
        },
    );
}

#[test]
fn enabling_secret_protection_during_import_still_allows_abort() {
    isolated(
        "ROCKET_MAIL_IMPORT",
        "enabling_secret_protection_during_import_still_allows_abort",
        || {
            let f = Fixture::new();
            let c = CancellationToken::new();
            f.write("conflict", "base\n");
            f.commit("base");
            let base = oid(&f);
            f.write("conflict", "mail\n");
            f.commit("mail");
            f.write(".env", "PASSWORD=synthetic_fixture_only\n");
            f.commit("Fixture environment file");
            let folder = f.root.canonicalize().unwrap().join("mail");
            let export = f
                .repo()
                .preview_patch_series(&base, "HEAD", &folder, &c)
                .unwrap();
            f.repo().export_patch_series(&export, &c).unwrap();
            f.run(&["reset", "--hard", &base]);
            f.write("conflict", "upstream\n");
            f.commit("upstream");
            let original = oid(&f);
            f.git.set_secret_protection(false);
            let p = f.repo().preview_mail_import(&folder, &c).unwrap();
            let r = f.repo().apply_mail_import(&p, &c).unwrap();
            assert!(r.state.unwrap().mail_import);
            f.git.set_secret_protection(true);
            f.write("conflict", "resolved\n");
            f.run(&["add", "conflict"]);
            let p = f
                .repo()
                .preview_mail_recovery(MailImportAction::Continue, &c)
                .unwrap();
            assert!(f.repo().apply_mail_import(&p, &c).is_err());
            let p = f
                .repo()
                .preview_mail_recovery(MailImportAction::Abort, &c)
                .unwrap();
            let r = f.repo().apply_mail_import(&p, &c).unwrap();
            assert!(r.write.result.is_ok(), "{r:?}");
            assert_eq!(oid(&f), original);
            assert!(!f.repository.join(".env").exists());
        },
    );
}

#[test]
fn a_stopped_import_says_where_it_stopped_and_what_continue_needs() {
    isolated("ROCKET_MAIL_IMPORT", "a_stopped_import_says_where_it_stopped_and_what_continue_needs", || {
        let (f, folder, _) = setup(true);
        let c = CancellationToken::new();
        let review = f.repo().preview_mail_import(&folder, &c).unwrap();
        assert_eq!(review.entries()[0].source.len(), 7, "the commit the patch came from");
        assert!(review.entries()[0].size() > review.entries()[0].patch.len(), "the whole file, not only its diff");
        assert!(review.entries()[1].display.added > 0);
        f.repo().apply_mail_import(&review, &c).unwrap();
        let step = f.repo().operation_state(&c).unwrap().step.unwrap();
        assert_eq!((step.at, step.total), (1, 2));
        assert_eq!(step.subject, "Series 文");
        assert_eq!(step.branch, None, "an import stays on its branch");
        let refused = f.repo().preview_mail_recovery(MailImportAction::Continue, &c).err().unwrap().to_string();
        assert!(refused.contains("series_unresolved") && refused.contains("資料.txt still has a conflict\n"), "{refused}");
        f.write("資料.txt", "fixed\n");
        let refused = f.repo().preview_mail_recovery(MailImportAction::Continue, &c).err().unwrap().to_string();
        assert!(refused.contains("still has a conflict") || refused.contains("aren't staged"), "{refused}");
        f.run(&["add", "資料.txt"]);
        let review = f.repo().preview_mail_recovery(MailImportAction::Continue, &c).unwrap();
        assert_eq!(review.current(), 1);
        assert!(String::from_utf8_lossy(review.staged()).contains("+fixed"));
        let skip = f.repo().preview_mail_recovery(MailImportAction::Skip, &c).unwrap();
        assert!(String::from_utf8_lossy(skip.staged()).contains("+fixed"), "Skip names the edits that go");
        assert_eq!(skip.changes(), [("資料.txt".to_owned(), false)]);
        let abort = f.repo().preview_mail_recovery(MailImportAction::Abort, &c).unwrap();
        assert!(abort.imported().is_empty(), "patch 1 hasn't made a commit yet");
        f.run(&["checkout", "HEAD", "--", "資料.txt"]);
        let refused = f.repo().preview_mail_recovery(MailImportAction::Continue, &c).err().unwrap().to_string();
        assert!(refused.contains("Nothing to save for this patch"), "git am --continue would refuse this too: {refused}");
    });
}
