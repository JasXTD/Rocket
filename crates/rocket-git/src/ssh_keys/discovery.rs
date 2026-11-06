//! Bounded, read-only discovery. Only public key contents are read during a scan.
use super::*;
use std::{io::Read, time::SystemTime};

const MAX_ENTRIES: usize = 512;
const MAX_KEYS: usize = 128;

#[derive(Clone, Debug)]
pub struct SshDiscoveredKey {
    pub key: PathBuf,
    pub kind: String,
    pub fingerprint: String,
    public_bytes: Vec<u8>,
    private_stamp: KeyStamp,
}

impl SshDiscoveredKey {
    /// The public key line from its `.pub` file, as the scan read it.
    pub fn public_key(&self) -> &str {
        // The scan accepted only one line of UTF-8 (`key_kind`).
        std::str::from_utf8(&self.public_bytes)
            .unwrap_or_default()
            .trim()
    }
}

/// Which file a found key is and when it last changed. Windows records its creation
/// time. Unix records the inode and its change time instead: some filesystems keep no
/// creation time, and any replacement, rename or permission change moves those.
#[derive(Clone, Debug, PartialEq, Eq)]
struct KeyStamp {
    length: u64,
    modified: SystemTime,
    #[cfg(not(unix))]
    created: SystemTime,
    #[cfg(unix)]
    changed: (u64, u64, i64, i64),
}
impl KeyStamp {
    fn read(file: &fs::File) -> Result<Self> {
        let m = file
            .metadata()
            .map_err(|e| Error::io("inspect SSH key", e))?;
        #[cfg(unix)]
        use std::os::unix::fs::MetadataExt;
        Ok(Self {
            length: m.len(),
            modified: m
                .modified()
                .map_err(|e| Error::io("inspect SSH key time", e))?,
            #[cfg(not(unix))]
            created: m
                .created()
                .map_err(|e| Error::io("inspect SSH key time", e))?,
            #[cfg(unix)]
            changed: (m.dev(), m.ino(), m.ctime(), m.ctime_nsec()),
        })
    }
}

#[derive(Clone, Debug)]
pub struct SshDiscovery {
    pub directory: PathBuf,
    pub keys: Vec<SshDiscoveredKey>,
    pub skipped: usize,
    pub truncated: bool,
}

fn public_bytes(file: &fs::File) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    file.take(16385)
        .read_to_end(&mut bytes)
        .map_err(|e| Error::io("read SSH public key", e))?;
    if bytes.len() > 16384 {
        return Err(fail(SshFailure::KeyInvalid));
    }
    Ok(bytes)
}

fn key_kind(bytes: &[u8]) -> Result<String> {
    let text = std::str::from_utf8(bytes).map_err(|_| fail(SshFailure::KeyInvalid))?;
    if text.lines().count() != 1 {
        return Err(fail(SshFailure::KeyInvalid));
    }
    let kind = text
        .split_whitespace()
        .next()
        .ok_or(fail(SshFailure::KeyInvalid))?;
    // These are the same software-key formats supported by profile import.
    if !matches!(
        kind,
        "ssh-ed25519"
            | "ssh-rsa"
            | "ecdsa-sha2-nistp256"
            | "ecdsa-sha2-nistp384"
            | "ecdsa-sha2-nistp521"
    ) {
        return Err(fail(SshFailure::KeyInvalid));
    }
    Ok(kind.to_owned())
}

