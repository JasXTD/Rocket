//! Canvas section 51: the scan budget's row in Secret protection, and the dependency
//! check's Scan limits from the fields to the scan, the push check and the state file.
use super::*;
use crate::{
    preferences::{Page, reveal_test_control},
    test_common::{Fixture, isolated},
};
use rocket_git::CancellationToken;
use serde_json::json;
use std::{fs, path::PathBuf, sync::Arc};

const MIB: u64 = 1024 * 1024;
const WIDE: Vec2 = vec2(1280.0, 860.0);

fn scratch() -> scan::Scratch {
    scan::Scratch::new(&std::env::temp_dir().join("rocket-dependency-tests")).unwrap()
}
fn limits(files: usize, file_mib: u64, total_mib: u64) -> Limits {
    Limits { files, file_mib, total_mib }
}
fn report() -> scan::Report {
    scan::Report { packages: 1, files: 1, findings: vec![], notes: vec![], checked: scan::now(), oldest_database: scan::now() }
}

#[test]
fn a_working_copy_scan_stops_at_each_limit_and_says_what_it_found() {
    let root = scratch();
    for folder in ["a", "b", "c"] {
        fs::create_dir(root.path.join(folder)).unwrap();
        fs::write(root.path.join(folder).join("Cargo.lock"), "version = 3\n").unwrap();
    }
    let cancel = CancellationToken::new();
    let error = scan::inventory(&root.path, &limits(2, 8, 64), &cancel).unwrap_err();
    assert_eq!(error_message(&error), "3 dependency files found. Scan limits allow 2 files.");
    assert!(scan::stopped_at_limit(&error));
    assert_eq!(scan::inventory(&root.path, &limits(3, 8, 64), &cancel).unwrap().files.len(), 3);
    // Each file, by its path in the repository and never shown as the limit itself.
    fs::write(root.path.join("b/Cargo.lock"), vec![b'#'; (MIB + 1) as usize]).unwrap();
    let error = scan::inventory(&root.path, &limits(3, 1, 64), &cancel).unwrap_err();
    assert_eq!(error_message(&error), "b/Cargo.lock is 1.1 MiB. Scan limits allow 1 MiB a file.");
    assert!(scan::stopped_at_limit(&error));
    // All files together.
    for folder in ["a", "b"] {
        fs::write(root.path.join(folder).join("Cargo.lock"), vec![b'#'; (MIB * 6 / 10) as usize]).unwrap();
    }
    let error = scan::inventory(&root.path, &limits(3, 1, 1), &cancel).unwrap_err();
    assert_eq!(error_message(&error), "Dependency files add up to 1.2 MiB. Scan limits allow 1 MiB in all.");
    assert!(scan::stopped_at_limit(&error));
    // Above the old fixed 8 MiB, once Each file allows it.
    fs::write(root.path.join("a/Cargo.lock"), vec![b'#'; (9 * MIB) as usize]).unwrap();
    assert!(scan::inventory(&root.path, &Limits::default(), &cancel).is_err());
    let inventory = scan::inventory(&root.path, &limits(3, 16, 64), &cancel).unwrap();
    assert_eq!(inventory.files.iter().map(|f| f.bytes.len() as u64).max(), Some(9 * MIB));
    // Other stops are not the user's limits.
    assert!(!scan::stopped_at_limit(&Error::InvalidInput("Dependency discovery exceeded 50,000 entries. Scan a smaller repository.".into())));
    assert!(!scan::stopped_at_limit(&Error::Cancelled));
}

