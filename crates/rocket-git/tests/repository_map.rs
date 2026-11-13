mod common;
use common::{Fixture, isolated};
use rocket_git::*;
fn token() -> CancellationToken {
    CancellationToken::new()
}

#[test]
fn history_link_budget_refuses_dense_initial_and_paged_snapshots() {
    isolated(
        "ROCKET_MAP_TEST",
        "history_link_budget_refuses_dense_initial_and_paged_snapshots",
        || {
            use std::io::Write;
            use std::process::Stdio;

            let f = Fixture::new();
            // Real commits, each well below the 1,024-parent parsing limit.
            // Commit 156 has exactly 12,000 aggregate links; 157 adds one.
            let mut input = String::new();
            for id in 1..=157 {
                let message = format!("dense history {id}");
                input.push_str(&format!(
                    "commit refs/heads/main\nmark :{id}\ncommitter Map Fixture <map@example.invalid> {} +0000\ndata {}\n{message}\n",
                    1_735_689_600 + id,
                    message.len()
                ));
                if id > 1 {
                    input.push_str(&format!("from :{}\n", id - 1));
                    let parents = match id {
                        156 => 65,
                        157 => 1,
                        _ => id - 1,
                    };
                    for parent in 1..parents {
                        input.push_str(&format!("merge :{parent}\n"));
                    }
                }
                input.push('\n');
            }
            input.push_str("reset refs/heads/at-limit\nfrom :156\n\ndone\n");
            let mut child = f
                .command(&f.repository, &["fast-import", "--quiet", "--done"])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            child
                .stdin
                .take()
                .unwrap()
                .write_all(input.as_bytes())
                .unwrap();
            let output = child.wait_with_output().unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            f.write("untracked-preserve", "local fixture data");
            let head = f.run(&["rev-parse", "HEAD"]);
            let status = f.run(&["status", "--porcelain=v1", "-z"]);
            let repo = f.repo();
            let options = MapOptions {
                kind: MapKind::History,
                revision: "at-limit".into(),
                ..Default::default()
            };
            let accepted = repo.repository_map(&options, &token()).unwrap();
            assert_eq!(accepted.nodes.len(), 156);
            assert_eq!(accepted.links.len(), 12_000);
            assert!(!accepted.limited && !accepted.has_more);
            assert!(accepted.continuation.is_none());

            let excessive = MapOptions {
                revision: "HEAD".into(),
                ..options.clone()
            };
            let error = repo
                .repository_map(&excessive, &token())
                .map(|map| map.links.len())
                .expect_err("history with 12,001 parent links was accepted");
            assert!(
                matches!(error, Error::Unsupported(ref message) if message.contains("12,000 parent links"))
            );

            let first = repo
                .repository_map(
                    &MapOptions {
                        page_size: 100,
                        ..excessive
                    },
                    &token(),
                )
                .unwrap();
            assert_eq!(first.nodes.len(), 100);
            assert!(first.links.len() < 12_000 && first.has_more);
            assert!(first.continuation.is_some());
            let cancelled = token();
            cancelled.cancel();
            assert!(matches!(
                repo.repository_map_more(&first, &cancelled),
                Err(Error::Cancelled)
            ));
            let error = repo
                .repository_map_more(&first, &token())
                .map(|map| map.links.len())
                .expect_err("Load more accepted 12,001 parent links");
            assert!(
                matches!(error, Error::Unsupported(ref message) if message.contains("12,000 parent links"))
            );
            assert_eq!(first.nodes.len(), 100);
            assert!(first.continuation.is_some());

            let first = repo
                .repository_map(
                    &MapOptions {
                        page_size: 100,
                        ..options
                    },
                    &token(),
                )
                .unwrap();
            let accepted = repo.repository_map_more(&first, &token()).unwrap();
            assert_eq!(accepted.links.len(), 12_000);
            assert!(!accepted.has_more && accepted.continuation.is_none());
            assert_eq!(head, f.run(&["rev-parse", "HEAD"]));
            assert_eq!(status, f.run(&["status", "--porcelain=v1", "-z"]));
            assert_eq!(
                std::fs::read(f.repository.join("untracked-preserve")).unwrap(),
                b"local fixture data"
            );
        },
    );
}

