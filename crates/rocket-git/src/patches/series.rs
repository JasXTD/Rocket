//! Export immutable, linear commit ranges without touching the live index.
//! Git writes stdout only; Rocket owns filenames and non-replacing publication.
use super::{files, *};
use crate::{local_files, parse as git_parse};
use std::{
    fs::{self, OpenOptions},
    io::Read,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};
const MAX_COMMITS: usize = 64;
static NEXT: AtomicU64 = AtomicU64::new(0);
fn bad(code: &str, message: &str) -> Error {
    Error::InvalidInput(format!("patch_series_{code}: {message}"))
}
fn destination(repo: &Repository, path: &Path) -> Result<()> {
    files::validate_destination(repo, path).map_err(|error| match error {
        Error::Patch(PatchFailure::OutputExists) => bad("output_exists", "This folder already exists
Rocket never writes into one, so choose a new name."),
        Error::Patch(PatchFailure::UnsafePath) => bad("destination", "Rocket can't save there
Choose a new folder inside one that exists, outside .git, with no links in the path."),
        other => other,
    })
}
#[derive(Clone)]
pub struct PatchSeriesEntry {
    pub id: ObjectId,
    pub subject: Vec<u8>,
    pub author: Vec<u8>,
    pub date: Vec<u8>,
    pub filename: String,
    pub paths: Vec<RepoPath>,
    bytes: Vec<u8>,
}
impl PatchSeriesEntry {
    pub fn patch(&self) -> &[u8] {
        &self.bytes
    }
}
impl fmt::Debug for PatchSeriesEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PatchSeriesEntry")
            .field("files", &self.paths.len())
            .field("bytes", &self.bytes.len())
            .finish()
    }
}
#[derive(Clone)]
pub struct PatchSeriesPreview {
    repository: PathBuf,
    base_input: String,
    tip_input: String,
    base: ObjectId,
    tip: ObjectId,
    destination: PathBuf,
    entries: Vec<PatchSeriesEntry>,
}
impl fmt::Debug for PatchSeriesPreview {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PatchSeriesPreview")
            .field("commits", &self.entries.len())
            .field("bytes", &self.total_bytes())
            .finish()
    }
}
impl PatchSeriesPreview {
    pub fn base(&self) -> &ObjectId {
        &self.base
    }
    pub fn tip(&self) -> &ObjectId {
        &self.tip
    }
    pub fn destination(&self) -> &Path {
        &self.destination
    }
    pub fn entries(&self) -> &[PatchSeriesEntry] {
        &self.entries
    }
    pub fn total_bytes(&self) -> usize {
        self.entries.iter().map(|e| e.bytes.len()).sum()
    }
}
pub struct PatchSeriesReport {
    pub result: Result<()>,
    pub published: bool,
    /// A failed or interrupted preparation is retained, never recursively deleted.
    /// It may contain partial files or external edits; inspect before removing it.
    pub retained: Option<PathBuf>,
}
impl Repository {
    /// Review the series from `first` to `last`, both included. The parent of `first`
    /// is the excluded base, so a repository's very first commit can't start a series.
    pub fn preview_patch_series_between(
        &self,
        first: &str,
        last: &str,
        destination: &Path,
        cancel: &CancellationToken,
    ) -> Result<PatchSeriesPreview> {
        let known = |revision: &str| {
            self.resolve_commit(revision, cancel).map_err(|error| match error {
                Error::Git { .. } => bad("unknown", &format!("Rocket can't find {revision}
Check the commit ID, branch or tag.")),
                other => other,
            })
        };
        let start = known(first)?;
        known(last)?;
        let base = self.resolve_commit(&format!("{start}^"), cancel).map_err(|error| match error {
            Error::Git { .. } => bad("first_commit", "This is the repository's first commit
A series can't include it. Start from the commit after it."),
            other => other,
        })?;
        let preview = self.preview_patch_series(base.as_str(), last, destination, cancel)?;
        // First's parent can also be an ancestor of Last when First sits on another line.
        if preview.entries.first().is_some_and(|e| e.id != start) {
            return Err(bad("nonlinear", "These commits can't make a series\nFirst isn't on the same line of history as Last."));
        }
        Ok(preview)
    }
    pub fn preview_patch_series(
        &self,
        base: &str,
        tip: &str,
        destination: &Path,
        cancel: &CancellationToken,
    ) -> Result<PatchSeriesPreview> {
        self.runner.step("patch-series-preview", "review_commit_series", || {
            self::destination(self, destination)?;
            let base_id = self.resolve_commit(base, cancel)?;
            let tip_id = self.resolve_commit(tip, cancel)?;
            let range = format!("{base_id}..{tip_id}");
            let output = self.read_diff("patch-series-preview", &args(&[
                "--no-replace-objects", "rev-list", "--reverse", "--topo-order", "--parents", "--max-count=65", &range, "--"
            ]), cancel)?;
            let lines: Vec<_> = output.split(|b| *b == b'\n').filter(|l| !l.is_empty()).collect();
            if lines.is_empty() { return Err(bad("empty", "No commits in this range
First has to be older than Last, on the same line of history.")); }
            if lines.len() > MAX_COMMITS { return Err(bad("limit", "Too many commits for one series
A series holds up to 64 commits and 64 MiB. Choose a smaller range.")); }
            let mut previous = base_id.clone();
            let mut ids = Vec::new();
            for line in lines {
                let parts: Vec<_> = line.split(|b| *b == b' ').collect();
                if parts.len() != 2 || parts[1] != previous.as_str().as_bytes() {
                    return Err(bad("nonlinear", "These commits can't make a series
There's a merge between First and Last, or they aren't on one line of history. Choose commits with no merge between them."));
                }
                let id = git_parse::oid(parts[0], "patch-series-preview")?;
                previous = id.clone(); ids.push(id);
            }
            if previous != tip_id { return Err(bad("nonlinear", "These commits can't make a series
Last isn't on the same line of history as First.")); }
            let mut entries = Vec::new();
            let mut total = 0;
            let mut parent = base_id.clone();
            for (index, id) in ids.into_iter().enumerate() {
                cancel.check()?;
                let meta = self.read_diff("patch-series-preview", &args(&[
                    "--no-replace-objects", "show", "-s", "--format=%s%x00%an%x00%aI%x00", "--no-show-signature", "--no-notes", "--no-decorate", id.as_str(), "--"
                ]), cancel)?;
                let fields: Vec<_> = meta.split(|b| *b == 0).collect();
                if fields.len() != 4 || fields[..3].iter().any(|v| v.len() > 16384) { return Err(bad("metadata", "Commit metadata exceeds the bounded preview format.")); }
                let names = self.read_diff("patch-series-preview", &args(&[
                    "--no-replace-objects", "diff", "--name-only", "-z", "--no-ext-diff", "--no-textconv", "--no-relative", "--no-renames", parent.as_str(), id.as_str(), "--"
                ]), cancel)?;
                let paths = names.split(|b| *b == 0).filter(|n| !n.is_empty()).map(|n| RepoPath::new(n.to_vec())).collect::<Result<Vec<_>>>()?;
                if paths.len() > 1000 { return Err(bad("limit", "Review at most 1,000 changed paths per commit.")); }
                let bytes = self.runner.step("patch-series-preview", "format_reviewed_commit", || self.read_diff("patch-series-preview", &args(&[
                    "--no-replace-objects", "-c", "format.from=false", "-c", "format.pretty=medium", "-c", "format.signatureFile=",
                    "format-patch", "--stdout", "--always", "--no-cover-letter", "--no-thread", "--no-attach", "--no-signoff", "--no-notes", "--no-base", "--no-signature", "--no-numbered", "--subject-prefix=PATCH", "--no-add-header", "--no-to", "--no-cc", "--no-force-in-body-from", "--no-zero-commit", "--binary", "--full-index", "--no-color", "--no-ext-diff", "--no-textconv", "--src-prefix=a/", "--dst-prefix=b/", "--find-renames", "--no-relative", "--unified=3", "-1", id.as_str(), "--"
                ]), cancel))?;
                total += bytes.len();
                if bytes.len() > PATCH_LIMIT || total > TOTAL_LIMIT { return Err(bad("limit", "This series is too big
A patch can be up to 16 MiB and a series up to 64 MiB. Choose a smaller range.")); }
                if !bytes.starts_with(format!("From {id} ").as_bytes()) { return Err(bad("format", "Git did not return the expected commit patch. No files were exported.")); }
                let filename = format!("{:04}-{}.patch", index + 1, &id.as_str()[..12]);
                parent = id.clone();
                entries.push(PatchSeriesEntry { id, subject: fields[0].to_vec(), author: fields[1].to_vec(), date: fields[2].to_vec(), filename, paths, bytes });
            }
            let preview = PatchSeriesPreview { repository: self.info().git_dir.clone(), base_input: base.into(), tip_input: tip.into(), base: base_id, tip: tip_id, destination: destination.into(), entries };
            self.recheck_patch_series(&preview, cancel)?;
            Ok(preview)
        })
    }
    fn recheck_patch_series(
        &self,
        p: &PatchSeriesPreview,
        cancel: &CancellationToken,
    ) -> Result<()> {
        cancel.check()?;
        if self.info().git_dir != p.repository
            || self.resolve_commit(&p.base_input, cancel)? != p.base
            || self.resolve_commit(&p.tip_input, cancel)? != p.tip
        {
            return Err(bad(
                "changed",
                "The commits changed after review
Review again. Nothing was saved.",
            ));
        }
        destination(self, &p.destination)
    }
    pub fn export_patch_series(
        &self,
        p: &PatchSeriesPreview,
        cancel: &CancellationToken,
    ) -> Result<PatchSeriesReport> {
        let _permit = self.acquire_write(cancel)?;
        let mut retained = None;
        let mut published = false;
        let result = self
            .runner
            .step("patch-series-export", "prepare_and_publish_series", || {
                self.recheck_patch_series(p, cancel)?;
                let _parent = files::pin_parent(&p.destination)?;
                let parent = p.destination.parent().unwrap();
                let scratch = parent.join(format!(
                    ".rocket-patch-series-{}-{}-{}",
                    std::process::id(),
                    SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_nanos(),
                    NEXT.fetch_add(1, Ordering::Relaxed)
                ));
                fs::create_dir(&scratch)
                    .map_err(|e| Error::io("reserve patch series preparation", e))?;
                retained = Some(scratch.clone());
                let scratch_guard = local_files::pin_directories(parent, &scratch)?;
                for entry in &p.entries {
                    cancel.check()?;
                    files::write_new(&scratch.join(&entry.filename), &entry.bytes, cancel)?;
                }
                self.runner
                    .step("patch-series-export", "verify_prepared_files", || {
                        verify_files(&scratch, p, cancel)
                    })?;
                self.runner.step(
                    "patch-series-export",
                    "recheck_series_before_publication",
                    || self.recheck_patch_series(p, cancel),
                )?;
                // Ref checks can wait on Git; catch edits made during that wait.
                self.runner
                    .step("patch-series-export", "recheck_prepared_files", || {
                        verify_files(&scratch, p, cancel)
                    })?;
                cancel.check()?;
                drop(scratch_guard);
                // Windows' non-replacing directory rename is all-or-nothing for the
                // visible destination. No user folder is merged or recursively removed.
                local_files::move_path(&scratch, &p.destination, false)?;
                published = true;
                retained = None;
                // Publication is already final: a late cancellation cannot imply undo.
                self.runner
                    .step("patch-series-export", "verify_published_series", || {
                        verify_files(&p.destination, p, &CancellationToken::new())
                    })
            });
        if let Some(log) = &self.runner.log {
            log.patch_series_result(
                p.entries.len(),
                published,
                retained.is_some(),
                result.is_ok(),
            );
        }
        Ok(PatchSeriesReport {
            result,
            published,
            retained,
        })
    }
}
fn verify_files(
    directory: &Path,
    p: &PatchSeriesPreview,
    cancel: &CancellationToken,
) -> Result<()> {
    let _guard = files::pin_parent(&directory.join("check"))?;
    let names = fs::read_dir(directory)
        .map_err(|e| Error::io("inspect exported series", e))?
        .collect::<std::io::Result<Vec<_>>>()
        .map_err(|e| Error::io("inspect exported patch", e))?;
    if names.len() != p.entries.len()
        || names.iter().any(|e| {
            !p.entries
                .iter()
                .any(|v| e.file_name() == std::ffi::OsStr::new(&v.filename))
        })
    {
        return Err(bad(
            "output_changed",
            "The prepared or exported folder changed. Inspect the reported folder; nothing was overwritten or removed.",
        ));
    }
    for entry in &p.entries {
        cancel.check()?;
        let filename = directory.join(&entry.filename);
        let mut options = OpenOptions::new();
        options.read(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.share_mode(1).custom_flags(0x00200000);
        }
        let file = options
            .open(&filename)
            .map_err(|e| Error::io("verify exported patch", e))?;
        let metadata = file
            .metadata()
            .map_err(|e| Error::io("inspect exported patch", e))?;
        if !metadata.is_file()
            || local_files::redirected(&metadata)
            || metadata.len() != entry.bytes.len() as u64
        {
            return Err(bad(
                "output_changed",
                "An exported patch changed. Inspect the reported folder before using it.",
            ));
        }
        crate::review::verify_preview_handle(&file, directory, &filename)?;
        let mut bytes = Vec::new();
        file.take(PATCH_LIMIT as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| Error::io("verify exported bytes", e))?;
        if bytes != entry.bytes {
            return Err(bad(
                "output_changed",
                "An exported patch changed. Inspect the reported folder before using it.",
            ));
        }
    }
    Ok(())
}
