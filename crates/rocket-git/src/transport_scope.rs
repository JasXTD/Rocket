//! Execution-time restrictions for the disposable MCP test process.
mod windows_ssh;
use crate::{
    CancellationToken, Error, Result,
    runner::{Runner, args},
};
use std::{
    ffi::{OsStr, OsString},
    path::{Path, PathBuf},
};

// These values select executable commands, not authentication data. A scoped
// fixture must not inherit a shell/credential launcher from its parent process.
pub(crate) const COMMAND_ENV: &[&str] = &[
    "GIT_SSH",
    "GIT_SSH_COMMAND",
    "GIT_SSH_VARIANT",
    "GIT_ASKPASS",
    "SSH_ASKPASS",
    "GIT_PROXY_COMMAND",
    "GIT_EXEC_PATH",
];

// Apply after caller config options, and propagate to LFS/child Git processes.
// Git documents /dev/null as its per-command hook-disable setting; unlike an
// empty directory in a fixture it cannot acquire a hook from repository content.
pub(crate) const PROCESS_CONFIG: &[(&str, &str)] = &[
    ("core.hooksPath", "/dev/null"),
    ("http.proxy", ""),
    ("http.sslVerify", "true"),
    ("http.followRedirects", "false"),
    ("http.curloptResolve", ""),
];

pub(crate) fn network_environment(key: &OsStr) -> bool {
    let name = key.to_string_lossy().to_ascii_uppercase();
    name.starts_with("GIT_SSL_")
        || name.starts_with("GIT_PROXY_SSL_")
        || matches!(
            name.as_str(),
            "HTTP_PROXY"
                | "HTTPS_PROXY"
                | "ALL_PROXY"
                | "NO_PROXY"
                | "GIT_HTTP_PROXY_AUTHMETHOD"
                | "CURL_CA_BUNDLE"
                | "SSL_CERT_FILE"
                | "SSL_CERT_DIR"
                | "SSLKEYLOGFILE"
                | "OPENSSL_CONF"
                | "OPENSSL_MODULES"
        )
}

// Counts the global options reviewed commands put before the Git verb. Add new ones
// here: validate refuses any other, so an unlisted flag can't pass as the verb.
pub(crate) fn config_prefix(command: &[OsString]) -> usize {
    let mut position = 0;
    while position < command.len() {
        let option = command[position].to_string_lossy();
        if matches!(&*option, "-c" | "--git-dir" | "--work-tree") && position + 1 < command.len() {
            position += 2;
        } else if option.starts_with("--config-env=")
            || matches!(&*option, "--no-replace-objects" | "--no-literal-pathspecs")
        {
            position += 1;
        } else {
            break;
        }
    }
    position
}

fn network_configuration(key: &str, value: Option<&[u8]>) -> bool {
    let setting = key.rsplit('.').next().unwrap_or(key);
    let empty = value == Some(b"");
    if key.starts_with("remote.") && setting == "proxy" {
        return !empty;
    }
    if !key.starts_with("http.") {
        return false;
    }
    let boolean = value.map(|v| String::from_utf8_lossy(v).to_ascii_lowercase());
    match setting {
        "sslverify" => !matches!(boolean.as_deref(), None | Some("true" | "yes" | "on" | "1")),
        "followredirects" => !matches!(boolean.as_deref(), Some("false" | "no" | "off" | "0" | "")),
        "proxy" | "curloptresolve" => !empty,
        _ => {
            setting.starts_with("ssl")
                || setting.starts_with("schannel")
                || setting.starts_with("proxy")
        }
    }
}

#[derive(Clone, Debug)]
pub struct TransportScope {
    root: PathBuf,
    network_urls: Vec<String>,
    windows_ssh: Option<std::sync::Arc<windows_ssh::WindowsSsh>>,
}

impl TransportScope {
    pub fn new(root: &Path, network_urls: Vec<String>) -> Result<Self> {
        let root = root
            .canonicalize()
            .map_err(|e| Error::io("resolve transport scope", e))?;
        if !root.is_dir()
            || network_urls.iter().any(|url| {
                url.chars().any(char::is_control)
                    || !(url.starts_with("https://")
                        || url.starts_with("ssh://")
                        || url.starts_with("git@"))
            })
        {
            return refused();
        }
        Ok(Self { root, network_urls, windows_ssh: None })
    }

