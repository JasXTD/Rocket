//! Read-only, direct tree comparisons. Refs are resolved once; later file reads
//! use the pinned commits and literal paths, never the displayed path labels.
use crate::{parse, repository::commit_revision_args, runner::args, *};

#[derive(Clone, Debug)]
pub struct ComparedFile {
    pub path: RepoPath,
    pub original_path: Option<RepoPath>,
    pub change: Change,
    pub similarity: Option<u8>,
}

#[derive(Clone)]
pub struct Comparison {
    pub base: ObjectId,
    /// For a local comparison, this equals base; use `local_staged` for its target.
    pub target: ObjectId,
    pub files: Vec<ComparedFile>,
    pub local_staged: Option<bool>,
    local_guard: Option<(std::path::PathBuf, std::sync::Arc<[u8]>)>,
}
impl std::fmt::Debug for Comparison {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Comparison")
            .field("base", &self.base)
            .field("target", &self.target)
            .field("files", &self.files)
            .field("local_staged", &self.local_staged)
            .finish_non_exhaustive()
    }
}

impl Comparison {
    pub fn file_options(&self, index: usize) -> Result<DiffOptions> {
        let file = self
            .files
            .get(index)
            .ok_or_else(|| Error::InvalidInput("Select a file from this comparison.".into()))?;
        // A renamed file needs both sides in the pathspec or Git would report an
        // addition. If the old path was also recreated, that change is included
        // too; consumers must show every returned patch, not silently drop it.
        let mut paths = file.original_path.iter().cloned().collect::<Vec<_>>();
        paths.push(file.path.clone());
        Ok(DiffOptions {
            target: if let Some(staged) = self.local_staged {
                DiffTarget::Local {
                    base: self.base.to_string(),
                    staged,
                }
            } else {
                DiffTarget::Between {
                    base: self.base.to_string(),
                    target: self.target.to_string(),
                }
            },
            paths,
            ..Default::default()
        })
    }
}

impl Repository {
    /// Compare the complete trees, not a merge-base/three-dot review. This does
    /// not check out a commit, refresh the index or run filters. Only a registered
    /// preview fetches anything: the file versions that differ, before comparing.
    pub fn compare_revisions(
        &self,
        base: &str,
        target: &str,
        cancel: &CancellationToken,
    ) -> Result<Comparison> {
        cancel.check()?;
        let resolve = |revision: &str, step| {
            self.runner.step(step, "resolve-commit", || {
                let bytes =
                    self.read_diff("resolve commit", &commit_revision_args(revision)?, cancel)?;
                parse::oid(parse::line(&bytes), "compare revisions")
            })
        };
        let base = resolve(base, "compare-base")?;
        let target = resolve(target, "compare-target")?;
        self.prepare_changes(Some(base.as_str()), target.as_str(), &[], cancel)?;
        let files = self.runner.step("compare-files", "read-identities", || {
            // The two full IDs were just validated. A dedicated metadata read
            // avoids resolving them again or generating any file bodies.
            let arguments = args(&[
                "diff",
                "--name-status",
                "-z",
                "--no-color",
                "--no-ext-diff",
                "--no-textconv",
                "--no-relative",
                "--ignore-submodules=none",
                "--submodule=short",
                "--find-renames",
                "--no-exit-code",
                base.as_str(),
                target.as_str(),
                "--",
            ]);
            let bytes = self.read_diff("compare files", &arguments, cancel)?;
            parse_files(&bytes, cancel)
        })?;
        cancel.check()?;
        Ok(Comparison {
            base,
            target,
            files,
            local_staged: None,
            local_guard: None,
        })
    }

    /// Read-only comparison to local tracked contents. The bounded binary patch
    /// binds working bytes (including binary content), not merely timestamps or
    /// Git's zero worktree object IDs. No temporary index or objects are written.
    pub fn compare_local(
        &self,
        base: &str,
        staged: bool,
        cancel: &CancellationToken,
    ) -> Result<Comparison> {
        self.require_worktree(cancel)?;
        let raw = self.read_diff("resolve commit", &commit_revision_args(base)?, cancel)?;
        let base = parse::oid(parse::line(&raw), "compare base")?;
        let guard = self.local_comparison_guard(&base, staged, cancel)?;
        let arguments = local_arguments(&base, staged, true);
        let files = parse_files(
            &self.read_diff("compare files", &arguments, cancel)?,
            cancel,
        )?;
        let comparison = Comparison {
            target: base.clone(),
            base,
            files,
            local_staged: Some(staged),
            local_guard: Some((self.info().git_dir.clone(), guard.into())),
        };
        self.validate_comparison(&comparison, cancel)?;
        Ok(comparison)
    }

    fn local_comparison_guard(
        &self,
        base: &ObjectId,
        staged: bool,
        cancel: &CancellationToken,
    ) -> Result<Vec<u8>> {
        self.runner
            .step("compare-check", "read-local-snapshot", || {
                let mut bounded = self.clone();
                bounded.runner.limits.stdout_bytes = 16 * 1024 * 1024;
                // Detect index changes even if the resulting worktree diff is identical.
                let mut guard = bounded.read_diff(
                    "compare local snapshot",
                    &args(&["ls-files", "--stage", "-z"]),
                    cancel,
                )?;
                // Index blob IDs already bind staged contents; only working
                // comparisons need to read mutable bodies on each validation.
                if !staged {
                    guard.push(0);
                    guard.extend(bounded.read_diff(
                        "compare local snapshot",
                        &local_arguments(base, false, false),
                        cancel,
                    )?);
                }
                cancel.check()?;
                Ok(guard)
            })
    }

