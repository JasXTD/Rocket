//! Ask Git to sign in an owned bare sandbox, then publish only through the
//! normal tag transaction. A signer failure/cancellation cannot create a tag.
use super::*;
use std::{
    ffi::OsString,
    path::Path,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};
static NEXT: AtomicU64 = AtomicU64::new(0);
impl Repository {
    pub(crate) fn sign_tag_object(
        &self,
        details: &TagDetails,
        expected: &RepositorySigning,
        cancel: &CancellationToken,
    ) -> Result<Vec<u8>> {
        self.runner.step("tag-sign", "sign_tag_in_sandbox", || {
            if self.signing(cancel)? != *expected {
                return Err(Error::Signing(SigningFailure::StaleSettings));
            }
            if !["ssh", "openpgp"].contains(&expected.format()) {
                return Err(Error::Signing(SigningFailure::UnsupportedFormat));
            }
            if expected.format() == "ssh"
                && expected.effective.key.as_ref().is_none_or(|v| v.is_empty())
            {
                return Err(Error::Signing(SigningFailure::KeyUnavailable));
            }
            let raw = details
                .annotation
                .as_ref()
                .ok_or(Error::Signing(SigningFailure::SignFailed))?;
            let at = raw
                .windows(2)
                .position(|w| w == b"\n\n")
                .ok_or(Error::Signing(SigningFailure::SignFailed))?;
            let message = raw[at + 2..].to_vec();
            let scratch = Scratch::new()?;
            let empty_config = scratch.path.join("empty.config");
            fs::write(&empty_config, b"")
                .map_err(|e| Error::io("prepare signing sandbox configuration", e))?;
            let git_dir = git_path(&scratch.path);
            let mut command = vec![OsString::from("--git-dir"), git_dir.clone()];
            let no_hooks = scratch.path.join("no-hooks");
            fs::create_dir(&no_hooks).map_err(|e| Error::io("prepare signing sandbox hooks", e))?;
            command.extend(args(&[
                "init",
                "--bare",
                "--quiet",
                "--template=",
                if details.target.as_str().len() == 64 {
                    "--object-format=sha256"
                } else {
                    "--object-format=sha1"
                },
            ]));
            self.runner
                .write(self.cwd(), &command, vec![], cancel)?
                .success("tag-sign")?;
            let objects = self.info().common_dir.join("objects");
            let alternate = git_path(&objects)
                .to_str()
                .ok_or_else(|| {
                    Error::Unsupported(
                        "Non-UTF-8 object paths cannot be used for signed tags".into(),
                    )
                })?
                .to_owned();
            if alternate.contains(['\n', '\r', '"']) {
                return Err(Error::Unsupported(
                    "This object path cannot be used for the signing sandbox".into(),
                ));
            }
            fs::write(
                scratch.path.join("objects/info/alternates"),
                format!("{alternate}\n"),
            )
            .map_err(|e| Error::io("prepare signing object access", e))?;
            let mut command = vec![OsString::from("--git-dir"), git_dir.clone()];
            for (key, value) in &expected.values {
                command.push("-c".into());
                command.push(format!("{key}={value}").into());
            }
            command.extend(args(&[
                "-c",
                &format!("core.hooksPath={}", git_path(&no_hooks).to_string_lossy()),
                "-c",
                "tag.gpgSign=false",
                "tag",
                "--sign",
                "--cleanup=verbatim",
                "--file=-",
                "--",
            ]));
            command.push(crate::model::bytes_to_os(&details.entry.name)?);
            command.push(details.target.as_str().into());
            self.runner.step("tag-sign", "run_tag_signer", || {
                self.runner
                    .signing_sandbox(self.cwd(), &command, message, cancel, &empty_config)?
                    .success("tag-sign")
            })?;
            let mut command = vec![OsString::from("--git-dir"), git_dir];
            command.extend(args(&["--no-replace-objects", "cat-file", "tag"]));
            let mut reference = OsString::from("refs/tags/");
            reference.push(crate::model::bytes_to_os(&details.entry.name)?);
            command.push(reference);
            let signed = self
                .runner
                .run(self.cwd(), &command, vec![], cancel)?
                .success("tag-sign")?;
            // Verify the unsigned prefix: the signer may add only a signature;
            // Git supplies the tagger timestamp, so compare target/name/message.
            let header = format!(
                "object {}\ntype commit\ntag {}\ntagger ",
                details.target,
                String::from_utf8_lossy(&details.entry.name)
            );
            if !signed.starts_with(header.as_bytes())
                || !signed
                    .windows(2)
                    .position(|w| w == b"\n\n")
                    .is_some_and(|body| signed[body..].starts_with(&raw[at..]))
            {
                return Err(Error::Signing(SigningFailure::SignFailed));
            }
            scratch.cleanup()?;
            if self.signing(cancel)? != *expected {
                return Err(Error::Signing(SigningFailure::StaleSettings));
            }
            cancel.check()?;
            Ok(signed)
        })
    }
}
struct Scratch {
    path: PathBuf,
    base: PathBuf,
    guards: Vec<fs::File>,
}
impl Scratch {
    fn new() -> Result<Self> {
        let base = std::env::temp_dir()
            .canonicalize()
            .map_err(|e| Error::io("resolve signing sandbox parent", e))?;
        let path = base.join(format!(
            "rocket-sign-{}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let root = base.ancestors().last().unwrap_or(Path::new(""));
        let mut guards = local_files::pin_directories(root, &base)?;
        fs::create_dir(&path).map_err(|e| Error::io("reserve signing sandbox", e))?;
        guards.extend(local_files::pin_directories(&base, &path)?);
        Ok(Self { path, base, guards })
    }
    fn cleanup(mut self) -> Result<()> {
        self.guards.clear();
        self.remove()
    }
    fn remove(&mut self) -> Result<()> {
        if self.path.as_os_str().is_empty() {
            return Ok(());
        }
        if self.path.parent() != Some(self.base.as_path())
            || self
                .path
                .canonicalize()
                .map_err(|e| Error::io("check signing sandbox cleanup", e))?
                .parent()
                != Some(self.base.as_path())
        {
            return Err(Error::Signing(SigningFailure::SignFailed));
        }
        fs::remove_dir_all(&self.path).map_err(|e| Error::io("remove signing sandbox", e))?;
        self.path = PathBuf::new();
        Ok(())
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        self.guards.clear();
        let _ = self.remove();
    }
}
