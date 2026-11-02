use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ChangeKind {
    Modified,
    Added(u32),
    Deleted(u32),
    Renamed,
}

pub(super) fn parse_hunks(
    patch: &[u8],
) -> std::result::Result<(Vec<Hunk>, ChangeKind), &'static str> {
    const UNSUPPORTED: &str = "Stage or unstage this file as a whole. Partial selection requires bounded regular text with a final newline and no executable-mode change.";
    if patch.is_empty() {
        return Err("No changes remain in this file.");
    }
    if patch.len() > REVIEW_LIMIT {
        return Err(
            "This patch is too large for partial selection. Stage or unstage the whole file.",
        );
    }
    let lines: Vec<_> = patch.split(|b| *b == b'\n').collect();
    if lines.len() > 50_000 || lines.iter().any(|line| line.len() > 8192) {
        return Err(UNSUPPORTED);
    }
    let mut hunks = Vec::<Hunk>::new();
    let mut files = 0;
    let mut regular = false;
    let mut kind = ChangeKind::Modified;
    let mut rename = (false, false);
    let mut counts = (0_usize, 0_usize);
    for (i, line) in lines.iter().enumerate() {
        if line.starts_with(b"diff --git ") {
            files += 1;
        } else if hunks.is_empty() && line.starts_with(b"index ") {
            regular |= line.ends_with(b" 100644") || line.ends_with(b" 100755");
        } else if hunks.is_empty()
            && (line.starts_with(b"new file mode ") || line.starts_with(b"deleted file mode "))
        {
            if kind != ChangeKind::Modified {
                return Err(UNSUPPORTED);
            }
            let mode = if line.ends_with(b" 100644") {
                0o100644
            } else if line.ends_with(b" 100755") {
                0o100755
            } else {
                return Err(UNSUPPORTED);
            };
            kind = if line.starts_with(b"new ") {
                ChangeKind::Added(mode)
            } else {
                ChangeKind::Deleted(mode)
            };
            regular = true;
        } else if hunks.is_empty() && line.starts_with(b"rename from ") {
            if rename.0 || !matches!(kind, ChangeKind::Modified | ChangeKind::Renamed) {
                return Err(UNSUPPORTED);
            }
            rename.0 = true;
            kind = ChangeKind::Renamed;
        } else if hunks.is_empty() && line.starts_with(b"rename to ") {
            if rename.1 || !matches!(kind, ChangeKind::Modified | ChangeKind::Renamed) {
                return Err(UNSUPPORTED);
            }
            rename.1 = true;
            kind = ChangeKind::Renamed;
        } else if hunks.is_empty() && line.starts_with(b"similarity index ") {
            // Git-generated rename metadata; the edited patch gets no similarity claim.
        } else if line.starts_with(b"@@ -") {
            if counts != (0, 0) {
                return Err(UNSUPPORTED);
            }
            let text = std::str::from_utf8(line).map_err(|_| UNSUPPORTED)?;
            let fields: Vec<_> = text.split_whitespace().collect();
            if fields.len() < 4 || fields[3] != "@@" {
                return Err(UNSUPPORTED);
            }
            let parse = |field: &str, prefix| -> Option<(usize, usize)> {
                let rest = field.strip_prefix(prefix)?;
                let (start, count) = rest.split_once(',').unwrap_or((rest, "1"));
                Some((start.parse().ok()?, count.parse().ok()?))
            };
            let (old, old_count) = parse(fields[1], '-').ok_or(UNSUPPORTED)?;
            let (new, new_count) = parse(fields[2], '+').ok_or(UNSUPPORTED)?;
            counts = (old_count, new_count);
            hunks.push(Hunk {
                old,
                new,
                lines: i + 1..i + 1,
            });
        } else if let Some(hunk) = hunks.last_mut() {
            if i == lines.len() - 1 && line.is_empty() {
                continue;
            }
            let (old, new) = match line.first() {
                Some(b' ') => (1, 1),
                Some(b'-') => (1, 0),
                Some(b'+') => (0, 1),
                _ => return Err(UNSUPPORTED),
            };
            counts.0 = counts.0.checked_sub(old).ok_or(UNSUPPORTED)?;
            counts.1 = counts.1.checked_sub(new).ok_or(UNSUPPORTED)?;
            hunk.lines.end = i + 1;
        } else if !(line.starts_with(b"--- a/")
            || line.starts_with(b"+++ b/")
            || line.starts_with(b"--- \"a/")
            || line.starts_with(b"+++ \"b/")
            || *line == b"--- /dev/null" && matches!(kind, ChangeKind::Added(_))
            || *line == b"+++ /dev/null" && matches!(kind, ChangeKind::Deleted(_)))
        {
            return Err(UNSUPPORTED);
        }
    }
    if files != 1 || !regular || hunks.is_empty() || counts != (0, 0) {
        return Err(UNSUPPORTED);
    }
    if kind == ChangeKind::Renamed && rename != (true, true) {
        return Err(UNSUPPORTED);
    }
    Ok((hunks, kind))
}

