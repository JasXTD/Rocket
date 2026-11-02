//! Git for Windows bundles its own HTTP decoder independently of Rocket/curl.
use std::process::Command;

pub(crate) const WARNING: &str = "HTTP(S) Git transport is disabled for this Windows Git runtime. Install Git for Windows 2.56.0.windows.4 or later and restart Rocket. Local and SSH Git operations remain available.";

/// Fail closed for unrecognized Windows builds. The fourth Windows release of
/// 2.56.0 updates Brotli to 1.2.1 (CVE-2026-111497); the core version is not enough.
pub(crate) fn patched_windows_git(version: &str) -> bool {
    let Some(version) = version.strip_prefix("git version ") else {
        return false;
    };
    let Some((core, build)) = version.split_once(".windows.") else {
        return false;
    };
    let numbers: Option<Vec<u32>> = core
        .split('.')
        .map(|part| {
            (!part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()))
                .then(|| part.parse().ok())
                .flatten()
        })
        .collect();
    let Some(numbers) = numbers.filter(|n| n.len() == 3) else {
        return false;
    };
    let Some(build) = (!build.is_empty() && build.bytes().all(|b| b.is_ascii_digit()))
        .then(|| build.parse::<u32>().ok())
        .flatten()
        .filter(|b| *b > 0)
    else {
        return false;
    };
    let core = (numbers[0], numbers[1], numbers[2]);
    core > (2, 56, 0) || core == (2, 56, 0) && build >= 4
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
            "2.56.0.windows.4",
            "2.56.0.windows.5",
            "2.56.1.windows.1",
            "2.57.0.windows.1",
            "3.0.0.windows.1",
        ] {
            assert!(
                patched_windows_git(&format!("git version {version}")),
                "{version}"
            );
        }
        for version in [
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
                !patched_windows_git(&format!("git version {version}")),
                "{version}"
            );
        }
        assert!(!patched_windows_git("2.56.0.windows.4"));
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
