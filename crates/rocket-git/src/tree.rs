//! Lazy, immutable repository browsing. Names are Git bytes, never filesystem paths.
use crate::{parse, repository::commit_revision_args, runner::args, *};
use std::path::PathBuf;

pub const TREE_ENTRY_LIMIT: usize = 50_000;
pub const TREE_FILE_LIMIT: usize = 2 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TreeEntryKind {
    Directory,
    File,
    Executable,
    Symlink,
    Submodule,
}

/// Only Rocket can construct a location. Descendants retain the original commit
/// and repository binding; display text is never round-tripped into arguments.
#[derive(Clone, Debug)]
pub struct TreeDirectory {
    repository: PathBuf,
    commit: ObjectId,
    object: ObjectId,
    path: Vec<u8>,
}
impl TreeDirectory {
    pub fn commit(&self) -> &ObjectId {
        &self.commit
    }
    pub fn path(&self) -> &[u8] {
        &self.path
    }
}

#[derive(Clone, Debug)]
pub struct TreeEntry {
    location: TreeDirectory,
    kind: TreeEntryKind,
}
impl TreeEntry {
    pub fn kind(&self) -> TreeEntryKind {
        self.kind
    }
    pub fn path(&self) -> &[u8] {
        self.location.path()
    }
    pub fn name(&self) -> &[u8] {
        self.path()
            .rsplit(|b| *b == b'/')
            .next()
            .unwrap_or_default()
    }
    pub fn object(&self) -> &ObjectId {
        &self.location.object
    }
    pub fn commit(&self) -> &ObjectId {
        self.location.commit()
    }
    pub fn directory(&self) -> Option<TreeDirectory> {
        (self.kind == TreeEntryKind::Directory).then(|| self.location.clone())
    }
}

#[derive(Clone, Debug)]
pub struct TreeListing {
    pub directory: TreeDirectory,
    pub entries: Vec<TreeEntry>,
}

impl Repository {
    /// Resolve once, then browse a commit snapshot. Branch movement cannot change it.
    pub fn repository_tree(
        &self,
        revision: &str,
        cancel: &CancellationToken,
    ) -> Result<TreeListing> {
        cancel.check()?;
        let commit = self.runner.step("tree-list", "resolve-revision", || {
            let output =
                self.read_tree_object("tree-list", &commit_revision_args(revision)?, cancel)?;
            parse::oid(parse::line(&output), "tree-list")
        })?;
        // ls-tree accepts a commit as the root tree-ish. Children use tree IDs.
        self.tree_directory(
            &TreeDirectory {
                repository: self.info().git_dir.clone(),
                object: commit.clone(),
                commit,
                path: Vec::new(),
            },
            cancel,
        )
    }

    pub fn tree_directory(
        &self,
        directory: &TreeDirectory,
        cancel: &CancellationToken,
    ) -> Result<TreeListing> {
        cancel.check()?;
        self.check_tree_location(directory)?;
        let bytes = self.runner.step("tree-list", "read-directory", || {
            // No recursion or blob sizes: missing promisor blobs must not prevent
            // browsing a locally available tree, or trigger network/filter work.
            self.read_tree_object(
                "tree-list",
                &args(&["ls-tree", "-z", "--full-tree", directory.object.as_str()]),
                cancel,
            )
        })?;
        let entries = self.runner.step("tree-list", "parse-directory", || {
            parse_entries(directory, &bytes, cancel)
        })?;
        cancel.check()?;
        Ok(TreeListing {
            directory: directory.clone(),
            entries,
        })
    }

    /// Read the recorded blob only. Symlink targets and submodule checkouts are
    /// never followed. Git filters, working files and lazy fetching are not used.
    pub fn tree_file(
        &self,
        entry: &TreeEntry,
        cancel: &CancellationToken,
    ) -> Result<HistoricalFile> {
        self.tree_file_with_limit(entry, TREE_FILE_LIMIT, cancel)
    }

    /// Bounded raw blob read for consumers such as dependency scanners. The
    /// normal preview keeps its 2 MiB limit; no caller can request over 8 MiB.
    pub fn tree_file_with_limit(
        &self,
        entry: &TreeEntry,
        limit: usize,
        cancel: &CancellationToken,
    ) -> Result<HistoricalFile> {
        if limit == 0 || limit > 8 * 1024 * 1024 {
            return Err(Error::InvalidInput("The blob read limit must be between 1 byte and 8 MiB.".into()));
        }
        cancel.check()?;
        self.check_tree_location(&entry.location)?;
        if matches!(
            entry.kind,
            TreeEntryKind::Directory | TreeEntryKind::Submodule
        ) {
            return Err(Error::InvalidInput(
                "Select a file or symbolic link to preview its contents.".into(),
            ));
        }
        let size = self.runner.step("tree-content", "check-blob-size", || {
            let bytes = self.read_tree_object(
                "tree-content",
                &args(&["cat-file", "-s", entry.object().as_str()]),
                cancel,
            )?;
            let size: usize = std::str::from_utf8(parse::line(&bytes))
                .ok()
                .and_then(|s| s.parse().ok())
                .ok_or_else(|| malformed("invalid blob size"))?;
            if size > limit {
                return Err(Error::Unsupported(if limit == TREE_FILE_LIMIT {
                    "This file exceeds the 2 MiB preview limit. Its contents were not loaded.".into()
                } else {
                    format!("This file exceeds the {limit}-byte read limit. Its contents were not loaded.")
                }));
            }
            Ok(size)
        })?;
        let bytes = self.runner.step("tree-content", "read-blob", || {
            self.read_tree_object(
                "tree-content",
                &args(&["cat-file", "blob", entry.object().as_str()]),
                cancel,
            )
        })?;
        if bytes.len() != size {
            return Err(malformed("blob size does not match metadata"));
        }
        cancel.check()?;
        let kind = if bytes.contains(&0) || std::str::from_utf8(&bytes).is_err() {
            HistoricalFileKind::Binary
        } else {
            HistoricalFileKind::Text
        };
        Ok(HistoricalFile { kind, bytes })
    }

