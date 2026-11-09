//! Search inside files with `git grep`: at the checkout (files on disk, tracked
//! ones and untracked ones that aren't ignored) or at a commit. One Git process,
//! read while it streams, bounded by match, per-file, line, byte and time limits.
//! Binary files are skipped. Paths stay exact bytes; nothing reaches a shell.
use crate::{lfs::LfsPointer, runner::args, *};
use std::{
    collections::VecDeque,
    io::{BufRead, Read},
    ops::Range,
    path::PathBuf,
    time::{Duration, Instant},
};

/// Matching lines kept in all; at this many the search stops.
pub const FILE_SEARCH_MATCHES: usize = 2_000;
/// Matching lines kept per file (Git's `--max-count`).
pub const FILE_SEARCH_FILE_MATCHES: usize = 100;
/// Characters shown of one matching line.
pub const FILE_SEARCH_LINE_CHARS: usize = 400;
/// A search ends after this long and keeps what it found.
pub const FILE_SEARCH_TIME: Duration = Duration::from_secs(30);
/// Lines shown either side of a match in a file too large to show whole.
pub const FILE_WINDOW_LINES: u64 = 200;
/// Bytes of one line kept in such a window.
pub const FILE_WINDOW_LINE_BYTES: usize = 8 * 1024;

/// Bytes of a line kept before its first match, and in all.
const BEFORE: usize = 160;
const WINDOW: usize = 1_600;
/// Match positions kept per line.
const MARKS: usize = 64;
/// Matching lines Git may write before the search stops early anyway.
const STREAM_BYTES: usize = 256 * 1024 * 1024;
const PATH_BYTES: usize = 64 * 1024;
/// Git marks each match with this colour; file text never holds it by accident
/// in practice, and other colours are switched off.
const MARK_COLOR: &str = "#010203";
const MARK_START: &[u8] = b"\x1b[38;2;1;2;3m";
const MARK_END: &[u8] = b"\x1b[m";
const BAD_PATTERN: &str = "Not a valid regular expression: ";
const NOT_DOWNLOADED: &str =
    "Some file contents of this commit aren't downloaded, so it can't be searched here.";
const LFS_LINES: [&[u8]; 3] = [
    b"version https://git-lfs.github.com/spec/v1",
    b"oid sha256:",
    b"size ",
];

/// Where a file search looks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FileSearchSource {
    /// Files on disk now: tracked ones and untracked ones that aren't ignored.
    Checkout,
    /// Files as committed in this commit.
    Commit(ObjectId),
}

#[derive(Clone, Debug)]
pub struct FileSearchQuery {
    /// One line of text, or a POSIX extended regular expression with `regex`.
    pub text: String,
    pub regex: bool,
    /// Letters must match exactly; otherwise case is ignored.
    pub match_case: bool,
    /// Only this file or folder, exact and case-sensitive.
    pub path: Option<RepoPath>,
    pub source: FileSearchSource,
}

/// One matching line.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FileSearchLine {
    /// 1-based line number.
    pub number: u64,
    /// The line around its first match: at most [`FILE_SEARCH_LINE_CHARS`]
    /// characters, tabs as four spaces, control characters escaped.
    pub text: String,
    /// Matches in `text`, as character ranges.
    pub marks: Vec<Range<usize>>,
    /// Matches in the whole line, as byte ranges.
    pub columns: Vec<Range<usize>>,
    /// `text` starts after the line starts, or ends before it ends.
    pub cut_start: bool,
    pub cut_end: bool,
    /// The shown bytes aren't valid UTF-8; a replacement mark stands in for them.
    pub not_utf8: bool,
}

/// A file's matching lines, in line order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FileSearchFile {
    /// Exact bytes of the path, relative to the repository root.
    pub path: Vec<u8>,
    pub lines: Vec<FileSearchLine>,
    /// [`FILE_SEARCH_FILE_MATCHES`] lines matched; Git didn't look for more.
    pub capped: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileSearchEnd {
    Complete,
    /// [`FILE_SEARCH_MATCHES`] lines matched, or matching lines were too long to
    /// read them all; the rest wasn't searched.
    TooMany,
    /// [`FILE_SEARCH_TIME`] passed; only files before that were searched.
    TimedOut,
}

/// Facts read after the search, one per reported file.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FileSearchDetail {
    pub size: Option<u64>,
    /// A Git LFS pointer: the file's content is stored outside Git, so only the
    /// pointer was searched.
    pub lfs_pointer: bool,
}

#[derive(Clone, Debug)]
pub struct FileSearchSummary {
    pub end: FileSearchEnd,
    pub files: usize,
    pub matches: usize,
    /// One per file, in the order the files were reported.
    pub details: Vec<FileSearchDetail>,
    pub elapsed: Duration,
}

