//! Patches are interpreted only in a disposable repository. Publication reuses
//! reviewed file replacement and recovery; no Git apply runs in the real worktree.
use crate::{repository::commit_revision_args, runner::args, *};
use std::{
    fmt,
    path::{Path, PathBuf},
};
mod files;
pub(crate) use files::Scratch as PreviewScratch;
pub(crate) mod parse;
mod extended;
mod modes;
mod series;
mod import;
pub use import::{MailImportAction, MailImportDisplay, MailImportEntry, MailImportPreview, MailImportReport, visible_path};
pub use series::{PatchSeriesEntry, PatchSeriesPreview, PatchSeriesReport};
pub const PATCH_LIMIT: usize = 16 * 1024 * 1024;
const TOTAL_LIMIT: usize = 64 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PatchFailure {
    Empty,
    Format,
    Unsupported,
    UnsafePath,
    Conflict,
    SourceChanged,
    OutputExists,
    Selection,
}
impl PatchFailure {
    pub fn code(self) -> &'static str {
        match self {
            Self::Empty => "patch_empty",
            Self::Format => "patch_format",
            Self::Unsupported => "patch_unsupported",
            Self::UnsafePath => "patch_path",
            Self::Conflict => "patch_conflict",
            Self::SourceChanged => "patch_source_changed",
            Self::OutputExists => "patch_output_exists",
            Self::Selection => "patch_selection",
        }
    }
    pub fn guidance(self) -> &'static str {
        match self {
            Self::Empty => "There are no changes to export or apply.",
            Self::Format => {
                "The patch is malformed or contains duplicate paths. Choose a Git-style diff or one format-patch email; combined merge diffs and multi-message mailboxes are not supported."
            }
            Self::Unsupported => {
                "This preview supports up to 100 regular files, size-bounded Git literal binary patches, renames and executable modes, with a 16 MiB patch/file limit and 64 MiB total contents. Binary deltas, symlinks, submodules, copies, mixed converted line endings and custom filters need Git or an external tool."
            }
            Self::UnsafePath => {
                "The patch contains an unsupported path. Paths must stay inside this working folder, use existing normal parent folders and avoid Git metadata, .gitattributes, .gitmodules, links and junctions."
            }
            Self::Conflict => {
                "The patch does not apply cleanly to the current working files. No patch was applied. Review the file versions or regenerate the patch; conflict markers and reject files were not written."
            }
            Self::SourceChanged => {
                "The patch source changed after review. Nothing new was applied or exported. Review again before retrying."
            }
            Self::OutputExists => {
                "The destination already exists. Choose a new patch filename; existing files are never overwritten."
            }
            Self::Selection => {
                "Select added or removed lines from the current unstaged diff. Hunk headers select all changes in that hunk."
            }
        }
    }
}
fn failure<T>(f: PatchFailure) -> Result<T> {
    Err(Error::Patch(f))
}

#[derive(Clone)]
pub struct PatchPreview {
    repository: PathBuf,
    files: Vec<FileEditPreview>,
    source: Option<(PathBuf, Vec<u8>)>,
    bytes: Vec<u8>,
    selection: bool,
    modes: Vec<(RepoPath,u32)>,
    index: Option<Vec<u8>>,
}
impl fmt::Debug for PatchPreview {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PatchPreview")
            .field("files", &self.files.len())
            .field("selection", &self.selection)
            .finish_non_exhaustive()
    }
}
impl PatchPreview {
    pub fn files(&self) -> &[FileEditPreview] {
        &self.files
    }
    pub fn patch(&self) -> &[u8] {
        &self.bytes
    }
    pub fn mode_changes(&self) -> &[(RepoPath,u32)] { &self.modes }
    pub fn label(&self) -> &'static str {
        if self.selection {
            "discard-selection"
        } else {
            "patch-import"
        }
    }
}
pub struct PatchReport {
    pub changed: Vec<RepoPath>,
    pub recovery: Vec<PathBuf>,
    pub result: Result<()>,
    pub verification: Result<()>,
    pub status: Result<Status>,
}

