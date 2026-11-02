#[allow(dead_code)]
mod common;
use common::{Fixture, isolated};
use rocket_git::*;

#[test]
#[cfg(windows)]
fn trusted_agent_profile_keeps_repository_command_and_proxy_refusals() {
    isolated(
        "ROCKET_TRANSPORT_SCOPE_CASE",
        "trusted_agent_profile_keeps_repository_command_and_proxy_refusals",
        || {
            let f = Fixture::new();
            f.write("file", "fixture");
            f.commit("fixture");
            let bare = f.root.join("allowed.git");
            f.run(&["init", "--bare", bare.to_str().unwrap()]);
            f.run(&["remote", "add", "origin", bare.to_str().unwrap()]);
            let public = f.root.join("identity.pub");
            std::fs::write(&public, "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOMqqnkVzrm0SdG6UOoqKLsabgH5C9okWi0dh2l9GKJl\n").unwrap();
            let scope = TransportScope::new(&f.root, vec![])
                .unwrap()
                .with_windows_github_agent(&public.canonicalize().unwrap())
                .unwrap();
            let repo = f
                .git
                .clone()
                .with_transport_scope(Some(scope))
                .open(&f.repository, &CancellationToken::new())
                .unwrap();
            for (key, value) in [
                ("core.sshCommand", "not-a-command"),
                ("credential.helper", "!exit 99"),
                ("http.proxy", "http://127.0.0.1:1"),
                ("http.sslVerify", "false"),
            ] {
                f.run(&["config", key, value]);
                let report = repo
                    .push(
                        "origin",
                        "main",
                        "main",
                        PushOptions::default(),
                        &CancellationToken::new(),
                    )
                    .unwrap();
                assert!(matches!(report.result, Err(Error::InvalidInput(_))));
                assert_eq!(report.remote_outcome, PushRemoteOutcome::NotAttempted);
                assert!(!bare.join("refs/heads/main").exists());
                f.run(&["config", "--unset-all", key]);
            }
            let report = repo
                .push(
                    "origin",
                    "main",
                    "main",
                    PushOptions::default(),
                    &CancellationToken::new(),
                )
                .unwrap();
            assert!(report.result.is_ok(), "{:?}", report.result);
            assert_eq!(report.remote_outcome, PushRemoteOutcome::Confirmed);
        },
    );
}

#[test]
fn scoped_transport_refuses_authentication_commands_without_executing_them() {
    isolated(
        "ROCKET_TRANSPORT_SCOPE_CASE",
        "scoped_transport_refuses_authentication_commands_without_executing_them",
        || {
            let f = Fixture::new();
            f.write("file", "fixture");
            f.commit("fixture");
            let bare = f.root.join("allowed.git");
            f.run(&["init", "--bare", bare.to_str().unwrap()]);
            f.run(&["remote", "add", "origin", bare.to_str().unwrap()]);
            let client = f
                .git
                .clone()
                .with_transport_scope(Some(TransportScope::new(&f.root, vec![]).unwrap()));
            let c = CancellationToken::new();
            let repo = client.open(&f.repository, &c).unwrap();
            for key in [
                "core.sshCommand",
                "core.askPass",
                "core.gitProxy",
                "credential.helper",
                "credential.https://example.invalid.helper",
            ] {
                f.run(&["config", key, "!exit 99"]);
                assert!(
                    matches!(
                        repo.fetch("origin", None, &c).unwrap().result,
                        Err(Error::InvalidInput(_))
                    ),
                    "accepted {key}"
                );
                let push = repo
                    .push("origin", "main", "main", PushOptions::default(), &c)
                    .unwrap();
                assert_eq!(push.remote_outcome, PushRemoteOutcome::NotAttempted);
                assert!(
                    matches!(push.result, Err(Error::InvalidInput(_))),
                    "accepted {key}"
                );
                assert!(
                    matches!(
                        repo.fetch_all_quietly(FetchTags::None, &c).unwrap().results[0].1,
                        Err(Error::InvalidInput(_))
                    ),
                    "accepted {key}"
                );
                assert!(
                    matches!(
                        repo.check_remote(bare.to_str().unwrap().as_bytes(), &c),
                        Err(Error::InvalidInput(_))
                    ),
                    "accepted {key}"
                );
                f.run(&["config", "--unset-all", key]);
            }
            f.run(&["config", "credential.helper", ""]);
            assert!(repo.fetch("origin", None, &c).unwrap().result.is_ok());
            let include = f.root.join("commands.config");
            std::fs::write(
                &include,
                "[credential \"https://example.invalid\"]\n helper = !exit 99\n",
            )
            .unwrap();
            f.run(&["config", "include.path", include.to_str().unwrap()]);
            assert!(matches!(
                repo.fetch("origin", None, &c).unwrap().result,
                Err(Error::InvalidInput(_))
            ));
            f.run(&["config", "--unset-all", "include.path"]);
            // A new disposable config file is used only by this isolated child.
            // Never write to the path supplied by the parent process environment.
            let clone_config = f.root.join("clone-auth.config");
            std::fs::write(&clone_config, "[core]\n askPass = never-execute-this\n").unwrap();
            let previous = std::env::var_os("GIT_CONFIG_GLOBAL");
            unsafe {
                std::env::set_var("GIT_CONFIG_GLOBAL", &clone_config);
            }
            let rejected = client.clone_repository(
                &CloneSource::Local(f.repository.clone()),
                f.root.join("refused-auth-clone"),
                None,
                &c,
            );
            unsafe {
                match previous {
                    Some(value) => std::env::set_var("GIT_CONFIG_GLOBAL", value),
                    None => std::env::remove_var("GIT_CONFIG_GLOBAL"),
                }
            }
            assert!(matches!(
                rejected.unwrap().command,
                Err(Error::InvalidInput(_))
            ));
        },
    );
}