#[test]
fn scoped_pagination_is_literal_pinned_and_cancelled_without_changing_checkout() {
    isolated(
        "ROCKET_MAP_TEST",
        "scoped_pagination_is_literal_pinned_and_cancelled_without_changing_checkout",
        || {
            let f = Fixture::new();
            f.write("base", "base");
            f.commit("base");
            let base = String::from_utf8(f.run(&["rev-parse", "HEAD"]))
                .unwrap()
                .trim()
                .to_owned();
            for i in 0..7 {
                f.write("应用/[文] 🚀.rs", format!("{i}\n"));
                f.commit(&format!("scope {i}"));
            }
            // Exercise the same command policy used by localhost MCP.
            let repo = f
                .git
                .clone()
                .with_transport_scope(Some(TransportScope::new(&f.root, vec![]).unwrap()))
                .open(&f.repository, &token())
                .unwrap();
            let first = repo
                .repository_map(
                    &MapOptions {
                        kind: MapKind::History,
                        base: base.clone(),
                        path: Some(RepoPath::new("应用/[文] 🚀.rs".as_bytes().to_vec()).unwrap()),
                        page_size: 2,
                        ..Default::default()
                    },
                    &token(),
                )
                .unwrap();
            assert_eq!(first.nodes.len(), 2);
            assert!(first.has_more);
            assert!(first.boundary_parents > 0);
            f.write("unrelated", "external");
            f.commit("outside pinned snapshot");
            let head = f.run(&["rev-parse", "HEAD"]);
            f.write("dirty", "keep");
            let index = f.run(&["ls-files", "--stage", "-z"]);
            let mut next = first.clone();
            while next.continuation.is_some() {
                next = repo.repository_map_more(&next, &token()).unwrap();
            }
            assert_eq!(next.nodes.len(), 7);
            assert_eq!(next.tip, first.tip);
            assert!(!next.has_more);
            assert!(
                !next
                    .nodes
                    .iter()
                    .any(|n| n.label == b"outside pinned snapshot" || n.label == b"base")
            );
            let empty = repo
                .repository_map(
                    &MapOptions {
                        kind: MapKind::History,
                        path: Some(RepoPath::new("应用/*.rs".as_bytes().to_vec()).unwrap()),
                        ..Default::default()
                    },
                    &token(),
                )
                .unwrap();
            assert!(empty.nodes.is_empty(), "wildcard must be literal");
            let cancelled = token();
            cancelled.cancel();
            assert!(matches!(
                repo.repository_map_more(&first, &cancelled),
                Err(Error::Cancelled)
            ));
            assert!(
                Fixture::new()
                    .repo()
                    .repository_map_more(&first, &token())
                    .is_err()
            );
            assert_eq!(f.run(&["rev-parse", "HEAD"]), head);
            assert_eq!(f.run(&["ls-files", "--stage", "-z"]), index);
            assert_eq!(std::fs::read(f.repository.join("dirty")).unwrap(), b"keep");
        },
    );
}
fn read(f: &Fixture, kind: MapKind) -> RepositoryMap {
    f.repo()
        .repository_map(
            &MapOptions {
                kind,
                ..Default::default()
            },
            &token(),
        )
        .unwrap()
}

#[test]
fn committed_code_unicode_dependencies_and_worktree_safety() {
    isolated(
        "ROCKET_MAP_TEST",
        "committed_code_unicode_dependencies_and_worktree_safety",
        || {
            let f = Fixture::new();
            f.write("Cargo.toml", "[workspace]\nmembers=['应用','core']\n");
            f.write("应用/Cargo.toml","[package]\nname='界面'\n[dependencies]\ncore={path='../core'}\n# ignored={path='../secret'}\n");
            f.write("core/Cargo.toml", "[package]\nname='engine'\n");
            f.write("应用/src/結果 🚀 é.rs", "fn main() {}\n");
            f.commit("map base");
            f.write("应用/src/結果 🚀 é.rs", "staged local change\n");
            f.run(&["add", "--all"]);
            f.write("应用/src/結果 🚀 é.rs", "unstaged local change\n");
            f.write("private-untracked", "keep");
            let head = f.run(&["rev-parse", "HEAD"]);
            let index = f.run(&["ls-files", "--stage", "-z"]);
            let config = std::fs::read(f.repository.join(".git/config")).unwrap();
            let map = read(&f, MapKind::Code);
            let ui = map
                .nodes
                .iter()
                .position(|n| n.label == "界面".as_bytes())
                .unwrap();
            let core = map.nodes.iter().position(|n| n.label == b"engine").unwrap();
            assert!(
                map.links
                    .iter()
                    .any(|e| e.from == ui && e.to == core && e.kind == MapLinkKind::DependsOn)
            );
            assert!(
                map.nodes
                    .iter()
                    .any(|n| n.path.as_deref() == Some("应用/src/結果 🚀 é.rs".as_bytes()))
            );
            assert!(!map.nodes.iter().any(|n| n.label == b"private-untracked"));
            assert_eq!(f.run(&["rev-parse", "HEAD"]), head);
            assert_eq!(f.run(&["ls-files", "--stage", "-z"]), index);
            assert_eq!(
                std::fs::read(f.repository.join(".git/config")).unwrap(),
                config
            );
            assert_eq!(
                std::fs::read_to_string(f.repository.join("应用/src/結果 🚀 é.rs")).unwrap(),
                "unstaged local change\n"
            );
            f.write("later", "next");
            f.commit("next");
            let again = read(&f, MapKind::Code);
            assert_ne!(map.tip, again.tip);
            assert!(!map.nodes.iter().any(|n| n.label == b"later"));
            assert!(again.nodes.iter().any(|n| n.label == b"later"));
        },
    );
}

