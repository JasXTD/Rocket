#![cfg(windows)]
//! Repository previews: a bare clone without file contents in a cache folder, the
//! explicit downloads registered previews make for their reads, the size limit, Keep
//! and Discard. The source is a local repository served with partial-clone support.
#[allow(dead_code)]
mod common;
use common::{Fixture, isolated};
use rocket_git::*;
use std::{fs, path::Path, sync::mpsc};

#[test]
fn keep_refuses_an_oversized_missing_object_graph_before_fetch_or_publication() {
    run_case(
        "keep_refuses_an_oversized_missing_object_graph_before_fetch_or_publication",
        || {
            use std::{io::Write, process::Stdio};
            let f = Fixture::new();
            let bare = f.root.join("oversized.git");
            f.run(&["init", "--bare", bare.to_str().unwrap()]);
            // Real Git tree/commit objects reference 100,001 absent blobs. No server,
            // large blob contents or a remote fetch is needed to exercise enumeration.
            let mut tree = f
                .command(&bare, &["mktree", "--missing"])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            {
                let mut input = std::io::BufWriter::new(tree.stdin.take().unwrap());
                for n in 1..=100_001 {
                    writeln!(input, "100644 blob {n:040x}\tf{n:06}").unwrap();
                }
                input.flush().unwrap();
            }
            let output = tree.wait_with_output().unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            let tree_id = String::from_utf8(output.stdout).unwrap();
            // Use the source fixture's synthetic identity for this bare commit.
            let commit = f.run_at(
                &bare,
                &[
                    "-c",
                    "user.name=Rocket Test",
                    "-c",
                    "user.email=rocket@example.invalid",
                    "commit-tree",
                    tree_id.trim(),
                    "-m",
                    "bounded fixture",
                ],
            );
            let commit = String::from_utf8(commit).unwrap();
            f.run_at(&bare, &["update-ref", "HEAD", commit.trim()]);
            let before = fs::read(bare.join("config")).unwrap();
            let destination = f.root.join("must-not-publish");
            let error = f
                .git
                .keep_preview(
                    &bare,
                    "https://example.invalid/unused",
                    &destination,
                    None,
                    &CancellationToken::new(),
                )
                .expect_err("oversized preview must fail");
            assert!(
                matches!(&error, Error::Unsupported(message) if message.contains("100,000 missing objects")),
                "wrong failure: {error}"
            );
            assert!(!destination.exists());
            assert_eq!(fs::read(bare.join("config")).unwrap(), before);
            assert_eq!(
                f.run_at(&bare, &["rev-parse", "HEAD"]),
                format!("{}\n", commit.trim()).into_bytes()
            );
            assert!(!bare.join("FETCH_HEAD").exists());
        },
    );
}

fn run_case(name: &str, run: fn()) {
    isolated("ROCKET_PREVIEW_CASE", name, run);
}

/// Three commits on main, one on a side branch; .mailmap and a nested file.
fn source() -> Fixture {
    let f = Fixture::new();
    f.write("a.txt", "one\n");
    f.write("dir/b.txt", "bee\n");
    f.write(".mailmap", "Rocket Tester <rocket@example.invalid> Rocket Test <rocket@example.invalid>\n");
    f.commit("first");
    f.write("a.txt", "one\ntwo\n");
    f.commit("second");
    f.run(&["branch", "side"]);
    f.write("dir/b.txt", "bee\nbuzz\n");
    f.commit("third");
    f.run(&["config", "uploadpack.allowFilter", "true"]);
    f
}

fn missing(f: &Fixture, repository: &Path, revision: &str) -> usize {
    let out = f.run_at(
        repository,
        &["--no-lazy-fetch", "rev-list", "--objects", "--missing=print", "--no-object-names", revision],
    );
    out.split(|b| *b == b'\n').filter(|l| l.starts_with(b"?")).count()
}