#[test]
fn more_than_256_files_run_in_groups_that_each_fit_a_command_line() {
    let base = PathBuf::from(format!("C:/Users/someone/AppData/Local/Rocket/state/dependency-security/work/scan-{}", "7".repeat(24)));
    let copies: Vec<PathBuf> = (0..1000).map(|i| base.join(i.to_string()).join("package-lock.json")).collect();
    let small = vec![1024u64; copies.len()];
    let runs = scan::runs(&copies, &small);
    assert!(runs.len() >= 4, "{runs:?}");
    let mut next = 0;
    for run in &runs {
        assert_eq!(run.start, next, "runs cover the files in order");
        assert!(!run.is_empty() && run.len() <= 256);
        let characters: usize = copies[run.clone()].iter().map(|c| c.as_os_str().len() + 16).sum();
        assert!(characters <= 24_000, "{characters}");
        next = run.end;
    }
    assert_eq!(next, copies.len());
    assert_eq!(scan::runs(&copies[..3], &small[..3]), vec![0..3]);
    assert!(scan::runs(&[], &[]).is_empty());
    // Short paths: 256 a run, the most rocket-git lets OSV take.
    let short: Vec<PathBuf> = (0..600).map(|i| PathBuf::from(format!("C:/w/{i}/go.mod"))).collect();
    assert_eq!(scan::runs(&short, &vec![1024; 600]), vec![0..256, 256..512, 512..600]);
    // And at most 64 MiB a run, so no run is bigger than a whole scan was before; the
    // old default (64 MiB in all) is still one run.
    assert_eq!(scan::runs(&short[..8], &[8 * MIB; 8]), vec![0..8]);
    assert_eq!(scan::runs(&short[..10], &[20 * MIB; 10]), vec![0..3, 3..6, 6..9, 9..10]);
    assert_eq!(scan::runs(&short[..2], &[64 * MIB, 1]), vec![0..1, 1..2]);
}

#[test]
fn packages_are_capped_per_osv_run_not_across_runs() {
    let input = |path: &str| scan::Input { path: path.into(), ecosystem: "npm".into(), bytes: vec![] };
    let inventory = scan::Inventory { files: vec![input("a/package-lock.json"), input("b/package-lock.json")], notes: vec![] };
    let copies = vec![PathBuf::from("C:/snapshot/0/package-lock.json"), PathBuf::from("C:/snapshot/1/package-lock.json")];
    let result = |copy: &str, count: usize| {
        let packages: Vec<_> = (0..count).map(|i| json!({"package":{"name":format!("p{i}"),"version":"1.0.0","ecosystem":"npm"}})).collect();
        serde_json::to_vec(&json!({"results":[{"source":{"path":copy,"type":"lockfile"},"packages":packages}]})).unwrap()
    };
    let mut merge = scan::Merge::new(&inventory);
    merge.add(&result("C:/snapshot/0/package-lock.json", 30_000), &inventory, &copies, 0..1).unwrap();
    merge.add(&result("C:/snapshot/1/package-lock.json", 30_000), &inventory, &copies, 1..2).unwrap();
    assert_eq!(merge.finish(&inventory).unwrap().packages, 60_000);
    let mut merge = scan::Merge::new(&inventory);
    assert!(merge.add(&result("C:/snapshot/0/package-lock.json", 50_001), &inventory, &copies, 0..1).is_err());
}

#[test]
fn results_of_several_runs_merge_and_each_run_names_only_its_own_files() {
    let input = |path: &str| scan::Input { path: path.into(), ecosystem: "npm".into(), bytes: vec![] };
    let inventory = scan::Inventory { files: vec![input("a/package-lock.json"), input("b/package-lock.json")], notes: vec![] };
    let copies = vec![PathBuf::from("C:/snapshot/0/package-lock.json"), PathBuf::from("C:/snapshot/1/package-lock.json")];
    let result = |copy: &str, name: &str, advisory: bool| {
        let (vulnerabilities, groups) = if advisory {
            (json!([{"id":"GHSA-2222-3333-4444","summary":"Fixture advisory"}]), json!([{"ids":["GHSA-2222-3333-4444"],"max_severity":"7.5"}]))
        } else {
            (json!(null), json!(null))
        };
        serde_json::to_vec(&json!({"results":[{"source":{"path":copy,"type":"lockfile"},"packages":[{
            "package":{"name":name,"version":"1.0.0","ecosystem":"npm"},"vulnerabilities":vulnerabilities,"groups":groups
        }]}]}))
        .unwrap()
    };
    let mut merge = scan::Merge::new(&inventory);
    assert_eq!(merge.add(&result("C:/snapshot/0/package-lock.json", "first", false), &inventory, &copies, 0..1).unwrap(), 0);
    assert_eq!(merge.add(&result("C:/snapshot/1/package-lock.json", "second", true), &inventory, &copies, 1..2).unwrap(), 1);
    let report = merge.finish(&inventory).unwrap();
    assert_eq!((report.packages, report.findings.len()), (2, 1));
    assert_eq!(report.findings[0].file, "b/package-lock.json");
    assert!(report.notes.is_empty(), "{:?}", report.notes);
    // A run's result naming another run's file is not taken.
    let mut merge = scan::Merge::new(&inventory);
    assert!(merge.add(&result("C:/snapshot/1/package-lock.json", "second", false), &inventory, &copies, 0..1).is_err());
    // A run that gave nothing leaves its files uncovered, and the report says so.
    let mut merge = scan::Merge::new(&inventory);
    merge.add(&result("C:/snapshot/0/package-lock.json", "first", false), &inventory, &copies, 0..1).unwrap();
    assert!(merge.finish(&inventory).unwrap().notes.iter().any(|n| n.contains("produced no inventory")));
}