pub(super) fn selection_patch(
    review: &ChangeReview,
    selected: &BTreeSet<usize>,
) -> Result<Vec<u8>> {
    let lines: Vec<_> = review.patch.split(|b| *b == b'\n').collect();
    let mut patch = Vec::new();
    if review.kind == ChangeKind::Modified || (review.kind == ChangeKind::Renamed && !review.staged)
    {
        for line in &lines[..review.hunks[0].lines.start - 1] {
            // IDs and similarity describe the full patch, not a partial selection.
            if !line.starts_with(b"index ") && !line.starts_with(b"similarity index ") {
                patch.extend_from_slice(line);
                patch.push(b'\n');
            }
        }
    } else {
        let all = selected.len() == review.changed_lines().len();
        let added = matches!(
            (review.kind, review.staged),
            (ChangeKind::Added(_), false) | (ChangeKind::Deleted(_), true)
        );
        let deleted = all
            && matches!(
                (review.kind, review.staged),
                (ChangeKind::Added(_), true) | (ChangeKind::Deleted(_), false)
            );
        let mode = match review.kind {
            ChangeKind::Added(m) | ChangeKind::Deleted(m) => m,
            _ => 0o100644,
        };
        let a = crate::patches::parse::quote_path(b"a/", &review.path);
        let b = crate::patches::parse::quote_path(b"b/", &review.path);
        patch.extend_from_slice(format!("diff --git {a} {b}\n").as_bytes());
        if added {
            patch.extend_from_slice(format!("new file mode {mode:o}\n").as_bytes());
        }
        if deleted {
            patch.extend_from_slice(format!("deleted file mode {mode:o}\n").as_bytes());
        }
        patch.extend_from_slice(
            format!(
                "--- {}\n+++ {}\n",
                if added { "/dev/null" } else { &a },
                if deleted { "/dev/null" } else { &b }
            )
            .as_bytes(),
        );
    }
    let mut delta: i64 = 0;
    for hunk in &review.hunks {
        if !hunk.lines.clone().any(|i| selected.contains(&i)) {
            continue;
        }
        let mut body = Vec::new();
        let (mut old_count, mut new_count) = (0_i64, 0_i64);
        for i in hunk.lines.clone() {
            let line = lines[i];
            let chosen = selected.contains(&i);
            let kind = match (review.staged, line[0], chosen) {
                (_, b' ', _) | (false, b'-', false) | (true, b'+', false) => b' ',
                (false, b'+', true) | (true, b'-', true) => b'+',
                (false, b'-', true) | (true, b'+', true) => b'-',
                _ => continue,
            };
            old_count += i64::from(kind != b'+');
            new_count += i64::from(kind != b'-');
            body.push(kind);
            body.extend_from_slice(&line[1..]);
            body.push(b'\n');
        }
        let old_start = if review.staged { hunk.new } else { hunk.old } as i64;
        let new_start = old_start + delta + i64::from(old_count == 0) - i64::from(new_count == 0);
        if new_start < 0 {
            return Err(Error::InvalidInput(
                "Invalid partial patch position. Refresh the diff.".into(),
            ));
        }
        patch.extend_from_slice(
            format!("@@ -{old_start},{old_count} +{new_start},{new_count} @@\n").as_bytes(),
        );
        patch.extend(body);
        delta += new_count - old_count;
    }
    Ok(patch)
}