/// A file opened at a match.
#[derive(Clone, Debug)]
pub enum SearchFileContent {
    Whole(HistoricalFile),
    /// Too large to show whole: lines around the match.
    Window(FileWindow),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FileWindow {
    /// 1-based number of the first line in `lines`.
    pub first: u64,
    /// Raw bytes of each line, without its newline.
    pub lines: Vec<Vec<u8>>,
    pub size: u64,
    /// Indexes into `lines` of lines cut at [`FILE_WINDOW_LINE_BYTES`], on a character edge.
    pub cut: Vec<usize>,
    /// The file ends inside the window.
    pub reaches_end: bool,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct SearchLimits {
    pub matches: usize,
    pub per_file: usize,
    pub time: Duration,
}
impl Default for SearchLimits {
    fn default() -> Self {
        Self {
            matches: FILE_SEARCH_MATCHES,
            per_file: FILE_SEARCH_FILE_MATCHES,
            time: FILE_SEARCH_TIME,
        }
    }
}

pub(crate) fn bad_pattern(reason: &str) -> String {
    format!("{BAD_PATTERN}{reason}")
}

/// Git's reason when `error` is a bad regular expression, for marking the field.
pub fn pattern_error(error: &Error) -> Option<&str> {
    match error {
        Error::InvalidInput(message) => message.strip_prefix(BAD_PATTERN),
        _ => None,
    }
}

/// Git's reason for refusing a regular expression, without the pattern:
/// `log -G` writes "fatal: invalid regex: <reason>" and `grep -E` writes
/// "fatal: -e option, '<pattern>': <reason>".
pub(crate) fn regex_failure(stderr: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(stderr);
    text.lines().find_map(|line| {
        let rest = line.strip_prefix("fatal: ")?;
        let reason = match rest.strip_prefix("invalid regex: ") {
            Some(reason) => reason,
            None => rest.strip_prefix("-e option, '")?.rsplit_once("': ")?.1,
        };
        let reason: String = reason
            .chars()
            .filter(|c| !c.is_control())
            .take(200)
            .collect();
        let reason = reason.trim();
        (!reason.is_empty()).then(|| reason.to_owned())
    })
}

fn check(query: &FileSearchQuery) -> Result<()> {
    if query.text.trim().is_empty()
        || query.text.len() > 1024
        || query.text.chars().any(char::is_control)
    {
        return Err(Error::InvalidInput(
            "Enter one line of text, up to 1,024 bytes.".into(),
        ));
    }
    Ok(())
}

impl Repository {
    /// Search inside files, handing each file's matches to `found` as Git reports
    /// them, in path order. Cancellation stops Git at once and returns
    /// `Err(Cancelled)` after the files found so far were handed over.
    pub fn search_files(
        &self,
        query: &FileSearchQuery,
        cancel: &CancellationToken,
        found: &mut (dyn FnMut(FileSearchFile) + Send),
    ) -> Result<FileSearchSummary> {
        self.search_files_within(query, cancel, found, SearchLimits::default())
    }

    pub(crate) fn search_files_within(
        &self,
        query: &FileSearchQuery,
        cancel: &CancellationToken,
        found: &mut (dyn FnMut(FileSearchFile) + Send),
        limits: SearchLimits,
    ) -> Result<FileSearchSummary> {
        let started = Instant::now();
        check(query)?;
        cancel.check()?;
        let mut arguments = Vec::new();
        let prefix = match &query.source {
            FileSearchSource::Checkout => {
                if self.info().worktree.is_none() {
                    return Err(Error::InvalidInput(
                        "This repository has no working tree. Search a revision instead.".into(),
                    ));
                }
                None
            }
            FileSearchSource::Commit(id) => {
                // A preview downloads that commit's files first; grep reads them all.
                self.prepare_tree(id.as_str(), cancel)?;
                // Read the recorded objects, as browsing does.
                arguments.push("--no-replace-objects".into());
                Some(format!("{}:", id.as_str()).into_bytes())
            }
        };
        for (slot, color) in [
            ("filename", "normal"),
            ("lineNumber", "normal"),
            ("column", "normal"),
            ("separator", "normal"),
            ("function", "normal"),
            ("selected", "normal"),
            ("context", "normal"),
            ("matchContext", MARK_COLOR),
            ("matchSelected", MARK_COLOR),
        ] {
            arguments.push("-c".into());
            arguments.push(format!("color.grep.{slot}={color}").into());
        }
        arguments.extend(args(&[
            "grep",
            "--color=always",
            "--no-column",
            "-z",
            "-n",
            "-I",
            "--full-name",
            "--no-textconv",
            "--max-count",
        ]));
        arguments.push(limits.per_file.to_string().into());
        arguments.push(if query.regex { "-E" } else { "-F" }.into());
        if !query.match_case {
            arguments.push("-i".into());
        }
        if query.source == FileSearchSource::Checkout {
            arguments.push("--untracked".into());
        }
        arguments.push("-e".into());
        arguments.push(query.text.clone().into());
        if let FileSearchSource::Commit(id) = &query.source {
            arguments.push(id.as_str().into());
        }
        if let Some(path) = &query.path {
            arguments.push("--".into());
            arguments.push(path.argument()?);
        }
        let mut parser = Parser::new(prefix, limits);
        let mut reported = Vec::new();
        let stream_limits = CommandLimits {
            timeout: limits.time,
            stdout_bytes: STREAM_BYTES,
            stderr_bytes: 1024 * 1024,
        };
        let outcome = {
            let mut sink = |chunk: &[u8]| {
                let more = parser.feed(chunk);
                for (file, pointer) in parser.ready.drain(..) {
                    reported.push((file.path.clone(), pointer));
                    found(file);
                }
                more
            };
            self.runner.step("file-search", "grep", || {
                self.runner.read_streamed(
                    self.cwd(),
                    &arguments,
                    Vec::new(),
                    cancel,
                    &stream_limits,
                    &mut sink,
                )
            })
        };
        // Whatever happened, the last file's complete lines are kept.
        parser.finish();
        for (file, pointer) in parser.ready.drain(..) {
            reported.push((file.path.clone(), pointer));
            found(file);
        }
        if let Some(error) = parser.error.take() {
            return Err(error);
        }
        // Stop is final even when Git had already written everything.
        cancel.check()?;
        let end = match outcome {
            Ok(_) if parser.full => FileSearchEnd::TooMany,
            Ok(output) => match output.status.code() {
                Some(0 | 1) => FileSearchEnd::Complete,
                _ => return Err(grep_failure(query, output)),
            },
            Err(Error::TimedOut) => FileSearchEnd::TimedOut,
            Err(Error::OutputLimit { stream: "stdout", .. }) => FileSearchEnd::TooMany,
            Err(error) => return Err(error),
        };
        let details = match self.search_details(&query.source, &reported, cancel) {
            Ok(details) => details,
            Err(Error::Cancelled) => return Err(Error::Cancelled),
            // Sizes and LFS labels are extras; a failure leaves them unknown.
            Err(_) => vec![FileSearchDetail::default(); reported.len()],
        };
        Ok(FileSearchSummary {
            end,
            files: reported.len(),
            matches: parser.matches,
            details,
            elapsed: started.elapsed(),
        })
    }

    /// Size of each reported file, and whether a pointer-like match is really a
    /// Git LFS pointer (checked with Rocket's own parser, files up to 1 KiB).
    fn search_details(
        &self,
        source: &FileSearchSource,
        files: &[(Vec<u8>, bool)],
        cancel: &CancellationToken,
    ) -> Result<Vec<FileSearchDetail>> {
        let mut details = vec![FileSearchDetail::default(); files.len()];
        if files.is_empty() {
            return Ok(details);
        }
        match source {
            FileSearchSource::Commit(commit) => {
                let objects = self.blob_info(commit, files.iter().map(|(p, _)| p.as_slice()), cancel)?;
                let mut pointers = Vec::new();
                for (i, info) in objects.iter().enumerate() {
                    if let Some((id, size)) = info {
                        details[i].size = Some(*size);
                        if files[i].1 && *size <= 1024 {
                            pointers.push((i, id.clone()));
                        }
                    }
                }
                if !pointers.is_empty() {
                    let blobs = self.small_blobs(pointers.iter().map(|(_, id)| id), cancel)?;
                    for ((i, _), bytes) in pointers.iter().zip(blobs) {
                        details[*i].lfs_pointer = bytes.is_some_and(|b| LfsPointer::parse(&b).is_ok());
                    }
                }
            }
            FileSearchSource::Checkout => {
                for (i, (path, pointer)) in files.iter().enumerate() {
                    cancel.check()?;
                    let Ok(full) = self.checkout_path(path) else { continue };
                    let Ok(meta) = std::fs::symlink_metadata(&full) else { continue };
                    if !meta.is_file() || crate::local_files::redirected(&meta) {
                        continue;
                    }
                    details[i].size = Some(meta.len());
                    if *pointer && meta.len() <= 1024 {
                        details[i].lfs_pointer = std::fs::read(&full)
                            .is_ok_and(|bytes| LfsPointer::parse(&bytes).is_ok());
                    }
                }
            }
        }
        Ok(details)
    }

    /// Blob ID and size of each path in `commit`; `None` where it isn't a blob.
    fn blob_info<'a>(
        &self,
        commit: &ObjectId,
        paths: impl Iterator<Item = &'a [u8]>,
        cancel: &CancellationToken,
    ) -> Result<Vec<Option<(ObjectId, u64)>>> {
        let mut input = Vec::new();
        let mut count = 0;
        for path in paths {
            input.extend_from_slice(commit.as_str().as_bytes());
            input.push(b':');
            input.extend_from_slice(path);
            input.push(0);
            count += 1;
        }
        let output = self
            .runner
            .run(
                self.cwd(),
                &args(&[
                    "--no-replace-objects",
                    "cat-file",
                    "--batch-check=%(objectname) %(objecttype) %(objectsize)",
                    "-Z",
                ]),
                input,
                cancel,
            )?
            .success("file-search")?;
        let records: Vec<&[u8]> = output
            .strip_suffix(b"\0")
            .unwrap_or(&output)
            .split(|b| *b == 0)
            .collect();
        if records.len() != count {
            return Err(Error::parse("file-search", "object list length"));
        }
        Ok(records
            .into_iter()
            .map(|record| {
                let mut fields = record.split(|b| *b == b' ');
                let id = fields.next().and_then(|v| crate::parse::oid(v, "file-search").ok())?;
                (fields.next()? == b"blob").then_some(())?;
                let size = std::str::from_utf8(fields.next()?).ok()?.parse().ok()?;
                fields.next().is_none().then_some((id, size))
            })
            .collect())
    }

    /// Contents of small blobs by ID, in order.
    fn small_blobs<'a>(
        &self,
        ids: impl Iterator<Item = &'a ObjectId>,
        cancel: &CancellationToken,
    ) -> Result<Vec<Option<Vec<u8>>>> {
        let mut input = Vec::new();
        let mut count = 0;
        for id in ids {
            input.extend_from_slice(id.as_str().as_bytes());
            input.push(0);
            count += 1;
        }
        let output = self
            .runner
            .run(
                self.cwd(),
                &args(&["--no-replace-objects", "cat-file", "--batch", "-Z"]),
                input,
                cancel,
            )?
            .success("file-search")?;
        let mut rest = output.as_slice();
        let mut blobs = Vec::with_capacity(count);
        for _ in 0..count {
            let end = rest
                .iter()
                .position(|b| *b == 0)
                .ok_or_else(|| Error::parse("file-search", "blob header"))?;
            let header = &rest[..end];
            rest = &rest[end + 1..];
            let mut fields = header.split(|b| *b == b' ');
            let (_, kind, size) = (fields.next(), fields.next(), fields.next());
            let size: Option<usize> = size
                .and_then(|s| std::str::from_utf8(s).ok())
                .and_then(|s| s.parse().ok());
            match (kind, size) {
                (Some(b"blob"), Some(size)) if rest.len() > size && rest[size] == 0 => {
                    blobs.push(Some(rest[..size].to_vec()));
                    rest = &rest[size + 1..];
                }
                (Some(_), Some(size)) if rest.len() > size => {
                    blobs.push(None);
                    rest = &rest[size + 1..];
                }
                _ => blobs.push(None),
            }
        }
        Ok(blobs)
    }

