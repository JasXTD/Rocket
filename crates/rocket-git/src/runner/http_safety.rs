//! Git security release checks and transport restrictions during the version probe.
use std::process::Command;

pub(crate) const WINDOWS_MINIMUM: &str = "2.56.1.windows.1";
pub(crate) const UNIX_MINIMUM: &str = "2.50.2";
pub(crate) const WARNING: &str = "HTTP(S) Git transport is disabled for an unverified Git runtime.";

/// Return the required version for an unsupported installation's maintenance series.
pub(crate) fn check_version(output: &str, windows: bool) -> Result<(), &'static str> {
    let version = output.strip_prefix("git version ");
    if windows {
        let supported = version
            .and_then(windows_build)
            .zip(windows_build(WINDOWS_MINIMUM))
            .is_some_and(|(found, minimum)| found >= minimum);
        return supported.then_some(()).ok_or(WINDOWS_MINIMUM);
    }

    // GHSA-vp2w-7xxj-jvxr was fixed on each maintenance branch separately.
    match version.and_then(unix_release) {
        Some([2, 51, 0..=2]) => Err("2.51.3"),
        Some([2, 52, 0]) => Err("2.52.1"),
        Some([2, 53, 0]) => Err("2.53.1"),
        Some([2, 54, 0]) => Err("2.54.1"),
        Some([2, 55, 0]) => Err("2.55.1"),
        Some([2, 56, 0]) | None => Err("2.56.1"),
        Some(found) if found >= [2, 50, 2] => Ok(()),
        Some(_) => Err(UNIX_MINIMUM),
    }
}

fn number(part: &str) -> Option<u32> {
    if !part.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    part.parse().ok()
}

fn release_numbers(version: &str) -> Option<[u32; 3]> {
    let mut parts = version.split('.');
    let core = [
        number(parts.next()?)?,
        number(parts.next()?)?,
        number(parts.next()?)?,
    ];
    parts.next().is_none().then_some(core)
}

fn windows_build(version: &str) -> Option<[u32; 4]> {
    let (core, build) = version.split_once(".windows.")?;
    let [major, minor, patch] = release_numbers(core)?;
    let build = number(build)?;
    (build > 0).then_some([major, minor, patch, build])
}

fn unix_release(version: &str) -> Option<[u32; 3]> {
    // Apple's release builds append a numeric vendor identifier after the core version.
    let core = if let Some((core, build)) = version.split_once(" (Apple Git-") {
        if !build
            .strip_suffix(')')?
            .split('.')
            .all(|part| number(part).is_some())
        {
            return None;
        }
        core
    } else {
        version
    };
    release_numbers(core)
}

