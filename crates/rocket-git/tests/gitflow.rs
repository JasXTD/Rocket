mod common;
use common::{Fixture, isolated};
use rocket_git::*;
use std::fmt::Debug;

fn token() -> CancellationToken {
    CancellationToken::new()
}
fn text(bytes: Vec<u8>) -> String {
    String::from_utf8(bytes).unwrap().trim_end().to_owned()
}
fn rev(f: &Fixture, revision: &str) -> String {
    text(f.run(&["rev-parse", revision]))
}
fn head(f: &Fixture) -> String {
    text(f.run(&["symbolic-ref", "--short", "HEAD"]))
}
fn exists(f: &Fixture, reference: &str) -> bool {
    f.output(&f.repository, &["show-ref", "--exists", reference])
        .status
        .success()
}
fn config_get(f: &Fixture, key: &str) -> String {
    text(f.run(&["config", "--get", key]))
}
fn failure<T: Debug>(result: Result<T>) -> GitFlowFailure {
    match result {
        Err(Error::GitFlow(failure)) => failure,
        other => panic!("expected a git-flow refusal: {other:?}"),
    }
}
/// main with one commit and git-flow set up with its defaults and a `v` tag prefix.
fn flow(format: &str) -> Fixture {
    let f = Fixture::with_format(format);
    f.write("app.txt", "base\n");
    f.commit("PRIVATE_BASE");
    let repo = f.repo();
    let mut config = GitFlowConfig::defaults("main");
    config.version_tag = "v".into();
    let preview = repo.preview_gitflow_init(&config, &token()).unwrap();
    let report = repo.apply_gitflow_init(&preview, &token()).unwrap();
    report.config.unwrap().unwrap();
    f
}
fn start(f: &Fixture, kind: GitFlowKind, name: &str) {
    let repo = f.repo();
    let preview = repo
        .preview_gitflow_start(kind, name, None, &token())
        .unwrap();
    let report = repo.apply_gitflow_start(&preview, &token()).unwrap();
    report.create.write.result.unwrap();
    report.switch.unwrap().unwrap().write.result.unwrap();
}
fn finish(f: &Fixture, branch: &str, options: &GitFlowFinishOptions) -> GitFlowFinishReport {
    let repo = f.repo();
    let preview = repo
        .preview_gitflow_finish(branch, options, &token())
        .unwrap();
    repo.apply_gitflow_finish(&preview, &token()).unwrap()
}
fn succeeded(report: &GitFlowFinishReport) {
    assert!(report.failure.is_none(), "{report:?}");
    assert!(report.skipped.is_empty() && !report.cancelled);
    for (step, result) in &report.steps {
        assert!(result.as_ref().unwrap().succeeded(), "{step:?}");
    }
    assert!(report.state.as_ref().unwrap().is_idle());
}
fn message(tag: &str) -> GitFlowFinishOptions {
    GitFlowFinishOptions {
        tag_message: tag.into(),
        ..Default::default()
    }
}

