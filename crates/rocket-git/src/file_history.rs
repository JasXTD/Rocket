//! Immutable, bounded file reads. Display strings never become Git arguments.
use crate::{parse, runner::args, *};
use std::path::PathBuf;

const HISTORY_LIMIT: usize = 10_000;
const FILE_LIMIT: usize = 2 * 1024 * 1024;

#[derive(Clone, Debug)]
pub enum FileHistoryStart {
    File {
        tip: ObjectId,
        path: RepoPath,
    },
    /// Fixed literal directory path, including deleted directories. Git does not
    /// infer directory renames; use file history to follow an individual rename.
    Directory {
        tip: ObjectId,
        path: RepoPath,
    },
    Continue(FileHistoryCursor),
}
#[derive(Clone, Debug)]
pub struct FileHistoryCursor {
    repository: PathBuf,
    tip: ObjectId,
    path: RepoPath,
    offset: usize,
    directory: bool,
}
#[derive(Clone, Debug)]
pub struct FileHistoryEntry {
    pub commit: Commit,
    /// Exact file path (including removals), or the fixed path in directory mode.
    pub path: RepoPath,
    pub previous_path: Option<RepoPath>,
    /// File-history metadata only. Directory entries do not infer per-file status.
    pub deleted: bool,
}
#[derive(Clone, Debug)]
pub struct FileHistoryPage {
    pub directory: bool,
    pub tip: ObjectId,
    pub path: RepoPath,
    pub entries: Vec<FileHistoryEntry>,
    pub next: Option<FileHistoryCursor>,
    pub limited: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HistoricalFileKind {
    Text,
    Binary,
    Missing,
    NonFile,
}
#[derive(Clone, Debug)]
pub struct HistoricalFile {
    pub kind: HistoricalFileKind,
    pub bytes: Vec<u8>,
}
#[derive(Clone, Debug)]
pub struct BlameLine {
    pub commit: ObjectId,
    pub original_line: usize,
    pub line: usize,
    pub path: RepoPath,
    pub author: Vec<u8>,
    pub summary: Vec<u8>,
    pub content: Vec<u8>,
    pub boundary: bool,
}
#[derive(Clone, Debug)]
pub struct BlamePage {
    pub lines: Vec<BlameLine>,
    pub total_lines: usize,
    pub next_line: Option<usize>,
}
#[derive(Clone, Debug)]
pub struct LineHistoryEntry {
    pub commit: Commit,
    /// The commit's diff limited to the followed lines, in Git's patch format.
    pub patch: Vec<u8>,
}
#[derive(Clone, Debug)]
pub struct LineHistoryPage {
    /// Newest first.
    pub entries: Vec<LineHistoryEntry>,
    /// More commits changed the lines than the page holds.
    pub limited: bool,
}

/// Most commits a line history lists.
const LINE_HISTORY_LIMIT: usize = 200;

impl Repository {
    /// Git's single-file --follow semantics (rename similarity, not copy tracking).
    /// Replay a bounded prefix: Git --skip with --follow can omit older renames.
    pub fn file_history(
        &self,
        start: FileHistoryStart,
        size: usize,
        cancel: &CancellationToken,
    ) -> Result<FileHistoryPage> {
        cancel.check()?;
        if !(1..=200).contains(&size) {
            return Err(Error::InvalidInput(
                "file history page size must be 1..200".into(),
            ));
        }
        let (tip, path, offset, directory) = match start {
            FileHistoryStart::File { tip, path } => (tip, path, 0, false),
            FileHistoryStart::Directory { tip, path } => (tip, path, 0, true),
            FileHistoryStart::Continue(c) => {
                if c.repository != self.info().git_dir {
                    return Err(Error::InvalidInput(
                        "file history cursor belongs to another repository/worktree".into(),
                    ));
                }
                (c.tip, c.path, c.offset, c.directory)
            }
        };
        if directory {
            return self.directory_history_page(tip, path, offset, size, cancel);
        }
        let count = offset.saturating_add(size).min(HISTORY_LIMIT);
        // --follow compares file contents to find renames.
        self.prepare_path(tip.as_str(), &path, cancel)?;
        let mut arguments = args(&[
            "-c",
            "log.showSignature=false",
            "log",
            "--follow",
            // --follow changes the active path as renames are visited. Date order
            // can visit a shared ancestor before the side branch's rename.
            "--topo-order",
            "--find-renames=50%",
            "--name-status",
            "-z",
            "--format=%x00ROCKET-COMMIT %H%x00",
            "--no-color",
            "--no-ext-diff",
            "--no-textconv",
            "--no-notes",
            "--no-decorate",
            &format!("--max-count={}", count + 1),
            tip.as_str(),
            "--",
        ]);
        arguments.push(path.argument()?);
        let output = self.read_diff("file-history", &arguments, cancel)?;
        let mut records = history_records(&output, &path, cancel)?;
        let more = records.len() > count;
        records.truncate(count);
        let records: Vec<_> = records.into_iter().skip(offset).collect();
        let ids: Vec<_> = records.iter().map(|r| r.id.clone()).collect();
        // A bounded history() read cannot retrieve arbitrary IDs without ancestry;
        // cat-file batch reads precisely the records selected by --follow.
        let commits = self.file_commits(&ids, cancel)?;
        let entries = records
            .into_iter()
            .zip(commits)
            .map(|(r, commit)| FileHistoryEntry {
                commit,
                path: r.path,
                previous_path: r.previous,
                deleted: r.deleted,
            })
            .collect();
        let next = (more && count < HISTORY_LIMIT).then(|| FileHistoryCursor {
            repository: self.info().git_dir.clone(),
            tip: tip.clone(),
            path: path.clone(),
            offset: count,
            directory: false,
        });
        cancel.check()?;
        Ok(FileHistoryPage {
            directory: false,
            tip,
            path,
            entries,
            next,
            limited: more && count == HISTORY_LIMIT,
        })
    }

