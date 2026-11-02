//! Reads behind the row 11 dialogs: merged branches for deletion, what a push would
//! send, which reflog commits no branch reaches, a reflog entry's details, and the
//! identity that applies without local overrides.
mod common;
use common::{Fixture, isolated};
use rocket_git::*;
use std::fs;

const CASE: &str = "ROCKET_ROW11_READS_CASE";

fn id(f: &Fixture, revision: &str) -> ObjectId {
    f.repo()
        .resolve_commit(revision, &CancellationToken::new())
        .unwrap()
}

#[test]
fn merge_states_follow_git_branch_delete() {
    isolated(CASE, "merge_states_follow_git_branch_delete", || {
        let f = Fixture::new();
        f.write("a.txt", "1\n");
        f.commit("base");
        f.run(&["branch", "merged"]);
        f.run(&["switch", "-c", "unmerged"]);
        f.write("u.txt", "1\n");
        f.commit("one");
        f.write("u.txt", "2\n");
        f.commit("two");
        f.run(&["switch", "main"]);
        // Its upstream is checked, not HEAD.
        let origin = f.root.join("origin.git");
        f.run(&["remote", "add", "origin", origin.to_str().unwrap()]);
        f.run(&["branch", "tracked", "unmerged"]);
        f.run(&["update-ref", "refs/remotes/origin/tracked", "main"]);
        f.run(&["branch", "--set-upstream-to=origin/tracked", "tracked"]);
        f.run(&["update-ref", "refs/remotes/origin/tracked", "unmerged~2"]);
        let names: Vec<Vec<u8>> = ["merged", "unmerged", "tracked", "missing"]
            .iter()
            .map(|n| n.as_bytes().to_vec())
            .collect();
        let states = f
            .repo()
            .branch_merge_states(&names, &CancellationToken::new())
            .unwrap();
        let state = |name: &str| states.iter().find(|s| s.name == name.as_bytes()).unwrap();
        assert_eq!(states.len(), 3, "a missing branch is left out: {states:?}");
        assert!(state("merged").merged && state("merged").unmerged == Some(0));
        assert!(!state("unmerged").merged);
        assert_eq!(state("unmerged").unmerged, Some(2));
        assert!(!state("unmerged").upstream);
        assert!(state("tracked").upstream);
        assert_eq!(state("tracked").unmerged, Some(2));
        // Each carries its last commit, which HEAD's history doesn't have.
        assert_eq!(state("unmerged").tip.subject_lossy(), "two");
        assert_eq!(state("merged").tip.subject_lossy(), "base");
        // Git agrees: the merged branch goes, the unmerged one stays.
        let report = f
            .repo()
            .delete_branches(&names[..2], &CancellationToken::new())
            .unwrap();
        assert!(report.results[0].1.as_ref().unwrap().write.result.is_ok());
        assert!(report.results[1].1.as_ref().unwrap().write.result.is_err());
    });
}

#[test]
fn push_outlook_counts_against_the_last_fetch() {
    isolated(CASE, "push_outlook_counts_against_the_last_fetch", || {
        let f = Fixture::new();
        f.write("a.txt", "1\n");
        f.commit("base");
        let origin = f.root.join("origin.git");
        f.run(&["init", "--bare", origin.to_str().unwrap()]);
        f.run(&["remote", "add", "origin", origin.to_str().unwrap()]);
        f.run(&["push", "--quiet", "-u", "origin", "main"]);
        f.write("a.txt", "2\n");
        f.commit("PRIVATE_local work");
        let cancel = CancellationToken::new();
        let outlook = f.repo().push_outlook(b"main", b"origin", b"main", &cancel).unwrap();
        assert_eq!(outlook.tracked, Some(id(&f, "HEAD~1")));
        assert_eq!((outlook.ahead, outlook.behind), (1, 0));
        assert_eq!(outlook.commits.len(), 1);
        assert_eq!(outlook.commits[0].subject_lossy(), "PRIVATE_local work");
        // A destination never fetched counts what no branch of that remote has.
        let new = f.repo().push_outlook(b"main", b"origin", b"feature/new", &cancel).unwrap();
        assert_eq!((new.tracked, new.ahead, new.behind), (None, 1, 0));
        // The remote moved on (as a fetch would show): the push would be refused.
        f.run(&["switch", "-c", "other", "HEAD~1"]);
        f.write("b.txt", "1\n");
        f.commit("theirs");
        f.run(&["update-ref", "refs/remotes/origin/main", "other"]);
        f.run(&["switch", "main"]);
        let behind = f.repo().push_outlook(b"main", b"origin", b"main", &cancel).unwrap();
        assert_eq!((behind.ahead, behind.behind), (1, 1));
        assert!(f.repo().push_outlook(b"gone", b"origin", b"main", &cancel).is_err());
        assert!(f.repo().push_outlook(b"main", b"-x", b"main", &cancel).is_err());
    });
}