#[test]
fn initialize_writes_git_flow_settings_and_creates_develop_once() {
    isolated(
        "ROCKET_GITFLOW_CASE",
        "initialize_writes_git_flow_settings_and_creates_develop_once",
        || {
            let f = Fixture::new();
            f.write("app.txt", "base\n");
            f.commit("PRIVATE_BASE");
            let repo = f.repo();
            let state = repo.gitflow_state(&token()).unwrap();
            assert!(state.config.is_none() && !state.incomplete());
            let suggested = state.suggested();
            assert_eq!(suggested, GitFlowConfig::defaults("main"));
            // Refused before anything is written.
            let mut bad = suggested.clone();
            bad.develop = "main".into();
            let refused = |c: &GitFlowConfig| failure(repo.preview_gitflow_init(c, &token()));
            assert_eq!(refused(&bad), GitFlowFailure::InvalidConfig);
            let mut bad = suggested.clone();
            bad.hotfix = "feature/hot".into();
            assert_eq!(refused(&bad), GitFlowFailure::InvalidConfig);
            let mut bad = suggested.clone();
            bad.feature = "bad..prefix/".into();
            assert_eq!(refused(&bad), GitFlowFailure::InvalidConfig);
            let mut bad = suggested.clone();
            bad.version_tag = "v?".into();
            assert_eq!(refused(&bad), GitFlowFailure::InvalidConfig);
            let mut bad = suggested.clone();
            bad.master = "trunk".into();
            assert_eq!(refused(&bad), GitFlowFailure::ProductionMissing);
            assert!(!exists(&f, "refs/heads/develop"));

            let preview = repo.preview_gitflow_init(&suggested, &token()).unwrap();
            assert_eq!(preview.changes().len(), 7);
            assert!(matches!(
                preview.develop(),
                GitFlowDevelop::Create { source, track: false, .. } if source == "main"
            ));
            let report = repo.apply_gitflow_init(&preview, &token()).unwrap();
            report.branch.unwrap().write.result.unwrap();
            report.config.unwrap().unwrap();
            assert_eq!(report.state.unwrap().config.as_ref(), Some(&suggested));
            assert_eq!(rev(&f, "develop"), rev(&f, "main"));
            assert_eq!(head(&f), "main", "Initialize never switches branches");
            for (key, value) in GITFLOW_KEYS.iter().zip([
                "main", "develop", "feature/", "release/", "hotfix/", "support/", "",
            ]) {
                assert_eq!(config_get(&f, key), value);
            }
            assert!(
                repo.preview_gitflow_init(&suggested, &token())
                    .unwrap()
                    .unchanged()
            );
            // A review spanning an external settings edit writes nothing.
            let mut changed = suggested.clone();
            changed.version_tag = "v".into();
            let preview = repo.preview_gitflow_init(&changed, &token()).unwrap();
            assert_eq!(
                preview.changes(),
                [("gitflow.prefix.versiontag", "v".into())]
            );
            f.run(&["config", "user.name", "Changed Elsewhere"]);
            assert_eq!(
                failure(repo.apply_gitflow_init(&preview, &token())),
                GitFlowFailure::StalePreview
            );
            assert_eq!(config_get(&f, "gitflow.prefix.versiontag"), "");
            // Partial settings are not a setup; the suggestion keeps the set values.
            f.run(&["config", "--unset", "gitflow.prefix.support"]);
            let state = repo.gitflow_state(&token()).unwrap();
            assert!(state.config.is_none() && state.incomplete());
            assert_eq!(state.suggested(), suggested);
        },
    );
}

#[test]
fn initialize_tracks_an_existing_origin_develop() {
    isolated(
        "ROCKET_GITFLOW_CASE",
        "initialize_tracks_an_existing_origin_develop",
        || {
            let f = Fixture::new();
            f.write("app.txt", "base\n");
            f.commit("PRIVATE_BASE");
            let origin = f.root.join("origin.git");
            f.run(&["clone", "--bare", ".", origin.to_str().unwrap()]);
            f.run(&["remote", "add", "origin", origin.to_str().unwrap()]);
            f.run(&["switch", "-c", "develop"]);
            f.write("develop.txt", "develop\n");
            f.commit("PRIVATE_DEVELOP");
            f.run(&["push", "origin", "develop"]);
            f.run(&["switch", "main"]);
            f.run(&["branch", "-D", "develop"]);
            f.run(&["fetch", "origin"]);
            let repo = f.repo();
            let config = GitFlowConfig::defaults("main");
            let preview = repo.preview_gitflow_init(&config, &token()).unwrap();
            let target = rev(&f, "origin/develop");
            assert!(matches!(
                preview.develop(),
                GitFlowDevelop::Create { source, target: t, track: true }
                    if source == "origin/develop" && t.as_str() == target
            ));
            let report = repo.apply_gitflow_init(&preview, &token()).unwrap();
            report.config.unwrap().unwrap();
            assert_eq!(rev(&f, "develop"), target);
            assert_eq!(config_get(&f, "branch.develop.remote"), "origin");
            assert_eq!(config_get(&f, "branch.develop.merge"), "refs/heads/develop");
            assert_eq!(rev(&f, "develop@{upstream}"), target);
        },
    );
}

