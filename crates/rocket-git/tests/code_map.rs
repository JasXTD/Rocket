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
fn rust_module_and_import_sites_follow_the_selected_commit_in_both_maps() {
    isolated("ROCKET_CODE_MAP_TEST", "rust_module_and_import_sites_follow_the_selected_commit_in_both_maps", || {
        let f = Fixture::new();
        let model = "src/資料/型.rs";
        let root = [
            "// mod fake; use crate::model::A;",
            "mod client;",
            "#[path = \"資料/型.rs\"]",
            "pub mod model;",
            "pub mod facade {",
            "    pub(crate) use super::model::{",
            "        A as Renamed,",
            "        B,",
            "    };",
            "}",
            "#[path = \"資料/型.rs\"] mod second_name;",
            "mod ambiguous;",
        ].join("\r\n");
        let client = [
            "use crate::facade::{",
            "    Renamed as Local,",
            "    B,",
            "};",
            "pub use crate::model::*;",
            "use crate::model::NotDeclared;",
            "const EXAMPLE: &str = stringify! { use crate::model::A; };",
            "fn run() { use crate::model::A; }",
            "use crate::model::{self as types};",
        ].join("\n");
        f.write("Cargo.toml", "[package]\nname='rust_sites'\nversion='0.1.0'\n");
        f.write("src/lib.rs", &root);
        f.write("src/client.rs", &client);
        f.write(model, "pub struct A;\npub struct B;\n");
        f.write("src/ambiguous.rs", "pub struct Which;\n");
        // Both conventional paths exist, even though the second exceeds the source-read limit.
        f.write("src/ambiguous/mod.rs", vec![b' '; 1024 * 1024 + 1]);
        f.commit("Original Rust paths");
        let original = head(&f);
        f.write("src/lib.rs", format!("// moved\n{root}"));
        f.write("src/client.rs", format!("// new\n// lines\n{client}"));
        f.commit("Move declarations and imports");
        let newer = head(&f);
        f.write("src/lib.rs", "// staged replacement\n");
        f.run(&["add", "--", "src/lib.rs"]);
        f.write("src/lib.rs", "// different working file\n");
        let repo = f.repo();
        for (tip, root_move, client_move) in [(original, 0, 0), (newer, 1, 2)] {
            let folders = repo.code_links(&tip, None, &token()).unwrap();
            let map = repo.repository_map(&MapOptions {
                revision: tip.as_str().into(), kind: MapKind::Modules, ..Default::default()
            }, &token()).unwrap();
            assert_eq!(map.tip.as_ref(), Some(&tip));
            let module_refs = &map.modules.as_ref().unwrap().references;
            assert_eq!(&folders.references, module_refs);
            assert!(module_refs.for_file(b"src/ambiguous.rs").is_none());
            let sites = module_refs.for_file(model.as_bytes()).expect("references to the Unicode module file");
            let actual: Vec<_> = sites.iter().map(|r| (r.source.as_ref(), r.line, r.kind)).collect();
            assert_eq!(actual, [
                (b"src/client.rs".as_slice(), 2 + client_move, "use"),
                (b"src/client.rs", 3 + client_move, "use"),
                (b"src/client.rs", 5 + client_move, "pub use"),
                (b"src/client.rs", 8 + client_move, "use"),
                (b"src/client.rs", 9 + client_move, "use"),
                (b"src/lib.rs", 4 + root_move, "mod"),
                (b"src/lib.rs", 7 + root_move, "pub(crate) use"),
                (b"src/lib.rs", 8 + root_move, "pub(crate) use"),
                (b"src/lib.rs", 11 + root_move, "mod"),
            ]);
            for site in sites {
                assert!(module_refs.for_file(&site.source).unwrap().contains(site));
            }
        }
    });
}