    /// The commits that changed `lines` (1-based, inclusive) of `path` as it is at `tip`,
    /// newest first, each with its diff limited to those lines: Git's `log -L`, which
    /// follows the lines through edits and the file through renames.
    pub fn line_history(
        &self,
        tip: &ObjectId,
        path: &RepoPath,
        lines: std::ops::RangeInclusive<usize>,
        cancel: &CancellationToken,
    ) -> Result<LineHistoryPage> {
        cancel.check()?;
        let (start, end) = lines.into_inner();
        if start == 0 || end < start {
            return Err(Error::InvalidInput(
                "a line range is 1-based, first line first".into(),
            ));
        }
        // -L names the file itself; Git reads it relative to the worktree root.
        let mut range = std::ffi::OsString::from(format!("-L{start},{end}:"));
        range.push(path.argument()?);
        let mut arguments = args(&[
            "-c",
            "log.showSignature=false",
            "log",
            "--format=%x00ROCKET-COMMIT %H%x00",
            "--no-color",
            "--no-ext-diff",
            "--no-textconv",
            "--no-notes",
            "--no-decorate",
            &format!("--max-count={}", LINE_HISTORY_LIMIT + 1),
        ]);
        arguments.push(range);
        arguments.push(tip.as_str().into());
        let output = self.read_diff("line-history", &arguments, cancel)?;
        // Each commit is "\0ROCKET-COMMIT <id>\0" followed by its patch.
        const MARKER: &[u8] = b"\0ROCKET-COMMIT ";
        let find = |bytes: &[u8]| bytes.windows(MARKER.len()).position(|w| w == MARKER);
        let mut rest = output.as_slice();
        if !rest.is_empty() && !rest.starts_with(MARKER) {
            return Err(Error::parse("line-history", "missing commit marker"));
        }
        let mut records = Vec::new();
        while let Some(record) = rest.strip_prefix(MARKER) {
            let end = record
                .iter()
                .position(|b| *b == 0)
                .ok_or_else(|| Error::parse("line-history", "incomplete commit marker"))?;
            let body = &record[end + 1..];
            let next = find(body).unwrap_or(body.len());
            let patch = body[..next].strip_prefix(b"\n\n").unwrap_or(&body[..next]);
            records.push((parse::oid(&record[..end], "line-history")?, patch.to_vec()));
            rest = &body[next..];
        }
        let limited = records.len() > LINE_HISTORY_LIMIT;
        records.truncate(LINE_HISTORY_LIMIT);
        let ids: Vec<_> = records.iter().map(|(id, _)| id.clone()).collect();
        let entries = self
            .file_commits(&ids, cancel)?
            .into_iter()
            .zip(records)
            .map(|(commit, (_, patch))| LineHistoryEntry { commit, patch })
            .collect();
        cancel.check()?;
        Ok(LineHistoryPage { entries, limited })
    }