fn opened(f: &Fixture, limit: Option<u64>) -> (PreviewCache, PreviewSlot, std::sync::Arc<PreviewDownloads>, Result<PreviewOpened>) {
    let cache = PreviewCache::open(f.root.join("previews")).unwrap();
    let slot = cache.reserve().unwrap();
    let meter = PreviewDownloads::new(limit);
    let (tx, rx) = mpsc::sync_channel(256);
    let result = f.git.open_preview(
        &CloneSource::Local(f.repository.clone()),
        &slot,
        limit,
        &meter,
        Some(&tx),
        &CancellationToken::new(),
    );
    drop(tx);
    let events: Vec<_> = rx.try_iter().collect();
    if result.is_ok() {
        assert!(events.contains(&CloneProgress::Phase(ClonePhase::DownloadingFiles)), "{events:?}");
        assert!(events.iter().any(|e| matches!(e, CloneProgress::Files { .. })), "{events:?}");
    }
    (cache, slot, meter, result)
}

#[test]
fn preview_is_bare_without_old_versions_and_downloads_only_when_registered() {
    run_case(
        "preview_is_bare_without_old_versions_and_downloads_only_when_registered",
        || {
            let f = source();
            let (cache, slot, meter, result) = opened(&f, Some(PREVIEW_LIMIT));
            let preview = result.unwrap();
            assert_eq!(preview.branch.as_deref(), Some("main"));
            assert_eq!(preview.repository, slot.repository());
            assert!(preview.size > 0 && meter.status().size > 0);
            let repository = &preview.repository;
            assert!(!repository.join("index").exists() && repository.join("HEAD").is_file());
            assert_eq!(f.run_at(repository, &["rev-parse", "--is-bare-repository"]), b"true\n");
            // The files of main are here; older versions are not.
            assert_eq!(missing(&f, repository, "HEAD^{tree}"), 0);
            assert!(missing(&f, repository, "--all") > 0);
            assert!(fs::read_dir(repository.join("hooks")).map(|d| d.count()).unwrap_or(0) == 0);

            let second = String::from_utf8(f.run(&["rev-parse", "HEAD~1"])).unwrap();
            let second = ObjectId::parse(second.trim()).unwrap();
            let diff = |repo: &Repository| {
                repo.diff(
                    &DiffOptions { target: DiffTarget::Commit { id: second.clone() }, ..Default::default() },
                    &CancellationToken::new(),
                )
            };
            // Not registered: Rocket never lets Git fetch behind a read.
            let plain = f.git.open(repository, &CancellationToken::new()).unwrap();
            assert!(!plain.is_preview());
            assert!(diff(&plain).is_err());
            // Registered: the read downloads exactly what it needs first.
            f.git.register_preview(repository, meter.clone()).unwrap();
            let repo = f.git.open(repository, &CancellationToken::new()).unwrap();
            assert!(repo.is_preview());
            let patch = String::from_utf8(diff(&repo).unwrap().patch).unwrap();
            assert!(patch.contains("+two"), "{patch}");
            assert!(missing(&f, repository, "--all") > 0, "only that commit's files came");
            let side = repo
                .compare_revisions("side", "main", &CancellationToken::new())
                .unwrap();
            assert!(!side.files.is_empty());
            assert!(!meter.status().running);
            f.git.unregister_preview(repository);
            assert!(!f.git.open(repository, &CancellationToken::new()).unwrap().is_preview());
            let folder = slot.directory().to_path_buf();
            slot.discard().unwrap();
            assert!(!folder.exists());
            assert_eq!(cache.remove_stale(), 0);
        },
    );
}