#[test]
fn history_parent_links_branches_merge_and_literal_paths() {
    isolated(
        "ROCKET_MAP_TEST",
        "history_parent_links_branches_merge_and_literal_paths",
        || {
            let f = Fixture::new();
            f.write("same.txt", "base");
            f.commit("root");
            f.run(&["switch", "-c", "side"]);
            f.write("branch 文", "side");
            f.commit("side commit");
            let side = f.run(&["rev-parse", "HEAD"]);
            f.run(&["switch", "main"]);
            f.write("main.txt", "main");
            f.commit("main commit");
            let main = read(&f, MapKind::History);
            assert_eq!(main.nodes.len(), 2);
            let all = f
                .repo()
                .repository_map(
                    &MapOptions {
                        kind: MapKind::History,
                        all_branches: true,
                        ..Default::default()
                    },
                    &token(),
                )
                .unwrap();
            assert_eq!(all.nodes.len(), 3);
            f.run(&["merge", "--no-ff", "side", "-m", "join"]);
            let merged = read(&f, MapKind::History);
            let merge = merged
                .nodes
                .iter()
                .position(|n| n.label == b"join")
                .unwrap();
            assert_eq!(merged.nodes[merge].count, 2);
            assert_eq!(merged.links.iter().filter(|e| e.to == merge).count(), 2);
            assert!(
                merged
                    .nodes
                    .iter()
                    .any(|n| n.commit.as_ref().unwrap().as_str().as_bytes() == side.trim_ascii())
            );
            assert!(
                merged
                    .nodes
                    .iter()
                    .any(|n| n.evidence.iter().any(|p| p == "branch 文".as_bytes()))
            );
            let mut options = MapOptions {
                kind: MapKind::History,
                revision: "--all".into(),
                ..Default::default()
            };
            assert!(f.repo().repository_map(&options, &token()).is_err());
            options.revision = "side".into();
            let side_map = f.repo().repository_map(&options, &token()).unwrap();
            assert_eq!(side_map.nodes.len(), 2);
        },
    );
}

#[test]
fn people_mailmap_shared_paths_period_and_renames() {
    isolated(
        "ROCKET_MAP_TEST",
        "people_mailmap_shared_paths_period_and_renames",
        || {
            let f = Fixture::new();
            f.run(&["config", "user.name", "Alias"]);
            f.run(&["config", "user.email", "old@example.invalid"]);
            f.write("src/検索.txt", "first");
            f.commit("one");
            f.run(&["config", "user.name", "林安"]);
            f.run(&["config", "user.email", "canonical@example.invalid"]);
            f.write(
                ".mailmap",
                "林安 <canonical@example.invalid> Alias <old@example.invalid>\n",
            );
            f.write("src/検索.txt", "second");
            f.commit("two");
            f.run(&["config", "user.name", "Other"]);
            f.run(&["config", "user.email", "other@example.invalid"]);
            f.write("src/検索.txt", "third");
            f.commit("three");
            f.run(&["mv", "src/検索.txt", "src/新しい.txt"]);
            f.commit("rename");
            let map = read(&f, MapKind::People);
            let people = map
                .nodes
                .iter()
                .filter(|n| n.kind == MapNodeKind::Person)
                .collect::<Vec<_>>();
            assert_eq!(people.len(), 2);
            assert_eq!(
                people
                    .iter()
                    .find(|n| n.label == "林安".as_bytes())
                    .unwrap()
                    .count,
                2
            );
            let shared = map
                .links
                .iter()
                .find(|e| e.kind == MapLinkKind::SharedFiles)
                .unwrap();
            assert_eq!(shared.weight, 1);
            assert!(
                shared
                    .evidence
                    .iter()
                    .any(|p| p == "src/検索.txt".as_bytes())
            );
            assert!(
                map.nodes
                    .iter()
                    .any(|n| n.evidence.iter().any(|p| p == "src/新しい.txt".as_bytes()))
            );
            let empty = f
                .repo()
                .repository_map(
                    &MapOptions {
                        kind: MapKind::People,
                        since: Some(i64::MAX),
                        ..Default::default()
                    },
                    &token(),
                )
                .unwrap();
            assert!(empty.nodes.is_empty());
            // Uncommitted mailmap changes never affect the pinned identity grouping.
            f.write(
                ".mailmap",
                "Wrong <wrong@example.invalid> <old@example.invalid>\n",
            );
            let again = read(&f, MapKind::People);
            assert_eq!(
                again
                    .nodes
                    .iter()
                    .filter(|n| n.kind == MapNodeKind::Person)
                    .count(),
                2
            );
        },
    );
}

