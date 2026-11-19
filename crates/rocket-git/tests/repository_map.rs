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
            package: package.map(|p| PackageRef {
                folder: p.as_bytes().to_vec(),
                language: ModuleLanguage::Rust,
            }),
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
            f.write("crates/ui/src/検索.rs", "use crate::app::*;\nuse std::fmt;\npub fn find() { let _ = Rocket::new(); start(); common_name(); }\nfn common_name() { crate::common::helper(); }\n");
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
            assert_eq!(map.scope.package.as_ref().map(|p| p.folder.as_slice()), Some(b"crates/ui".as_slice()));
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
            // Another crate a use line names is listed as outside; the package's own modules are not.
            assert_eq!(details.outside.get(&module("検索")), Some(&vec![b"std".to_vec()]));
            assert!(!details.outside.contains_key(&module("tests")));
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
fn committed_java_kotlin_c_and_csharp_projects() {
    isolated("ROCKET_MAP_TEST", "committed_java_kotlin_c_and_csharp_projects", || {
        let f = Fixture::new();
        // A Gradle build: a Kotlin app using a Java library, each its own subproject.
        f.write("jvm/settings.gradle.kts", "rootProject.name = \"shop\"\ninclude(\":app\", \":lib\")\n");
        f.write("jvm/app/build.gradle.kts", "plugins { kotlin(\"jvm\") }\n");
        f.write("jvm/app/src/main/kotlin/com/acme/app/Main.kt", "package com.acme.app\n\nimport com.acme.app.ui.Screen\nimport com.acme.lib.store.Db\nimport kotlinx.coroutines.runBlocking\n\nfun main() = runBlocking { Screen().show(Db.open()) }\n");
        f.write("jvm/app/src/main/kotlin/com/acme/app/ui/Screen.kt", "package com.acme.app.ui\n\nclass Screen {\n    fun show(x: Any) {}\n    fun hide() {}\n}\n");
        f.write("jvm/app/src/test/kotlin/com/acme/app/MainTest.kt", "package com.acme.app\n\nimport org.junit.Test\n");
        f.write("jvm/lib/build.gradle.kts", "plugins { `java-library` }\n");
        f.write("jvm/lib/src/main/java/com/acme/lib/store/Db.java", "package com.acme.lib.store;\n\nimport java.util.List;\n\npublic final class Db {\n    public static Db open() { return new Db(); }\n    public void close() {}\n}\n");
        // A CMake project in C: the build below src is part of it.
        f.write("native/CMakeLists.txt", "cmake_minimum_required(VERSION 3.20)\nproject(acme C)\nadd_subdirectory(src)\n");
        f.write("native/src/CMakeLists.txt", "add_library(acme acme.c)\n");
        f.write("native/include/acme/acme.h", "#ifndef ACME_H\n#define ACME_H\n#include <stddef.h>\nint acme_open(const char *path);\n#endif\n");
        f.write("native/src/acme.c", "#include \"acme/acme.h\"\n#include \"util.h\"\n\nint acme_open(const char *path) { return (int)util_len(path); }\n");
        f.write("native/src/util.h", "static size_t util_len(const char *s) { return 0; }\n");
        // A C# solution of two projects.
        f.write("dotnet/Acme.Web/Acme.Web.csproj", "<Project Sdk=\"Microsoft.NET.Sdk.Web\" />\n");
        f.write("dotnet/Acme.Web/Controllers/HomeController.cs", "using Acme.Core.Models;\nusing Microsoft.AspNetCore.Mvc;\n\nnamespace Acme.Web.Controllers;\n\npublic class HomeController : Controller\n{\n    public IActionResult Index() => View(new User(\"a\"));\n}\n");
        f.write("dotnet/Acme.Web/Program.cs", "var app = WebApplication.Create(args);\napp.Run();\n");
        f.write("dotnet/Acme.Core/Acme.Core.csproj", "<Project Sdk=\"Microsoft.NET.Sdk\" />\n");
        f.write("dotnet/Acme.Core/Models/User.cs", "namespace Acme.Core.Models;\n\npublic record User(string Name);\n");
        f.commit("languages");
        let package = |folder: &str, language| PackageRef { folder: folder.as_bytes().to_vec(), language };
        let read = |choice: Option<PackageRef>| f.repo().repository_map(&MapOptions { kind: MapKind::Modules, package: choice, ..Default::default() }, &token()).unwrap();

        let code = f.repo().repository_map(&MapOptions { kind: MapKind::Code, ..Default::default() }, &token()).unwrap();
        let found: Vec<(String, ModuleLanguage, Option<Dialect>, usize)> =
            code.module_packages.iter().map(|p| (String::from_utf8_lossy(&p.folder).into_owned(), p.language, p.dialect, p.files)).collect();
        assert_eq!(
            found,
            [
                // The root build holds no file of its own; the test source set is left out.
                ("jvm/app".into(), ModuleLanguage::Java, Some(Dialect::Kotlin), 2),
                ("jvm/lib".into(), ModuleLanguage::Java, None, 1),
                ("native".into(), ModuleLanguage::C, None, 3),
                ("dotnet/Acme.Core".into(), ModuleLanguage::CSharp, None, 1),
                ("dotnet/Acme.Web".into(), ModuleLanguage::CSharp, None, 2),
            ]
        );

        // Three languages with three files each: Java comes first, then its largest project.
        let map = read(None);
        assert_eq!(map.scope.package, Some(package("jvm/app", ModuleLanguage::Java)));
        let names: Vec<(String, String)> = map
            .nodes
            .iter()
            .filter(|n| n.kind == MapNodeKind::Crate)
            .map(|n| (String::from_utf8_lossy(&n.label).into_owned(), String::from_utf8_lossy(&n.detail).into_owned()))
            .collect();
        assert_eq!(
            names,
            [
                ("app".into(), "Kotlin project".into()),
                ("lib".into(), "Java project".into()),
                ("acme".into(), "C project".into()),
                ("Acme.Core".into(), "C# project".into()),
                ("Acme.Web".into(), "C# project".into()),
            ]
        );
        let module = |map: &RepositoryMap, key: &str| map.nodes.iter().position(|n| n.key == format!("module:{key}").as_bytes()).unwrap_or_else(|| panic!("no module {key}"));
        let uses = |map: &RepositoryMap, from: &str, to: &str| {
            map.links
                .iter()
                .find(|e| e.kind == MapLinkKind::DependsOn && e.from == module(map, from) && e.to == module(map, to))
                .map(|e| e.evidence.iter().map(|n| String::from_utf8_lossy(n).into_owned()).collect::<Vec<_>>())
        };
        let outside = |map: &RepositoryMap, key: &str| -> Vec<String> {
            map.modules.as_ref().unwrap().outside.get(&module(map, key)).map(|o| o.iter().map(|n| String::from_utf8_lossy(n).into_owned()).collect()).unwrap_or_default()
        };
        let details = map.modules.as_ref().unwrap();
        assert_eq!((details.language, details.root), (ModuleLanguage::Java, None));
        assert_eq!(uses(&map, "com.acme.app", "com.acme.app.ui"), Some(vec!["Screen".into()]));
        assert_eq!(map.nodes[module(&map, "com.acme.app.ui")].label, b"ui");
        assert_eq!(map.nodes[module(&map, "com.acme.app.ui")].detail, b"Java package");
        // The library is another project: its package is outside this one.
        assert_eq!(outside(&map, "com.acme.app"), ["com.acme.lib.store", "kotlinx.coroutines"]);
        let outline: Vec<String> = details.outline[&module(&map, "com.acme.app.ui")].iter().map(|o| format!("{:?} {} {}", o.kind, String::from_utf8_lossy(&o.name), o.functions)).collect();
        assert_eq!(outline, ["Class Screen 2"]);

        // C: files and folders; includes beside the file and by the end of their path.
        let map = read(Some(package("native", ModuleLanguage::C)));
        assert_eq!(map.modules.as_ref().unwrap().language, ModuleLanguage::C);
        assert_eq!(uses(&map, "src/acme.c", "include/acme/acme.h"), Some(vec!["acme_open".into()]));
        assert_eq!(uses(&map, "src/acme.c", "src/util.h"), Some(vec!["util_len".into()]));
        assert_eq!(outside(&map, "include/acme/acme.h"), ["stddef.h"]);
        assert_eq!(map.nodes[module(&map, "src/acme.c")].detail, b"C source");

        // C#: namespaces; top-level statements are in the global namespace.
        let map = read(Some(package("dotnet/Acme.Web", ModuleLanguage::CSharp)));
        assert_eq!(map.modules.as_ref().unwrap().language, ModuleLanguage::CSharp);
        assert_eq!(map.nodes[module(&map, "")].label, b"(global namespace)");
        assert_eq!(outside(&map, "Acme.Web.Controllers"), ["Acme.Core.Models", "Microsoft.AspNetCore.Mvc"]);

        // The code map joins the projects: an import or a using line names the other's folder.
        let tip = ObjectId::parse(String::from_utf8(f.run(&["rev-parse", "HEAD"])).unwrap().trim()).unwrap();
        let links = f.repo().code_links(&tip, None, &token()).unwrap();
        let name = |i: usize| {
            let e = &links.ends[i];
            format!("{}{}", String::from_utf8_lossy(&e.path), if e.folder { "/" } else { "" })
        };
        let mut pairs: Vec<String> = links.links.iter().map(|l| format!("{} -> {}", name(l.from), name(l.to))).collect();
        pairs.sort();
        assert_eq!(
            pairs,
            [
                "dotnet/Acme.Web/Controllers/ -> dotnet/Acme.Core/Models/",
                "jvm/app/src/main/kotlin/com/acme/app/ -> jvm/app/src/main/kotlin/com/acme/app/ui/",
                "jvm/app/src/main/kotlin/com/acme/app/ -> jvm/lib/src/main/java/com/acme/lib/store/",
                "native/src/acme.c -> native/include/acme/acme.h",
                "native/src/acme.c -> native/src/util.h",
            ]
        );
    });
}