    /// A checkout file's path on disk, inside the working tree.
    fn checkout_path(&self, path: &[u8]) -> Result<PathBuf> {
        let worktree = self.info().worktree.as_ref().ok_or_else(|| {
            Error::InvalidInput("This repository has no working tree.".into())
        })?;
        let text = std::str::from_utf8(path)
            .map_err(|_| Error::InvalidInput("This path isn't valid text on Windows.".into()))?;
        let mut full = worktree.clone();
        for part in text.split('/') {
            // On Windows `\` and `:` would let a component leave the working tree.
            if part.is_empty() || part == "." || part == ".." || (cfg!(windows) && part.contains(['\\', ':'])) {
                return Err(Error::InvalidInput("Not a path inside the repository.".into()));
            }
            full.push(part);
        }
        Ok(full)
    }

    /// Whether Git reads `text` as a POSIX extended regular expression: a grep of the
    /// empty tree compiles it and searches nothing. `git log -G` compiles its
    /// expression only when it reads a commit's diff, so a step that reads none (a path
    /// no commit in it touched) wouldn't say the expression is bad.
    pub(crate) fn check_expression(&self, text: &str, cancel: &CancellationToken) -> Result<()> {
        let empty = match self.info().object_format {
            ObjectFormat::Sha1 => "4b825dc642cb6eb9a060e54bf8d69288fbee4904",
            ObjectFormat::Sha256 => "6ef19b41225c5369f1c104d45d8d85efa9b057b53b14b4b9b939dd74decc5321",
        };
        let arguments = vec!["grep".into(), "-E".into(), "-e".into(), text.into(), empty.into()];
        let output = self.runner.run(self.cwd(), &arguments, Vec::new(), cancel)?;
        if output.status.code() == Some(128)
            && let Some(reason) = regex_failure(&output.stderr)
        {
            return Err(Error::InvalidInput(bad_pattern(&reason)));
        }
        match output.status.code() {
            Some(0 | 1) => Ok(()),
            _ => Err(Error::Git {
                operation: "history-search",
                code: output.status.code(),
                stdout: Vec::new(),
                stderr: output.stderr,
            }),
        }
    }