    fn directory_history_page(
        &self,
        tip: ObjectId,
        path: RepoPath,
        offset: usize,
        size: usize,
        cancel: &CancellationToken,
    ) -> Result<FileHistoryPage> {
        let count = offset.saturating_add(size).min(HISTORY_LIMIT);
        // Full history retains relevant sides of merges. The immutable tip and
        // fixed path keep pagination stable even if branches/working files move.
        let mut arguments = args(&[
            "-c",
            "log.showSignature=false",
            "log",
            "--full-history",
            "--no-follow",
            "--root",
            "--topo-order",
            "-z",
            "--format=%H",
            "--no-patch",
            "--no-color",
            "--no-ext-diff",
            "--no-textconv",
            "--no-notes",
            "--no-decorate",
            &format!("--max-count={}", count + 1),
            tip.as_str(),
            "--",
        ]);
        arguments.push(path.argument()?);
        let output = self.read_diff("directory-history", &arguments, cancel)?;
        let ids = if output.is_empty() {
            Vec::new()
        } else {
            output
                .strip_suffix(&[0])
                .ok_or_else(|| Error::parse("directory-history", "missing record terminator"))?
                .split(|b| *b == 0)
                .map(|id| parse::oid(id, "directory-history"))
                .collect::<Result<Vec<_>>>()?
        };
        let more = ids.len() > count;
        let ids: Vec<_> = ids.into_iter().take(count).skip(offset).collect();
        let entries = self
            .file_commits(&ids, cancel)?
            .into_iter()
            .map(|commit| FileHistoryEntry {
                commit,
                path: path.clone(),
                previous_path: None,
                deleted: false,
            })
            .collect();
        cancel.check()?;
        Ok(FileHistoryPage {
            directory: true,
            tip: tip.clone(),
            path: path.clone(),
            entries,
            next: (more && count < HISTORY_LIMIT).then(|| FileHistoryCursor {
                repository: self.info().git_dir.clone(),
                tip,
                path,
                offset: count,
                directory: true,
            }),
            limited: more && count == HISTORY_LIMIT,
        })
    }

    fn file_commits(&self, ids: &[ObjectId], cancel: &CancellationToken) -> Result<Vec<Commit>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        // Bind the repository just like read_diff, including for the stdin batch.
        let mut arguments = args(&["--git-dir"]);
        arguments.push(crate::runner::git_path(&self.info().git_dir));
        arguments.extend(args(&["cat-file", "--batch"]));
        let input = ids
            .iter()
            .flat_map(|id| id.as_str().bytes().chain(std::iter::once(b'\n')))
            .collect();
        let bytes = self
            .runner
            .run(self.cwd(), &arguments, input, cancel)?
            .success("file-history")?;
        parse::commits(&bytes, ids)
    }

