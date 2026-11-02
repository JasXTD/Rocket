//! Repository-local signing preferences and bounded, explicit verification.
//! Git remains the signer/verifier; no private key material enters Rocket.
use crate::{
    local_files,
    runner::{args, git_path},
    *,
};
use std::{collections::BTreeMap, fs, io::Write, path::PathBuf};
mod tag;
mod verify;
pub use verify::*;

const KEYS: [&str; 4] = [
    "gpg.format",
    "user.signingkey",
    "commit.gpgsign",
    "gpg.ssh.allowedsignersfile",
];
const PATTERN: &str = "^(gpg\\..*|user\\.(signingkey|name|email)|commit\\.gpgsign)$";
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SigningOptions {
    pub format: Option<String>,
    pub key: Option<String>,
    pub sign_commits: Option<bool>,
    pub allowed_signers: Option<String>,
}
impl SigningOptions {
    pub fn validate(&self) -> Result<()> {
        if self
            .format
            .as_deref()
            .is_some_and(|f| !["ssh", "openpgp"].contains(&f))
        {
            return Err(Error::Signing(SigningFailure::UnsupportedFormat));
        }
        for text in [&self.key, &self.allowed_signers].into_iter().flatten() {
            if text.is_empty()
                || text.len() > 8192
                || text.trim() != text
                || text.chars().any(char::is_control)
            {
                return Err(Error::InvalidInput("Use a key identifier or file path without surrounding whitespace or control characters (maximum 8 KiB).".into()));
            }
        }
        Ok(())
    }
    fn values(&self) -> [Option<String>; 4] {
        [
            self.format.clone(),
            self.key.clone(),
            self.sign_commits.map(|v| v.to_string()),
            self.allowed_signers.clone(),
        ]
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepositorySigning {
    pub local: SigningOptions,
    pub effective: SigningOptions,
    pub(crate) values: BTreeMap<String, String>,
    pub(crate) config: Vec<u8>,
    pub(crate) repository: PathBuf,
}
impl RepositorySigning {
    pub fn program(&self) -> &str {
        match self.format() {
            "ssh" => self
                .values
                .get("gpg.ssh.program")
                .map(String::as_str)
                .unwrap_or("ssh-keygen"),
            "openpgp" => self
                .values
                .get("gpg.openpgp.program")
                .or_else(|| self.values.get("gpg.program"))
                .map(String::as_str)
                .unwrap_or("gpg"),
            _ => "Configured Git verifier",
        }
    }
    pub fn format(&self) -> &str {
        self.effective.format.as_deref().unwrap_or("openpgp")
    }
}
pub struct SigningWriteReport {
    pub result: Result<WriteOutput>,
    pub observed: Result<RepositorySigning>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SigningFailure {
    StaleSettings,
    UnsupportedFormat,
    KeyUnavailable,
    ToolUnavailable,
    UnlockFailed,
    SignFailed,
    InvalidSignature,
    UntrustedSigner,
    Expired,
    Revoked,
    CannotVerify,
}
impl SigningFailure {
    pub fn code(self) -> &'static str {
        match self {
            Self::StaleSettings => "signing_settings_changed",
            Self::UnsupportedFormat => "signing_format_unsupported",
            Self::KeyUnavailable => "signing_key_unavailable",
            Self::ToolUnavailable => "signing_tool_unavailable",
            Self::UnlockFailed => "signing_unlock_failed",
            Self::SignFailed => "signing_failed",
            Self::InvalidSignature => "signature_invalid",
            Self::UntrustedSigner => "signature_untrusted",
            Self::Expired => "signature_expired",
            Self::Revoked => "signature_revoked",
            Self::CannotVerify => "signature_unavailable",
        }
    }
    pub fn guidance(self) -> &'static str {
        match self {
            Self::StaleSettings => {
                "Signing settings changed since review. Reload them and review again; Rocket did not replace the newer settings."
            }
            Self::UnsupportedFormat => {
                "This signing editor supports SSH and OpenPGP. Keep inherited settings for other formats and configure them with Git."
            }
            Self::KeyUnavailable => {
                "The signing key is missing or unavailable. Check the key identifier/path and whether the required key is loaded in your agent."
            }
            Self::ToolUnavailable => {
                "Git could not start its signing tool. Check the configured GPG or SSH signing program and its installation."
            }
            Self::UnlockFailed => {
                "The key could not be unlocked or the signer refused the request. Unlock it through your signing tool or agent, then retry. Rocket does not collect passphrases."
            }
            Self::SignFailed => {
                "Git did not confirm a signature. Check the signing key, signer and agent. Inspect refreshed state before retrying a commit; cancellation does not undo completed writes."
            }
            Self::InvalidSignature => {
                "The signature does not validate for this object. Inspect the object and obtain a trusted copy."
            }
            Self::UntrustedSigner => {
                "The signature is cryptographically valid, but your trust policy did not accept the signer. Check the fingerprint independently before changing trust settings."
            }
            Self::Expired => {
                "The signature or signing key has expired. Check its validity period and your verification policy."
            }
            Self::Revoked => {
                "The signing key has been revoked. Do not treat this signature as trusted."
            }
            Self::CannotVerify => {
                "Verification could not establish a result. Check the verification tool, public key and trust configuration (SSH requires an allowed signers file)."
            }
        }
    }
    pub(crate) fn classify(bytes: &[u8]) -> Self {
        let text = String::from_utf8_lossy(bytes).to_ascii_lowercase();
        if [
            "cannot run",
            "cannot spawn",
            "not found",
            "no such file or directory",
        ]
        .iter()
        .any(|v| text.contains(v))
            && ["gpg", "ssh-keygen", "program"]
                .iter()
                .any(|v| text.contains(v))
        {
            Self::ToolUnavailable
        } else if [
            "bad passphrase",
            "incorrect passphrase",
            "agent refused",
            "sign_and_send_pubkey",
            "no pinentry",
            "inappropriate ioctl",
            "operation cancelled",
            "operation canceled",
        ]
        .iter()
        .any(|v| text.contains(v))
        {
            Self::UnlockFailed
        } else if [
            "no secret key",
            "couldn't load public key",
            "could not load public key",
            "no private key",
            "no signing key",
            "user.signingkey needs",
            "not a key",
        ]
        .iter()
        .any(|v| text.contains(v))
        {
            Self::KeyUnavailable
        } else {
            Self::SignFailed
        }
    }
}
impl Error {
    pub fn signing_failure(&self) -> Option<SigningFailure> {
        match self {
            Self::Signing(f) => Some(*f),
            Self::Git {
                operation, stderr, ..
            } if ["commit", "amend", "tag-sign"].contains(operation) => {
                let text = String::from_utf8_lossy(stderr).to_ascii_lowercase();
                (text.contains("failed to sign")
                    || text.contains("signing failed")
                    || *operation == "tag-sign")
                    .then(|| SigningFailure::classify(stderr))
            }
            _ => None,
        }
    }
}
impl Repository {
    pub fn signing(&self, cancel: &CancellationToken) -> Result<RepositorySigning> {
        self.runner
            .step("signing-read", "read_signing_settings", || {
                let config = local_files::read_config(&self.info().common_dir.join("config"))?;
                let local = self.signing_values(true, cancel)?;
                let values = self.signing_values(false, cancel)?;
                if local_files::read_config(&self.info().common_dir.join("config"))? != config {
                    return Err(Error::Signing(SigningFailure::StaleSettings));
                }
                Ok(RepositorySigning {
                    local: options(&local)?,
                    effective: options(&values)?,
                    values,
                    config,
                    repository: self.info().common_dir.clone(),
                })
            })
    }
    fn signing_values(
        &self,
        local: bool,
        cancel: &CancellationToken,
    ) -> Result<BTreeMap<String, String>> {
        let mut command = args(&["config"]);
        if local {
            command.extend(args(&["--local", "--no-includes"]));
        }
        command.extend(args(&["--null", "--get-regexp", PATTERN]));
        let out = self.runner.run(self.cwd(), &command, vec![], cancel)?;
        if out.status.code() == Some(1) && out.stdout.is_empty() {
            return Ok(BTreeMap::new());
        }
        let bytes = out.success("signing-read")?;
        let mut values = BTreeMap::new();
        for row in bytes.split(|b| *b == 0).filter(|r| !r.is_empty()) {
            let at = row.iter().position(|b| *b == b'\n').unwrap_or(row.len());
            let key = String::from_utf8_lossy(&row[..at]).to_ascii_lowercase();
            // A bare Git boolean is true; an explicitly empty value is false.
            let value = if at == row.len() {
                if key == KEYS[2] {
                    "true".into()
                } else {
                    String::new()
                }
            } else {
                String::from_utf8(row[at + 1..].to_vec()).map_err(|_| {
                    Error::Unsupported("Non-UTF-8 signing settings must be edited with Git".into())
                })?
            };
            // Git treats these as aliases and the last occurrence wins. Preserve
            // that ordering before replaying the effective settings in a sandbox.
            values.insert(
                if key == "gpg.program" {
                    "gpg.openpgp.program".into()
                } else {
                    key
                },
                value,
            );
        }
        Ok(values)
    }
    /// Atomic update of four local keys, retaining all unrelated bytes and includes.
    pub fn set_signing(
        &self,
        expected: &RepositorySigning,
        options: &SigningOptions,
        cancel: &CancellationToken,
    ) -> Result<SigningWriteReport> {
        options.validate()?;
        let _permit = self.acquire_write(cancel)?;
        let result = self.runner.observe("write_result", "signing-save", || {
            cancel.check()?;
            if self.signing(cancel)? != *expected {
                return Err(Error::Signing(SigningFailure::StaleSettings));
            }
            let config = self.info().common_dir.join("config");
            let lock = config.with_extension("lock");
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&lock)
                .map_err(|e| {
                    Error::io(
                        "lock signing config; finish other Git writers before retrying",
                        e,
                    )
                })?;
            let mut owned = ConfigLock(Some(lock.clone()));
            if self.signing(cancel)? != *expected {
                return Err(Error::Signing(SigningFailure::StaleSettings));
            }
            file.write_all(&expected.config)
                .and_then(|_| file.sync_all())
                .map_err(|e| Error::io("prepare signing config", e))?;
            drop(file);
            for ((key, value), old) in KEYS
                .into_iter()
                .zip(options.values())
                .zip(expected.local.values())
            {
                if value.is_none() && old.is_none() {
                    continue;
                }
                let mut command = args(&["config", "--file"]);
                command.push(git_path(&lock));
                command.extend(args(&[
                    "--no-includes",
                    if value.is_some() {
                        "--replace-all"
                    } else {
                        "--unset-all"
                    },
                    key,
                ]));
                if let Some(value) = value {
                    command.push(value.into());
                }
                self.runner
                    .step("signing-save", "prepare_signing_setting", || {
                        self.runner
                            .write(self.cwd(), &command, vec![], cancel)?
                            .success("signing-save")
                    })?;
            }
            if self.signing(cancel)? != *expected {
                return Err(Error::Signing(SigningFailure::StaleSettings));
            }
            local_files::read_config(&lock)?;
            fs::OpenOptions::new()
                .write(true)
                .open(&lock)
                .and_then(|f| f.sync_all())
                .map_err(|e| Error::io("flush signing config", e))?;
            cancel.check()?;
            local_files::move_path(&lock, &config, true)?;
            owned.0 = None;
            Ok(WriteOutput {
                stdout: vec![],
                stderr: vec![],
            })
        });
        let observed = self
            .runner
            .step("signing-save", "reload_signing_settings", || {
                self.signing(&CancellationToken::new())
            });
        Ok(SigningWriteReport { result, observed })
    }
}
fn options(values: &BTreeMap<String, String>) -> Result<SigningOptions> {
    let sign_commits = values
        .get(KEYS[2])
        .map(|v| match v.to_ascii_lowercase().as_str() {
            "true" | "yes" | "on" => Ok(true),
            "" | "false" | "no" | "off" => Ok(false),
            number => number.parse::<i64>().map(|n| n != 0).map_err(|_| {
                Error::InvalidInput("Invalid commit.gpgsign boolean; correct it with Git".into())
            }),
        })
        .transpose()?;
    Ok(SigningOptions {
        format: values.get(KEYS[0]).cloned(),
        key: values.get(KEYS[1]).cloned(),
        sign_commits,
        allowed_signers: values.get(KEYS[3]).cloned(),
    })
}
struct ConfigLock(Option<PathBuf>);
impl Drop for ConfigLock {
    fn drop(&mut self) {
        if let Some(path) = &self.0 {
            let _ = fs::remove_file(path);
        }
    }
}
