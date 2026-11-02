#![cfg(windows)]
mod common;
use common::Fixture;
use rocket_git::*;
use std::fs;
fn token() -> CancellationToken {
    CancellationToken::new()
}
fn isolated(name: &str, f: fn()) {
    common::isolated("ROCKET_ISSUE_LINK_CASE", name, f);
}
fn rule(name: &str, pattern: &str, url: &str) -> IssueLinkRule {
    IssueLinkRule {
        name: name.into(),
        pattern: pattern.into(),
        url: url.into(),
    }
}
fn jira() -> IssueLinkRule {
    rule(
        "jira",
        r"\b[A-Z][A-Z0-9]+-\d+\b",
        "https://jira.example.com/browse/{id}",
    )
}
fn redmine() -> IssueLinkRule {
    rule(
        "redmine",
        r"#(\d+)",
        "https://redmine.example.com/issues/{1}",
    )
}

#[test]
fn rules_round_trip_through_the_repository_config() {
    isolated("rules_round_trip_through_the_repository_config", || {
        let f = Fixture::new();
        let repo = f.repo();
        let global = std::env::var_os("GIT_CONFIG_GLOBAL").unwrap();
        let before_global = fs::read(&global).unwrap();
        let empty = repo.issue_link_settings(&token()).unwrap();
        assert!(empty.local.is_empty() && empty.inherited.is_empty());
        assert!(empty.links().is_empty());
        let report = repo
            .set_issue_links(&empty, &[jira(), redmine()], &token())
            .unwrap();
        assert!(report.result.is_ok(), "{:?}", report.result.err());
        let saved = report.settings.unwrap();
        assert_eq!(saved.local, [jira(), redmine()]);
        assert!(saved.problems.is_empty());
        let config = fs::read_to_string(f.repository.join(".git/config")).unwrap();
        assert!(config.contains("[rocket \"issuelink.jira\"]"), "{config}");
        assert!(config.contains("email = rocket@example.invalid"));
        f.write("a.txt", "a");
        f.commit("PROJ-7 fixes #12");
        let message = String::from_utf8(f.run(&["log", "-1", "--format=%B"])).unwrap();
        let links = saved.links().find(&message);
        let urls: Vec<_> = links.iter().map(|l| l.url.as_str()).collect();
        assert_eq!(
            urls,
            [
                "https://jira.example.com/browse/PROJ-7",
                "https://redmine.example.com/issues/12"
            ]
        );
        // Replacing the list removes a rule and keeps the order of the rest.
        let report = repo
            .set_issue_links(&saved, &[redmine()], &token())
            .unwrap();
        assert!(report.result.is_ok());
        let after = report.settings.unwrap();
        assert_eq!(after.local, [redmine()]);
        let config = fs::read_to_string(f.repository.join(".git/config")).unwrap();
        assert!(!config.contains("issuelink.jira"), "{config}");
        assert!(config.contains("email = rocket@example.invalid"));
        assert_eq!(fs::read(&global).unwrap(), before_global);
        assert!(!f.repository.join(".git/config.lock").exists());
    });
}

#[test]
fn global_and_included_rules_apply_and_a_local_rule_replaces_them() {
    isolated(
        "global_and_included_rules_apply_and_a_local_rule_replaces_them",
        || {
            let f = Fixture::new();
            let repo = f.repo();
            let global = std::env::var_os("GIT_CONFIG_GLOBAL").unwrap();
            fs::write(
                &global,
                "[rocket \"issuelink.jira\"]\n\tpattern = OLD-\\\\d+\n\turl = https://old.example.com/{id}\n\
                 [rocket \"issuelink.shared\"]\n\tpattern = SH-\\\\d+\n\turl = https://shared.example.com/{id}\n\
                 [rocket \"issuelink.half\"]\n\tpattern = H-\\\\d+\n",
            )
            .unwrap();
            let included = f.root.join("included.config");
            let included_text = "[rocket \"issuelink.inc\"]\n\tpattern = INC-\\\\d+\n\turl = https://inc.example.com/{id}\n";
            fs::write(&included, included_text).unwrap();
            f.run(&["config", "include.path", included.to_str().unwrap()]);
            let before = repo.issue_link_settings(&token()).unwrap();
            assert!(before.local.is_empty());
            let names: Vec<_> = before
                .inherited
                .iter()
                .map(|(scope, r)| (*scope, r.name.as_str()))
                .collect();
            assert_eq!(
                names,
                [
                    (IssueLinkScope::Global, "jira"),
                    (IssueLinkScope::Global, "shared"),
                    (IssueLinkScope::Repository, "inc"),
                ]
            );
            assert!(before.problems.iter().any(|p| p.contains("half")));
            let urls = |s: &IssueLinkSettings, text: &str| -> Vec<String> {
                s.links().find(text).into_iter().map(|l| l.url).collect()
            };
            assert_eq!(
                urls(&before, "OLD-1 SH-2 INC-3 H-4"),
                [
                    "https://old.example.com/OLD-1",
                    "https://shared.example.com/SH-2",
                    "https://inc.example.com/INC-3"
                ]
            );
            // A repository rule named jira replaces the global one.
            let saved = repo
                .set_issue_links(&before, &[jira()], &token())
                .unwrap()
                .settings
                .unwrap();
            assert_eq!(saved.local, [jira()]);
            assert!(!saved.inherited.iter().any(|(_, r)| r.name == "jira"));
            assert_eq!(
                urls(&saved, "OLD-1 PROJ-2 SH-3"),
                [
                    "https://jira.example.com/browse/OLD-1",
                    "https://jira.example.com/browse/PROJ-2",
                    "https://jira.example.com/browse/SH-3"
                ]
            );
            assert_eq!(fs::read_to_string(&included).unwrap(), included_text);
        },
    );
}