#[test]
#[ignore = "requires the reviewed OSV binary (ROCKET_OSV_TEST_EXE) and a synthetic crates.io database (ROCKET_OSV_GROUP_CACHE)"]
fn actual_osv_scans_more_than_256_files_in_groups() {
    let exe = PathBuf::from(std::env::var_os("ROCKET_OSV_TEST_EXE").expect("the reviewed OSV binary"));
    let cache = PathBuf::from(std::env::var_os("ROCKET_OSV_GROUP_CACHE").expect("a cache holding osv-scalibr/crates.io/all.zip"));
    let root = scratch();
    let work = scratch();
    for i in 0..300 {
        let folder = root.path.join(format!("p{i:03}"));
        fs::create_dir(&folder).unwrap();
        fs::write(folder.join("Cargo.lock"), concat!(
            "version = 3\n\n[[package]]\nname = \"rocket-group-fixture\"\nversion = \"1.0.0\"\n",
            "source = \"registry+https://github.com/rust-lang/crates.io-index\"\n",
        )).unwrap();
    }
    let cancel = CancellationToken::new();
    let error = scan::scan(&root.path, &exe, &cache, &work.path, None, true, &Limits::default(), &cancel).err().expect("over the default file limit");
    assert_eq!(error_message(&error), "300 dependency files found. Scan limits allow 256 files.");
    let started = Instant::now();
    let result = scan::scan(&root.path, &exe, &cache, &work.path, None, true, &limits(300, 8, 64), &cancel).unwrap();
    eprintln!("300 files in {:?}", started.elapsed());
    assert_eq!((result.report.files, result.report.packages, result.report.findings.len()), (300, 300, 300));
    assert!(result.report.notes.iter().all(|n| !n.contains("produced no inventory")), "{:?}", result.report.notes);
    let files: std::collections::BTreeSet<_> = result.report.findings.iter().map(|f| f.file.replace('\\', "/")).collect();
    assert_eq!(files.len(), 300, "every file's finding is its own");
    assert!(files.contains("p000/Cargo.lock") && files.contains("p299/Cargo.lock"), "{:?}", files.iter().next());
}

