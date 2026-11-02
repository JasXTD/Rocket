use super::*;
use serde_json::{Value, json};
use std::{fs, path::PathBuf};

struct Fixture(PathBuf);

#[test]
fn repository_permission_is_explicit_and_part_of_draft_binding() {
    let f = Fixture::new();
    install_example(&f.0, "repository").unwrap();
    let package = Package::load(&f.0.join("repository")).unwrap();
    assert!(package.repository && !package.network && !package.writes);
    let mut profile = profiles::default_connection("Default");
    let old_binding = profiles::binding(&profile);
    profile["repository"] = json!(false);
    assert_eq!(profiles::binding(&profile), old_binding);
    profile["repository"] = json!(true);
    assert_ne!(profiles::binding(&profile), old_binding);
    let mut store = profiles::empty(); profiles::ensure(&mut store, "repository");
    store["profiles"]["repository"]["connections"]["default"]["repository"] = json!("yes");
    assert!(profiles::validate(&store).is_err());
}

#[test]
fn turning_the_share_on_carries_a_draft_and_nothing_else_does() {
    let mut profile = profiles::default_connection("Default");
    let mut draft = json!({"binding":profiles::binding(&profile)});
    profile["repository"] = json!(true);
    profiles::carry_draft(&mut draft, &profile);
    assert_eq!(draft["binding"], profiles::binding(&profile));

    profile["repository"] = json!(false);
    profiles::carry_draft(&mut draft, &profile);
    assert_ne!(draft["binding"], profiles::binding(&profile), "it may hold what was shared");

    let mut draft = json!({"binding":profiles::binding(&profile)});
    profile["repository"] = json!(true);
    profile["origin"] = json!("https://other.example.invalid");
    profiles::carry_draft(&mut draft, &profile);
    assert_ne!(draft["binding"], profiles::binding(&profile), "another server is another connection");
}

#[test]
fn repository_context_has_no_paths_contents_or_identities_and_supports_unborn_bare_and_cancel() {
    let f = Fixture::new();
    let git = |directory: &std::path::Path, args: &[&str]| {
        let out = std::process::Command::new("git").current_dir(directory)
            .env("GIT_CONFIG_GLOBAL", f.0.join("empty-config"))
            .env("GIT_CONFIG_NOSYSTEM", "1").env("GIT_TEMPLATE_DIR", f.0.join("empty-template"))
            .args(args).output().unwrap();
        assert!(out.status.success(), "fixture Git failed");
    };
    fs::write(f.0.join("empty-config"), "").unwrap(); fs::create_dir(f.0.join("empty-template")).unwrap();
    let work = f.0.join("PRIVATE-資料"); fs::create_dir(&work).unwrap();
    git(&work, &["init", "--initial-branch=feature/資料"]);
    let client = rocket_git::GitClient::discover().unwrap();
    let cancel = rocket_git::CancellationToken::new();
    let context = repository_context(&client, &work, None, &cancel).unwrap();
    assert_eq!(context["branch"], "feature/資料"); assert_eq!(context["unborn"], true);
    for (key, value) in [("user.name", "PRIVATE identity"), ("user.email", "fixture@example.invalid"), ("commit.gpgsign", "false"), ("core.autocrlf", "false")] { git(&work, &["config", key, value]); }
    fs::write(work.join("private-file.txt"), "PRIVATE file contents\n").unwrap();
    git(&work, &["add", "--", "private-file.txt"]); git(&work, &["commit", "-m", "PRIVATE commit message"]);
    fs::write(work.join("private-file.txt"), "staged\n").unwrap();git(&work, &["add", "--", "private-file.txt"]);
    fs::write(work.join("private-file.txt"), "unstaged\n").unwrap();fs::write(work.join("untracked.txt"), "PRIVATE untracked").unwrap();
    let before = fs::read(work.join(".git/index")).unwrap();
    let context = repository_context(&client, &work, None, &cancel).unwrap();
    assert_eq!(context["changes"], json!({"staged":1,"unstaged":1,"untracked":1,"conflicted":0}));
    assert!(!context.to_string().contains("PRIVATE"));assert!(!context.to_string().contains("private-file"));
    assert!(!context.to_string().contains("fixture@example"));assert_eq!(fs::read(work.join(".git/index")).unwrap(), before);
    git(&work, &["checkout", "--detach"]);
    assert_eq!(repository_context(&client,&work,None,&cancel).unwrap()["detached"], true);
    let bare = f.0.join("bare.git"); fs::create_dir(&bare).unwrap();git(&bare, &["init", "--bare", "--initial-branch=main"]);
    let context = repository_context(&client,&bare,None,&cancel).unwrap();assert_eq!(context["bare"], true);assert!(context["changes"].is_null());
    cancel.cancel();assert!(matches!(repository_context(&client,&work,None,&cancel),Err(rocket_git::Error::Cancelled)));
}