    /// Explicit local-operator opt-in; accepts only an in-scope public key. The
    /// Windows service retains private keys. Repository SSH overrides still fail.
    pub fn with_windows_github_agent(mut self, public_key: &Path) -> Result<Self> {
        self.windows_ssh = Some(std::sync::Arc::new(windows_ssh::WindowsSsh::new(&self.root, &self.network_urls, public_key)?));
        Ok(self)
    }

    pub(crate) fn configure_ssh(&self, command: &mut std::process::Command) {
        if let Some(profile) = &self.windows_ssh { profile.configure(command); }
    }

    fn url(
        &self,
        runner: &Runner,
        value: &OsStr,
        receiving: bool,
        cancel: &CancellationToken,
    ) -> Result<()> {
        let value = value.to_str().ok_or_else(scope_error)?;
        if value.is_empty() || value.chars().any(char::is_control) {
            return refused();
        }
        if self.network_urls.iter().any(|allowed| allowed == value) {
            return Ok(());
        }
        let path = Path::new(value);
        if !path.is_absolute() {
            return refused();
        }
        let resolved = path.canonicalize().map_err(|_| scope_error())?;
        if !resolved.starts_with(&self.root) || !resolved.is_dir() {
            return refused();
        }
        // Verify the resolved destination and its Git administration directories;
        // an in-scope worktree can otherwise point at an out-of-scope repository.
        let _guards = crate::local_files::pin_directories(&self.root, &resolved)?;
        let repository = crate::GitClient::from_runner(runner.clone()).open(&resolved, cancel)?;
        if !repository.info().git_dir.starts_with(&self.root)
            || !repository.info().common_dir.starts_with(&self.root)
        {
            return refused();
        }
        if receiving {
            self.local_receiver(runner, &repository, cancel)?;
        }
        Ok(())
    }