#[test]
fn a_kept_result_holds_a_fingerprint_and_is_reused_only_for_the_same_files() {
    let root = scratch();
    let cache = scratch();
    fs::write(root.path.join("Cargo.lock"), "version = 3\n").unwrap();
    let inert = cache.path.join("inert.exe");
    fs::write(&inert, "owned inert fixture").unwrap();
    fs::create_dir_all(cache.path.join("osv-scalibr/crates.io")).unwrap();
    fs::write(cache.path.join("osv-scalibr/crates.io/all.zip"), "owned inert cache").unwrap();
    let cancel = CancellationToken::new();
    let inventory = scan::inventory(&root.path, &Limits::default(), &cancel).unwrap();
    let previous = scan::Cached {
        database: scan::databases(&cache.path, &inventory.ecosystems(), &cancel).unwrap(),
        inventory: inventory.fingerprint().unwrap(),
        report: Arc::new(report()),
    };
    // The same files and data: the kept result, and nothing runs.
    let reused = scan::scan(&root.path, &inert, &cache.path, &cache.path, Some(&previous), false, &Limits::default(), &cancel).unwrap();
    assert!(Arc::ptr_eq(&reused.report, &previous.report));
    // Changed contents need a new scan, which the inert program can't give.
    fs::write(root.path.join("Cargo.lock"), "version = 4\n").unwrap();
    let changed = scan::inventory(&root.path, &Limits::default(), &cancel).unwrap();
    assert_ne!(changed.fingerprint().unwrap(), previous.inventory);
    assert!(scan::scan(&root.path, &inert, &cache.path, &cache.path, Some(&previous), false, &Limits::default(), &cancel).is_err());
}

#[test]
fn a_pushed_tip_is_read_within_the_limits() {
    isolated(
        "ROCKET_SCAN_LIMITS_CASE",
        "dependency_security::limits_tests::a_pushed_tip_is_read_within_the_limits",
        || {
            let f = Fixture::new();
            f.write("a/Cargo.lock", "version = 3\n");
            f.write("b/Cargo.lock", vec![b'#'; (9 * MIB) as usize]);
            f.write("c/go.mod", "module example.com/c\n");
            f.commit("dependency files");
            let id = rocket_git::ObjectId::parse(String::from_utf8(f.run(&["rev-parse", "HEAD"])).unwrap().trim()).unwrap();
            let repo = f.repo();
            let cancel = CancellationToken::new();
            let error = scan::committed_inventory(&repo, &id, &limits(2, 16, 64), &cancel).unwrap_err();
            assert_eq!(error_message(&error), "3 dependency files found. Scan limits allow 2 files.");
            let error = scan::committed_inventory(&repo, &id, &Limits::default(), &cancel).unwrap_err();
            assert_eq!(error_message(&error), "b/Cargo.lock is over 8 MiB. Scan limits allow 8 MiB a file.");
            assert!(scan::stopped_at_limit(&error));
            // 9 MiB is exactly Each file, and the other two files take All files over it.
            let error = scan::committed_inventory(&repo, &id, &limits(3, 9, 9), &cancel).unwrap_err();
            assert_eq!(error_message(&error), "Dependency files add up to over 9 MiB. Scan limits allow 9 MiB in all.");
            assert!(scan::stopped_at_limit(&error));
            // A file over the old fixed 8 MiB is read once Each file allows it.
            let inventory = scan::committed_inventory(&repo, &id, &limits(3, 16, 64), &cancel).unwrap();
            assert_eq!(inventory.files.len(), 3);
            assert_eq!(inventory.files[1].bytes.len() as u64, 9 * MIB);
        },
    );
}

fn frame(ctx: &Context, app: &mut Rocket, size: Vec2, events: Vec<Event>) {
    app.poll(ctx);
    app.frame_widgets.clear();
    let mut out = ctx.run_ui(
        RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)), events, ..Default::default() },
        |ui| app.dialog_ui(ui.ctx()),
    );
    out.textures_delta.clear();
}
fn settle(app: &mut Rocket, ctx: &Context) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while app.backend.is_none() || app.busy() || app.dependencies.running() {
        app.poll(ctx);
        assert!(
            Instant::now() < deadline,
            "Rocket did not settle: backend {} working {} busy {} scanner setup {} dependency job {}",
            app.backend.is_some(), app.working(), app.busy(), app.dependencies.setup.running(), app.dependencies.job.is_some()
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}
fn click(ctx: &Context, app: &mut Rocket, key: &str) {
    let at = reveal_test_control(ctx, app, WIDE, key, frame).rect.center();
    let button = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    frame(ctx, app, WIDE, vec![Event::PointerMoved(at), button(true)]);
    frame(ctx, app, WIDE, vec![button(false)]);
}
fn key(key: Key, modifiers: Modifiers) -> Event {
    Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }
}
/// Replaces a field's text, as a user selecting it all and typing.
fn type_into(ctx: &Context, app: &mut Rocket, field: &str, text: &str) {
    click(ctx, app, field);
    frame(ctx, app, WIDE, vec![key(Key::A, Modifiers::COMMAND)]);
    frame(ctx, app, WIDE, vec![Event::Text(text.into())]);
}
fn enter(ctx: &Context, app: &mut Rocket) {
    frame(ctx, app, WIDE, vec![key(Key::Enter, Modifiers::NONE)]);
    frame(ctx, app, WIDE, vec![]);
}

