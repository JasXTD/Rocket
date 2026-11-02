#![cfg(windows)]
#[allow(dead_code)]
mod common;
use common::*;
use rocket_git::*;
use std::{fs, os::windows::process::CommandExt, path::Path, process::Command};
fn key(path: &Path) {
    let out = Command::new(r"C:\Windows\System32\OpenSSH\ssh-keygen.exe")
        .args(["-q", "-t", "ed25519", "-N", ""])
        .arg("-f")
        .arg(path)
        .creation_flags(0x0800_0000)
        .output()
        .unwrap();
    assert!(out.status.success(), "disposable key generation");
}
fn base(f: &Fixture) -> std::path::PathBuf {
    fs::canonicalize(&f.root)
        .unwrap()
        .to_string_lossy()
        .trim_start_matches(r"\\?\")
        .into()
}
fn manager(f: &Fixture) -> SshManager {
    SshManager::new(base(f).join("SSH 資料/rocket-keys"), None).unwrap()
}
fn register(f: &Fixture, m: &SshManager, alias: &str) -> SshProfile {
    let p = base(f).join(format!("{alias}-既存"));
    key(&p);
    m.prepare(
        alias,
        "github.com",
        "git",
        22,
        Some(&p),
        &CancellationToken::new(),
    )
    .unwrap()
    .finish(&CancellationToken::new())
    .unwrap()
}
#[test]
fn ssh_profiles_unicode_and_matching_pairs() {
    isolated(
        "ROCKET_SSH_CASE",
        "ssh_profiles_unicode_and_matching_pairs",
        || {
            let f = Fixture::new();
            let m = manager(&f);
            let c = CancellationToken::new();
            let a = register(&f, &m, "rocket-personal");
            let b = register(&f, &m, "rocket-work");
            assert_ne!(a.fingerprint, b.fingerprint);
            assert!(!a.encrypted);
            let original = fs::read(&a.key).unwrap();
            assert!(
                m.check_config(&a, &c)
                    .unwrap()
                    .contains("git@github.com:22")
            );
            assert!(m.check_config(&b, &c).is_ok());
            assert_eq!(m.inventory(&c).unwrap().profiles.len(), 2);
            assert_eq!(fs::read(&a.key).unwrap(), original);
            fs::write(a.config.with_file_name("config"), "Match exec \"bad\"\n").unwrap();
            assert!(matches!(
                m.guard(&a, &c),
                Err(Error::Ssh(SshFailure::Changed))
            ));
        },
    );
}
#[test]
fn ssh_binding_preview_external_edits_cancel_and_recovery() {
    isolated(
        "ROCKET_SSH_CASE",
        "ssh_binding_preview_external_edits_cancel_and_recovery",
        || {
            let f = Fixture::new();
            let m = manager(&f);
            let c = CancellationToken::new();
            let p = register(&f, &m, "rocket-test");
            let repo = f.git.open(&f.repository, &c).unwrap();
            let config = repo.info().common_dir.join("config");
            let before = fs::read(&config).unwrap();
            let r = repo.preview_ssh_binding(&m, Some(&p), &c).unwrap();
            let stop = CancellationToken::new();
            stop.cancel();
            assert!(matches!(
                repo.apply_ssh_binding(&m, &r, &stop),
                Err(Error::Cancelled)
            ));
            assert_eq!(fs::read(&config).unwrap(), before);
            let mut changed = before.clone();
            changed.extend(b"\n# external edit\n");
            fs::write(&config, &changed).unwrap();
            assert!(matches!(
                repo.apply_ssh_binding(&m, &r, &c),
                Err(Error::Ssh(SshFailure::Changed))
            ));
            assert_eq!(fs::read(&config).unwrap(), changed);
            let r = repo.preview_ssh_binding(&m, Some(&p), &c).unwrap();
            let backup = repo.apply_ssh_binding(&m, &r, &c).unwrap();
            assert_eq!(fs::read(backup.join("displaced")).unwrap(), changed);
            let applied = fs::read(&config).unwrap();
            assert_ne!(applied, changed);
            let r = repo.preview_ssh_binding(&m, None, &c).unwrap();
            assert!(r.restoring);
            repo.apply_ssh_binding(&m, &r, &c).unwrap();
            assert_eq!(fs::read(&config).unwrap(), changed);
        },
    );
}
#[test]
fn ssh_creation_preserves_existing_and_cancellation() {
    isolated(
        "ROCKET_SSH_CASE",
        "ssh_creation_preserves_existing_and_cancellation",
        || {
            let f = Fixture::new();
            let m = manager(&f);
            let c = CancellationToken::new();
            let p = m
                .prepare("rocket-new", "gitlab.com", "git", 22, None, &c)
                .unwrap();
            let folder = p.directory.clone();
            drop(p);
            assert!(!folder.exists());
            fs::create_dir(&folder).unwrap();
            fs::write(folder.join("important"), "retain").unwrap();
            assert!(matches!(
                m.prepare("rocket-new", "gitlab.com", "git", 22, None, &c),
                Err(Error::Ssh(SshFailure::Exists))
            ));
            assert_eq!(fs::read(folder.join("important")).unwrap(), b"retain");
            let stop = CancellationToken::new();
            stop.cancel();
            assert!(matches!(
                m.prepare("rocket-cancel", "gitlab.com", "git", 22, None, &stop),
                Err(Error::Cancelled)
            ));
            assert!(!m.root().join("rocket-cancel").exists());
        },
    );
}
#[test]
fn ssh_rejects_injection_mismatched_keys_and_unencrypted_generation() {
    isolated(
        "ROCKET_SSH_CASE",
        "ssh_rejects_injection_mismatched_keys_and_unencrypted_generation",
        || {
            let f = Fixture::new();
            let m = manager(&f);
            let c = CancellationToken::new();
            for host in [
                "github.com\nProxyCommand bad",
                "-oProxyCommand=bad",
                "a b",
                "*.example.com",
            ] {
                assert!(matches!(
                    m.prepare("rocket-invalid", host, "git", 22, None, &c),
                    Err(Error::Ssh(SshFailure::InvalidProfile))
                ));
            }
            let p = m
                .prepare("rocket-unencrypted", "github.com", "git", 22, None, &c)
                .unwrap();
            let folder = p.directory.clone();
            key(&p.key);
            assert!(matches!(
                p.finish(&c),
                Err(Error::Ssh(SshFailure::Unencrypted))
            ));
            assert!(!folder.join("profile").exists());
            assert!(folder.join("id_ed25519").exists());
            let a = register(&f, &m, "rocket-one");
            let b = register(&f, &m, "rocket-two");
            let pub_path = |p: &Path| std::path::PathBuf::from(format!("{}.pub", p.display()));
            fs::copy(pub_path(&b.key), pub_path(&a.key)).unwrap();
            assert!(matches!(
                m.guard(&a, &c),
                Err(Error::Ssh(SshFailure::KeyInvalid))
            ));
        },
    );
}
#[test]
fn ssh_logs_are_typed_and_payload_free() {
    isolated(
        "ROCKET_SSH_CASE",
        "ssh_logs_are_typed_and_payload_free",
        || {
            let f = Fixture::new();
            let log = DiagnosticLog::open(f.root.join("logs")).unwrap();
            let m = SshManager::new(base(&f).join("SECRET_FOLDER"), Some(log.clone())).unwrap();
            let c = CancellationToken::new();
            let r: Result<()> = log.request("ssh-create", |_| {
                m.prepare("bad", "PRIVATE_HOST", "git", 22, None, &c)
                    .map(|_| ())
            });
            assert!(r.is_err());
            let text = fs::read_to_string(log.directory().join("rocket.jsonl")).unwrap();
            assert!(text.contains("ssh_profile_invalid"));
            assert!(text.contains("ssh-create"));
            assert!(!text.contains("SECRET_FOLDER"));
            assert!(!text.contains("PRIVATE_HOST"));
        },
    );
}

#[test]
fn ssh_binding_refuses_overrides_and_preserves_existing_locks() {
    isolated(
        "ROCKET_SSH_CASE",
        "ssh_binding_refuses_overrides_and_preserves_existing_locks",
        || {
            let f = Fixture::new();
            let m = manager(&f);
            let c = CancellationToken::new();
            let p = register(&f, &m, "rocket-overrides");
            let repo = f.git.open(&f.repository, &c).unwrap();
            let config = repo.info().common_dir.join("config");
            let before = fs::read(&config).unwrap();
            for extra in [
                "\n[include]\npath = missing-config\n",
                "\n[extensions]\nworktreeConfig = true\n",
            ] {
                let mut bytes = before.clone();
                bytes.extend(extra.as_bytes());
                fs::write(&config, &bytes).unwrap();
                assert!(matches!(
                    repo.preview_ssh_binding(&m, Some(&p), &c),
                    Err(Error::Ssh(SshFailure::Overrides))
                ));
                assert_eq!(fs::read(&config).unwrap(), bytes);
            }
            fs::write(&config, &before).unwrap();
            let preview = repo.preview_ssh_binding(&m, Some(&p), &c).unwrap();
            let lock = config.with_extension("lock");
            fs::write(&lock, "owned by another writer").unwrap();
            assert!(repo.apply_ssh_binding(&m, &preview, &c).is_err());
            assert_eq!(fs::read(&lock).unwrap(), b"owned by another writer");
            assert_eq!(fs::read(&config).unwrap(), before);
        },
    );
}