#[test]
fn source_discovery_keeps_production_java_and_kotlin_folders() {
    isolated("ROCKET_MAP_TEST", "source_discovery_keeps_production_java_and_kotlin_folders", || {
        let f = Fixture::new();
        f.write("build.gradle.kts", concat!(
            "plugins { kotlin(\"jvm\") }\n",
            "sourceSets.main { java.srcDir(\"src/latest/java\") }\n",
            "kotlin.sourceSets.named(\"main\") { kotlin.srcDir(\"src/contest/kotlin\") }\n",
        ));
        f.write("src/main/java/core/Stable.java", "package core;\npublic class Stable {}\n");
        f.write("src/latest/java/latest/Latest.java", "package latest;\npublic class Latest {}\n");
        f.write("src/contest/kotlin/contest/Game.kt", "package contest\nclass Game\n");
        for set in ["test", "testFixtures", "jvmTest", "androidTestDebug", "integration-test"] {
            f.write(&format!("src/{set}/java/excluded/Check.java"), "package excluded;\nclass Check {}\n");
        }
        f.commit("production and test source sets");

        let code = read(&f, MapKind::Code);
        assert_eq!(code.module_packages[0].files, 3);
        let modules = read(&f, MapKind::Modules);
        let mut packages: Vec<_> = modules.nodes.iter().filter(|n| n.detail == b"Java package").map(|n| std::str::from_utf8(&n.label).unwrap()).collect();
        packages.sort_unstable();
        assert_eq!(packages, ["contest", "core", "latest"]);
    });
}