#[test]
fn reflog_entries_name_their_commit_and_whether_a_branch_has_it() {
    isolated(CASE, "reflog_entries_name_their_commit_and_whether_a_branch_has_it", || {
        let f = Fixture::new();
        f.write("a.txt", "1\n");
        f.commit("PRIVATE_base");
        f.write("a.txt", "2\n");
        f.commit("PRIVATE_kept");
        f.write("a.txt", "3\n");
        f.commit("PRIVATE_lost");
        let lost = id(&f, "HEAD");
        f.run(&["reset", "--quiet", "--hard", "HEAD~1"]);
        let cancel = CancellationToken::new();
        let page = f.repo().reflog_scoped(ReflogScope::Head, 10, &cancel).unwrap();
        let reset = &page.entries[0];
        assert!(String::from_utf8_lossy(&reset.subject).starts_with("reset: moving to HEAD~1"));
        assert_eq!(reset.commit_subject, b"PRIVATE_kept");
        let ids: Vec<ObjectId> = page.entries.iter().map(|e| e.id.clone()).collect();
        let off = f.repo().off_branches(&ids, &cancel).unwrap().unwrap();
        assert_eq!(off, std::collections::HashSet::from([lost.clone()]));
        // The details compare the selected commit with the current tip.
        let details = f.repo().recovery_details(&lost, &cancel).unwrap();
        assert_eq!(details.commit.subject_lossy(), "PRIVATE_lost");
        assert_eq!(details.relation, Some((1, 0)));
        assert!(String::from_utf8_lossy(&details.patch).contains("+3"));
        let previous = f.repo().recovery_details(&id(&f, "HEAD~1"), &cancel).unwrap();
        assert_eq!(previous.relation, Some((0, 1)));
        // A reset preview carries the same relation.
        let preview = f
            .repo()
            .preview_recovery(&lost, RecoveryAction::Reset(ResetMode::Soft), &cancel)
            .unwrap();
        assert_eq!(preview.relation(), Some((1, 0)));
    });
}

#[test]
fn identity_reports_what_applies_without_local_overrides() {
    isolated(CASE, "identity_reports_what_applies_without_local_overrides", || {
        let f = Fixture::new();
        let global = std::env::var_os("GIT_CONFIG_GLOBAL").expect("isolated global config");
        fs::write(&global, "[user]\n\tname = PRIVATE Global Name\n").unwrap();
        let cancel = CancellationToken::new();
        let identity = f.repo().identity(&cancel).unwrap();
        // The fixture sets both locally; the global file has only a name.
        assert_eq!(identity.local.name.as_deref(), Some("Rocket Test"));
        assert_eq!(identity.inherited.name.as_deref(), Some("PRIVATE Global Name"));
        assert_eq!(identity.inherited.email, None);
        fs::write(&global, "").unwrap();
        assert_eq!(f.repo().identity(&cancel).unwrap().inherited, LocalIdentity::default());
    });
}
