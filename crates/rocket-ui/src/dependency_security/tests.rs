use super::*;
use rocket_git::{CancellationToken, ReadTask};
use serde_json::json;
use std::{fs, path::Path, sync::Arc};

fn scratch() -> scan::Scratch {
    scan::Scratch::new(&std::env::temp_dir().join("rocket-dependency-tests")).unwrap()
}
fn sample() -> (scan::Inventory, Vec<PathBuf>, serde_json::Value) {
    let inv = scan::Inventory {
        files: vec![scan::Input {
            path: "package-lock.json".into(),
            ecosystem: "npm".into(),
            bytes: vec![],
        }],
        notes: vec![],
    };
    let copies = vec![PathBuf::from("C:/snapshot/0/package-lock.json")];
    let value = json!({"results":[{"source":{"path":"C:/snapshot/0/package-lock.json","type":"lockfile"},"packages":[{
        "package":{"name":"example","version":"1.0.0","ecosystem":"npm"},
        "groups":[{"ids":["GHSA-2222-3333-4444","TEST-2099-1"],"max_severity":"9.8"}],
        "vulnerabilities":[
            {"id":"GHSA-2222-3333-4444","aliases":["CVE-2099-12345"],"summary":"Fixture advisory",
             "affected":[{"package":{"name":"example","ecosystem":"npm"},"ranges":[{"events":[{"introduced":"0"},{"fixed":"2.0.0"}]}]}]},
            {"id":"TEST-2099-1","aliases":["CVE-2099-12345"],"summary":"Same advisory"}
        ]
    }]}]});
    (inv, copies, value)
}
#[test]
fn npm_missing_resolutions_are_incomplete_before_scanner_or_cache_use() {
    let root = scratch();
    for name in ["package-lock.json", "npm-shrinkwrap.json"] {
        for version in [1, 2, 3] {
            for resolved in [None, Some(json!("")), Some(json!(null)), Some(json!("garbage")), Some(json!("https:///package.tgz")), Some(json!("https://registry.npmjs.org/%zz"))] {
                let mut package = json!({"version":"1.2.3"});
                if let Some(resolved) = resolved { package["resolved"] = resolved; }
                let lock = if version == 1 {
                    json!({"lockfileVersion":version,"dependencies":{"example":package}})
                } else {
                    json!({"lockfileVersion":version,"packages":{"":{"name":"fixture"},"node_modules/example":package}})
                };
                fs::write(root.path.join(name), serde_json::to_vec(&lock).unwrap()).unwrap();
                let error = scan::scan(&root.path, &root.path.join("scanner-must-not-run.exe"), &root.path, &root.path, None, false, &CancellationToken::new()).err().expect("incomplete npm coverage");
                assert!(error.to_string().contains("npm lockfile") && error.to_string().contains("incomplete"), "{name} v{version}: {error}");
                fs::remove_file(root.path.join(name)).unwrap();
            }
        }
    }
}

