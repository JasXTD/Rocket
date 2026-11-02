//! Read-only image versions bound to raw Git paths and immutable blob IDs.
use crate::{parse, runner::args, *};
use std::{fs, io::Read, path::PathBuf};
mod batch;

pub const IMAGE_FILE_LIMIT: usize = 16 * 1024 * 1024;

#[derive(Clone, Debug)]
enum Source {
    Missing,
    Blob(ObjectId),
    Gitlink(ObjectId),
    Working,
    Unsupported,
}
#[derive(Clone, Debug)]
pub struct ImageComparison {
    repository: PathBuf,
    path: RepoPath,
    before: Source,
    after: Source,
    before_label: &'static str,
    after_label: &'static str,
    comparison_guard: Option<std::sync::Arc<Comparison>>,
}
impl ImageComparison {
    pub(crate) fn validate(&self, repo: &Repository, cancel: &CancellationToken) -> Result<()> {
        cancel.check()?;
        if self.repository != repo.info().git_dir {
            return Err(Error::InvalidInput("Image selection belongs to a different repository. Reload the comparison.".into()));
        }
        if let Some(comparison) = &self.comparison_guard { repo.validate_comparison(comparison, cancel)?; }
        Ok(())
    }
    pub fn bind_comparison(&mut self, comparison: std::sync::Arc<Comparison>) {
        self.comparison_guard = Some(comparison);
    }
    pub(crate) fn gitlinks(&self) -> Option<(Option<ObjectId>, Option<ObjectId>)> {
        if !matches!(self.before, Source::Gitlink(_)) && !matches!(self.after, Source::Gitlink(_)) { return None; }
        let id = |source: &Source| if let Source::Gitlink(id) = source { Some(id.clone()) } else { None };
        Some((id(&self.before), id(&self.after)))
    }
    pub fn path(&self) -> &RepoPath {
        &self.path
    }
    pub fn before_label(&self) -> &'static str {
        self.before_label
    }
    pub fn after_label(&self) -> &'static str {
        self.after_label
    }
}
pub struct ImageVersions {
    pub before: Option<Vec<u8>>,
    pub after: Option<Vec<u8>>,
}

/// Extensions identify preview candidates only. Decoding verifies the actual format
/// (TGA, which has no signature, is decoded as TGA only under a `.tga` name).
pub fn image_path(path: &[u8]) -> bool {
    path.rsplit(|b| *b == b'.').next().is_some_and(|ext| {
        [b"png".as_slice(), b"jpg", b"jpeg", b"bmp", b"tga", b"webp"]
            .iter()
            .any(|v| ext.eq_ignore_ascii_case(v))
    })
}

impl Repository {
    /// Discover exact before/after object identities without reading image bodies.
    pub fn image_comparisons(
        &self,
        options: &DiffOptions,
        cancel: &CancellationToken,
    ) -> Result<Vec<ImageComparison>> {
        self.file_comparisons(options, cancel).map(|items| {
            items
                .into_iter()
                .filter(|item| image_path(item.path.as_bytes()))
                .collect()
        })
    }

    /// Raw identities for external comparisons, including text and binary files.
    /// Uses the same bounded, filter-free version reader as image previews.
    pub fn file_comparisons(
        &self,
        options: &DiffOptions,
        cancel: &CancellationToken,
    ) -> Result<Vec<ImageComparison>> {
        self.runner.step("image-list", "read-identities", || {
            let mut arguments = self.diff_arguments(options, cancel)?;
            // --unified also enables patch output; raw records must remain a
            // pure NUL-delimited stream, without a trailing text patch.
            arguments.retain(|arg| !arg.to_string_lossy().starts_with("--unified="));
            arguments[1] = "--raw".into();
            arguments.insert(1, "--no-abbrev".into());
            arguments.insert(1, "-z".into());
            arguments.insert(0, "--no-replace-objects".into());
            let bytes = self.read_diff("image-list", &arguments, cancel)?;
            let (before_label, after_label) = match options.target {
                DiffTarget::Staged => ("HEAD", "Index"),
                DiffTarget::Unstaged => ("Index", "Working file"),
                DiffTarget::Commit { .. } => ("First parent", "Commit"),
                DiffTarget::Between { .. } => ("Base commit", "Target commit"),
                DiffTarget::Local { staged: true, .. } => ("Base commit", "Index"),
                DiffTarget::Local { staged: false, .. } => ("Base commit", "Working file"),
            };
            parse_images(
                self.info().git_dir.clone(),
                &bytes,
                matches!(options.target, DiffTarget::Unstaged | DiffTarget::Local { staged: false, .. }),
                before_label,
                after_label,
                cancel,
            )
        })
    }