#[test]
fn broken_package_does_not_block_healthy_catalog() {
    let f = Fixture::new();
    install_example(&f.0, "playground").unwrap();
    fs::create_dir(f.0.join("broken")).unwrap();
    fs::write(f.0.join("broken/integration.json"), "invalid fixture").unwrap();
    let catalog = discover(&f.0, &rocket_git::CancellationToken::new()).unwrap();
    assert_eq!(catalog.packages.len(), 1);
    assert_eq!(catalog.packages[0].id, "playground");
    assert_eq!(catalog.issues.len(), 1);
    assert_eq!(catalog.issues[0].folder, "broken");
    assert!(
        fs::read_to_string(f.0.join("broken/integration.json"))
            .unwrap()
            .contains("invalid fixture")
    );
}
impl Fixture {
    fn new() -> Self {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("rocket-lua-test-{}-{stamp}", std::process::id()));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn example_install_preserves_existing_files_and_approval_binds_exact_source() {
    let f = Fixture::new();
    let root = f.0.join("整合");
    install_example(&root, "playground").unwrap();
    let dir = root.join("playground");
    let p = Package::load(&dir).unwrap();
    let snapshot = p.snapshot();
    assert!(p.approved(&snapshot));
    let source = dir.join("main.lua");
    fs::write(
        &source,
        "-- local edits\nreturn function() return ui.text('資料') end",
    )
    .unwrap();
    assert!(install_example(&root, "playground").is_err());
    assert!(
        fs::read_to_string(&source)
            .unwrap()
            .starts_with("-- local edits")
    );
    assert!(p.verify().is_err());
    assert!(!Package::load(&dir).unwrap().approved(&snapshot));
    let c = rocket_git::CancellationToken::new();
    c.cancel();
    assert!(discover(&root, &c).is_err());
}

#[test]
fn manifest_permissions_bytecode_and_duplicate_ids_fail_closed() {
    let f = Fixture::new();
    install_example(&f.0, "playground").unwrap();
    let dir = f.0.join("playground");
    let manifest = dir.join("integration.json");
    let original = fs::read(&manifest).unwrap();
    let mut v: serde_json::Value = serde_json::from_slice(&original).unwrap();
    v["permissions"]["shell"] = json!(true);
    fs::write(&manifest, v.to_string()).unwrap();
    assert!(Package::load(&dir).is_err());
    fs::write(&manifest, &original).unwrap();
    fs::write(dir.join("main.lua"), b"\x1bLua bytecode").unwrap();
    assert!(Package::load(&dir).is_err());
    fs::write(
        dir.join("main.lua"),
        "return function() return ui.text('ok') end",
    )
    .unwrap();
    let other = f.0.join("duplicate");
    fs::create_dir(&other).unwrap();
    fs::write(other.join("integration.json"), original).unwrap();
    fs::write(
        other.join("main.lua"),
        "return function() return ui.text('ok') end",
    )
    .unwrap();
    install_example(&f.0, "jira").unwrap();
    let catalog = discover(&f.0, &rocket_git::CancellationToken::new()).unwrap();
    assert_eq!(catalog.packages.len(), 1);
    assert_eq!(catalog.packages[0].id, "jira");
    assert_eq!(catalog.issues.len(), 2);
    assert!(
        catalog
            .issues
            .iter()
            .all(|e| e.error.starts_with("integration_duplicate:"))
    );
}

#[test]
fn encrypted_store_preserves_external_edits_and_uncertainty() {
    let f = Fixture::new();
    let (value, raw) = storage::load(&f.0).unwrap();
    assert!(raw.is_none());
    let mut value = value;
    profiles::ensure(&mut value, "test");
    profiles::connection_mut(&mut value, "test", "default")["uncertain"] = json!(true);
    profiles::connection_mut(&mut value, "test", "default")["token"] =
        json!("fixture-private-credential");
    let saved = storage::save(&f.0, &value, None).unwrap();
    assert!(!String::from_utf8_lossy(&saved).contains("fixture-private-credential"));
    let (loaded, current) = storage::load(&f.0).unwrap();
    assert_eq!(loaded, value);
    assert_eq!(current.as_deref(), Some(saved.as_slice()));
    assert!(storage::save(&f.0, &value, None).is_err());
    fs::write(f.0.join("integration-accounts.dpapi"), "external edit").unwrap();
    assert!(storage::load(&f.0).is_err());
    assert!(storage::save(&f.0, &value, Some(&saved)).is_err());
    assert_eq!(
        fs::read_to_string(f.0.join("integration-accounts.dpapi")).unwrap(),
        "external edit"
    );
}

#[cfg(feature = "runtime")]
#[test]
fn lua_memory_and_bytecode_are_bounded_and_cached_reads_replay() {
    let run = |source: &str, responses: serde_json::Value| {
        runtime::run(json!({"api":1,"source":source,"context":{},"responses":responses}))
    };
    assert_eq!(
        run(
            "return function() return string.rep('x',40000000) end",
            json!({})
        )["kind"],
        "error"
    );
    assert_eq!(run("\u{1b}Lua", json!({}))["kind"], "error");
    let req = Request::parse(&json!({"method":"GET","path":"/issues"})).unwrap();
    let mut cache = serde_json::Map::new();
    cache.insert(req.key(), json!({"status":200,"json":{"name":"資料"}}));
    let v = run(
        "return function() local r=rocket.http{method='GET',path='/issues'}; return ui.text(r.json.name) end",
        cache.into(),
    );
    assert_eq!(v["kind"], "view");
    assert_eq!(v["view"]["text"], "資料");
}

#[test]
fn legacy_connections_migrate_in_memory_without_touching_the_file() {
    let f = Fixture::new();
    let legacy = json!({"version":1,"profiles":{"jira":{"enabled":true,"uncertain":true,
        "origin":"https://fixture.invalid","auth":"bearer","token":"fixture-legacy",
        "remember":true,"settings":{"project":"ROCKET"},"approved":{}}}});
    let raw = rocket_git::protect_hosting_secret(legacy.to_string().as_bytes(), true).unwrap();
    let path = f.0.join("integration-accounts.dpapi");
    fs::write(&path, &raw).unwrap();
    let (loaded, snapshot) = storage::load(&f.0).unwrap();
    assert_eq!(loaded["version"], 2);
    assert_eq!(profiles::selected(&loaded, "jira"), "default");
    let p = profiles::connection(&loaded, "jira", "default");
    assert_eq!(p["uncertain"], true);
    assert_eq!(p["enabled"], true);
    assert_eq!(p["token"], "fixture-legacy");
    assert_eq!(p["name"], "Default");
    assert_eq!(fs::read(&path).unwrap(), raw);
    let next = storage::save(&f.0, &loaded, snapshot.as_deref()).unwrap();
    assert_ne!(next, raw);
    assert_eq!(storage::load(&f.0).unwrap().0, loaded);
}

#[test]
fn connection_drafts_preserve_unicode_state_and_refuse_changed_binding() {
    let f = Fixture::new();
    install_example(&f.0, "playground").unwrap();
    let package = Package::load(&f.0.join("playground")).unwrap();
    let node = Node::parse(&json!({"type":"page","title":"資料","children":[
        {"type":"input","id":"note","label":"Draft"},
        {"type":"checkbox","id":"flag","label":"Include"},
        {"type":"select","id":"kind","options":["work","personal"]}
    ]}))
    .unwrap();
    let values = std::collections::HashMap::from([
        ("note".into(), json!("資料 🚀 e\u{301}")),
        ("flag".into(), json!(true)),
        ("kind".into(), json!("work")),
    ]);
    let mut store = profiles::empty();
    profiles::ensure(&mut store, "playground");
    let mut work = profiles::default_connection("Work 資料");
    work["uncertain"] = json!(true);
    work["draft"] =
        profiles::capture(&package, &work, &node, &values, &json!({"count":2})).unwrap();
    store["profiles"]["playground"]["connections"]["work"] = work.clone();
    store["profiles"]["playground"]["connections"]["personal"] =
        profiles::default_connection("Personal");
    let raw = storage::save(&f.0, &store, None).unwrap();
    assert!(!raw.windows("資料".len()).any(|w| w == "資料".as_bytes()));
    let loaded = storage::load(&f.0).unwrap().0;
    let restored = profiles::restore(
        &package,
        profiles::connection(&loaded, "playground", "work"),
    )
    .unwrap()
    .unwrap();
    assert_eq!(restored.values, values);
    assert_eq!(restored.state["count"], 2);
    assert_eq!(restored.view.snapshot(), node.snapshot());
    assert!(
        profiles::restore(
            &package,
            profiles::connection(&loaded, "playground", "personal")
        )
        .unwrap()
        .is_none()
    );
    work["origin"] = json!("https://other.invalid");
    assert!(profiles::restore(&package, &work).is_err());
    work["origin"] = Value::Null;
    work["revision"] = json!("new-account");
    assert!(profiles::restore(&package, &work).is_err());
    fs::write(
        f.0.join("playground/main.lua"),
        "return function() return ui.text('changed') end",
    )
    .unwrap();
    let changed = Package::load(&f.0.join("playground")).unwrap();
    assert!(
        profiles::restore(
            &changed,
            profiles::connection(&loaded, "playground", "work")
        )
        .is_err()
    );
}

#[test]
fn draft_storage_rejects_invalid_values_and_connection_limits() {
    let f = Fixture::new();
    install_example(&f.0, "playground").unwrap();
    let package = Package::load(&f.0.join("playground")).unwrap();
    let node = Node::parse(&json!({"type":"input","id":"note","label":"Note"})).unwrap();
    let p = profiles::default_connection("Default");
    let mut values = std::collections::HashMap::from([("missing".into(), json!("stale"))]);
    assert!(profiles::capture(&package, &p, &node, &values, &json!({})).is_err());
    values.clear();
    values.insert("note".into(), json!("x".repeat(4097)));
    assert!(profiles::capture(&package, &p, &node, &values, &json!({})).is_err());
    values.clear();
    assert!(
        profiles::capture(
            &package,
            &p,
            &node,
            &values,
            &json!({"large":"x".repeat(32768)})
        )
        .is_err()
    );
    let mut store = profiles::empty();
    profiles::ensure(&mut store, "playground");
    for n in 0..16 {
        store["profiles"]["playground"]["connections"][format!("c{n}")] =
            profiles::default_connection(&format!("Connection {n}"));
    }
    assert!(storage::save(&f.0, &store, None).is_err());
    store["profiles"]["playground"]["connections"]
        .as_object_mut()
        .unwrap()
        .remove("c15");
    store["profiles"]["playground"]["connections"]["c0"]["name"] = json!("default");
    assert!(storage::save(&f.0, &store, None).is_err());
    assert!(!f.0.join("integration-accounts.dpapi").exists());
}

fn package_folder(root: &std::path::Path, folder: &str, id: &str, version: &str, source: &str) -> PathBuf {
    let directory = root.join(folder);
    fs::create_dir_all(&directory).unwrap();
    let manifest = json!({"api":1,"id":id,"name":"Team dashboard","version":version,
        "permissions":{"network":true,"repository":true}});
    fs::write(directory.join("integration.json"), manifest.to_string()).unwrap();
    fs::write(directory.join("main.lua"), source).unwrap();
    directory
}

#[test]
fn bundled_jira_lists_before_it_is_on_disk_and_lands_on_its_own_folder() {
    let f = Fixture::new();
    let root = f.0.join("integrations");
    let list = bundled(&root);
    assert_eq!(list.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(), ["jira", "playground"]);
    assert_eq!(list[1].name, "PlayGround");
    assert!(list[0].network && list[0].writes && list[0].repository);
    assert!(list[0].verify().is_err(), "not on disk yet");
    let landed = install_package(&root, &list[0], None).unwrap();
    assert_eq!(landed.directory, root.join("jira"));
    assert!(landed.same_files(&list[0]));
    let catalog = discover(&root, &rocket_git::CancellationToken::new()).unwrap();
    assert_eq!(catalog.packages.len(), 1);
    let again = install_package(&root, &list[0], None).unwrap_err();
    assert!(matches!(again, rocket_git::Error::InvalidInput(e) if e.starts_with("integration_exists:")));
}

#[test]
fn commands_are_part_of_api_1_and_a_newer_number_is_still_refused() {
    let f = Fixture::new();
    let directory = package_folder(&f.0, "team", "team", "1.0.0", "return function() return ui.text('ok') end");
    let manifest = directory.join("integration.json");
    let write = |value: Value| fs::write(&manifest, value.to_string()).unwrap();
    let refused = || match read_package_file(&manifest) {
        Err(rocket_git::Error::InvalidInput(e)) => e,
        other => panic!("{other:?}"),
    };
    let base = |permissions: Value, commands: Value| json!({"api":1,"id":"team","name":"Team","version":"1.0.0","permissions":permissions,"commands":commands});
    write(base(json!({"repository":true}), json!([{"id":"open","label":"Open board"},{"id":"find","label":"Find builds","target":"commit"},{"id":"branch","label":"Branch builds","target":"branch"}])));
    let package = read_package_file(&manifest).unwrap();
    assert_eq!(package.commands.len(), 3);
    assert_eq!(package.commands[0], Command { id: "open".into(), label: "Open board".into(), target: None });
    assert_eq!(package.commands[1].target, Some(Target::Commit));
    assert_eq!(package.commands[2].target, Some(Target::Branch));
    write(base(json!({}), json!([{"id":"find","label":"Find","target":"commit"}])));
    assert_eq!(refused(), "integration_manifest: Commit and branch commands need the repository permission.");
    write(base(json!({"repository":true}), json!([{"id":"a","label":"A","target":"file"}])));
    assert_eq!(refused(), "integration_manifest: A command's target must be commit or branch.");
    write(base(json!({}), json!([{"id":"a","label":"A"},{"id":"a","label":"B"}])));
    assert_eq!(refused(), "integration_manifest: Commands must have unique IDs, maximum 8.");
    let nine: Vec<Value> = (0..9).map(|i| json!({"id":format!("c{i}"),"label":"A"})).collect();
    write(base(json!({}), json!(nine)));
    assert_eq!(refused(), "integration_manifest: Commands must have unique IDs, maximum 8.");
    write(base(json!({}), json!([{"id":"a","label":"two\nlines"}])));
    assert_eq!(refused(), "integration_manifest: A command needs a one-line label of at most 64 bytes.");
    // Other unknown fields stay refused, and a package without commands is as it was.
    write(json!({"api":1,"id":"team","name":"Team","version":"1.0.0","permissions":{},"menus":[]}));
    assert!(refused().starts_with("integration_manifest: Unsupported API version or manifest fields."));
    write(json!({"api":1,"id":"team","name":"Team","version":"1.0.0","permissions":{}}));
    assert!(read_package_file(&manifest).unwrap().commands.is_empty());
    write(json!({"api":2,"id":"team","name":"Team","version":"1.0.0","permissions":{}}));
    assert!(refused().starts_with("integration_api: Team needs Lua API 2."));
}

#[test]
fn a_review_reads_the_approved_commands_and_compares_them_by_id() {
    let f = Fixture::new();
    let directory = package_folder(&f.0, "team", "team", "1.0.0", "return function() return ui.text('ok') end");
    let manifest = directory.join("integration.json");
    let write = |commands: Value| fs::write(&manifest, json!({"api":1,"id":"team","name":"Team","version":"1.0.0","permissions":{"repository":true},"commands":commands}).to_string()).unwrap();
    write(json!([{"id":"keep","label":"Keep"},{"id":"rename","label":"Old name"},{"id":"move","label":"Move","target":"branch"},{"id":"drop","label":"Drop"}]));
    let approved = read_package_file(&manifest).unwrap();
    let copy = ApprovedCopy::read(&approved.snapshot()).unwrap();
    assert_eq!(copy.commands, approved.commands);
    write(json!([{"id":"keep","label":"Keep"},{"id":"rename","label":"New name"},{"id":"move","label":"Move","target":"commit"},{"id":"add","label":"Add"}]));
    let newer = read_package_file(&manifest).unwrap();
    assert!(!newer.approved(&approved.snapshot()), "a changed command asks for a review");
    let lines = compare_commands(&copy.commands, &newer.commands);
    let summary: Vec<(&str, &CommandChange)> = lines.iter().map(|l| (l.command.id.as_str(), &l.change)).collect();
    assert_eq!(summary, [
        ("add", &CommandChange::New),
        ("rename", &CommandChange::Changed { label: Some("Old name".into()), target: None }),
        ("move", &CommandChange::Changed { label: None, target: Some(Some(Target::Branch)) }),
        ("drop", &CommandChange::Removed),
        ("keep", &CommandChange::Same),
    ]);
    assert!(compare_commands(&copy.commands, &copy.commands).iter().all(|l| l.change == CommandChange::Same));
}

#[test]
fn install_copies_the_reviewed_bytes_and_update_keeps_the_folder() {
    let f = Fixture::new();
    let root = f.0.join("integrations");
    let source = package_folder(&f.0.join("downloads"), "team", "team-dashboard", "2.3.0", "return function() return ui.text('v1') end");
    let reviewed = read_package_file(&source.join("integration.json")).unwrap();
    // The file changes after the review: what lands is what the review showed.
    fs::write(source.join("main.lua"), "return function() return ui.text('swapped') end").unwrap();
    let old = install_package(&root, &reviewed, None).unwrap();
    assert!(old.source.contains("v1"));
    let newer = package_folder(&f.0.join("downloads"), "team-2", "team-dashboard", "2.4.0", "return function() return ui.text('v2') end");
    let reviewed = read_package_file(&newer.join("integration.json")).unwrap();
    let landed = install_package(&root, &reviewed, Some(&old)).unwrap();
    assert_eq!(landed.directory, old.directory);
    assert_eq!(landed.version, "2.4.0");
    assert_eq!(fs::read_dir(f.0.join("integrations")).unwrap().count(), 1);
    assert!(fs::read_dir(&f.0).unwrap().all(|e| !e.unwrap().file_name().to_string_lossy().starts_with("integrations-")), "no folder left beside it");
    // An edit made after the review stops the update and keeps both copies intact.
    fs::write(landed.directory.join("main.lua"), "-- edited\nreturn function() return ui.text('v2') end").unwrap();
    let stale = install_package(&root, &reviewed, Some(&landed)).unwrap_err();
    assert!(matches!(stale, rocket_git::Error::InvalidInput(e) if e.starts_with("integration_changed:")));
    assert!(fs::read_to_string(landed.directory.join("main.lua")).unwrap().starts_with("-- edited"));
}

#[test]
fn a_chosen_file_says_what_is_wrong() {
    let f = Fixture::new();
    let code = |file: &std::path::Path| match read_package_file(file) {
        Err(rocket_git::Error::InvalidInput(e)) => e,
        other => panic!("{other:?}"),
    };
    let directory = package_folder(&f.0, "newer", "board", "1.0.0", "return function() return ui.text('ok') end");
    let manifest = directory.join("integration.json");
    fs::write(&manifest, json!({"api":2,"id":"board","name":"Board view","version":"1.0.0","commands":[]}).to_string()).unwrap();
    assert_eq!(code(&manifest), "integration_api: Board view needs Lua API 2. This Rocket has API 1. Update Rocket to install it.");
    fs::write(&manifest, "not json").unwrap();
    assert_eq!(code(&manifest), "integration_manifest: The manifest is not valid JSON.");
    fs::write(&manifest, json!({"api":1,"permissions":{}}).to_string()).unwrap();
    assert_eq!(code(&manifest), "integration_manifest: The manifest needs a name, an ID and a version.");
    assert!(code(&directory.join("main.lua")).starts_with("integration_file:"));
    fs::write(&manifest, json!({"api":1,"id":"a","name":"A","version":"1","permissions":{}}).to_string()).unwrap();
    fs::remove_file(directory.join("main.lua")).unwrap();
    assert_eq!(code(&manifest), "integration_files: main.lua must be beside integration.json.");
}

#[test]
fn remove_takes_only_a_folder_of_the_extensions_folder() {
    let f = Fixture::new();
    let root = f.0.join("integrations");
    let keep = package_folder(&f.0, "elsewhere", "other", "1.0.0", "return function() return ui.text('ok') end");
    assert!(remove_package(&root, &keep).is_err());
    assert!(keep.join("main.lua").exists());
    let inside = package_folder(&root, "team", "team", "1.0.0", "return function() return ui.text('ok') end");
    fs::write(inside.join("notes.txt"), "extra").unwrap();
    remove_package(&root, &inside).unwrap();
    assert!(!inside.exists());
    assert!(root.exists());
    assert!(fs::read_dir(&f.0).unwrap().all(|e| !e.unwrap().file_name().to_string_lossy().starts_with("integrations-")));
}

#[test]
fn duplicates_name_their_folders_so_one_copy_can_go() {
    let f = Fixture::new();
    let root = f.0.join("integrations");
    let source = "return function() return ui.text('ok') end";
    package_folder(&root, "one", "team", "1.0.0", source);
    let two = package_folder(&root, "two", "team", "1.0.1", source);
    let catalog = discover(&root, &rocket_git::CancellationToken::new()).unwrap();
    assert!(catalog.packages.is_empty());
    assert_eq!(catalog.issues[1].directory, two);
    assert_eq!(catalog.issues[1].duplicate, Some(("team".to_owned(), "1.0.1".to_owned())));
    remove_package(&root, &two).unwrap();
    let catalog = discover(&root, &rocket_git::CancellationToken::new()).unwrap();
    assert_eq!(catalog.packages.len(), 1);
}

#[test]
fn one_review_covers_the_connections_and_the_switch_reads_saved_state() {
    let f = Fixture::new();
    let root = f.0.join("integrations");
    let directory = package_folder(&root, "team", "team", "1.0.0", "return function() return ui.text('ok') end");
    let package = Package::load(&directory).unwrap();
    let mut store = profiles::empty();
    assert!(!profiles::extension_enabled(&store, "team"));
    // Nothing is set up: the selected connection carries the approval, with no server yet.
    profiles::approve(&mut store, &package);
    assert!(profiles::extension_enabled(&store, "team"));
    assert!(profiles::reviewed(&store, &package));
    assert!(!profiles::needs_review(&store, &package));
    {
        let list = store["profiles"]["team"]["connections"].as_object_mut().unwrap();
        let mut work = profiles::default_connection("Work");
        work["origin"] = json!("https://work.example.invalid");
        work["writes"] = json!(true);
        list.insert("work".into(), work);
        let mut idle = profiles::default_connection("Not set up");
        idle["revision"] = json!("idle");
        list.insert("idle".into(), idle);
    }
    // Changed files need one review, which reaches every connection that is set up.
    fs::write(directory.join("main.lua"), "return function() return ui.text('new') end").unwrap();
    let changed = Package::load(&directory).unwrap();
    assert!(profiles::needs_review(&store, &changed));
    assert!(!profiles::reviewed(&store, &changed));
    profiles::approve(&mut store, &changed);
    assert!(!profiles::needs_review(&store, &changed));
    for key in ["default", "work"] {
        assert!(changed.approved(&profiles::connection(&store, "team", key)["approved"]));
    }
    let idle = profiles::connection(&store, "team", "idle");
    assert_eq!(idle["enabled"], false);
    assert!(idle.get("approved").is_none());
    assert_eq!(profiles::connection(&store, "team", "work")["writes"], true, "access stays per connection");
    // Off keeps the approval, so on needs no second review.
    profiles::disable(&mut store, "team");
    assert!(!profiles::extension_enabled(&store, "team"));
    assert!(profiles::reviewed(&store, &changed));
    assert!(profiles::validate(&store).is_ok());
    assert_eq!(profiles::saved_connections(&store, "team"), ["Work"]);
    store["profiles"]["team"]["connections"]["work"]["uncertain"] = json!(true);
    assert_eq!(profiles::unchecked_write(&store, "team").as_deref(), Some("Work"));
    profiles::remove(&mut store, "team");
    assert!(store["profiles"]["team"].is_null());
    assert!(profiles::validate(&store).is_ok());
}

#[test]
fn compare_shows_changes_with_context_and_counts_them() {
    let old = (1..=20).map(|n| format!("line {n}")).collect::<Vec<_>>().join("\n");
    let new = old.replace("line 5\n", "changed 5\nadded\n").replace("\nline 18", "");
    let change = compare_source(&old, &new);
    assert_eq!((change.added, change.removed, change.changes), (2, 2, 2));
    let kinds: Vec<LineKind> = change.lines.iter().map(|l| l.kind).collect();
    assert_eq!(kinds.iter().filter(|k| **k == LineKind::Gap).count(), 1);
    let added = change.lines.iter().find(|l| l.kind == LineKind::Added).unwrap();
    assert_eq!((added.number, added.text.as_str()), (5, "changed 5"));
    let removed = change.lines.iter().rfind(|l| l.kind == LineKind::Removed).unwrap();
    assert_eq!((removed.number, removed.text.as_str()), (18, "line 18"));
    assert_eq!(compare_source(&old, &old), SourceChange::default());
    // Too large for the table: still counted, as a whole replacement of the middle.
    let a = (0..3000).map(|n| format!("a{n}")).collect::<Vec<_>>().join("\n");
    let b = (0..3000).map(|n| format!("b{n}")).collect::<Vec<_>>().join("\n");
    let change = compare_source(&a, &b);
    assert_eq!((change.added, change.removed, change.changes), (3000, 3000, 1));
}

#[test]
fn approved_copy_reads_what_the_review_compares_against() {
    let f = Fixture::new();
    let directory = package_folder(&f.0, "team", "team", "1.2.3", "return function() return ui.text('ok') end");
    let package = Package::load(&directory).unwrap();
    let copy = ApprovedCopy::read(&package.snapshot()).unwrap();
    assert_eq!(copy.version, "1.2.3");
    assert!(copy.network && copy.repository && !copy.writes);
    assert_eq!(copy.source, package.source);
    assert!(ApprovedCopy::read(&json!({})).is_none());
}

/// The bundled scripts, run in the real interpreter against an answering host.
#[cfg(feature = "runtime")]
mod scripts {
    use super::*;
    const JIRA: &str = include_str!("../examples/jira/main.lua");
    const PLAYGROUND: &str = include_str!("../examples/playground/main.lua");