    fn local_receiver(
        &self,
        runner: &Runner,
        repository: &crate::Repository,
        cancel: &CancellationToken,
    ) -> Result<()> {
        // Git deliberately clears -c/GIT_CONFIG_COUNT before a local receive-pack.
        // Never mutate that repository's hooks; refuse an active receiving hook.
        let info = repository.info();
        let output = runner.run(
            &info.git_dir,
            &args(&["config", "--path", "--get", "core.hooksPath"]),
            vec![],
            cancel,
        )?;
        let hooks = if output.status.success() {
            let value = std::str::from_utf8(crate::parse::line(&output.stdout))
                .map_err(|_| scope_error())?;
            if value == "/dev/null" {
                return Ok(());
            }
            let path = Path::new(value);
            if path.is_absolute() {
                path.to_path_buf()
            } else {
                info.git_dir.join(path)
            }
        } else if output.status.code() == Some(1) {
            info.common_dir.join("hooks")
        } else {
            return refused();
        };
        let problem = || {
            Error::InvalidInput(
                "MCP local push requires a fixture receiver without active hooks".into(),
            )
        };
        let parent = hooks
            .parent()
            .ok_or_else(problem)?
            .canonicalize()
            .map_err(|_| problem())?;
        if !parent.starts_with(&self.root) {
            return Err(problem());
        }
        let _guards = crate::local_files::pin_directories(&self.root, &parent)?;
        let metadata = match std::fs::symlink_metadata(&hooks) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(_) => return Err(problem()),
        };
        if !metadata.is_dir() || crate::local_files::redirected(&metadata) {
            return Err(problem());
        }
        let resolved = hooks.canonicalize().map_err(|_| problem())?;
        let _hook_guards = crate::local_files::pin_directories(&self.root, &resolved)?;
        for (index, item) in std::fs::read_dir(&resolved)
            .map_err(|_| problem())?
            .enumerate()
        {
            cancel.check()?;
            let item = item.map_err(|_| problem())?;
            if index >= 128 || !item.file_name().to_string_lossy().ends_with(".sample") {
                return Err(problem());
            }
        }
        Ok(())
    }

    pub(crate) fn validate(
        &self,
        runner: &Runner,
        cwd: &Path,
        command: &[OsString],
        cancel: &CancellationToken,
    ) -> Result<()> {
        let position = config_prefix(command);
        let Some(verb) = command.get(position).and_then(|s| s.to_str()) else {
            return Ok(());
        };
        // --version is a whole command; anything else here could hide the real verb.
        if verb.starts_with('-') && verb != "--version" {
            return Err(Error::InvalidInput(
                "MCP transport refuses a Git option it doesn't recognize".into(),
            ));
        }
        if !matches!(verb, "fetch" | "push" | "clone" | "ls-remote") {
            return Ok(());
        }
        let operands = &command[position + 1..];
        let delimiter = operands
            .iter()
            .position(|a| a == "--")
            .ok_or_else(scope_error)?;
        if verb == "ls-remote" && operands[..delimiter].iter().any(|a| a == "--get-url") {
            return Ok(());
        }
        let source = operands.get(delimiter + 1).ok_or_else(scope_error)?;
        if source.to_string_lossy().starts_with('-') {
            return refused();
        }
        // These reads never use transport. Keep invocation overrides and inherited
        // config identical to the command that follows, including conditional includes.
        let read = self.configuration(runner, cwd, &command[..position], cancel)?;
        if verb == "clone" {
            return self.url(&read, source, false, cancel);
        }
        let mut resolve = command[..position].to_vec();
        resolve.extend(args(&["remote", "get-url", "--all"]));
        if verb == "push" {
            resolve.push("--push".into());
        }
        resolve.push(source.clone());
        let resolved = read.run(cwd, &resolve, vec![], cancel)?;
        if resolved.status.success() {
            let urls = std::str::from_utf8(&resolved.stdout).map_err(|_| scope_error())?;
            if urls.is_empty() {
                return refused();
            }
            for url in urls.lines() {
                self.url(&read, OsStr::new(url), verb == "push", cancel)?;
            }
        } else {
            // A reviewed command can pass an explicit endpoint rather than a
            // remote name. Relative paths and remote helpers fail this check.
            self.url(&read, source, verb == "push", cancel)?;
        }
        cancel.check()
    }

    /// LFS launches child Git/SSH/credential processes too. Its endpoint review is
    /// separate, but it must enforce the same effective command-override policy.
    pub(crate) fn configuration(
        &self,
        runner: &Runner,
        cwd: &Path,
        prefix: &[OsString],
        cancel: &CancellationToken,
    ) -> Result<Runner> {
        if std::env::vars_os().any(|(key, value)| !value.is_empty() && network_environment(&key)) {
            return Err(Error::InvalidInput("MCP transport refuses inherited proxy and TLS overrides; use a clean fixture environment".into()));
        }
        if std::env::vars_os().any(|(key, value)| {
            !value.is_empty()
                && COMMAND_ENV
                    .iter()
                    .any(|name| key.to_string_lossy().eq_ignore_ascii_case(name))
        }) {
            return Err(Error::InvalidInput("MCP transport refuses inherited SSH, askpass and helper-command overrides; use a clean fixture environment".into()));
        }
        let mut read = runner.clone();
        read.transport_scope = None;
        let mut config = prefix.to_vec();
        config.extend(args(&["config", "--null", "--list", "--includes"]));
        let raw = read
            .run(cwd, &config, vec![], cancel)?
            .success("transport configuration")?;
        for record in raw.split(|b| *b == 0).filter(|r| !r.is_empty()) {
            let mut fields = record.splitn(2, |b| *b == b'\n');
            let key = String::from_utf8_lossy(fields.next().unwrap()).to_ascii_lowercase();
            let value = fields.next();
            if network_configuration(&key, value) {
                return Err(Error::InvalidInput("MCP transport requires direct HTTPS with certificate verification and default trust; proxy, routing and TLS overrides are refused".into()));
            }
            let value = value.unwrap_or_default();
            if !value.is_empty()
                && (matches!(
                    key.as_str(),
                    "core.sshcommand" | "core.askpass" | "core.gitproxy"
                ) || (key.starts_with("credential.") && key.ends_with(".helper")))
            {
                return Err(Error::InvalidInput("MCP transport refuses configured SSH, askpass and credential-helper commands; use a clean fixture configuration".into()));
            }
            if (key.starts_with("url.")
                && (key.ends_with(".insteadof") || key.ends_with(".pushinsteadof")))
                || (key.starts_with("remote.")
                    && (key.ends_with(".vcs")
                        || key.ends_with(".uploadpack")
                        || key.ends_with(".receivepack")))
            {
                return Err(Error::InvalidInput("MCP transport refuses URL rewrites and custom remote transport commands; use an explicit fixture destination".into()));
            }
        }
        Ok(read)
    }
}

fn scope_error() -> Error {
    Error::InvalidInput("Transport destination is outside the approved MCP fixture scope".into())
}
fn refused<T>() -> Result<T> {
    Err(scope_error())
}