#[test]
fn start_creates_and_switches_by_git_flow_rules() {
    isolated(
        "ROCKET_GITFLOW_CASE",
        "start_creates_and_switches_by_git_flow_rules",
        || {
            let f = flow("sha1");
            let repo = f.repo();
            let review = |kind, name: &str, base: Option<&str>| {
                repo.preview_gitflow_start(kind, name, base, &token())
            };
            // A feature takes tracked changes along, as git checkout -b does.
            f.write("app.txt", "carried\n");
            let preview = review(GitFlowKind::Feature, "login 文", None);
            assert_eq!(failure(preview), GitFlowFailure::InvalidName);
            let preview = review(GitFlowKind::Feature, "login-文", None).unwrap();
            assert_eq!(preview.branch(), "feature/login-文");
            assert_eq!(preview.base(), "develop");
            assert_eq!(preview.base_id().as_str(), rev(&f, "develop"));
            assert_eq!(preview.carried(), 1);
            assert_eq!(preview.unpushed(), None);
            let report = repo.apply_gitflow_start(&preview, &token()).unwrap();
            report.create.write.result.unwrap();
            report.switch.unwrap().unwrap().write.result.unwrap();
            assert_eq!(head(&f), "feature/login-文");
            assert_eq!(
                std::fs::read_to_string(f.repository.join("app.txt")).unwrap(),
                "carried\n"
            );
            assert_eq!(
                failure(review(GitFlowKind::Feature, "login-文", None)),
                GitFlowFailure::BranchExists
            );
            assert_eq!(
                failure(review(GitFlowKind::Feature, "a..b", None)),
                GitFlowFailure::InvalidName
            );
            // Releases, hotfixes and support branches need a clean checkout.
            assert_eq!(
                failure(review(GitFlowKind::Release, "1.0", None)),
                GitFlowFailure::DirtyWorktree
            );
            f.run(&["restore", "app.txt"]);
            start(&f, GitFlowKind::Release, "1.0");
            assert_eq!(head(&f), "release/1.0");
            assert_eq!(rev(&f, "release/1.0"), rev(&f, "develop"));
            assert_eq!(
                failure(review(GitFlowKind::Release, "1.1", None)),
                GitFlowFailure::ReleaseInProgress
            );
            // Hotfixes start from main; an existing version tag is refused.
            f.run(&["tag", "v2.0", "main"]);
            assert_eq!(
                failure(review(GitFlowKind::Hotfix, "2.0", None)),
                GitFlowFailure::TagExists
            );
            let hotfix = review(GitFlowKind::Hotfix, "2.0.1", None).unwrap();
            assert_eq!((hotfix.base(), hotfix.tag()), ("main", Some("v2.0.1")));
            // Support branches start from a chosen base.
            let support = review(GitFlowKind::Support, "2.x", Some("v2.0")).unwrap();
            assert_eq!(support.base_id().as_str(), rev(&f, "v2.0^{commit}"));
            assert_eq!(
                failure(review(GitFlowKind::Support, "3.x", Some("no-such-base"))),
                GitFlowFailure::BaseMissing
            );
            // A base behind origin is refused; one ahead reports its unpushed commits.
            f.run(&["switch", "develop"]);
            f.write("ahead.txt", "ahead\n");
            f.commit("PRIVATE_AHEAD");
            let ahead = rev(&f, "develop");
            f.run(&["reset", "--hard", "HEAD~1"]);
            f.run(&["update-ref", "refs/remotes/origin/develop", &ahead]);
            assert_eq!(
                failure(review(GitFlowKind::Feature, "late", None)),
                GitFlowFailure::BehindRemote
            );
            f.run(&["update-ref", "refs/remotes/origin/develop", "develop"]);
            f.write("local.txt", "local\n");
            f.commit("PRIVATE_LOCAL");
            let preview = review(GitFlowKind::Feature, "late", None).unwrap();
            assert_eq!(preview.unpushed(), Some(1));
            // A review spanning an external change writes nothing.
            f.write("external.txt", "external\n");
            f.commit("PRIVATE_EXTERNAL");
            assert_eq!(
                failure(repo.apply_gitflow_start(&preview, &token())),
                GitFlowFailure::StalePreview
            );
            assert!(!exists(&f, "refs/heads/feature/late"));
            assert_eq!(head(&f), "develop");
        },
    );
}

