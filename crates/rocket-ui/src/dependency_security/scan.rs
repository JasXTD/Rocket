//! Snapshot-only offline scanning. Repository files are data, never configuration.
#[path = "npm_coverage.rs"]
mod npm_coverage;
use super::limits::{Limits, MIB};
use rocket_git::{CancellationToken, Error, OSV_VERSION, Result, osv_file_digest, run_offline_osv};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Read,
    ops::Range,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

/// rocket-git refuses an OSV run with more lockfiles than this.
const FILES_PER_RUN: usize = 256;
/// Lockfile bytes one run may take: a whole scan's default before Scan limits, so no
/// run outgrows what OSV's fixed time, output and package caps were sized for.
const BYTES_PER_RUN: u64 = 64 * MIB;
/// Packages one OSV result may list.
const PACKAGES_PER_RUN: usize = 50_000;
/// Characters of `--lockfile` arguments one run may take. Windows allows a command line
/// 32,767; the program, its fixed arguments and the configuration path need the rest.
const CHARACTERS_PER_RUN: usize = 24_000;
/// Words every scan-limit stop says, so the page can tell a limit from other failures.
const LIMIT_WORDS: &str = "Scan limits allow";
const SKIP: &[&str] = &[
    ".git",
    "node_modules",
    "target",
    ".venv",
    "venv",
    ".gradle",
    ".next",
    "vendor",
];
// The official npm archive is already ~219 MB (2026-10-25). Keep a meaningful
// cap above the current feed size rather than silently rejecting a valid feed.
const DB_LIMIT: u64 = 512 * 1024 * 1024;
const FORMATS: &[(&str, &str)] = &[
    ("Cargo.lock", "crates.io"),
    ("package-lock.json", "npm"),
    ("npm-shrinkwrap.json", "npm"),
    ("pnpm-lock.yaml", "npm"),
    ("yarn.lock", "npm"),
    ("bun.lock", "npm"),
    ("uv.lock", "PyPI"),
    ("poetry.lock", "PyPI"),
    ("Pipfile.lock", "PyPI"),
    ("pdm.lock", "PyPI"),
    ("pylock.toml", "PyPI"),
    ("requirements.txt", "PyPI"),
    ("go.mod", "Go"),
    ("packages.lock.json", "NuGet"),
    ("packages.config", "NuGet"),
    ("gradle.lockfile", "Maven"),
    ("buildscript-gradle.lockfile", "Maven"),
    ("composer.lock", "Packagist"),
    ("Gemfile.lock", "RubyGems"),
    ("pubspec.lock", "Pub"),
    ("mix.lock", "Hex"),
    ("conan.lock", "ConanCenter"),
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Input {
    pub path: PathBuf,
    pub ecosystem: String,
    pub bytes: Vec<u8>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Inventory {
    pub files: Vec<Input>,
    pub notes: Vec<String>,
}
impl Inventory {
    pub fn ecosystems(&self) -> BTreeSet<String> {
        self.files.iter().map(|f| f.ecosystem.clone()).collect()
    }
    /// Each file's path, ecosystem and SHA-256, with the coverage notes: what a result
    /// is kept by, so only a running scan holds the files themselves.
    pub fn fingerprint(&self) -> Result<Fingerprint> {
        let mut files = Vec::with_capacity(self.files.len());
        for file in &self.files {
            files.push((file.path.clone(), file.ecosystem.clone(), rocket_git::sha256_bytes(&file.bytes)?));
        }
        Ok(Fingerprint { files, notes: self.notes.clone() })
    }
}
/// See [`Inventory::fingerprint`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Fingerprint {
    files: Vec<(PathBuf, String, String)>,
    notes: Vec<String>,
}
impl Fingerprint {
    pub fn ecosystems(&self) -> BTreeSet<String> {
        self.files.iter().map(|f| f.1.clone()).collect()
    }
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Database {
    pub ecosystem: String,
    pub digest: String,
    pub modified: u64,
}
#[derive(Clone, Debug)]
pub struct Finding {
    pub package: String,
    pub version: String,
    pub ecosystem: String,
    pub file: String,
    pub ids: Vec<String>,
    pub severity: String,
    pub summary: String,
    pub fixed: Vec<String>,
}
#[derive(Clone, Debug)]
pub struct Report {
    pub packages: usize,
    pub files: usize,
    pub findings: Vec<Finding>,
    pub notes: Vec<String>,
    pub checked: u64,
    pub oldest_database: u64,
}
#[derive(Clone)]
pub struct Cached {
    /// The files this result covers, by fingerprint.
    pub inventory: Fingerprint,
    pub database: Vec<Database>,
    pub report: Arc<Report>,
}

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}
fn invalid(message: &str) -> Error {
    Error::InvalidInput(message.into())
}
/// A scan limit stopped the scan before OSV ran. Every such message says
/// "Scan limits allow", which [`stopped_at_limit`] looks for.
pub(super) fn stop(message: String) -> Error {
    debug_assert!(message.contains(LIMIT_WORDS));
    Error::InvalidInput(message)
}
/// The scan stopped at one of the user's Scan limits, which the user can raise.
pub fn stopped_at_limit(error: &Error) -> bool {
    matches!(error, Error::InvalidInput(message) if message.contains(LIMIT_WORDS))
}
/// MiB to one decimal for a size over `limit` MiB, never shown as the limit itself.
fn mib_over(bytes: u64, limit: u64) -> String {
    let tenths = (bytes.saturating_mul(10).saturating_add(MIB / 2) / MIB).max(limit * 10 + 1);
    if tenths.is_multiple_of(10) { (tenths / 10).to_string() } else { format!("{}.{}", tenths / 10, tenths % 10) }
}
fn shown_path(path: &Path) -> String {
    clean(&path.to_string_lossy().replace('\\', "/"), 300)
}
fn count_stop(found: usize, limits: &Limits) -> Error {
    stop(format!(
        "{} dependency files found. {LIMIT_WORDS} {} files.",
        crate::pages::number_text(found),
        crate::pages::number_text(limits.files)
    ))
}
fn io(action: &'static str, source: std::io::Error) -> Error {
    Error::Io { action, source }
}
fn reparse(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}

fn manifest_ecosystem(name: &str) -> Option<&'static str> {
    match name {
        "Cargo.toml" => Some("crates.io"),
        "package.json" => Some("npm"),
        "pyproject.toml" | "Pipfile" | "setup.py" | "setup.cfg" => Some("PyPI"),
        "pom.xml" | "build.gradle" | "build.gradle.kts" => Some("Maven"),
        "composer.json" => Some("Packagist"),
        "Gemfile" => Some("RubyGems"),
        "pubspec.yaml" => Some("Pub"),
        "mix.exs" => Some("Hex"),
        "conanfile.py" | "conanfile.txt" => Some("ConanCenter"),
        _ if name.ends_with(".csproj")
            || name.ends_with(".fsproj")
            || name.ends_with(".vbproj") =>
        {
            Some("NuGet")
        }
        _ => None,
    }
}

