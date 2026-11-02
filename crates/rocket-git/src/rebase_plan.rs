//! Data-only interactive plans. The only commands are the reviewed `exec` rows;
//! the editor writes exactly the reviewed todo and accepts nothing else from a plan.
use crate::{Error, ObjectId, RebaseFailure, Result};
use std::{
    collections::BTreeMap,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::Command,
};

pub(crate) const PLAN_FILE: &str = "rocket-plan";
pub(crate) const JOURNAL_FILE: &str = "rocket-interactive-plan";
pub(crate) const MESSAGE_FILE: &str = "rocket-message-pending";
const MAX_PLAN: usize = 1024 * 1024;
/// One `exec` command, in bytes.
pub const MAX_REBASE_COMMAND: usize = 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RebaseInstruction {
    Pick,
    Reword,
    /// Replay the commit, then stop so it can be amended before continuing.
    Edit,
    Squash,
    Fixup,
    Drop,
}
impl RebaseInstruction {
    pub fn label(self) -> &'static str {
        match self {
            Self::Pick => "pick",
            Self::Reword => "reword",
            Self::Edit => "edit",
            Self::Squash => "squash",
            Self::Fixup => "fixup",
            Self::Drop => "drop",
        }
    }
    pub fn needs_message(self) -> bool {
        matches!(self, Self::Reword | Self::Squash)
    }
    fn parse(value: &[u8]) -> Result<Self> {
        match value {
            b"pick" | b"p" => Ok(Self::Pick),
            b"reword" | b"r" => Ok(Self::Reword),
            b"edit" | b"e" => Ok(Self::Edit),
            b"squash" | b"s" => Ok(Self::Squash),
            b"fixup" | b"f" => Ok(Self::Fixup),
            b"drop" | b"d" => Ok(Self::Drop),
            _ => unsupported(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RebasePlanEntry {
    pub commit: ObjectId,
    pub instruction: RebaseInstruction,
    /// Used only by Reword/Squash. A squash message describes the combined commit.
    pub message: String,
    /// A one-line command Git runs through its shell after this row is applied.
    pub exec: Option<String>,
    /// Stop after this row (and its command) so the result can be inspected.
    pub stop_after: bool,
}
impl RebasePlanEntry {
    /// A row with no message, command or stop.
    pub fn new(commit: ObjectId, instruction: RebaseInstruction) -> Self {
        Self {
            commit,
            instruction,
            message: String::new(),
            exec: None,
            stop_after: false,
        }
    }
}

/// One parsed line of Git's todo/done files.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum TodoLine {
    Commit(RebaseInstruction, Vec<u8>),
    Exec(Vec<u8>),
    Break,
    /// `update-ref <ref>` (`--update-refs`): the branch moves to the commit
    /// replayed just before this line when the rebase finishes.
    UpdateRef(Vec<u8>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Plan {
    pub preserve_merges: bool,
    pub original: ObjectId,
    /// None for a rebase from the root commit: Git then creates a temporary empty
    /// base when it starts and names it in both `onto` and `squash-onto`.
    pub onto: Option<ObjectId>,
    pub branch: Vec<u8>,
    pub entries: Vec<RebasePlanEntry>,
    /// `--update-refs`: each branch (full ref) and the original commit it points at.
    pub update_refs: Vec<(ObjectId, Vec<u8>)>,
}
fn unsupported<T>() -> Result<T> {
    Err(Error::Rebase(RebaseFailure::UnsupportedSequence))
}
fn invalid<T>() -> Result<T> {
    Err(Error::Rebase(RebaseFailure::InvalidPlan))
}
impl Plan {
    pub fn message_for_done(&self, done: &[u8]) -> Result<Option<&str>> {
        if self.preserve_merges { return Ok(None); }
        for line in todo_lines(done)?.iter().rev() {
            // A stop or command ends the commit being described; a branch update
            // does not, so the commit before it is still the one being described.
            let TodoLine::Commit(action, id) = line else {
                if matches!(line, TodoLine::UpdateRef(_)) {
                    continue;
                }
                break;
            };
            if *action == RebaseInstruction::Drop {
                continue;
            }
            let row = self
                .entries
                .iter()
                .find(|r| r.commit.as_str().as_bytes().starts_with(id))
                .ok_or(Error::Rebase(RebaseFailure::UnsupportedSequence))?;
            if action.needs_message() {
                return Ok(Some(&row.message));
            }
            if matches!(action, RebaseInstruction::Pick | RebaseInstruction::Edit) {
                break;
            }
        }
        Ok(None)
    }
    /// Whether any row runs a command; such plans need a longer command limit.
    pub fn runs_commands(&self) -> bool {
        self.entries.iter().any(|r| r.exec.is_some())
    }
    fn extended(entries: &[RebasePlanEntry]) -> bool {
        entries
            .iter()
            .any(|r| r.instruction == RebaseInstruction::Edit || r.exec.is_some() || r.stop_after)
    }
    pub fn validate(&self, ids: &[ObjectId]) -> Result<()> {
        Self::validate_entries(&self.entries, ids)?;
        if self.preserve_merges && (self.onto.is_none() || !self.update_refs.is_empty() || self.entries.iter().any(|r|r.instruction!=RebaseInstruction::Pick || r.exec.is_some() || r.stop_after || !r.message.is_empty())) { return invalid(); }
        let mut refs = std::collections::HashSet::new();
        for (tip, reference) in &self.update_refs {
            if !ids.contains(tip) || !refs.insert(reference) || !branch_ref(reference) {
                return invalid();
            }
        }
        Ok(())
    }
    /// Where each branch update goes: after the row of the commit it points at,
    /// or after the squash/fixup rows that fold into that row, so the branch gets
    /// the combined commit. Rows' commands and stops come first.
    fn update_ref_rows(&self) -> Vec<(usize, &[u8])> {
        let mut rows: Vec<(usize, &[u8])> = self
            .update_refs
            .iter()
            .filter_map(|(tip, reference)| {
                let mut row = self.entries.iter().position(|r| r.commit == *tip)?;
                while self.entries.get(row + 1).is_some_and(|r| {
                    matches!(
                        r.instruction,
                        RebaseInstruction::Squash | RebaseInstruction::Fixup
                    )
                }) {
                    row += 1;
                }
                Some((row, reference.as_slice()))
            })
            .collect();
        rows.sort();
        rows
    }
    pub fn validate_entries(entries: &[RebasePlanEntry], ids: &[ObjectId]) -> Result<()> {
        if ids.is_empty() || ids.len() > 200 || ids.len() != entries.len() {
            return invalid();
        }
        let mut seen = std::collections::HashSet::new();
        let mut kept = false;
        let mut size = 0;
        for (i, row) in entries.iter().enumerate() {
            if !ids.contains(&row.commit) || !seen.insert(&row.commit) {
                return invalid();
            }
            if matches!(
                row.instruction,
                RebaseInstruction::Squash | RebaseInstruction::Fixup
            ) && !kept
            {
                return invalid();
            }
            if row.instruction != RebaseInstruction::Drop {
                kept = true;
            }
            if row.instruction.needs_message()
                && (row.message.trim().is_empty()
                    || row.message.contains('\0')
                    || row.message.len() > 64 * 1024)
            {
                return invalid();
            }
            size += row.message.len();
            if row.exec.is_some() || row.stop_after {
                // Stops and commands follow a complete kept commit, never a dropped
                // row or the middle of a squash/fixup chain.
                let next = entries[i + 1..]
                    .iter()
                    .find(|r| r.instruction != RebaseInstruction::Drop);
                if row.instruction == RebaseInstruction::Drop
                    || next.is_some_and(|r| {
                        matches!(
                            r.instruction,
                            RebaseInstruction::Squash | RebaseInstruction::Fixup
                        )
                    })
                {
                    return invalid();
                }
            }
            if let Some(command) = &row.exec {
                // Git trims and splits todo lines: the command must be one exact line.
                if command.is_empty()
                    || command.len() > MAX_REBASE_COMMAND
                    || command.trim() != command
                    || command.chars().any(char::is_control)
                {
                    return invalid();
                }
                size += command.len();
            }
        }
        if size > 512 * 1024 {
            return invalid();
        }
        Ok(())
    }
    /// Version 1 is kept for plans without the newer actions, so a rebase started
    /// with only pick/reword/squash/fixup/drop stays readable by earlier builds.
    /// An empty destination marks a rebase from the root commit; earlier builds
    /// refuse those rebases before reading the plan, because of `squash-onto`.
    pub fn encode(&self) -> Vec<u8> {
        fn field(out: &mut Vec<u8>, b: &[u8]) {
            out.extend_from_slice(&(b.len() as u32).to_le_bytes());
            out.extend_from_slice(b);
        }
        // Version 3 adds branch updates after version 2's rows.
        let updates = !self.update_refs.is_empty();
        let extended = updates || Self::extended(&self.entries);
        let mut out = if self.preserve_merges {
            b"ROCKET-REBASE-4\0".to_vec()
        } else if updates {
            b"ROCKET-REBASE-3\0".to_vec()
        } else if extended {
            b"ROCKET-REBASE-2\0".to_vec()
        } else {
            b"ROCKET-REBASE-1\0".to_vec()
        };
        field(&mut out, self.original.as_str().as_bytes());
        field(
            &mut out,
            self.onto
                .as_ref()
                .map_or(b"".as_slice(), |onto| onto.as_str().as_bytes()),
        );
        field(&mut out, &self.branch);
        out.extend_from_slice(&(self.entries.len() as u32).to_le_bytes());
        for row in &self.entries {
            field(&mut out, row.instruction.label().as_bytes());
            field(&mut out, row.commit.as_str().as_bytes());
            field(&mut out, row.message.as_bytes());
            if extended {
                field(&mut out, row.exec.as_deref().unwrap_or_default().as_bytes());
                field(&mut out, if row.stop_after { b"1" } else { b"0" });
            }
        }
        if updates {
            out.extend_from_slice(&(self.update_refs.len() as u32).to_le_bytes());
            for (tip, reference) in &self.update_refs {
                field(&mut out, tip.as_str().as_bytes());
                field(&mut out, reference);
            }
        }
        out
    }
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let preserve_merges = bytes.starts_with(b"ROCKET-REBASE-4\0");
        fn field<'a>(bytes: &mut &'a [u8]) -> Result<&'a [u8]> {
            let Some((length, rest)) = bytes.split_at_checked(4) else {
                return unsupported();
            };
            let n = u32::from_le_bytes(length.try_into().unwrap()) as usize;
            let Some((value, rest)) = rest.split_at_checked(n) else {
                return unsupported();
            };
            *bytes = rest;
            Ok(value)
        }
        let (extended, updates, mut bytes) =
            if let Some(rest) = bytes.strip_prefix(b"ROCKET-REBASE-4\0") {
                (false, false, rest)
            } else if let Some(rest) = bytes.strip_prefix(b"ROCKET-REBASE-3\0") {
                (true, true, rest)
            } else if let Some(rest) = bytes.strip_prefix(b"ROCKET-REBASE-2\0") {
                (true, false, rest)
            } else if let Some(rest) = bytes.strip_prefix(b"ROCKET-REBASE-1\0") {
                (false, false, rest)
            } else {
                return unsupported();
            };
        if bytes.len() > MAX_PLAN {
            return unsupported();
        }
        let original = crate::parse::oid(field(&mut bytes)?, "rebase plan")?;
        let onto = match field(&mut bytes)? {
            b"" => None,
            onto => Some(crate::parse::oid(onto, "rebase plan")?),
        };
        let branch = field(&mut bytes)?.to_vec();
        let Some((count, rest)) = bytes.split_at_checked(4) else {
            return unsupported();
        };
        bytes = rest;
        let n = u32::from_le_bytes(count.try_into().unwrap()) as usize;
        if n == 0 || n > 200 || branch.is_empty() || branch.contains(&0) {
            return unsupported();
        }
        let text = |bytes: &[u8]| {
            String::from_utf8(bytes.to_vec())
                .map_err(|_| Error::Rebase(RebaseFailure::UnsupportedSequence))
        };
        let mut entries = Vec::new();
        for _ in 0..n {
            let instruction = RebaseInstruction::parse(field(&mut bytes)?)?;
            let commit = crate::parse::oid(field(&mut bytes)?, "rebase plan")?;
            let message = text(field(&mut bytes)?)?;
            let (exec, stop_after) = if extended {
                let command = field(&mut bytes)?;
                let stop = match field(&mut bytes)? {
                    b"0" => false,
                    b"1" => true,
                    _ => return unsupported(),
                };
                (
                    (!command.is_empty()).then(|| text(command)).transpose()?,
                    stop,
                )
            } else {
                (None, false)
            };
            entries.push(RebasePlanEntry {
                commit,
                instruction,
                message,
                exec,
                stop_after,
            });
        }
        let mut update_refs = Vec::new();
        if updates {
            let Some((count, rest)) = bytes.split_at_checked(4) else {
                return unsupported();
            };
            bytes = rest;
            let n = u32::from_le_bytes(count.try_into().unwrap()) as usize;
            if n == 0 || n > 200 {
                return unsupported();
            }
            for _ in 0..n {
                let tip = crate::parse::oid(field(&mut bytes)?, "rebase plan")?;
                update_refs.push((tip, field(&mut bytes)?.to_vec()));
            }
        }
        // Version 1 predates edit, commands and stops; version 3 adds branch updates.
        if !bytes.is_empty() || (!updates && extended != Self::extended(&entries)) {
            return unsupported();
        }
        let plan = Self {
            preserve_merges,
            original,
            onto,
            branch,
            entries,
            update_refs,
        };
        plan.validate(
            &plan
                .entries
                .iter()
                .map(|r| r.commit.clone())
                .collect::<Vec<_>>(),
        )?;
        Ok(plan)
    }
    /// The reviewed todo: each commit, then its command, then its stop.
    pub fn todo(&self) -> Vec<u8> {
        let mut todo = Vec::new();
        for line in self.expected() {
            match line {
                TodoLine::Commit(action, id) => {
                    todo.extend_from_slice(format!("{} ", action.label()).as_bytes());
                    todo.extend_from_slice(&id);
                }
                TodoLine::Exec(command) => {
                    todo.extend_from_slice(b"exec ");
                    todo.extend_from_slice(&command);
                }
                TodoLine::Break => todo.extend_from_slice(b"break"),
                TodoLine::UpdateRef(reference) => {
                    todo.extend_from_slice(b"update-ref ");
                    todo.extend_from_slice(&reference);
                }
            }
            todo.push(b'\n');
        }
        todo
    }
    /// The reviewed lines: each commit, its command, its stop, then the branch
    /// updates placed after it.
    fn expected(&self) -> Vec<TodoLine> {
        let updates = self.update_ref_rows();
        let mut lines = Vec::new();
        for (i, r) in self.entries.iter().enumerate() {
            lines.push(TodoLine::Commit(
                r.instruction,
                r.commit.as_str().as_bytes().to_vec(),
            ));
            if let Some(command) = &r.exec {
                lines.push(TodoLine::Exec(command.as_bytes().to_vec()));
            }
            if r.stop_after {
                lines.push(TodoLine::Break);
            }
            for (_, reference) in updates.iter().filter(|(row, _)| *row == i) {
                lines.push(TodoLine::UpdateRef(reference.to_vec()));
            }
        }
        lines
    }
    /// Git moves consumed lines from the todo to done; together they must still
    /// be exactly the reviewed plan, with commit IDs possibly abbreviated.
    pub fn verify_sequence(&self, metadata: &BTreeMap<String, Vec<u8>>) -> Result<()> {
        if self.preserve_merges { return crate::rebase_merges::verify(metadata, &self.entries.iter().map(|r|r.commit.clone()).collect::<Vec<_>>()); }
        let mut rows = Vec::new();
        for name in ["done", "git-rebase-todo"] {
            if let Some(bytes) = metadata.get(name) {
                rows.extend(todo_lines(bytes)?);
            }
        }
        let expected = self.expected();
        if rows.len() != expected.len() {
            return unsupported();
        }
        for (row, want) in rows.iter().zip(&expected) {
            let same = match (row, want) {
                (TodoLine::Commit(action, id), TodoLine::Commit(planned, full)) => {
                    action == planned
                        && full.starts_with(id)
                        && self
                            .entries
                            .iter()
                            .filter(|r| r.commit.as_str().as_bytes().starts_with(id))
                            .count()
                            == 1
                }
                (TodoLine::Exec(a), TodoLine::Exec(b)) => a == b,
                (TodoLine::Break, TodoLine::Break) => true,
                (TodoLine::UpdateRef(a), TodoLine::UpdateRef(b)) => a == b,
                _ => false,
            };
            if !same {
                return unsupported();
            }
        }
        Ok(())
    }
    /// The full ID of the plan row an abbreviated todo ID names, if exactly one.
    pub fn commit_for(&self, id: &[u8]) -> Option<&ObjectId> {
        let mut rows = self
            .entries
            .iter()
            .filter(|r| r.commit.as_str().as_bytes().starts_with(id));
        let first = rows.next()?;
        rows.next().is_none().then_some(&first.commit)
    }
}
/// A local branch ref Git's `update-ref` todo command may name: `refs/heads/`
/// and a name without spaces or control characters.
pub(crate) fn branch_ref(reference: &[u8]) -> bool {
    reference.len() > 11
        && reference.len() <= 1024
        && reference.starts_with(b"refs/heads/")
        && !reference
            .iter()
            .any(|b| b.is_ascii_whitespace() || b.is_ascii_control())
}
/// Parse todo/done lines. Commit lines keep their (possibly abbreviated) ID;
/// `exec` keeps its exact command text; `break` takes no argument; `update-ref`
/// names a local branch.
pub(crate) fn todo_lines(bytes: &[u8]) -> Result<Vec<TodoLine>> {
    let mut result = Vec::new();
    for line in bytes.split(|b| *b == b'\n').map(<[u8]>::trim_ascii) {
        if line.is_empty() || line.starts_with(b"#") {
            continue;
        }
        let end = line
            .iter()
            .position(u8::is_ascii_whitespace)
            .unwrap_or(line.len());
        let (word, rest) = (&line[..end], line[end..].trim_ascii_start());
        match word {
            b"exec" | b"x" if !rest.is_empty() => result.push(TodoLine::Exec(rest.to_vec())),
            b"break" | b"b" if rest.is_empty() => result.push(TodoLine::Break),
            b"update-ref" | b"u" if branch_ref(rest) => {
                result.push(TodoLine::UpdateRef(rest.to_vec()))
            }
            b"exec" | b"x" | b"break" | b"b" | b"update-ref" | b"u" => return unsupported(),
            _ => {
                let action = RebaseInstruction::parse(word)?;
                let id = rest
                    .split(u8::is_ascii_whitespace)
                    .next()
                    .unwrap_or_default();
                if id.len() < 7 || id.len() > 64 || !id.iter().all(u8::is_ascii_hexdigit) {
                    return unsupported();
                }
                result.push(TodoLine::Commit(action, id.to_vec()));
            }
        }
    }
    Ok(result)
}

pub(crate) fn regular(path: &Path, directory: bool) -> Result<()> {
    let m = fs::symlink_metadata(path).map_err(|e| Error::io("rebase plan file", e))?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if m.file_attributes() & 0x400 != 0 {
            return unsupported();
        }
    }
    if m.file_type().is_symlink() || (directory && !m.is_dir()) || (!directory && !m.is_file()) {
        return unsupported();
    }
    Ok(())
}
pub(crate) fn read(path: &Path) -> Result<Vec<u8>> {
    regular(path, false)?;
    let mut bytes = Vec::new();
    fs::File::open(path)
        .map_err(|e| Error::io("read rebase plan", e))?
        .take(MAX_PLAN as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| Error::io("read rebase plan", e))?;
    if bytes.len() > MAX_PLAN {
        return unsupported();
    }
    Ok(bytes)
}
fn write_new(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| Error::io("save rebase plan", e))?;
    f.write_all(bytes)
        .and_then(|()| f.sync_all())
        .map_err(|e| Error::io("save rebase plan", e))
}
fn replace(path: &Path, bytes: &[u8]) -> Result<()> {
    regular(path, false)?;
    // Publish complete data and replace the directory entry rather than truncating
    // an existing hard link. The caller has already checked the exact Git-owned path.
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let temporary = path.with_file_name(format!(".rocket-editor-{}-{stamp}", std::process::id()));
    write_new(&temporary, bytes)?;
    let result = regular(path, false).and_then(|()| {
        fs::rename(&temporary, path).map_err(|e| Error::io("publish rebase editor data", e))
    });
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}