#[test]
fn the_budget_is_a_row_of_secret_protection_and_enter_applies_it() {
    isolated(
        "ROCKET_SCAN_LIMITS_CASE",
        "dependency_security::limits_tests::the_budget_is_a_row_of_secret_protection_and_enter_applies_it",
        || {
            let f = Fixture::new();
            let ctx = Context::default();
            let mut app = Rocket::with_storage(&ctx, vec![], f.root.join("logs"), f.root.join("state"));
            settle(&mut app, &ctx);
            app.show_preferences(Page::Security);
            let field = reveal_test_control(&ctx, &mut app, WIDE, "pref-secret-scan-mib", frame);
            // Inside the Secret protection card: below its switch, above What gets checked?,
            // and above Guard AI API keys' card.
            let top = |app: &Rocket, key: &str| app.frame_widgets.get(key).map(|c| c.rect.top());
            if let Some(switch) = top(&app, "pref-secret-protection") {
                assert!(switch < field.rect.top());
            }
            let coverage = top(&app, "security-secret-coverage").expect("What gets checked? is in view");
            assert!(field.rect.top() < coverage);
            if let Some(guard) = top(&app, "pref-ai-key-guard") {
                assert!(coverage < guard);
            }
            assert!(!app.frame_widgets.contains_key("pref-dependency-files") || top(&app, "pref-dependency-files").unwrap() > coverage);
            assert!(!app.frame_widgets["pref-secret-scan-apply"].enabled, "nothing typed yet");
            type_into(&ctx, &mut app, "pref-secret-scan-mib", "1000");
            assert_eq!(app.dependencies.scan_budget.text.as_deref(), Some("1000"));
            assert_eq!(app.secret_scan_mib, 256, "typing alone applies nothing");
            frame(&ctx, &mut app, WIDE, vec![]);
            assert!(app.frame_widgets["pref-secret-scan-apply"].enabled);
            enter(&ctx, &mut app);
            assert_eq!(app.secret_scan_mib, 1000);
            assert!(app.dependencies.scan_budget.text.is_none() && app.dependencies.scan_budget.error.is_none());
            // Not a number: refused, and the budget in use stays.
            type_into(&ctx, &mut app, "pref-secret-scan-mib", "1,000");
            assert_eq!(app.dependencies.scan_budget.text.as_deref(), Some("1,000"), "typed {:?}", app.dependencies.scan_budget.error);
            enter(&ctx, &mut app);
            assert_eq!(app.secret_scan_mib, 1000);
            assert_eq!(app.dependencies.scan_budget.error.as_deref(), Some("Enter a positive whole number of MiB."));
            click(&ctx, &mut app, "pref-secret-scan-reset");
            assert_eq!(app.secret_scan_mib, 256);
            assert!(app.dependencies.scan_budget.error.is_none());
        },
    );
}

