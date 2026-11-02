#[allow(dead_code, reason = "This target uses only part of the shared integration fixture")]
mod common;
use common::*;
use rocket_git::*;
use std::{fs, path::PathBuf};

#[test]
fn global_identity_preserves_includes_comments_and_repository_overrides() {
    isolated(
        "ROCKET_GLOBAL_CASE",
        "global_identity_preserves_includes_comments_and_repository_overrides",
        || {
            let f = Fixture::new();
            let path = PathBuf::from(std::env::var_os("GIT_CONFIG_GLOBAL").unwrap());
            let include = path.with_file_name("identity-included.config");
            fs::write(
                &include,
                "[user]\n name = Inherited\n email = inherited@example.invalid\n",
            )
            .unwrap();
            let include_text = include.to_string_lossy().replace('\\', "/");
            let original = format!(
                "# preserve 文\n[include]\n path = \"{include_text}\"\n[core]\n autocrlf = false\n[user]\n name = Before\n email = before@example.invalid\n"
            );
            fs::write(&path, &original).unwrap();
            let repo_config = fs::read(f.repository.join(".git/config")).unwrap();
            let token = CancellationToken::new();
            let expected = f.git.global_identity(&token).unwrap();
            assert_eq!(
                expected.path.canonicalize().unwrap(),
                path.canonicalize().unwrap()
            );
            assert_eq!(expected.direct.name.as_deref(), Some("Before"));
            assert!(!expected.origins.is_empty());
            let new = LocalIdentity {
                name: Some("新しい 名前".into()),
                email: Some("new@example.invalid".into()),
            };
            f.git.set_global_identity(&expected, &new, &token).unwrap();
            let saved = f.git.global_identity(&token).unwrap();
            assert_eq!(saved.direct, new);
            assert_eq!(saved.effective, new);
            let bytes = fs::read_to_string(&path).unwrap();
            assert!(bytes.contains("# preserve 文"));
            assert!(bytes.contains("autocrlf = false"));
            assert!(bytes.contains(&include_text));
            assert_eq!(
                fs::read(f.repository.join(".git/config")).unwrap(),
                repo_config
            );
            f.git
                .set_global_identity(&saved, &LocalIdentity::default(), &token)
                .unwrap();
            let cleared = f.git.global_identity(&token).unwrap();
            assert_eq!(cleared.direct, LocalIdentity::default());
            assert_eq!(cleared.effective.name.as_deref(), Some("Inherited"));
            assert_eq!(
                fs::read_to_string(include).unwrap(),
                "[user]\n name = Inherited\n email = inherited@example.invalid\n"
            );
        },
    );
}

#[test]
fn global_identity_rejects_stale_locked_invalid_and_cancelled_writes() {
    isolated(
        "ROCKET_GLOBAL_CASE",
        "global_identity_rejects_stale_locked_invalid_and_cancelled_writes",
        || {
            let f = Fixture::new();
            let path = PathBuf::from(std::env::var_os("GIT_CONFIG_GLOBAL").unwrap());
            fs::write(&path, "# original\n").unwrap();
            let token = CancellationToken::new();
            let expected = f.git.global_identity(&token).unwrap();
            let value = LocalIdentity {
                name: Some("Fixture".into()),
                email: Some("fixture@example.invalid".into()),
            };
            fs::write(&path, "# external\n").unwrap();
            assert!(
                f.git
                    .set_global_identity(&expected, &value, &token)
                    .is_err()
            );
            assert_eq!(fs::read(&path).unwrap(), b"# external\n");
            let expected = f.git.global_identity(&token).unwrap();
            let cancelled = CancellationToken::new();
            cancelled.cancel();
            assert!(matches!(
                f.git.set_global_identity(&expected, &value, &cancelled),
                Err(Error::Cancelled)
            ));
            let mut lock = path.as_os_str().to_owned();
            lock.push(".lock");
            fs::write(&lock, "owned elsewhere").unwrap();
            assert!(
                f.git
                    .set_global_identity(&expected, &value, &token)
                    .is_err()
            );
            assert_eq!(fs::read(&lock).unwrap(), b"owned elsewhere");
            fs::remove_file(&lock).unwrap();
            let invalid = LocalIdentity {
                name: Some("bad\nvalue".into()),
                email: None,
            };
            assert!(
                f.git
                    .set_global_identity(&expected, &invalid, &token)
                    .is_err()
            );
            assert_eq!(fs::read(&path).unwrap(), b"# external\n");
            assert!(!PathBuf::from(lock).exists());
        },
    );
}

#[test]
fn global_identity_creates_only_the_reviewed_missing_file() {
    isolated(
        "ROCKET_GLOBAL_CASE",
        "global_identity_creates_only_the_reviewed_missing_file",
        || {
            let f = Fixture::new();
            let path = PathBuf::from(std::env::var_os("GIT_CONFIG_GLOBAL").unwrap());
            fs::remove_file(&path).unwrap();
            let token = CancellationToken::new();
            let observed = f.git.global_identity(&token).unwrap();
            assert_eq!(
                observed.path.parent().unwrap().canonicalize().unwrap(),
                path.parent().unwrap().canonicalize().unwrap()
            );
            assert_eq!(observed.path.file_name(), path.file_name());
            f.git
                .set_global_identity(
                    &observed,
                    &LocalIdentity {
                        name: Some("New".into()),
                        email: None,
                    },
                    &token,
                )
                .unwrap();
            assert_eq!(
                f.git
                    .global_identity(&token)
                    .unwrap()
                    .effective
                    .name
                    .as_deref(),
                Some("New")
            );
        },
    );
}
