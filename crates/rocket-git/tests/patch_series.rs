mod common;
use common::{Fixture, isolated};
use rocket_git::*;
use std::fs;
fn c() -> CancellationToken {
    CancellationToken::new()
}
fn text(f: &Fixture, args: &[&str]) -> String {
    String::from_utf8(f.run(args)).unwrap().trim().into()
}
fn state(f: &Fixture) -> (Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>) {
    (
        f.run(&["rev-parse", "HEAD"]),
        fs::read(f.repository.join(".git/index")).unwrap(),
        f.run(&["--no-optional-locks", "status", "--porcelain=v2", "-z"]),
        fs::read(f.repository.join(".git/config")).unwrap(),
    )
}
fn range(f: &Fixture) -> String {
    f.write("keep.txt", "base\n");
    f.commit("base");
    let base = text(f, &["rev-parse", "HEAD"]);
    f.write("资源/旧.txt", "Unicode content\n");
    f.commit("Add Unicode 文");
    f.run(&["mv", "资源/旧.txt", "资源/新.txt"]);
    f.commit("Rename file");
    fs::write(f.repository.join("asset.bin"), [0, 1, 2, 255, 0, 42]).unwrap();
    f.commit("Binary update");
    base
}
#[test]
fn series_roundtrips_metadata_unicode_binary_and_preserves_local_state() {
    isolated(
        "ROCKET_PATCH_SERIES",
        "series_roundtrips_metadata_unicode_binary_and_preserves_local_state",
        || {
            for format in ["sha1", "sha256"] {
                let f = Fixture::with_format(format);
                let base = range(&f);
                f.run(&["config", "format.headers", "X-Private: PRIVATE_HEADER"]);
                f.run(&["config", "format.to", "PRIVATE_RECIPIENT"]);
                f.run(&["config", "format.signoff", "true"]);
                f.run(&["config", "format.attach", "PRIVATE_BOUNDARY"]);
                f.run(&["config", "format.useAutoBase", "true"]);
                f.run(&[
                    "config",
                    "format.signatureFile",
                    f.root.join("nonexistent-signature").to_str().unwrap(),
                ]);
                f.write("keep.txt", "staged\n");
                f.run(&["add", "keep.txt"]);
                f.write("keep.txt", "working\n");
                f.write("未跟踪.txt", "local only\n");
                let before = state(&f);
                let repo = f.repo();
                let output = f.root.canonicalize().unwrap().join("导出 系列");
                let p = repo
                    .preview_patch_series(&base, "HEAD", &output, &c())
                    .unwrap();
                assert_eq!(p.entries().len(), 3);
                assert!(p.entries()[0].subject.ends_with("文".as_bytes()));
                assert!(
                    p.entries()[1]
                        .paths
                        .iter()
                        .any(|p| p.as_bytes() == "资源/新.txt".as_bytes())
                );
                assert!(!output.exists());
                assert_eq!(state(&f), before);
                for e in p.entries() {
                    let s = String::from_utf8_lossy(e.patch());
                    assert!(!s.contains("PRIVATE_"));
                    assert!(!s.contains("Signed-off-by:"));
                }
                let r = repo.export_patch_series(&p, &c()).unwrap();
                assert!(r.result.is_ok(), "{:?}", r.result);
                assert!(r.published);
                assert!(r.retained.is_none());
                assert_eq!(fs::read_dir(&output).unwrap().count(), 3);
                assert_eq!(state(&f), before);
                assert_eq!(
                    fs::read_to_string(f.repository.join("keep.txt")).unwrap(),
                    "working\n"
                );
                let restored = f.root.join("restored");
                fs::create_dir(&restored).unwrap();
                assert!(
                    f.command(
                        &f.root,
                        &[
                            "clone",
                            "--no-hardlinks",
                            f.repository.to_str().unwrap(),
                            restored.to_str().unwrap()
                        ]
                    )
                    .output()
                    .unwrap()
                    .status
                    .success()
                );
                let run = |a: &[&str]| {
                    let o = f.command(&restored, a).output().unwrap();
                    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
                    o.stdout
                };
                run(&["config", "user.name", "Rocket Test"]);
                run(&["config", "user.email", "rocket@example.invalid"]);
                run(&["checkout", "--detach", &base]);
                for entry in p.entries() {
                    let file = output.join(&entry.filename);
                    assert_eq!(fs::read(&file).unwrap(), entry.patch());
                    run(&[
                        "-c",
                        "commit.gpgsign=false",
                        "am",
                        file.to_str().unwrap().trim_start_matches(r"\\?\"),
                    ]);
                }
                assert_eq!(
                    run(&["rev-parse", "HEAD^{tree}"]),
                    f.run(&["rev-parse", "HEAD^{tree}"])
                );
                let r = format!("{base}..HEAD");
                assert_eq!(
                    run(&[
                        "log",
                        "--reverse",
                        "--format=%an%x00%ae%x00%aI%x00%B%x00",
                        &r
                    ]),
                    f.run(&[
                        "log",
                        "--reverse",
                        "--format=%an%x00%ae%x00%aI%x00%B%x00",
                        &r
                    ])
                );
                assert_eq!(
                    fs::read(restored.join("asset.bin")).unwrap(),
                    [0, 1, 2, 255, 0, 42]
                );
            }
        },
    );
}
#[test]
fn series_refuses_merges_unrelated_and_empty_ranges() {
    isolated(
        "ROCKET_PATCH_SERIES",
        "series_refuses_merges_unrelated_and_empty_ranges",
        || {
            let f = Fixture::new();
            let base = range(&f);
            let repo = f.repo();
            let dest = f.root.canonicalize().unwrap().join("out");
            assert!(
                repo.preview_patch_series("HEAD", "HEAD", &dest, &c())
                    .unwrap_err()
                    .to_string()
                    .contains("patch_series_empty")
            );
            f.run(&["branch", "side", &base]);
            f.run(&["checkout", "side"]);
            f.write("other.txt", "side\n");
            f.commit("side");
            assert!(
                repo.preview_patch_series("main", "side", &dest, &c())
                    .unwrap_err()
                    .to_string()
                    .contains("patch_series_nonlinear")
            );
            f.run(&["checkout", "main"]);
            f.run(&["merge", "--no-edit", "side"]);
            assert!(
                repo.preview_patch_series(&base, "HEAD", &dest, &c())
                    .unwrap_err()
                    .to_string()
                    .contains("patch_series_nonlinear")
            );
            assert!(!dest.exists());
        },
    );
}
#[test]
fn series_rechecks_refs_and_never_replaces_existing_destinations() {
    isolated(
        "ROCKET_PATCH_SERIES",
        "series_rechecks_refs_and_never_replaces_existing_destinations",
        || {
            let f = Fixture::new();
            let base = range(&f);
            let repo = f.repo();
            let dest = f.root.canonicalize().unwrap().join("out");
            let p = repo
                .preview_patch_series(&base, "main", &dest, &c())
                .unwrap();
            f.write("new.txt", "new\n");
            f.commit("external commit");
            let saved = state(&f);
            let r = repo.export_patch_series(&p, &c()).unwrap();
            assert!(!r.published);
            assert!(
                r.result
                    .unwrap_err()
                    .to_string()
                    .contains("patch_series_changed")
            );
            assert!(!dest.exists());
            assert_eq!(state(&f), saved);
            let p = repo
                .preview_patch_series(&base, "main", &dest, &c())
                .unwrap();
            fs::create_dir(&dest).unwrap();
            fs::write(dest.join("sentinel"), b"keep").unwrap();
            let r = repo.export_patch_series(&p, &c()).unwrap();
            assert!(!r.published);
            assert!(r.result.is_err());
            assert_eq!(fs::read(dest.join("sentinel")).unwrap(), b"keep");
            assert_eq!(fs::read_dir(&dest).unwrap().count(), 1);
            assert!(
                repo.preview_patch_series(&base, "main", &dest, &c())
                    .is_err()
            );
            assert!(
                repo.preview_patch_series(&base, "main", &f.repository.join(".git/export"), &c())
                    .is_err()
            );
            assert!(
                repo.preview_patch_series(&base, "main", std::path::Path::new("relative"), &c())
                    .is_err()
            );
            assert!(
                repo.preview_patch_series(&base, "main", &f.root.join("missing/child"), &c())
                    .is_err()
            );
        },
    );
}
#[test]
fn series_cancellation_and_empty_commits_are_explicit() {
    isolated(
        "ROCKET_PATCH_SERIES",
        "series_cancellation_and_empty_commits_are_explicit",
        || {
            let f = Fixture::new();
            let base = range(&f);
            f.run(&["commit", "--allow-empty", "-m", "Keep empty commit"]);
            let repo = f.repo();
            let dest = f.root.canonicalize().unwrap().join("out");
            let saved = state(&f);
            let cancel = c();
            cancel.cancel();
            assert!(matches!(
                repo.preview_patch_series(&base, "HEAD", &dest, &cancel),
                Err(Error::Cancelled)
            ));
            let p = repo
                .preview_patch_series(&base, "HEAD", &dest, &c())
                .unwrap();
            assert_eq!(p.entries().len(), 4);
            assert!(p.entries()[3].paths.is_empty());
            assert!(String::from_utf8_lossy(p.entries()[3].patch()).contains("Keep empty commit"));
            assert!(matches!(
                repo.export_patch_series(&p, &cancel),
                Err(Error::Cancelled)
            ));
            assert!(!dest.exists());
            assert_eq!(state(&f), saved);
        },
    );
}

#[test]
fn series_limit_refuses_without_silently_truncating_the_range() {
    isolated(
        "ROCKET_PATCH_SERIES",
        "series_limit_refuses_without_silently_truncating_the_range",
        || {
            let f = Fixture::new();
            f.write("a.txt", "base\n");
            f.commit("base");
            let base = text(&f, &["rev-parse", "HEAD"]);
            for i in 0..65 {
                f.run(&["commit", "--allow-empty", "-m", &format!("Commit {i}")]);
            }
            let output = f.root.canonicalize().unwrap().join("too-many");
            let saved = state(&f);
            let error = f
                .repo()
                .preview_patch_series(&base, "HEAD", &output, &c())
                .unwrap_err();
            assert!(error.to_string().contains("patch_series_limit"), "{error}");
            assert!(!output.exists());
            assert_eq!(state(&f), saved);
        },
    );
}

#[test]
fn first_and_last_both_count_and_a_refusal_says_why() {
    isolated(
        "ROCKET_PATCH_SERIES",
        "first_and_last_both_count_and_a_refusal_says_why",
        || {
            let f = Fixture::new();
            let base = range(&f);
            let ids = text(&f, &["rev-list", "--reverse", &format!("{base}..HEAD")]);
            let ids: Vec<&str> = ids.lines().collect();
            let folder = f.root.canonicalize().unwrap().join("between");
            let repo = f.repo();
            let refused = |first: &str, last: &str| {
                repo.preview_patch_series_between(first, last, &folder, &c()).err().map(|e| e.to_string()).unwrap_or_default()
            };
            let all = repo.preview_patch_series_between(&ids[0][..7], "HEAD", &folder, &c()).unwrap();
            assert_eq!(all.entries().iter().map(|e| e.id.as_str()).collect::<Vec<_>>(), ids);
            assert_eq!(all.base().as_str(), base);
            let one = repo.preview_patch_series_between(ids[1], ids[1], &folder, &c()).unwrap();
            assert_eq!(one.entries().len(), 1, "First and Last can be the same commit");
            assert!(refused(&base, "HEAD").contains("patch_series_first_commit"));
            assert!(refused("not-a-commit", "HEAD").contains("patch_series_unknown"));
            assert!(refused(ids[0], "not-a-commit").contains("Rocket can't find not-a-commit"));
            assert!(refused(ids[2], ids[0]).contains("patch_series_empty"), "First newer than Last");
            f.run(&["switch", "-c", "side", ids[0]]);
            f.write("side.txt", "side\n");
            f.commit("Side");
            f.run(&["switch", "-"]);
            f.run(&["merge", "--no-ff", "-m", "Merge side", "side"]);
            let series = refused(ids[0], "HEAD");
            assert!(series.contains("patch_series_nonlinear") && series.contains("can't make a series\n"), "{series}");
        },
    );
}

#[test]
fn a_first_commit_on_another_line_is_refused_not_left_out() {
    isolated(
        "ROCKET_PATCH_SERIES",
        "a_first_commit_on_another_line_is_refused_not_left_out",
        || {
            let f = Fixture::new();
            f.write("base.txt", "base\n");
            f.commit("base");
            f.run(&["switch", "-c", "side"]);
            f.write("side.txt", "side\n");
            f.commit("Side");
            let first = text(&f, &["rev-parse", "HEAD"]);
            f.run(&["switch", "main"]);
            f.write("main.txt", "main\n");
            f.commit("Main");
            let folder = f.root.canonicalize().unwrap().join("between");
            let refused = f.repo().preview_patch_series_between(&first, "main", &folder, &c()).err().unwrap().to_string();
            assert!(refused.contains("patch_series_nonlinear") && refused.contains("First isn't on the same line"), "{refused}");
        },
    );
}
