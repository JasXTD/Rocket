use super::*;
use crate::test_common as common;
use std::{fs, path::Path};

#[test]
fn selected_worktree_changes_are_real_scoped_and_cancelable() {
    common::isolated(
        "ROCKET_AGENT_CHANGES",
        "agents::tests::selected_worktree_changes_are_real_scoped_and_cancelable",
        || {
            use changes::Area;
            let f = common::Fixture::new();
            fs::write(f.repository.join("資料.txt"), "base\n").unwrap();
            f.commit("base");
            let r = record(&f.repository, &std::env::current_exe().unwrap());
            f.run(&[
                "worktree",
                "add",
                "-b",
                &r.branch,
                r.destination.to_str().unwrap(),
            ]);
            fs::write(f.repository.join("source-only.txt"), "never in agent diff").unwrap();
            fs::write(r.destination.join("資料.txt"), "staged\n").unwrap();
            assert!(
                f.command(&r.destination, &["add", "--", "資料.txt"])
                    .output()
                    .unwrap()
                    .status
                    .success()
            );
            fs::write(r.destination.join("資料.txt"), "staged\nunstaged\n").unwrap();
            fs::write(r.destination.join("新規.txt"), "untracked text\n").unwrap();
            let backend = Backend::discover(&f.root.join("logs")).unwrap();
            let c = rocket_git::CancellationToken::new();
            let snapshot = changes::read(&backend, &r, &c, None, Arc::new(|| {})).unwrap();
            assert_eq!(snapshot.files.len(), 3);
            for (area, needle, absent) in [
                (Area::Staged, "+staged", "+unstaged"),
                (Area::Unstaged, "+unstaged", "source-only"),
                (Area::Untracked, "untracked text", "source-only"),
            ] {
                let file = snapshot.files.iter().find(|f| f.area == area).unwrap();
                let patch = changes::read_patch(&backend, &r, file, &c).unwrap();
                if area == Area::Untracked {
                    assert!(patch.review.is_none());
                    assert!(patch.notice.is_none());
                }
                let text = patch
                    .files
                    .iter()
                    .flat_map(|f| f.lines.iter().map(|l| l.text.as_str()))
                    .collect::<Vec<_>>()
                    .join("\n");
                assert!(text.contains(needle), "{text}");
                assert!(!text.contains(absent), "{text}");
            }
            let cancelled = rocket_git::CancellationToken::new();
            cancelled.cancel();
            assert!(matches!(
                changes::read(&backend, &r, &cancelled, None, Arc::new(|| {})),
                Err(rocket_git::Error::Cancelled)
            ));
            assert!(matches!(
                changes::read_patch(&backend, &r, &snapshot.files[0], &cancelled),
                Err(rocket_git::Error::Cancelled)
            ));
            assert!(
                f.command(&r.destination, &["checkout", "-b", "agents/external"])
                    .output()
                    .unwrap()
                    .status
                    .success()
            );
            assert!(changes::read(&backend, &r, &c, None, Arc::new(|| {})).is_err());
            assert!(changes::read_patch(&backend, &r, &snapshot.files[0], &c).is_err());
            assert_eq!(
                fs::read_to_string(r.destination.join("資料.txt")).unwrap(),
                "staged\nunstaged\n"
            );
            assert_eq!(
                fs::read_to_string(f.repository.join("source-only.txt")).unwrap(),
                "never in agent diff"
            );
        },
    );
}

#[test]
fn changing_selected_session_drops_old_changes_and_state() {
    let a = record(
        &std::env::temp_dir().join("a"),
        &std::env::current_exe().unwrap(),
    );
    let mut b = a.clone();
    b.id = 2;
    b.destination = std::env::temp_dir().join("b");
    let mut s = changes::State::default();
    s.select_owner(&a);
    s.patch = Some(crate::diff::Patch::default());
    s.error = Some("old error".into());
    s.selected = Some(1);
    s.select_owner(&b);
    assert_eq!(s.owner, Some(b));
    assert!(s.patch.is_none());
    assert!(s.error.is_none());
    assert!(s.selected.is_none());
    assert!(!s.busy());
}

