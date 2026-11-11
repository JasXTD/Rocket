use super::*;
use crate::test_common as common;
use std::{fs, path::Path};

#[test]
fn activity_deduplicates_attention_and_rejects_other_sessions_and_late_turns() {
    use events::Activity as A;
    let mut s=Session::restored(record(&std::env::temp_dir().join("source"),&std::env::current_exe().unwrap()));
    s.phase=Phase::Running;
    let event=|kind,at,turn:&str,new_turn| events::Event{id:"11111111-2222-3333-4444-555555555555".into(),kind,at,turn:Some(turn.into()),new_turn};
    assert!(!s.observe_event(event(A::Working,1,"one",true)));
    assert!(s.observe_event(event(A::Waiting,2,"one",false)));
    assert!(!s.observe_event(event(A::Waiting,3,"one",false)));
    assert_eq!(s.notifications,1);s.attention=false;
    assert!(!s.observe_event(event(A::Waiting,4,"one",false)));assert!(!s.attention);
    assert!(s.observe_event(event(A::Complete,5,"one",false)));assert_eq!(s.notifications,2);
    assert!(!s.observe_event(event(A::Working,6,"two",true)));assert!(!s.attention);
    assert!(!s.observe_event(event(A::Complete,7,"one",false)));assert_eq!(s.activity_state,A::Working);
    let mut other=event(A::Waiting,8,"two",false);other.id="aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee".into();
    assert!(!s.observe_event(other));assert_eq!(s.activity_state,A::Working);
    assert!(s.observe_event(event(A::Complete,9,"two",false)));
    assert!(!s.observe_event(event(A::Working,8,"two",true)));assert_eq!(s.activity_state,A::Complete);
    s.phase=Phase::Stopping;assert!(!s.observe_event(event(A::Waiting,10,"two",false)));
}

#[test]
fn resume_revalidates_reviewed_executable_arguments_and_worktree_without_setup() {
    common::isolated("ROCKET_AGENT_RESUME","agents::tests::resume_revalidates_reviewed_executable_arguments_and_worktree_without_setup",||{
        let f=common::Fixture::new();fs::write(f.repository.join("base"),"base\n").unwrap();f.commit("base");
        let mut r=record(&f.repository,&std::env::current_exe().unwrap());r.provider=Provider::Codex;r.tracking=true;r.command.arguments.clear();
        f.run(&["worktree","add","-b",&r.branch,r.destination.to_str().unwrap()]);
        let b=Backend::discover(&f.root.join("logs")).unwrap();let c=rocket_git::CancellationToken::new();
        let p=runtime::preview(&b,r.clone(),Mode::Restart,&c).unwrap();r=p.record.clone();
        r.approval=Some(runtime::approval(&r,&p.program.as_ref().unwrap().digest,&c).unwrap());
        r.conversation=Some("11111111-2222-3333-4444-555555555555".into());
        r.setup=Some(Command{program:f.root.join("unavailable-setup.exe").to_string_lossy().into_owned(),arguments:"--never-run-on-resume".into()});
        let encoded=model::encode(&[r.clone()]).unwrap();let restored=model::decode(&encoded).unwrap();
        let resumed=runtime::preview(&b,restored[0].clone(),Mode::Resume,&c).unwrap();
        assert!(resumed.setup.is_none());assert_eq!(resumed.record.setup,r.setup);
        let mut state=State::default();state.load(vec![r.clone()]);state.existing_form(Mode::Resume);
        assert_eq!(state.form.as_ref().unwrap().proposed().setup,r.setup);
        let mut changed=r.clone();changed.command.arguments="--model\nchanged-model".into();
        assert!(runtime::preview(&b,changed,Mode::Resume,&c).err().unwrap().to_string().contains("changed"));
        let mut changed=r.clone();changed.approval=Some("0".repeat(64));assert!(runtime::preview(&b,changed,Mode::Resume,&c).is_err());
        let cancelled=rocket_git::CancellationToken::new();cancelled.cancel();assert!(matches!(runtime::preview(&b,r.clone(),Mode::Resume,&cancelled),Err(rocket_git::Error::Cancelled)));
        f.command(&r.destination,&["switch","--detach"]).output().unwrap();assert!(runtime::preview(&b,r,Mode::Resume,&c).is_err());
    });
}