    /// Read an exact committed blob, never the working file or a symlink target.
    pub fn historical_file(
        &self,
        commit: &ObjectId,
        path: &RepoPath,
        cancel: &CancellationToken,
    ) -> Result<HistoricalFile> {
        cancel.check()?;
        if self.is_preview() {
            // ls-tree -l reads the blob for its size: download this version first.
            let mut entry = args(&["ls-tree", "-z", "--full-tree", commit.as_str(), "--"]);
            entry.push(path.argument()?);
            let listed = self.read_diff("file-content", &entry, cancel)?;
            if let Some(id) = listed.split(|b| *b == b' ' || *b == b'\t').nth(2) {
                self.prepare_objects(&[parse::oid(id, "file-content")?], cancel)?;
            }
        }
        let mut arguments = args(&["ls-tree", "-l", "-z", "--full-tree", commit.as_str(), "--"]);
        arguments.push(path.argument()?);
        let output = self.read_diff("file-content", &arguments, cancel)?;
        if output.is_empty() {
            return Ok(HistoricalFile {
                kind: HistoricalFileKind::Missing,
                bytes: Vec::new(),
            });
        }
        let record = output
            .strip_suffix(&[0])
            .ok_or_else(|| malformed("unterminated tree record"))?;
        let tab = record
            .iter()
            .position(|b| *b == b'\t')
            .ok_or_else(|| malformed("invalid tree entry"))?;
        if &record[tab + 1..] != path.as_bytes() {
            return Err(malformed("tree returned a different path"));
        }
        let fields: Vec<_> = record[..tab]
            .split(|b| b.is_ascii_whitespace())
            .filter(|s| !s.is_empty())
            .collect();
        if fields.len() != 4 {
            return Err(malformed("invalid tree metadata"));
        }
        if fields[1] != b"blob" {
            return Ok(HistoricalFile {
                kind: HistoricalFileKind::NonFile,
                bytes: Vec::new(),
            });
        }
        let oid = parse::oid(fields[2], "file-content")?;
        let size = number(fields[3])?;
        if size > FILE_LIMIT {
            return Err(Error::Unsupported(
                "Historical file exceeds the 2 MiB preview limit.".into(),
            ));
        }
        let bytes = self.read_diff(
            "file-content",
            &args(&["cat-file", "blob", oid.as_str()]),
            cancel,
        )?;
        if bytes.len() != size {
            return Err(malformed("blob size does not match tree metadata"));
        }
        let kind = if bytes.contains(&0) || std::str::from_utf8(&bytes).is_err() {
            HistoricalFileKind::Binary
        } else {
            HistoricalFileKind::Text
        };
        cancel.check()?;
        Ok(HistoricalFile { kind, bytes })
    }

    pub fn file_blame(
        &self,
        commit: &ObjectId,
        path: &RepoPath,
        first: usize,
        size: usize,
        cancel: &CancellationToken,
    ) -> Result<BlamePage> {
        cancel.check()?;
        if first == 0 || !(1..=500).contains(&size) {
            return Err(Error::InvalidInput(
                "blame requires a positive start line and page size 1..500".into(),
            ));
        }
        let file = self.historical_file(commit, path, cancel)?;
        if file.kind != HistoricalFileKind::Text {
            return Err(Error::Unsupported(
                match file.kind {
                    HistoricalFileKind::Missing => {
                        "This file is absent at the selected commit (it may have been deleted)."
                    }
                    HistoricalFileKind::Binary => {
                        "Blame is available for UTF-8 text files, not binary or other encodings."
                    }
                    _ => "Blame requires a file, not a directory or submodule.",
                }
                .into(),
            ));
        }
        let total = file.bytes.iter().filter(|b| **b == b'\n').count()
            + usize::from(!file.bytes.is_empty() && !file.bytes.ends_with(b"\n"));
        if total == 0 {
            return Ok(BlamePage {
                lines: Vec::new(),
                total_lines: 0,
                next_line: None,
            });
        }
        if first > total {
            return Err(Error::InvalidInput(
                "blame start line is beyond the committed file".into(),
            ));
        }
        let last = first.saturating_add(size - 1).min(total);
        self.prepare_path(commit.as_str(), path, cancel)?;
        let mut arguments = args(&[
            "-c",
            "blame.ignoreRevsFile=",
            "blame",
            "--line-porcelain",
            "--encoding=none",
            "--no-textconv",
            "-L",
            &format!("{first},{last}"),
            commit.as_str(),
            "--",
        ]);
        arguments.push(path.argument()?);
        let bytes = self.read_diff("file-blame", &arguments, cancel)?;
        let lines = blame_records(&bytes, first, last, cancel)?;
        // Verify attribution lines correspond to the immutable blob read above.
        for (line, source) in lines
            .iter()
            .zip(file.bytes.split(|b| *b == b'\n').skip(first - 1))
        {
            if line.content != source {
                return Err(malformed("blame content does not match committed blob"));
            }
        }
        cancel.check()?;
        Ok(BlamePage {
            lines,
            total_lines: total,
            next_line: (last < total).then_some(last + 1),
        })
    }
}

