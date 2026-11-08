#![cfg(any(windows, target_os = "linux", target_os = "macos"))]
#[allow(dead_code)]
mod common;
use common::*;
use rocket_git::*;
use std::{fs, path::Path};
fn key(path: &Path) {
    let out = ssh_keygen()
        .args(["-q", "-t", "ed25519", "-N", ""])
        .arg("-f")
        .arg(path)
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

#[cfg(target_os = "macos")]
#[test]
fn macos_key_acl_grants_are_refused_without_changing_permissions() {
    isolated("ROCKET_SSH_CASE", "macos_key_acl_grants_are_refused_without_changing_permissions", || {
        let f=Fixture::new();
        let manager=manager(&f);
        let profile=register(&f,&manager,"rocket-acl");
        let bytes=fs::read(&profile.key).unwrap();
        assert!(std::process::Command::new("/bin/chmod").args(["+a","everyone allow read"]).arg(&profile.key).status().unwrap().success());
        assert!(matches!(manager.guard(&profile,&CancellationToken::new()),Err(Error::Ssh(SshFailure::KeyPermissions))));
        assert_eq!(fs::read(&profile.key).unwrap(),bytes);
        // Remove only the ACL added to this disposable fixture, then verify retry.
        assert!(std::process::Command::new("/bin/chmod").arg("-N").arg(&profile.key).status().unwrap().success());
        assert!(manager.guard(&profile,&CancellationToken::new()).is_ok());
    });
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

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn mode(path: &Path) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    fs::symlink_metadata(path).unwrap().permissions().mode() & 0o777
}
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn set_mode(path: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn linux_profiles_are_private_and_keys_other_users_can_open_are_refused() {
    isolated(
        "ROCKET_SSH_CASE",
        "linux_profiles_are_private_and_keys_other_users_can_open_are_refused",
        || {
            let f = Fixture::new();
            let m = manager(&f);
            let c = CancellationToken::new();
            let p = register(&f, &m, "rocket-private");
            // Folders Rocket creates are 0700 and its files 0600; ssh-keygen made the key 0600.
            for folder in [m.root().parent().unwrap(), m.root(), p.config.parent().unwrap()] {
                assert_eq!(mode(folder), 0o700, "{}", folder.display());
            }
            for file in [p.config.clone(), p.config.with_file_name("profile")] {
                assert_eq!(mode(&file), 0o600, "{}", file.display());
            }
            assert!(m.check_config(&p, &c).unwrap().starts_with("OpenSSH: git@github.com:22"));

            // A key other users can open is refused, as ssh refuses it, and kept as it is.
            set_mode(&p.key, 0o640);
            assert!(matches!(m.guard(&p, &c), Err(Error::Ssh(SshFailure::KeyPermissions))));
            assert_eq!(m.inventory(&c).unwrap().invalid, 1);
            assert_eq!(mode(&p.key), 0o640);
            set_mode(&p.key, 0o600);
            assert!(m.guard(&p, &c).is_ok());
            let open = base(&f).join("open-key");
            key(&open);
            set_mode(&open, 0o604);
            assert!(matches!(
                m.prepare("rocket-open", "github.com", "git", 22, Some(&open), &c),
                Err(Error::Ssh(SshFailure::KeyPermissions))
            ));
            assert!(!m.root().join("rocket-open").exists());
            // A link to a key is refused like a Windows reparse point.
            let link = base(&f).join("linked-key");
            std::os::unix::fs::symlink(&p.key, &link).unwrap();
            std::os::unix::fs::symlink(format!("{}.pub", p.key.display()), base(&f).join("linked-key.pub")).unwrap();
            assert!(matches!(
                m.prepare("rocket-linked", "github.com", "git", 22, Some(&link), &c),
                Err(Error::Ssh(SshFailure::UnsafePath))
            ));

            // Binding keeps the repository config's own permissions.
            let repo = f.git.open(&f.repository, &c).unwrap();
            let config = repo.info().common_dir.join("config");
            set_mode(&config, 0o600);
            let review = repo.preview_ssh_binding(&m, Some(&p), &c).unwrap();
            let backup = repo.apply_ssh_binding(&m, &review, &c).unwrap();
            assert_eq!(mode(&config), 0o600);
            assert!(fs::read_to_string(&config).unwrap().contains("sshCommand = '/usr/bin/ssh' -F '"));
            assert_eq!(mode(backup.parent().unwrap()), 0o700);
            assert_eq!(mode(&backup.join("after")), 0o600);
        },
    );
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn linux_ssh_agent_is_the_one_ssh_auth_sock_names() {
    isolated(
        "ROCKET_SSH_CASE",
        "linux_ssh_agent_is_the_one_ssh_auth_sock_names",
        || {
            use std::{process::Command, time::{Duration, Instant}};
            let f = Fixture::new();
            let m = manager(&f);
            let c = CancellationToken::new();
            let p = register(&f, &m, "rocket-agent");
            // No socket named: no agent, and ssh-add is not even asked.
            assert_eq!(m.agent(&c).unwrap(), (false, vec![]));
            let socket = base(&f).join("agent.sock");
            let mut agent = Command::new("/usr/bin/ssh-agent").arg("-D").arg("-a").arg(&socket).spawn().unwrap();
            let start = Instant::now();
            while !socket.exists() {
                assert!(start.elapsed() < Duration::from_secs(10), "ssh-agent did not start");
                std::thread::sleep(Duration::from_millis(20));
            }
            // SAFETY: this isolated test process has no other threads using the environment.
            unsafe { std::env::set_var("SSH_AUTH_SOCK", &socket) };
            assert_eq!(m.agent(&c).unwrap(), (true, vec![]));
            let added = Command::new("/usr/bin/ssh-add").arg(&p.key).env("SSH_AUTH_SOCK", &socket).output().unwrap();
            assert!(added.status.success(), "{}", String::from_utf8_lossy(&added.stderr));
            assert_eq!(m.agent(&c).unwrap(), (true, vec![p.fingerprint.clone()]));
            m.unload(&p, &c).unwrap();
            assert_eq!(m.agent(&c).unwrap(), (true, vec![]));
            // A link to the socket, as tmux setups keep one, reaches the same agent.
            let link = base(&f).join("agent-link.sock");
            std::os::unix::fs::symlink(&socket, &link).unwrap();
            unsafe { std::env::set_var("SSH_AUTH_SOCK", &link) };
            assert_eq!(m.agent(&c).unwrap(), (true, vec![]));
            // A path that isn't a socket is no agent either.
            let plain = base(&f).join("not-a-socket");
            fs::write(&plain, "").unwrap();
            unsafe { std::env::set_var("SSH_AUTH_SOCK", &plain) };
            assert_eq!(m.agent(&c).unwrap(), (false, vec![]));
            assert!(matches!(m.unload(&p, &c), Err(Error::Ssh(SshFailure::AgentUnavailable))));
            agent.kill().unwrap();
            agent.wait().unwrap();
        },
    );
}