#[test]
fn integration_preserves_local_changes_rechecks_branch_tips_and_cancels() {
    common::isolated("ROCKET_AGENT_INTEGRATE","agents::tests::integration_preserves_local_changes_rechecks_branch_tips_and_cancels",||{
        let f=common::Fixture::new();fs::write(f.repository.join("資料.txt"),"base\n").unwrap();f.commit("base");
        let r=record(&f.repository,&std::env::current_exe().unwrap());
        f.run(&["worktree","add","-b",&r.branch,r.destination.to_str().unwrap()]);
        let b=Backend::discover(&f.root.join("logs")).unwrap();let c=rocket_git::CancellationToken::new();
        fs::write(r.destination.join("資料.txt"),"agent\n").unwrap();assert!(integration::read(&b,&r,&c).is_err());
        for args in [vec!["add","."],vec!["commit","-m","agent change"]]{assert!(f.command(&r.destination,&args).output().unwrap().status.success());}
        fs::write(f.repository.join("keep.txt"),"local\n").unwrap();assert!(integration::read(&b,&r,&c).is_err());assert_eq!(fs::read_to_string(f.repository.join("keep.txt")).unwrap(),"local\n");fs::remove_file(f.repository.join("keep.txt")).unwrap();
        let review=integration::read(&b,&r,&c).unwrap();assert_eq!(review.merge.incoming(),1);assert_eq!(review.comparison.files[0].path.as_bytes(),"資料.txt".as_bytes());
        assert!(f.command(&r.destination,&["commit","--allow-empty","-m","later agent change"]).output().unwrap().status.success());
        let result=b.request("agent-integration-preview",false,|g|g.open(&r.source,&c)?.apply_merge(&review.merge,&c));assert!(matches!(result,Err(rocket_git::Error::Merge(rocket_git::MergeFailure::StalePreview))));
        let cancel=rocket_git::CancellationToken::new();cancel.cancel();assert!(matches!(integration::read(&b,&r,&cancel),Err(rocket_git::Error::Cancelled)));
        assert_eq!(fs::read_to_string(f.repository.join("資料.txt")).unwrap(),"base\n");
        let review=integration::read(&b,&r,&c).unwrap();let report=b.request("agent-integration-preview",false,|g|g.open(&r.source,&c)?.apply_merge(&review.merge,&c)).unwrap();assert!(report.write.result.is_ok());
        assert_eq!(fs::read_to_string(f.repository.join("資料.txt")).unwrap(),"agent\n");assert!(r.destination.exists());
    });
}

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
        tracking:false, conversation:None, approval:None,attention:false,last_activity:Default::default(),
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
fn copilot_and_existing_provider_records_roundtrip_without_relaunch() {
    for provider in Provider::ALL {
        let mut r = record(&std::env::temp_dir().join("source"), &std::env::current_exe().unwrap());
        r.provider = provider;
        r.command.arguments = provider.command().arguments;
        assert_eq!(r.command.args(), if provider == Provider::Copilot { vec!["--no-auto-update"] } else { vec![] });
        let mut s = State::default();
        s.load(model::decode(&model::encode(&[r.clone()]).unwrap()).unwrap());
        assert_eq!(s.records(), vec![r]);
        assert_eq!(s.sessions[0].phase, Phase::Restored);
        assert!(!s.active());
        assert!(s.sessions[0].terminal.is_none());
    }
    let mut r = record(&std::env::temp_dir().join("source"), &std::env::current_exe().unwrap());
    r.provider = Provider::Copilot;
    let bytes = model::encode(&[r]).unwrap();
    let unknown = String::from_utf8(bytes).unwrap().replace("\"copilot\"", "\"unknown-provider\"");
    assert!(model::decode(unknown.as_bytes()).is_err());
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