/// Apply last, after clone/scoped policy. This environment allowlist reaches
/// child Git processes and wins over URL rewrites and protocol.*.allow config.
/// Unknown remote helpers are excluded too: they could invoke the same decoder.
pub(crate) fn restrict(command: &mut Command) {
    let inherited = command
        .get_envs()
        .find(|(key, _)| *key == "GIT_ALLOW_PROTOCOL")
        .map(|(_, value)| value.map(|v| v.to_owned()))
        .unwrap_or_else(|| std::env::var_os("GIT_ALLOW_PROTOCOL"));
    let allowed = ["file", "ssh"]
        .into_iter()
        .filter(|protocol| {
            inherited
                .as_ref()
                .is_none_or(|value| value.to_string_lossy().split(':').any(|p| p == *protocol))
        })
        .collect::<Vec<_>>()
        .join(":");
    command.env("GIT_ALLOW_PROTOCOL", allowed);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_security_release_requires_the_complete_build_version() {
        for version in [
            "2.56.1.windows.1",
            "2.56.1.windows.2",
            "2.57.0.windows.1",
            "3.0.0.windows.1",
        ] {
            assert!(
                check_version(&format!("git version {version}"), true).is_ok(),
                "{version}"
            );
        }
        for version in [
            "2.56.0.windows.4",
            "2.56.0.windows.99",
            "2.56.1",
            "2.56.1.windows.1.dirty",
            "2.56.0.windows.3",
            "2.56.0.windows.1",
            "2.55.0.windows.9",
            "2.46.0.windows.1",
            "2.56.0",
            "2.57.0",
            "2.56.0.windows.4.dirty",
            "2.56.0.windows.",
            "2.57.0.windows.0",
            "2.57.0.windows.+1",
            "2.57.windows.1",
            "2.57.0.rc1.windows.1",
            "2.57.0.windows.4294967296",
        ] {
            assert!(
                check_version(&format!("git version {version}"), true).is_err(),
                "{version}"
            );
        }
        assert!(check_version("2.56.1.windows.1", true).is_err());
    }

    #[test]
    fn unix_security_baselines_follow_each_upstream_maintenance_series() {
        for (old, fixed) in [
            ("2.50.1", "2.50.2"),
            ("2.51.2", "2.51.3"),
            ("2.52.0", "2.52.1"),
            ("2.53.0", "2.53.1"),
            ("2.54.0", "2.54.1"),
            ("2.55.0", "2.55.1"),
            ("2.56.0", "2.56.1"),
        ] {
            for suffix in ["", " (Apple Git-155.1)"] {
                assert_eq!(
                    check_version(&format!("git version {old}{suffix}"), false),
                    Err(fixed)
                );
                assert_eq!(
                    check_version(&format!("git version {fixed}{suffix}"), false),
                    Ok(())
                );
            }
        }
        for version in ["2.50.10", "2.51.10", "2.56.2", "2.57.0", "3.0.0"] {
            assert!(
                check_version(&format!("git version {version}"), false).is_ok(),
                "{version}"
            );
        }
        for version in [
            "2.46.0",
            "2.49.99",
            "2.51.0",
            "2.51.1",
            "2.56",
            "2.56.1-rc0",
            "2.57.0.rc1",
            "2.57.0.1.gabcdef",
            "2.56.1.windows.1",
            "2.56.+1",
            "2.56.1 (Apple Git-)",
            "2.56.1 (Apple Git-155) extra",
            "4294967296.0.0",
        ] {
            assert!(
                check_version(&format!("git version {version}"), false).is_err(),
                "{version}"
            );
        }
        assert!(check_version("2.56.1", false).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn discovery_checks_the_selected_unix_git_before_use() {
        use crate::{CancellationToken, CommandLimits, Error, GitClient};
        use std::{fs, os::unix::fs::PermissionsExt};

        let dir =
            crate::test_fixture_root::base().join(format!("git-version-{}", std::process::id()));
        fs::create_dir(&dir).unwrap();
        let git = dir.join("git");
        for (version, allowed) in [("2.51.2", false), ("2.51.3 (Apple Git-155)", true)] {
            let output = format!("git version {version}");
            fs::write(&git, format!(
                "#!/bin/sh\nfor arg do\n  if [ \"$arg\" = --version ]; then echo '{output}'; exit 0; fi\ndone\nexit 91\n"
            )).unwrap();
            fs::set_permissions(&git, fs::Permissions::from_mode(0o700)).unwrap();
            let client = GitClient::discover_with_options(
                Some(&git),
                CommandLimits::default(),
                None,
                &CancellationToken::new(),
            );
            if allowed {
                assert_eq!(client.unwrap().version(), output);
            } else {
                assert!(
                    matches!(client, Err(Error::Unsupported(reason)) if reason.contains("Git 2.51.3"))
                );
            }
        }
        fs::remove_file(git).unwrap();
        fs::remove_dir(dir).unwrap();
    }

    #[test]
    fn runtime_policy_only_narrows_an_existing_allowlist() {
        for (before, after) in [
            ("https:ssh:file", "file:ssh"),
            ("file", "file"),
            ("ssh", "ssh"),
            ("https", ""),
            ("ext:git:http", ""),
            ("", ""),
        ] {
            let mut command = Command::new("unused");
            command.env("GIT_ALLOW_PROTOCOL", before);
            restrict(&mut command);
            let actual = command
                .get_envs()
                .find(|(key, _)| *key == "GIT_ALLOW_PROTOCOL")
                .unwrap()
                .1
                .unwrap();
            assert_eq!(actual, after);
        }
    }
}