#[test]
fn scoped_transport_does_not_execute_an_inherited_ssh_command() {
    isolated(
        "ROCKET_TRANSPORT_SCOPE_CASE",
        "scoped_transport_does_not_execute_an_inherited_ssh_command",
        || {
            let f = Fixture::new();
            let source = "ssh://127.0.0.1:1/never-contacted.git";
            f.run(&["remote", "add", "origin", source]);
            let marker = f.root.join("unexpected-command");
            let command = format!(
                "printf fixture > '{}'; exit 1",
                marker
                    .to_string_lossy()
                    .replace('\\', "/")
                    .replace('\'', "'\\''")
            );
            // This exact test runs in its own process. The inert command exits before SSH/network.
            unsafe {
                std::env::set_var("GIT_ALLOW_PROTOCOL", "ssh");
            }
            let client = f.git.clone().with_transport_scope(Some(
                TransportScope::new(&f.root, vec![source.into()]).unwrap(),
            ));
            let c = CancellationToken::new();
            let repo = client.open(&f.repository, &c).unwrap();
            for key in [
                "GIT_SSH_COMMAND",
                "GIT_SSH",
                "GIT_SSH_VARIANT",
                "GIT_ASKPASS",
                "SSH_ASKPASS",
                "GIT_EXEC_PATH",
                "GIT_PROXY_COMMAND",
            ] {
                unsafe {
                    std::env::set_var(key, &command);
                }
                let result = repo.fetch("origin", None, &c).unwrap().result;
                unsafe {
                    std::env::remove_var(key);
                }
                assert!(!marker.exists(), "inherited command executed via {key}");
                assert!(
                    matches!(result, Err(Error::InvalidInput(_))),
                    "{key}: {result:?}"
                );
            }
        },
    );
}

#[test]
fn scoped_transports_recheck_rewrites_for_fetch_push_and_background_fetch() {
    isolated(
        "ROCKET_TRANSPORT_SCOPE_CASE",
        "scoped_transports_recheck_rewrites_for_fetch_push_and_background_fetch",
        || {
            let f = Fixture::new();
            f.write("file", "fixture");
            f.commit("fixture");
            let bare = f.root.join("allowed.git");
            f.run(&["init", "--bare", bare.to_str().unwrap()]);
            f.run(&["remote", "add", "origin", bare.to_str().unwrap()]);
            let client = f
                .git
                .clone()
                .with_transport_scope(Some(TransportScope::new(&f.root, vec![]).unwrap()));
            let c = CancellationToken::new();
            let repo = client.open(&f.repository, &c).unwrap();
            assert!(
                repo.push("origin", "main", "main", PushOptions::default(), &c)
                    .unwrap()
                    .result
                    .is_ok()
            );
            assert!(repo.fetch("origin", None, &c).unwrap().result.is_ok());
            // Changes after the repository was opened must be checked at execution.
            for suffix in ["insteadOf", "pushInsteadOf"] {
                let key = format!("url.https://example.invalid/never-contacted.{suffix}");
                f.run(&["config", &key, bare.to_str().unwrap()]);
                assert!(matches!(
                    repo.fetch("origin", None, &c).unwrap().result,
                    Err(Error::InvalidInput(_))
                ));
                assert!(matches!(
                    repo.push("origin", "main", "main", PushOptions::default(), &c)
                        .unwrap()
                        .result,
                    Err(Error::InvalidInput(_))
                ));
                let quiet = repo.fetch_all_quietly(FetchTags::None, &c).unwrap();
                assert_eq!(quiet.results.len(), 1);
                assert!(matches!(&quiet.results[0].1, Err(Error::InvalidInput(_))));
                f.run(&["config", "--unset-all", &key]);
            }
            assert!(repo.fetch("origin", None, &c).unwrap().result.is_ok());
        },
    );
}

