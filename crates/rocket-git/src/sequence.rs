//! Reviewed multi-commit cherry-pick/revert sequences. A private journal records
//! the reviewed commits; recovery is offered only while Git's sequencer state is
//! still exactly the unapplied suffix of that list for the reviewed start.
use crate::{CommitFailure, CommitOperation, Error, ObjectId, Result, parse};
use std::{
    fs,
    io::{Read, Write},
    path::Path,
};

pub(crate) const JOURNAL: &str = "rocket-commit-sequence";
const MAGIC: &str = "ROCKET-SEQUENCE-1";
/// Commits accepted in one reviewed sequence.
pub const MAX_SEQUENCE: usize = 100;
const MAX_JOURNAL: u64 = 16 * 1024;
const MAX_TODO: u64 = 1024 * 1024;

fn refused<T>(failure: CommitFailure) -> Result<T> {
    Err(Error::CommitAction(failure))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Journal {
    pub operation: CommitOperation,
    /// HEAD when the sequence started; Git's abort returns here.
    pub original: ObjectId,
    /// The reviewed commits, in application order.
    pub commits: Vec<ObjectId>,
}
impl Journal {
    pub fn encode(&self) -> Vec<u8> {
        let mut text = format!("{MAGIC}\n{}\n{}\n", self.operation.label(), self.original);
        for id in &self.commits {
            text.push_str(&format!("{id}\n"));
        }
        text.into_bytes()
    }
    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let malformed = || Error::CommitAction(CommitFailure::SequenceJournal);
        let text = std::str::from_utf8(bytes).map_err(|_| malformed())?;
        let mut lines = text.strip_suffix('\n').ok_or_else(malformed)?.split('\n');
        if lines.next() != Some(MAGIC) {
            return Err(malformed());
        }
        let operation = match lines.next() {
            Some("cherry-pick") => CommitOperation::CherryPick,
            Some("revert") => CommitOperation::Revert,
            _ => return Err(malformed()),
        };
        let original =
            ObjectId::parse(lines.next().ok_or_else(malformed)?).map_err(|_| malformed())?;
        let commits = lines
            .map(|line| ObjectId::parse(line).map_err(|_| malformed()))
            .collect::<Result<Vec<_>>>()?;
        let unique: std::collections::HashSet<_> = commits.iter().collect();
        if !(2..=MAX_SEQUENCE).contains(&commits.len()) || unique.len() != commits.len() {
            return Err(malformed());
        }
        Ok(Self {
            operation,
            original,
            commits,
        })
    }
}

/// A bounded regular file; absent is None, links and reparse points are refused.
fn bounded(path: &Path, limit: u64) -> Result<Option<Vec<u8>>> {
    let meta = match fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(Error::io("read sequence state", error)),
        Ok(meta) => meta,
    };
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if meta.file_attributes() & 0x400 != 0 {
            return refused(CommitFailure::UnsupportedSequence);
        }
    }
    if !meta.is_file() || meta.file_type().is_symlink() {
        return refused(CommitFailure::UnsupportedSequence);
    }
    let mut bytes = Vec::new();
    fs::File::open(path)
        .map_err(|e| Error::io("read sequence state", e))?
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| Error::io("read sequence state", e))?;
    if bytes.len() as u64 > limit {
        return refused(CommitFailure::UnsupportedSequence);
    }
    Ok(Some(bytes))
}

/// The journal and its exact bytes, if one exists.
pub(crate) fn read_journal(git_dir: &Path) -> Result<Option<(Journal, Vec<u8>)>> {
    match bounded(&git_dir.join(JOURNAL), MAX_JOURNAL)? {
        Some(bytes) => Ok(Some((Journal::decode(&bytes)?, bytes))),
        None => Ok(None),
    }
}

pub(crate) fn write_journal(git_dir: &Path, journal: &Journal) -> Result<Vec<u8>> {
    let bytes = journal.encode();
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(git_dir.join(JOURNAL))
        .map_err(|e| Error::io("save sequence review", e))?;
    file.write_all(&bytes)
        .and_then(|()| file.sync_all())
        .map_err(|e| Error::io("save sequence review", e))?;
    Ok(bytes)
}

/// Remove the journal only if it is still exactly the expected review.
pub(crate) fn remove_journal(git_dir: &Path, expected: &[u8]) -> Result<()> {
    match bounded(&git_dir.join(JOURNAL), MAX_JOURNAL)? {
        None => Ok(()),
        Some(bytes) if bytes == expected => fs::remove_file(git_dir.join(JOURNAL))
            .map_err(|e| Error::io("clean sequence review", e)),
        Some(_) => refused(CommitFailure::SequenceJournal),
    }
}