    /// Open a file at a search match: whole up to the 2 MiB preview limit,
    /// otherwise [`FILE_WINDOW_LINES`] lines either side of `line`.
    pub fn search_file(
        &self,
        source: &FileSearchSource,
        path: &[u8],
        line: u64,
        cancel: &CancellationToken,
    ) -> Result<SearchFileContent> {
        cancel.check()?;
        let first = line.saturating_sub(FILE_WINDOW_LINES).max(1);
        let last = line.saturating_add(FILE_WINDOW_LINES);
        match source {
            FileSearchSource::Commit(commit) => {
                let info = self.blob_info(commit, std::iter::once(path), cancel)?;
                let Some(Some((id, size))) = info.into_iter().next() else {
                    return Err(Error::InvalidInput(
                        "This file isn't in that commit any more.".into(),
                    ));
                };
                if size as usize <= crate::TREE_FILE_LIMIT {
                    let bytes = self
                        .runner
                        .run(
                            self.cwd(),
                            &args(&["--no-replace-objects", "cat-file", "blob", id.as_str()]),
                            Vec::new(),
                            cancel,
                        )?
                        .success("file-search")?;
                    return Ok(SearchFileContent::Whole(historical(bytes)));
                }
                let mut window = WindowReader::new(first, last, size);
                let limits = CommandLimits {
                    stdout_bytes: usize::MAX,
                    ..self.runner.limits.clone()
                };
                let output = self.runner.read_streamed(
                    self.cwd(),
                    &args(&["--no-replace-objects", "cat-file", "blob", id.as_str()]),
                    Vec::new(),
                    cancel,
                    &limits,
                    &mut |chunk| window.feed(chunk),
                )?;
                if !window.done && !output.status.success() {
                    return Err(Error::Git {
                        operation: "file-search",
                        code: output.status.code(),
                        stdout: Vec::new(),
                        stderr: output.stderr,
                    });
                }
                Ok(SearchFileContent::Window(window.finish()))
            }
            FileSearchSource::Checkout => {
                let full = self.checkout_path(path)?;
                let root = std::fs::canonicalize(self.info().worktree.as_ref().unwrap())
                    .map_err(|e| Error::io("find the working tree", e))?;
                let real = std::fs::canonicalize(&full)
                    .map_err(|e| Error::io("open the file", e))?;
                if !real.starts_with(&root) {
                    return Err(Error::InvalidInput(
                        "This file leads outside the repository, so it isn't shown.".into(),
                    ));
                }
                let file = std::fs::File::open(&real).map_err(|e| Error::io("open the file", e))?;
                let meta = file.metadata().map_err(|e| Error::io("inspect the file", e))?;
                if !meta.is_file() {
                    return Err(Error::InvalidInput("This is not a file.".into()));
                }
                if meta.len() as usize <= crate::TREE_FILE_LIMIT {
                    let mut bytes = Vec::new();
                    (&file)
                        .take(crate::TREE_FILE_LIMIT as u64 + 1)
                        .read_to_end(&mut bytes)
                        .map_err(|e| Error::io("read the file", e))?;
                    if bytes.len() <= crate::TREE_FILE_LIMIT {
                        return Ok(SearchFileContent::Whole(historical(bytes)));
                    }
                }
                // Large, or it grew since it was measured: read it again from the start.
                let file = std::fs::File::open(&real).map_err(|e| Error::io("open the file", e))?;
                let mut window = WindowReader::new(first, last, meta.len());
                let mut reader = std::io::BufReader::with_capacity(64 * 1024, file);
                loop {
                    cancel.check()?;
                    let chunk = reader.fill_buf().map_err(|e| Error::io("read the file", e))?;
                    if chunk.is_empty() {
                        break;
                    }
                    let length = chunk.len();
                    let more = window.feed(chunk);
                    reader.consume(length);
                    if !more {
                        break;
                    }
                }
                Ok(SearchFileContent::Window(window.finish()))
            }
        }
    }
}

/// Classify bytes the way the file browser does: text when valid UTF-8 without NUL.
fn historical(bytes: Vec<u8>) -> HistoricalFile {
    let kind = if bytes.contains(&0) || std::str::from_utf8(&bytes).is_err() {
        HistoricalFileKind::Binary
    } else {
        HistoricalFileKind::Text
    };
    HistoricalFile { kind, bytes }
}