#[test]
fn finish_feature_follows_git_flow_merge_rules() {
    isolated(
        "ROCKET_GITFLOW_CASE",
        "finish_feature_follows_git_flow_merge_rules",
        || {
            for format in ["sha1", "sha256"] {
                let f = flow(format);
                let repo = f.repo();
                // One commit: develop may fast-forward, then the branch is deleted.
                start(&f, GitFlowKind::Feature, "one");
                f.write("one.txt", "one\n");
                f.commit("PRIVATE_ONE");
                let tip = rev(&f, "feature/one");
                let preview = repo
                    .preview_gitflow_finish("feature/one", &Default::default(), &token())
                    .unwrap();
                assert_eq!(preview.kind(), GitFlowKind::Feature);
                assert_eq!(preview.ends_on(), Some("develop"));
                assert!(matches!(
                    preview.steps(),
                    [
                        GitFlowStep::Switch { branch },
                        GitFlowStep::Merge { mode: MergeMode::AllowFastForward, fast_forward: true, into, .. },
                        GitFlowStep::Delete { .. },
                    ] if branch == "develop" && into == "develop"
                ));
                let report = repo.apply_gitflow_finish(&preview, &token()).unwrap();
                succeeded(&report);
                assert_eq!(rev(&f, "develop"), tip);
                assert!(!exists(&f, "refs/heads/feature/one"));
                assert_eq!(head(&f), "develop");
                // More than one commit: a merge commit with git-flow's message.
                start(&f, GitFlowKind::Feature, "two");
                f.write("two.txt", "two\n");
                f.commit("PRIVATE_TWO_A");
                f.write("two.txt", "two b\n");
                f.commit("PRIVATE_TWO_B");
                let tip = rev(&f, "feature/two");
                succeeded(&finish(&f, "feature/two", &Default::default()));
                assert_eq!(rev(&f, "develop^2"), tip);
                assert_eq!(
                    text(f.run(&["log", "-1", "--format=%s", "develop"])),
                    "Merge branch 'feature/two' into develop"
                );
                // No fast-forward on request, and the branch kept on request.
                start(&f, GitFlowKind::Feature, "three");
                f.write("three.txt", "three\n");
                f.commit("PRIVATE_THREE");
                let tip = rev(&f, "feature/three");
                let keep = GitFlowFinishOptions {
                    no_ff: true,
                    delete_branch: false,
                    ..Default::default()
                };
                let preview = repo
                    .preview_gitflow_finish("feature/three", &keep, &token())
                    .unwrap();
                assert_eq!(preview.keep(), Some(&GitFlowKeep::Requested));
                succeeded(&repo.apply_gitflow_finish(&preview, &token()).unwrap());
                assert_eq!(rev(&f, "develop^2"), tip);
                assert!(exists(&f, "refs/heads/feature/three"));
                assert_eq!(
                    failure(repo.preview_gitflow_finish("feature/three", &keep, &token())),
                    GitFlowFailure::NothingToDo
                );
                // Finishing again only deletes the merged branch.
                let preview = repo
                    .preview_gitflow_finish("feature/three", &Default::default(), &token())
                    .unwrap();
                assert!(matches!(preview.steps(), [GitFlowStep::Delete { .. }]));
                assert!(preview.notes().contains(&GitFlowNote::Merged {
                    into: "develop".into()
                }));
                succeeded(&repo.apply_gitflow_finish(&preview, &token()).unwrap());
                assert!(!exists(&f, "refs/heads/feature/three"));
                // A feature tracking develop itself is deleted: develop contains it by then.
                start(&f, GitFlowKind::Feature, "four");
                f.write("four.txt", "four\n");
                f.commit("PRIVATE_FOUR");
                f.run(&["config", "branch.feature/four.remote", "."]);
                f.run(&["config", "branch.feature/four.merge", "refs/heads/develop"]);
                let preview = repo
                    .preview_gitflow_finish("feature/four", &Default::default(), &token())
                    .unwrap();
                assert_eq!(preview.keep(), None);
                succeeded(&repo.apply_gitflow_finish(&preview, &token()).unwrap());
                assert!(!exists(&f, "refs/heads/feature/four"));
            }
        },
    );
}

