//! Local acceptance opt-in for the real CLI, with no account or AI service.
use super::{Policy, read_regular};
use rocket_ui::agents::model::Command;
use std::{ffi::OsString, path::Path};

const ARGUMENTS: &str = "--no-auto-update\n--no-custom-instructions\n--disable-builtin-mcps\n--disable-client-telemetry\n--deny-tool\nshell\nwrite\nread\nurl\nmemory";

impl Policy {
    /// MCP cannot create the consent marker, change the environment or select
    /// arbitrary programs/arguments. Input is restricted to /help and one fixed
    /// lifecycle prompt answered only by the required loopback mock provider.
    pub fn real_copilot_probe(&self, command: &Command) -> bool {
        self.copilot_probe(command, |key| std::env::var_os(key))
    }

    fn copilot_probe(&self, command: &Command, env: impl Fn(&str) -> Option<OsString>) -> bool {
        let value = |key| env(key).and_then(|v| v.into_string().ok());
        if command.arguments != ARGUMENTS
            || !read_regular(&self.root.join("rocket-agent-copilot"), 64)
                .is_ok_and(|s| s == "offline-copilot-lifecycle-v1\n")
            || value("COPILOT_OFFLINE").as_deref() != Some("true")
            || value("COPILOT_PROVIDER_TYPE").as_deref() != Some("openai")
            || value("COPILOT_MODEL").as_deref() != Some("rocket-lifecycle-fixture")
        {
            return false;
        }
        let endpoint = value("COPILOT_PROVIDER_BASE_URL").unwrap_or_default();
        if !endpoint
            .strip_prefix("http://127.0.0.1:")
            .and_then(|s| s.strip_suffix("/v1"))
            .and_then(|s| s.parse::<u16>().ok())
            .is_some_and(|p| p != 0)
        {
            return false;
        }
        for key in [
            "COPILOT_GITHUB_TOKEN",
            "GH_TOKEN",
            "GITHUB_TOKEN",
            "GH_ENTERPRISE_TOKEN",
            "GITHUB_ENTERPRISE_TOKEN",
            "COPILOT_PROVIDER_API_KEY",
            "COPILOT_PROVIDER_BEARER_TOKEN",
            "COPILOT_PROVIDER_API_KEY_COMMAND",
            "COPILOT_PROVIDERS_CONFIG",
            "NODE_OPTIONS",
            "BASH_ENV",
            "ENV",
        ] {
            if env(key).is_some() {
                return false;
            }
        }
        // These are per-process directories, never changes to system settings.
        for key in [
            "COPILOT_HOME",
            "HOME",
            "USERPROFILE",
            "APPDATA",
            "LOCALAPPDATA",
            "TEMP",
            "TMP",
        ] {
            let Some(path) = env(key).and_then(|p| Path::new(&p).canonicalize().ok()) else {
                return false;
            };
            if !path.starts_with(&self.root) || !path.is_dir() {
                return false;
            }
        }
        let expected = self.root.join("copilot-bin").join(if cfg!(windows) {
            "copilot.exe"
        } else {
            "copilot"
        });
        expected
            .canonicalize()
            .ok()
            .filter(|p| p.starts_with(&self.root) && p.is_file())
            .is_some_and(|p| Path::new(&command.program).canonicalize().ok().as_ref() == Some(&p))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{collections::BTreeMap, fs};

    #[test]
    fn copilot_probe_requires_consent_exact_argv_isolated_home_and_loopback() {
        let root = std::env::temp_dir().join(format!(
            "rocket-copilot-policy-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&root).unwrap();
        let p = Policy::new(&root).unwrap();
        fs::create_dir(root.join("copilot-bin")).unwrap();
        let program = root.join("copilot-bin").join(if cfg!(windows) {
            "copilot.exe"
        } else {
            "copilot"
        });
        fs::write(&program, "not executed").unwrap();
        let mut command = Command {
            program: program.to_string_lossy().into_owned(),
            arguments: ARGUMENTS.into(),
        };
        let mut env: BTreeMap<&str, OsString> = BTreeMap::new();
        for key in [
            "COPILOT_HOME",
            "HOME",
            "USERPROFILE",
            "APPDATA",
            "LOCALAPPDATA",
            "TEMP",
            "TMP",
        ] {
            env.insert(key, root.clone().into());
        }
        for (key, value) in [
            ("COPILOT_OFFLINE", "true"),
            ("COPILOT_PROVIDER_TYPE", "openai"),
            ("COPILOT_MODEL", "rocket-lifecycle-fixture"),
            ("COPILOT_PROVIDER_BASE_URL", "http://127.0.0.1:45000/v1"),
        ] {
            env.insert(key, value.into());
        }
        assert!(!p.copilot_probe(&command, |k| env.get(k).cloned()));
        fs::write(
            root.join("rocket-agent-copilot"),
            "offline-copilot-lifecycle-v1\n",
        )
        .unwrap();
        assert!(p.copilot_probe(&command, |k| env.get(k).cloned()));
        for (key, value) in [
            ("COPILOT_OFFLINE", "false"),
            ("COPILOT_PROVIDER_BASE_URL", "https://example.invalid/v1"),
            ("COPILOT_PROVIDER_BASE_URL", "http://127.0.0.1:0/v1"),
            ("COPILOT_HOME", "missing"),
            ("GH_TOKEN", "synthetic"),
            ("COPILOT_PROVIDERS_CONFIG", "other.json"),
            ("COPILOT_PROVIDER_API_KEY_COMMAND", "anything"),
        ] {
            assert!(
                !p.copilot_probe(&command, |k| if k == key {
                    Some(value.into())
                } else {
                    env.get(k).cloned()
                }),
                "{key}"
            );
        }
        for arguments in [
            "--no-auto-update",
            &format!("{ARGUMENTS}\n--allow-all"),
            &ARGUMENTS.replace("--deny-tool\nshell\nwrite\nread\nurl\nmemory", "--available-tools\nshell"),
        ] {
            command.arguments = arguments.into();
            assert!(!p.copilot_probe(&command, |k| env.get(k).cloned()));
        }
        command.arguments = ARGUMENTS.into();
        command.program = std::env::current_exe()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        assert!(!p.copilot_probe(&command, |k| env.get(k).cloned()));
        fs::remove_file(root.join("rocket-agent-copilot")).unwrap();
        fs::remove_file(program).unwrap();
        fs::remove_dir(root.join("copilot-bin")).unwrap();
        fs::remove_dir(root).unwrap();
    }
}