#[derive(Clone, Debug)]
pub enum PatchExportSource {
    Unstaged,
    Staged,
    Commit(String),
}
#[derive(Clone)]
pub struct PatchExportPreview {
    repository: PathBuf,
    source: PatchExportSource,
    bytes: Vec<u8>,
    paths: Vec<RepoPath>,
    destination: PathBuf,
    commit: Option<ObjectId>,
}
impl fmt::Debug for PatchExportPreview {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PatchExportPreview")
            .field("files", &self.paths.len())
            .finish_non_exhaustive()
    }
}
impl PatchExportPreview {
    pub fn patch(&self) -> &[u8] {
        &self.bytes
    }
    pub fn paths(&self) -> &[RepoPath] {
        &self.paths
    }
    pub fn destination(&self) -> &Path {
        &self.destination
    }
    pub fn commit(&self) -> Option<&ObjectId> {
        self.commit.as_ref()
    }
}
impl Repository {
    pub fn preview_patch_import(
        &self,
        source: &Path,
        cancel: &CancellationToken,
    ) -> Result<PatchPreview> {
        self.runner.step("patch-preview", "read-patch", || {
            let bytes = files::read(source, cancel)?;
            let mut preview = self.plan_patch(&bytes, false, cancel)?;
            preview.source = Some((source.to_path_buf(), bytes));
            Ok(preview)
        })
    }
    /// Plan a patch given as text, such as the clipboard's, with the same checks
    /// as a patch file. The preview keeps the bytes, so there is no source file
    /// to compare again before applying.
    pub fn preview_patch_text(
        &self,
        bytes: &[u8],
        cancel: &CancellationToken,
    ) -> Result<PatchPreview> {
        self.runner.step("patch-preview", "read-patch-text", || {
            if bytes.is_empty() {
                return failure(PatchFailure::Empty);
            }
            if bytes.len() > PATCH_LIMIT {
                return failure(PatchFailure::Unsupported);
            }
            self.plan_patch(bytes, false, cancel)
        })
    }
    pub fn preview_discard_selection(
        &self,
        review: &ChangeReview,
        selected: &[usize],
        cancel: &CancellationToken,
    ) -> Result<PatchPreview> {
        self.runner.step("discard-preview", "review-selection", || {
            if !review.belongs_to(self) {
                return Err(Error::FileEdit(FileEditFailure::WrongRepository));
            }
            let current = self.review_change(review.path(), false, cancel)?;
            if current.patch() != review.patch() {
                return Err(Error::FileEdit(FileEditFailure::Stale));
            }
            let bytes = review.discard_patch(selected)?;
            let preview = self.plan_patch(&bytes, true, cancel)?;
            if self.review_change(review.path(), false, cancel)?.patch() != review.patch() {
                return Err(Error::FileEdit(FileEditFailure::Stale));
            }
            Ok(preview)
        })
    }
    fn plan_patch(
        &self,
        bytes: &[u8],
        selection: bool,
        cancel: &CancellationToken,
    ) -> Result<PatchPreview> {
        self.require_worktree(cancel)?;
        let operation = if selection {
            "discard-preview"
        } else {
            "patch-preview"
        };
        let parsed = self
            .runner
            .step(operation, "parse-patch", || parse::parse(bytes, cancel))?;
        let mut bases = Vec::new();
        let mut total = 0usize;
        self.runner.step(operation, "snapshot-targets", || {
            for patch in &parsed {
                cancel.check()?;
                if let Some(mode)=patch.before_mode {
                    let mut a=args(&["ls-files","--stage","-z","--"]);a.push(patch.original.as_ref().unwrap_or(&patch.path).argument()?);
                    let current=self.runner.run(self.cwd(),&a,vec![],cancel)?.success("patch-mode-preview")?;
                    if !current.starts_with(format!("{mode:o} ").as_bytes()){return Err(Error::PatchTarget{failure:PatchFailure::Conflict,path:patch.path.clone()});}
                }
                let paths:Vec<_> = if let Some(original)=&patch.original { vec![(original,false,true),(&patch.path,true,false)] } else { vec![(&patch.path,patch.added,patch.deleted)] };
                for (path,added,deleted) in paths {
                let p = self.patch_file_base(
                    path,
                    if selection {
                        FileEditSource::Selection
                    } else {
                        FileEditSource::Patch
                    },
                    cancel,
                )?;
                if added && p.before().is_some() || !added && p.before().is_none() {
                    return Err(Error::PatchTarget {
                        failure: PatchFailure::Conflict,
                        path: patch.path.clone(),
                    });
                }
                let (input, crlf) = if patch.binary { (p.before().map(<[u8]>::to_vec),false) } else { Self::patch_input(&p)? };
                total += input.as_ref().map_or(0, Vec::len);
                if total > TOTAL_LIMIT {
                    return failure(PatchFailure::Unsupported);
                }
                bases.push((p, input, crlf, deleted, patch.binary));
                }
            }
            Ok(())
        })?;
        let scratch = files::Scratch::new()?;
        let planned = (|| {
            self.runner
                .run(
                    &scratch.path,
                    &args(&[
                        "-c",
                        "init.defaultBranch=rocket-patch-preview",
                        "init",
                        "--quiet",
                        "--template=",
                    ]),
                    Vec::new(),
                    cancel,
                )?
                .success("patch-sandbox")?;
            // Highest-precedence attributes disable every conversion in the sandbox.
            std::fs::create_dir_all(scratch.path.join(".git/info"))
                .map_err(|e| Error::io("prepare patch sandbox attributes", e))?;
            files::write_new(
                &scratch.path.join(".git/info/attributes"),
                b"* -text -filter -ident -working-tree-encoding\n",
                cancel,
            )?;
            for (p, input, _, _, _) in &bases {
                let path = scratch.path.join(p.path().argument()?);
                std::fs::create_dir_all(path.parent().unwrap())
                    .map_err(|e| Error::io("prepare patch sandbox parent", e))?;
                if let Some(input) = input {
                    files::write_new(&path, input, cancel)?;
                }
            }
            let canonical: Vec<u8> = parsed
                .iter()
                .flat_map(|p| p.bytes.iter().copied())
                .collect();
            let mut command = args(&[
                "-c",
                "core.autocrlf=false",
                "-c",
                "core.eol=lf",
                "-c",
                "core.attributesFile=",
                "-c",
                "apply.ignoreWhitespace=no",
                "apply",
                "--whitespace=nowarn",
                "-p1",
            ]);
            if selection {
                command.push("--unidiff-zero".into());
            }
            let mut check = command.clone();
            check.extend(args(&["--check", "-"]));
            self.runner.step(operation, "check-patch", || {
                let out = self
                    .runner
                    .run(&scratch.path, &check, canonical.clone(), cancel)?;
                if !out.status.success() {
                    // Identify the conflicting file without exposing Git stderr in diagnostics.
                    for file in &parsed {
                        let single =
                            self.runner
                                .run(&scratch.path, &check, file.bytes.clone(), cancel)?;
                        if !single.status.success() {
                            return Err(Error::PatchTarget {
                                failure: PatchFailure::Conflict,
                                path: file.path.clone(),
                            });
                        }
                    }
                    return failure(PatchFailure::Conflict);
                }
                Ok(())
            })?;
            command.push("-".into());
            self.runner.step(operation, "build-patch-result", || {
                let out = self
                    .runner
                    .run(&scratch.path, &command, canonical, cancel)?;
                if !out.status.success() {
                    return failure(PatchFailure::Conflict);
                }
                Ok(())
            })?;
            let mut result = Vec::new();
            let mut result_size = 0;
            for (p, _, crlf, deleted, binary) in bases {
                cancel.check()?;
                let after = if deleted {
                    None
                } else {
                    Some(files::read(
                        &scratch.path.join(p.path().argument()?),
                        cancel,
                    )?)
                };
                result_size += after.as_ref().map_or(0, Vec::len);
                if result_size > TOTAL_LIMIT {
                    return failure(PatchFailure::Unsupported);
                }
                result.push(if binary {self.finish_raw_patch_file(p,after,cancel)?}else{self.finish_patch_file(p, after, crlf, cancel)?});
            }
            for p in &result {
                self.recheck_file_edit(p, cancel)?;
            }
            let modes:Vec<_>=parsed.iter().filter_map(|p|p.mode.map(|m|(p.path.clone(),m))).collect();
            let index=if modes.is_empty(){None}else{Some(modes::snapshot(self,cancel)?)};
            Ok(PatchPreview {
                repository: self.info().git_dir.clone(),
                files: result,
                source: None,
                bytes: bytes.to_vec(),
                selection,
                modes,
                index,
            })
        })();
        let cleanup = self
            .runner
            .step(operation, "cleanup-preview", || scratch.cleanup());
        match planned {
            Ok(preview) => {
                cleanup?;
                Ok(preview)
            }
            Err(e) => Err(e),
        }
    }
    pub fn apply_patch_preview(
        &self,
        p: &PatchPreview,
        cancel: &CancellationToken,
    ) -> Result<PatchReport> {
        let _permit = self.acquire_write(cancel)?;
        self.apply_patch_preview_locked(p, cancel)
    }
    pub(crate) fn apply_patch_preview_locked(&self, p: &PatchPreview, cancel: &CancellationToken) -> Result<PatchReport> {
        self.runner.step(p.label(), "revalidate-patch", || {
            if p.repository != self.info().git_dir {
                return Err(Error::FileEdit(FileEditFailure::WrongRepository));
            }
            if let Some((path, bytes)) = &p.source {
                match files::read(path, cancel) {
                    Ok(current) if current == *bytes => (),
                    Err(Error::Cancelled) => return Err(Error::Cancelled),
                    _ => return failure(PatchFailure::SourceChanged),
                }
            }
            for file in &p.files {
                self.recheck_file_edit(file, cancel)?;
            }
            if let Some(index)=&p.index && modes::snapshot(self,cancel)?!=*index { return failure(PatchFailure::SourceChanged); }
            Ok(())
        })?;
        let mut changed = Vec::new();
        let mut recovery = Vec::new();
        let mut result = self.runner.step(p.label(), "apply-patch-files", || {
            for file in &p.files {
                cancel.check()?;
                if file.before()==file.after(){continue;}
                let mut applied = false;
                let mut directory = None;
                let r = self.publish_patch_file(file, cancel, &mut applied, &mut directory);
                if applied {
                    changed.push(file.path().clone());
                }
                if let Some(dir) = directory {
                    recovery.push(dir);
                }
                r?;
            }
            Ok(())
        });
        let fresh = CancellationToken::new();
        let verification = self.runner.step(p.label(), "verify-patch-files", || {
            for file in &p.files {
                self.verify_patch_file(file, changed.contains(file.path()), &fresh)?;
            }
            Ok(())
        });
        if result.is_ok() && verification.is_ok() && !p.modes.is_empty() {
            result=self.runner.step(p.label(),"publish-patch-modes",||modes::publish(self,p,cancel,&mut recovery));
            if result.is_ok(){ for (path,_) in &p.modes { if !changed.contains(path){changed.push(path.clone());} } }
        }
        let status = self
            .runner
            .observe("refresh", "status", || self.status(&fresh));
        if let Some(log) = &self.runner.log {
            log.patch_outcome(
                p.label(),
                p.files.len(),
                changed.len(),
                recovery.len(),
                result.is_ok(),
                verification.is_ok(),
            );
        }
        Ok(PatchReport {
            changed,
            recovery,
            result,
            verification,
            status,
        })
    }
    fn export_bytes(
        &self,
        source: &PatchExportSource,
        cancel: &CancellationToken,
    ) -> Result<(Vec<u8>, Option<ObjectId>)> {
        let mut a = args(&[
            "--no-replace-objects",
            "diff",
            "--patch",
            "--binary",
            "--full-index",
            "--no-color",
            "--no-ext-diff",
            "--no-textconv",
            "--no-relative",
            "--src-prefix=a/",
            "--dst-prefix=b/",
            "--find-renames",
            "--ignore-submodules=none",
            "--submodule=short",
            "--no-exit-code",
            "--unified=3",
        ]);
        let mut commit = None;
        match source {
            PatchExportSource::Unstaged => (),
            PatchExportSource::Staged => a.push("--cached".into()),
            PatchExportSource::Commit(revision) => {
                let id = crate::parse::oid(
                    crate::parse::line(&self.read_diff(
                        "patch-export-preview",
                        &commit_revision_args(revision)?,
                        cancel,
                    )?),
                    "patch-export-preview",
                )?;
                a[1] = "show".into();
                a.extend(args(&[
                    "--format=",
                    "--first-parent",
                    "--root",
                    "--no-show-signature",
                    "--no-notes",
                    "--no-decorate",
                ]));
                a.push(format!("{id}^{{commit}}").into());
                commit = Some(id);
            }
        }
        a.push("--".into());
        Ok((self.read_diff("patch-export-preview", &a, cancel)?, commit))
    }
    pub fn preview_patch_export(
        &self,
        source: PatchExportSource,
        destination: &Path,
        cancel: &CancellationToken,
    ) -> Result<PatchExportPreview> {
        self.runner
            .step("patch-export-preview", "review-export", || {
                files::validate_destination(self, destination)?;
                let (bytes, commit) = self.export_bytes(&source, cancel)?;
                let parsed = parse::parse(&bytes, cancel)?;
                Ok(PatchExportPreview {
                    repository: self.info().git_dir.clone(),
                    source,
                    bytes,
                    paths: parsed.into_iter().map(|p| p.path).collect(),
                    destination: destination.to_path_buf(),
                    commit,
                })
            })
    }
    fn recheck_patch_export(
        &self,
        p: &PatchExportPreview,
        cancel: &CancellationToken,
    ) -> Result<()> {
        if p.repository != self.info().git_dir {
            return Err(Error::FileEdit(FileEditFailure::WrongRepository));
        }
        let (bytes, commit) = self.export_bytes(&p.source, cancel)?;
        if bytes != p.bytes || commit != p.commit {
            return failure(PatchFailure::SourceChanged);
        }
        files::validate_destination(self, &p.destination)
    }
    pub fn export_patch(&self, p: &PatchExportPreview, cancel: &CancellationToken) -> Result<()> {
        self.runner.step("patch-export", "publish-export", || {
            self.recheck_patch_export(p, cancel)?;
            files::export(self, p, cancel)
        })
    }
}