#[test]
fn finish_release_merges_into_main_tags_and_merges_back() {
    isolated(
        "ROCKET_GITFLOW_CASE",
        "finish_release_merges_into_main_tags_and_merges_back",
        || {
            let f = flow("sha1");
            let repo = f.repo();
            start(&f, GitFlowKind::Release, "1.2");
            f.write("app.txt", "release\n");
            f.commit("PRIVATE_RELEASE");
            let tip = rev(&f, "release/1.2");
            let options = message("Release 1.2 文");
            let preview = repo
                .preview_gitflow_finish("release/1.2", &options, &token())
                .unwrap();
            let kinds: Vec<_> = preview.steps().iter().map(GitFlowStep::label).collect();
            assert_eq!(
                kinds,
                [
                    "switch_branch",
                    "merge_branch",
                    "create_tag",
                    "switch_branch",
                    "merge_branch",
                    "delete_branch"
                ]
            );
            assert!(matches!(&preview.steps()[2], GitFlowStep::Tag { name, .. } if name == "v1.2"));
            assert!(preview.steps().iter().all(|s| !matches!(
                s,
                GitFlowStep::Merge { mode, .. } if *mode != MergeMode::NoFastForward
            )));
            succeeded(&repo.apply_gitflow_finish(&preview, &token()).unwrap());
            assert_eq!(rev(&f, "main^2"), tip);
            assert_eq!(
                text(f.run(&["log", "-1", "--format=%s", "main"])),
                "Merge branch 'release/1.2' into main"
            );
            assert_eq!(text(f.run(&["cat-file", "-t", "v1.2"])), "tag");
            assert_eq!(rev(&f, "v1.2^{commit}"), rev(&f, "main"));
            assert_eq!(
                text(f.run(&["tag", "-l", "--format=%(contents)", "v1.2"])),
                "Release 1.2 文"
            );
            assert_eq!(rev(&f, "develop^2"), tip);
            assert!(!exists(&f, "refs/heads/release/1.2"));
            assert_eq!(head(&f), "develop");
            // A new tag needs a message.
            start(&f, GitFlowKind::Hotfix, "1.2.1");
            f.write("fix.txt", "fix\n");
            f.commit("PRIVATE_FIX");
            assert_eq!(
                failure(repo.preview_gitflow_finish("hotfix/1.2.1", &message("  "), &token())),
                GitFlowFailure::TagMessage
            );
            // A tag of that version elsewhere is refused.
            f.run(&["tag", "v1.2.1", "main~1"]);
            assert_eq!(
                failure(repo.preview_gitflow_finish("hotfix/1.2.1", &message("Fix"), &token())),
                GitFlowFailure::TagExists
            );
            // Support branches have no finish.
            f.run(&["branch", "support/1.x", "main"]);
            assert_eq!(
                failure(repo.preview_gitflow_finish("support/1.x", &message("x"), &token())),
                GitFlowFailure::NotFinishable
            );
        },
    );
}

#[test]
fn finish_stops_on_conflicts_and_resumes_after_the_merge_is_committed() {
    isolated(
        "ROCKET_GITFLOW_CASE",
        "finish_stops_on_conflicts_and_resumes_after_the_merge_is_committed",
        || {
            let f = flow("sha1");
            let repo = f.repo();
            f.run(&["switch", "develop"]);
            f.write("app.txt", "develop\n");
            f.commit("PRIVATE_DEVELOP");
            start(&f, GitFlowKind::Hotfix, "1.0.1");
            f.write("app.txt", "hotfix\n");
            f.commit("PRIVATE_HOTFIX");
            let tip = rev(&f, "hotfix/1.0.1");
            let report = finish(&f, "hotfix/1.0.1", &message("Hotfix 1.0.1"));
            assert_eq!(report.failure, Some(GitFlowFailure::Conflicts));
            assert!(matches!(report.skipped[..], [GitFlowStep::Delete { .. }]));
            let (step, last) = report.steps.last().unwrap();
            assert!(matches!(step, GitFlowStep::Merge { into, .. } if into == "develop"));
            assert!(last.as_ref().unwrap().conflicts());
            // main, its merge and the tag are done; develop holds the stopped merge.
            assert_eq!(rev(&f, "main^2"), tip);
            assert_eq!(rev(&f, "v1.0.1^{commit}"), rev(&f, "main"));
            assert_eq!(head(&f), "develop");
            assert!(!report.state.unwrap().merge_heads.is_empty());
            // Resolve with Rocket's own merge flow, then finish again.
            f.write("app.txt", "resolved\n");
            f.run(&["add", "app.txt"]);
            repo.continue_merge("Merge branch 'hotfix/1.0.1' into develop", &token())
                .unwrap()
                .write
                .result
                .unwrap();
            let preview = repo
                .preview_gitflow_finish("hotfix/1.0.1", &message("unused"), &token())
                .unwrap();
            assert!(matches!(preview.steps(), [GitFlowStep::Delete { .. }]));
            for note in [
                GitFlowNote::Merged {
                    into: "main".into(),
                },
                GitFlowNote::Merged {
                    into: "develop".into(),
                },
            ] {
                assert!(preview.notes().contains(&note), "{note:?}");
            }
            assert!(preview.notes().iter().any(|n| matches!(
                n,
                GitFlowNote::Tagged { name, .. } if name == "v1.0.1"
            )));
            succeeded(&repo.apply_gitflow_finish(&preview, &token()).unwrap());
            assert!(!exists(&f, "refs/heads/hotfix/1.0.1"));
        },
    );
}

