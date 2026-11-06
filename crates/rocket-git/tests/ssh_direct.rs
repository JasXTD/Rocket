#![cfg(any(windows, target_os = "linux"))]
//! On Linux a saved passphrase is sealed with Rocket's key in the desktop keyring, so the
//! tests that save one are ignored by default there: run them with --include-ignored in a
//! session with an unlocked keyring (see guide.md).
#[allow(dead_code)]
mod common;
use common::*;
use rocket_git::*;
use std::{
    fs,
    path::PathBuf,
    process::{Command, Output},
    time::{Duration, Instant},
};

/// Starts helpers without a console window on Windows.
fn quiet(command: &mut Command) -> &mut Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    command
}

fn setup() -> (Fixture, SshManager, SshProfile, String) {
    let f = Fixture::new();
    let root = fs::canonicalize(&f.root)
        .unwrap()
        .to_string_lossy()
        .trim_start_matches(r"\\?\")
        .to_owned();
    let key = PathBuf::from(&root).join("鍵 identity");
    let secret = format!("Disposable SSH phrase {}!", std::process::id());
    let status = ssh_keygen()
        .args(["-q", "-t", "ed25519", "-a", "16", "-N", &secret, "-f"])
        .arg(&key)
        .status()
        .unwrap();
    assert!(status.success());
    let m = SshManager::new(PathBuf::from(root).join("profiles"), None).unwrap();
    let p = m
        .prepare(
            "rocket-direct",
            "github.com",
            "git",
            22,
            Some(&key),
            &CancellationToken::new(),
        )
        .unwrap()
        .finish(&CancellationToken::new())
        .unwrap();
    (f, m, p, secret)
}
fn helper(p: &SshProfile, args: &[&str]) -> Output {
    quiet(
        Command::new(env!("CARGO_BIN_EXE_rocket-ssh"))
            .arg("--profile")
            .arg(p.config.parent().unwrap())
            .args(args),
    )
    .output()
    .unwrap()
}

#[test]
#[cfg_attr(target_os = "linux", ignore = "needs an unlocked desktop keyring")]
fn direct_binding_and_helper_recheck_custom_services_without_contacting_a_host() {
    isolated(
        "ROCKET_SSH_DIRECT_CASE",
        "direct_binding_and_helper_recheck_custom_services_without_contacting_a_host",
        || {
            let (f, m, p, secret) = setup();
            let c = CancellationToken::new();
            let p = m.remember_passphrase(&p, secret, &c).unwrap();
            let repo = f.git.open(&f.repository, &c).unwrap();
            let config = repo.info().common_dir.join("config");
            let original = fs::read(&config).unwrap();
            for key in [
                "remote.origin.uploadpack",
                "remote.other.receivepack",
                "remote.other.vcs",
            ] {
                f.run(&["config", key, "git-upload-pack ; echo unexpected"]);
                let edited = fs::read(&config).unwrap();
                assert!(matches!(
                    repo.preview_ssh_authentication(&m, Some(&p), SshAuthentication::Direct, &c),
                    Err(Error::Ssh(SshFailure::Overrides))
                ));
                assert_eq!(fs::read(&config).unwrap(), edited);
                fs::write(&config, &original).unwrap();
            }
            let review = repo
                .preview_ssh_authentication(&m, Some(&p), SshAuthentication::Direct, &c)
                .unwrap();
            f.run(&["config", "remote.origin.receivepack", "git-receive-pack"]);
            let edited = fs::read(&config).unwrap();
            assert!(matches!(
                repo.apply_ssh_binding(&m, &review, &c),
                Err(Error::Ssh(SshFailure::Overrides))
            ));
            assert_eq!(fs::read(&config).unwrap(), edited);
            fs::write(&config, &original).unwrap();
            repo.apply_ssh_binding(&m, &review, &c).unwrap();
            let bound = fs::read(&config).unwrap();
            // -G only prints OpenSSH's effective configuration, never connects.
            let probe = |remote_command: &str| {
                quiet(
                    Command::new(env!("CARGO_BIN_EXE_rocket-ssh"))
                        .current_dir(&f.repository)
                        .env("GIT_CONFIG_GLOBAL", f.root.join("empty.config"))
                        .env("GIT_CONFIG_SYSTEM", f.root.join("empty.config"))
                        .arg("--profile")
                        .arg(p.config.parent().unwrap())
                        .args(["-G", "git@github.com", remote_command]),
                )
                .output()
                .unwrap()
            };
            assert!(probe("git-upload-pack 'team/repo.git'").status.success());
            for command in [
                "git-upload-pack 'repo'; echo unexpected",
                "git-lfs-transfer repo download && echo unexpected",
            ] {
                let output = probe(command);
                assert!(!output.status.success());
                assert!(output.stdout.is_empty());
            }
            for key in [
                "remote.origin.uploadpack",
                "remote.other.receivepack",
                "remote.other.vcs",
            ] {
                f.run(&["config", key, "git-upload-pack"]);
                let edited = fs::read(&config).unwrap();
                let output = probe("git-upload-pack 'team/repo.git'");
                assert!(!output.status.success());
                assert!(output.stdout.is_empty());
                assert!(String::from_utf8_lossy(&output.stderr).contains("ssh_config_overrides"));
                assert_eq!(fs::read(&config).unwrap(), edited);
                fs::write(&config, &bound).unwrap();
            }
            // An effective global include is checked too, even after binding.
            let included = f.root.join("included.config");
            fs::write(
                &included,
                b"[remote \"origin\"]\nuploadpack = git-upload-pack\n",
            )
            .unwrap();
            f.run(&[
                "config",
                "--file",
                f.root.join("empty.config").to_str().unwrap(),
                "include.path",
                included.to_str().unwrap(),
            ]);
            let output = probe("git-upload-pack 'team/repo.git'");
            assert!(!output.status.success());
            assert!(String::from_utf8_lossy(&output.stderr).contains("ssh_config_overrides"));
            assert_eq!(fs::read(&config).unwrap(), bound);
        },
    );
}

#[test]
#[cfg_attr(target_os = "linux", ignore = "needs an unlocked desktop keyring")]
fn direct_key_verifies_encrypts_binds_and_forgets_without_an_agent() {
    isolated(
        "ROCKET_SSH_DIRECT_CASE",
        "direct_key_verifies_encrypts_binds_and_forgets_without_an_agent",
        || {
            let (f, m, p, secret) = setup();
            let c = CancellationToken::new();
            let before = fs::read(&p.key).unwrap();
            let config = fs::read(&p.config).unwrap();
            assert!(matches!(
                m.remember_passphrase(&p, "Wrong disposable phrase".into(), &c),
                Err(Error::Ssh(SshFailure::UnlockFailed))
            ));
            assert!(!p.config.with_file_name("passphrase.dpapi").exists());
            let p = m.remember_passphrase(&p, secret.clone(), &c).unwrap();
            assert!(p.passphrase_saved);
            let settings = m
                .check_authentication(&p, SshAuthentication::Direct, &c)
                .unwrap();
            assert!(settings.contains("agent disabled"));
            assert!(settings.contains("host verification required"));
            let ciphertext = fs::read(p.config.with_file_name("passphrase.dpapi")).unwrap();
            assert!(
                !ciphertext
                    .windows(secret.len())
                    .any(|v| v == secret.as_bytes())
            );
            let out = helper(&p, &["-G", "git@github.com"]);
            assert!(
                out.status.success(),
                "{}",
                String::from_utf8_lossy(&out.stderr)
            );
            let config_dump = String::from_utf8(out.stdout).unwrap();
            for value in [
                "identityagent none",
                "stricthostkeychecking true",
                "identitiesonly yes",
                "passwordauthentication no",
            ] {
                assert!(config_dump.contains(value), "Missing {value}");
            }
            for arguments in [
                vec!["-G", "git@another.invalid"],
                vec!["-o", "StrictHostKeyChecking=no", "git@github.com"],
                vec!["-p", "2222", "git@github.com"],
            ] {
                assert!(!helper(&p, &arguments).status.success());
            }
            let repo = f.git.open(&f.repository, &c).unwrap();
            let cfg = repo.info().common_dir.join("config");
            let original = fs::read(&cfg).unwrap();
            let review = repo
                .preview_ssh_authentication(&m, Some(&p), SshAuthentication::Direct, &c)
                .unwrap();
            repo.apply_ssh_binding(&m, &review, &c).unwrap();
            assert!(
                String::from_utf8(fs::read(&cfg).unwrap())
                    .unwrap()
                    .contains(&format!("rocket-ssh{}", std::env::consts::EXE_SUFFIX))
            );
            let restore = repo.preview_ssh_binding(&m, None, &c).unwrap();
            repo.apply_ssh_binding(&m, &restore, &c).unwrap();
            assert_eq!(fs::read(cfg).unwrap(), original);
            let p = m.forget_passphrase(&p, &c).unwrap();
            assert!(!p.passphrase_saved);
            assert!(matches!(
                repo.preview_ssh_authentication(&m, Some(&p), SshAuthentication::Direct, &c),
                Err(Error::Ssh(SshFailure::PassphraseUnavailable))
            ));
            assert_eq!(fs::read(&p.key).unwrap(), before);
            assert_eq!(fs::read(&p.config).unwrap(), config);
        },
    );
}
#[test]
fn unencrypted_direct_key_needs_no_saved_passphrase() {
    isolated(
        "ROCKET_SSH_DIRECT_CASE",
        "unencrypted_direct_key_needs_no_saved_passphrase",
        || {
            let f = Fixture::new();
            let root = fs::canonicalize(&f.root)
                .unwrap()
                .to_string_lossy()
                .trim_start_matches(r"\\?\")
                .to_owned();
            let key = PathBuf::from(&root).join("unencrypted 鍵");
            let status = ssh_keygen()
                .args(["-q", "-t", "ed25519", "-N", "", "-f"])
                .arg(&key)
                .status()
                .unwrap();
            assert!(status.success());
            let c = CancellationToken::new();
            let m = SshManager::new(PathBuf::from(root).join("profiles"), None).unwrap();
            let p = m
                .prepare("rocket-plain", "github.com", "git", 22, Some(&key), &c)
                .unwrap()
                .finish(&c)
                .unwrap();
            assert!(!p.encrypted && !p.passphrase_saved);
            assert!(
                m.check_authentication(&p, SshAuthentication::Direct, &c)
                    .unwrap()
                    .contains("agent disabled")
            );
            let repo = f.git.open(&f.repository, &c).unwrap();
            let cfg = repo.info().common_dir.join("config");
            let original = fs::read(&cfg).unwrap();
            let review = repo
                .preview_ssh_authentication(&m, Some(&p), SshAuthentication::Direct, &c)
                .unwrap();
            repo.apply_ssh_binding(&m, &review, &c).unwrap();
            repo.apply_ssh_binding(&m, &repo.preview_ssh_binding(&m, None, &c).unwrap(), &c)
                .unwrap();
            assert_eq!(fs::read(cfg).unwrap(), original);
            assert!(!p.config.with_file_name("passphrase.dpapi").exists());
        },
    );
}
#[test]
#[cfg_attr(target_os = "linux", ignore = "needs an unlocked desktop keyring")]
fn direct_key_cancellation_and_external_edits_preserve_credentials() {
    isolated(
        "ROCKET_SSH_DIRECT_CASE",
        "direct_key_cancellation_and_external_edits_preserve_credentials",
        || {
            let (_f, m, p, secret) = setup();
            let c = CancellationToken::new();
            c.cancel();
            assert!(matches!(
                m.remember_passphrase(&p, secret.clone(), &c),
                Err(Error::Cancelled)
            ));
            let c = CancellationToken::new();
            let stop = c.clone();
            let pending_directory = p.config.parent().unwrap().to_path_buf();
            let start = Instant::now();
            let thread = std::thread::spawn(move || {
                let deadline = Instant::now() + Duration::from_secs(10);
                while Instant::now() < deadline {
                    if fs::read_dir(&pending_directory).unwrap().any(|e| {
                        e.unwrap()
                            .file_name()
                            .to_string_lossy()
                            .starts_with("pending-")
                    }) {
                        // The protected request now exists: cancel the actual
                        // helper/verification phase, not just an initial read.
                        stop.cancel();
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(2));
                }
                stop.cancel()
            });
            assert!(matches!(
                m.remember_passphrase(&p, secret.clone(), &c),
                Err(Error::Cancelled)
            ));
            thread.join().unwrap();
            assert!(start.elapsed() < Duration::from_secs(10));
            assert!(!p.config.with_file_name("passphrase.dpapi").exists());
            assert!(!fs::read_dir(p.config.parent().unwrap()).unwrap().any(|e| {
                e.unwrap()
                    .file_name()
                    .to_string_lossy()
                    .starts_with("pending-")
            }));
            let old = fs::read(&p.config).unwrap();
            fs::write(&p.config, b"# external\n").unwrap();
            assert!(matches!(
                m.remember_passphrase(&p, secret, &CancellationToken::new()),
                Err(Error::Ssh(SshFailure::Changed))
            ));
            assert_eq!(fs::read(&p.config).unwrap(), b"# external\n");
            fs::write(&p.config, old).unwrap();
        },
    );
}

#[test]
#[cfg_attr(target_os = "linux", ignore = "needs an unlocked desktop keyring")]
fn credential_gate_rejects_concurrent_writers_and_recovers_after_owner_releases() {
    isolated(
        "ROCKET_SSH_DIRECT_CASE",
        "credential_gate_rejects_concurrent_writers_and_recovers_after_owner_releases",
        || {
            let (_f, m, p, secret) = setup();
            let c = CancellationToken::new();
            let path = p.config.with_file_name("passphrase.lock");
            let owner = fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create_new(true)
                .open(&path)
                .unwrap();
            owner.lock().unwrap();
            assert!(matches!(
                m.remember_passphrase(&p, secret.clone(), &c),
                Err(Error::Ssh(SshFailure::Changed))
            ));
            assert!(!p.config.with_file_name("passphrase.dpapi").exists());
            // A closed process handle releases the OS lock. A leftover empty file
            // must not permanently block the next attempt or need manual deletion.
            drop(owner);
            let saved = m.remember_passphrase(&p, secret, &c).unwrap();
            assert!(saved.passphrase_saved);
            m.forget_passphrase(&saved, &c).unwrap();
        },
    );
}
#[test]
#[cfg_attr(target_os = "linux", ignore = "needs an unlocked desktop keyring")]
fn encrypted_credentials_cannot_move_between_profiles_or_answer_host_prompts() {
    isolated(
        "ROCKET_SSH_DIRECT_CASE",
        "encrypted_credentials_cannot_move_between_profiles_or_answer_host_prompts",
        || {
            let (_f, m, p, secret) = setup();
            let c = CancellationToken::new();
            let p = m.remember_passphrase(&p, secret, &c).unwrap();
            let q = m
                .prepare("rocket-other", "github.com", "git", 22, Some(&p.key), &c)
                .unwrap()
                .finish(&c)
                .unwrap();
            fs::copy(
                p.config.with_file_name("passphrase.dpapi"),
                q.config.with_file_name("passphrase.dpapi"),
            )
            .unwrap();
            let q = m.load(&q.alias, &c).unwrap();
            let out = helper(&q, &["git@github.com", "git-upload-pack 'never-contact'"]);
            assert!(!out.status.success());
            assert!(out.stdout.is_empty());
            for prompt in [
                "The authenticity of host cannot be established. Continue?",
                "git@github.com's password:",
                "Enter passphrase for key 'C:/other/key':",
            ] {
                let out = quiet(
                    Command::new(env!("CARGO_BIN_EXE_rocket-ssh"))
                        .arg(prompt)
                        .env("ROCKET_SSH_PROFILE", p.config.parent().unwrap())
                        .env(
                            "ROCKET_SSH_CREDENTIAL",
                            p.config.with_file_name("passphrase.dpapi"),
                        ),
                )
                .output()
                .unwrap();
                assert!(!out.status.success());
                assert!(out.stdout.is_empty());
            }
        },
    );
}
