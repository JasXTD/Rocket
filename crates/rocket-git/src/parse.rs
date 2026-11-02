use crate::{model::bytes_to_os, *};
use std::path::PathBuf;

pub(crate) fn oid(bytes: &[u8], operation: &'static str) -> Result<ObjectId> {
    let text =
        std::str::from_utf8(bytes).map_err(|_| Error::parse(operation, "non-ASCII object ID"))?;
    ObjectId::parse(text).map_err(|_| Error::parse(operation, "invalid object ID"))
}

pub(crate) fn line(bytes: &[u8]) -> &[u8] {
    bytes.strip_suffix(b"\n").unwrap_or(bytes)
}

pub(crate) fn path(bytes: &[u8]) -> Result<PathBuf> {
    Ok(PathBuf::from(bytes_to_os(bytes)?))
}

fn nul<'a>(input: &mut &'a [u8], operation: &'static str) -> Result<&'a [u8]> {
    let end = input
        .iter()
        .position(|&b| b == 0)
        .ok_or_else(|| Error::parse(operation, "unterminated NUL record"))?;
    let result = &input[..end];
    *input = &input[end + 1..];
    Ok(result)
}

fn fields(record: &[u8], count: usize) -> Result<Vec<&[u8]>> {
    let values: Vec<_> = record.splitn(count, |&b| b == b' ').collect();
    if values.len() != count || values.iter().any(|value| value.is_empty()) {
        return Err(Error::parse("status", "malformed status fields"));
    }
    Ok(values)
}

fn change(value: u8) -> Result<Change> {
    match value {
        b'.' => Ok(Change::Unchanged),
        b'A' => Ok(Change::Added),
        b'M' => Ok(Change::Modified),
        b'D' => Ok(Change::Deleted),
        b'R' => Ok(Change::Renamed),
        b'C' => Ok(Change::Copied),
        b'T' => Ok(Change::TypeChanged),
        b'U' => Ok(Change::Unmerged),
        _ => Err(Error::parse("status", "unknown change code")),
    }
}

fn submodule(value: &[u8]) -> Result<Option<SubmoduleState>> {
    if value == b"N..." {
        return Ok(None);
    }
    if value.len() != 4
        || value[0] != b'S'
        || !matches!(value[1], b'.' | b'C')
        || !matches!(value[2], b'.' | b'M')
        || !matches!(value[3], b'.' | b'U')
    {
        return Err(Error::parse("status", "invalid submodule state"));
    }
    Ok(Some(SubmoduleState {
        commit_changed: value[1] == b'C',
        tracked_changes: value[2] == b'M',
        untracked_changes: value[3] == b'U',
    }))
}