struct Record {
    id: ObjectId,
    path: RepoPath,
    previous: Option<RepoPath>,
    deleted: bool,
}
fn malformed(detail: &str) -> Error {
    Error::parse("file history/blame", detail)
}
fn number(bytes: &[u8]) -> Result<usize> {
    std::str::from_utf8(bytes)
        .ok()
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| malformed("invalid line number or size"))
}
fn history_records(
    bytes: &[u8],
    initial: &RepoPath,
    cancel: &CancellationToken,
) -> Result<Vec<Record>> {
    if !bytes.is_empty() && !bytes.ends_with(&[0]) {
        return Err(malformed("unterminated history"));
    }
    let mut tokens = bytes.split(|b| *b == 0);
    let mut out: Vec<Record> = Vec::new();
    let mut current = initial.clone();
    for_token(&mut tokens, &mut out, &mut current, cancel)?;
    Ok(out)
}
fn for_token<'a>(
    tokens: &mut impl Iterator<Item = &'a [u8]>,
    out: &mut Vec<Record>,
    current: &mut RepoPath,
    cancel: &CancellationToken,
) -> Result<()> {
    while let Some(token) = tokens.next() {
        cancel.check()?;
        let token = token.strip_prefix(b"\n").unwrap_or(token);
        if token.is_empty() {
            continue;
        }
        if let Some(id) = token.strip_prefix(b"ROCKET-COMMIT ") {
            out.push(Record {
                id: parse::oid(id, "file-history")?,
                path: current.clone(),
                previous: None,
                deleted: false,
            });
            continue;
        }
        let record = out
            .last_mut()
            .ok_or_else(|| malformed("change before history header"))?;
        let status = *token
            .first()
            .ok_or_else(|| malformed("missing change status"))?;
        if !b"AMDRCT".contains(&status) {
            return Err(malformed("unsupported file change status"));
        }
        let first = RepoPath::new(
            tokens
                .next()
                .ok_or_else(|| malformed("missing change path"))?
                .to_vec(),
        )?;
        let (path, previous) = if matches!(status, b'R' | b'C') {
            let path = RepoPath::new(
                tokens
                    .next()
                    .ok_or_else(|| malformed("missing rename destination"))?
                    .to_vec(),
            )?;
            (path, Some(first))
        } else {
            (first, None)
        };
        // --follow emits the selected file's change. Reject ambiguity instead of
        // deriving a path from a human-formatted patch or silently choosing a file.
        if path != *current {
            return Err(Error::Unsupported("Git could not follow one file through this history. Choose an exact file path (not a directory) or start at an older commit.".into()));
        }
        record.path = path;
        record.previous = previous.clone();
        record.deleted = status == b'D';
        if let Some(previous) = previous {
            *current = previous;
        }
    }
    Ok(())
}

fn blame_records(
    bytes: &[u8],
    first: usize,
    last: usize,
    cancel: &CancellationToken,
) -> Result<Vec<BlameLine>> {
    let mut input = bytes
        .strip_suffix(b"\n")
        .unwrap_or(bytes)
        .split(|b| *b == b'\n');
    let mut out = Vec::new();
    while let Some(header) = input.next() {
        cancel.check()?;
        let fields: Vec<_> = header.split(|b| *b == b' ').collect();
        if !(3..=4).contains(&fields.len()) {
            return Err(malformed("invalid blame header"));
        }
        let commit = parse::oid(fields[0], "file-blame")?;
        let original_line = number(fields[1])?;
        let line = number(fields[2])?;
        if original_line == 0 || line != first + out.len() || line > last {
            return Err(malformed("out-of-order blame line"));
        }
        let (mut author, mut summary, mut path, mut content) = (None, None, None, None);
        let mut boundary = false;
        for metadata in input.by_ref() {
            if let Some(value) = metadata.strip_prefix(b"\t") {
                content = Some(value.to_vec());
                break;
            }
            if let Some(value) = metadata.strip_prefix(b"author ") {
                author = Some(value.to_vec());
            }
            if let Some(value) = metadata.strip_prefix(b"summary ") {
                summary = Some(value.to_vec());
            }
            if let Some(value) = metadata.strip_prefix(b"filename ") {
                path = Some(RepoPath::new(unquote(value)?)?);
            }
            if metadata == b"boundary" {
                boundary = true;
            }
        }
        out.push(BlameLine {
            commit,
            original_line,
            line,
            path: path.ok_or_else(|| malformed("blame filename missing"))?,
            author: author.ok_or_else(|| malformed("blame author missing"))?,
            summary: summary.ok_or_else(|| malformed("blame summary missing"))?,
            content: content.ok_or_else(|| malformed("blame content missing"))?,
            boundary,
        });
    }
    if out.len() != last - first + 1 {
        return Err(malformed("incomplete blame output"));
    }
    Ok(out)
}

