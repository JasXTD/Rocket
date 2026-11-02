#![cfg(windows)]
//! Claude Code conflict proposals through the CLI stand-in: only the reviewed context
//! is sent, with fixed instructions, and replies are bounded like other AI requests.
#[allow(dead_code)]
mod common;
use common::{Fixture, isolated};
use rocket_git::*;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

fn token() -> CancellationToken {
    CancellationToken::new()
}
fn stub(f: &Fixture) -> PathBuf {
    let folder = f.root.join("AI fixture 文");
    fs::create_dir_all(&folder).unwrap();
    let exe = folder.join("claude.exe");
    let out = Command::new("rustc")
        .args(["--edition=2024", "--crate-name=claude_cli_fixture"])
        .arg(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/claude_cli_fixture.rs"))
        .arg("-o")
        .arg(&exe)
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    exe
}
fn request(exe: &Path) -> DraftRequest {
    DraftRequest {
        provider: DraftProvider::Claude,
        executable: Some(exe.into()),
        style: DraftStyle::Plain,
        language: "English".into(),
    }
}
fn proposal(replacement: &str) -> String {
    format!(
        "{{\"resolutions\":[{{\"id\":0,\"replacement\":{replacement:?},\"explanation\":\"Keeps both lines.\",\"confidence\":\"medium\"}}]}}"
    )
}

#[test]
fn claude_code_proposals_send_only_the_context_and_keep_reply_limits() {
    isolated(
        "ROCKET_CONFLICT_PROPOSAL_CASE",
        "claude_code_proposals_send_only_the_context_and_keep_reply_limits",
        || {
            let f = Fixture::new();
            f.write("PRIVATE_UNRELATED.txt", "PRIVATE_UNRELATED\n");
            let repo = f.repo();
            let exe = stub(&f);
            let folder = exe.parent().unwrap();
            let context = r#"{"api":1,"path":"文.txt","sections":[{"id":0,"ours":"PRIVATE_OURS\n","theirs":"PRIVATE_THEIRS\n"}]}"#;
            // A proposal larger than a commit message is accepted as returned.
            let large = proposal(&"PRIVATE_OURS 界\n".repeat(1200));
            assert!(large.len() > 8 * 1024 && large.len() < 32 * 1024);
            fs::write(folder.join("reply"), &large).unwrap();
            let reply = repo.propose_conflict_resolution(&request(&exe), context, &token()).unwrap();
            assert_eq!(reply, large);
            let prompt = fs::read_to_string(folder.join("request-1.txt")).unwrap();
            assert!(prompt.contains(context));
            assert!(prompt.contains("<conflict-context>"));
            assert!(!prompt.contains("PRIVATE_UNRELATED"), "only the reviewed context is sent");
            let arguments = fs::read_to_string(folder.join("arguments-1.txt")).unwrap();
            assert!(arguments.contains("You resolve Git merge conflicts"));
            assert!(arguments.contains("--tools"));
            // A fenced reply is unwrapped, as for drafts.
            fs::write(folder.join("mode"), "fence").unwrap();
            fs::write(folder.join("reply"), proposal("PRIVATE_OURS\n")).unwrap();
            let reply = repo.propose_conflict_resolution(&request(&exe), context, &token()).unwrap();
            assert_eq!(reply, proposal("PRIVATE_OURS\n"));
            // Over the proposal limit, or not Claude Code's JSON: refused, never returned.
            fs::write(folder.join("mode"), "ok").unwrap();
            fs::write(folder.join("reply"), proposal(&"x".repeat(33 * 1024))).unwrap();
            assert!(matches!(
                repo.propose_conflict_resolution(&request(&exe), context, &token()),
                Err(Error::CommitDraft { failure: DraftFailure::InvalidReply, .. })
            ));
            fs::write(folder.join("mode"), "garbage").unwrap();
            assert!(matches!(
                repo.propose_conflict_resolution(&request(&exe), context, &token()),
                Err(Error::CommitDraft { failure: DraftFailure::InvalidReply, .. })
            ));
            // A cancelled request does not start Claude Code.
            let cancelled = token();
            cancelled.cancel();
            let before = fs::read_dir(folder).unwrap().count();
            assert!(matches!(
                repo.propose_conflict_resolution(&request(&exe), context, &cancelled),
                Err(Error::Cancelled)
            ));
            assert_eq!(fs::read_dir(folder).unwrap().count(), before);
        },
    )
}