#[test]
fn activity_is_bounded_and_does_not_repeat_unchanged_phases() {
    let mut s = Session::restored(record(
        &std::env::temp_dir().join("source"),
        &std::env::current_exe().unwrap(),
    ));
    for _ in 0..40 {
        s.phase = Phase::Running;
        s.observe_phase();
        s.observe_phase();
        s.phase = Phase::Stopped;
        s.observe_phase();
    }
    assert_eq!(s.activity.len(), 32);
    assert!(
        s.activity
            .iter()
            .zip(s.activity.iter().skip(1))
            .all(|(a, b)| a.phase != b.phase)
    );
    assert!(s.terminal.is_none());
}

fn record(source: &Path, program: &Path) -> Record {
    Record {
        id: 1,
        name: "資料 task".into(),
        source: source.to_owned(),
        destination: source.parent().unwrap().join("agent 資料"),
        branch: "agents/unicode".into(),
        base: "HEAD".into(),
        provider: Provider::Custom,
        command: Command {
            program: program.to_string_lossy().into_owned(),
            arguments: "--version".into(),
        },
        setup: None,
    }
}
#[test]
fn definitions_roundtrip_without_relaunch_and_refuse_malformed_metadata() {
    let r = record(
        &std::env::temp_dir().join("source"),
        &std::env::current_exe().unwrap(),
    );
    let bytes = model::encode(std::slice::from_ref(&r)).unwrap();
    let mut s = State::default();
    s.load(model::decode(&bytes).unwrap());
    assert_eq!(s.records(), vec![r.clone()]);
    assert!(!s.active());
    assert!(!s.working());
    assert_eq!(s.sessions[0].phase, Phase::Restored);
    assert!(model::encode(&[r.clone(), r.clone()]).is_err());
    let mut bad = r.clone();
    bad.command.arguments = "first\n\u{202e}hidden".into();
    assert!(model::encode(&[bad]).is_err());
    let mut bad = r;
    bad.command.program = "relative.exe".into();
    assert!(model::encode(&[bad]).is_err());
    assert!(model::decode(br#"[{"id":1}]"#).is_err());
}
#[test]
fn cancelled_saved_session_never_starts_a_process() {
    let r = record(
        &std::env::temp_dir().join("source"),
        &std::env::current_exe().unwrap(),
    );
    let mut s = Session::restored(r);
    s.pending = true;
    s.phase = Phase::Saving;
    s.stop();
    assert!(!s.active());
    assert_eq!(s.phase, Phase::Stopping);
    assert!(s.terminal.is_none());
}
#[test]
fn agent_preview_protects_existing_destination_and_stale_worktree() {
    common::isolated(
        "ROCKET_AGENT_CASE",
        "agents::tests::agent_preview_protects_existing_destination_and_stale_worktree",
        || {
            let f = common::Fixture::new();
            fs::write(f.repository.join("資料.txt"), "base").unwrap();
            f.commit("base");
            let backend = Backend::discover(&f.root.join("logs")).unwrap();
            let c = rocket_git::CancellationToken::new();
            let r = record(&f.repository, &std::env::current_exe().unwrap());
            fs::create_dir(&r.destination).unwrap();
            fs::write(r.destination.join("keep"), "keep").unwrap();
            assert!(runtime::preview(&backend, r.clone(), Mode::Create, &c).is_err());
            let mut next = r.clone();
            next.destination = f.root.join("second 資料");
            let p = Arc::new(runtime::preview(&backend, next, Mode::Create, &c).unwrap());
            let mut state = State::default();
            state.form = Some(Form {
                record: p.record.clone(),
                destination: f
                    .root
                    .join("changed after review")
                    .to_string_lossy()
                    .into_owned(),
                setup_enabled: false,
                setup: Command::default(),
                mode: Mode::Create,
                plan: Some(p.clone()),
                task: None,
                error: None,
            });
            assert!(
                state.confirm().is_err(),
                "changing the reviewed destination requires a new preview"
            );
            assert!(state.sessions.is_empty());
            state.form.as_mut().unwrap().destination =
                p.record.destination.to_string_lossy().into_owned();
            state.confirm().unwrap();
            assert_eq!(state.sessions[0].phase, Phase::Saving);
            assert_eq!(state.sessions[0].activity.len(), 1);
            assert_eq!(state.sessions[0].activity[0].phase, Phase::Saving);
            state.stop_all();
            assert!(!state.active());
            f.run(&["branch", "agents/unicode"]);
            let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(|| {});
            assert!(runtime::start(&backend, &p, false, wake, &c).is_err());
            assert!(!p.record.destination.exists());
            assert_eq!(
                fs::read_to_string(r.destination.join("keep")).unwrap(),
                "keep"
            );
        },
    );
}