fn modules(f: &Fixture, package: Option<&str>) -> Result<RepositoryMap> {
    f.repo().repository_map(
        &MapOptions {
            kind: MapKind::Modules,
            package: package.map(|p| p.as_bytes().to_vec()),
            ..Default::default()
        },
        &token(),
    )
}

#[test]
fn committed_rust_modules_connections_and_unlinked_files() {
    isolated(
        "ROCKET_MAP_TEST",
        "committed_rust_modules_connections_and_unlinked_files",
        || {
            let f = Fixture::new();
            f.write("Cargo.toml", "[workspace]\nmembers=['crates/ui','crates/core']\n");
            f.write("crates/ui/Cargo.toml", "[package]\nname='界面'\n");
            f.write("crates/core/Cargo.toml", "[package]\nname='engine'\n");
            f.write("crates/core/src/lib.rs", "pub fn run() {}\n");
            f.write(
                "crates/ui/src/lib.rs",
                "mod app;\npub mod 検索;\n#[cfg(test)]\nmod tests;\n#[path = \"../../shared/common.rs\"]\nmod common;\npub use app::Rocket;\n",
            );
            f.write("crates/ui/src/app.rs", "pub struct Rocket;\nimpl Rocket { pub fn new() -> Self { Self } fn draw(&self) {} }\npub fn start() { crate::検索::find(); }\n");
            f.write("crates/ui/src/検索.rs", "use crate::app::*;\npub fn find() { let _ = Rocket::new(); start(); common_name(); }\nfn common_name() { crate::common::helper(); }\n");
            f.write("crates/ui/src/tests.rs", "use super::*;\n#[test]\nfn works() { let _ = Rocket; }\n");
            f.write("crates/shared/common.rs", "pub fn helper() {}\n");
            f.write("crates/ui/src/main.rs", "mod cli;\nfn main() { 界面::app::start(); }\n");
            f.write("crates/ui/src/cli.rs", "pub fn args() {}\n");
            f.write("crates/ui/src/bin/tool.rs", "fn main() {}\n");
            f.write("crates/ui/src/stray.rs", "// never declared\n");
            f.write("crates/ui/src/latin1.rs", [0xe9u8, 0x0a]);
            f.commit("modules");
            f.write("crates/ui/src/app.rs", "uncommitted local change\n");
            f.write("crates/ui/src/untracked.rs", "fn private() {}\n");
            let head = f.run(&["rev-parse", "HEAD"]);
            let index = f.run(&["ls-files", "--stage", "-z"]);

            // Without a package, the one with the most Rust files is read.
            let map = modules(&f, None).unwrap();
            assert_eq!(map.kind, MapKind::Modules);
            assert_eq!(map.scope.package.as_deref(), Some(b"crates/ui".as_slice()));
            let packages: Vec<(&[u8], usize)> = map
                .nodes
                .iter()
                .filter(|n| n.kind == MapNodeKind::Crate)
                .map(|n| (n.label.as_slice(), n.count))
                .collect();
            assert_eq!(packages, [(b"engine".as_slice(), 1), ("界面".as_bytes(), 9)]);
            let details = map.modules.as_ref().unwrap();
            assert_eq!(map.nodes[details.package].label, "界面".as_bytes());
            let module = |name: &str| {
                map.nodes
                    .iter()
                    .position(|n| n.kind == MapNodeKind::Module && n.key == format!("module:{name}").as_bytes())
                    .unwrap_or_else(|| panic!("no module {name}"))
            };
            let root = details.root.unwrap();
            assert_eq!(root, module(""));
            assert_eq!(map.nodes[root].label, b"lib.rs");
            assert_eq!(map.nodes[module("common")].path.as_deref(), Some(b"crates/shared/common.rs".as_slice()));
            assert_eq!(map.nodes[module("tests")].detail, b"Test module");
            // The committed file, not the working tree's change.
            assert_eq!(map.nodes[module("app")].count, 3);
            assert!(!map.nodes.iter().any(|n| n.path.as_deref() == Some(b"crates/ui/src/untracked.rs".as_slice())));
            let uses = |from: &str, to: &str| {
                map.links
                    .iter()
                    .find(|e| e.kind == MapLinkKind::DependsOn && e.from == module(from) && e.to == module(to))
                    .map(|e| e.evidence.iter().map(|n| String::from_utf8_lossy(n).into_owned()).collect::<Vec<_>>())
            };
            assert_eq!(uses("", "app"), Some(vec!["Rocket".into()]));
            assert_eq!(uses("app", "検索"), Some(vec!["find".into()]));
            assert_eq!(uses("検索", "app"), Some(vec!["Rocket".into(), "start".into()]));
            assert_eq!(uses("検索", "common"), Some(vec!["helper".into()]));
            assert_eq!(uses("tests", "app"), Some(vec!["Rocket".into()]));
            assert!(uses("app", "").is_none());
            assert!(
                map.links
                    .iter()
                    .any(|e| e.kind == MapLinkKind::Contains && e.from == root && e.to == module("検索"))
            );
            let outline: Vec<String> = details.outline[&module("app")]
                .iter()
                .map(|o| format!("{:?} {}", o.kind, String::from_utf8_lossy(&o.name)))
                .collect();
            assert_eq!(outline, ["Struct Rocket", "Impl Rocket", "Function start"]);
            let unlinked: Vec<(String, UnlinkedReason)> = details
                .unlinked
                .iter()
                .map(|(p, r)| (String::from_utf8_lossy(p).into_owned(), *r))
                .collect();
            assert_eq!(
                unlinked,
                [
                    ("crates/ui/src/bin/tool.rs".into(), UnlinkedReason::Program),
                    ("crates/ui/src/cli.rs".into(), UnlinkedReason::Program),
                    ("crates/ui/src/latin1.rs".into(), UnlinkedReason::NotUtf8),
                    ("crates/ui/src/main.rs".into(), UnlinkedReason::Program),
                    ("crates/ui/src/stray.rs".into(), UnlinkedReason::NotDeclared),
                ]
            );

            // A chosen package, and one the snapshot does not have.
            let core = modules(&f, Some("crates/core")).unwrap();
            assert_eq!(core.nodes.iter().filter(|n| n.kind == MapNodeKind::Module).count(), 1);
            assert!(matches!(modules(&f, Some("crates/gone")), Err(Error::InvalidInput(_))));
            let cancel = token();
            cancel.cancel();
            assert!(matches!(
                f.repo().repository_map(&MapOptions { kind: MapKind::Modules, ..Default::default() }, &cancel),
                Err(Error::Cancelled)
            ));
            // Reading changed nothing.
            assert_eq!(f.run(&["rev-parse", "HEAD"]), head);
            assert_eq!(f.run(&["ls-files", "--stage", "-z"]), index);
            assert_eq!(std::fs::read(f.repository.join("crates/ui/src/app.rs")).unwrap(), b"uncommitted local change\n");
        },
    );
}