impl SshManager {
    /// A single SSH folder, never a recursive drive search or SSH-config evaluation.
    pub fn discover_keys(
        &self,
        directory: &Path,
        cancel: &CancellationToken,
    ) -> Result<SshDiscovery> {
        self.runner
            .step("ssh-discover", "scan_public_key_pairs", || {
                cancel.check()?;
                files::safe_path(directory)?;
                let mut result = SshDiscovery {
                    directory: directory.into(),
                    keys: vec![],
                    skipped: 0,
                    truncated: false,
                };
                match fs::symlink_metadata(directory) {
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                        if let Some(log) = &self.runner.log {
                            log.ssh_discovery_result(0, 0, false);
                        }
                        return Ok(result);
                    }
                    Err(e) => return Err(Error::io("inspect SSH discovery folder", e)),
                    Ok(_) => {}
                }
                let _parents = pin(directory)?;
                let entries =
                    fs::read_dir(directory).map_err(|e| Error::io("scan SSH folder", e))?;
                for (count, entry) in entries.enumerate() {
                    cancel.check()?;
                    if count == MAX_ENTRIES || result.keys.len() == MAX_KEYS {
                        result.truncated = true;
                        break;
                    }
                    let entry = entry.map_err(|e| Error::io("read SSH folder entry", e))?;
                    let public = entry.path();
                    if !public
                        .extension()
                        .is_some_and(|v| v.eq_ignore_ascii_case("pub"))
                    {
                        continue;
                    }
                    let key = public.with_extension("");
                    let candidate = (|| {
                        let private = files::locked(&key)?;
                        let private_stamp = KeyStamp::read(&private)?;
                        if private_stamp.length == 0 || private_stamp.length > 1024 * 1024 {
                            return Err(fail(SshFailure::KeyInvalid));
                        }
                        let public_file = files::locked(&public)?;
                        let public_bytes = public_bytes(&public_file)?;
                        let kind = key_kind(&public_bytes)?;
                        // Both paths stay pinned and write-locked while OpenSSH reads only .pub.
                        let fingerprint = self.fingerprint(&key, cancel)?;
                        Ok(SshDiscoveredKey {
                            key,
                            kind,
                            fingerprint,
                            public_bytes,
                            private_stamp,
                        })
                    })();
                    match candidate {
                        Ok(key) => result.keys.push(key),
                        Err(Error::Cancelled) => return Err(Error::Cancelled),
                        Err(_) => result.skipped += 1,
                    }
                }
                cancel.check()?;
                result.keys.sort_by(|a, b| a.key.cmp(&b.key));
                if let Some(log) = &self.runner.log {
                    log.ssh_discovery_result(result.keys.len(), result.skipped, result.truncated);
                }
                Ok(result)
            })
    }

    /// Recheck the reviewed pair, holding its files through profile publication.
    /// A scan never activates a key, creates a profile or reads private key contents.
    pub fn import_discovered(
        &self,
        key: &SshDiscoveredKey,
        alias: &str,
        host: &str,
        user: &str,
        port: u16,
        cancel: &CancellationToken,
    ) -> Result<SshProfile> {
        self.runner
            .step("ssh-create", "recheck_discovered_key", || {
                let _guard = self.guard_discovered(key, cancel)?;
                // The existing importer verifies that the actual private/public payloads match.
                self.prepare(alias, host, user, port, Some(&key.key), cancel)?
                    .finish(cancel)
            })
    }

    /// Hold a found pair's files after checking they are still the ones the scan read:
    /// the same private file and the same public key.
    pub fn guard_discovered(
        &self,
        key: &SshDiscoveredKey,
        cancel: &CancellationToken,
    ) -> Result<SshKeyGuard> {
        cancel.check()?;
        let parents = pin(key.key.parent().ok_or(fail(SshFailure::UnsafePath))?)?;
        let private = files::locked(&key.key).map_err(|_| fail(SshFailure::Changed))?;
        let public =
            files::locked(&files::public_path(&key.key)).map_err(|_| fail(SshFailure::Changed))?;
        if KeyStamp::read(&private)? != key.private_stamp
            || public_bytes(&public)? != key.public_bytes
        {
            return Err(fail(SshFailure::Changed));
        }
        // Using it means ssh-add or a profile reads it: OpenSSH refuses an open key.
        #[cfg(unix)]
        files::private_key(&private)?;
        Ok(SshKeyGuard {
            _files: vec![private, public],
            _parents: parents,
        })
    }

    /// Remove a found key's identity from ssh-agent, after rechecking its files.
    pub fn unload_discovered(
        &self,
        key: &SshDiscoveredKey,
        cancel: &CancellationToken,
    ) -> Result<()> {
        self.runner
            .step("ssh-unload", "remove_selected_identity", || {
                let _guard = self.guard_discovered(key, cancel)?;
                self.remove_identity(&key.key, &key.fingerprint, cancel)
            })
    }
}