/// The working copy's dependency files within `limits`. Every file is found and measured
/// first, so a stop says how many there are or how large, and nothing is read past them.
pub fn inventory(root: &Path, limits: &Limits, cancel: &CancellationToken) -> Result<Inventory> {
    if !root.is_absolute() {
        return Err(invalid(
            "Open a local repository before scanning dependencies.",
        ));
    }
    let root = root
        .canonicalize()
        .map_err(|e| io("read dependency root", e))?;
    let mut pending = vec![(root.clone(), 0usize)];
    // Path, path in the repository, ecosystem and size, for each dependency file.
    let mut found: Vec<(PathBuf, PathBuf, &'static str, u64)> = Vec::new();
    let mut notes = BTreeSet::new();
    let mut manifests = Vec::new();
    let mut entries = 0;
    while let Some((folder, depth)) = pending.pop() {
        cancel.check()?;
        if depth > 24 {
            return Err(invalid(
                "Dependency discovery exceeded the folder-depth limit.",
            ));
        }
        for entry in fs::read_dir(folder).map_err(|e| io("list dependency files", e))? {
            cancel.check()?;
            let entry = entry.map_err(|e| io("read dependency entry", e))?;
            entries += 1;
            if entries > 50_000 {
                return Err(invalid(
                    "Dependency discovery exceeded 50,000 entries. Scan a smaller repository.",
                ));
            }
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if SKIP.contains(&name.as_ref()) {
                continue;
            }
            let path = entry.path();
            let metadata =
                fs::symlink_metadata(&path).map_err(|e| io("inspect dependency file", e))?;
            if reparse(&metadata) {
                notes.insert("Symbolic links and junctions were skipped; their dependencies are not covered.".into());
                continue;
            }
            if metadata.is_dir() {
                pending.push((path, depth + 1));
                continue;
            }
            if !metadata.is_file() {
                continue;
            }
            let relative = path
                .strip_prefix(&root)
                .map_err(|_| invalid("Dependency file is outside the repository."))?
                .to_path_buf();
            if let Some(ecosystem) = manifest_ecosystem(&name) {
                manifests.push((relative.clone(), ecosystem));
            }
            let Some((_, ecosystem)) = FORMATS.iter().find(|(file, _)| *file == name) else {
                continue;
            };
            found.push((path, relative, *ecosystem, metadata.len()));
        }
    }
    found.sort_by(|a, b| a.1.cmp(&b.1));
    if found.len() > limits.files {
        return Err(count_stop(found.len(), limits));
    }
    if let Some((_, relative, _, size)) = found.iter().find(|f| f.3 > limits.file_bytes()) {
        return Err(stop(format!(
            "{} is {} MiB. {LIMIT_WORDS} {} MiB a file.",
            shown_path(relative),
            mib_over(*size, limits.file_mib),
            limits.file_mib
        )));
    }
    let sizes: u64 = found.iter().map(|f| f.3).sum();
    if sizes > limits.total_bytes() {
        return Err(stop(format!(
            "Dependency files add up to {} MiB. {LIMIT_WORDS} {} MiB in all.",
            mib_over(sizes, limits.total_mib),
            limits.total_mib
        )));
    }
    let mut files = Vec::with_capacity(found.len());
    let mut total = 0u64;
    for (path, relative, ecosystem, _) in found {
        cancel.check()?;
        let canonical = path
            .canonicalize()
            .map_err(|e| io("resolve dependency file", e))?;
        if !canonical.starts_with(&root) {
            return Err(invalid("Dependency file resolves outside the repository."));
        }
        let mut bytes = Vec::new();
        fs::File::open(&path)
            .map_err(|e| io("open dependency file", e))?
            .take(limits.file_bytes() + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| io("read dependency file", e))?;
        // Measured above; a file that grew since is not scanned part-way.
        if bytes.len() as u64 > limits.file_bytes() {
            return Err(invalid("A dependency file grew beyond the scan limit."));
        }
        total += bytes.len() as u64;
        if total > limits.total_bytes() {
            return Err(invalid("Dependency files grew beyond the scan limit."));
        }
        if matches!(relative.file_name().and_then(|n| n.to_str()), Some("requirements.txt" | "go.mod")) {
            notes.insert("Manifest-only dependencies may omit resolved or indirect versions; no package manager was run.".into());
        }
        files.push(Input {
            path: relative,
            ecosystem: ecosystem.into(),
            bytes,
        });
    }
    Ok(complete_inventory(files, manifests, notes))
}

/// Scan the immutable tip actually being pushed, never the index or working copy.
pub fn committed_inventory(
    repo: &rocket_git::Repository,
    source: &rocket_git::ObjectId,
    limits: &Limits,
    cancel: &CancellationToken,
) -> Result<Inventory> {
    use rocket_git::TreeEntryKind;
    let mut pending = vec![(repo.repository_tree(source.as_str(), cancel)?, 0usize)];
    let mut found = Vec::new();
    let mut manifests = Vec::new();
    let mut notes = BTreeSet::new();
    let mut entries = 0usize;
    while let Some((listing, depth)) = pending.pop() {
        cancel.check()?;
        if depth > 24 {
            return Err(invalid(
                "Dependency discovery exceeded the folder-depth limit.",
            ));
        }
        for entry in listing.entries {
            cancel.check()?;
            entries += 1;
            if entries > 50_000 {
                return Err(invalid("Dependency discovery exceeded 50,000 entries."));
            }
            let name = String::from_utf8_lossy(entry.name());
            if SKIP.contains(&name.as_ref()) {
                continue;
            }
            match entry.kind() {
                TreeEntryKind::Directory => {
                    pending.push((
                        repo.tree_directory(&entry.directory().unwrap(), cancel)?,
                        depth + 1,
                    ));
                    continue;
                }
                TreeEntryKind::Symlink | TreeEntryKind::Submodule => {
                    notes.insert("Symbolic links and submodule contents were skipped; their dependencies are not covered.".into());
                    continue;
                }
                TreeEntryKind::File | TreeEntryKind::Executable => {}
            }
            let manifest = manifest_ecosystem(&name);
            let format = FORMATS.iter().find(|(file, _)| *file == name);
            if manifest.is_none() && format.is_none() {
                continue;
            }
            let relative = PathBuf::from(std::str::from_utf8(entry.path()).map_err(|_| {
                invalid("A dependency path cannot be represented safely. The push was not scanned.")
            })?);
            if let Some(ecosystem) = manifest {
                manifests.push((relative.clone(), ecosystem));
            }
            let Some((_, ecosystem)) = format else {
                continue;
            };
            found.push((entry, relative, *ecosystem));
        }
    }
    found.sort_by(|a, b| a.1.cmp(&b.1));
    if found.len() > limits.files {
        return Err(count_stop(found.len(), limits));
    }
    let mut files = Vec::with_capacity(found.len());
    let mut total = 0u64;
    for (entry, relative, ecosystem) in found {
        cancel.check()?;
        // Git refuses a blob over the limit from its size, before reading it.
        let bytes = match repo.tree_file_with_limit(&entry, limits.file_bytes() as usize, cancel) {
            Ok(file) => file.bytes,
            Err(Error::Unsupported(_)) => {
                return Err(stop(format!(
                    "{} is over {} MiB. {LIMIT_WORDS} {} MiB a file.",
                    shown_path(&relative),
                    limits.file_mib,
                    limits.file_mib
                )));
            }
            Err(error) => return Err(error),
        };
        total += bytes.len() as u64;
        if total > limits.total_bytes() {
            return Err(stop(format!(
                "Dependency files add up to over {} MiB. {LIMIT_WORDS} {} MiB in all.",
                limits.total_mib, limits.total_mib
            )));
        }
        if matches!(relative.file_name().and_then(|n| n.to_str()), Some("requirements.txt" | "go.mod")) {
            notes.insert("Manifest-only dependencies may omit resolved or indirect versions; no package manager was run.".into());
        }
        files.push(Input {
            path: relative,
            ecosystem: ecosystem.into(),
            bytes,
        });
    }
    Ok(complete_inventory(files, manifests, notes))
}

fn complete_inventory(
    mut files: Vec<Input>,
    manifests: Vec<(PathBuf, &'static str)>,
    mut notes: BTreeSet<String>,
) -> Inventory {
    files.sort_by(|a, b| a.path.cmp(&b.path));
    for (manifest, ecosystem) in manifests {
        let folder = manifest.parent().unwrap_or(Path::new(""));
        if !files
            .iter()
            .any(|f| f.ecosystem == ecosystem && f.path.parent() == manifest.parent())
        {
            // Workspaces can share a root lockfile, but an unrelated parent project
            // can also have one. Do not silently claim complete child coverage.
            let parent_lock = files.iter().any(|f| {
                f.ecosystem == ecosystem && f.path.parent().is_some_and(|p| folder.starts_with(p))
            });
            notes.insert(if parent_lock {
                "Some manifests rely on an ancestor lockfile; verify that their packages belong to that workspace."
            } else {
                "Some dependency manifests have no matching supported lockfile; coverage may be incomplete."
            }.into());
        }
    }
    Inventory {
        files,
        notes: notes.into_iter().collect(),
    }
}

pub fn databases(
    cache: &Path,
    ecosystems: &BTreeSet<String>,
    cancel: &CancellationToken,
) -> Result<Vec<Database>> {
    let mut result = Vec::new();
    for ecosystem in ecosystems {
        cancel.check()?;
        // OSV 2.7 uses osv-scalibr, despite older documentation naming osv-scanner.
        let file = cache.join("osv-scalibr").join(ecosystem).join("all.zip");
        let metadata = fs::symlink_metadata(&file).map_err(|_| {
            invalid("Advisory data is missing. Choose Update advisory data, then scan again.")
        })?;
        if !metadata.is_file() || reparse(&metadata) || metadata.len() > DB_LIMIT {
            return Err(invalid(
                "Advisory data is invalid or exceeds the 512 MiB limit per ecosystem.",
            ));
        }
        let digest = osv_file_digest(&file, DB_LIMIT, cancel)?;
        let modified = metadata
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_secs());
        result.push(Database {
            ecosystem: ecosystem.clone(),
            digest,
            modified,
        });
    }
    Ok(result)
}

pub struct Scratch {
    pub path: PathBuf,
}
impl Scratch {
    pub fn new(base: &Path) -> Result<Self> {
        fs::create_dir_all(base).map_err(|e| io("create scanner workspace", e))?;
        let base = base
            .canonicalize()
            .map_err(|e| io("resolve scanner workspace", e))?;
        static NEXT: AtomicU64 = AtomicU64::new(0);
        for _ in 0..20 {
            let name = format!(
                "scan-{}-{}-{}",
                std::process::id(),
                now(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            );
            let path = base.join(name);
            match fs::create_dir(&path) {
                Ok(()) => {
                    let scratch = Self { path };
                    fs::write(
                        scratch.path.join("osv-scanner.toml"),
                        b"# Rocket: no project overrides or ignored advisories.\n",
                    )
                    .map_err(|e| io("create scanner configuration", e))?;
                    return Ok(scratch);
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(io("create scanner snapshot", e)),
            }
        }
        Err(invalid("Could not allocate a unique scanner snapshot."))
    }
    fn remove(&self) -> Result<()> {
        fs::remove_dir_all(&self.path).map_err(|e| io("remove dependency snapshot", e))
    }
    fn copy(&self, inv: &Inventory) -> Result<Vec<PathBuf>> {
        let mut paths = Vec::new();
        for (index, file) in inv.files.iter().enumerate() {
            let folder = self.path.join(index.to_string());
            fs::create_dir(&folder).map_err(|e| io("create dependency snapshot", e))?;
            let path = folder.join(
                file.path
                    .file_name()
                    .ok_or_else(|| invalid("Missing lockfile name."))?,
            );
            fs::write(&path, &file.bytes).map_err(|e| io("copy dependency snapshot", e))?;
            paths.push(path);
        }
        Ok(paths)
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        // Only this create-new, canonical-base-owned directory is removed.
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[allow(clippy::too_many_arguments)]
pub fn scan(
    root: &Path,
    executable: &Path,
    cache: &Path,
    work: &Path,
    previous: Option<&Cached>,
    force: bool,
    limits: &Limits,
    cancel: &CancellationToken,
) -> Result<Cached> {
    let inv = inventory(root, limits, cancel)?;
    let cached = scan_inventory(inv, executable, cache, work, previous, force, cancel)?;
    // What OSV saw must still be there. Any other stop now (a file added past a limit,
    // one gone) means the files changed during the scan, not that a limit stopped it.
    let changed = || invalid("Dependency files or advisory data changed during the scan. Scan again.");
    let again = match inventory(root, limits, cancel) {
        Ok(again) => again,
        Err(Error::Cancelled) => return Err(Error::Cancelled),
        Err(_) => return Err(changed()),
    };
    if again.fingerprint()? != cached.inventory {
        return Err(changed());
    }
    Ok(cached)
}

pub fn scan_inventory(
    inv: Inventory,
    executable: &Path,
    cache: &Path,
    work: &Path,
    previous: Option<&Cached>,
    force: bool,
    cancel: &CancellationToken,
) -> Result<Cached> {
    npm_coverage::check(&inv, cancel)?;
    if inv.files.is_empty() {
        return Err(invalid(
            "No supported dependency files found. This repository has not been scanned.",
        ));
    }
    if !executable.is_file() {
        return Err(invalid(
            "OSV-Scanner is unavailable. Choose the reviewed scanner in Security settings.",
        ));
    }
    let ecosystems = inv.ecosystems();
    let db = databases(cache, &ecosystems, cancel)?;
    let fingerprint = inv.fingerprint()?;
    if !force
        && let Some(old) = previous
        && old.inventory == fingerprint
        && old.database == db
    {
        return Ok(old.clone());
    }
    let scratch = Scratch::new(work)?;
    let copies = scratch.copy(&inv)?;
    let sizes: Vec<u64> = inv.files.iter().map(|f| f.bytes.len() as u64).collect();
    let runs = runs(&copies, &sizes);
    let mut merge = Merge::new(&inv);
    let mut without_packages = 0;
    // One run after another; each keeps OSV's own time and output limits.
    for run in &runs {
        cancel.check()?;
        let output = run_offline_osv(executable, &scratch.path, cache, &copies[run.clone()], false, cancel)?;
        match output.code {
            Some(0 | 1) => {}
            // No package versions in this run's files: they produced no inventory.
            Some(128) => {
                without_packages += 1;
                continue;
            }
            _ => {
                return Err(invalid(
                    "OSV could not complete the offline scan. Check the dependency files and refresh advisory data.",
                ));
            }
        }
        let added = merge.add(&output.json, &inv, &copies, run.clone())?;
        if output.code == Some(0) && added > 0 || output.code == Some(1) && added == 0 {
            return Err(invalid(
                "OSV returned an inconsistent scan result; it is not considered complete.",
            ));
        }
        if output.diagnostics {
            merge.note("OSV emitted diagnostics; this result may be incomplete. Raw diagnostic text is not logged.");
        }
    }
    if without_packages == runs.len() {
        return Err(invalid(
            "OSV found no package versions to check. Coverage is incomplete.",
        ));
    }
    let mut report = merge.finish(&inv)?;
    if databases(cache, &ecosystems, cancel)? != db {
        return Err(invalid(
            "Dependency files or advisory data changed during the scan. Scan again.",
        ));
    }
    report.checked = now();
    report.oldest_database = db.iter().map(|d| d.modified).min().unwrap_or(0);
    if report.checked.saturating_sub(report.oldest_database) > 7 * 86_400 {
        report.notes.push(
            "Advisory data is more than seven days old. Update it to check for newer advisories."
                .into(),
        );
    }
    scratch.remove()?;
    Ok(Cached {
        inventory: fingerprint,
        database: db,
        report: Arc::new(report),
    })
}

/// OSV's runs for these copies (of files `sizes` bytes long), in order: at most 256
/// files and 64 MiB a run, and few enough characters that its command line fits. A scan
/// within the old fixed limits is still one run.
pub fn runs(copies: &[PathBuf], sizes: &[u64]) -> Vec<Range<usize>> {
    debug_assert_eq!(copies.len(), sizes.len());
    let mut runs = Vec::new();
    let (mut start, mut characters, mut bytes) = (0, 0, 0);
    for (index, (copy, size)) in copies.iter().zip(sizes).enumerate() {
        // `--lockfile`, the `:` before the path, quotes and the spaces between.
        let needed = copy.as_os_str().len() + 16;
        if index > start
            && (index - start == FILES_PER_RUN
                || characters + needed > CHARACTERS_PER_RUN
                || bytes + size > BYTES_PER_RUN)
        {
            runs.push(start..index);
            (start, characters, bytes) = (index, 0, 0);
        }
        characters += needed;
        bytes += size;
    }
    if start < copies.len() {
        runs.push(start..copies.len());
    }
    runs
}

fn clean(text: &str, max: usize) -> String {
    text.chars()
        .filter(|c| {
            !c.is_control() && !matches!(*c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
        })
        .take(max)
        .collect()
}
fn string<'a>(value: &'a Value, name: &str) -> Result<&'a str> {
    value[name]
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= 8192)
        .ok_or_else(|| invalid("OSV returned an invalid package record."))
}
fn normalized(path: &Path) -> String {
    let text = path.to_string_lossy();
    let text = if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        format!("//{rest}")
    } else {
        text.trim_start_matches(r"\\?\").to_owned()
    };
    text.replace('\\', "/").to_lowercase()
}
fn safe_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() < 120
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_' || c == b'.')
}
fn severity(score: Option<&Value>) -> String {
    let value = score.and_then(|v| {
        v.as_str()
            .and_then(|s| s.parse::<f64>().ok())
            .or_else(|| v.as_f64())
    });
    match value.filter(|v| v.is_finite() && (0.0..=10.0).contains(v)) {
        Some(v) if v >= 9.0 => "Critical",
        Some(v) if v >= 7.0 => "High",
        Some(v) if v >= 4.0 => "Medium",
        Some(_) => "Low",
        None => "Unknown",
    }
    .into()
}

/// One scan's OSV results, a run at a time.
pub fn parse(bytes: &[u8], inv: &Inventory, copies: &[PathBuf]) -> Result<Report> {
    let mut merge = Merge::new(inv);
    merge.add(bytes, inv, copies, 0..copies.len())?;
    merge.finish(inv)
}

/// OSV's results for one scan's files, gathered run by run into one report.
pub struct Merge {
    report: Report,
    sources: BTreeSet<usize>,
}
impl Merge {
    pub fn new(inv: &Inventory) -> Self {
        Self {
            report: Report {
                packages: 0,
                files: inv.files.len(),
                findings: Vec::new(),
                notes: inv.notes.clone(),
                checked: 0,
                oldest_database: 0,
            },
            sources: BTreeSet::new(),
        }
    }
    fn note(&mut self, text: &str) {
        self.report.notes.push(text.into());
    }
    /// The report once every run is in: packages from at least one file, and a note
    /// when some file produced none.
    pub fn finish(self, inv: &Inventory) -> Result<Report> {
        let Self { mut report, sources } = self;
        if report.packages == 0 {
            return Err(invalid(
                "No resolved packages were scanned; coverage is incomplete.",
            ));
        }
        if sources.len() != inv.files.len() {
            report
                .notes
                .push("Some dependency files produced no inventory; coverage is incomplete.".into());
        }
        report.notes.sort();
        report.notes.dedup();
        Ok(report)
    }
    /// One run's JSON, for the copies in `run`. Returns how many advisories it added.
    pub fn add(&mut self, bytes: &[u8], inv: &Inventory, copies: &[PathBuf], run: Range<usize>) -> Result<usize> {
        let Self { report, sources } = self;
        let value: Value = serde_json::from_slice(bytes)
            .map_err(|_| invalid("OSV returned invalid JSON; no clean result is available."))?;
        let groups = value["results"]
            .as_array()
            .ok_or_else(|| invalid("OSV did not return a package inventory."))?;
        if groups.len() > FILES_PER_RUN
            || copies.len() != inv.files.len()
            || run.start > run.end
            || run.end > copies.len()
        {
            return Err(invalid("OSV result exceeds the file limit."));
        }
        let before = report.findings.len();
        // Packages are counted per OSV result, as when one run took every file.
        let mut listed = 0usize;
        for source in groups {
            let path = string(&source["source"], "path")?;
            // Only this run's own copies; a path from another run is not its result.
            let index = copies[run.clone()]
                .iter()
                .position(|p| normalized(p) == normalized(Path::new(path)))
                .map(|i| run.start + i)
                .ok_or_else(|| invalid("OSV returned a source outside the dependency snapshot."))?;
            sources.insert(index);
            let packages = source["packages"]
                .as_array()
                .ok_or_else(|| invalid("OSV returned an invalid inventory."))?;
            report.packages += packages.len();
            listed += packages.len();
            if listed > PACKAGES_PER_RUN {
                return Err(invalid("OSV result exceeds 50,000 packages."));
            }
            for p in packages {
                let name = string(&p["package"], "name")?;
                let version = p["package"]["version"].as_str().unwrap_or("");
                let ecosystem = p["package"]["ecosystem"].as_str().unwrap_or("");
                if version.is_empty() || ecosystem.is_empty() {
                    report
                        .notes
                        .push("Some package versions or ecosystems could not be identified.".into());
                }
                if p["vulnerabilities"].is_null() {
                    continue;
                }
                let vulnerabilities = p["vulnerabilities"]
                    .as_array()
                    .ok_or_else(|| invalid("OSV returned an invalid advisory list."))?;
                let mut active = BTreeMap::new();
                for advisory in vulnerabilities.iter().filter(|v| v["withdrawn"].is_null()) {
                    let id = string(advisory, "id")?;
                    if !safe_id(id) || active.insert(id, advisory).is_some() {
                        return Err(invalid("OSV returned an invalid advisory identifier."));
                    }
                }
                let mut seen = BTreeSet::new();
                let fallback: Vec<_> = active.keys().map(|id| json!({"ids":[id]})).collect();
                let reported = if p["groups"].is_null() {
                    &fallback
                } else {
                    p["groups"]
                        .as_array()
                        .ok_or_else(|| invalid("OSV returned invalid advisory groups."))?
                };
                let grouped: BTreeSet<_> = reported
                    .iter()
                    .flat_map(|g| {
                        g["ids"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter_map(Value::as_str)
                    })
                    .collect();
                if active.keys().any(|id| !grouped.contains(id)) {
                    return Err(invalid(
                        "OSV omitted an advisory from its groups; the result is incomplete.",
                    ));
                }
                for group in reported {
                    let ids: Vec<_> = group["ids"]
                        .as_array()
                        .ok_or_else(|| invalid("OSV returned an invalid advisory group."))?
                        .iter()
                        .filter_map(Value::as_str)
                        .filter(|id| active.contains_key(id))
                        .collect();
                    if ids.is_empty() {
                        continue;
                    }
                    if ids.iter().any(|id| !safe_id(id)) {
                        return Err(invalid("OSV returned an invalid advisory identifier."));
                    }
                    let aliases: BTreeSet<_> = ids
                        .iter()
                        .flat_map(|id| {
                            std::iter::once(*id).chain(
                                active[id]["aliases"]
                                    .as_array()
                                    .into_iter()
                                    .flatten()
                                    .filter_map(Value::as_str),
                            )
                        })
                        .filter(|s| safe_id(s))
                        .map(str::to_owned)
                        .collect();
                    if !aliases.is_disjoint(&seen) {
                        continue;
                    }
                    seen.extend(aliases.iter().cloned());
                    let mut fixed = BTreeSet::new();
                    for id in &ids {
                        for affected in active[id]["affected"].as_array().into_iter().flatten() {
                            if affected["package"]["name"].as_str() != Some(name)
                                || affected["package"]["ecosystem"].as_str() != Some(ecosystem)
                            {
                                continue;
                            }
                            for range in affected["ranges"].as_array().into_iter().flatten() {
                                for event in range["events"].as_array().into_iter().flatten() {
                                    if let Some(v) = event["fixed"].as_str() {
                                        fixed.insert(clean(v, 100));
                                    }
                                }
                            }
                        }
                    }
                    let advisory = active[ids[0]];
                    let mut level = severity(group.get("max_severity"));
                    if level == "Unknown" {
                        level = match advisory["database_specific"]["severity"]
                            .as_str()
                            .unwrap_or("")
                            .to_ascii_uppercase()
                            .as_str()
                        {
                            "CRITICAL" => "Critical",
                            "HIGH" => "High",
                            "MODERATE" | "MEDIUM" => "Medium",
                            "LOW" => "Low",
                            _ => "Unknown",
                        }
                        .into();
                    }
                    if report.findings.len() == 5000 {
                        return Err(invalid("OSV result exceeds 5,000 findings."));
                    }
                    report.findings.push(Finding {
                        package: clean(name, 240),
                        version: clean(version, 120),
                        ecosystem: clean(ecosystem, 80),
                        file: clean(&inv.files[index].path.to_string_lossy(), 300),
                        ids: ids.into_iter().map(str::to_owned).collect(),
                        severity: level,
                        summary: clean(
                            advisory["summary"]
                                .as_str()
                                .unwrap_or("Published security advisory"),
                            600,
                        ),
                        fixed: fixed.into_iter().collect(),
                    });
                }
            }
        }
        Ok(report.findings.len() - before)
    }
}

/// Downloads only ecosystem-wide public data using a synthetic inventory. No repository
/// path, package name or version is passed to this network-enabled operation.
pub fn update(
    executable: &Path,
    cache: &Path,
    work: &Path,
    ecosystems: &[String],
    cancel: &CancellationToken,
) -> Result<()> {
    let allowed: BTreeSet<_> = FORMATS.iter().map(|(_, e)| *e).collect();
    if ecosystems.is_empty() || ecosystems.iter().any(|e| !allowed.contains(e.as_str())) {
        return Err(invalid("No supported advisory ecosystem was selected."));
    }
    let scratch = Scratch::new(work)?;
    let new_cache = scratch.path.join("download");
    fs::create_dir(&new_cache).map_err(|e| io("create advisory download cache", e))?;
    let file = scratch.path.join("osv-scanner.json");
    let packages: Vec<_> = ecosystems.iter().map(|e| json!({"package":{"name":"rocket-public-database-probe", "version":"0.0.0", "ecosystem":e}})).collect();
    fs::write(
        &file,
        serde_json::to_vec(&json!({"results":[{"packages":packages}]})).unwrap(),
    )
    .map_err(|e| io("prepare advisory update", e))?;
    let output = run_offline_osv(executable, &scratch.path, &new_cache, &[file], true, cancel)?;
    if !matches!(output.code, Some(0 | 1)) || output.diagnostics {
        return Err(invalid(
            "Advisory download did not complete. Existing data was preserved.",
        ));
    }
    let mut checked = Vec::new();
    for ecosystem in ecosystems {
        let source = new_cache
            .join("osv-scalibr")
            .join(ecosystem)
            .join("all.zip");
        osv_file_digest(&source, DB_LIMIT, cancel)?;
        checked.push((ecosystem, source));
    }
    cancel.check()?;
    for (ecosystem, source) in checked {
        let folder = cache.join("osv-scalibr").join(ecosystem);
        fs::create_dir_all(&folder).map_err(|e| io("create advisory cache", e))?;
        // OSV never writes into the live cache. Each verified archive replaces its
        // previous version atomically on Windows; scans and updates are serialized.
        fs::rename(source, folder.join("all.zip")).map_err(|e| io("publish advisory data", e))?;
    }
    scratch.remove()?;
    Ok(())
}

pub fn executable() -> PathBuf {
    let relative = format!(
        "runtime/osv-scanner/{OSV_VERSION}/osv-scanner{}",
        std::env::consts::EXE_SUFFIX
    );
    if let Ok(exe) = std::env::current_exe() {
        for parent in exe.ancestors().skip(1).take(4) {
            let path = parent.join(&relative);
            if path.is_file() {
                return path;
            }
        }
        return exe.parent().unwrap_or(Path::new(".")).join(relative);
    }
    PathBuf::from(relative)
}