fn grep_failure(query: &FileSearchQuery, output: crate::runner::Output) -> Error {
    if query.regex
        && output.status.code() == Some(128)
        && let Some(reason) = regex_failure(&output.stderr)
    {
        return Error::InvalidInput(bad_pattern(&reason));
    }
    let text = String::from_utf8_lossy(&output.stderr);
    if text.contains("from promisor remote") || text.contains("lazy fetching disabled") {
        return Error::InvalidInput(NOT_DOWNLOADED.into());
    }
    Error::Git {
        operation: "file-search",
        code: output.status.code(),
        stdout: Vec::new(),
        stderr: output.stderr,
    }
}

/// Keeps lines `first..=last` of a stream, each cut at [`FILE_WINDOW_LINE_BYTES`].
struct WindowReader {
    first: u64,
    last: u64,
    line: u64,
    current: Vec<u8>,
    lines: Vec<Vec<u8>>,
    size: u64,
    cut: Vec<usize>,
    /// The current line is longer than [`FILE_WINDOW_LINE_BYTES`].
    line_cut: bool,
    done: bool,
}
impl WindowReader {
    fn new(first: u64, last: u64, size: u64) -> Self {
        Self {
            first,
            last,
            line: 1,
            current: Vec::new(),
            lines: Vec::new(),
            size,
            cut: Vec::new(),
            line_cut: false,
            done: false,
        }
    }
    /// Keep the current line: a cut one ends on a character edge, a whole one loses a CRLF's \r.
    fn keep_line(&mut self) {
        let mut line = std::mem::take(&mut self.current);
        if self.line_cut {
            line.truncate(utf8_prefix(&line));
            self.cut.push(self.lines.len());
        } else if line.last() == Some(&b'\r') {
            line.pop();
        }
        self.lines.push(line);
    }
    /// False once the window is complete.
    fn feed(&mut self, chunk: &[u8]) -> bool {
        for &b in chunk {
            if b == b'\n' {
                if self.line >= self.first {
                    self.keep_line();
                }
                self.line_cut = false;
                self.line += 1;
                if self.line > self.last {
                    self.done = true;
                    return false;
                }
            } else if self.line >= self.first {
                if self.current.len() < FILE_WINDOW_LINE_BYTES {
                    self.current.push(b);
                } else {
                    self.line_cut = true;
                }
            }
        }
        true
    }
    fn finish(mut self) -> FileWindow {
        let reaches_end = !self.done;
        // A last line without a newline still counts; an empty one after it doesn't.
        if !self.done && self.line >= self.first && !self.current.is_empty() {
            self.keep_line();
        }
        FileWindow { first: self.first, lines: self.lines, size: self.size, cut: self.cut, reaches_end }
    }
}

enum Field {
    Path,
    Number,
    Text,
}

/// Reads `git grep -z -n --color=always` as it streams: `path NUL line NUL text LF`
/// with each match wrapped in [`MARK_START`] and [`MARK_END`].
struct Parser {
    prefix: Option<Vec<u8>>,
    limits: SearchLimits,
    field: Field,
    path: Vec<u8>,
    number: u64,
    digits: usize,
    line: LineScan,
    current: Option<(FileSearchFile, bool)>,
    /// Files complete, with whether every match looked like a Git LFS pointer line.
    ready: Vec<(FileSearchFile, bool)>,
    matches: usize,
    full: bool,
    error: Option<Error>,
}

impl Parser {
    fn new(prefix: Option<Vec<u8>>, limits: SearchLimits) -> Self {
        Self {
            prefix,
            limits,
            field: Field::Path,
            path: Vec::new(),
            number: 0,
            digits: 0,
            line: LineScan::default(),
            current: None,
            ready: Vec::new(),
            matches: 0,
            full: false,
            error: None,
        }
    }

    fn fail(&mut self, detail: &'static str) -> bool {
        self.error = Some(Error::parse("file-search", detail));
        false
    }

    /// False when the search has enough, or the output can't be read.
    fn feed(&mut self, chunk: &[u8]) -> bool {
        if self.full || self.error.is_some() {
            return false;
        }
        for &b in chunk {
            match self.field {
                Field::Path if b == 0 => {
                    self.field = Field::Number;
                    self.number = 0;
                    self.digits = 0;
                }
                Field::Path if self.path.len() == PATH_BYTES => return self.fail("path too long"),
                Field::Path => self.path.push(b),
                Field::Number if b == 0 && self.digits > 0 => self.field = Field::Text,
                Field::Number if b.is_ascii_digit() && self.digits < 19 => {
                    self.number = self.number * 10 + u64::from(b - b'0');
                    self.digits += 1;
                }
                Field::Number => return self.fail("line number"),
                Field::Text if b == b'\n' => {
                    if !self.end_line() {
                        return false;
                    }
                    self.field = Field::Path;
                    self.path.clear();
                }
                Field::Text => self.line.push(b),
            }
        }
        true
    }

    fn end_line(&mut self) -> bool {
        let path = match &self.prefix {
            Some(prefix) => match self.path.strip_prefix(prefix.as_slice()) {
                Some(path) => path.to_vec(),
                None => return self.fail("unexpected path"),
            },
            None => self.path.clone(),
        };
        if path.is_empty() {
            return self.fail("empty path");
        }
        let (line, pointer) = std::mem::take(&mut self.line).finish(self.number);
        if self.current.as_ref().is_none_or(|(file, _)| file.path != path) {
            if let Some(done) = self.current.take() {
                self.ready.push(done);
            }
            self.current = Some((FileSearchFile { path, ..Default::default() }, true));
        }
        let (file, pointer_like) = self.current.as_mut().unwrap();
        if file.lines.len() >= self.limits.per_file {
            file.capped = true;
            return true;
        }
        *pointer_like &= pointer;
        file.lines.push(line);
        file.capped = file.lines.len() >= self.limits.per_file;
        self.matches += 1;
        if self.matches >= self.limits.matches {
            self.full = true;
            return false;
        }
        true
    }

    /// Hand over the last file; an unfinished line is dropped.
    fn finish(&mut self) {
        if let Some(done) = self.current.take() {
            self.ready.push(done);
        }
    }
}