pub(crate) fn status(mut input: &[u8]) -> Result<Status> {
    let mut head_name = None;
    let mut head_oid = None;
    let mut upstream = None;
    let mut ahead_behind = None;
    let mut entries = Vec::new();
    while !input.is_empty() {
        let record = nul(&mut input, "status")?;
        if let Some(header) = record.strip_prefix(b"# ") {
            if let Some(value) = header.strip_prefix(b"branch.head ") {
                head_name = Some(value.to_vec());
            } else if let Some(value) = header.strip_prefix(b"branch.oid ") {
                head_oid = Some(value.to_vec());
            } else if let Some(value) = header.strip_prefix(b"branch.upstream ") {
                upstream = Some(value.to_vec());
            } else if let Some(value) = header.strip_prefix(b"branch.ab ") {
                let values = fields(value, 2)?;
                let ahead = values[0]
                    .strip_prefix(b"+")
                    .ok_or_else(|| Error::parse("status", "missing ahead sign"))?;
                let behind = values[1]
                    .strip_prefix(b"-")
                    .ok_or_else(|| Error::parse("status", "missing behind sign"))?;
                ahead_behind = Some((number(ahead, "status")?, number(behind, "status")?));
            }
            continue; // Git explicitly allows unknown future headers.
        }
        let kind = match record.first() {
            Some(b'1') => EntryKind::Tracked,
            Some(b'2') => EntryKind::RenameOrCopy,
            Some(b'u') => EntryKind::Unmerged,
            Some(b'?') => EntryKind::Untracked,
            Some(b'!') => EntryKind::Ignored,
            _ => return Err(Error::parse("status", "unknown record type")),
        };
        if matches!(kind, EntryKind::Untracked | EntryKind::Ignored) {
            if record.get(1) != Some(&b' ') {
                return Err(Error::parse("status", "missing path separator"));
            }
            // Git marks collapsed untracked/ignored directories with a trailing
            // slash, including embedded repositories even with -uall. It is a
            // record marker, not an empty component in the repository path.
            let path = &record[2..];
            let path = path.strip_suffix(b"/").unwrap_or(path);
            entries.push(StatusEntry {
                path: RepoPath::new(path.to_vec())?,
                original_path: None,
                kind,
                index: Change::Unchanged,
                worktree: Change::Unchanged,
                submodule: None,
                similarity: None,
            });
            continue;
        }
        let count = match kind {
            EntryKind::Tracked => 9,
            EntryKind::RenameOrCopy => 10,
            _ => 11,
        };
        let values = fields(record, count)?;
        if values[1].len() != 2 {
            return Err(Error::parse("status", "invalid XY field"));
        }
        let (original_path, similarity) = if kind == EntryKind::RenameOrCopy {
            let score = values[8];
            if !matches!(score.first(), Some(b'R' | b'C')) {
                return Err(Error::parse("status", "invalid rename kind"));
            }
            let score: u8 = number(&score[1..], "status")?;
            if score > 100 {
                return Err(Error::parse("status", "invalid rename score"));
            }
            (
                Some(RepoPath::new(nul(&mut input, "status")?.to_vec())?),
                Some(score),
            )
        } else {
            (None, None)
        };
        entries.push(StatusEntry {
            path: RepoPath::new(values[count - 1].to_vec())?,
            original_path,
            kind,
            index: change(values[1][0])?,
            worktree: change(values[1][1])?,
            submodule: submodule(values[2])?,
            similarity,
        });
    }
    let name = head_name.ok_or_else(|| Error::parse("status", "missing branch.head"))?;
    let target = head_oid.ok_or_else(|| Error::parse("status", "missing branch.oid"))?;
    let head = if target == b"(initial)" {
        Head::Unborn { branch: name }
    } else if name == b"(detached)" {
        Head::Detached {
            target: oid(&target, "status")?,
        }
    } else {
        Head::Branch {
            name,
            target: oid(&target, "status")?,
        }
    };
    Ok(Status {
        head,
        upstream,
        ahead_behind,
        entries,
    })
}

pub(crate) fn branches(mut input: &[u8]) -> Result<Vec<Branch>> {
    let mut result = Vec::new();
    while !input.is_empty() {
        // Fixed fields terminated by NUL; the final LF is Git's format record separator.
        // This also permits a checked-out worktree directory containing newlines on Unix.
        let mut values = Vec::with_capacity(6);
        for _ in 0..6 {
            values.push(nul(&mut input, "branches")?);
        }
        input = input
            .strip_prefix(b"\n")
            .ok_or_else(|| Error::parse("branches", "missing record newline"))?;
        let kind = if values[0].starts_with(b"refs/heads/") {
            BranchKind::Local
        } else if values[0].starts_with(b"refs/remotes/") {
            BranchKind::Remote
        } else {
            return Err(Error::parse("branches", "unexpected ref namespace"));
        };
        if !matches!(values[2], b"*" | b" ") {
            return Err(Error::parse("branches", "invalid HEAD marker"));
        }
        result.push(Branch {
            reference: values[0].to_vec(),
            target: oid(values[1], "branches")?,
            kind,
            is_head: values[2] == b"*",
            upstream: optional(values[3]),
            symbolic_target: optional(values[4]),
            checked_out_at: if values[5].is_empty() {
                None
            } else {
                Some(path(values[5])?)
            },
        });
    }
    Ok(result)
}

fn optional(bytes: &[u8]) -> Option<Vec<u8>> {
    if bytes.is_empty() {
        None
    } else {
        Some(bytes.to_vec())
    }
}

