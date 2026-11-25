mod common;
use common::{Fixture, isolated};
use rocket_git::*;

fn token() -> CancellationToken {
    CancellationToken::new()
}
fn head(f: &Fixture) -> ObjectId {
    ObjectId::parse(String::from_utf8(f.run(&["rev-parse", "HEAD"])).unwrap().trim()).unwrap()
}
/// Each link as `from -> to`, a folder end ending in `/`.
fn pairs(links: &CodeLinks) -> Vec<String> {
    let name = |i: usize| {
        let e = &links.ends[i];
        format!("{}{}", String::from_utf8_lossy(&e.path), if e.folder { "/" } else { "" })
    };
    let mut out: Vec<String> = links.links.iter().map(|l| format!("{} -> {}", name(l.from), name(l.to))).collect();
    out.sort();
    out
}

#[test]
fn script_loads_link_only_committed_helpers_in_both_maps() {
    isolated("ROCKET_CODE_MAP_TEST", "script_loads_link_only_committed_helpers_in_both_maps", || {
        let f = Fixture::new();
        let ps = ". (Join-Path $PSScriptRoot 'lib/補 助.ps1')\r\nImport-Module \"$PSScriptRoot/../shared/module.psm1\"\r\n";
        let utf16: Vec<_> = [0xff, 0xfe].into_iter().chain(ps.encode_utf16().flat_map(u16::to_le_bytes)).collect();
        f.write("tools/build.ps1", utf16);
        f.write("tools/lib/補 助.ps1", "# committed helper\n");
        f.write("shared/module.psm1", "# committed module\n");
        f.write("tools/build.sh", "root=$(cd \"$(dirname \"$0\")/..\" && pwd -P)\nsource \"$root/shared/helper.bash\"\n. './lib/local.sh'\n");
        f.write("tools/lib/local.sh", "# committed script\n");
        f.write("shared/helper.bash", "# committed shell helper\n");
        f.commit("script loads");
        let tip = head(&f);
        f.write("tools/build.ps1", ". \"$PSScriptRoot/untracked.ps1\"\n");
        f.write("tools/untracked.ps1", "# local only\n");
        let repo = f.repo();
        let all = repo.code_links(&tip, None, &token()).unwrap();
        let paths = pairs(&all);
        assert_eq!(paths, [
            "tools/build.ps1 -> shared/module.psm1", "tools/build.ps1 -> tools/lib/補 助.ps1",
            "tools/build.sh -> shared/helper.bash", "tools/build.sh -> tools/lib/local.sh",
        ]);
        let subset = repo.code_links(&tip, Some(&[b"tools".to_vec()]), &token()).unwrap();
        assert_eq!(paths, pairs(&subset));
        assert_eq!(subset.packages.len(), 2);
        for package in subset.packages {
            let map = repo.repository_map(&MapOptions { kind: MapKind::Modules, package: Some(package), ..Default::default() }, &token()).unwrap();
            let edges: Vec<_> = map.links.iter().filter(|e| e.kind == MapLinkKind::DependsOn).collect();
            assert_eq!(edges.len(), 1);
            let e = edges[0];
            let ps = map.nodes[e.from].path.as_deref() == Some(b"tools/build.ps1");
            let helper: &[u8] = if ps { "tools/lib/補 助.ps1".as_bytes() } else { b"tools/lib/local.sh" };
            assert_eq!(map.nodes[e.to].path.as_deref(), Some(helper));
            let outside = &map.modules.as_ref().unwrap().outside[&e.from];
            assert_eq!(outside, &[if ps { b"shared/module.psm1".to_vec() } else { b"shared/helper.bash".to_vec() }]);
        }
        let cancelled = token();
        cancelled.cancel();
        assert!(matches!(repo.code_links(&tip, None, &cancelled), Err(Error::Cancelled)));
    });
}