#[test]
fn scoped_transports_refuse_outside_pushurls_helpers_and_included_rewrites() {
    isolated(
        "ROCKET_TRANSPORT_SCOPE_CASE",
        "scoped_transports_refuse_outside_pushurls_helpers_and_included_rewrites",
        || {
            let f = Fixture::new();
            let outside = Fixture::new();
            f.write("file", "fixture");
            f.commit("fixture");
            let bare = f.root.join("allowed.git");
            f.run(&["init", "--bare", bare.to_str().unwrap()]);
            f.run(&["remote", "add", "origin", bare.to_str().unwrap()]);
            let client = f
                .git
                .clone()
                .with_transport_scope(Some(TransportScope::new(&f.root, vec![]).unwrap()));
            let c = CancellationToken::new();
            let repo = client.open(&f.repository, &c).unwrap();
            f.run(&[
                "config",
                "remote.origin.pushurl",
                outside.repository.to_str().unwrap(),
            ]);
            assert!(matches!(
                repo.push("origin", "main", "main", PushOptions::default(), &c)
                    .unwrap()
                    .result,
                Err(Error::InvalidInput(_))
            ));
            f.run(&["config", "--unset-all", "remote.origin.pushurl"]);
            f.run(&["config", "remote.origin.vcs", "never-launch-this-helper"]);
            assert!(matches!(
                repo.fetch("origin", None, &c).unwrap().result,
                Err(Error::InvalidInput(_))
            ));
            f.run(&["config", "--unset-all", "remote.origin.vcs"]);
            let included = f.root.join("included.config");
            std::fs::write(
                &included,
                "[url \"https://example.invalid/never-contacted\"]\n insteadOf = approved\n",
            )
            .unwrap();
            f.run(&["config", "include.path", included.to_str().unwrap()]);
            assert!(matches!(
                repo.fetch("origin", None, &c).unwrap().result,
                Err(Error::InvalidInput(_))
            ));
            assert!(matches!(
                repo.check_remote(bare.to_str().unwrap().as_bytes(), &c),
                Err(Error::InvalidInput(_))
            ));
            f.run(&["config", "--unset-all", "include.path"]);
            assert!(
                repo.check_remote(bare.to_str().unwrap().as_bytes(), &c)
                    .is_ok()
            );
            assert!(matches!(
                repo.check_remote(outside.repository.to_str().unwrap().as_bytes(), &c),
                Err(Error::InvalidInput(_))
            ));
        },
    );
}

#[test]
#[cfg(windows)]
fn scoped_clone_accepts_local_sources_and_refuses_external_git_directories() {
    isolated(
        "ROCKET_TRANSPORT_SCOPE_CASE",
        "scoped_clone_accepts_local_sources_and_refuses_external_git_directories",
        || {
            let f = Fixture::new();
            f.write("file", "fixture");
            f.commit("fixture");
            let outside = Fixture::new();
            outside.write("file", "outside fixture");
            outside.commit("fixture");
            let client = f
                .git
                .clone()
                .with_transport_scope(Some(TransportScope::new(&f.root, vec![]).unwrap()));
            let c = CancellationToken::new();
            let destination = f.root.join("accepted-clone");
            let accepted = client
                .clone_repository(
                    &CloneSource::Local(f.repository.clone()),
                    &destination,
                    None,
                    &c,
                )
                .unwrap();
            assert!(accepted.command.is_ok(), "{:?}", accepted.command);
            assert!(matches!(accepted.publication, Some(Ok(()))));
            assert_eq!(
                std::fs::read_to_string(destination.join("file")).unwrap(),
                "fixture"
            );

            // The worktree path is in scope, but its administration lives outside.
            let worktree = f.root.join("external-admin-worktree");
            outside.run(&[
                "worktree",
                "add",
                "--detach",
                worktree.to_str().unwrap(),
                "HEAD",
            ]);
            for (index, source) in [outside.repository.clone(), worktree]
                .into_iter()
                .enumerate()
            {
                let destination = f.root.join(format!("refused-clone-{index}"));
                let refused = client
                    .clone_repository(&CloneSource::Local(source), &destination, None, &c)
                    .unwrap();
                assert!(
                    matches!(refused.command, Err(Error::InvalidInput(_))),
                    "{:?}",
                    refused.command
                );
                assert!(!destination.exists());
            }
        },
    );
}