#[test]
fn rust_embedded_files_connect_both_maps_at_the_selected_commit() {
    isolated("ROCKET_CODE_MAP_TEST", "rust_embedded_files_connect_both_maps_at_the_selected_commit", || {
        let f = Fixture::new();
        f.write("Cargo.toml", "[package]\nname='embed_demo'\nversion='0.1.0'\n");
        f.write("src/lib.rs", r##"mod update;
            pub const GUIDE: &str = include_str!("../docs/入門 #1.md");
            pub const RAW: &[u8] = std::include_bytes![r#"../docs/入門 #1.md"#,];
            pub const PICTURE: &[u8] = include_bytes!("../img/planet.bin");
            pub const GENERATED: &str = include_str!(concat!("../", "decoy"));
            const EXAMPLE: &str = stringify!(include_str!("../decoy"));
            // include_bytes!("../decoy");
        "##);
        f.write("src/update.rs", r#"pub const GUIDE: &str = include_str!("../docs/入門 #1.md");"#);
        f.write("docs/入門 #1.md", "guide");
        f.write("img/planet.bin", vec![0xff; 1024 * 1024 + 1]);
        f.write("decoy", "fixture");
        f.commit("Committed assets");
        let tip = head(&f);
        f.write("src/update.rs", r#"include_str!("../untracked")"#);
        f.write("untracked", "working edit");
        let repo = f.repo();
        let all = repo.code_links(&tip, None, &token()).unwrap();
        let expected = ["src/lib.rs -> docs/入門 #1.md", "src/lib.rs -> img/planet.bin", "src/update.rs -> docs/入門 #1.md"];
        assert_eq!(pairs(&all), expected);
        assert!(!all.limited);
        let map = repo.repository_map(&MapOptions { kind: MapKind::Modules, ..Default::default() }, &token()).unwrap();
        let refs = &map.modules.as_ref().unwrap().references;
        assert_eq!(refs, &all.references, "both views must open the same Rust source lines");
        let guide = refs.for_file("docs/入門 #1.md".as_bytes()).unwrap();
        let sites: Vec<_> = guide.iter().map(|r| (r.source.as_ref(), r.line, r.kind)).collect();
        assert_eq!(sites, [(b"src/lib.rs".as_slice(), 2, "include_str!"), (b"src/lib.rs", 3, "include_bytes!"),
            (b"src/update.rs", 1, "include_str!")]);
        assert!(refs.for_file(b"decoy").is_none());
        let mut pairs = Vec::new();
        for edge in map.links.iter().filter(|l| l.kind == MapLinkKind::DependsOn) {
            let path = |i: usize| String::from_utf8_lossy(map.nodes[i].path.as_deref().unwrap());
            let pair = format!("{} -> {}", path(edge.from), path(edge.to));
            if pair == expected[0] {
                assert_eq!(edge.evidence, [b"include_bytes!".to_vec(), b"include_str!".to_vec()]);
            }
            pairs.push(pair);
        }
        pairs.sort();
        assert_eq!(pairs, expected);
        assert!(!map.limited);
    });
}

#[test]
fn web_assets_connect_shared_styles_pages_and_binary_files_from_the_selected_commit() {
    isolated("ROCKET_CODE_MAP_TEST", "web_assets_connect_shared_styles_pages_and_binary_files_from_the_selected_commit", || {
        let f = Fixture::new();
        f.write("pages/index.html", r##"<LINK HREF='../styles/site.css?v=3#main' ReL='alternate stylesheet'>
            <script src="../js/site.js"></script><img src="pictures/%E7%8C%AB.png#two">
            <picture><source srcset="data:image/png;base64,AA 1x, pictures/猫.png 2x, pictures/large.png 3x"></picture>
            <!-- <img src="decoy.png"> --><script>const example = '<img src="decoy.png">'</script>
            <img src="//outside.invalid/decoy.png"><img src="../missing.png">"##);
        f.write("second/about.htm", r#"<link href="site.css" rel="stylesheet"><base href="../styles/">"#);
        f.write("styles/site.css", r##"@import url('theme/a.css?v=2');
            .icon { background: url(../pages/pictures/%E7%8C%AB.png#icon) }
            /* url(decoy.png) */ p::before { content: "url(decoy.png)" }"##);
        f.write("styles/theme/a.css", r#"@font-face{src:URL("../../fonts/font.woff2?#v")}"#);
        for path in ["pages/pictures/猫.png","pages/pictures/large.png","fonts/font.woff2","pages/decoy.png","styles/decoy.png"] {
            f.write(path, b"\0\xffbinary".as_slice());
        }
        f.write("js/site.js", "// helper\n");
        f.write("pages/packed.HTM", b"\0<img src='decoy.png'>".as_slice());
        f.commit("Shared web assets");
        let tip = head(&f);
        f.write("styles/site.css", "body {background:url(local-only.png)}");
        f.write("styles/local-only.png", "local");
        let repo = f.repo();
        let expected = [
            "pages/index.html -> js/site.js", "pages/index.html -> pages/pictures/large.png",
            "pages/index.html -> pages/pictures/猫.png", "pages/index.html -> styles/site.css",
            "second/about.htm -> styles/site.css", "styles/site.css -> pages/pictures/猫.png",
            "styles/site.css -> styles/theme/a.css", "styles/theme/a.css -> fonts/font.woff2",
        ];
        let all = repo.code_links(&tip, None, &token()).unwrap();
        assert_eq!(pairs(&all), expected);
        let selected = repo.code_links(&tip, Some(&[Vec::new()]), &token()).unwrap();
        assert_eq!(pairs(&selected), expected);
        let catalog = repo.repository_map(&MapOptions { kind: MapKind::Code, ..Default::default() }, &token()).unwrap();
        let group = catalog.module_packages.iter().find(|p| p.name.as_slice() == b"HTML / CSS").unwrap();
        assert_eq!(group.files, 5);
        let map = repo.repository_map(&MapOptions { kind: MapKind::Modules, package: Some(group.reference()), ..Default::default() }, &token()).unwrap();
        let mut edges = map.links.iter().filter(|e| e.kind == MapLinkKind::DependsOn).map(|e| {
            let path = |i: usize| String::from_utf8_lossy(map.nodes[i].path.as_deref().unwrap());
            format!("{} -> {}", path(e.from), path(e.to))
        }).collect::<Vec<_>>();
        edges.sort();
        assert_eq!(edges, expected);
        let refs = &map.modules.as_ref().unwrap().references;
        assert_eq!(refs, &all.references);
        let sites: Vec<_> = refs.for_file("pages/pictures/猫.png".as_bytes()).unwrap().iter()
            .map(|r| (r.source.as_ref(), r.line, r.kind)).collect();
        assert_eq!(sites, [(b"pages/index.html".as_slice(), 2, "image"), (b"pages/index.html", 3, "srcset"),
            (b"styles/site.css", 2, "url()")]);
        let mut merged = all;
        merged.merge(selected);
        assert_eq!(&merged.references, refs, "reading a folder again must not duplicate its reference sites");
        assert!(map.modules.unwrap().unlinked.contains(&(b"pages/packed.HTM".to_vec(), UnlinkedReason::Binary)));
    });
}

#[test]
fn web_assets_share_a_budget_across_pages_and_stylesheets() {
    isolated("ROCKET_CODE_MAP_TEST", "web_assets_share_a_budget_across_pages_and_stylesheets", || {
        let f = Fixture::new();
        f.write("a.html", format!("<img src=keep.png>{}", "<img src=missing>".repeat(6500)));
        f.write("b.css", format!("a{{src:url(keep.png)}}{}a{{src:url(late.png)}}", "a{src:url(missing)}".repeat(6500)));
        f.write("keep.png", "keep");
        f.write("late.png", "later");
        f.commit("Bound the shared web asset read");
        let repo = f.repo();
        let all = repo.code_links(&head(&f), None, &token()).unwrap();
        assert!(all.sources_limited && all.limited);
        assert_eq!(pairs(&all), ["a.html -> keep.png", "b.css -> keep.png"]);
        let group = all.packages.into_iter().find(|p| p.language == ModuleLanguage::HtmlCss).unwrap();
        let map = repo.repository_map(&MapOptions { kind: MapKind::Modules, package: Some(group), ..Default::default() }, &token()).unwrap();
        assert!(map.limited);
        assert_eq!(map.links.iter().filter(|l| l.kind == MapLinkKind::DependsOn).count(), 2);
    });
}

#[test]
fn workflow_links_use_the_committed_step_directory_and_local_targets() {
    isolated("ROCKET_CODE_MAP_TEST", "workflow_links_use_the_committed_step_directory_and_local_targets", || {
        let f = Fixture::new();
        let workflow = ".github/workflows/build.yml";
        f.write(workflow, r#"
name: Build
jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - run: bash check.sh
      - run: |
          printf '%s\n' 'bash decoy.sh'
          node --test "scripts/検査.cjs"
        working-directory: .
      - uses: ./.github/actions/check
      - uses: actions/checkout@v4
        with:
          run: bash decoy.sh
      - run: bash decoy.sh
        working-directory: ${{ matrix.folder }}
      - run: bash ../outside.sh
        working-directory: .
    defaults:
      run:
        working-directory: jobs
  common:
    uses: ./.github/workflows/shared.yaml
  inherited:
    runs-on: ubuntu-latest
    steps:
      - run: ./root-helper.sh
defaults:
  run:
    working-directory: ci
"#);
        f.write(".github/workflows/shared.yaml", "on: workflow_call\njobs: {}\n");
        f.write(".github/actions/check/action.yaml", "name: Local check\n");
        for file in ["jobs/check.sh","ci/root-helper.sh","scripts/検査.cjs","decoy.sh","jobs/decoy.sh","outside.sh"] {
            f.write(file, "# fixture\n");
        }
        f.write("notes.yml", "jobs:\n  fake:\n    steps:\n      - run: bash decoy.sh\n");
        f.commit("Workflow references");
        let tip = head(&f);
        f.write(workflow, "jobs: {}\n");
        f.write("new.sh", "# untracked\n");
        let repo = f.repo();
        let catalog = repo.repository_map(&MapOptions { kind: MapKind::Code, ..Default::default() }, &token()).unwrap();
        assert!(catalog.module_packages.iter().any(|p| p.language == ModuleLanguage::GitHubActions && p.files == 2));
        let all = repo.code_links(&tip, None, &token()).unwrap();
        let expected: Vec<_> = [".github/actions/check/action.yaml",".github/workflows/shared.yaml","ci/root-helper.sh","jobs/check.sh","scripts/検査.cjs"]
            .iter().map(|p| format!("{workflow} -> {p}")).collect();
        assert_eq!(pairs(&all), expected);
        let subset = repo.code_links(&tip, Some(&[b".github/workflows".to_vec()]), &token()).unwrap();
        assert_eq!(pairs(&subset), expected);
        let package = subset.packages.into_iter().find(|p| p.folder.as_slice() == b".github/workflows").unwrap();
        let map = repo.repository_map(&MapOptions { kind: MapKind::Modules, package: Some(package), ..Default::default() }, &token()).unwrap();
        let mut links = map.links.iter().filter(|l| l.kind == MapLinkKind::DependsOn).map(|edge| {
            let path = |i: usize| String::from_utf8_lossy(map.nodes[i].path.as_deref().unwrap());
            format!("{} -> {}", path(edge.from), path(edge.to))
        }).collect::<Vec<_>>();
        links.sort();
        assert_eq!(links, expected);
    });
}

#[test]
fn lua_links_committed_modules_and_rocket_entries_in_both_maps() {
    isolated("ROCKET_CODE_MAP_TEST", "lua_links_committed_modules_and_rocket_entries_in_both_maps", || {
        let f = Fixture::new();
        f.write("run.lua", r#"local ui = require("ui.面板")
            local lib = require 'lib'
            require("decoy" .. suffix)
            -- require("decoy")
            local text = [=[require("decoy")]=]
            object:require("decoy")
            require("ambiguous")
            require("plugin.main")
            require("other.main")"#);
        for path in ["ui/面板.lua", "lib/init.lua", "decoy.lua", "ambiguous.lua", "ambiguous/init.lua"] {
            f.write(path, "return {}\n");
        }
        f.write("plugin/integration.json", include_str!("../../rocket-integrations/examples/jira/integration.json"));
        f.write("plugin/main.lua", r#"require("decoy")"#);
        f.write("other/integration.json", r#"{"api":2}"#);
        f.write("other/main.lua", "return require 'lib'");
        f.commit("Lua module and integration fixtures");
        let tip = head(&f);
        f.write("run.lua", "require('untracked')\n");
        f.write("untracked.lua", "return 2");
        let repo = f.repo();
        let catalog = repo.repository_map(&MapOptions { kind: MapKind::Code, ..Default::default() }, &token()).unwrap();
        let groups: Vec<_> = catalog.module_packages.iter().filter(|p| p.language == ModuleLanguage::Lua)
            .map(|p| (p.folder.as_slice(), p.files)).collect();
        assert_eq!(groups, [(b"".as_slice(), 7), (b"plugin".as_slice(), 1)]);
        let all = repo.code_links(&tip, None, &token()).unwrap();
        let expected = ["other/main.lua -> lib/init.lua", "plugin/integration.json -> plugin/main.lua",
            "run.lua -> lib/init.lua", "run.lua -> other/main.lua", "run.lua -> ui/面板.lua"];
        assert_eq!(pairs(&all), expected);
        let mut module_links = Vec::new();
        for package in all.packages {
            let map = repo.repository_map(&MapOptions { kind: MapKind::Modules, package: Some(package), ..Default::default() }, &token()).unwrap();
            assert_eq!(map.module_packages.iter().find(|p| p.folder.as_slice() == b"plugin").unwrap().files, 1);
            for edge in map.links.iter().filter(|e| e.kind == MapLinkKind::DependsOn) {
                let path = |i: usize| String::from_utf8_lossy(map.nodes[i].path.as_deref().unwrap());
                module_links.push(format!("{} -> {}", path(edge.from), path(edge.to)));
            }
        }
        module_links.sort();
        assert_eq!(module_links, expected);
        let stopped = token();
        stopped.cancel();
        assert!(matches!(repo.code_links(&tip, None, &stopped), Err(Error::Cancelled)));
    });
}

#[test]
fn lua_respects_binary_targets_entry_limits_and_partial_reads() {
    isolated("ROCKET_CODE_MAP_TEST", "lua_respects_binary_targets_entry_limits_and_partial_reads", || {
        let f = Fixture::new();
        let manifest = include_str!("../../rocket-integrations/examples/repository/integration.json");
        f.write("plugin/integration.json", manifest);
        f.write("plugin/main.lua", " ".repeat(64 * 1024 + 1));
        f.write("binary.lua", b"\x1bLua\0require('real')".as_slice());
        f.write("indirect.lua", "require 'binary'");
        f.write("real.lua", "return 1");
        f.write("many.lua", "require('real')\n".repeat(12_001));
        f.commit("Bounded Lua reads");
        let tip = head(&f);
        let repo = f.repo();
        let all = repo.code_links(&tip, None, &token()).unwrap();
        assert!(all.limited && all.sources_limited);
        assert_eq!(pairs(&all), ["many.lua -> real.lua"]);
        let package = all.packages.into_iter().find(|p| p.folder.is_empty() && p.language == ModuleLanguage::Lua).unwrap();
        let map = repo.repository_map(&MapOptions { kind: MapKind::Modules, package: Some(package), ..Default::default() }, &token()).unwrap();
        assert!(map.limited);
        assert!(map.modules.unwrap().unlinked.contains(&(b"binary.lua".to_vec(), UnlinkedReason::Binary)));
    });
}

#[test]
fn native_bridges_follow_the_selected_build_and_committed_symbols() {
    isolated("ROCKET_CODE_MAP_TEST", "native_bridges_follow_the_selected_build_and_committed_symbols", || {
        let f = Fixture::new();
        f.write("crates/spell/Cargo.toml", "[package]\nname='spell'\n");
        f.write("crates/spell/build.rs", include_str!("../../rocket-spell/build.rs"));
        f.write("tools/build-objc.rs", include_str!("../../../tools/build-objc.rs"));
        f.write("crates/spell/src/lib.rs", "mod macos;");
        f.write("crates/spell/src/macos.rs", include_str!("../../rocket-spell/src/macos.rs"));
        f.write("crates/spell/src/macos.m", include_str!("../../rocket-spell/src/macos.m"));
        f.write("bindings/Cargo.toml", "[package]\nname='bindings'\n");
        f.write("bindings/build.rs", r#"
            fn unused() { cc::Build::new().file("src/native/decoy.c").compile("decoy"); }
            fn main() {
                stringify!(cc::Build::new().file("src/native/decoy.c").compile("decoy"));
                cc::Build::new().file("src/native/介面.mm").file("src/native/other.c").compile("bridge");
            }"#);
        f.write("bindings/src/lib.rs", "mod ffi; pub fn read() { unsafe { ffi::local() } }");
        f.write("bindings/src/ffi.rs", r#"unsafe extern "C" {
            #[link_name = "shared_name"] pub fn local();
            fn internal(); fn prototype(); fn mangled(); fn duplicate(); fn only_elsewhere();
            fn inherited_static(); fn cpp_block(); fn inline_only();
        }
        #[link(name = "an_external_library")] unsafe extern "C" { fn wrong_library(); }
        "#);
        f.write("bindings/src/native/介面.mm", r#"#import "標頭.h"
#import <AppKit/AppKit.h>
extern "C" { void shared_name() {} void duplicate() {} static void internal() {} }
extern "C" { extern "C++" { void cpp_block() {} } }
extern "C" void prototype();
void mangled() {}
void wrong_library() {}
"#);
        f.write("bindings/src/native/標頭.h", "#include <stdint.h>\n");
        f.write("bindings/src/native/other.c", "void duplicate() {} void wrong_library() {} static void inherited_static(void); void inherited_static(void) {} inline void inline_only() {}");
        f.write("bindings/src/native/decoy.c", "void shared_name() {}");
        f.write("crates/spell/src/decoy.m", "void shared_name() {} void only_elsewhere() {}");
        f.commit("native targets and framework imports");
        let tip = head(&f);
        f.write("bindings/src/native/介面.mm", "// unsaved edits must survive\n");
        let repo = f.repo();
        let links = repo.code_links(&tip, None, &token()).unwrap();
        let symbols = |from: &str, to: &str| links.links.iter().find(|l|
            links.ends[l.from].path == from.as_bytes() && links.ends[l.to].path == to.as_bytes()).map(|l| &l.names);
        assert!(symbols("crates/spell/src/macos.rs", "crates/spell/src/macos.m").is_some_and(|n| n.iter().any(|s| s == "rocket_spell_check")), "missing Rocket spelling bridge");
        for from in ["bindings/src/lib.rs", "bindings/src/ffi.rs"] {
            assert_eq!(symbols(from, "bindings/src/native/介面.mm").unwrap(), &["shared_name"]);
            assert!(symbols(from, "bindings/src/native/other.c").is_none());
            assert!(symbols(from, "crates/spell/src/decoy.m").is_none());
        }
        assert!(symbols("bindings/src/native/介面.mm", "bindings/src/native/標頭.h").is_some());
        let partial = repo.code_links(&tip, Some(&[b"bindings".to_vec()]), &token()).unwrap();
        assert!(pairs(&partial).contains(&"bindings/src/lib.rs -> bindings/src/native/介面.mm".to_owned()));
        let stopped = token();
        stopped.cancel();
        assert!(matches!(repo.code_links(&tip, None, &stopped), Err(Error::Cancelled)));
    });
}

#[test]
fn native_builds_respect_manifest_selection_and_incomplete_sources() {
    isolated("ROCKET_CODE_MAP_TEST", "native_builds_respect_manifest_selection_and_incomplete_sources", || {
        let f = Fixture::new();
        let build = r#"fn main() { cc::Build::new().file("native.c").compile("example"); }"#;
        let rust = r#"unsafe extern "C" { fn example(); }"#;
        for (folder, setting) in [("custom", "build='tools/custom.rs'"), ("disabled", "build=false"), ("partial", "")] {
            f.write(&format!("{folder}/Cargo.toml"), format!("[package]\nname='{folder}'\n{setting}\n"));
            f.write(&format!("{folder}/src/lib.rs"), rust);
            f.write(&format!("{folder}/native.c"), "int example(void) { return 7; }");
            f.write(&format!("{folder}/build.rs"), build);
        }
        f.write("custom/build.rs", r#"fn main() { cc::Build::new().file("decoy.c").compile("wrong"); }"#);
        f.write("custom/decoy.c", "int example(void) { return 1; }");
        f.write("custom/tools/custom.rs", build);
        f.write("partial/build.rs", r#"fn main() { cc::Build::new().file("native.c").file("large.c").compile("example"); }"#);
        f.write("partial/large.c", " ".repeat(1024 * 1024 + 1));
        f.commit("native build choices");
        let repo = f.repo();
        let links = repo.code_links(&head(&f), None, &token()).unwrap();
        assert!(links.limited && links.sources_limited);
        let native: Vec<_> = pairs(&links).into_iter().filter(|p| p.ends_with(".c")).collect();
        assert_eq!(native, ["custom/src/lib.rs -> custom/native.c"]);
    });
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