#[test]
fn script_load_budget_preserves_known_edges_and_marks_partial_reads() {
    isolated("ROCKET_CODE_MAP_TEST", "script_load_budget_preserves_known_edges_and_marks_partial_reads", || {
        let f = Fixture::new();
        f.write("ci/main.sh", "source ./helper.sh\n".repeat(13_000));
        f.write("ci/helper.sh", "# helper\n");
        f.commit("repeated loads");
        let repo = f.repo();
        let tip = head(&f);
        let mut all = repo.code_links(&tip, None, &token()).unwrap();
        let partial = repo.code_links(&tip, Some(&[b"ci".to_vec()]), &token()).unwrap();
        assert!(partial.sources_limited);
        all.merge(partial);
        assert!(all.limited && all.sources_limited);
        assert_eq!(pairs(&all), ["ci/main.sh -> ci/helper.sh"]);
        let map = repo.repository_map(&MapOptions { kind: MapKind::Modules, ..Default::default() }, &token()).unwrap();
        assert!(map.limited);
        assert_eq!(map.links.iter().filter(|e| e.kind == MapLinkKind::DependsOn).count(), 1);
    });
}

#[test]
fn empty_packages_are_bounded_and_can_be_read_later() {
    isolated("ROCKET_CODE_MAP_TEST", "empty_packages_are_bounded_and_can_be_read_later", || {
        let f = Fixture::new();
        for i in 0..=CODE_MAP_PACKAGES {
            f.write(&format!("p{i:04}/Cargo.toml"), "[package]\nname='empty'\n");
        }
        f.commit("empty packages");
        let repo = f.repo();
        let tip = head(&f);
        let mut links = repo.code_links(&tip, None, &token()).unwrap();
        assert_eq!(links.files, 0);
        assert_eq!(links.packages.len(), CODE_MAP_PACKAGES);
        assert!(links.limited);
        assert_eq!(links.unread.len(), 1);
        let folder = links.unread[0].folder.clone();
        let more = repo.code_links(&tip, Some(&[folder]), &token()).unwrap();
        assert_eq!(more.packages, links.unread);
        links.merge(more);
        assert!(!links.limited && links.unread.is_empty());
    });
}

#[test]
fn code_links_bound_codeowners_work_without_showing_partial_ownership() {
    isolated("ROCKET_CODE_MAP_TEST", "code_links_bound_codeowners_work_without_showing_partial_ownership", || {
        let f = Fixture::new();
        let rules: String = (0..10_000).map(|i| format!("/missing-{i}/** @owner\n")).collect();
        f.write("CODEOWNERS", &rules);
        for i in 0..200 { f.write(&format!("file-{i}.md"), "data\n"); }
        f.commit("bounded ownership");
        let links = f.repo().code_links(&head(&f), None, &token()).unwrap();
        assert!(links.owners_limited && links.limited);
        assert!(links.owners.is_none(), "partial rules must not mislabel files as unowned");
    });
}

#[test]
fn code_links_bound_path_followups_by_total_bytes() {
    isolated("ROCKET_CODE_MAP_TEST", "code_links_bound_path_followups_by_total_bytes", || {
        let f = Fixture::new();
        f.write("Cargo.toml", "[package]\nname='bounded'\n");
        let mut source = String::new();
        // These files are outside src/tests/examples/benches. They can only be
        // reached by the follow-up reader, not its initial package reservation.
        let body = format!("//{}\n", "x".repeat(1024 * 1024 - 3));
        for i in 0..66 {
            source.push_str(&format!("#[path = \"../shared/m{i}.rs\"]\nmod m{i};\n"));
            f.write(&format!("shared/m{i}.rs"), &body);
        }
        f.write("src/lib.rs", &source);
        f.commit("bounded follow-ups");
        let links = f.repo().code_links(&head(&f), None, &token()).unwrap();
        assert!(links.limited);
        assert!(links.files <= 64, "retained {} source files", links.files);
        assert_eq!(links.lines.len(), links.files);
        assert!(links.lines.len() > 1, "valid follow-ups still load");
        let cancelled = token(); cancelled.cancel();
        assert!(matches!(f.repo().code_links(&head(&f), None, &cancelled), Err(Error::Cancelled)));
    });
}