fn unquote(bytes: &[u8]) -> Result<Vec<u8>> {
    if !bytes.starts_with(b"\"") {
        return Ok(bytes.to_vec());
    }
    let inner = bytes
        .strip_prefix(b"\"")
        .and_then(|s| s.strip_suffix(b"\""))
        .ok_or_else(|| malformed("unterminated quoted filename"))?;
    let mut out = Vec::new();
    let mut i = 0;
    while i < inner.len() {
        let b = inner[i];
        i += 1;
        if b != b'\\' {
            out.push(b);
            continue;
        }
        let b = *inner
            .get(i)
            .ok_or_else(|| malformed("incomplete filename escape"))?;
        i += 1;
        let value = match b {
            b'a' => 7,
            b'b' => 8,
            b't' => 9,
            b'n' => 10,
            b'v' => 11,
            b'f' => 12,
            b'r' => 13,
            b'"' => b'"',
            b'\\' => b'\\',
            b'0'..=b'3' => {
                let rest = inner
                    .get(i..i + 2)
                    .ok_or_else(|| malformed("incomplete octal escape"))?;
                if !rest.iter().all(|b| matches!(b, b'0'..=b'7')) {
                    return Err(malformed("invalid octal escape"));
                }
                i += 2;
                (b - b'0') * 64 + (rest[0] - b'0') * 8 + (rest[1] - b'0')
            }
            _ => return Err(malformed("invalid filename escape")),
        };
        out.push(value);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn porcelain_rejects_missing_metadata_truncated_content_and_wrong_line_numbers() {
        let oid = "1".repeat(40);
        let good = format!(
            "{oid} 9 1 1\nauthor 林安\nsummary hello\nfilename \"old\\tname.txt\"\n\tfilename is source text\n"
        );
        let rows = blame_records(good.as_bytes(), 1, 1, &CancellationToken::new()).unwrap();
        assert_eq!(rows[0].path.as_bytes(), b"old\tname.txt");
        assert_eq!(rows[0].content, b"filename is source text");
        for bad in [
            good.replace("filename \"old\\tname.txt\"\n", ""),
            good.replace("\tfilename is source text\n", ""),
            good.replace(" 9 1 1", " 9 2 1"),
        ] {
            assert!(blame_records(bad.as_bytes(), 1, 1, &CancellationToken::new()).is_err());
        }
        assert!(blame_records(good.as_bytes(), 1, 2, &CancellationToken::new()).is_err());
        for bad in [
            b"\"bad\\q\"".as_slice(),
            b"\"bad\\400\"",
            b"\"bad\\0\"",
            b"\"unclosed",
        ] {
            assert!(unquote(bad).is_err());
        }
    }
    #[test]
    fn quoted_utf8_bytes_and_nul_history_paths_are_lossless() {
        assert_eq!(
            unquote(b"\"\\346\\226\\207 \\360\\237\\232\\200\"").unwrap(),
            "文 🚀".as_bytes()
        );
        let path = RepoPath::new(b"ROCKET-COMMIT filename\nM".to_vec()).unwrap();
        let bytes = format!(
            "\0ROCKET-COMMIT {}\0\0\nA\0{}\0",
            "2".repeat(64),
            path.display_lossy()
        );
        let records = history_records(bytes.as_bytes(), &path, &CancellationToken::new()).unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].path, path);
    }
}