#[test]
fn broken_rules_are_listed_inactive_and_can_be_removed() {
    isolated(
        "broken_rules_are_listed_inactive_and_can_be_removed",
        || {
            let f = Fixture::new();
            let repo = f.repo();
            f.run(&["config", "rocket.issuelink.half.pattern", "X-1"]);
            f.run(&["config", "rocket.issuelink.bad.pattern", "(?=x)"]);
            f.run(&[
                "config",
                "rocket.issuelink.bad.url",
                "https://x.example/{id}",
            ]);
            f.run(&["config", "rocket.issuelink.http.pattern", "Y-1"]);
            f.run(&[
                "config",
                "rocket.issuelink.http.url",
                "http://x.example/{id}",
            ]);
            let settings = repo.issue_link_settings(&token()).unwrap();
            let names: Vec<_> = settings.local.iter().map(|r| r.name.as_str()).collect();
            assert_eq!(names, ["half", "bad", "http"]);
            assert_eq!(settings.local[0].url, "");
            assert_eq!(settings.problems.len(), 3, "{:?}", settings.problems);
            assert!(settings.links().is_empty());
            assert!(settings.links().find("X-1 Y-1 x").is_empty());
            let report = repo.set_issue_links(&settings, &[], &token()).unwrap();
            assert!(report.result.is_ok(), "{:?}", report.result.err());
            let after = report.settings.unwrap();
            assert!(after.local.is_empty() && after.problems.is_empty());
            let config = fs::read_to_string(f.repository.join(".git/config")).unwrap();
            assert!(!config.contains("issuelink"), "{config}");
        },
    );
}

#[test]
fn stale_invalid_and_locked_saves_preserve_the_config() {
    isolated("stale_invalid_and_locked_saves_preserve_the_config", || {
        let f = Fixture::new();
        let repo = f.repo();
        let config = f.repository.join(".git/config");
        let before = repo.issue_link_settings(&token()).unwrap();
        for rules in [
            vec![rule("jira", "A-1", "http://x.example/{id}")],
            vec![rule("two words", "A-1", "https://x.example/{id}")],
            vec![rule("x", "(", "https://x.example/{id}")],
            vec![rule("x", "A-1", "https://x.example/{2}")],
            vec![jira(), rule("JIRA", "B-1", "https://y.example/{id}")],
            (0..=MAX_ISSUE_LINK_RULES)
                .map(|i| rule(&format!("r{i}"), "A-1", "https://x.example/{id}"))
                .collect(),
        ] {
            let original = fs::read(&config).unwrap();
            assert!(repo.set_issue_links(&before, &rules, &token()).is_err());
            assert_eq!(fs::read(&config).unwrap(), original);
        }
        // Changed since reading: refused, and the newer edit is kept.
        f.run(&["config", "rocket.issuelink.other.pattern", "Z-1"]);
        let original = fs::read(&config).unwrap();
        let report = repo.set_issue_links(&before, &[jira()], &token()).unwrap();
        assert!(matches!(report.result, Err(Error::InvalidInput(_))));
        assert_eq!(fs::read(&config).unwrap(), original);
        // A config.lock held by another writer is left alone.
        let fresh = repo.issue_link_settings(&token()).unwrap();
        fs::write(config.with_extension("lock"), b"other writer").unwrap();
        let report = repo.set_issue_links(&fresh, &[jira()], &token()).unwrap();
        assert!(report.result.is_err());
        assert_eq!(
            fs::read(config.with_extension("lock")).unwrap(),
            b"other writer"
        );
        assert_eq!(fs::read(&config).unwrap(), original);
        fs::remove_file(config.with_extension("lock")).unwrap();
        // Cancellation before the write publishes nothing.
        let cancel = token();
        cancel.cancel();
        assert!(repo.set_issue_links(&fresh, &[jira()], &cancel).is_err());
        assert_eq!(fs::read(&config).unwrap(), original);
    });
}