#[test]
fn code_links_cross_packages_tests_path_modules_and_codeowners() {
    isolated("ROCKET_CODE_MAP_TEST", "code_links_cross_packages_tests_path_modules_and_codeowners", || {
        let f = Fixture::new();
        f.write("Cargo.toml", "[workspace]\nmembers=['crates/ui','crates/core']\n");
        f.write("crates/core/Cargo.toml", "[package]\nname='engine'\n");
        f.write("crates/core/src/lib.rs", "pub mod store;\npub fn run() {}\n");
        f.write("crates/core/src/store.rs", "pub struct Store;\nuse crate::run;\n");
        f.write("crates/core/tests/smoke.rs", "mod common;\nuse engine::store::Store;\n#[test] fn t() { common::fixture(); }\n");
        f.write("crates/core/tests/common/mod.rs", "pub fn fixture() {}\n");
        f.write("crates/ui/Cargo.toml", "[package]\nname='ui-shell'\n[dependencies]\nengine={path='../core'}\n");
        f.write(
            "crates/ui/src/lib.rs",
            "mod view;\nuse engine::run;\n#[cfg(test)]\n#[path = \"../../core/tests/common/mod.rs\"]\nmod shared;\npub fn start() { run(); view::draw(); }\n#[cfg(test)]\nfn check() { shared::fixture(); }\n",
        );
        f.write("crates/ui/src/view.rs", "pub fn draw() {}\n");
        f.write("tools/package.json", "{\"name\": \"tools\"}\n");
        f.write("tools/a.js", "const b = require('./b.js');\n");
        f.write("tools/b.js", "module.exports = 1;\n");
        f.write("web/package.json", "{\"name\": \"web\"}\n");
        f.write("web/index.js", "import t from 'tools';\nimport fs from 'node:fs';\n");
        f.write(".github/CODEOWNERS", "# owners\n/crates/core/ @core-team\n*.md @docs\n/web/\n");
        f.write("README.md", "hello\n");
        f.commit("code");
        let tip = head(&f);
        let repo = f.repo();
        let links = repo.code_links(&tip, None, &token()).unwrap();
        let found = pairs(&links);
        for expected in [
            "crates/core/src/store.rs -> crates/core/src/lib.rs",
            "crates/core/tests/smoke.rs -> crates/core/src/lib.rs",
            "crates/core/tests/smoke.rs -> crates/core/tests/common/mod.rs",
            "crates/ui/src/lib.rs -> crates/core/",
            "crates/ui/src/lib.rs -> crates/ui/src/view.rs",
            "crates/ui/src/lib.rs -> crates/core/tests/common/mod.rs",
            "tools/a.js -> tools/b.js",
            "web/index.js -> tools/",
        ] {
            assert!(found.contains(&expected.to_string()), "missing {expected}: {found:#?}");
        }
        assert!(!found.iter().any(|p| p.contains("node:fs")), "outside packages are not drawn: {found:#?}");
        assert_eq!(links.packages.len(), 4, "{:?}", links.packages);
        assert!(links.unread.is_empty() && !links.limited);
        // Nine code files: the #[path] file ui shares with core's tests counts once.
        assert_eq!(links.files, 9);
        assert_eq!(links.lines.get(b"crates/ui/src/lib.rs".as_slice()), Some(&8));
        let owners = links.owners.as_ref().unwrap();
        assert_eq!(owners.file, b".github/CODEOWNERS");
        assert!(!owners.gitlab);
        assert_eq!(owners.rules, 3);
        let set = |p: &str| owners.files.get(p.as_bytes()).copied();
        let core = set("crates/core/src/lib.rs").flatten().unwrap();
        assert_eq!(owners.sets[core as usize], vec!["@core-team".to_string()]);
        assert_eq!(owners.sets[set("README.md").flatten().unwrap() as usize], vec!["@docs".to_string()]);
        assert_eq!(set("web/index.js"), Some(None), "a line without owners leaves files unowned");
        assert_eq!(set("tools/a.js"), None, "no line matches");
        // One folder alone, as opening a folder past the limit reads it.
        let core_only = repo.code_links(&tip, Some(&[b"crates/core".to_vec()]), &token()).unwrap();
        assert_eq!(core_only.packages.len(), 1);
        assert!(core_only.owners.is_none());
        assert!(pairs(&core_only).iter().all(|p| p.starts_with("crates/core/")));
        // Kinds come from paths alone.
        assert_eq!(file_kind(b"crates/core/tests/smoke.rs"), FileKind::Tests);
        assert_eq!(file_kind(b"crates/core/Cargo.toml"), FileKind::Config);
    });
}

