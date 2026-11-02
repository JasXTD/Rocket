//! Git-generated merge plans, normalized to data-only instructions and pinned IDs.
use crate::{Error, ObjectId, RebaseFailure, Result};
use std::collections::{BTreeMap, BTreeSet};
pub(crate) const TODO_FILE: &str = "rocket-merge-todo";
fn invalid<T>() -> Result<T> {
    Err(Error::Rebase(RebaseFailure::UnsupportedSequence))
}

pub(crate) fn canonical(bytes: &[u8], ids: &[ObjectId], complete: bool) -> Result<Vec<u8>> {
    if bytes.len() > 1024 * 1024 {
        return invalid();
    }
    let text = std::str::from_utf8(bytes)
        .map_err(|_| Error::Rebase(RebaseFailure::UnsupportedSequence))?;
    let mut out = String::new();
    let mut seen = BTreeSet::new();
    let mut labels = BTreeSet::new();
    let resolve = |id: &str| -> Result<String> {
        if id.len() < 7 || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
            return invalid();
        }
        let candidates: Vec<_> = ids.iter().filter(|v| v.as_str().starts_with(id)).collect();
        if candidates.len() != 1 {
            return invalid();
        }
        Ok(candidates[0].to_string())
    };
    for line in text
        .lines()
        .map(str::trim)
        .filter(|s| !s.is_empty() && !s.starts_with('#'))
    {
        let words: Vec<_> = line.split_whitespace().take_while(|s| *s != "#").collect();
        let normalized = match words.as_slice() {
            ["pick", id, ..] => {
                let id = resolve(id)?;
                if !seen.insert(id.clone()) {
                    return invalid();
                }
                format!("pick {id}")
            }
            ["label", label] if valid_label(label) => {
                if !labels.insert(*label) {
                    return invalid();
                }
                format!("label {label}")
            }
            ["reset", label]
                if valid_label(label)
                    && (labels.contains(label)
                        || (label.len() >= 7 && label.bytes().all(|b| b.is_ascii_hexdigit()))) =>
            {
                format!("reset {label}")
            }
            ["merge", "-C", id, parents @ ..]
                if !parents.is_empty() && parents.iter().all(|p| labels.contains(p)) =>
            {
                let id = resolve(id)?;
                if !seen.insert(id.clone()) {
                    return invalid();
                }
                format!("merge -C {id} {}", parents.join(" "))
            }
            _ => return invalid(),
        };
        out.push_str(&normalized);
        out.push('\n');
        if out.lines().count() > 2000 {
            return invalid();
        }
    }
    if complete && (seen.len() != ids.len() || !labels.contains("onto")) {
        return invalid();
    }
    Ok(out.into_bytes())
}
fn valid_label(label: &str) -> bool {
    !label.is_empty()
        && label.len() <= 512
        && !label.starts_with('-')
        && !label.chars().any(|c| {
            c.is_control()
                || c.is_whitespace()
                || matches!(c, '/' | '\\' | '~' | '^' | ':' | '?' | '*' | '[')
        })
        && !label.contains("..")
        && !label.contains("@{")
}
pub(crate) fn verify(metadata: &BTreeMap<String, Vec<u8>>, ids: &[ObjectId]) -> Result<()> {
    let expected = metadata
        .get(TODO_FILE)
        .ok_or(Error::Rebase(RebaseFailure::PlanJournal))?;
    let mut actual = metadata.get("done").cloned().unwrap_or_default();
    actual.push(b'\n');
    actual.extend(
        metadata
            .get("git-rebase-todo")
            .map_or(&[][..], Vec::as_slice),
    );
    if canonical(&actual, ids, true)? != canonical(expected, ids, true)? {
        return invalid();
    }
    Ok(())
}