pub(crate) fn number<T: std::str::FromStr>(bytes: &[u8], operation: &'static str) -> Result<T> {
    std::str::from_utf8(bytes)
        .ok()
        .and_then(|text| text.parse().ok())
        .ok_or_else(|| Error::parse(operation, "invalid number"))
}

pub(crate) fn commits(mut input: &[u8], expected: &[ObjectId]) -> Result<Vec<Commit>> {
    let mut commits = Vec::with_capacity(expected.len());
    for expected_id in expected {
        let end = input
            .iter()
            .position(|&b| b == b'\n')
            .ok_or_else(|| Error::parse("history", "missing batch header"))?;
        let header: Vec<_> = input[..end].split(|&b| b == b' ').collect();
        if header.len() != 3 || header[1] != b"commit" || oid(header[0], "history")? != *expected_id
        {
            return Err(Error::parse(
                "history",
                "unexpected batch object (missing or not a commit)",
            ));
        }
        let size: usize = number(header[2], "history")?;
        input = &input[end + 1..];
        if size >= input.len() || input[size] != b'\n' {
            return Err(Error::parse("history", "truncated batch object"));
        }
        commits.push(commit(expected_id.clone(), &input[..size])?);
        input = &input[size + 1..];
    }
    if !input.is_empty() {
        return Err(Error::parse("history", "unexpected trailing batch data"));
    }
    Ok(commits)
}

fn commit(id: ObjectId, raw: &[u8]) -> Result<Commit> {
    let end = raw
        .windows(2)
        .position(|value| value == b"\n\n")
        .ok_or_else(|| Error::parse("history", "missing commit header separator"))?;
    let mut tree = None;
    let mut parents = Vec::new();
    let mut author = None;
    let mut committer = None;
    let mut encoding = None;
    for header in raw[..end].split(|&b| b == b'\n') {
        if let Some(value) = header.strip_prefix(b"tree ") {
            tree = Some(oid(value, "history")?);
        } else if let Some(value) = header.strip_prefix(b"parent ") {
            // Bound allocations and every ancestry consumer. Do not silently
            // truncate the model: merge/rebase callers need complete ancestry.
            if parents.len() == 1024 {
                return Err(Error::Unsupported("Commit has more than 1024 parents; history cannot be displayed safely".into()));
            }
            parents.push(oid(value, "history")?);
        } else if let Some(value) = header.strip_prefix(b"author ") {
            author = Some(signature(value)?);
        } else if let Some(value) = header.strip_prefix(b"committer ") {
            committer = Some(signature(value)?);
        } else if let Some(value) = header.strip_prefix(b"encoding ") {
            encoding = Some(value.to_vec());
        }
        // gpgsig/mergetag and their space-prefixed continuation lines are metadata.
    }
    Ok(Commit {
        id,
        tree: tree.ok_or_else(|| Error::parse("history", "missing tree"))?,
        parents,
        author: author.ok_or_else(|| Error::parse("history", "missing author"))?,
        committer: committer.ok_or_else(|| Error::parse("history", "missing committer"))?,
        encoding,
        message: raw[end + 2..].to_vec(),
    })
}

