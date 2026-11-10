use super::*;
use crate::test_common as common;
use std::{fs, path::Path};

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
            let mut state=State::default();
            state.form=Some(Form{record:p.record.clone(),destination:f.root.join("changed after review").to_string_lossy().into_owned(),setup_enabled:false,setup:Command::default(),mode:Mode::Create,plan:Some(p.clone()),task:None,error:None});
            assert!(state.confirm().is_err(),"changing the reviewed destination requires a new preview");
            assert!(state.sessions.is_empty());
            state.form.as_mut().unwrap().destination=p.record.destination.to_string_lossy().into_owned();
            state.confirm().unwrap();
            assert_eq!(state.sessions[0].phase,Phase::Saving);
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
