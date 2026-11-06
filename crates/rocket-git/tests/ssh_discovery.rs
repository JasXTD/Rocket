#![cfg(any(windows, target_os = "linux"))]
#[allow(dead_code)]
mod common;
use common::*;
use rocket_git::*;
use std::{
    fs,
    path::{Path, PathBuf},
};

fn home(f: &Fixture) -> PathBuf {
    PathBuf::from(
        fs::canonicalize(&f.root)
            .unwrap()
            .to_string_lossy()
            .trim_start_matches(r"\\?\"),
    )
    .join("SSH 資料")
}
fn key(path: &Path) {
    let out = ssh_keygen()
    .args([
        "-q",
        "-t",
        "ed25519",
        "-N",
        "disposable-scan-passphrase",
        "-C",
        "ROCKET_PRIVATE_SCAN_COMMENT",
        "-f",
    ])
    .arg(path)
    .output()
    .unwrap();
    assert!(out.status.success(), "generate disposable key");
}
fn manager(root: &Path) -> SshManager {
    SshManager::new(root.join("rocket-keys"), None).unwrap()
}

#[test]
fn ssh_discovery_lists_public_pairs_without_creating_or_unlocking() {
    isolated(
        "ROCKET_SSH_SCAN_CASE",
        "ssh_discovery_lists_public_pairs_without_creating_or_unlocking",
        || {
            let f = Fixture::new();
            let root = home(&f);
            fs::create_dir(&root).unwrap();
            let key_path = root.join("工作.key");
            key(&key_path);
            let private = fs::read(&key_path).unwrap();
            let public = fs::read(key_path.with_file_name("工作.key.pub")).unwrap();
            fs::write(root.join("config"), "Match exec \"must-not-run\"\r\n").unwrap();
            fs::write(root.join("orphan.pub"), &public).unwrap();
            fs::write(root.join("invalid"), "private placeholder").unwrap();
            fs::write(root.join("invalid.pub"), "not a public key").unwrap();
            fs::create_dir(root.join("nested")).unwrap();
            fs::copy(&key_path, root.join("nested/hidden")).unwrap();
            fs::write(root.join("nested/hidden.pub"), &public).unwrap();
            let m = manager(&root);
            let c = CancellationToken::new();
            let found = m.discover_keys(&root, &c).unwrap();
            assert_eq!(found.keys.len(), 1);
            assert_eq!(found.skipped, 2);
            assert!(!found.truncated);
            assert_eq!(found.keys[0].key, key_path);
            assert_eq!(found.keys[0].kind, "ssh-ed25519");
            assert!(found.keys[0].fingerprint.starts_with("SHA256:"));
            assert!(!m.root().exists());
            assert_eq!(fs::read(&key_path).unwrap(), private);
            assert_eq!(
                fs::read(root.join("config")).unwrap(),
                b"Match exec \"must-not-run\"\r\n"
            );
            let profile = m
                .import_discovered(
                    &found.keys[0],
                    "rocket-discovered",
                    "github.com",
                    "git",
                    22,
                    &c,
                )
                .unwrap();
            assert!(profile.encrypted);
            assert_eq!(profile.key, key_path);
            assert_eq!(profile.fingerprint, found.keys[0].fingerprint);
            assert_eq!(fs::read(&key_path).unwrap(), private);
            assert_eq!(fs::read(root.join("工作.key.pub")).unwrap(), public);
            assert!(
                m.check_config(&profile, &c)
                    .unwrap()
                    .contains("IdentitiesOnly yes")
            );
        },
    );
}

#[test]
fn ssh_discovery_rejects_changed_pair_and_checks_private_payload_on_import() {
    isolated(
        "ROCKET_SSH_SCAN_CASE",
        "ssh_discovery_rejects_changed_pair_and_checks_private_payload_on_import",
        || {
            let f = Fixture::new();
            let root = home(&f);
            fs::create_dir(&root).unwrap();
            let p = root.join("id_ed25519");
            key(&p);
            let m = manager(&root);
            let c = CancellationToken::new();
            let found = m.discover_keys(&root, &c).unwrap();
            fs::write(
                root.join("id_ed25519.pub"),
                fs::read(root.join("id_ed25519.pub"))
                    .unwrap()
                    .into_iter()
                    .chain(*b"\n")
                    .collect::<Vec<_>>(),
            )
            .unwrap();
            assert!(matches!(
                m.import_discovered(&found.keys[0], "rocket-stale", "github.com", "git", 22, &c),
                Err(Error::Ssh(SshFailure::Changed))
            ));
            assert!(!m.root().exists());
            // Public-only discovery may list a file pair; the importer must verify the private payload.
            let public = found.keys[0].fingerprint.clone();
            let data = fs::read(root.join("id_ed25519.pub")).unwrap();
            fs::write(root.join("id_ed25519.pub"), &data[..data.len() - 1]).unwrap();
            fs::write(&p, "not a private key").unwrap();
            let found = m.discover_keys(&root, &c).unwrap();
            assert_eq!(found.keys[0].fingerprint, public);
            assert!(matches!(
                m.import_discovered(
                    &found.keys[0],
                    "rocket-invalid",
                    "github.com",
                    "git",
                    22,
                    &c
                ),
                Err(Error::Ssh(SshFailure::KeyInvalid))
            ));
            assert!(!m.root().exists());
            fs::write(&p, "externally changed private placeholder").unwrap();
            fs::remove_file(&p).unwrap();
            assert!(matches!(
                m.import_discovered(
                    &found.keys[0],
                    "rocket-missing",
                    "github.com",
                    "git",
                    22,
                    &c
                ),
                Err(Error::Ssh(SshFailure::Changed))
            ));
            fs::write(&p, "externally changed private placeholder").unwrap();
            assert!(matches!(
                m.import_discovered(&found.keys[0], "rocket-stale", "github.com", "git", 22, &c),
                Err(Error::Ssh(SshFailure::Changed))
            ));
        },
    );
}

#[test]
fn ssh_discovery_missing_folder_bounds_and_cancel_preserve_files() {
    isolated(
        "ROCKET_SSH_SCAN_CASE",
        "ssh_discovery_missing_folder_bounds_and_cancel_preserve_files",
        || {
            let f = Fixture::new();
            let root = home(&f);
            let m = manager(&root);
            let c = CancellationToken::new();
            assert!(m.discover_keys(&root, &c).unwrap().keys.is_empty());
            assert!(!root.exists());
            fs::create_dir(&root).unwrap();
            for i in 0..513 {
                fs::write(root.join(format!("unrelated-{i}")), b"keep").unwrap();
            }
            let found = m.discover_keys(&root, &c).unwrap();
            assert!(found.truncated);
            assert!(found.keys.is_empty());
            c.cancel();
            assert!(matches!(m.discover_keys(&root, &c), Err(Error::Cancelled)));
            assert_eq!(fs::read_dir(&root).unwrap().count(), 513);
            assert!(!m.root().exists());
            assert!(matches!(
                m.discover_keys(&root.join("../elsewhere"), &CancellationToken::new()),
                Err(Error::Ssh(SshFailure::UnsafePath))
            ));
        },
    );
}

/// A found key shows its public key without a profile, and is used (unlocked or
/// removed from the agent) only while its files are the ones the scan read.
#[test]
fn ssh_found_key_shows_its_public_key_and_is_held_while_unchanged() {
    isolated(
        "ROCKET_SSH_SCAN_CASE",
        "ssh_found_key_shows_its_public_key_and_is_held_while_unchanged",
        || {
            let f = Fixture::new();
            let root = home(&f);
            fs::create_dir(&root).unwrap();
            let p = root.join("id_ed25519");
            key(&p);
            let m = manager(&root);
            let c = CancellationToken::new();
            let found = m.discover_keys(&root, &c).unwrap();
            let public = fs::read_to_string(root.join("id_ed25519.pub")).unwrap();
            assert_eq!(found.keys[0].public_key(), public.trim());
            assert!(found.keys[0].public_key().starts_with("ssh-ed25519 "));
            let guard = m.guard_discovered(&found.keys[0], &c).unwrap();
            // Windows: the files cannot change while a key is being unlocked or removed.
            // Linux has no such lock; the recheck below finds a change instead.
            #[cfg(windows)]
            {
                assert!(fs::write(&p, "replacement").is_err());
                assert!(fs::remove_file(root.join("id_ed25519.pub")).is_err());
            }
            drop(guard);
            fs::write(
                root.join("id_ed25519.pub"),
                format!("{}\n\n", public.trim()),
            )
            .unwrap();
            assert!(matches!(
                m.guard_discovered(&found.keys[0], &c),
                Err(Error::Ssh(SshFailure::Changed))
            ));
            // Refused before ssh-agent is asked, so no agent is needed here.
            assert!(matches!(
                m.unload_discovered(&found.keys[0], &c),
                Err(Error::Ssh(SshFailure::Changed))
            ));
            assert!(!m.root().exists(), "nothing is created for a found key");
        },
    );
}

/// Linux: a found key other users can open is listed, so the page can say why, but it
/// is never handed to ssh-add or a profile; a key replaced after the scan is refused.
#[cfg(target_os = "linux")]
#[test]
fn linux_found_keys_other_users_can_open_are_listed_but_not_used() {
    isolated(
        "ROCKET_SSH_SCAN_CASE",
        "linux_found_keys_other_users_can_open_are_listed_but_not_used",
        || {
            use std::os::unix::fs::PermissionsExt;
            let f = Fixture::new();
            let root = home(&f);
            fs::create_dir(&root).unwrap();
            let p = root.join("id_ed25519");
            key(&p);
            fs::set_permissions(&p, fs::Permissions::from_mode(0o644)).unwrap();
            let m = manager(&root);
            let c = CancellationToken::new();
            let found = m.discover_keys(&root, &c).unwrap();
            assert_eq!(found.keys.len(), 1);
            assert!(matches!(m.guard_discovered(&found.keys[0], &c), Err(Error::Ssh(SshFailure::KeyPermissions))));
            assert!(matches!(m.unload_discovered(&found.keys[0], &c), Err(Error::Ssh(SshFailure::KeyPermissions))),
                "refused before ssh-agent is asked");
            assert!(matches!(
                m.import_discovered(&found.keys[0], "rocket-open", "github.com", "git", 22, &c),
                Err(Error::Ssh(SshFailure::KeyPermissions))
            ));
            assert!(!m.root().exists());
            assert_eq!(fs::metadata(&p).unwrap().permissions().mode() & 0o777, 0o644, "left as it was");
            // The same key, replaced by a copy after the scan, is another file.
            fs::set_permissions(&p, fs::Permissions::from_mode(0o600)).unwrap();
            let found = m.discover_keys(&root, &c).unwrap();
            let bytes = fs::read(&p).unwrap();
            fs::remove_file(&p).unwrap();
            fs::write(&p, &bytes).unwrap();
            fs::set_permissions(&p, fs::Permissions::from_mode(0o600)).unwrap();
            assert!(matches!(m.guard_discovered(&found.keys[0], &c), Err(Error::Ssh(SshFailure::Changed))));
        },
    );
}