pub(crate) fn signature(value: &[u8]) -> Result<Signature> {
    let close = value
        .iter()
        .rposition(|&b| b == b'>')
        .ok_or_else(|| Error::parse("history", "missing email end"))?;
    let open = value[..close]
        .iter()
        .rposition(|&b| b == b'<')
        .ok_or_else(|| Error::parse("history", "missing email start"))?;
    let time = value[close + 1..]
        .strip_prefix(b" ")
        .ok_or_else(|| Error::parse("history", "missing timestamp"))?;
    let values: Vec<_> = time.split(|&b| b == b' ').collect();
    if values.len() != 2 {
        return Err(Error::parse("history", "invalid signature time"));
    }
    let zone = values[1];
    if zone.len() != 5
        || !matches!(zone[0], b'+' | b'-')
        || !zone[1..].iter().all(u8::is_ascii_digit)
    {
        return Err(Error::parse("history", "invalid timezone"));
    }
    let hours: i32 = number(&zone[1..3], "history")?;
    let minutes: i32 = number(&zone[3..], "history")?;
    if hours > 23 || minutes > 59 {
        return Err(Error::parse("history", "timezone out of range"));
    }
    let name = value[..open].strip_suffix(b" ").unwrap_or(&value[..open]);
    Ok(Signature {
        name: name.to_vec(),
        email: value[open + 1..close].to_vec(),
        timestamp: number(values[0], "history")?,
        offset_minutes: (hours * 60 + minutes) * if zone[0] == b'-' { -1 } else { 1 },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_unusual_status_path_bytes_and_ignores_new_headers() {
        let status = status(
            b"# branch.oid (initial)\0# branch.head main\0# future value\0? line\n\xff.txt\0",
        )
        .unwrap();
        assert_eq!(status.entries[0].path.as_bytes(), b"line\n\xff.txt");
    }

    #[test]
    fn accepts_status_directory_markers_but_rejects_unsafe_paths() {
        let header = "# branch.head main\0# branch.oid (initial)\0";
        let parsed = status(format!("{header}? vendor/描画 🚀 é/\0! cache/\0").as_bytes()).unwrap();
        assert_eq!(
            parsed.entries[0].path.as_bytes(),
            "vendor/描画 🚀 é".as_bytes()
        );
        assert_eq!(parsed.entries[0].kind, EntryKind::Untracked);
        assert_eq!(parsed.entries[1].path.as_bytes(), b"cache");
        assert_eq!(parsed.entries[1].kind, EntryKind::Ignored);
        for marker in ['?', '!'] {
            for path in ["/", "../", "a/../", "a//", "./"] {
                assert!(status(format!("{header}{marker} {path}\0").as_bytes()).is_err());
            }
        }
        // The general API remains strict; only Git's status directory marker is
        // normalized, not arbitrary caller-supplied paths.
        assert!(RepoPath::new(b"cache/".to_vec()).is_err());
    }

    #[test]
    fn rejects_truncated_and_invalid_status_records() {
        for data in [
            b"? missing-nul".as_slice(),
            b"2 R. N... 100644 100644 100644 a b R100 new\0",
            b"1 M N... 100644 100644 100644 a b file\0",
            b"# branch.ab +999999999999999999999999 -0\0",
        ] {
            assert!(status(data).is_err());
        }
    }

    #[test]
    fn malformed_batch_lengths_and_missing_objects_are_errors() {
        let id = ObjectId::parse(&"a".repeat(40)).unwrap();
        assert!(
            commits(
                format!("{id} missing\n").as_bytes(),
                std::slice::from_ref(&id)
            )
            .is_err()
        );
        assert!(
            commits(
                format!("{id} commit 999999999999999999999999\nx\n").as_bytes(),
                std::slice::from_ref(&id)
            )
            .is_err()
        );
        assert!(commits(format!("{id} commit 10\nx\n").as_bytes(), &[id]).is_err());
    }

    #[test]
    fn excessive_commit_ancestry_is_rejected_without_truncating_the_model() {
        let id = ObjectId::parse(&"a".repeat(40)).unwrap();
        let raw = format!("tree {id}\n{}author Fixture <fixture@example.invalid> 0 +0000\ncommitter Fixture <fixture@example.invalid> 0 +0000\n\nsubject", format!("parent {id}\n").repeat(1024));
        assert_eq!(commit(id.clone(), raw.as_bytes()).unwrap().parents.len(), 1024);
        let excessive = raw.replacen("author ", &format!("parent {id}\nauthor "), 1);
        assert!(matches!(commit(id, excessive.as_bytes()), Err(Error::Unsupported(_))));
    }

    #[test]
    fn validates_paths_without_turning_display_text_into_arguments() {
        for path in [
            b"".as_slice(),
            b"../outside",
            b"/absolute",
            b"a/../b",
            b"a\0b",
        ] {
            assert!(RepoPath::new(path.to_vec()).is_err());
        }
        assert_eq!(
            RepoPath::new(b"-literal [name]".to_vec())
                .unwrap()
                .as_bytes(),
            b"-literal [name]"
        );
    }
}