#[test]
fn source_discovery_recognizes_cpp_interfaces_and_uppercase_c() {
    isolated("ROCKET_MAP_TEST", "source_discovery_recognizes_cpp_interfaces_and_uppercase_c", || {
        let f = Fixture::new();
        f.write("modules/CMakeLists.txt", concat!(
            "cmake_minimum_required(VERSION 3.28)\nproject(math LANGUAGES CXX)\nadd_library(math)\n",
            "target_compile_features(math PUBLIC cxx_std_20)\n",
            "target_sources(math PUBLIC FILE_SET CXX_MODULES FILES math.cppm)\n",
        ));
        f.write("modules/math.cppm", "export module math;\nexport int sum(int a, int b) { return a + b; }\n");
        f.write("classic/CMakeLists.txt", "project(classic LANGUAGES CXX)\nadd_library(classic legacy.C)\n");
        f.write("classic/legacy.C", "class Legacy {};\n");
        f.write("plain/CMakeLists.txt", "project(plain LANGUAGES C)\nadd_library(plain plain.c)\n");
        f.write("plain/plain.c", "int plain(void) { return 1; }\n");
        f.commit("C++ modules, uppercase C++ source and plain C");

        let code = read(&f, MapKind::Code);
        let projects: Vec<_> = code.module_packages.iter().map(|p| (std::str::from_utf8(&p.folder).unwrap(), p.dialect, p.files)).collect();
        assert_eq!(projects, [("classic", Some(Dialect::Cpp), 1), ("modules", Some(Dialect::Cpp), 1), ("plain", None, 1)]);
        let repo = f.repo();
        for (folder, path, detail) in [
            ("classic", "classic/legacy.C", "C++ source"),
            ("modules", "modules/math.cppm", "C++ source"),
            ("plain", "plain/plain.c", "C source"),
        ] {
            let options = MapOptions {
                kind: MapKind::Modules,
                package: Some(PackageRef { folder: folder.as_bytes().to_vec(), language: ModuleLanguage::C }),
                ..Default::default()
            };
            let modules = repo.repository_map(&options, &token()).unwrap();
            let file = modules.nodes.iter().find(|n| n.path.as_deref() == Some(path.as_bytes())).unwrap_or_else(|| panic!("missing {path}"));
            assert_eq!(file.detail, detail.as_bytes(), "{path}");
        }
    });
}