#[test]
fn code_links_reach_packages_at_the_top_and_files_of_other_projects() {
    isolated("ROCKET_CODE_MAP_TEST", "code_links_reach_packages_at_the_top_and_files_of_other_projects", || {
        let f = Fixture::new();
        // A Go module at the top: its top package has no folder card, so its files stand
        // for it, both ways.
        f.write("go.mod", "module example.com/app\n\ngo 1.22\n");
        f.write("app.go", "package app\n\nimport \"example.com/app/internal/store\"\n\nfunc Run() { store.Open() }\n");
        f.write("internal/store/store.go", "package store\n\nfunc Open() {}\n");
        f.write("cmd/tool/main.go", "package main\n\nimport \"example.com/app\"\n\nfunc main() { app.Run() }\n");
        // A Cargo package at the top, used by a member: its library stands for it.
        f.write("Cargo.toml", "[package]\nname='root-lib'\n[workspace]\nmembers=['member']\n");
        f.write("src/lib.rs", "pub fn hello() {}\n");
        f.write("member/Cargo.toml", "[package]\nname='member'\n[dependencies]\nroot-lib={path='..'}\n");
        f.write("member/src/lib.rs", "use root_lib::hello;\npub fn go() { hello(); }\n");
        // A relative import into another project names that file.
        f.write("scripts/package.json", "{\"name\": \"scripts\", \"type\": \"module\"}\n");
        f.write("scripts/build.js", "import util from '../lib/util.js';\n");
        f.write("lib/util.js", "export default 1;\n");
        f.commit("top");
        let tip = head(&f);
        let links = f.repo().code_links(&tip, None, &token()).unwrap();
        let found = pairs(&links);
        for expected in ["app.go -> internal/store/", "cmd/tool/ -> app.go", "member/src/lib.rs -> src/lib.rs", "scripts/build.js -> lib/util.js"] {
            assert!(found.contains(&expected.to_string()), "missing {expected}: {found:#?}");
        }
        assert!(links.ends.iter().all(|e| !e.path.is_empty()), "no end is the top folder: {found:#?}");
    });
}

#[test]
fn code_history_counts_commits_by_path_with_the_pinned_mailmap() {
    isolated("ROCKET_CODE_MAP_TEST", "code_history_counts_commits_by_path_with_the_pinned_mailmap", || {
        let f = Fixture::new();
        f.run(&["config", "user.name", "Alias"]);
        f.run(&["config", "user.email", "old@example.invalid"]);
        f.write("src/a.rs", "1\n");
        f.commit("one");
        f.run(&["commit", "--allow-empty", "-m", "metadata only"]);
        f.run(&["config", "user.name", "林安"]);
        f.run(&["config", "user.email", "canonical@example.invalid"]);
        f.write(".mailmap", "林安 <canonical@example.invalid> Alias <old@example.invalid>\n");
        f.write("src/a.rs", "2\n");
        f.write("src/deep/b.rs", "b\n");
        f.commit("two");
        f.run(&["config", "user.name", "Other"]);
        f.run(&["config", "user.email", "other@example.invalid"]);
        f.write("docs/guide.md", "g\n");
        f.write("src/deep/b.rs", "b2\n");
        f.commit("three");
        // A merge commit is left out.
        f.run(&["checkout", "-b", "side", "HEAD~1"]);
        f.write("side.txt", "s\n");
        f.commit("side");
        f.run(&["checkout", "main"]);
        f.run(&["merge", "--no-ff", "-m", "merge side", "side"]);
        let tip = head(&f);
        let history = f.repo().code_history(&tip, &token()).unwrap();
        assert_eq!(history.commits.len(), 5, "include the empty commit, exclude the merge");
        assert!(!history.limited);
        assert_eq!(history.authors.len(), 2, "Alias is 林安 through the snapshot's .mailmap: {:?}", history.authors);
        assert_eq!(history.file_commits(b"src/a.rs", None), 2);
        assert_eq!(history.folder_commits(b"src", false, None), 3);
        assert_eq!(history.folder_commits(b"src", true, None), 2);
        let (owner, commits) = history.owner(b"src/a.rs").unwrap();
        assert_eq!(history.authors[owner as usize], "林安".as_bytes());
        assert_eq!(commits, 2);
        // Commits on the same day; a period after them all counts none.
        let newest = history.commits[0].time;
        assert_eq!(history.folder_commits(b"", false, Some(newest + 1)), 0);
    });
}