#[test]
fn downloads_report_what_arrived_and_remember_a_failure() {
    run_case("downloads_report_what_arrived_and_remember_a_failure", || {
        let f = source();
        let (_cache, slot, meter, result) = opened(&f, Some(PREVIEW_LIMIT));
        let repository = result.unwrap().repository;
        f.git.register_preview(&repository, meter.clone()).unwrap();
        let repo = f.git.open(&repository, &CancellationToken::new()).unwrap();
        let commit = |revision: &str| {
            let id = String::from_utf8(f.run(&["rev-parse", revision])).unwrap();
            ObjectId::parse(id.trim()).unwrap()
        };
        let diff = |id: ObjectId| {
            repo.diff(
                &DiffOptions { target: DiffTarget::Commit { id }, ..Default::default() },
                &CancellationToken::new(),
            )
        };
        // With the source gone, the download fails and the meter says why.
        let moved = f.root.join("moved-away");
        fs::rename(&f.repository, &moved).unwrap();
        assert!(diff(commit_at(&moved, "HEAD~1")).is_err());
        let status = meter.status();
        assert!(!status.running && status.files == 0 && status.received == 0 && status.downloaded == 0);
        assert!(status.failure.is_some(), "{status:?}");
        // Back again: the next download clears it and everything returns to rest.
        fs::rename(&moved, &f.repository).unwrap();
        let patch = String::from_utf8(diff(commit("HEAD~1")).unwrap().patch).unwrap();
        assert!(patch.contains("+two"), "{patch}");
        let status = meter.status();
        assert_eq!((status.running, status.files, status.received, status.downloaded, status.failure), (false, 0, 0, 0, None));
        assert!(status.size > 0);
        f.git.unregister_preview(&repository);
        slot.discard().unwrap();
    });
}

/// `revision`'s commit in the repository at `path`.
fn commit_at(path: &Path, revision: &str) -> ObjectId {
    let output = std::process::Command::new("git").args(["-C", path.to_str().unwrap(), "rev-parse", revision]).output().unwrap();
    ObjectId::parse(String::from_utf8(output.stdout).unwrap().trim()).unwrap()
}

#[test]
fn preview_limit_stops_and_keeps_nothing() {
    run_case("preview_limit_stops_and_keeps_nothing", || {
        let f = source();
        let (_cache, slot, _meter, result) = opened(&f, Some(1));
        let error = result.unwrap_err();
        assert_eq!(error.clone_failure(), Some(CloneFailure::SizeLimit), "{error}");
        assert!(!slot.repository().exists());
        slot.discard().unwrap();
    });
}

#[test]
fn keep_makes_a_normal_clone_pointing_at_the_original_address() {
    run_case("keep_makes_a_normal_clone_pointing_at_the_original_address", || {
        let f = source();
        let (_cache, slot, _meter, result) = opened(&f, None);
        let preview = result.unwrap();
        let destination = f.root.join("kept");
        let url = "https://example.invalid/team/tool.git";
        let (tx, rx) = mpsc::sync_channel(256);
        let kept = f
            .git
            .keep_preview(&preview.repository, url, &destination, Some(&tx), &CancellationToken::new())
            .unwrap();
        drop(tx);
        assert!(matches!(kept.clone.publication, Some(Ok(()))), "{:?}", kept.clone.publication);
        assert!(matches!(kept.origin, Some(Ok(()))), "{:?}", kept.origin);
        let events: Vec<_> = rx.try_iter().collect();
        assert!(events.contains(&CloneProgress::Phase(ClonePhase::DownloadingFiles)));
        assert_eq!(f.run_at(&destination, &["remote", "get-url", "origin"]), format!("{url}\n").into_bytes());
        assert_eq!(f.run_at(&destination, &["rev-parse", "--abbrev-ref", "@{upstream}"]), b"origin/main\n");
        assert_eq!(missing(&f, &destination, "--all"), 0);
        assert!(f.run_at(&destination, &["status", "--porcelain"]).is_empty());
        assert!(f.output(&destination, &["config", "--get", "remote.origin.promisor"]).stdout.is_empty());
        assert_eq!(fs::read(destination.join("dir/b.txt")).unwrap(), b"bee\nbuzz\n");
        // The preview stays usable until it is discarded.
        assert_eq!(missing(&f, &preview.repository, "--all"), 0);
        slot.discard().unwrap();
    });
}