#[test]
fn npm_coverage_gap_invalidates_a_previous_clean_cached_result() {
    let root = scratch();
    let cache = scratch();
    fs::write(root.path.join("package-lock.json"), r#"{"lockfileVersion":3,"packages":{"node_modules/example":{"version":"1.2.3","resolved":"garbage"}}}"#).unwrap();
    let exe = cache.path.join("must-never-run.exe");
    fs::write(&exe, "owned inert fixture").unwrap();
    fs::create_dir_all(cache.path.join("osv-scalibr/npm")).unwrap();
    fs::write(cache.path.join("osv-scalibr/npm/all.zip"), "owned inert cache").unwrap();
    let cancel = CancellationToken::new();
    let inv = scan::inventory(&root.path, &cancel).unwrap();
    let previous = scan::Cached {
        database: scan::databases(&cache.path, &inv, &cancel).unwrap(), inventory: inv,
        report: Arc::new(scan::Report { packages:1, files:1, findings:vec![], notes:vec![], checked:scan::now(), oldest_database:scan::now() }),
    };
    let error = scan::scan(&root.path, &exe, &cache.path, &cache.path, Some(&previous), false, &cancel).err().expect("old clean cache must not bypass the coverage guard");
    assert!(error.to_string().contains("incomplete"));
    let mut state = State::default();
    state.repositories.insert(42, RepositoryScan { cached:Some(previous), error:Some(error_message(&error)), next:None });
    assert_eq!(state.summary(Some(42))["status"], "incomplete");
    assert_eq!(state.summary(Some(42))["blocking"], false);
}

#[test]
#[ignore = "requires the reviewed OSV binary and synthetic offline npm database"]
fn actual_osv_missing_npm_resolution_is_not_accepted_as_clean() {
    let (exe, fixtures) = live_paths();
    let root = scratch(); let cache = scratch(); let work = scratch();
    copy_databases(&fixtures, &cache.path);
    let lockfile = root.path.join("package-lock.json");
    let mut lock: serde_json::Value = serde_json::from_slice(&fs::read(fixtures.join("vulnerable/package-lock.json")).unwrap()).unwrap();
    for package in lock["packages"].as_object_mut().unwrap().values_mut() {
        package.as_object_mut().unwrap().remove("resolved");
    }
    fs::write(&lockfile, serde_json::to_vec(&lock).unwrap()).unwrap();
    let cancel = CancellationToken::new();
    // Observe the upstream false-negative using the real, hash-verified scanner
    // and only synthetic records. No package manager or network is involved.
    let raw = rocket_git::run_offline_osv(&exe, &root.path, &cache.path, std::slice::from_ref(&lockfile), false, &cancel).unwrap();
    assert_eq!(raw.code, Some(0));
    let inventory = scan::inventory(&root.path, &cancel).unwrap();
    let unchecked = scan::parse(&raw.json, &inventory, std::slice::from_ref(&lockfile)).unwrap();
    assert!(unchecked.packages > 0 && unchecked.findings.is_empty());
    let error = scan::scan(&root.path, &exe, &cache.path, &work.path, None, true, &cancel).err().expect("upstream clean result must not be accepted");
    assert!(error.to_string().contains("incomplete"));
    fs::copy(fixtures.join("vulnerable/package-lock.json"), &lockfile).unwrap();
    let covered = scan::scan(&root.path, &exe, &cache.path, &work.path, None, true, &cancel).unwrap();
    assert!(!covered.report.findings.is_empty());
}

#[test]
#[ignore = "requires the reviewed OSV binary and synthetic offline npm database"]
fn actual_osv_malformed_npm_sources_are_not_accepted_as_clean() {
    let (exe, fixtures) = live_paths();
    let root = scratch(); let cache = scratch(); let work = scratch();
    copy_databases(&fixtures, &cache.path);
    let lockfile = root.path.join("package-lock.json");
    let original = fs::read(fixtures.join("vulnerable/package-lock.json")).unwrap();
    let cancel = CancellationToken::new();
    for source in ["garbage", "../registry-package.tgz", "https:///package.tgz", "https://registry.npmjs.org/%zz"] {
        let mut lock: serde_json::Value = serde_json::from_slice(&original).unwrap();
        for package in lock["packages"].as_object_mut().unwrap().values_mut() {
            package["resolved"] = json!(source);
        }
        fs::write(&lockfile, serde_json::to_vec(&lock).unwrap()).unwrap();
        let raw = rocket_git::run_offline_osv(&exe, &root.path, &cache.path, std::slice::from_ref(&lockfile), false, &cancel).unwrap();
        assert_eq!(raw.code, Some(0));
        let inv = scan::inventory(&root.path, &cancel).unwrap();
        let unchecked = scan::parse(&raw.json, &inv, std::slice::from_ref(&lockfile)).unwrap();
        assert!(unchecked.packages > 0 && unchecked.findings.is_empty());
        let error = scan::scan(&root.path, &exe, &cache.path, &work.path, None, true, &cancel).err().expect("malformed npm source must be incomplete");
        assert!(error.to_string().contains("incomplete"));
    }
    for source in ["https://registry.example.invalid/package.tgz", "http://registry.example.invalid/package.tgz", "https://[::1]/package.tgz", "git+https://example.invalid/package.git", "github:owner/package", "garbage.git"] {
        let mut lock: serde_json::Value = serde_json::from_slice(&original).unwrap();
        for package in lock["packages"].as_object_mut().unwrap().values_mut() {
            package["resolved"] = json!(source);
        }
        fs::write(&lockfile, serde_json::to_vec(&lock).unwrap()).unwrap();
        let covered = scan::scan(&root.path, &exe, &cache.path, &work.path, None, true, &cancel).unwrap();
        assert!(!covered.report.findings.is_empty(), "recognized source must retain advisory coverage: {source}");
    }
}

#[test]
fn defaults_are_warning_only_and_metadata_omits_private_findings() {
    let state = State::default();
    assert!(state.enabled);
    let json = state.summary(None);
    assert_eq!(json["blocking"], false);
    assert_eq!(json["status"], "not_scanned");
    assert!(json.get("package_names").is_none());
}
#[test]
fn advisory_aliases_are_grouped_and_fixed_versions_are_package_specific() {
    let (inv, copies, mut value) = sample();
    value["results"][0]["packages"][0]["vulnerabilities"][0]["affected"].as_array_mut().unwrap().push(json!({"package":{"name":"other","ecosystem":"npm"},"ranges":[{"events":[{"fixed":"99.0.0"}]}]}));
    let report = scan::parse(&serde_json::to_vec(&value).unwrap(), &inv, &copies).unwrap();
    assert_eq!(report.packages, 1);
    assert_eq!(report.findings.len(), 1);
    assert_eq!(report.findings[0].severity, "Critical");
    assert_eq!(report.findings[0].fixed, vec!["2.0.0"]);
    assert_eq!(report.findings[0].file, "package-lock.json");
}
#[test]
fn unknown_scores_are_not_reported_as_low_and_controls_are_removed() {
    let (inv, copies, mut value) = sample();
    value["results"][0]["packages"][0]["groups"][0]["max_severity"] = json!("NaN");
    value["results"][0]["packages"][0]["vulnerabilities"][0]["summary"] =
        json!("safe\u{1b}\n\u{202e}text");
    let report = scan::parse(&serde_json::to_vec(&value).unwrap(), &inv, &copies).unwrap();
    assert_eq!(report.findings[0].severity, "Unknown");
    assert_eq!(report.findings[0].summary, "safetext");
}
#[test]
fn invalid_json_missing_inventory_and_forged_paths_fail_explicitly() {
    let (inv, copies, mut value) = sample();
    for bytes in [b"not json".as_slice(), b"{}", b"{\"results\":[]}"] {
        assert!(scan::parse(bytes, &inv, &copies).is_err());
    }
    value["results"][0]["source"]["path"] = json!("C:/private/outside.json");
    assert!(scan::parse(&serde_json::to_vec(&value).unwrap(), &inv, &copies).is_err());
}
#[test]
fn withdrawn_records_are_not_displayed_as_current_advisories() {
    let (inv, copies, mut value) = sample();
    for advisory in value["results"][0]["packages"][0]["vulnerabilities"]
        .as_array_mut()
        .unwrap()
    {
        advisory["withdrawn"] = json!("2026-10-25T00:00:00Z");
    }
    assert!(
        scan::parse(&serde_json::to_vec(&value).unwrap(), &inv, &copies)
            .unwrap()
            .findings
            .is_empty()
    );
}
#[test]
fn advisory_links_reject_non_identifiers() {
    let (inv, copies, mut value) = sample();
    value["results"][0]["packages"][0]["groups"][0]["ids"] = json!(["https://untrusted.invalid/"]);
    value["results"][0]["packages"][0]["vulnerabilities"][0]["id"] =
        json!("https://untrusted.invalid/");
    assert!(scan::parse(&serde_json::to_vec(&value).unwrap(), &inv, &copies).is_err());
}
#[test]
fn discovery_is_bounded_and_missing_lockfiles_remain_visible() {
    let root = scratch();
    fs::write(
        root.path.join("Cargo.toml"),
        "[package]\nname='example'\nversion='1.0.0'",
    )
    .unwrap();
    fs::create_dir(root.path.join("nested")).unwrap();
    fs::write(root.path.join("nested/package-lock.json"), "{}").unwrap();
    fs::create_dir(root.path.join("node_modules")).unwrap();
    fs::write(root.path.join("node_modules/Cargo.lock"), "not included").unwrap();
    let inv = scan::inventory(&root.path, &CancellationToken::new()).unwrap();
    assert_eq!(inv.files.len(), 1);
    assert_eq!(inv.files[0].path, Path::new("nested/package-lock.json"));
    assert!(!inv.notes.is_empty());
    let file = fs::File::create(root.path.join("Cargo.lock")).unwrap();
    file.set_len(8 * 1024 * 1024 + 1).unwrap();
    assert!(scan::inventory(&root.path, &CancellationToken::new()).is_err());
}
#[test]
fn unrelated_lockfiles_do_not_hide_missing_ecosystems_or_unverified_workspace_members() {
    let root = scratch();
    fs::write(root.path.join("Cargo.lock"), "").unwrap();
    fs::write(root.path.join("package.json"), "{}").unwrap();
    let inv = scan::inventory(&root.path, &CancellationToken::new()).unwrap();
    assert!(inv.notes.iter().any(|n| n.contains("no matching")));
    fs::remove_file(root.path.join("package.json")).unwrap();
    fs::create_dir(root.path.join("child")).unwrap();
    fs::write(root.path.join("child/Cargo.toml"), "").unwrap();
    let inv = scan::inventory(&root.path, &CancellationToken::new()).unwrap();
    assert!(inv.notes.iter().any(|n| n.contains("ancestor lockfile")));
}
#[test]
fn malformed_advisories_and_missing_groups_never_produce_a_clean_result() {
    let (inv, copies, original) = sample();
    for replacement in [json!({}), json!([{}]), json!([{"id":""}])] {
        let mut value = original.clone();
        value["results"][0]["packages"][0]["vulnerabilities"] = replacement;
        assert!(scan::parse(&serde_json::to_vec(&value).unwrap(), &inv, &copies).is_err());
    }
    let mut value = original;
    value["results"][0]["packages"][0]["groups"] = json!([]);
    assert!(scan::parse(&serde_json::to_vec(&value).unwrap(), &inv, &copies).is_err());
}
#[test]
fn inventory_cancellation_and_empty_projects_do_not_claim_success() {
    let root = scratch();
    let cancel = CancellationToken::new();
    cancel.cancel();
    assert!(matches!(
        scan::inventory(&root.path, &cancel),
        Err(Error::Cancelled)
    ));
    assert!(
        scan::scan(
            &root.path,
            &root.path.join("missing.exe"),
            &root.path,
            &root.path,
            None,
            true,
            &CancellationToken::new()
        )
        .is_err()
    );
}
#[test]
fn missing_or_oversized_database_is_not_a_clean_scan() {
    let (inv, _, _) = sample();
    let root = scratch();
    assert!(scan::databases(&root.path, &inv, &CancellationToken::new()).is_err());
    let folder = root.path.join("osv-scalibr/npm");
    fs::create_dir_all(&folder).unwrap();
    fs::File::create(folder.join("all.zip"))
        .unwrap()
        .set_len(512 * 1024 * 1024 + 1)
        .unwrap();
    assert!(scan::databases(&root.path, &inv, &CancellationToken::new()).is_err());
}
#[test]
fn temporary_snapshot_is_removed_on_scope_exit() {
    let path = {
        let root = scratch();
        root.path.clone()
    };
    assert!(!path.exists());
}
#[test]
fn failed_scan_messages_never_echo_output_or_private_paths() {
    let err = Error::Io {
        action: "PRIVATE_ACTION",
        source: std::io::Error::other("PRIVATE_OUTPUT_TOKEN"),
    };
    let shown = error_message(&err);
    assert!(!shown.contains("PRIVATE"));
}
#[test]
fn unc_snapshot_sources_match_the_same_canonical_windows_path() {
    let (inv, _, mut value) = sample();
    let copies = vec![PathBuf::from(
        r"\\?\UNC\server\share\scan\package-lock.json",
    )];
    value["results"][0]["source"]["path"] = json!(r"\\server\share\scan\package-lock.json");
    assert_eq!(
        scan::parse(&serde_json::to_vec(&value).unwrap(), &inv, &copies)
            .unwrap()
            .findings
            .len(),
        1
    );
}

fn live_paths() -> (PathBuf, PathBuf) {
    (
        std::env::var_os("ROCKET_OSV_TEST_EXE")
            .map(PathBuf::from)
            .expect("Set ROCKET_OSV_TEST_EXE to the reviewed OSV binary"),
        std::env::var_os("ROCKET_OSV_TEST_FIXTURES")
            .map(PathBuf::from)
            .expect("Set ROCKET_OSV_TEST_FIXTURES to synthetic fixture directory"),
    )
}
fn copy_databases(source: &Path, dest: &Path) {
    for ecosystem in ["npm", "crates.io"] {
        let target = dest.join("osv-scalibr").join(ecosystem);
        fs::create_dir_all(&target).unwrap();
        fs::copy(
            source
                .join("cache/osv-scalibr")
                .join(ecosystem)
                .join("all.zip"),
            target.join("all.zip"),
        )
        .unwrap();
    }
}
#[test]
#[ignore = "requires explicitly approved OSV binary and synthetic offline databases"]
fn actual_osv_detects_vulnerable_and_fixed_dependencies_and_invalidates_cache() {
    let (exe, fixtures) = live_paths();
    let root = scratch();
    let work = scratch();
    let cache = scratch();
    copy_databases(&fixtures, &cache.path);
    for name in ["Cargo.lock", "package-lock.json"] {
        fs::copy(fixtures.join("vulnerable").join(name), root.path.join(name)).unwrap();
    }
    let cancel = CancellationToken::new();
    let vulnerable = scan::scan(
        &root.path,
        &exe,
        &cache.path,
        &work.path,
        None,
        true,
        &cancel,
    )
    .unwrap();
    assert_eq!(vulnerable.report.packages, 2);
    assert_eq!(vulnerable.report.findings.len(), 2);
    assert!(
        vulnerable
            .report
            .findings
            .iter()
            .all(|f| f.fixed == vec!["2.0.0"])
    );
    let reused = scan::scan(
        &root.path,
        &exe,
        &cache.path,
        &work.path,
        Some(&vulnerable),
        false,
        &cancel,
    )
    .unwrap();
    assert!(Arc::ptr_eq(&vulnerable.report, &reused.report));
    for name in ["Cargo.lock", "package-lock.json"] {
        fs::copy(fixtures.join("fixed").join(name), root.path.join(name)).unwrap();
    }
    let fixed = scan::scan(
        &root.path,
        &exe,
        &cache.path,
        &work.path,
        Some(&vulnerable),
        false,
        &cancel,
    )
    .unwrap();
    assert!(fixed.report.findings.is_empty());
    assert_eq!(fixed.report.packages, 2);
    fs::write(cache.path.join("osv-scalibr/npm/all.zip"), "corrupt cache").unwrap();
    assert!(
        scan::scan(
            &root.path,
            &exe,
            &cache.path,
            &work.path,
            Some(&fixed),
            false,
            &cancel
        )
        .is_err()
    );
}
#[test]
#[ignore = "requires explicitly approved OSV binary and synthetic offline databases"]
fn actual_osv_ignores_repo_suppressions_and_never_runs_build_scripts() {
    let (exe, fixtures) = live_paths();
    let root = scratch();
    let work = scratch();
    let cache = scratch();
    copy_databases(&fixtures, &cache.path);
    for name in ["Cargo.lock", "package-lock.json"] {
        fs::copy(fixtures.join("vulnerable").join(name), root.path.join(name)).unwrap();
    }
    fs::write(
        root.path.join("osv-scanner.toml"),
        "[[PackageOverrides]]\nignore = true\n",
    )
    .unwrap();
    fs::write(
        root.path.join("Cargo.toml"),
        "[package]\nname='fixture-crate'\nversion='1.0.0'\nbuild='build.rs'\n",
    )
    .unwrap();
    fs::write(
        root.path.join("build.rs"),
        "fn main(){panic!(\"BUILD_SCRIPT_MUST_NOT_RUN\");}",
    )
    .unwrap();
    fs::write(
        root.path.join(".env"),
        "PRIVATE_FIXTURE_SENTINEL=do-not-copy",
    )
    .unwrap();
    let before = fs::read(root.path.join("Cargo.lock")).unwrap();
    let result = scan::scan(
        &root.path,
        &exe,
        &cache.path,
        &work.path,
        None,
        true,
        &CancellationToken::new(),
    )
    .unwrap();
    assert_eq!(result.report.findings.len(), 2);
    assert_eq!(before, fs::read(root.path.join("Cargo.lock")).unwrap());
    assert!(!root.path.join("target").exists());
    assert_eq!(
        fs::read_dir(&work.path).unwrap().count(),
        1,
        "only the fixture's own empty config remains"
    );
}
#[test]
#[ignore = "requires explicitly approved OSV binary and synthetic offline databases"]
fn actual_osv_malformed_and_empty_files_fail_without_clean_results() {
    let (exe, fixtures) = live_paths();
    let root = scratch();
    let work = scratch();
    let cache = scratch();
    copy_databases(&fixtures, &cache.path);
    for text in ["not json", "{}", "{\"lockfileVersion\":3,\"packages\":{}}"] {
        fs::write(root.path.join("package-lock.json"), text).unwrap();
        assert!(
            scan::scan(
                &root.path,
                &exe,
                &cache.path,
                &work.path,
                None,
                true,
                &CancellationToken::new()
            )
            .is_err()
        );
    }
}
#[test]
#[ignore = "requires explicitly approved OSV binary and synthetic offline databases"]
fn actual_scan_cancellation_finishes_and_cleans_its_snapshot() {
    let (exe, fixtures) = live_paths();
    let root = scratch();
    let work = scratch();
    let cache = scratch();
    copy_databases(&fixtures, &cache.path);
    fs::copy(
        fixtures.join("vulnerable/Cargo.lock"),
        root.path.join("Cargo.lock"),
    )
    .unwrap();
    let rp = root.path.clone();
    let wp = work.path.clone();
    let cp = cache.path.clone();
    let task = ReadTask::spawn(move |cancel| scan::scan(&rp, &exe, &cp, &wp, None, true, &cancel))
        .unwrap();
    std::thread::sleep(Duration::from_millis(20));
    task.cancel();
    let start = Instant::now();
    assert!(matches!(task.wait(), Err(Error::Cancelled)));
    assert!(start.elapsed() < Duration::from_secs(5));
    assert_eq!(fs::read_dir(&work.path).unwrap().count(), 1);
}

#[test]
#[ignore = "requires explicitly approved OSV binary and extra synthetic ecosystem fixtures"]
fn actual_osv_checks_eight_more_dependency_formats_offline() {
    let (exe, fixtures) = live_paths();
    let extra = fixtures.parent().unwrap().join("extra-fixtures");
    let work = scratch();
    for version in ["1.0.0", "2.0.0"] {
        for name in [
            "requirements-txt",
            "go-mod",
            "gradle-lockfile",
            "yarn-lock",
            "packages-lock-json",
            "composer-lock",
            "Gemfile-lock",
            "pubspec-lock",
        ] {
            let result = scan::scan(
                &extra.join(version).join(name),
                &exe,
                &extra.join("cache"),
                &work.path,
                None,
                true,
                &CancellationToken::new(),
            )
            .unwrap_or_else(|e| panic!("{name} {version}: {e:?}"));
            assert_eq!(
                result.report.findings.len(),
                usize::from(version == "1.0.0"),
                "{name} {version}"
            );
        }
    }
}

fn frame(ctx: &Context, app: &mut Rocket, size: Vec2, events: Vec<Event>) {
    app.poll(ctx);
    app.frame_widgets.clear();
    let mut out = ctx.run_ui(
        RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
            events,
            ..Default::default()
        },
        |ui| {
            app.dialog_ui(ui.ctx());
        },
    );
    out.textures_delta.clear();
}
fn settle(app: &mut Rocket, ctx: &Context) {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        app.poll(ctx);
        if app.backend.is_some() && !app.busy() && !app.dependencies.running() {
            break;
        }
        assert!(Instant::now() < deadline, "Rocket did not settle");
        std::thread::sleep(Duration::from_millis(5));
    }
}
#[test]
fn pointer_preference_persists_off_after_restart() {
    crate::test_common::isolated(
        "ROCKET_DEPENDENCY_PERSIST_CASE",
        "dependency_security::tests::pointer_preference_persists_off_after_restart",
        || {
            let f = crate::test_common::Fixture::new();
            let ctx = Context::default();
            let mut app =
                Rocket::with_storage(&ctx, vec![], f.root.join("logs"), f.root.join("state"));
            settle(&mut app, &ctx);
            assert!(app.dependencies.enabled);
            app.show_preferences(crate::preferences::Page::Security);
            for _ in 0..3 {
                frame(&ctx, &mut app, vec2(1280.0, 860.0), vec![]);
            }
            let pos = app.frame_widgets["pref-dependency-warnings"].rect.center();
            frame(
                &ctx,
                &mut app,
                vec2(1280.0, 860.0),
                vec![
                    Event::PointerMoved(pos),
                    Event::PointerButton {
                        pos,
                        button: PointerButton::Primary,
                        pressed: true,
                        modifiers: Modifiers::NONE,
                    },
                    Event::PointerButton {
                        pos,
                        button: PointerButton::Primary,
                        pressed: false,
                        modifiers: Modifiers::NONE,
                    },
                ],
            );
            assert!(!app.dependencies.enabled);
            app.begin_quit();
            settle(&mut app, &ctx);
            drop(app);
            let mut reopened =
                Rocket::with_storage(&ctx, vec![], f.root.join("logs2"), f.root.join("state"));
            settle(&mut reopened, &ctx);
            assert!(!reopened.dependencies.enabled);
            assert!(!reopened.dependencies.running());
        },
    );
}
#[test]
fn background_scan_leaves_git_available_but_shutdown_waits_for_cleanup() {
    crate::test_common::isolated(
        "ROCKET_DEPENDENCY_QUIT_CASE",
        "dependency_security::tests::background_scan_leaves_git_available_but_shutdown_waits_for_cleanup",
        || {
            let f = crate::test_common::Fixture::new();
            let ctx = Context::default();
            let mut app = Rocket::with_storage(
                &ctx,
                vec![f.repository.clone()],
                f.root.join("logs"),
                f.root.join("state"),
            );
            settle(&mut app, &ctx);
            app.dependencies.enabled = false;
            app.dependencies.repositories.clear();
            settle(&mut app, &ctx);
            let owner = app.tab().unwrap().id;
            let path = app.tab().unwrap().path.clone();
            let (release, receiver) = std::sync::mpsc::channel();
            let task = ReadTask::spawn(move |cancel| {
                receiver.recv_timeout(Duration::from_secs(10)).unwrap();
                cancel.check()?;
                panic!("Shutdown did not cancel the scan");
            })
            .unwrap();
            app.dependencies.job = Some(Job {
                owner,
                path,
                update: false,
                discard: false,
                task,
            });
            assert!(!app.busy());
            assert!(app.can_write());
            app.request_close();
            assert!(app.quitting);
            assert!(app.busy());
            release.send(()).unwrap();
            settle(&mut app, &ctx);
            assert!(!app.busy());
            assert!(!app.dependencies.running());
            assert_eq!(
                app.dependencies.summary(Some(owner))["status"],
                "not_scanned"
            );
        },
    );
}
#[test]
fn preferences_controls_render_at_supported_sizes_and_mcp_cannot_change_policy() {
    crate::test_common::isolated(
        "ROCKET_DEPENDENCY_UI_CASE",
        "dependency_security::tests::preferences_controls_render_at_supported_sizes_and_mcp_cannot_change_policy",
        || {
            let f = crate::test_common::Fixture::new();
            let ctx = Context::default();
            let mut app =
                Rocket::with_storage(&ctx, vec![], f.root.join("logs"), f.root.join("state"));
            app.dependencies.fixture_executable = Some(f.root.join("not-installed.exe"));
            let deadline = Instant::now() + Duration::from_secs(15);
            while app.backend.is_none() || app.busy() {
                app.poll(&ctx);
                assert!(Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(5));
            }
            app.show_preferences(crate::preferences::Page::Security);
            for size in [vec2(1280.0, 860.0), vec2(780.0, 620.0)] {
                app.dependencies.setup_page = false;
                for _ in 0..3 {
                    frame(&ctx, &mut app, size, vec![]);
                }
                for key in [
                    "pref-page-security",
                    "pref-secret-protection",
                    "pref-dependency-warnings",
                    "pref-dependency-push",
                    "dependency-setup-open",
                ] {
                    for _ in 0..12 {
                        if app.frame_widgets[key].rect.height() >= 18.0 { break; }
                        let scroll = app.frame_widgets["prefs-scroll"].rect;
                        let delta = if app.frame_widgets[key].rect.center().y < scroll.center().y { 150.0 } else { -150.0 };
                        frame(&ctx, &mut app, size, vec![Event::PointerMoved(scroll.center()), Event::MouseWheel { unit: MouseWheelUnit::Point, delta: vec2(0.0, delta), phase: TouchPhase::Move, source: MouseWheelSource::Unknown, modifiers: Modifiers::NONE }]);
                    }
                    let control = &app.frame_widgets[key];
                    assert!(
                        control.rect.width() > 10.0 && control.rect.height() > 10.0,
                        "{key} {size:?}"
                    );
                    assert!(Rect::from_min_size(Pos2::ZERO, size).contains_rect(control.rect));
                }
            }
            app.dependencies.editable = false;
            frame(&ctx, &mut app, vec2(1280.0, 860.0), vec![]);
            assert!(!app.frame_widgets["pref-dependency-warnings"].enabled);
            assert!(!app.frame_widgets["pref-dependency-push"].enabled);
            app.dependencies.setup_page = true;
            frame(&ctx, &mut app, vec2(1280.0, 860.0), vec![]);
            assert!(!app.frame_widgets["dependencies-update"].enabled);
            assert!(!app.frame_widgets["dependency-scanner-choose"].enabled);
            // Disabling controls is not the authorization boundary: direct requests
            // from the restricted host must also leave the profile untouched.
            app.dependencies.setup.loaded = true;
            app.dependencies.setup.ready = true;
            app.choose_dependency_scanner(std::env::current_exe().unwrap());
            app.start_dependency_scan(true, true);
            assert!(!app.dependencies.running());
            assert!(
                !f.root
                    .join("state/dependency-security/scanner-selection")
                    .exists()
            );
            assert!(app.dependencies.enabled);
        },
    );
}