#[test]
fn code_history_preserves_control_prefixed_paths_in_git_objects() {
    isolated("ROCKET_CODE_MAP_TEST", "code_history_preserves_control_prefixed_paths_in_git_objects", || {
        let f = Fixture::new();
        f.write("data", "ordinary file content");
        let oid = String::from_utf8(f.run(&["hash-object", "-w", "data"])).unwrap();
        let oid = oid.trim();
        // Git trees can store these names even on systems that cannot check them out.
        // The second name follows the first path's NUL separator directly.
        let records = f.root.join("tree-input");
        let entries = ["\x01first", "\x01second"].map(|name| format!("100644 blob {oid}\t{name}\0"));
        std::fs::write(&records, entries.concat()).unwrap();
        let tree = f.command(&f.repository, &["mktree", "-z"])
            .stdin(std::fs::File::open(records).unwrap())
            .output()
            .unwrap();
        assert!(tree.status.success(), "{}", String::from_utf8_lossy(&tree.stderr));
        let tree_id = String::from_utf8(tree.stdout).unwrap();
        let commit = String::from_utf8(f.run(&["commit-tree", tree_id.trim(), "-m", "two paths"])).unwrap();
        let tip = ObjectId::parse(commit.trim()).unwrap();
        let history = f.repo().code_history(&tip, &token()).unwrap();
        assert_eq!(history.commits.len(), 1);
        assert_eq!(history.commits[0].id, tip);
        assert_eq!(history.paths.len(), 2);
        assert_eq!(history.file_commits(b"\x01first", None), 1);
        assert_eq!(history.file_commits(b"\x01second", None), 1);
    });
}

#[test]
fn code_changes_compare_from_where_branches_split() {
    isolated("ROCKET_CODE_MAP_TEST", "code_changes_compare_from_where_branches_split", || {
        let f = Fixture::new();
        f.write("src/a.rs", "one\ntwo\n");
        f.write("src/old.rs", "keep\n");
        f.write("gone.txt", "bye\n");
        f.commit("base");
        f.run(&["branch", "base"]);
        f.run(&["checkout", "-b", "feature"]);
        f.write("src/a.rs", "one\nTWO\nthree\n");
        f.run(&["mv", "src/old.rs", "src/new.rs"]);
        f.run(&["rm", "gone.txt"]);
        f.write("added.md", "new\n");
        f.commit("feature work");
        // The base moves on after the split; its change is not the feature's.
        f.run(&["checkout", "base"]);
        f.write("base-only.txt", "later\n");
        f.commit("base later");
        f.run(&["checkout", "feature"]);
        let tip = head(&f);
        let repo = f.repo();
        let changes = repo.code_changes("base", &tip, &token()).unwrap();
        assert_eq!(changes.outcome, ChangesOutcome::Ready);
        assert_eq!(changes.commits, 1);
        let by = |p: &str| changes.files.iter().find(|c| c.path.as_bytes() == p.as_bytes()).map(|c| c.change);
        assert_eq!(by("src/a.rs"), Some(Change::Modified));
        assert_eq!(by("src/new.rs"), Some(Change::Renamed));
        assert_eq!(by("gone.txt"), Some(Change::Deleted));
        assert_eq!(by("added.md"), Some(Change::Added));
        assert_eq!(by("base-only.txt"), None);
        assert_eq!(changes.lines.get(b"src/a.rs".as_slice()), Some(&(2, 1)));
        // Nothing changed since itself; unknown names and unrelated history are states.
        let same = repo.code_changes("feature", &tip, &token()).unwrap();
        assert_eq!((same.outcome, same.files.len()), (ChangesOutcome::Ready, 0));
        assert_eq!(repo.code_changes("no-such-branch", &tip, &token()).unwrap().outcome, ChangesOutcome::BaseNotFound);
        f.run(&["checkout", "--orphan", "unrelated"]);
        f.run(&["rm", "-r", "--cached", "--quiet", "."]);
        f.write("other.txt", "x\n");
        f.run(&["add", "other.txt"]);
        f.run(&["commit", "--no-verify", "-m", "unrelated"]);
        f.run(&["checkout", "-f", "feature"]);
        assert_eq!(repo.code_changes("unrelated", &tip, &token()).unwrap().outcome, ChangesOutcome::NoCommonHistory);
    });
}
