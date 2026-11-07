//! Release archives are data. No script or executable from a download runs here.
use rocket_git::{
    CancellationToken, Error, Result,
    app_update::{AppRelease, UpdateClient, UpdateVersion},
    osv_file_digest,
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Read,
    path::{Component, Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU8, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

pub const MAX_FILE: u64 = 256 * 1024 * 1024;
pub const MAX_TREE: u64 = 1024 * 1024 * 1024;
/// The programs a packaged Rocket must contain. The first one starts Rocket.
#[cfg(windows)]
pub const BINARIES: [&str; 7] = [
    "rocket.exe",
    "rocket-mcp.exe",
    "rocket-cli.exe",
    "rocket-rebase-editor.exe",
    "rocket-terminal-host.exe",
    "rocket-lua-host.exe",
    "rocket-ssh.exe",
];
/// Linux has no terminal host: the terminal opens a pseudoterminal itself.
#[cfg(not(windows))]
pub const BINARIES: [&str; 6] = [
    "rocket",
    "rocket-mcp",
    "rocket-cli",
    "rocket-rebase-editor",
    "rocket-lua-host",
    "rocket-ssh",
];
pub const ENTRYPOINT: &str = BINARIES[0];
/// The platform an update manifest must name, as its file name does.
fn platform() -> Result<&'static str> {
    rocket_git::app_update::UPDATE_PLATFORM
        .as_ref()
        .map(|target| target.platform)
        .ok_or_else(|| error("Rocket has no update bundle for this platform."))
}
pub fn error(message: &str) -> Error {
    Error::InvalidInput(message.into())
}
pub fn io(source: std::io::Error) -> Error {
    Error::Io {
        action: "prepare app update",
        source,
    }
}
pub fn plain(path: &Path) -> Result<()> {
    if !path.is_absolute()
        || path
            .components()
            .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
    {
        return Err(error(
            "Update paths must be absolute, without parent traversal.",
        ));
    }
    for part in path.ancestors() {
        match fs::symlink_metadata(part) {
            Ok(m) => {
                #[cfg(windows)]
                {
                    use std::os::windows::fs::MetadataExt;
                    if m.file_attributes() & 0x400 != 0 {
                        return Err(error("Update path contains a link or reparse point."));
                    }
                }
                if m.file_type().is_symlink() {
                    return Err(error("Update path contains a link."));
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(io(e)),
        }
    }
    Ok(())
}
pub fn relative(path: &str) -> bool {
    !path.is_empty()
        && path.len() < 1024
        && !path
            .bytes()
            .any(|b| b < 32 || matches!(b, b'\\' | b':' | b'<' | b'>' | b'"' | b'|' | b'?' | b'*'))
        && path.split('/').all(|p| {
            let stem = p.split('.').next().unwrap_or("").to_ascii_lowercase();
            !p.is_empty()
                && p != "."
                && p != ".."
                && !p.ends_with(['.', ' '])
                && !["con", "prn", "aux", "nul", "conin$", "conout$"].contains(&stem.as_str())
                && !((stem.starts_with("com") || stem.starts_with("lpt"))
                    && stem.chars().count() == 4
                    && stem
                        .chars()
                        .last()
                        .is_some_and(|c| c.is_ascii_digit() || matches!(c, '¹' | '²' | '³')))
        })
}
pub fn read(path: &Path, max: u64) -> Result<Vec<u8>> {
    plain(path)?;
    let m = fs::metadata(path).map_err(io)?;
    if !m.is_file() || m.len() > max {
        return Err(error("Update metadata exceeds its size limit."));
    }
    let mut bytes = Vec::new();
    fs::File::open(path)
        .map_err(io)?
        .take(max + 1)
        .read_to_end(&mut bytes)
        .map_err(io)?;
    if bytes.len() as u64 > max {
        return Err(error("Update metadata changed while reading."));
    }
    Ok(bytes)
}
pub fn json_file(path: &Path) -> Result<Value> {
    serde_json::from_slice(&read(path, 4 * 1024 * 1024)?)
        .map_err(|_| error("Update metadata is invalid."))
}
pub fn digest(path: &Path, cancel: &CancellationToken) -> Result<String> {
    plain(path)?;
    osv_file_digest(path, MAX_FILE, cancel)
}
pub fn inventory(root: &Path, cancel: &CancellationToken) -> Result<BTreeMap<String, String>> {
    plain(root)?;
    let mut pending = vec![root.to_path_buf()];
    let mut files = BTreeMap::new();
    let mut names = BTreeSet::new();
    let mut entries = 0;
    let mut total = 0u64;
    while let Some(dir) = pending.pop() {
        cancel.check()?;
        for item in fs::read_dir(&dir).map_err(io)? {
            entries += 1;
            if entries > 20000 {
                return Err(error("Release contains too many entries."));
            }
            let item = item.map_err(io)?;
            let path = item.path();
            plain(&path)?;
            let m = item.metadata().map_err(io)?;
            if m.is_dir() {
                pending.push(path);
                continue;
            }
            if !m.is_file() {
                return Err(error("Release contains a special file."));
            }
            let name = path
                .strip_prefix(root)
                .map_err(|_| error("Release path escaped its directory."))?
                .to_str()
                .ok_or_else(|| error("Release path is not Unicode."))?
                .replace('\\', "/");
            if !relative(&name) || !names.insert(name.to_lowercase()) {
                return Err(error("Release has an unsafe or duplicate path."));
            }
            total = total
                .checked_add(m.len())
                .ok_or_else(|| error("Release size overflow."))?;
            if files.len() >= 20000 || total > MAX_TREE {
                return Err(error("Unpacked release exceeds its size limit."));
            }
            files.insert(name, digest(&path, cancel)?);
        }
    }
    Ok(files)
}
pub fn manifest(
    bundle: &Path,
    version: UpdateVersion,
    cancel: &CancellationToken,
) -> Result<BTreeMap<String, String>> {
    let value = json_file(&bundle.join("rocket-update.json"))?;
    if value["schema"] != 1
        || value["version"] != version.text()
        || value["platform"] != platform()?
        || value["entrypoint"] != ENTRYPOINT
    {
        return Err(error(
            "The bundle version or platform does not match this update.",
        ));
    }
    let records = value["files"]
        .as_array()
        .filter(|a| a.len() <= 20000)
        .ok_or_else(|| error("Update file manifest is missing."))?;
    let mut expected = BTreeMap::new();
    let mut names = BTreeSet::new();
    let mut total = 0u64;
    for file in records {
        cancel.check()?;
        let name = file["path"]
            .as_str()
            .filter(|s| {
                relative(s)
                    && !s.eq_ignore_ascii_case("rocket-update.json")
                    && !s.eq_ignore_ascii_case("SHA256SUMS.txt")
            })
            .ok_or_else(|| error("Update manifest contains an unsafe path."))?;
        let sha = file["sha256"]
            .as_str()
            .filter(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()))
            .ok_or_else(|| error("Update manifest contains an invalid hash."))?;
        let size = file["size"]
            .as_u64()
            .filter(|n| *n <= MAX_FILE)
            .ok_or_else(|| error("Update manifest contains an oversized file."))?;
        total = total
            .checked_add(size)
            .ok_or_else(|| error("Update size overflow."))?;
        if total > MAX_TREE || !names.insert(name.to_lowercase()) {
            return Err(error(
                "Update manifest contains duplicate paths or exceeds the size limit.",
            ));
        }
        if fs::metadata(bundle.join(name)).map_err(io)?.len() != size {
            return Err(error("Update file length does not match its manifest."));
        }
        expected.insert(name.to_owned(), sha.to_ascii_lowercase());
    }
    for name in BINARIES {
        if !expected.contains_key(name) {
            return Err(error("The update is missing a required Rocket executable."));
        }
    }
    let actual = installed(bundle, cancel)?;
    let mut payload = actual.clone();
    payload.remove("rocket-update.json");
    payload.remove("SHA256SUMS.txt");
    if payload != expected {
        return Err(error(
            "Update files do not exactly match the release manifest. Nothing was installed.",
        ));
    }
    Ok(actual)
}
pub fn installed(root: &Path, cancel: &CancellationToken) -> Result<BTreeMap<String, String>> {
    let sums = String::from_utf8(read(&root.join("SHA256SUMS.txt"), 4 * 1024 * 1024)?)
        .map_err(|_| error("Installed release manifest is invalid."))?;
    let mut expected = BTreeMap::new();
    let mut names = BTreeSet::new();
    for line in sums.lines() {
        cancel.check()?;
        let (sha, name) = line
            .split_once("  ")
            .ok_or_else(|| error("Installed release manifest is invalid."))?;
        if sha.len() != 64
            || !sha.bytes().all(|b| b.is_ascii_hexdigit())
            || !relative(name)
            || name.eq_ignore_ascii_case("SHA256SUMS.txt")
            || !names.insert(name.to_lowercase())
        {
            return Err(error(
                "Installed release manifest contains an invalid or duplicate file.",
            ));
        }
        expected.insert(name.into(), sha.to_ascii_lowercase());
    }
    for name in BINARIES {
        if !expected.contains_key(name) {
            return Err(error(
                "Use a packaged Rocket installation for in-place updates.",
            ));
        }
    }
    expected.insert(
        "SHA256SUMS.txt".into(),
        digest(&root.join("SHA256SUMS.txt"), cancel)?,
    );
    if inventory(root, cancel)? != expected {
        return Err(error(
            "The installation contains changed or extra files. They are preserved; install the new bundle in a separate folder.",
        ));
    }
    Ok(expected)
}
#[derive(Clone)]
pub struct Prepared {
    pub job: PathBuf,
    pub version: UpdateVersion,
    pub files: BTreeMap<String, String>,
}
pub fn unique(parent: &Path) -> Result<PathBuf> {
    plain(parent)?;
    fs::create_dir_all(parent).map_err(io)?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| error("System clock is unavailable."))?
        .as_nanos();
    let job = parent.join(format!("update-{nonce}-{}", std::process::id()));
    fs::create_dir(&job).map_err(io)?;
    Ok(job)
}
pub fn prepare(
    client: &UpdateClient,
    release: &AppRelease,
    cache: &Path,
    phase: &Arc<AtomicU8>,
    cancel: &CancellationToken,
) -> Result<Prepared> {
    let job = unique(cache)?;
    phase.store(1, Ordering::Release);
    let archive = client.download(release, &job, cancel)?;
    phase.store(2, Ordering::Release);
    // Hold the ZIP against modification/deletion across verification and extraction.
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(1);
    }
    let _lease = options.open(&archive).map_err(io)?;
    if osv_file_digest(&archive, rocket_git::app_update::UPDATE_MAX_ARCHIVE, cancel)?
        != release.checksum()
    {
        return Err(error("Downloaded archive changed before extraction."));
    }
    let bundle = job.join("bundle");
    client.extract(&archive, &bundle, cancel)?;
    phase.store(3, Ordering::Release);
    let files = manifest(&bundle, release.version, cancel)?;
    fs::write(
        job.join("prepared.json"),
        serde_json::to_vec_pretty(
            &json!({"schema":1,"version":release.version.text(),"files":files}),
        )
        .unwrap(),
    )
    .map_err(io)?;
    drop(_lease);
    fs::remove_file(archive).map_err(io)?;
    Ok(Prepared {
        job,
        version: release.version,
        files,
    })
}
pub fn copy_tree(
    source: &Path,
    destination: &Path,
    files: &BTreeMap<String, String>,
    cancel: &CancellationToken,
) -> Result<()> {
    plain(destination)?;
    fs::create_dir(destination).map_err(io)?;
    for (name, sha) in files {
        cancel.check()?;
        let input = source.join(name);
        let output = destination.join(name);
        plain(&input)?;
        plain(&output)?;
        fs::create_dir_all(output.parent().unwrap()).map_err(io)?;
        let mut options = fs::OpenOptions::new();
        options.read(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.share_mode(1);
        }
        let mut from = options.open(&input).map_err(io)?;
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        // Unpacked files carry no permissions; only Rocket's own programs run.
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(if BINARIES.contains(&name.as_str()) {
                0o755
            } else {
                0o644
            });
        }
        let mut to = options.open(&output).map_err(io)?;
        let mut buffer = [0u8; 65536];
        let mut count = 0u64;
        loop {
            cancel.check()?;
            let n = from.read(&mut buffer).map_err(io)?;
            if n == 0 {
                break;
            }
            count += n as u64;
            if count > MAX_FILE {
                return Err(error("Release file changed during preparation."));
            }
            std::io::Write::write_all(&mut to, &buffer[..n]).map_err(io)?;
        }
        drop(to);
        if digest(&output, cancel)? != *sha {
            return Err(error("Release changed while preparing the installation."));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn paths_reject_windows_aliases_and_escapes() {
        for path in [
            "../x",
            "/x",
            "C:/x",
            "a//b",
            "a\\b",
            "a:ads",
            "CON.txt",
            "a/LPT1",
            "a.",
            "a /b",
            "a/*",
            "COM¹.dat",
            "conout$",
        ] {
            assert!(!relative(path), "{path}");
        }
        for path in ["rocket.exe", "資料/README.md", "git/2.56.0/bin/git.exe"] {
            assert!(relative(path), "{path}");
        }
    }
    fn fixture() -> PathBuf {
        let root = unique(&std::env::temp_dir()).unwrap();
        for name in BINARIES {
            fs::write(root.join(name), b"synthetic binary").unwrap();
        }
        fs::create_dir(root.join("資料")).unwrap();
        fs::write(root.join("資料/README.md"), "Unicode file").unwrap();
        let token = CancellationToken::new();
        let payload = inventory(&root, &token).unwrap();
        let records:Vec<_>=payload.iter().map(|(name,sha)| json!({"path":name,"sha256":sha,"size":fs::metadata(root.join(name)).unwrap().len()})).collect();
        fs::write(root.join("rocket-update.json"),serde_json::to_vec(&json!({"schema":1,"version":"9.0.0","platform":platform().unwrap(),"entrypoint":ENTRYPOINT,"files":records})).unwrap()).unwrap();
        let sums = inventory(&root, &token)
            .unwrap()
            .iter()
            .map(|(name, sha)| format!("{sha}  {name}\n"))
            .collect::<String>();
        fs::write(root.join("SHA256SUMS.txt"), sums).unwrap();
        root
    }
    fn cleanup(root: PathBuf) {
        // Exact freshly-created fixture, under the resolved OS temporary directory.
        assert!(
            root.canonicalize()
                .unwrap()
                .starts_with(std::env::temp_dir().canonicalize().unwrap())
        );
        assert!(
            root.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("update-")
        );
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn complete_package_verifies_and_can_be_verified_after_installation() {
        let root = fixture();
        let token = CancellationToken::new();
        let result = manifest(&root, UpdateVersion(9, 0, 0), &token).unwrap();
        assert_eq!(result, installed(&root, &token).unwrap());
        assert!(result.contains_key("資料/README.md"));
        assert!(manifest(&root, UpdateVersion(9, 0, 1), &token).is_err());
        cleanup(root);
    }
    #[test]
    fn update_missing_ssh_helper_is_rejected_even_with_matching_manifests() {
        let root = fixture();
        // The SSH helper comes last on every platform.
        let ssh = *BINARIES.last().unwrap();
        fs::remove_file(root.join(ssh)).unwrap();
        let update = root.join("rocket-update.json");
        let mut json: Value = serde_json::from_slice(&fs::read(&update).unwrap()).unwrap();
        json["files"]
            .as_array_mut()
            .unwrap()
            .retain(|entry| entry["path"] != ssh);
        fs::write(update, serde_json::to_vec(&json).unwrap()).unwrap();
        let token = CancellationToken::new();
        let sums = inventory(&root, &token)
            .unwrap()
            .iter()
            .map(|(name, sha)| format!("{sha}  {name}\n"))
            .collect::<String>();
        fs::write(root.join("SHA256SUMS.txt"), sums).unwrap();
        assert!(manifest(&root, UpdateVersion(9, 0, 0), &token).is_err());
        assert!(installed(&root, &token).is_err());
        cleanup(root);
    }
    #[test]
    fn changed_and_extra_files_are_preserved_and_rejected() {
        let root = fixture();
        let token = CancellationToken::new();
        fs::write(root.join("personal.txt"), "keep me").unwrap();
        assert!(installed(&root, &token).is_err());
        assert_eq!(fs::read(root.join("personal.txt")).unwrap(), b"keep me");
        fs::remove_file(root.join("personal.txt")).unwrap();
        fs::write(root.join(ENTRYPOINT), "changed binary!").unwrap();
        assert!(manifest(&root, UpdateVersion(9, 0, 0), &token).is_err());
        cleanup(root);
    }
    #[test]
    fn cancelled_validation_does_not_modify_package() {
        let root = fixture();
        let token = CancellationToken::new();
        let before = inventory(&root, &token).unwrap();
        token.cancel();
        assert!(matches!(
            manifest(&root, UpdateVersion(9, 0, 0), &token),
            Err(Error::Cancelled)
        ));
        assert_eq!(before, inventory(&root, &CancellationToken::new()).unwrap());
        cleanup(root);
    }
    /// The archive tools/package-release.sh made passes the checks an update runs, from
    /// unpacking to the installed-release check that the next update starts from.
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    #[test]
    #[ignore = "needs ROCKET_TEST_UPDATE_ARCHIVE and ROCKET_TEST_UPDATE_VERSION from tools/package-release.sh"]
    fn a_packaged_linux_release_passes_the_update_checks() {
        let archive = PathBuf::from(std::env::var_os("ROCKET_TEST_UPDATE_ARCHIVE").unwrap());
        let version =
            UpdateVersion::parse(&std::env::var("ROCKET_TEST_UPDATE_VERSION").unwrap()).unwrap();
        let token = CancellationToken::new();
        let job = unique(&std::env::temp_dir()).unwrap();
        let client =
            UpdateClient::new(rocket_git::app_update::UpdateEndpoint::Loopback(9), None).unwrap();
        let bundle = job.join("bundle");
        client.extract(&archive, &bundle, &token).unwrap();
        let files = manifest(&bundle, version, &token).unwrap();
        for name in BINARIES {
            assert!(files.contains_key(name), "{name}");
        }
        assert_eq!(files, installed(&bundle, &token).unwrap());
        // As installed, its programs run and nothing else does.
        let installation = job.join("installation");
        copy_tree(&bundle, &installation, &files, &token).unwrap();
        use std::os::unix::fs::PermissionsExt;
        for name in files.keys() {
            let mode = fs::metadata(installation.join(name))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(
                mode & 0o111 != 0,
                BINARIES.contains(&name.as_str()),
                "{name}"
            );
        }
        assert_eq!(files, installed(&installation, &token).unwrap());
        cleanup(job);
    }
}