#[test]
fn a_snapshot_without_rust_packages_has_no_modules() {
    isolated("ROCKET_MAP_TEST", "a_snapshot_without_rust_packages_has_no_modules", || {
        let f = Fixture::new();
        f.write("README.md", "no packages");
        f.write("Cargo.toml", "[workspace]\n");
        f.commit("plain");
        let map = modules(&f, None).unwrap();
        assert!(map.nodes.is_empty() && map.modules.is_none());
    });
}

#[test]
fn empty_cancelled_and_bounded_code_maps() {
    isolated(
        "ROCKET_MAP_TEST",
        "empty_cancelled_and_bounded_code_maps",
        || {
            let f = Fixture::new();
            for kind in [MapKind::Code, MapKind::History, MapKind::People] {
                assert!(read(&f, kind).nodes.is_empty());
                let cancel = token();
                cancel.cancel();
                assert!(matches!(
                    f.repo().repository_map(
                        &MapOptions {
                            kind,
                            ..Default::default()
                        },
                        &cancel
                    ),
                    Err(Error::Cancelled)
                ));
            }
            for n in 0..MAP_NODES + 2 {
                f.write(&format!("files/{n}.rs"), "data");
            }
            f.commit("large tree");
            let map = read(&f, MapKind::Code);
            assert!(map.limited);
            assert_eq!(map.nodes.len(), MAP_NODES);
            assert_eq!(map.scanned, MAP_NODES + 2);
            assert!(
                map.links
                    .iter()
                    .all(|e| e.from < map.nodes.len() && e.to < map.nodes.len())
            );
        },
    );
}