/// One line of text as it streams: only a window around its first match is kept.
#[derive(Default)]
struct LineScan {
    /// Text bytes seen so far, marks excluded.
    raw: usize,
    /// The first bytes, shown when Git marked nothing.
    head: Vec<u8>,
    /// The last bytes before the first match.
    before: VecDeque<u8>,
    /// Bytes from `window_start`, once the first match began.
    window: Vec<u8>,
    window_start: usize,
    started: bool,
    in_mark: bool,
    mark_start: usize,
    marks: Vec<Range<usize>>,
    escape: Vec<u8>,
}

impl LineScan {
    fn push(&mut self, b: u8) {
        if self.escape.is_empty() && b != 0x1b {
            return self.text(b);
        }
        self.escape.push(b);
        if self.escape == MARK_START {
            self.escape.clear();
            return self.open();
        }
        // Git's mark ends with the plain reset, which coloured text (saved terminal
        // output) also uses: outside a match it is text.
        if self.in_mark && self.escape == MARK_END {
            self.escape.clear();
            return self.close();
        }
        if MARK_START.starts_with(&self.escape) || (self.in_mark && MARK_END.starts_with(&self.escape)) {
            return;
        }
        // Not one of Git's marks: the bytes are text.
        let pending = std::mem::take(&mut self.escape);
        self.text(pending[0]);
        for &b in &pending[1..] {
            self.push(b);
        }
    }

    fn text(&mut self, b: u8) {
        if self.head.len() < WINDOW {
            self.head.push(b);
        }
        if self.started {
            if self.window.len() < WINDOW {
                self.window.push(b);
            }
        } else {
            self.before.push_back(b);
            if self.before.len() > BEFORE {
                self.before.pop_front();
            }
        }
        self.raw += 1;
    }

    fn open(&mut self) {
        if self.in_mark {
            return;
        }
        self.in_mark = true;
        self.mark_start = self.raw;
        if !self.started {
            self.started = true;
            self.window_start = self.raw - self.before.len();
            self.window = self.before.drain(..).collect();
        }
    }

    fn close(&mut self) {
        if self.in_mark {
            self.in_mark = false;
            if self.raw > self.mark_start && self.marks.len() < MARKS {
                self.marks.push(self.mark_start..self.raw);
            }
        }
    }

    /// The line, and whether it starts like a Git LFS pointer line.
    fn finish(mut self, number: u64) -> (FileSearchLine, bool) {
        for b in std::mem::take(&mut self.escape) {
            self.text(b);
        }
        self.close();
        let (start, mut bytes) = if self.started {
            (self.window_start, std::mem::take(&mut self.window))
        } else {
            (0, std::mem::take(&mut self.head))
        };
        let mut length = self.raw;
        // A CRLF file keeps \r before each newline; it isn't part of the line.
        if start + bytes.len() == length && bytes.last() == Some(&b'\r') {
            bytes.pop();
            length -= 1;
        }
        let end = start + bytes.len();
        // Don't show a character cut in half at either edge as a replacement mark.
        let mut lead = 0;
        if start > 0 {
            while lead < bytes.len().min(3) && bytes[lead] & 0xC0 == 0x80 {
                lead += 1;
            }
        }
        let mut tail = bytes.len();
        if end < length {
            tail = utf8_prefix(&bytes[lead..]) + lead;
        }
        let shown = start + lead..start + tail;
        let pointer = start == 0 && LFS_LINES.iter().any(|p| bytes.starts_with(p));
        let mut text = String::new();
        let mut marks = Vec::new();
        let mut chars = 0usize;
        let mut cut_end = shown.end < length;
        let mut not_utf8 = false;
        let mut at = shown.start;
        // In valid UTF-8 a mark edge inside a character (a regex "." matches one byte
        // here) moves out to the character's edge, so the text decodes whole.
        let valid = std::str::from_utf8(&bytes[shown.start - start..shown.end - start]).is_ok();
        let inside = |i: usize| i > shown.start && i < shown.end && bytes[i - start] & 0xC0 == 0x80;
        // Split the shown bytes at match edges so each piece decodes on its own.
        let mut edges: Vec<(usize, bool)> = Vec::new();
        for mark in &self.marks {
            let (mut a, mut b) = (mark.start.max(shown.start), mark.end.min(shown.end));
            if valid {
                while inside(a) {
                    a -= 1;
                }
                while inside(b) {
                    b += 1;
                }
            }
            if a < b && a >= edges.last().map_or(shown.start, |e| e.0) {
                edges.push((a, true));
                edges.push((b, false));
            }
        }
        edges.push((shown.end, false));
        let mut mark_from = None;
        for (edge, opens) in edges {
            if edge > at {
                let piece = &bytes[at - start..edge - start];
                not_utf8 |= std::str::from_utf8(piece).is_err();
                for c in String::from_utf8_lossy(piece).chars() {
                    if chars >= FILE_SEARCH_LINE_CHARS {
                        cut_end = true;
                        break;
                    }
                    if c == '\t' {
                        text.push_str("    ");
                        chars += 4;
                    } else if c.is_control() {
                        let escaped: String = c.escape_default().collect();
                        chars += escaped.chars().count();
                        text.push_str(&escaped);
                    } else {
                        text.push(c);
                        chars += 1;
                    }
                }
                at = edge;
            }
            if opens {
                mark_from = Some(chars);
            } else if let Some(from) = mark_from.take() {
                let to = chars.min(FILE_SEARCH_LINE_CHARS);
                if from < to {
                    marks.push(from..to);
                }
            }
        }
        (
            FileSearchLine {
                number,
                text,
                marks,
                columns: self.marks,
                cut_start: shown.start > 0,
                cut_end,
                not_utf8,
            },
            pointer,
        )
    }
}