    fn read_tree_object(
        &self,
        operation: &'static str,
        arguments: &[std::ffi::OsString],
        cancel: &CancellationToken,
    ) -> Result<Vec<u8>> {
        // Replacement refs can change between navigation requests. Read the
        // actual recorded objects so the displayed commit remains a snapshot.
        let mut plain = args(&["--no-replace-objects"]);
        plain.extend_from_slice(arguments);
        self.read_diff(operation, &plain, cancel)
    }

    fn check_tree_location(&self, location: &TreeDirectory) -> Result<()> {
        if location.repository != self.info().git_dir {
            return Err(Error::InvalidInput(
                "Repository browser selection belongs to another repository/worktree.".into(),
            ));
        }
        Ok(())
    }
}

fn malformed(message: &str) -> Error {
    Error::InvalidOutput {
        operation: "tree-list",
        detail: message.into(),
    }
}

fn parse_entries(
    directory: &TreeDirectory,
    bytes: &[u8],
    cancel: &CancellationToken,
) -> Result<Vec<TreeEntry>> {
    if bytes.is_empty() {
        return Ok(Vec::new());
    }
    let bytes = bytes
        .strip_suffix(&[0])
        .ok_or_else(|| malformed("unterminated tree entry"))?;
    let mut entries = Vec::new();
    for record in bytes.split(|b| *b == 0) {
        cancel.check()?;
        if entries.len() == TREE_ENTRY_LIMIT {
            return Err(Error::Unsupported(format!(
                "This folder exceeds the {TREE_ENTRY_LIMIT} entry limit. No partial list is displayed."
            )));
        }
        let tab = record
            .iter()
            .position(|b| *b == b'\t')
            .ok_or_else(|| malformed("missing tree entry separator"))?;
        let name = &record[tab + 1..];
        // A tree record names one component. Unusual bytes are retained, including
        // names that cannot be checked out on Windows; browsing uses only IDs.
        if name.is_empty() || name.contains(&b'/') || matches!(name, b"." | b"..") {
            return Err(malformed("invalid tree entry name"));
        }
        let fields: Vec<_> = record[..tab].split(|b| *b == b' ').collect();
        if fields.len() != 3 {
            return Err(malformed("invalid tree metadata"));
        }
        let kind = match (fields[0], fields[1]) {
            (b"040000", b"tree") => TreeEntryKind::Directory,
            (b"100644", b"blob") => TreeEntryKind::File,
            (b"100755", b"blob") => TreeEntryKind::Executable,
            (b"120000", b"blob") => TreeEntryKind::Symlink,
            (b"160000", b"commit") => TreeEntryKind::Submodule,
            _ => return Err(malformed("unsupported tree entry mode/type")),
        };
        let object = parse::oid(fields[2], "tree-list")?;
        if object.as_str().len() != directory.commit.as_str().len() {
            return Err(malformed("tree entry has the wrong object format"));
        }
        let mut path = directory.path.clone();
        if !path.is_empty() {
            path.push(b'/');
        }
        path.extend_from_slice(name);
        entries.push(TreeEntry {
            location: TreeDirectory {
                repository: directory.repository.clone(),
                commit: directory.commit.clone(),
                object,
                path,
            },
            kind,
        });
    }
    entries.sort_unstable_by(|a, b| {
        (a.kind != TreeEntryKind::Directory)
            .cmp(&(b.kind != TreeEntryKind::Directory))
            .then_with(|| a.name().cmp(b.name()))
    });
    cancel.check()?;
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tree_records_are_byte_exact_and_reject_malformed_metadata() {
        let directory = TreeDirectory {
            repository: PathBuf::new(),
            commit: ObjectId::parse(&"a".repeat(40)).unwrap(),
            object: ObjectId::parse(&"b".repeat(40)).unwrap(),
            path: b"parent".to_vec(),
        };
        let record = |mode: &str, kind: &str, name: &[u8]| {
            let mut b = format!("{mode} {kind} {}\t", "c".repeat(40)).into_bytes();
            b.extend_from_slice(name);
            b.push(0);
            b
        };
        let mut bytes = record("100644", "blob", b"odd\tline\n\xff");
        bytes.extend(record("040000", "tree", "検索 🚀 é".as_bytes()));
        let entries = parse_entries(&directory, &bytes, &CancellationToken::new()).unwrap();
        assert_eq!(entries[0].kind(), TreeEntryKind::Directory);
        assert_eq!(entries[1].name(), b"odd\tline\n\xff");
        assert_eq!(entries[1].path(), b"parent/odd\tline\n\xff");
        for invalid in [
            record("040000", "blob", b"x"),
            record("100644", "blob", b"../x"),
            b"broken".to_vec(),
        ] {
            assert!(parse_entries(&directory, &invalid, &CancellationToken::new()).is_err());
        }
        let cancel = CancellationToken::new();
        cancel.cancel();
        assert!(matches!(
            parse_entries(&directory, &bytes, &cancel),
            Err(Error::Cancelled)
        ));
    }
}