    /// One callback as the host runs it: each read is answered and the script starts again.
    fn drive(source: &str, context: &Value, answer: &dyn Fn(&str) -> Value) -> (Value, Vec<String>) {
        let mut cache = serde_json::Map::new();
        let mut reads = vec![];
        loop {
            let result = runtime::run(json!({"api":1,"source":source,"context":context,"responses":cache}));
            if result["kind"] != "request" {
                return (result, reads);
            }
            let request = Request::parse(&result["request"]).unwrap();
            if request.writes() {
                return (result, reads);
            }
            reads.push(request.path.clone());
            cache.insert(request.key(), json!({"status":200,"json":answer(&request.path)}));
        }
    }
    fn jira_service(path: &str) -> Value {
        if path.starts_with("/rest/api/3/search/jql") {
            json!({"isLast":true,"issues":[
                {"key":"ROCKET-7","fields":{"summary":"Keep the login redirect","status":{"name":"In progress","statusCategory":{"key":"indeterminate"}}}},
                {"key":"ROCKET-3","fields":{"summary":"Sign-in loops","status":{"name":"Done","statusCategory":{"key":"done"}}}}]})
        } else if path.contains("/createmeta/") && path.contains("/issuetypes/20") {
            json!({"fields":[{"fieldId":"summary","required":true},{"fieldId":"customfield_1","name":"Team","required":true}]})
        } else if path.contains("/createmeta/") && path.contains("/issuetypes/") {
            json!({"fields":[{"fieldId":"summary","required":true}]})
        } else if path.contains("/createmeta/") {
            json!({"issueTypes":[{"id":"10","name":"Task"},{"id":"20","name":"Bug"},{"id":"30","name":"Sub-task","subtask":true}]})
        } else if path.ends_with("/editmeta") {
            json!({"fields":{"summary":{"operations":["set"]},"description":{"operations":["set"]}}})
        } else if path.contains("/transitions") {
            json!({"transitions":[{"id":"31","name":"In review"},{"id":"41","name":"Done"},{"id":"51","name":"Done"},{"id":"61","name":"Blocked","fields":{"reason":{"required":true}}}]})
        } else if path.contains("/comment") {
            json!({"total":2,"comments":[{"author":{"displayName":"Ada Park"},"created":"2026-10-29T11:20:00.000+0000","body":{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"Reproduced"}]}]}}]})
        } else if path.starts_with("/rest/api/3/issue/ROCKET-") {
            json!({"fields":{"summary":"Keep the login redirect","updated":"2026-10-29T14:02:00.000+0000","status":{"name":"In progress","statusCategory":{"key":"indeterminate"}},
                "assignee":{"displayName":"Ada Park"},"issuetype":{"name":"Task"},"description":{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"It loops."}]}]}}})
        } else {
            panic!("unexpected read {path}");
        }
    }
    fn context(event: &str, state: &Value) -> Value {
        json!({"event":event,"settings":{"project":"ROCKET"},"values":{},"state":state})
    }
    fn view(result: &Value) -> Node {
        assert_eq!(result["kind"], "view", "{result}");
        Node::parse(&result["view"]).unwrap()
    }
    fn words(n: &Node, out: &mut Vec<String>) {
        out.push(n.label.clone());
        out.extend(n.options.iter().cloned());
        for row in &n.rows {
            out.extend(row.iter().map(|c| c.text.clone()));
        }
        for c in &n.children {
            words(c, out);
        }
    }
    fn shows(n: &Node, text: &str) -> bool {
        let mut all = vec![];
        words(n, &mut all);
        all.iter().any(|w| w.contains(text))
    }
    fn find<'a>(n: &'a Node, kind: &str) -> Option<&'a Node> {
        if n.kind == kind {
            return Some(n);
        }
        n.children.iter().find_map(|c| find(c, kind))
    }
    fn write(result: &Value) -> Request {
        assert_eq!(result["kind"], "request", "{result}");
        Request::parse(&result["request"]).unwrap()
    }
    /// The first panel, then the issue opened from its list.
    fn opened() -> Value {
        let (first, _) = drive(JIRA, &context("open", &json!({})), &jira_service);
        let mut click = context("issues", &first["state"]);
        click["row"] = json!("ROCKET-7");
        let (result, reads) = drive(JIRA, &click, &jira_service);
        assert!(reads.len() <= 8, "{reads:?}");
        result["state"].clone()
    }

    #[test]
    fn jira_opens_on_two_panes_and_a_row_opens_its_issue() {
        let (first, reads) = drive(JIRA, &context("open", &json!({})), &jira_service);
        assert_eq!(reads.len(), 1);
        assert!(reads[0].contains("currentUser"), "{reads:?}");
        let page = view(&first);
        assert!(page.has_split());
        let table = find(&page, "table").unwrap();
        assert!(table.clickable());
        assert_eq!(table.keys, ["ROCKET-7", "ROCKET-3"]);
        assert_eq!(table.rows[0][2].tone.as_deref(), Some("accent"));
        assert_eq!(table.rows[1][2].tone.as_deref(), Some("success"));
        assert_eq!(find(&page, "split").unwrap().focus, 0, "nothing open yet");
        assert!(shows(&page, "Choose an issue from the list"));
        assert_eq!(find(&page, "segments").unwrap().options.len(), 3, "This branch needs repository details");

        let mut click = context("issues", &first["state"]);
        click["row"] = json!("ROCKET-7");
        let (second, reads) = drive(JIRA, &click, &jira_service);
        assert_eq!(reads.len(), 4, "the list and three reads of the issue: {reads:?}");
        let page = view(&second);
        assert_eq!(find(&page, "table").unwrap().selected.as_deref(), Some("ROCKET-7"));
        assert_eq!(find(&page, "split").unwrap().focus, 1);
        assert!(shows(&page, "It loops.") && shows(&page, "Reproduced") && shows(&page, "Comments · 2"));
        assert!(shows(&page, "Task · Ada Park · Updated 2026-10-29 14:02"));
        let moves = &find(&page, "select").unwrap().options;
        assert_eq!(moves, &["In review", "Done", "Done · 51"], "no raw ID unless two moves share a name");
        assert!(shows(&page, "1 moves need extra fields"));
    }

    #[test]
    fn jira_search_takes_a_key_words_or_jql() {
        let (first, _) = drive(JIRA, &context("open", &json!({})), &jira_service);
        let search = |typed: &str| {
            let mut c = context("search", &first["state"]);
            c["values"]["search_0"] = json!(typed);
            drive(JIRA, &c, &jira_service)
        };
        let (_, reads) = search("rocket-7");
        assert!(reads.iter().any(|r| r.starts_with("/rest/api/3/issue/ROCKET-7?")), "{reads:?}");
        let (_, reads) = search("login \"loop\"");
        assert!(reads[0].contains(&rocket_http_encoded("text ~ \"login \\\"loop\\\"\"")), "{reads:?}");
        let (_, reads) = search("assignee = me ORDER BY created");
        assert!(reads[0].contains(&rocket_http_encoded("assignee = me ORDER BY created")), "{reads:?}");
    }
    /// The form `rocket.url_encode` gives a query.
    fn rocket_http_encoded(text: &str) -> String {
        let (result, _) = drive(&format!("return function() return ui.text(rocket.url_encode({text:?})) end"), &json!({"event":"open","settings":{},"values":{},"state":{}}), &jira_service);
        result["view"]["text"].as_str().unwrap().to_owned()
    }

    #[test]
    fn jira_create_asks_for_the_type_and_checks_required_fields_itself() {
        let (first, _) = drive(JIRA, &context("open", &json!({})), &jira_service);
        let (form, _) = drive(JIRA, &context("create_open", &first["state"]), &jira_service);
        let page = view(&form);
        assert_eq!(find(&page, "select").unwrap().options, ["Task", "Bug"], "no sub-tasks");
        assert_eq!(find(&page, "split").unwrap().focus, 1);
        let mut create = context("create", &form["state"]);
        create["values"] = json!({"issue_type":"Bug","summary":"New"});
        let (blocked, _) = drive(JIRA, &create, &jira_service);
        assert!(shows(&view(&blocked), "Bug needs Team, which this form can’t fill. Create it in Jira."));
        create["values"] = json!({"issue_type":"Task","summary":"New","description":"Body"});
        let (sent, _) = drive(JIRA, &create, &jira_service);
        let request = write(&sent);
        assert_eq!(request.path, "/rest/api/3/issue");
        assert_eq!(request.body.as_ref().unwrap()["fields"]["issuetype"]["id"], "10");
        create["values"] = json!({"issue_type":"Task","summary":""});
        assert!(shows(&view(&drive(JIRA, &create, &jira_service).0), "Enter a summary of 1–255 characters."));
        let (cancelled, _) = drive(JIRA, &context("pane_cancel", &form["state"]), &jira_service);
        assert!(shows(&view(&cancelled), "Choose an issue from the list"));
    }

    #[test]
    fn jira_edits_moves_and_comments_stop_at_a_checked_write() {
        let state = opened();
        let (edit, reads) = drive(JIRA, &context("edit_open", &state), &jira_service);
        assert!(reads.iter().any(|r| r.ends_with("/editmeta")));
        assert!(reads.iter().any(|r| r.contains("fields=summary")), "from the issue as it is now: {reads:?}");
        assert!(shows(&view(&edit), "Rich text stays unless you replace it."));
        let mut save = context("edit_issue", &edit["state"]);
        save["values"] = json!({"edit_summary_ROCKET-7":"Renamed"});
        let request = write(&drive(JIRA, &save, &jira_service).0);
        assert_eq!(request.method, "PUT");
        assert_eq!(request.body.as_ref().unwrap()["fields"]["summary"], "Renamed");
        assert_eq!(request.check.as_ref().unwrap().expected, "2026-10-29T14:02:00.000+0000");

        let mut transition = context("transition_issue", &state);
        transition["values"] = json!({"transition_ROCKET-7":"Done · 51"});
        let request = write(&drive(JIRA, &transition, &jira_service).0);
        assert_eq!(request.path, "/rest/api/3/issue/ROCKET-7/transitions");
        assert_eq!(request.body.as_ref().unwrap()["transition"]["id"], "51");

        let mut comment = context("comment", &state);
        comment["values"] = json!({"comment_body_ROCKET-7":"Thanks"});
        assert_eq!(write(&drive(JIRA, &comment, &jira_service).0).path, "/rest/api/3/issue/ROCKET-7/comment");
    }

    fn ids(result: &Value) -> Vec<String> {
        let mut ids = vec![];
        view(result).control_ids(&mut ids);
        ids
    }

    #[test]
    fn jira_refresh_keeps_a_form_until_its_request_went_through() {
        let list = drive(JIRA, &context("open", &json!({})), &jira_service).0;
        let form = drive(JIRA, &context("create_open", &list["state"]), &jira_service).0;
        assert!(ids(&form).iter().any(|id| id == "summary"));

        let refreshed = drive(JIRA, &context("open", &form["state"]), &jira_service).0;
        assert!(ids(&refreshed).iter().any(|id| id == "summary"), "Refresh leaves the form");

        let mut done = context("open", &form["state"]);
        done["confirmed"] = json!(true);
        let closed = drive(JIRA, &done, &jira_service).0;
        assert!(!ids(&closed).iter().any(|id| id == "summary"), "the form is done");
    }

    #[test]
    fn jira_comments_stay_with_their_issue_and_leave_after_sending() {
        let state = opened();
        let issue = drive(JIRA, &context("open", &state), &jira_service).0;
        assert!(ids(&issue).iter().any(|id| id == "comment_body_ROCKET-7"));

        // Text typed for another issue is not this issue's comment.
        let mut comment = context("comment", &state);
        comment["values"] = json!({"comment_body_ROCKET-6":"For the other one"});
        let stopped = drive(JIRA, &comment, &jira_service).0;
        assert!(ids(&stopped).iter().any(|id| id == "comment_body_ROCKET-7"), "asks for a comment instead");

        let mut sent = context("open", &state);
        sent["confirmed"] = json!(true);
        let after = drive(JIRA, &sent, &jira_service).0;
        assert!(ids(&after).iter().any(|id| id == "comment_body_ROCKET-7_1"), "a new, empty box");
    }

    #[test]
    fn jira_refresh_reads_the_checked_out_branch_again() {
        let on = |event: &str, branch: &str, state: &Value| {
            let mut c = context(event, state);
            c["repository"] = json!({"branch":branch});
            c["values"] = json!({"scope":"This branch"});
            drive(JIRA, &c, &jira_service)
        };
        let first = on("open", "feature/ROCKET-7-login", &json!({})).0;
        let (chosen, reads) = on("scope", "feature/ROCKET-7-login", &first["state"]);
        assert!(reads.iter().any(|r| r.contains("ROCKET-7")), "{reads:?}");
        let (_, reads) = on("open", "feature/ROCKET-9-next", &chosen["state"]);
        assert!(reads.iter().any(|r| r.contains("ROCKET-9")), "{reads:?}");
    }

    #[test]
    fn jira_commands_land_on_the_page() {
        let command = |id: &str, target: Value, state: &Value| {
            let mut c = context("command", state);
            c["command"] = json!(id);
            if !target.is_null() {
                c["target"] = target;
            }
            drive(JIRA, &c, &jira_service)
        };
        let commit = json!({"kind":"commit","commit":"4f1c2a9e0b1d"});
        let (found, reads) = command("find-commit", commit.clone(), &json!({}));
        assert!(reads[0].contains("4f1c2a9"), "{reads:?}");
        assert!(!reads[0].contains("4f1c2a9e"), "the short ID: {reads:?}");
        let page = view(&found);
        assert!(shows(&page, "Issues · mention 4f1c2a9"));
        assert!(find(&page, "segments").unwrap().value == "", "no scope is chosen");
        assert_eq!(find(&page, "input").unwrap().value, "text ~ \"4f1c2a9\"", "the search shows what ran");

        let (form, _) = command("create-issue", Value::Null, &json!({}));
        assert!(shows(&view(&form), "New issue · ROCKET"));

        let (open, reads) = command("open-branch", json!({"kind":"branch","branch":"feature/ROCKET-7-login"}), &json!({}));
        assert!(reads.iter().any(|r| r.starts_with("/rest/api/3/issue/ROCKET-7?")), "{reads:?}");
        assert!(shows(&view(&open), "Comments · 2"));
        let (none, _) = command("open-branch", json!({"kind":"branch","branch":"rocket-ui-test/lua"}), &json!({}));
        assert!(shows(&view(&none), "rocket-ui-test/lua has no issue key in its name."));

        let (nothing_open, _) = command("comment-commit", commit.clone(), &json!({}));
        assert!(shows(&view(&nothing_open), "Open an issue first."));
        let request = write(&command("comment-commit", commit, &opened()).0);
        assert_eq!(request.path, "/rest/api/3/issue/ROCKET-7/comment");
        assert_eq!(request.body.as_ref().unwrap()["body"]["content"][0]["content"][0]["text"], "Commit 4f1c2a9");
    }

    #[test]
    fn jira_this_branch_needs_repository_details_and_a_key_in_the_name() {
        let mut c = context("open", &json!({}));
        c["repository"] = json!({"branch":"feature/ROCKET-7-login"});
        let (first, _) = drive(JIRA, &c, &jira_service);
        assert!(find(&view(&first), "segments").unwrap().options.contains(&"This branch".to_owned()));
        let mut scope = context("scope", &first["state"]);
        scope["repository"] = json!({"branch":"feature/ROCKET-7-login"});
        scope["values"] = json!({"scope":"This branch"});
        let (_, reads) = drive(JIRA, &scope, &jira_service);
        assert!(reads[0].contains("ROCKET-7"), "{reads:?}");
        scope["repository"] = json!({"branch":"main"});
        let (none, reads) = drive(JIRA, &scope, &jira_service);
        assert!(reads.is_empty());
        assert!(shows(&view(&none), "main has no issue key in its name."));
        assert!(shows(&view(&none), "No issues match."));
    }

    #[test]
    fn playground_shows_every_section_and_what_a_command_receives() {
        let service = |path: &str| panic!("PlayGround is offline: {path}");
        let (first, reads) = drive(PLAYGROUND, &json!({"event":"open","settings":{},"values":{},"state":{}}), &service);
        assert!(reads.is_empty());
        let page = view(&first);
        assert!(shows(&page, "Hello, Rocket · updated 0 times"));
        assert!(find(&page, "code").is_some());
        for section in ["Controls", "Text", "Data", "Layout", "Commands"] {
            let (shown, _) = drive(PLAYGROUND, &json!({"event":"section","settings":{},"values":{"section":section},"state":first["state"]}), &service);
            let page = view(&shown);
            assert_eq!(find(&page, "segments").unwrap().value, section);
            assert!(page.has_split(), "{section}");
        }
        let (once, _) = drive(PLAYGROUND, &json!({"event":"increment","settings":{},"values":{"name":"Ada"},"state":first["state"]}), &service);
        assert!(shows(&view(&once), "Hello, Ada · updated once"), "no \"1 times\"");
        let (row, _) = drive(PLAYGROUND, &json!({"event":"rows","row":"marks","settings":{},"values":{},"state":{"section":"Data"}}), &service);
        assert_eq!(find(&view(&row), "table").unwrap().selected.as_deref(), Some("marks"));
        let (command, _) = drive(PLAYGROUND, &json!({"event":"command","command":"show-target","target":{"kind":"commit","commit":"4f1c2a9"},"settings":{},"values":{},"state":{}}), &service);
        let page = view(&command);
        assert_eq!(find(&page, "segments").unwrap().value, "Commands");
        assert!(shows(&page, "ctx.target.commit") && shows(&page, "4f1c2a9") && shows(&page, "show-target"));
        let (hello, _) = drive(PLAYGROUND, &json!({"event":"command","command":"say-hello","settings":{},"values":{},"state":{}}), &service);
        assert!(shows(&view(&hello), "say-hello") && shows(&view(&hello), "none"));
    }
}