    pub fn untracked_image(
        &self,
        path: &RepoPath,
        cancel: &CancellationToken,
    ) -> Result<ImageComparison> {
        cancel.check()?;
        if self.info().worktree.is_none() {
            return Err(Error::WorktreeRequired);
        }
        Ok(ImageComparison {
            repository: self.info().git_dir.clone(),
            path: path.clone(),
            before: Source::Missing,
            after: Source::Working,
            before_label: "Not present",
            after_label: "Untracked file",
            comparison_guard: None,
        })
    }

    pub fn image_versions(
        &self,
        selection: &ImageComparison,
        cancel: &CancellationToken,
    ) -> Result<ImageVersions> {
        selection.validate(self, cancel)?;
        let mut ids = Vec::new();
        for source in [&selection.before, &selection.after] {
            if let Source::Blob(id) = source && !ids.contains(id) { ids.push(id.clone()); }
        }
        let mut blobs = self.runner.step("image-preview", "read-blobs", || batch::read(self, &ids, cancel))?;
        let before = self.runner.step("image-preview", "read-before", || {
            self.image_source(&selection.before, &selection.path, &mut blobs, cancel)
        })?;
        let after = self.runner.step("image-preview", "read-after", || {
            if matches!((&selection.before, &selection.after), (Source::Blob(a), Source::Blob(b)) if a == b) {
                cancel.check()?;
                Ok(before.clone())
            } else {
                self.image_source(&selection.after, &selection.path, &mut blobs, cancel)
            }
        })?;
        cancel.check()?;
        if let Some(c) = &selection.comparison_guard { self.validate_comparison(c, cancel)?; }
        Ok(ImageVersions { before, after })
    }

