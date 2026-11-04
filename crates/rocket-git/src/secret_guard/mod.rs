//! Default-on local preflight for Rocket-created commits and Git pushes.
//! Detection is conservative and bounded; failures never silently authorize a write.
mod objects;
mod patterns;
#[cfg(test)]
mod audit_tests;
use crate::{
    CancellationToken, Error, Result,
    runner::{Runner, args},
};
use std::{ffi::OsString, fmt, path::Path};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SecretKind {
    AiApiKey,
    GithubToken,
    PrivateKey,
    Credential,
    EnvironmentFile,
    Incomplete,
}
impl SecretKind {
    pub fn code(self) -> &'static str {
        match self {
            Self::AiApiKey => "ai_api_key_detected",
            Self::GithubToken => "github_token_detected",
            Self::PrivateKey => "private_key_detected",
            Self::Credential => "credential_detected",
            Self::EnvironmentFile => "environment_file_blocked",
            Self::Incomplete => "secret_scan_incomplete",
        }
    }
    fn label(self) -> &'static str {
        match self {
            Self::AiApiKey => "a possible AI-provider API key",
            Self::GithubToken => "a GitHub token",
            Self::PrivateKey => "a private key",
            Self::Credential => "a possible hardcoded credential",
            Self::EnvironmentFile => "an environment file",
            Self::Incomplete => "an incomplete safety check",
        }
    }
}

/// Contains only a fixed reason and a sanitized location; never matching content.
#[derive(Clone, Debug)]
pub struct SecretProtectionFailure {
    pub(crate) operation: &'static str,
    pub(crate) kind: SecretKind,
    pub(crate) location: String,
    pub(crate) line: Option<usize>,
}
impl SecretProtectionFailure {
    pub fn code(&self) -> &'static str {
        self.kind.code()
    }
    pub fn operation(&self) -> &'static str {
        self.operation
    }
    /// A short, value-free explanation suitable for a collapsed activity row.
    pub fn summary(&self) -> String {
        let (action, unchanged) = if self.operation == "push" {
            ("Push", "Nothing was sent.")
        } else {
            ("Commit", "Nothing was committed.")
        };
        format!("{action} blocked: {}. {unchanged}", self.kind.label())
    }
    pub fn location(&self) -> &str {
        &self.location
    }
    pub fn line(&self) -> Option<usize> {
        self.line
    }
    pub fn guidance(&self) -> &'static str {
        if self.kind == SecretKind::Incomplete {
            "The safety scan could not finish. Inspect missing objects, repository size and scan limits. No commit or push process was started. Do not bypass protection or retry through another tool."
        } else {
            "Remove the secret or environment file from the staged snapshot and every commit being published, then retry. Deleting it only in the latest commit is insufficient. Keep secrets outside Git and use a placeholder-only .env.example. Revoke exposed credentials. Do not bypass protection or retry through another tool."
        }
    }
}
impl fmt::Display for SecretProtectionFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} blocked [{}]: {} in {}",
            self.operation,
            self.code(),
            self.kind.label(),
            self.location
        )?;
        if let Some(line) = self.line {
            write!(f, " at line {line}")?;
        }
        write!(
            f,
            ". No {} process was started. {}",
            self.operation,
            self.guidance()
        )
    }
}

fn refusal(
    operation: &'static str,
    kind: SecretKind,
    location: String,
    line: Option<usize>,
) -> Error {
    Error::SecretProtection(SecretProtectionFailure {
        operation,
        kind,
        location,
        line,
    })
}
fn incomplete(operation: &'static str) -> Error {
    refusal(
        operation,
        SecretKind::Incomplete,
        "repository objects or scan limits".into(),
        None,
    )
}

pub(crate) fn command_position(arguments: &[OsString]) -> usize {
    let mut i = 0;
    while i < arguments.len() {
        if arguments[i] == "-c" {
            i += 2;
        } else if arguments[i] == "--no-replace-objects"
            || arguments[i].to_string_lossy().starts_with("--config-env=")
        {
            i += 1;
        } else {
            break;
        }
    }
    i
}