#[test]
fn finish_refuses_unsafe_states_and_keeps_unpushed_branches() {
    isolated(
        "ROCKET_GITFLOW_CASE",
        "finish_refuses_unsafe_states_and_keeps_unpushed_branches",
        || {
            let f = flow("sha1");
            let repo = f.repo();
            start(&f, GitFlowKind::Feature, "x");
            f.write("x.txt", "x\n");
            f.commit("PRIVATE_X");
            let review = || repo.preview_gitflow_finish("feature/x", &Default::default(), &token());
            f.write("app.txt", "dirty\n");
            assert_eq!(failure(review()), GitFlowFailure::DirtyWorktree);
            f.run(&["restore", "app.txt"]);
            // Behind its origin counterpart: refused. Ahead with an upstream: kept.
            f.write("x.txt", "remote\n");
            f.commit("PRIVATE_REMOTE");
            let remote = rev(&f, "HEAD");
            f.run(&["reset", "--hard", "HEAD~1"]);
            f.run(&["update-ref", "refs/remotes/origin/feature/x", &remote]);
            assert_eq!(failure(review()), GitFlowFailure::BehindRemote);
            f.run(&["update-ref", "refs/remotes/origin/feature/x", "feature/x~1"]);
            f.run(&[
                "config",
                "remote.origin.url",
                f.root.join("none.git").to_str().unwrap(),
            ]);
            f.run(&[
                "config",
                "remote.origin.fetch",
                "+refs/heads/*:refs/remotes/origin/*",
            ]);
            f.run(&["config", "branch.feature/x.remote", "origin"]);
            f.run(&["config", "branch.feature/x.merge", "refs/heads/feature/x"]);
            let preview = review().unwrap();
            assert_eq!(
                preview.keep(),
                Some(&GitFlowKeep::Unpushed("origin/feature/x".into()))
            );
            assert!(preview.notes().contains(&GitFlowNote::Unpushed {
                branch: "feature/x".into(),
                commits: 1
            }));
            assert!(
                preview
                    .steps()
                    .iter()
                    .all(|s| !matches!(s, GitFlowStep::Delete { .. }))
            );
            f.run(&["update-ref", "-d", "refs/remotes/origin/feature/x"]);
            // develop checked out in another worktree cannot be switched to here.
            let linked = f.root.join("linked 文");
            f.run(&["worktree", "add", linked.to_str().unwrap(), "develop"]);
            assert_eq!(failure(review()), GitFlowFailure::CheckedOutElsewhere);
            f.run(&["worktree", "remove", linked.to_str().unwrap()]);
            // A review spanning an external commit writes nothing.
            let preview = review().unwrap();
            f.run(&["switch", "develop"]);
            f.write("other.txt", "other\n");
            f.commit("PRIVATE_OTHER");
            let develop = rev(&f, "develop");
            assert_eq!(
                failure(repo.apply_gitflow_finish(&preview, &token())),
                GitFlowFailure::StalePreview
            );
            assert_eq!(rev(&f, "develop"), develop);
            assert!(exists(&f, "refs/heads/feature/x"));
            // A branch outside the prefixes is not a git-flow branch.
            assert_eq!(
                failure(repo.preview_gitflow_finish("main", &Default::default(), &token())),
                GitFlowFailure::InvalidName
            );
            // Without git-flow settings nothing starts or finishes.
            f.run(&["config", "--unset", "gitflow.branch.develop"]);
            assert_eq!(failure(review()), GitFlowFailure::NotInitialized);
            assert_eq!(
                failure(repo.preview_gitflow_start(GitFlowKind::Feature, "y", None, &token())),
                GitFlowFailure::NotInitialized
            );
        },
    );
}