    fn image_source(
        &self,
        source: &Source,
        path: &RepoPath,
        blobs: &mut Vec<(ObjectId, Vec<u8>)>,
        cancel: &CancellationToken,
    ) -> Result<Option<Vec<u8>>> {
        cancel.check()?;
        match source {
            Source::Missing => Ok(None),
            Source::Unsupported | Source::Gitlink(_) => Err(Error::Unsupported(
                concat!(
                    "Image previews require regular files; links, submodules ",
                    "and unresolved conflicts are not decoded."
                )
                .into(),
            )),
            Source::Working => read_working(self, path, cancel).map(Some),
            Source::Blob(oid) => {
                let index = blobs.iter().position(|(id, _)| id == oid).ok_or_else(invalid)?;
                Ok(Some(blobs.swap_remove(index).1))
            }
        }
    }
}
fn invalid() -> Error {
    Error::InvalidOutput {
        operation: "image-list",
        detail: "Invalid image comparison metadata. Refresh and retry.".into(),
    }
}
fn too_large() -> Error {
    Error::Unsupported(
        "Image exceeds the 16 MiB file preview limit. Whole-file staging remains available.".into(),
    )
}
fn parse_source(mode: &[u8], oid: &[u8], working: bool) -> Result<Source> {
    if mode == b"000000" {
        return Ok(Source::Missing);
    }
    if mode == b"160000" { return Ok(Source::Gitlink(parse::oid(oid, "submodule diff")?)); }
    if !matches!(mode, b"100644" | b"100755") {
        return Ok(Source::Unsupported);
    }
    let oid = parse::oid(oid, "image-list")?;
    if working {
        return Ok(Source::Working);
    }
    if oid.as_str().bytes().all(|b| b == b'0') {
        return Err(invalid());
    }
    Ok(Source::Blob(oid))
}
fn parse_images(
    repository: PathBuf,
    bytes: &[u8],
    working: bool,
    before_label: &'static str,
    after_label: &'static str,
    cancel: &CancellationToken,
) -> Result<Vec<ImageComparison>> {
    if bytes.is_empty() {
        return Ok(Vec::new());
    }
    if !bytes.ends_with(&[0]) {
        return Err(invalid());
    }
    let mut fields = bytes[..bytes.len() - 1].split(|b| *b == 0);
    let mut out = Vec::new();
    let mut records = 0;
    while let Some(header) = fields.next() {
        cancel.check()?;
        records += 1;
        if records > 50_000 {
            return Err(Error::Unsupported(
                "Image file list exceeds 50,000 changes. Select a single file.".into(),
            ));
        }
        let header = header.strip_prefix(b"\n").unwrap_or(header);
        let parts: Vec<_> = header
            .strip_prefix(b":")
            .ok_or_else(invalid)?
            .split(|b| *b == b' ')
            .collect();
        if parts.len() != 5 {
            return Err(invalid());
        }
        let first = fields.next().ok_or_else(invalid)?;
        let path = if matches!(parts[4].first(), Some(b'R' | b'C')) {
            fields.next().ok_or_else(invalid)?
        } else {
            first
        };
        let unresolved = parts[4].first() == Some(&b'U');
        out.push(ImageComparison {
            repository: repository.clone(),
            path: RepoPath::new(path)?,
            before: if unresolved {
                Source::Unsupported
            } else {
                parse_source(parts[0], parts[2], false)?
            },
            after: if unresolved {
                Source::Unsupported
            } else {
                parse_source(parts[1], parts[3], working)?
            },
            before_label,
            after_label,
            comparison_guard: None,
        });
    }
    Ok(out)
}
pub(crate) fn read_working(
    repo: &Repository,
    path: &RepoPath,
    cancel: &CancellationToken,
) -> Result<Vec<u8>> {
    let root = repo
        .info()
        .worktree
        .as_ref()
        .ok_or(Error::WorktreeRequired)?;
    if path
        .as_bytes()
        .split(|b| *b == b'/')
        .any(|p| p.eq_ignore_ascii_case(b".git"))
    {
        return Err(Error::Unsupported(
            "Git metadata is not an image preview source.".into(),
        ));
    }
    let full = root.join(path.argument()?);
    let _parents = crate::local_files::pin_directories(root, full.parent().ok_or_else(invalid)?)?;
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(0x00200000);
        options.share_mode(1);
    }
    let mut file = options
        .open(&full)
        .map_err(|e| Error::io("open image preview", e))?;
    crate::review::verify_preview_handle(&file, root, &full)?;
    let before = file
        .metadata()
        .map_err(|e| Error::io("inspect image preview", e))?;
    if !before.is_file() || crate::local_files::redirected(&before) {
        return Err(Error::Unsupported(
            "Image preview does not follow links or directories.".into(),
        ));
    }
    if before.len() > IMAGE_FILE_LIMIT as u64 {
        return Err(too_large());
    }
    let mut bytes = Vec::new();
    let mut chunk = [0u8; 65536];
    loop {
        cancel.check()?;
        let n = file
            .read(&mut chunk)
            .map_err(|e| Error::io("read image preview", e))?;
        if n == 0 {
            break;
        }
        if bytes.len() + n > IMAGE_FILE_LIMIT {
            return Err(too_large());
        }
        bytes.extend_from_slice(&chunk[..n]);
    }
    let after = file
        .metadata()
        .map_err(|e| Error::io("recheck image preview", e))?;
    if before.len() != after.len() || before.modified().ok() != after.modified().ok() {
        return Err(Error::InvalidInput(
            "The working image changed while loading. Reload the comparison.".into(),
        ));
    }
    cancel.check()?;
    Ok(bytes)
}