#[test]
#[ignore = "requires approved local OSV and explicitly authorized public advisory download"]
fn fresh_profile_downloads_public_data_and_restarts_with_its_selected_scanner() {
    crate::test_common::isolated(
        "ROCKET_OSV_FRESH_PROFILE_CASE",
        "dependency_security::tests::fresh_profile_downloads_public_data_and_restarts_with_its_selected_scanner",
        || {
            let approved =
                PathBuf::from(std::env::var_os("ROCKET_OSV_TEST_EXE").expect("reviewed local OSV"));
            let f = crate::test_common::Fixture::new();
            f.write("Cargo.lock", concat!(
                "version = 3\n\n[[package]]\nname = \"rocket-setup-fixture\"\n",
                "version = \"1.0.0\"\nsource = \"registry+https://github.com/rust-lang/crates.io-index\"\n",
            ));
            let ctx = Context::default();
            let profile = f.root.join("fresh-profile");
            assert!(!profile.exists());
            let mut app = Rocket::with_storage(
                &ctx,
                vec![f.repository.clone()],
                f.root.join("logs"),
                profile.clone(),
            );
            // Simulate the distributed package, which intentionally has no OSV helper.
            app.dependencies.fixture_executable = Some(f.root.join("not-bundled.exe"));
            settle(&mut app, &ctx);
            assert!(!app.dependencies.setup.ready);
            assert!(!profile.join("dependency-security/data").exists());
            app.choose_dependency_scanner(approved.clone());
            settle(&mut app, &ctx);
            assert!(app.dependencies.setup.ready);
            assert_eq!(app.dependencies.setup.program.as_ref(), Some(&approved));
            assert!(
                app.dependencies
                    .repositories
                    .values()
                    .all(|s| s.cached.is_none())
            );
            app.start_dependency_scan(true, true);
            let deadline = Instant::now() + Duration::from_secs(240);
            loop {
                app.poll(&ctx);
                if !app.dependencies.running() {
                    let state = &app.dependencies.repositories[&app.tab().unwrap().id];
                    assert!(
                        state.error.is_none(),
                        "Fresh-profile download/scan failed: {:?}",
                        state.error
                    );
                    assert_eq!(
                        state
                            .cached
                            .as_ref()
                            .expect("completed offline scan")
                            .report
                            .packages,
                        1
                    );
                    break;
                }
                assert!(
                    Instant::now() < deadline,
                    "Public advisory download did not settle"
                );
                std::thread::sleep(Duration::from_millis(20));
            }
            let data = profile.join("dependency-security/data/osv-scalibr/crates.io/all.zip");
            assert!(fs::metadata(&data).unwrap().len() > 0);
            let hash =
                rocket_git::osv_file_digest(&data, 512 * 1024 * 1024, &CancellationToken::new())
                    .unwrap();
            app.request_close();
            settle(&mut app, &ctx);
            drop(app);
            let mut reopened = Rocket::with_storage(
                &ctx,
                vec![f.repository.clone()],
                f.root.join("logs-restart"),
                profile.clone(),
            );
            reopened.dependencies.fixture_executable = Some(f.root.join("not-bundled.exe"));
            settle(&mut reopened, &ctx);
            assert!(reopened.dependencies.setup.ready);
            assert_eq!(reopened.dependency_paths().0, approved);
            assert_eq!(
                reopened.dependencies.summary(reopened.tab().map(|t| t.id))["status"],
                "scanned"
            );
            assert_eq!(
                rocket_git::osv_file_digest(&data, 512 * 1024 * 1024, &CancellationToken::new())
                    .unwrap(),
                hash
            );
            reopened.request_close();
            settle(&mut reopened, &ctx);
            drop(reopened);
            let other_profile = f.root.join("other-profile");
            let mut other = Rocket::with_storage(
                &ctx,
                vec![],
                f.root.join("logs-other"),
                other_profile.clone(),
            );
            other.dependencies.fixture_executable = Some(f.root.join("not-bundled.exe"));
            settle(&mut other, &ctx);
            assert!(!other.dependencies.setup.ready);
            assert!(!other_profile.join("dependency-security/data").exists());
            assert!(
                !other_profile
                    .join("dependency-security/scanner-selection")
                    .exists()
            );
            other.request_close();
            settle(&mut other, &ctx);
        },
    );
}