#[test]
fn code_links_name_packages_whole_reach_the_top_folder_and_survive_folder_reads() {
    isolated("ROCKET_MAP_TEST", "code_links_name_packages_whole_reach_the_top_folder_and_survive_folder_reads", || {
        let f = Fixture::new();
        // Maven modules: a uses b; web imports a library's com.acme.payments, beside app's com.acme.
        for module in ["a", "b", "app", "web"] {
            f.write(&format!("{module}/pom.xml"), format!("<project><artifactId>{module}</artifactId></project>\n"));
        }
        f.write("a/src/main/java/com/acme/a/A.java", "package com.acme.a;\n\nimport com.acme.b.B;\n\npublic class A { B b; }\n");
        f.write("b/src/main/java/com/acme/b/B.java", "package com.acme.b;\n\npublic class B {}\n");
        f.write("app/src/main/java/com/acme/Application.java", "package com.acme;\n\npublic class Application {}\n");
        f.write("web/src/main/java/com/acme/web/Web.java", "package com.acme.web;\n\nimport com.acme.payments.Gateway;\n\npublic class Web { Gateway gateway; }\n");
        // A C# project at the top of the repository.
        f.write("App.csproj", "<Project Sdk=\"Microsoft.NET.Sdk\" />\n");
        f.write("Program.cs", "using App.Models;\n\nnamespace App;\n\nclass Program { User user; }\n");
        f.write("Models/User.cs", "namespace App.Models;\n\npublic class User {}\n");
        f.commit("jvm and dotnet");
        let tip = ObjectId::parse(String::from_utf8(f.run(&["rev-parse", "HEAD"])).unwrap().trim()).unwrap();
        let pairs = |links: &CodeLinks| {
            let name = |i: usize| {
                let e = &links.ends[i];
                format!("{}{}", String::from_utf8_lossy(&e.path), if e.folder { "/" } else { "" })
            };
            let mut out: Vec<String> = links.links.iter().map(|l| format!("{} -> {}", name(l.from), name(l.to))).collect();
            out.sort();
            out
        };
        let all = f.repo().code_links(&tip, None, &token()).unwrap();
        // No arrow from web to app: com.acme.payments is no part of com.acme.
        assert_eq!(pairs(&all), ["Program.cs -> Models/", "a/src/main/java/com/acme/a/ -> b/src/main/java/com/acme/b/"]);
        // Reading a's folder alone still finds b's package by its folders.
        let only = f.repo().code_links(&tip, Some(&[b"a".to_vec()]), &token()).unwrap();
        assert_eq!(pairs(&only), ["a/src/main/java/com/acme/a/ -> b/src/main/java/com/acme/b/"]);
    });
}