pub(crate) struct Editor {
    git_dir: PathBuf,
    helper: PathBuf,
    source: Option<PathBuf>,
    journal: Vec<u8>,
}
impl Editor {
    pub fn cleanup(&self) -> Result<()> {
        if self.git_dir.join("rebase-merge").exists() || self.git_dir.join("rebase-apply").exists()
        {
            return Ok(());
        }
        let path = self.git_dir.join(JOURNAL_FILE);
        if !path
            .try_exists()
            .map_err(|e| Error::io("rebase journal cleanup", e))?
        {
            return Ok(());
        }
        if read(&path)? != self.journal {
            return Err(Error::Rebase(RebaseFailure::PlanJournal));
        }
        let plan=Plan::decode(&self.journal)?;
        let topology=self.git_dir.join(crate::rebase_merges::TODO_FILE);
        if plan.preserve_merges && topology.try_exists().map_err(|e|Error::io("rebase topology cleanup",e))? {
            crate::rebase_merges::canonical(&read(&topology)?, &plan.entries.iter().map(|r|r.commit.clone()).collect::<Vec<_>>(),true)?;
            fs::remove_file(topology).map_err(|e|Error::io("rebase topology cleanup",e))?;
        }
        fs::remove_file(path).map_err(|e| Error::io("rebase journal cleanup", e))
    }
    pub fn new(git_dir: &Path, plan: Option<&Plan>) -> Result<Self> {
        let exe = std::env::current_exe().map_err(|e| Error::io("locate rebase editor", e))?;
        let directory = exe
            .parent()
            .ok_or(Error::Rebase(RebaseFailure::EditorUnavailable))?;
        let name = format!("rocket-rebase-editor{}", std::env::consts::EXE_SUFFIX);
        let mut helper = directory.join(&name);
        if !helper.is_file()
            && matches!(
                directory.file_name().and_then(|s| s.to_str()),
                Some("deps" | "examples")
            )
        {
            helper = directory.parent().unwrap_or(directory).join(&name);
        }
        if !helper.is_file() {
            return Err(Error::Rebase(RebaseFailure::EditorUnavailable));
        }
        regular(&helper, false)?;
        regular(git_dir, true)?;
        let journal = if let Some(plan) = plan {
            plan.encode()
        } else {
            read(&git_dir.join(JOURNAL_FILE))?
        };
        let source = if plan.is_some() {
            if plan.is_some_and(|p|p.preserve_merges) && git_dir.join(crate::rebase_merges::TODO_FILE).try_exists().map_err(|e|Error::io("rebase topology",e))? { return Err(Error::Rebase(RebaseFailure::PlanJournal)); }
            let path = git_dir.join(JOURNAL_FILE);
            if path
                .try_exists()
                .map_err(|e| Error::io("rebase journal", e))?
            {
                return Err(Error::Rebase(RebaseFailure::PlanJournal));
            }
            write_new(&path, &journal)?;
            Some(path)
        } else {
            None
        };
        Ok(Self {
            git_dir: git_dir.into(),
            helper,
            source,
            journal,
        })
    }
    pub fn configure(&self, command: &mut Command) -> Result<()> {
        // Git invokes editors through its shell. Quote only our installed helper;
        // repository paths and user messages are passed as environment/data files.
        let path = self
            .helper
            .to_str()
            .ok_or(Error::Rebase(RebaseFailure::EditorUnavailable))?;
        let path = path
            .strip_prefix("\\\\?\\")
            .unwrap_or(path)
            .replace('\\', "/");
        let quoted = format!("'{}'", path.replace('\'', "'\\''"));
        command
            .env("GIT_SEQUENCE_EDITOR", format!("{quoted} sequence"))
            .env("GIT_EDITOR", format!("{quoted} message"))
            .env("ROCKET_REBASE_GIT_DIR", &self.git_dir);
        command.env_remove("ROCKET_REBASE_PLAN_SOURCE");
        if let Some(source) = &self.source {
            command.env("ROCKET_REBASE_PLAN_SOURCE", source);
        }
        Ok(())
    }
}
impl Drop for Editor {
    fn drop(&mut self) {
        // Retain the reviewed plan across cancellation/crash, including the interval
        // before Git has invoked the sequence editor. Never delete externally changed data.
        let _ = self.cleanup();
    }
}