pub(crate) fn check(
    runner: &Runner,
    cwd: &Path,
    arguments: &[OsString],
    input: &[u8],
    cancel: &CancellationToken,
) -> Result<()> {
    let position = command_position(arguments);
    match arguments.get(position).and_then(|a| a.to_str()) {
        Some("commit") => check_commit(runner, cwd, arguments, input, cancel),
        Some("push") => check_push(runner, cwd, &arguments[position + 1..], cancel),
        _ => Ok(()),
    }
}

pub(crate) fn check_import_text(runner: &Runner, cwd: &Path, message: &[u8], files: &[(Vec<u8>,Vec<u8>)], cancel: &CancellationToken) -> Result<()> {
    if !runner.content_guard_enabled() { return Ok(()); }
    let scan=objects::Scan::new(runner,cwd,"commit",cancel);
    scan.message(message,"imported commit message".into())?;
    for (path,bytes) in files { scan.path(path)?;scan.content(bytes,patterns::safe_path(path))?; }
    Ok(())
}
pub(crate) fn check_import_index(runner: &Runner, cwd: &Path, cancel: &CancellationToken) -> Result<()> {
    if !runner.content_guard_enabled() { return Ok(()); }
    check_commit(runner,cwd,&args(&["commit"]),&[],cancel)
}

fn check_commit(
    runner: &Runner,
    cwd: &Path,
    arguments: &[OsString],
    input: &[u8],
    cancel: &CancellationToken,
) -> Result<()> {
    let mut scan = objects::Scan::new(runner, cwd, "commit", cancel);
    scan.message(input, "commit message".into())?;
    if input.is_empty() && arguments.iter().any(|a| a == "--amend") {
        let id = scan.resolve("HEAD")?;
        // Message reuse must not bypass detection. The staged snapshot below is
        // the new tree, so do not inspect the old tree when removing a secret.
        scan.single(&id, "reused commit message".into(), false)?;
    }
    let command = args(&["--no-replace-objects", "ls-files", "--stage", "-z"]);
    let before = scan.read(&command, vec![])?;
    for record in before.split(|b| *b == 0).filter(|r| !r.is_empty()) {
        let tab = record
            .iter()
            .position(|b| *b == b'\t')
            .ok_or_else(|| incomplete("commit"))?;
        let fields = record[..tab].split(|b| *b == b' ').collect::<Vec<_>>();
        if fields.len() != 3 || !matches!(fields[2], b"0" | b"1" | b"2" | b"3") {
            return Err(incomplete("commit"));
        }
        // Inspect all conflict stages as well. If they are clean, Git retains
        // its normal unresolved-conflict diagnostic without skipping any blob.
        let path = &record[tab + 1..];
        scan.path(path)?;
        if fields[0] != b"160000" {
            scan.enqueue(
                objects::oid(fields[1]).ok_or_else(|| incomplete("commit"))?,
                format!("staged {}", patterns::safe_path(path)),
            )?;
        }
    }
    scan.finish()?;
    if scan.read(&command, vec![])? != before {
        return Err(incomplete("commit"));
    }
    Ok(())
}

fn check_push(
    runner: &Runner,
    cwd: &Path,
    arguments: &[OsString],
    cancel: &CancellationToken,
) -> Result<()> {
    let mut scan = objects::Scan::new(runner, cwd, "push", cancel);
    let separator = arguments
        .iter()
        .position(|a| a == "--")
        .ok_or_else(|| incomplete("push"))?;
    // Rocket always supplies an explicit destination plus literal refspecs. Do
    // not guess a configured default, wildcard, mirror, or --all selection.
    if arguments.len() < separator + 3 {
        return Err(incomplete("push"));
    }
    for spec in &arguments[separator + 2..] {
        let spec = spec.to_str().ok_or_else(|| incomplete("push"))?;
        let (source, destination) = spec.split_once(':').ok_or_else(|| incomplete("push"))?;
        if !destination.starts_with("refs/") || destination.contains('*') {
            return Err(incomplete("push"));
        }
        if source.is_empty() {
            continue;
        } // Ref deletion publishes no objects.
        // All callers pin the source object before scanning. This binds the
        // checked graph to the actual push even if a hook moves a local branch.
        let id = objects::oid(source.as_bytes()).ok_or_else(|| incomplete("push"))?;
        scan.enqueue(id.clone(), format!("outgoing object {id}"))?;
    }
    scan.finish()
}