#[test]
fn committed_javascript_python_and_go_packages() {
    isolated("ROCKET_MAP_TEST", "committed_javascript_python_and_go_packages", || {
        let f = Fixture::new();
        f.write("web/package.json", "{\"name\": \"@acme/web\", \"version\": \"1.0.0\"}\n");
        f.write("web/tsconfig.json", "{\n  // aliases\n  \"compilerOptions\": { \"baseUrl\": \".\", \"paths\": { \"@/*\": [\"src/*\"] } },\n}\n");
        f.write("web/src/app.tsx", "import React from 'react';\nimport { card } from './view/card';\nimport * as view from '@/view';\nexport function App() { view.list(); return card(); }\n");
        f.write("web/src/view/card.ts", "export function card() { return 1; }\nexport class Card { draw() {} size() {} }\n");
        f.write("web/src/view/index.ts", "export * from './card';\nexport function list() {}\n");
        f.write("tools/build.cjs", "const fs = require('node:fs');\nmodule.exports = { build };\nfunction build() {}\n");
        f.write("py/shop/__init__.py", "from .cart import Cart\n");
        f.write("py/shop/cart.py", "from decimal import Decimal\nfrom shop.pay import card\nclass Cart:\n    def add(self): pass\n    def total(self): pass\n");
        f.write("py/shop/pay/__init__.py", "");
        f.write("py/shop/pay/card.py", "def charge(): pass\ndef _secret(): pass\n");
        f.write("svc/go.mod", "module example.com/svc\n\ngo 1.22\n");
        f.write("svc/main.go", "package main\n\nimport (\n\t\"fmt\"\n\t\"example.com/svc/store\"\n)\n\nfunc main() { fmt.Println(store.Open()) }\n");
        f.write("svc/store/db.go", "package store\n\ntype DB struct{}\n\nfunc Open() *DB { return nil }\nfunc (d *DB) Close() {}\n");
        f.write("svc/store/db_test.go", "package store\n\nimport \"testing\"\n");
        f.write("svc/store/gen.go", "//go:build ignore\n\npackage main\n");
        f.commit("languages");
        let head = f.run(&["rev-parse", "HEAD"]);
        let package = |folder: &str, language| PackageRef { folder: folder.as_bytes().to_vec(), language };
        let read = |choice: Option<PackageRef>| {
            f.repo()
                .repository_map(&MapOptions { kind: MapKind::Modules, package: choice, ..Default::default() }, &token())
                .unwrap()
        };

        // Code finds them by their paths, so it can offer Modules without a Cargo package.
        let code = f.repo().repository_map(&MapOptions { kind: MapKind::Code, ..Default::default() }, &token()).unwrap();
        let found: Vec<(String, ModuleLanguage, bool, usize)> = code
            .module_packages
            .iter()
            .map(|p| (String::from_utf8_lossy(&p.folder).into_owned(), p.language, p.dialect == Some(Dialect::TypeScript), p.files))
            .collect();
        assert_eq!(
            found,
            [
                ("tools".into(), ModuleLanguage::JavaScript, false, 1),
                ("web".into(), ModuleLanguage::JavaScript, true, 3),
                ("py/shop".into(), ModuleLanguage::Python, false, 4),
                ("svc".into(), ModuleLanguage::Go, false, 3),
            ]
        );

        // Without a choice: the language with the most files (JavaScript's 4 tie with
        // Python's; the first language wins), then its largest package.
        let map = read(None);
        assert_eq!(map.scope.package, Some(package("web", ModuleLanguage::JavaScript)));
        let names: Vec<(String, String)> = map
            .nodes
            .iter()
            .filter(|n| n.kind == MapNodeKind::Crate)
            .map(|n| (String::from_utf8_lossy(&n.label).into_owned(), String::from_utf8_lossy(&n.detail).into_owned()))
            .collect();
        assert_eq!(
            names,
            [
                ("tools".into(), "JavaScript folder".into()),
                ("@acme/web".into(), "TypeScript project".into()),
                ("shop".into(), "Python package".into()),
                ("example.com/svc".into(), "Go module".into()),
            ]
        );
        let details = map.modules.as_ref().unwrap();
        assert_eq!(details.language, ModuleLanguage::JavaScript);
        assert_eq!(details.root, None);
        let module = |map: &RepositoryMap, key: &str| {
            map.nodes
                .iter()
                .position(|n| n.key == format!("module:{key}").as_bytes())
                .unwrap_or_else(|| panic!("no module {key}"))
        };
        let uses = |map: &RepositoryMap, from: &str, to: &str| {
            map.links
                .iter()
                .find(|e| e.kind == MapLinkKind::DependsOn && e.from == module(map, from) && e.to == module(map, to))
                .map(|e| e.evidence.iter().map(|n| String::from_utf8_lossy(n).into_owned()).collect::<Vec<_>>())
        };
        assert_eq!(uses(&map, "app.tsx", "view/card.ts"), Some(vec!["card".into()]));
        assert_eq!(uses(&map, "app.tsx", "view/index.ts"), Some(vec!["*".into(), "list".into()]));
        assert_eq!(uses(&map, "view/index.ts", "view/card.ts"), Some(vec!["*".into()]));
        assert_eq!(map.nodes[module(&map, "view")].detail, b"Folder");
        assert_eq!(details.outside[&module(&map, "app.tsx")], [b"react".to_vec()]);
        let outline: Vec<String> = details.outline[&module(&map, "view/card.ts")]
            .iter()
            .map(|o| format!("{:?} {} {} {}", o.kind, String::from_utf8_lossy(&o.name), o.exported, o.functions))
            .collect();
        assert_eq!(outline, ["Function card true 0", "Class Card true 2"]);

        // A Python package: its __init__.py is the root; dotted imports connect modules.
        let map = read(Some(package("py/shop", ModuleLanguage::Python)));
        let details = map.modules.as_ref().unwrap();
        assert_eq!(details.root, Some(module(&map, "")));
        assert_eq!(uses(&map, "", "cart"), Some(vec!["Cart".into()]));
        assert_eq!(uses(&map, "cart", "pay.card"), Some(vec!["card".into()]));
        assert_eq!(map.nodes[module(&map, "pay")].detail, b"Python package");
        assert_eq!(details.outside[&module(&map, "cart")], [b"decimal".to_vec()]);
        let outline: Vec<(String, bool)> = details.outline[&module(&map, "pay.card")].iter().map(|o| (String::from_utf8_lossy(&o.name).into_owned(), o.exported)).collect();
        assert_eq!(outline, [("charge".into(), true), ("_secret".into(), false)]);

        // A Go module: packages stand alone; a //go:build ignore file is listed apart.
        let map = read(Some(package("svc", ModuleLanguage::Go)));
        let details = map.modules.as_ref().unwrap();
        assert_eq!((details.language, details.root), (ModuleLanguage::Go, None));
        assert_eq!(uses(&map, "", "store"), Some(vec!["Open".into()]));
        assert_eq!(map.nodes[module(&map, "")].label, b"main");
        assert_eq!(map.nodes[module(&map, "store")].files, 1);
        assert_eq!(details.unlinked, [(b"svc/store/gen.go".to_vec(), UnlinkedReason::BuildIgnore)]);
        assert_eq!(details.outside[&module(&map, "")], [b"fmt".to_vec()]);
        let outline: Vec<String> = details.outline[&module(&map, "store")].iter().map(|o| format!("{:?} {} {}", o.kind, String::from_utf8_lossy(&o.name), o.functions)).collect();
        assert_eq!(outline, ["Type DB 1", "Function Open 0"]);

        // A package of another language in the same folder is told apart, and one the
        // snapshot does not have is refused.
        assert!(matches!(
            f.repo().repository_map(&MapOptions { kind: MapKind::Modules, package: Some(package("svc", ModuleLanguage::Python)), ..Default::default() }, &token()),
            Err(Error::InvalidInput(_))
        ));
        assert_eq!(f.run(&["rev-parse", "HEAD"]), head);
    });
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
