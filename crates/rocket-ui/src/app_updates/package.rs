//! Release archives are data. No script or executable from a download runs here.
use super::progress::{FilePhase, RestartProgress};
use rocket_git::{
    CancellationToken, Error, Result,
    app_update::{AppRelease, UpdateClient, UpdateVersion},
    osv_file_digest, sha256_reader,
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Read,
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicBool, AtomicU8, Ordering},
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
#[cfg(not(any(windows, target_os = "macos")))]
pub const BINARIES: [&str; 6] = [
    "rocket",
    "rocket-mcp",
    "rocket-cli",
    "rocket-rebase-editor",
    "rocket-lua-host",
    "rocket-ssh",
];
#[cfg(target_os = "macos")]
pub const BINARIES: [&str; 5] = [
    "Contents/MacOS/rocket", "Contents/MacOS/rocket-mcp", "Contents/MacOS/rocket-cli",
    "Contents/MacOS/rocket-rebase-editor", "Contents/MacOS/rocket-ssh",
];
pub const ENTRYPOINT: &str = BINARIES[0];
pub fn prepared_root(job: &Path) -> PathBuf {
    let bundle = job.join("bundle");
    if cfg!(target_os = "macos") { bundle.join("Rocket.app") } else { bundle }
}
pub fn is_packaged(root: &Path) -> bool {
    root.join(if cfg!(target_os = "macos") { "Contents/Info.plist" } else { "SHA256SUMS.txt" }).is_file()
}
#[cfg(unix)]
pub fn executable_path(name: &str) -> bool {
    BINARIES.contains(&name) || (cfg!(target_os = "macos") && name == "Contents/Helpers/RocketLua.app/Contents/MacOS/rocket-lua-host")
}
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
fn reject_link(metadata: &fs::Metadata) -> Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return Err(error("Update path contains a link or reparse point."));
        }
    }
    if metadata.file_type().is_symlink() { return Err(error("Update path contains a link.")); }
    Ok(())
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
            Ok(m) => reject_link(&m)?,
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
type FileCounter<'a> = Option<(&'a RestartProgress, FilePhase)>;

// Each worker owns a disjoint slice. Collect and bound the inventory before
// starting I/O, join every worker before returning, and count only successful files.
fn process_files<T: Send>(
    files: &mut [T],
    cancel: &CancellationToken,
    counter: FileCounter<'_>,
    operation: impl Fn(&mut T) -> Result<()> + Sync,
) -> Result<()> {
    if let Some((progress, phase)) = counter {
        progress.begin(phase, files.len());
    }
    let failed = AtomicBool::new(false);
    let work = |chunk: &mut [T]| {
        for file in chunk {
            if failed.load(Ordering::Relaxed) { break; }
            if let Err(e) = cancel.check().and_then(|()| operation(file)) {
                failed.store(true, Ordering::Relaxed);
                return Err(e);
            }
            if let Some((progress, _)) = counter { progress.file_done(); }
        }
        Ok(())
    };
    // Small packages do not need a worker pool. Four is also the limit used by
    // bundled Git verification, keeping simultaneous disk reads and buffers bounded.
    let workers = std::thread::available_parallelism().map_or(1, usize::from)
        .min(4).min(files.len().div_ceil(64));
    if workers <= 1 { return work(files); }
    let chunk_size = files.len().div_ceil(workers);
    std::thread::scope(|scope| {
        let mut jobs = Vec::with_capacity(workers);
        for chunk in files.chunks_mut(chunk_size) {
            let job = std::thread::Builder::new().spawn_scoped(scope, || work(chunk))
                .map_err(|e| { failed.store(true, Ordering::Relaxed); io(e) })?;
            jobs.push(job);
        }
        for job in jobs {
            job.join().map_err(|_| error("Update file worker stopped unexpectedly."))??;
        }
        Ok(())
    })
}