#[test]
fn applying_limits_drops_a_scan_still_running_with_the_old_ones() {
    isolated(
        "ROCKET_SCAN_LIMITS_CASE",
        "dependency_security::limits_tests::applying_limits_drops_a_scan_still_running_with_the_old_ones",
        || {
            let f = Fixture::new();
            let ctx = Context::default();
            let mut app = Rocket::with_storage(&ctx, vec![f.repository.clone()], f.root.join("logs"), f.root.join("state"));
            app.dependencies.fixture_executable = Some(f.root.join("not-installed.exe"));
            settle(&mut app, &ctx);
            app.dependencies.enabled = false;
            app.dependencies.repositories.clear();
            let owner = app.tab().unwrap().id;
            let path = app.tab().unwrap().path.clone();
            let task = rocket_git::ReadTask::spawn(|cancel| loop {
                cancel.check()?;
                std::thread::sleep(Duration::from_millis(5));
            })
            .unwrap();
            app.dependencies.job = Some(Job { owner, path, update: false, discard: false, task });
            app.dependencies.repositories.entry(owner).or_default().next = Some(Instant::now() + Duration::from_secs(60));
            app.apply_dependency_limits(limits(300, 8, 64));
            assert!(app.dependencies.job.as_ref().is_some_and(|job| job.discard));
            settle(&mut app, &ctx);
            assert!(app.dependencies.job.is_none());
            let state = &app.dependencies.repositories[&owner];
            assert!(state.next.is_none(), "the old scan's end doesn't push the next one 30 s out");
            assert!(state.error.is_none() && !state.limited);
        },
    );
}

#[test]
fn scan_limits_apply_with_enter_refuse_bad_values_and_survive_a_restart() {
    isolated(
        "ROCKET_SCAN_LIMITS_CASE",
        "dependency_security::limits_tests::scan_limits_apply_with_enter_refuse_bad_values_and_survive_a_restart",
        || {
            let f = Fixture::new();
            let ctx = Context::default();
            let mut app = Rocket::with_storage(&ctx, vec![], f.root.join("logs"), f.root.join("state"));
            app.dependencies.fixture_executable = Some(f.root.join("not-installed.exe"));
            settle(&mut app, &ctx);
            app.show_preferences(Page::Security);
            assert!(!reveal_test_control(&ctx, &mut app, WIDE, "pref-dependency-limits-apply", frame).enabled);
            assert!(!app.frame_widgets["pref-dependency-limits-reset"].enabled, "the defaults are in use");
            type_into(&ctx, &mut app, "pref-dependency-files", "300");
            enter(&ctx, &mut app);
            type_into(&ctx, &mut app, "pref-dependency-total-mib", "128");
            enter(&ctx, &mut app);
            assert_eq!(app.dependencies.limits, limits(300, 8, 128));
            // Out of range: the message, and the limits in use stay.
            type_into(&ctx, &mut app, "pref-dependency-file-mib", "65");
            enter(&ctx, &mut app);
            assert_eq!(app.dependencies.limits, limits(300, 8, 128));
            assert_eq!(app.dependencies.limits_edit.error.as_deref(), Some("Each file can be 1 to 64 MiB."));
            type_into(&ctx, &mut app, "pref-dependency-file-mib", "16");
            assert!(app.dependencies.limits_edit.error.is_none(), "typing clears the message");
            click(&ctx, &mut app, "pref-dependency-limits-apply");
            assert_eq!(app.dependencies.limits, limits(300, 16, 128));
            assert!(app.dependencies.limits_edit.text.is_none());
            assert_eq!(app.dependencies.summary(None)["limits"]["file_mib"], 16);
            app.begin_quit();
            settle(&mut app, &ctx);
            drop(app);
            let mut reopened = Rocket::with_storage(&ctx, vec![], f.root.join("logs2"), f.root.join("state"));
            reopened.dependencies.fixture_executable = Some(f.root.join("not-installed.exe"));
            settle(&mut reopened, &ctx);
            assert_eq!(reopened.dependencies.limits, limits(300, 16, 128));
            reopened.show_preferences(Page::Security);
            click(&ctx, &mut reopened, "pref-dependency-limits-reset");
            assert_eq!(reopened.dependencies.limits, Limits::default());
            // The user's control only: disabled for MCP, and a direct request changes nothing.
            reopened.dependencies.editable = false;
            assert!(!reveal_test_control(&ctx, &mut reopened, WIDE, "pref-dependency-files", frame).enabled);
            reopened.apply_dependency_limits(limits(9, 8, 64));
            assert_eq!(reopened.dependencies.limits, Limits::default());
        },
    );
}
