mod common;
use common::{Fixture, isolated};
use rocket_git::*;
use std::{fs, process::Command};
fn token() -> CancellationToken {
    CancellationToken::new()
}
fn text(f: &Fixture, a: &[&str]) -> String {
    String::from_utf8(f.run(a)).unwrap().trim_end().into()
}
fn setup() -> Fixture {
    let f = Fixture::new();
    f.write("検索 🚀.txt", "base\n");
    f.commit("PRIVATE unsigned");
    f
}
fn ssh(f: &Fixture) -> SigningOptions {
    let key = f.root.join("disposable key 文");
    let result = Command::new("C:/Program Files/Git/usr/bin/ssh-keygen.exe")
        .args(["-q", "-t", "ed25519", "-N", "", "-f"])
        .arg(&key)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let allowed = f.root.join("allowed signers 文");
    fs::write(
        &allowed,
        format!(
            "rocket@example.invalid {}",
            fs::read_to_string(key.with_extension("pub")).unwrap()
        ),
    )
    .unwrap();
    f.run(&[
        "config",
        "gpg.ssh.program",
        "C:/Program Files/Git/usr/bin/ssh-keygen.exe",
    ]);
    SigningOptions {
        format: Some("ssh".into()),
        key: Some(key.to_str().unwrap().into()),
        sign_commits: Some(true),
        allowed_signers: Some(allowed.to_str().unwrap().into()),
    }
}
#[test]
fn settings_stale_locked_cancelled_and_inherited() {
    isolated(
        "ROCKET_SIGNING_CASE",
        "settings_stale_locked_cancelled_and_inherited",
        || {
            let f = setup();
            let repo = f.repo();
            let before = repo.signing(&token()).unwrap();
            let desired = ssh(&f);
            let report = repo.set_signing(&before, &desired, &token()).unwrap();
            assert!(report.result.is_err());
            let before = repo.signing(&token()).unwrap();
            let old = fs::read(f.repository.join(".git/config")).unwrap();
            fs::write(f.repository.join(".git/config.lock"), "existing").unwrap();
            assert!(
                repo.set_signing(&before, &desired, &token())
                    .unwrap()
                    .result
                    .is_err()
            );
            assert_eq!(fs::read(f.repository.join(".git/config")).unwrap(), old);
            assert_eq!(
                fs::read(f.repository.join(".git/config.lock")).unwrap(),
                b"existing"
            );
            fs::remove_file(f.repository.join(".git/config.lock")).unwrap();
            let cancel = token();
            cancel.cancel();
            assert!(repo.set_signing(&before, &desired, &cancel).is_err());
            assert_eq!(fs::read(f.repository.join(".git/config")).unwrap(), old);
            let report = repo.set_signing(&before, &desired, &token()).unwrap();
            report.result.unwrap();
            assert_eq!(report.observed.unwrap().effective, desired);
            let settings = repo.signing(&token()).unwrap();
            repo.set_signing(&settings, &SigningOptions::default(), &token())
                .unwrap()
                .result
                .unwrap();
            assert!(repo.signing(&token()).unwrap().local.key.is_none());
            assert_eq!(text(&f, &["config", "user.name"]), "Rocket Test");
        },
    )
}
#[test]
fn ssh_commit_tag_verify_and_preserve_local_state() {
    isolated(
        "ROCKET_SIGNING_CASE",
        "ssh_commit_tag_verify_and_preserve_local_state",
        || {
            let f = setup();
            let repo = f.repo();
            let unsigned = ObjectId::parse(&text(&f, &["rev-parse", "HEAD"])).unwrap();
            assert_eq!(
                repo.verify_signature(&SignatureTarget::Commit(unsigned), &token())
                    .unwrap()
                    .status,
                SignatureStatus::Unsigned
            );
            let desired = ssh(&f);
            repo.set_signing(&repo.signing(&token()).unwrap(), &desired, &token())
                .unwrap()
                .result
                .unwrap();
            f.write("検索 🚀.txt", "signed\n");
            repo.stage_all(&token()).unwrap().result.unwrap();
            repo.commit("PRIVATE signed 中文", &token())
                .unwrap()
                .result
                .unwrap();
            let head = ObjectId::parse(&text(&f, &["rev-parse", "HEAD"])).unwrap();
            let v = repo
                .verify_signature(&SignatureTarget::Commit(head), &token())
                .unwrap();
            assert_eq!(v.status, SignatureStatus::Valid, "{v:?}");
            let index = fs::read(f.repository.join(".git/index")).unwrap();
            f.write("検索 🚀.txt", "dirty\n");
            let preview = repo
                .preview_signed_tag("release/検索-🚀", "HEAD", "PRIVATE annotation\n", &token())
                .unwrap();
            let report = repo.apply_tag(&preview, &token()).unwrap();
            report.result.unwrap();
            report.verification.unwrap();
            let tag = report.observed.unwrap().unwrap();
            assert_ne!(tag.id, preview.details().entry.id);
            assert_eq!(fs::read(f.repository.join(".git/index")).unwrap(), index);
            assert_eq!(
                fs::read(f.repository.join("検索 🚀.txt")).unwrap(),
                b"dirty\n"
            );
            let v = repo
                .verify_signature(&SignatureTarget::Tag(tag.clone()), &token())
                .unwrap();
            assert_eq!(v.status, SignatureStatus::Valid, "{v:?}");
            fs::write(desired.allowed_signers.unwrap(), "").unwrap();
            assert_eq!(
                repo.verify_signature(&SignatureTarget::Tag(tag), &token())
                    .unwrap()
                    .status,
                SignatureStatus::Untrusted
            );
            let preview = repo
                .preview_signed_tag("cancelled", "HEAD", "message", &token())
                .unwrap();
            let cancel = token();
            cancel.cancel();
            assert!(repo.apply_tag(&preview, &cancel).is_err());
            assert!(
                !repo
                    .tags(100, &token())
                    .unwrap()
                    .entries
                    .iter()
                    .any(|t| t.name == b"cancelled")
            );
        },
    )
}