/// Git's sequencer state: the starting HEAD and the commits still to apply,
/// including the current one while it is stopped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Sequencer {
    pub head: ObjectId,
    pub todo: Vec<(CommitOperation, Vec<u8>)>,
    /// Where Git expects HEAD to be. It won't move a branch that is anywhere else.
    pub abort_safety: Vec<u8>,
    /// Raw head/todo/abort-safety bytes, so any change invalidates a preview.
    pub raw: Vec<u8>,
}
pub(crate) fn read_sequencer(git_dir: &Path) -> Result<Option<Sequencer>> {
    let directory = git_dir.join("sequencer");
    match fs::symlink_metadata(&directory) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(Error::io("read sequence state", error)),
        Ok(meta) if !meta.is_dir() || meta.file_type().is_symlink() => {
            return refused(CommitFailure::UnsupportedSequence);
        }
        Ok(_) => {}
    }
    let head = bounded(&directory.join("head"), 256)?
        .ok_or(Error::CommitAction(CommitFailure::UnsupportedSequence))?;
    let todo = bounded(&directory.join("todo"), MAX_TODO)?.unwrap_or_default();
    let safety = bounded(&directory.join("abort-safety"), 256)?.unwrap_or_default();
    let mut lines = Vec::new();
    for line in todo.split(|b| *b == b'\n').map(<[u8]>::trim_ascii) {
        if line.is_empty() || line.starts_with(b"#") {
            continue;
        }
        let mut fields = line
            .split(u8::is_ascii_whitespace)
            .filter(|f| !f.is_empty());
        let operation = match fields.next() {
            Some(b"pick" | b"p") => CommitOperation::CherryPick,
            Some(b"revert") => CommitOperation::Revert,
            _ => return refused(CommitFailure::UnsupportedSequence),
        };
        let id = fields.next().unwrap_or_default();
        if id.len() < 4 || id.len() > 64 || !id.iter().all(u8::is_ascii_hexdigit) {
            return refused(CommitFailure::UnsupportedSequence);
        }
        lines.push((operation, id.to_vec()));
    }
    let mut raw = head.clone();
    raw.push(0);
    raw.extend(&todo);
    raw.push(0);
    raw.extend(&safety);
    Ok(Some(Sequencer {
        head: parse::oid(parse::line(&head), "sequence state")?,
        todo: lines,
        abort_safety: parse::line(&safety).to_vec(),
        raw,
    }))
}

/// The reviewed commits Git has not finished, if its sequencer is exactly the
/// journal's unapplied suffix for the same start and operation.
pub(crate) fn remaining(journal: &Journal, sequencer: &Sequencer) -> Option<Vec<ObjectId>> {
    if sequencer.head != journal.original || sequencer.todo.len() > journal.commits.len() {
        return None;
    }
    let start = journal.commits.len() - sequencer.todo.len();
    for ((operation, id), commit) in sequencer.todo.iter().zip(&journal.commits[start..]) {
        let unique = journal
            .commits
            .iter()
            .filter(|c| c.as_str().as_bytes().starts_with(id))
            .count()
            == 1;
        if *operation != journal.operation || !commit.as_str().as_bytes().starts_with(id) || !unique
        {
            return None;
        }
    }
    Some(journal.commits[start..].to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn id(c: char) -> ObjectId {
        ObjectId::parse(&c.to_string().repeat(40)).unwrap()
    }
    #[test]
    fn journals_round_trip_and_reject_invalid_reviews() {
        let journal = Journal {
            operation: CommitOperation::Revert,
            original: id('a'),
            commits: vec![id('b'), id('c')],
        };
        assert_eq!(Journal::decode(&journal.encode()).unwrap(), journal);
        for bad in [
            b"ROCKET-SEQUENCE-1\nrevert\n".to_vec(),
            journal.encode()[1..].to_vec(),
            String::from_utf8(journal.encode())
                .unwrap()
                .replace("revert", "amend")
                .into_bytes(),
            format!(
                "ROCKET-SEQUENCE-1\nrevert\n{}\n{}\n{}\n",
                id('a'),
                id('b'),
                id('b')
            )
            .into_bytes(),
            format!("ROCKET-SEQUENCE-1\nrevert\n{}\n{}\n", id('a'), id('b')).into_bytes(),
        ] {
            assert!(Journal::decode(&bad).is_err());
        }
    }
    #[test]
    fn only_an_exact_unapplied_suffix_matches_the_journal() {
        let journal = Journal {
            operation: CommitOperation::CherryPick,
            original: id('a'),
            commits: vec![id('b'), id('c'), id('d')],
        };
        let sequencer = |head: ObjectId, todo: &[(CommitOperation, &str)]| Sequencer {
            head,
            todo: todo
                .iter()
                .map(|(o, i)| (*o, i.as_bytes().to_vec()))
                .collect(),
            abort_safety: Vec::new(),
            raw: Vec::new(),
        };
        let pick = CommitOperation::CherryPick;
        assert_eq!(
            remaining(
                &journal,
                &sequencer(id('a'), &[(pick, "ccccccc"), (pick, "ddddddd")])
            ),
            Some(vec![id('c'), id('d')])
        );
        for wrong in [
            sequencer(id('b'), &[(pick, "ddddddd")]),
            sequencer(id('a'), &[(pick, "ddddddd"), (pick, "ccccccc")]),
            sequencer(id('a'), &[(CommitOperation::Revert, "ddddddd")]),
            sequencer(id('a'), &[(pick, "eeeeeee")]),
            sequencer(
                id('a'),
                &[
                    (pick, "bbbbbbb"),
                    (pick, "ccccccc"),
                    (pick, "ddddddd"),
                    (pick, "ddddddd"),
                ],
            ),
        ] {
            assert_eq!(remaining(&journal, &wrong), None);
        }
    }
}