/// The length of the longest prefix of `bytes` that doesn't end inside a character.
fn utf8_prefix(bytes: &[u8]) -> usize {
    let mut end = bytes.len();
    // Look back over at most three continuation bytes for the lead byte.
    for back in 1..=4.min(bytes.len()) {
        let b = bytes[bytes.len() - back];
        if b & 0xC0 == 0x80 {
            continue;
        }
        let needed = match b {
            0x00..=0x7F => 1,
            0xC0..=0xDF => 2,
            0xE0..=0xEF => 3,
            0xF0..=0xF7 => 4,
            _ => 1,
        };
        if needed > back {
            end = bytes.len() - back;
        }
        break;
    }
    end
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(path: &str, line: u64, text: &[u8]) -> Vec<u8> {
        let mut b = path.as_bytes().to_vec();
        b.push(0);
        b.extend_from_slice(line.to_string().as_bytes());
        b.push(0);
        b.extend_from_slice(text);
        b.push(b'\n');
        b
    }
    fn marked(text: &str) -> Vec<u8> {
        // [x] marks a match, as Git colours it.
        text.replace('[', "\x1b[38;2;1;2;3m").replace(']', "\x1b[m").into_bytes()
    }
    fn parse_all(stream: &[u8], chunk: usize, limits: SearchLimits, prefix: Option<&str>) -> (Vec<FileSearchFile>, Parser) {
        let mut parser = Parser::new(prefix.map(|p| p.as_bytes().to_vec()), limits);
        let mut files = Vec::new();
        for piece in stream.chunks(chunk) {
            let more = parser.feed(piece);
            files.extend(parser.ready.drain(..).map(|(f, _)| f));
            if !more {
                break;
            }
        }
        parser.finish();
        files.extend(parser.ready.drain(..).map(|(f, _)| f));
        (files, parser)
    }

    #[test]
    fn records_group_by_file_with_marks_whatever_the_chunk_size() {
        let mut stream = Vec::new();
        stream.extend(record("src/a.rs", 3, &marked("let [x] = [x];")));
        stream.extend(record("src/a.rs", 9, &marked("\t[X]\r")));
        stream.extend(record("docs/文.md", 1, &marked("é [é] e")));
        for chunk in [1, 2, 3, 7, 64, 4096] {
            let (files, parser) = parse_all(&stream, chunk, SearchLimits::default(), None);
            assert!(parser.error.is_none());
            assert_eq!(files.len(), 2, "chunk {chunk}");
            assert_eq!(files[0].path, b"src/a.rs");
            let first = &files[0].lines[0];
            assert_eq!((first.number, first.text.as_str()), (3, "let x = x;"));
            assert_eq!(first.marks, [4..5, 8..9]);
            assert_eq!(first.columns, [4..5, 8..9]);
            // Tabs show as four spaces; the CRLF's \r is dropped.
            let second = &files[0].lines[1];
            assert_eq!((second.text.as_str(), second.marks.clone()), ("    X", vec![4..5]));
            assert!(!second.cut_end);
            // Marks count characters, columns count bytes.
            let third = &files[1].lines[0];
            assert_eq!(third.text, "é é e");
            assert_eq!(third.marks, [2..3]);
            assert_eq!(third.columns, [3..5]);
            assert!(!third.not_utf8);
        }
    }

    #[test]
    fn long_lines_keep_a_bounded_window_around_the_first_match() {
        let mut text = "a".repeat(5_000).into_bytes();
        text.extend(marked("[needle]"));
        text.extend("z".repeat(2_000_000).into_bytes());
        let (files, _) = parse_all(&record("min.js", 1, &text), 8192, SearchLimits::default(), None);
        let line = &files[0].lines[0];
        assert!(line.cut_start && line.cut_end);
        assert_eq!(line.text.chars().count(), FILE_SEARCH_LINE_CHARS);
        assert_eq!(&line.text[BEFORE..BEFORE + 6], "needle");
        assert_eq!(line.marks, [BEFORE..BEFORE + 6]);
        assert_eq!(line.columns, [5_000..5_006]);
    }

    #[test]
    fn invalid_utf8_controls_and_stray_escapes_are_shown_safely() {
        let mut text = b"caf\xe9 ".to_vec();
        text.extend(marked("[RETRY]"));
        text.extend(b" \x1b[1mbold\x07");
        let (files, _) = parse_all(&record("legacy/latin1.txt", 1, &text), 5, SearchLimits::default(), None);
        let line = &files[0].lines[0];
        assert!(line.not_utf8);
        assert_eq!(line.text, "caf\u{fffd} RETRY \\u{1b}[1mbold\\u{7}");
        assert_eq!(line.marks, [5..10]);
        // A character cut at the window's edge is trimmed rather than shown broken:
        // 160 bytes before the match start in the middle of an "é".
        let mut text = "é".repeat(400).into_bytes();
        text.push(b'a');
        text.extend(marked("[x]"));
        let (files, _) = parse_all(&record("a", 1, &text), 3, SearchLimits::default(), None);
        let line = &files[0].lines[0];
        assert!(line.cut_start && !line.not_utf8);
        assert!(line.text.starts_with('é') && line.text.ends_with('x'));
    }

    #[test]
    fn colour_resets_in_text_and_mark_edges_inside_a_character_stay_correct() {
        // A saved terminal log's reset outside a match is text, so columns keep counting it.
        let mut text = b"\x1b[31mERROR\x1b[m ".to_vec();
        text.extend(marked("[retry]"));
        let (files, _) = parse_all(&record("ci.log", 1, &text), 2, SearchLimits::default(), None);
        let line = &files[0].lines[0];
        assert_eq!(line.columns, [14..19]);
        assert_eq!(line.text, "\\u{1b}[31mERROR\\u{1b}[m retry");
        assert_eq!(line.marks, [24..29]);
        // A regex "." matches one byte, so Git's mark can end inside "é": it widens to
        // the whole character and the text stays valid UTF-8.
        let mut text = b"\x1b[38;2;1;2;3mcaf\xc3\x1b[m\xa9 x".to_vec();
        text.push(b'!');
        let (files, _) = parse_all(&record("b.txt", 1, &text), 3, SearchLimits::default(), None);
        let line = &files[0].lines[0];
        assert!(!line.not_utf8, "{line:?}");
        assert_eq!(line.text, "café x!");
        assert_eq!(line.marks, [0..4]);
    }

    #[test]
    fn limits_stop_the_stream_and_keep_complete_files() {
        let mut stream = Vec::new();
        for file in 0..5 {
            for n in 1..=4 {
                stream.extend(record(&format!("f{file}"), n, &marked("[hit]")));
            }
        }
        let limits = SearchLimits { matches: 10, per_file: 3, time: FILE_SEARCH_TIME };
        let (files, parser) = parse_all(&stream, 11, limits, None);
        assert!(parser.full);
        assert_eq!(parser.matches, 10);
        assert_eq!(files.iter().map(|f| f.lines.len()).collect::<Vec<_>>(), [3, 3, 3, 1]);
        assert!(files[0].capped && !files[3].capped);
    }

    #[test]
    fn commit_paths_lose_their_revision_prefix_and_bad_records_fail() {
        let id = "a".repeat(40);
        let prefix = format!("{id}:");
        let stream = record(&format!("{id}:src/x.rs"), 2, &marked("[y]"));
        let (files, parser) = parse_all(&stream, 4, SearchLimits::default(), Some(&prefix));
        assert!(parser.error.is_none());
        assert_eq!(files[0].path, b"src/x.rs");
        let (_, parser) = parse_all(&record("other:src/x.rs", 2, b"y"), 4, SearchLimits::default(), Some(&prefix));
        assert!(parser.error.is_some());
        let mut bad = b"path\0x1\0text\n".to_vec();
        let (_, parser) = parse_all(&bad, 4, SearchLimits::default(), None);
        assert!(parser.error.is_some());
        bad = b"path\0\0text\n".to_vec();
        let (_, parser) = parse_all(&bad, 4, SearchLimits::default(), None);
        assert!(parser.error.is_some());
    }

    #[test]
    fn pointer_lines_and_regex_failures_are_recognised() {
        let (_, pointer) = LineScan { ..Default::default() }.finish(1);
        assert!(!pointer);
        let mut scan = LineScan::default();
        for b in marked("oid sha256:[4d7a]2146") {
            scan.push(b);
        }
        assert!(scan.finish(2).1);
        assert_eq!(
            regex_failure(b"fatal: invalid regex: Unmatched ( or \\(\n").as_deref(),
            Some("Unmatched ( or \\(")
        );
        assert_eq!(
            regex_failure(b"fatal: -e option, 'RETRY_(': Unmatched ( or \\(\n").as_deref(),
            Some("Unmatched ( or \\(")
        );
        assert_eq!(regex_failure(b"fatal: not a git repository\n"), None);
        let error = Error::InvalidInput(bad_pattern("Unmatched ( or \\("));
        assert_eq!(pattern_error(&error), Some("Unmatched ( or \\("));
        assert_eq!(pattern_error(&Error::InvalidInput("other".into())), None);
    }

    #[test]
    fn windows_keep_lines_around_the_match_and_cut_long_ones() {
        let text: Vec<u8> = (1..=1000).flat_map(|n| format!("line {n}\r\n").into_bytes()).collect();
        let mut window = WindowReader::new(498, 502, text.len() as u64);
        let more = text.chunks(7).all(|c| window.feed(c));
        assert!(!more);
        let window = window.finish();
        assert_eq!(window.first, 498);
        assert_eq!(window.lines, ["line 498", "line 499", "line 500", "line 501", "line 502"].map(|s| s.as_bytes().to_vec()));
        assert!(!window.reaches_end);
        let mut window = WindowReader::new(1, 10, 0);
        // A cut never splits a character: here it stops before the "é" it would halve.
        let mut long = vec![b'x'; FILE_WINDOW_LINE_BYTES - 1];
        long.extend("é".repeat(10).into_bytes());
        long.extend(b"\nlast");
        assert!(window.feed(&long));
        let window = window.finish();
        assert!(window.cut == [0] && window.reaches_end);
        assert_eq!(window.lines.len(), 2);
        assert_eq!(window.lines[0].len(), FILE_WINDOW_LINE_BYTES - 1);
        assert!(std::str::from_utf8(&window.lines[0]).is_ok());
        assert_eq!(window.lines[1], b"last");
    }

    #[test]
    fn the_time_limit_ends_a_search_without_an_error() {
        let base = crate::test_fixture_root::base();
        let root = base.join(format!(
            "rocket-time-limit-{}-{}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("a.txt"), "needle\n").unwrap();
        std::fs::write(root.join("empty.config"), "").unwrap();
        let git = crate::GitClient::discover().unwrap();
        let status = std::process::Command::new(git.executable())
            .args(["init", "-q", "--initial-branch=main"])
            .current_dir(&root)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", root.join("empty.config"))
            .status()
            .unwrap();
        assert!(status.success());
        let repo = git.open(&root, &CancellationToken::new()).unwrap();
        let query = FileSearchQuery {
            text: "needle".into(),
            regex: false,
            match_case: false,
            path: None,
            source: FileSearchSource::Checkout,
        };
        let limits = SearchLimits { time: Duration::from_millis(1), ..Default::default() };
        let summary = repo.search_files_within(&query, &CancellationToken::new(), &mut |_| {}, limits);
        let cleanup = std::fs::remove_dir_all(&root);
        assert_eq!(summary.unwrap().end, FileSearchEnd::TimedOut);
        cleanup.unwrap();
    }

    #[test]
    fn utf8_prefixes_stop_before_a_cut_character() {
        assert_eq!(utf8_prefix("aé".as_bytes()), 3);
        assert_eq!(utf8_prefix(&"aé".as_bytes()[..2]), 1);
        assert_eq!(utf8_prefix(&"a🚀".as_bytes()[..3]), 1);
        assert_eq!(utf8_prefix(b""), 0);
    }
}