    /// Call around lazy reads, including images/external comparisons, to avoid
    /// presenting new file contents under an older local comparison listing.
    pub fn validate_comparison(
        &self,
        comparison: &Comparison,
        cancel: &CancellationToken,
    ) -> Result<()> {
        cancel.check()?;
        if let (Some(staged), Some((directory, observed))) =
            (comparison.local_staged, &comparison.local_guard)
            && (*directory != self.info().git_dir
                || self
                    .local_comparison_guard(&comparison.base, staged, cancel)?
                    .as_slice()
                    != observed.as_ref())
        {
            return Err(Error::InvalidInput("Local comparison changed. The index or tracked working files changed after loading. Compare again to refresh; no files were written.".into()));
        }
        cancel.check()
    }
}

// Base has already been peeled to a commit. Avoid resolving it again on each
// local validation; only mutable index/worktree data need a fresh read.
fn local_arguments(base: &ObjectId, staged: bool, metadata: bool) -> Vec<std::ffi::OsString> {
    let mut a = args(&[
        "diff",
        "--no-color",
        "--no-ext-diff",
        "--no-textconv",
        "--no-relative",
        "--src-prefix=a/",
        "--dst-prefix=b/",
        "--ignore-submodules=none",
        "--submodule=short",
        "--find-renames",
        "--no-exit-code",
    ]);
    a.extend(if metadata {
        args(&["--name-status", "-z"])
    } else {
        args(&["--patch", "--binary", "--full-index", "--unified=3"])
    });
    if staged {
        a.push("--cached".into());
    }
    a.push(base.as_str().into());
    a.push("--".into());
    a
}

fn parse_files(mut bytes: &[u8], cancel: &CancellationToken) -> Result<Vec<ComparedFile>> {
    fn field<'a>(bytes: &mut &'a [u8]) -> Result<&'a [u8]> {
        let end = bytes
            .iter()
            .position(|&b| b == 0)
            .ok_or_else(|| Error::parse("compare files", "unterminated record"))?;
        let value = &bytes[..end];
        *bytes = &bytes[end + 1..];
        Ok(value)
    }
    fn path(bytes: &mut &[u8]) -> Result<RepoPath> {
        RepoPath::new(field(bytes)?.to_vec())
            .map_err(|_| Error::parse("compare files", "invalid repository path"))
    }
    let mut files = Vec::new();
    while !bytes.is_empty() {
        cancel.check()?;
        if files.len() == 100_000 {
            return Err(Error::Unsupported(
                "Comparison exceeds the 100,000-file limit.".into(),
            ));
        }
        let status = field(&mut bytes)?;
        let change = match status.first() {
            Some(b'A') => Change::Added,
            Some(b'D') => Change::Deleted,
            Some(b'M') => Change::Modified,
            Some(b'T') => Change::TypeChanged,
            Some(b'R') => Change::Renamed,
            Some(b'C') => Change::Copied,
            Some(b'U') => Change::Unmerged,
            _ => return Err(Error::parse("compare files", "unknown change status")),
        };
        let pair = matches!(change, Change::Renamed | Change::Copied);
        let similarity = if pair {
            let score = std::str::from_utf8(&status[1..])
                .ok()
                .filter(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
                .and_then(|s| s.parse::<u8>().ok())
                .filter(|&v| v <= 100)
                .ok_or_else(|| Error::parse("compare files", "invalid similarity score"))?;
            Some(score)
        } else if status.len() == 1 {
            None
        } else {
            return Err(Error::parse("compare files", "invalid change status"));
        };
        let original_path = if pair { Some(path(&mut bytes)?) } else { None };
        let path = path(&mut bytes)?;
        files.push(ComparedFile {
            path,
            original_path,
            change,
            similarity,
        });
    }
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn raw_names_and_rename_scores_are_preserved_and_bad_records_are_rejected() {
        let files = parse_files(
            "R097\0src/旧名\t.rs\0src/検索/新名 🚀.rs\0A\0--literal [a].md\0".as_bytes(),
            &CancellationToken::new(),
        )
        .unwrap();
        assert_eq!(files[0].path.as_bytes(), "src/検索/新名 🚀.rs".as_bytes());
        assert_eq!(
            files[0].original_path.as_ref().unwrap().as_bytes(),
            "src/旧名\t.rs".as_bytes()
        );
        assert_eq!(files[0].similarity, Some(97));
        assert_eq!(files[1].change, Change::Added);
        for bytes in [
            b"M\0no terminator".as_slice(),
            b"R101\0a\0b\0",
            b"R\0a\0b\0",
            b"A\0../escape\0",
            b"X\0a\0",
            b"M1\0a\0",
        ] {
            assert!(parse_files(bytes, &CancellationToken::new()).is_err());
        }
        let cancel = CancellationToken::new();
        cancel.cancel();
        assert!(matches!(
            parse_files(b"A\0name\0", &cancel),
            Err(Error::Cancelled)
        ));
    }
}