#[test]
fn boolean_forms_and_literal_annotation_are_not_misclassified() {
    isolated(
        "ROCKET_SIGNING_CASE",
        "boolean_forms_and_literal_annotation_are_not_misclassified",
        || {
            let f = setup();
            let repo = f.repo();
            f.run(&["config", "commit.gpgsign", ""]);
            assert_eq!(
                repo.signing(&token()).unwrap().effective.sign_commits,
                Some(false)
            );
            f.run(&["config", "commit.gpgsign", "2"]);
            assert_eq!(
                repo.signing(&token()).unwrap().effective.sign_commits,
                Some(true)
            );
            f.run(&["config", "--unset-all", "commit.gpgsign"]);
            use std::io::Write;
            std::fs::OpenOptions::new()
                .append(true)
                .open(f.repository.join(".git/config"))
                .unwrap()
                .write_all(b"\n[commit]\n gpgSign\n")
                .unwrap();
            assert_eq!(
                repo.signing(&token()).unwrap().effective.sign_commits,
                Some(true)
            );
            let p = repo
                .preview_create_tag(
                    "literal",
                    "HEAD",
                    Some("Example text: -----BEGIN unrelated text"),
                    &token(),
                )
                .unwrap();
            let r = repo.apply_tag(&p, &token()).unwrap();
            r.result.unwrap();
            assert_eq!(
                repo.verify_signature(
                    &SignatureTarget::Tag(r.observed.unwrap().unwrap()),
                    &token()
                )
                .unwrap()
                .status,
                SignatureStatus::Unsigned
            );
        },
    )
}

#[test]
fn signed_tag_stale_settings_and_concurrent_name_keep_existing_data() {
    isolated(
        "ROCKET_SIGNING_CASE",
        "signed_tag_stale_settings_and_concurrent_name_keep_existing_data",
        || {
            let f = setup();
            let repo = f.repo();
            let desired = ssh(&f);
            repo.set_signing(&repo.signing(&token()).unwrap(), &desired, &token())
                .unwrap()
                .result
                .unwrap();
            let p = repo
                .preview_signed_tag("stale", "HEAD", "message", &token())
                .unwrap();
            f.run(&["config", "user.signingkey", "missing"]);
            assert!(matches!(
                repo.apply_tag(&p, &token()),
                Err(Error::Signing(SigningFailure::StaleSettings))
            ));
            assert!(repo.tags(100, &token()).unwrap().entries.is_empty());
            repo.set_signing(&repo.signing(&token()).unwrap(), &desired, &token())
                .unwrap()
                .result
                .unwrap();
            let p = repo
                .preview_signed_tag("occupied", "HEAD", "message", &token())
                .unwrap();
            f.run(&["tag", "--no-sign", "occupied"]);
            let existing = text(&f, &["rev-parse", "refs/tags/occupied"]);
            assert!(repo.apply_tag(&p, &token()).is_err());
            assert_eq!(text(&f, &["rev-parse", "refs/tags/occupied"]), existing);
        },
    )
}

#[test]
fn signer_alias_order_and_sandbox_only_global_includes_are_respected() {
    isolated(
        "ROCKET_SIGNING_CASE",
        "signer_alias_order_and_sandbox_only_global_includes_are_respected",
        || {
            let f = setup();
            let repo = f.repo();
            f.run(&["config", "gpg.program", "first"]);
            f.run(&["config", "gpg.openpgp.program", "second"]);
            assert_eq!(repo.signing(&token()).unwrap().program(), "second");
            f.run(&["config", "--unset-all", "gpg.program"]);
            f.run(&["config", "gpg.program", "last"]);
            assert_eq!(repo.signing(&token()).unwrap().program(), "last");
            let options = ssh(&f);
            repo.set_signing(&repo.signing(&token()).unwrap(), &options, &token())
                .unwrap()
                .result
                .unwrap();
            // A global conditional include applicable only inside the owned sandbox
            // must not change the target repository's observed tag signing behavior.
            f.run(&["config", "--unset-all", "gpg.ssh.program"]);
            let global = std::env::var_os("GIT_CONFIG_GLOBAL").unwrap();
            let config = f.root.join("sandbox-only.config");
            fs::write(&config,"[user]\n name = PRIVATE wrong identity\n email = wrong@example.invalid\n[gpg]\n format = openpgp\n[gpg \"ssh\"]\n program = rocket-fixture-missing-signer\n").unwrap();
            let old = fs::read(&global).unwrap();
            fs::write(
                &global,
                format!(
                    "[includeIf \"gitdir:**/rocket-sign-*/\"]\n path = {}\n",
                    config.to_str().unwrap().replace('\\', "/")
                ),
            )
            .unwrap();
            let p = repo
                .preview_signed_tag("isolated", "HEAD", "message", &token())
                .unwrap();
            let result = repo.apply_tag(&p, &token());
            fs::write(global, old).unwrap();
            let r = result.unwrap();
            r.result.unwrap();
            let entry = r.observed.unwrap().unwrap();
            let details = repo.inspect_tag(&entry, &token()).unwrap();
            let annotation = String::from_utf8(details.annotation.unwrap()).unwrap();
            assert!(annotation.contains("tagger Rocket Test <rocket@example.invalid>"));
            assert!(!annotation.contains("wrong identity"));
            assert_eq!(
                repo.verify_signature(&SignatureTarget::Tag(entry), &token())
                    .unwrap()
                    .status,
                SignatureStatus::Valid
            );
        },
    )
}