pub(super) fn inventory(root: &Path, cancel: &CancellationToken, counter: FileCounter<'_>) -> Result<BTreeMap<String, String>> {
    let mut pending = vec![root.to_path_buf()];
    let mut files = Vec::new();
    let mut names = BTreeSet::new();
    let mut entries = 0;
    let mut total = 0u64;
    while let Some(dir) = pending.pop() {
        cancel.check()?;
        plain(&dir)?;
        for item in fs::read_dir(&dir).map_err(io)? {
            cancel.check()?;
            entries += 1;
            if entries > 20000 {
                return Err(error("Release contains too many entries."));
            }
            let item = item.map_err(io)?;
            let path = item.path();
            // DirEntry metadata does not follow symlinks. Check every directory
            // before descent and each file's full ancestry immediately before reading.
            let m = item.metadata().map_err(io)?;
            reject_link(&m)?;
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
            if files.len() >= 20000 || m.len() > MAX_FILE || total > MAX_TREE {
                return Err(error("Unpacked release exceeds its size limit."));
            }
            files.push((name, path, m.len(), String::new()));
        }
    }
    process_files(&mut files, cancel, counter, |(_, path, size, hash)| {
        plain(path)?;
        // Keep each read within the size used for the aggregate tree limit,
        // including if a file changes between directory enumeration and hashing.
        let mut reader = fs::File::open(path).map_err(io)?.take(*size + 1);
        *hash = sha256_reader(&mut reader, *size, cancel)?;
        if reader.limit() != 1 { return Err(error("Release file length changed during verification.")); }
        Ok(())
    })?;
    Ok(files.into_iter().map(|(name, _, _, hash)| (name, hash)).collect())
}
pub fn manifest(
    bundle: &Path,
    version: UpdateVersion,
    cancel: &CancellationToken,
    counter: FileCounter<'_>,
) -> Result<BTreeMap<String, String>> {
    let value = json_file(&bundle.join("rocket-update.json"))?;
    #[cfg(target_os = "macos")]
    let app = bundle.join("Rocket.app");
    #[cfg(target_os = "macos")]
    let bundle = app.as_path();
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
    let actual = installed(bundle, cancel, counter)?;
    #[cfg(target_os = "macos")]
    rocket_git::app_update::macos::verify_app(bundle, Some(version), cancel)?;
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
#[cfg(target_os = "macos")]
pub fn installed(root: &Path, cancel: &CancellationToken, counter: FileCounter<'_>) -> Result<BTreeMap<String, String>> {
    plain(root)?;
    let files = inventory(root, cancel, counter)?;
    for name in BINARIES {
        if !files.contains_key(name) { return Err(error("The Mac update is missing a required Rocket executable.")); }
    }
    rocket_git::app_update::macos::verify_app(root, None, cancel)?;
    Ok(files)
}
#[cfg(not(target_os = "macos"))]
pub fn installed(root: &Path, cancel: &CancellationToken, counter: FileCounter<'_>) -> Result<BTreeMap<String, String>> {
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
    if inventory(root, cancel, counter)? != expected {
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
    #[cfg(target_os = "macos")]
    let resolved = rocket_git::local_storage::resolve_system_alias(parent).map_err(io)?;
    #[cfg(target_os = "macos")]
    let parent = resolved.as_path();
    plain(parent)?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| error("System clock is unavailable."))?
        .as_nanos();
    let job = parent.join(format!("update-{nonce}-{}", std::process::id()));
    #[cfg(not(windows))]
    rocket_git::local_storage::create_private_new(&job).map_err(io)?;
    #[cfg(windows)]
    {
        fs::create_dir_all(parent).map_err(io)?;
        fs::create_dir(&job).map_err(io)?;
    }
    Ok(job)
}
pub fn prepare(
    client: &UpdateClient,
    release: &AppRelease,
    cache: &Path,
    phase: &AtomicU8,
    cancel: &CancellationToken,
) -> Result<Prepared> {
    #[cfg(unix)]
    rocket_git::local_storage::create_protected(cache).map_err(io)?;
    let job = unique(cache)?;
    match prepare_files(client, release, &job, phase, cancel) {
        Ok(files) => Ok(Prepared { job, version: release.version, files }),
        Err(failure) => {
            // Only this fresh, unpublished job belongs to preparation. Installed
            // trees, recovery backups and other attempts live outside it.
            if let Err(cleanup) = plain(&job).and_then(|()| fs::remove_dir_all(&job).map_err(io)) {
                return Err(error(&format!("{failure}. Could not remove incomplete update: {cleanup}")));
            }
            Err(failure)
        }
    }
}

fn prepare_files(
    client: &UpdateClient,
    release: &AppRelease,
    job: &Path,
    phase: &AtomicU8,
    cancel: &CancellationToken,
) -> Result<BTreeMap<String, String>> {
    phase.store(1, Ordering::Release);
    let archive = client.download(release, job, cancel)?;
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
    #[cfg(target_os = "macos")]
    {
        use std::os::unix::fs::PermissionsExt;
        let names: BTreeSet<_> = fs::read_dir(&bundle).map_err(io)?.map(|e| e.map(|e| e.file_name())).collect::<std::io::Result<_>>().map_err(io)?;
        if names != [std::ffi::OsString::from("Rocket.app"), std::ffi::OsString::from("rocket-update.json")].into_iter().collect() {
            return Err(error("Mac update must contain exactly Rocket.app and its update manifest."));
        }
        for name in BINARIES.into_iter().chain(std::iter::once("Contents/Helpers/RocketLua.app/Contents/MacOS/rocket-lua-host")) {
            let path = prepared_root(job).join(name);
            plain(&path)?;
            if path.exists() { fs::set_permissions(path, fs::Permissions::from_mode(0o755)).map_err(io)?; }
        }
    }
    phase.store(3, Ordering::Release);
    let files = manifest(&bundle, release.version, cancel, None)?;
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
    cancel.check()?;
    Ok(files)
}
pub fn copy_tree(
    source: &Path,
    destination: &Path,
    files: &BTreeMap<String, String>,
    cancel: &CancellationToken,
    counter: FileCounter<'_>,
) -> Result<()> {
    plain(destination)?;
    fs::create_dir(destination).map_err(io)?;
    let mut entries: Vec<_> = files.iter().collect();
    process_files(&mut entries, cancel, counter, |&mut (name, sha)| {
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
            options.mode(if executable_path(name) {
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
        Ok(())
    })
}

#[cfg(all(test, not(target_os = "macos")))]
mod tests {
    use super::*;
    #[cfg(unix)]
    #[test]
    fn update_jobs_are_private_and_shared_parents_get_no_job() {
        use std::os::unix::fs::PermissionsExt;
        let root = unique(&std::env::temp_dir()).unwrap();
        assert_eq!(fs::metadata(&root).unwrap().permissions().mode() & 0o777, 0o700);
        let cache = root.join("cache");
        fs::create_dir(&cache).unwrap();
        fs::set_permissions(&cache, fs::Permissions::from_mode(0o777)).unwrap();
        assert!(unique(&cache).is_err());
        assert_eq!(fs::read_dir(&cache).unwrap().count(), 0);
        cleanup(root);
    }

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
        let payload = inventory(&root, &token, None).unwrap();
        let records:Vec<_>=payload.iter().map(|(name,sha)| json!({"path":name,"sha256":sha,"size":fs::metadata(root.join(name)).unwrap().len()})).collect();
        fs::write(root.join("rocket-update.json"),serde_json::to_vec(&json!({"schema":1,"version":"9.0.0","platform":platform().unwrap(),"entrypoint":ENTRYPOINT,"files":records})).unwrap()).unwrap();
        let sums = inventory(&root, &token, None)
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
    fn restart_progress_counts_only_verified_files_and_resets_between_phases() {
        let root = fixture();
        let token = CancellationToken::new();
        for n in 0..256 { fs::write(root.join(format!("document-{n}.md")), b"fixture").unwrap(); }
        let progress = RestartProgress::default();
        let files = inventory(&root, &token, Some((&progress, FilePhase::CheckInstalled))).unwrap();
        let verified = progress.snapshot().unwrap();
        assert_eq!((verified.phase, verified.done, verified.total), (FilePhase::CheckInstalled, files.len(), files.len()));

        let parent = unique(&std::env::temp_dir()).unwrap();
        let stage = parent.join("stage");
        copy_tree(&root, &stage, &files, &token, Some((&progress, FilePhase::Stage))).unwrap();
        let staged = progress.snapshot().unwrap();
        assert_eq!((staged.phase, staged.done, staged.total), (FilePhase::Stage, files.len(), files.len()));
        assert_eq!(inventory(&stage, &token, None).unwrap(), files);

        fs::write(root.join("document-0.md"), b"edited fixture").unwrap();
        assert!(copy_tree(&root, &parent.join("changed"), &files, &token, Some((&progress, FilePhase::Stage))).is_err());
        assert!(progress.snapshot().unwrap().done < files.len());
        token.cancel();
        assert!(copy_tree(&root, &parent.join("cancelled"), &files, &token, Some((&progress, FilePhase::Stage))).is_err());
        assert_eq!(progress.snapshot().unwrap().done, 0);
        cleanup(parent);
        cleanup(root);
    }
    #[test]
    fn complete_package_verifies_and_can_be_verified_after_installation() {
        let root = fixture();
        let token = CancellationToken::new();
        let result = manifest(&root, UpdateVersion(9, 0, 0), &token, None).unwrap();
        assert_eq!(result, installed(&root, &token, None).unwrap());
        assert!(result.contains_key("資料/README.md"));
        assert!(manifest(&root, UpdateVersion(9, 0, 1), &token, None).is_err());
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
        let sums = inventory(&root, &token, None)
            .unwrap()
            .iter()
            .map(|(name, sha)| format!("{sha}  {name}\n"))
            .collect::<String>();
        fs::write(root.join("SHA256SUMS.txt"), sums).unwrap();
        assert!(manifest(&root, UpdateVersion(9, 0, 0), &token, None).is_err());
        assert!(installed(&root, &token, None).is_err());
        cleanup(root);
    }
    #[test]
    fn changed_and_extra_files_are_preserved_and_rejected() {
        let root = fixture();
        let token = CancellationToken::new();
        fs::write(root.join("personal.txt"), "keep me").unwrap();
        assert!(installed(&root, &token, None).is_err());
        assert_eq!(fs::read(root.join("personal.txt")).unwrap(), b"keep me");
        fs::remove_file(root.join("personal.txt")).unwrap();
        fs::write(root.join(ENTRYPOINT), "changed binary!").unwrap();
        assert!(manifest(&root, UpdateVersion(9, 0, 0), &token, None).is_err());
        cleanup(root);
    }
    #[test]
    fn cancelled_validation_does_not_modify_package() {
        let root = fixture();
        let token = CancellationToken::new();
        let before = inventory(&root, &token, None).unwrap();
        token.cancel();
        assert!(matches!(
            manifest(&root, UpdateVersion(9, 0, 0), &token, None),
            Err(Error::Cancelled)
        ));
        assert_eq!(before, inventory(&root, &CancellationToken::new(), None).unwrap());
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
        let files = manifest(&bundle, version, &token, None).unwrap();
        for name in BINARIES {
            assert!(files.contains_key(name), "{name}");
        }
        assert_eq!(files, installed(&bundle, &token, None).unwrap());
        // As installed, its programs run and nothing else does.
        let installation = job.join("installation");
        copy_tree(&bundle, &installation, &files, &token, None).unwrap();
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
        assert_eq!(files, installed(&installation, &token, None).unwrap());
        cleanup(job);
    }
}