/// Private application helper entry point, exported for the bundled editor binary.
/// It edits only the exact Git-owned todo/message paths inside the supplied Git directory.
#[doc(hidden)]
pub fn run_rebase_editor() -> Result<()> {
    let mut args = std::env::args_os().skip(1);
    let mode = args
        .next()
        .ok_or(Error::Rebase(RebaseFailure::UnsupportedSequence))?;
    let file = PathBuf::from(
        args.next()
            .ok_or(Error::Rebase(RebaseFailure::UnsupportedSequence))?,
    );
    if args.next().is_some() {
        return unsupported();
    }
    let git = PathBuf::from(
        std::env::var_os("ROCKET_REBASE_GIT_DIR")
            .ok_or(Error::Rebase(RebaseFailure::UnsupportedSequence))?,
    );
    regular(&git, true)?;
    let git = fs::canonicalize(git).map_err(|e| Error::io("rebase editor directory", e))?;
    let directory = git.join("rebase-merge");
    regular(&directory, true)?;
    regular(&file, false)?;
    let file = fs::canonicalize(file).map_err(|e| Error::io("rebase editor target", e))?;
    if mode == "sequence" {
        if file != directory.join("git-rebase-todo") {
            return unsupported();
        }
        let source = PathBuf::from(
            std::env::var_os("ROCKET_REBASE_PLAN_SOURCE")
                .ok_or(Error::Rebase(RebaseFailure::UnsupportedSequence))?,
        );
        regular(&source, false)?;
        let source = fs::canonicalize(source).map_err(|e| Error::io("rebase editor source", e))?;
        if source != git.join(JOURNAL_FILE) {
            return unsupported();
        }
        let bytes = read(&source)?;
        let plan = Plan::decode(&bytes)?;
        if plan.preserve_merges {
            if read(&directory.join("orig-head"))?.trim_ascii()!=plan.original.as_str().as_bytes()
                || read(&directory.join("onto"))?.trim_ascii()!=plan.onto.as_ref().ok_or(Error::Rebase(RebaseFailure::InvalidPlan))?.as_str().as_bytes()
                || read(&directory.join("head-name"))?.trim_ascii()!=[b"refs/heads/".as_slice(),plan.branch.as_slice()].concat() { return unsupported(); }
            let todo=crate::rebase_merges::canonical(&read(&file)?, &plan.entries.iter().map(|r|r.commit.clone()).collect::<Vec<_>>(),true)?;
            write_new(&git.join(crate::rebase_merges::TODO_FILE),&todo)?;
            write_new(&directory.join(PLAN_FILE),&bytes)?;
            return replace(&file,&todo);
        }
        // Git's own proposal is one pick per replayed commit and, with
        // `--update-refs`, an update for exactly the branches the plan moves.
        let mut original = Vec::new();
        let mut proposed = std::collections::BTreeSet::new();
        for line in todo_lines(&read(&file)?)? {
            match line {
                TodoLine::Commit(RebaseInstruction::Pick, id) => original.push(id),
                TodoLine::UpdateRef(reference) => {
                    proposed.insert(reference);
                }
                _ => return unsupported(),
            }
        }
        if proposed
            != plan
                .update_refs
                .iter()
                .map(|(_, reference)| reference.clone())
                .collect::<std::collections::BTreeSet<_>>()
        {
            return unsupported();
        }
        if original.len() != plan.entries.len() {
            return unsupported();
        }
        let ids = plan
            .entries
            .iter()
            .map(|r| r.commit.clone())
            .collect::<Vec<_>>();
        let mut seen = std::collections::HashSet::new();
        for abbreviated in original {
            let matches = ids
                .iter()
                .filter(|id| id.as_str().as_bytes().starts_with(&abbreviated))
                .collect::<Vec<_>>();
            if matches.len() != 1 || !seen.insert(matches[0]) {
                return unsupported();
            }
        }
        // A root rebase replays onto the empty commit Git just created for it.
        let onto = match &plan.onto {
            Some(onto) => onto.as_str().as_bytes().to_vec(),
            None => read(&directory.join("squash-onto"))?.trim_ascii().to_vec(),
        };
        if read(&directory.join("orig-head"))?.trim_ascii() != plan.original.as_str().as_bytes()
            || read(&directory.join("onto"))?.trim_ascii() != onto
            || read(&directory.join("head-name"))?.trim_ascii()
                != [b"refs/heads/".as_slice(), plan.branch.as_slice()].concat()
        {
            return unsupported();
        }
        write_new(&directory.join(PLAN_FILE), &bytes)?;
        replace(&file, &plan.todo())
    } else if mode == "message" {
        if file != git.join("COMMIT_EDITMSG") {
            return unsupported();
        }
        let bytes = read(&directory.join(PLAN_FILE))?;
        if read(&git.join(JOURNAL_FILE))? != bytes {
            return unsupported();
        }
        let plan = Plan::decode(&bytes)?;
        if plan.preserve_merges {
            let mut metadata=BTreeMap::new();
            for name in ["done","git-rebase-todo"] { metadata.insert(name.into(),read(&directory.join(name))?); }
            metadata.insert(crate::rebase_merges::TODO_FILE.into(),read(&git.join(crate::rebase_merges::TODO_FILE))?);
            return plan.verify_sequence(&metadata);
        }
        let mut meta = BTreeMap::new();
        for name in ["done", "git-rebase-todo"] {
            meta.insert(name.into(), read(&directory.join(name))?);
        }
        plan.verify_sequence(&meta)?;
        if let Some(message) = plan.message_for_done(&meta["done"])? {
            // A rejected commit-msg hook can leave a clean index after Git has
            // replayed the changes but before the reviewed message was saved.
            // Pin the pre-amend HEAD so Continue never amends a different commit.
            // message_for_done only returns a message when done ends with a commit.
            let last = match todo_lines(&meta["done"])?.pop() {
                Some(TodoLine::Commit(_, id)) => id,
                _ => return unsupported(),
            };
            let row = plan
                .entries
                .iter()
                .find(|r| r.commit.as_str().as_bytes().starts_with(&last))
                .ok_or(Error::Rebase(RebaseFailure::UnsupportedSequence))?;
            let head =
                crate::parse::oid(read(&git.join("HEAD"))?.trim_ascii(), "rebase message head")?;
            let marker = directory.join(MESSAGE_FILE);
            let content = format!("{}\n{head}\n", row.commit);
            if marker
                .try_exists()
                .map_err(|e| Error::io("rebase message marker", e))?
            {
                replace(&marker, content.as_bytes())?;
            } else {
                write_new(&marker, content.as_bytes())?;
            }
            return replace(&file, message.as_bytes());
        }
        Ok(())
    } else {
        unsupported()
    }
}
