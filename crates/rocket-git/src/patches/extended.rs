//! Canonicalize extended Git headers so the sandbox never sees unvalidated paths.
use super::*;
use parse::{FilePatch, header_path, quote_path, validate_hunks};
use std::collections::HashSet;
fn path(b: &[u8], prefix: &[u8]) -> Result<RepoPath> {
    header_path(b, prefix)?.ok_or(Error::Patch(PatchFailure::Format))
}
fn mode(b: &[u8]) -> Result<u32> {
    match b {
        b"100644" => Ok(0o100644),
        b"100755" => Ok(0o100755),
        _ => failure(PatchFailure::Unsupported),
    }
}
fn pair(header: &[u8]) -> Result<(RepoPath, RepoPath)> {
    // Git quotes unusual names; spaces in unquoted names require one unambiguous b/ separator.
    let mut candidates = Vec::new();
    for (i, b) in header.iter().enumerate() {
        if *b == b' '
            && (header[i + 1..].starts_with(b"b/") || header[i + 1..].starts_with(b"\"b/"))
            && let (Ok(a), Ok(b)) = (path(&header[..i], b"a/"), path(&header[i + 1..], b"b/"))
        {
            candidates.push((a, b));
        }
    }
    if candidates.len() != 1 {
        return failure(PatchFailure::Format);
    }
    Ok(candidates.remove(0))
}
pub(super) fn parse(bytes: &[u8], c: &CancellationToken) -> Result<Vec<FilePatch>> {
    if bytes.is_empty() {
        return failure(PatchFailure::Empty);
    }
    if bytes.len() > PATCH_LIMIT || bytes.contains(&0) || std::str::from_utf8(bytes).is_err() {
        return failure(PatchFailure::Unsupported);
    }
    let mut lines: Vec<_> = bytes.split_inclusive(|b| *b == b'\n').collect();
    if lines.len() > 200_000 {
        return failure(PatchFailure::Unsupported);
    }
    if bytes.starts_with(b"From ") {
        let first = lines[0].split(|b| *b == b' ').nth(1).unwrap_or_default();
        if !matches!(first.len(), 40 | 64) || !first.iter().all(u8::is_ascii_hexdigit) {
            return failure(PatchFailure::Format);
        }
        let start = lines
            .iter()
            .position(|l| l.starts_with(b"diff --git "))
            .ok_or(Error::Patch(PatchFailure::Format))?;
        if lines[start..].iter().any(|l| l.starts_with(b"From ")) {
            return failure(PatchFailure::Unsupported);
        }
        lines = lines[start..].to_vec();
        if let Some(end) = lines.iter().rposition(|l| *l == b"-- \n") {
            lines.truncate(end);
        }
    }
    if !lines[0].starts_with(b"diff --git ") {
        return failure(PatchFailure::Format);
    }
    let starts: Vec<_> = lines
        .iter()
        .enumerate()
        .filter_map(|(i, l)| l.starts_with(b"diff --git ").then_some(i))
        .chain(Some(lines.len()))
        .collect();
    if starts.len() > 101 {
        return failure(PatchFailure::Unsupported);
    }
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    let mut expanded_binary = 0usize;
    for range in starts.windows(2) {
        c.check()?;
        let section = &lines[range[0]..range[1]];
        let header = section[0].strip_suffix(b"\n").unwrap_or(section[0]);
        let (old, new) = pair(&header[b"diff --git ".len()..])?;
        let (mut added, mut deleted, mut binary) = (false, false, false);
        let (mut from, mut to, mut old_mode, mut new_mode) = (None, None, None, None);
        let mut index = None;
        let mut body = section.len();
        let mut text_old = None;
        let mut text_new = None;
        for (i, line) in section.iter().enumerate().skip(1) {
            let l = line.strip_suffix(b"\n").unwrap_or(line);
            if l == b"GIT binary patch" {
                binary = true;
                body = i;
                break;
            }
            if l.starts_with(b"@@ ") {
                body = i;
                break;
            }
            if let Some(v) = l.strip_prefix(b"new file mode ") {
                added = true;
                new_mode = Some(mode(v)?);
            } else if let Some(v) = l.strip_prefix(b"deleted file mode ") {
                deleted = true;
                old_mode = Some(mode(v)?);
            } else if let Some(v) = l.strip_prefix(b"old mode ") {
                old_mode = Some(mode(v)?);
            } else if let Some(v) = l.strip_prefix(b"new mode ") {
                new_mode = Some(mode(v)?);
            } else if let Some(v) = l.strip_prefix(b"rename from ") {
                from = Some(path(v, b"")?);
            } else if let Some(v) = l.strip_prefix(b"rename to ") {
                to = Some(path(v, b"")?);
            } else if let Some(v) = l.strip_prefix(b"similarity index ") {
                if !v
                    .strip_suffix(b"%")
                    .is_some_and(|v| !v.is_empty() && v.iter().all(u8::is_ascii_digit))
                {
                    return failure(PatchFailure::Format);
                }
            } else if let Some(v) = l.strip_prefix(b"index ") {
                if !v
                    .iter()
                    .all(|b| b.is_ascii_hexdigit() || matches!(b, b'.' | b' '))
                {
                    return failure(PatchFailure::Format);
                }
                index = Some(l);
            } else if let Some(v) = l.strip_prefix(b"--- ") {
                text_old = Some(header_path(v, b"a/")?);
            } else if let Some(v) = l.strip_prefix(b"+++ ") {
                text_new = Some(header_path(v, b"b/")?);
            } else {
                return failure(PatchFailure::Unsupported);
            }
        }
        if added && deleted
            || (old != new && (from.as_ref() != Some(&old) || to.as_ref() != Some(&new)))
            || (old == new && (from.is_some() || to.is_some()))
        {
            return failure(PatchFailure::Format);
        }
        let rename = old != new;
        if rename && (added || deleted) {
            return failure(PatchFailure::Format);
        }
        for p in if rename { vec![&old, &new] } else { vec![&new] } {
            if !seen.insert(p.display_lossy().to_lowercase()) {
                return failure(PatchFailure::Format);
            }
        }
        let mut canonical = format!(
            "diff --git {} {}\n",
            quote_path(b"a/", &old),
            quote_path(b"b/", &new)
        );
        if added {
            canonical.push_str(&format!("new file mode {:o}\n", new_mode.unwrap()));
        }
        if deleted {
            canonical.push_str(&format!("deleted file mode {:o}\n", old_mode.unwrap()));
        }
        if !added && !deleted && old_mode != new_mode {
            let (Some(a), Some(b)) = (old_mode, new_mode) else {
                return failure(PatchFailure::Format);
            };
            canonical.push_str(&format!("old mode {a:o}\nnew mode {b:o}\n"));
        }
        if rename {
            canonical.push_str(&format!(
                "rename from {}\nrename to {}\n",
                quote_path(b"", &old),
                quote_path(b"", &new)
            ));
        }
        if let Some(index) = index {
            canonical.push_str(std::str::from_utf8(index).unwrap());
            canonical.push('\n');
        }
        if body < section.len() && !binary {
            if text_old != Some(if added { None } else { Some(old.clone()) })
                || text_new != Some(if deleted { None } else { Some(new.clone()) })
            {
                return failure(PatchFailure::Format);
            }
            validate_hunks(&section[body..], c)?;
            canonical.push_str(&format!(
                "--- {}\n+++ {}\n",
                if added {
                    "/dev/null".into()
                } else {
                    quote_path(b"a/", &old)
                },
                if deleted {
                    "/dev/null".into()
                } else {
                    quote_path(b"b/", &new)
                }
            ));
        } else if !binary && !added && !deleted && !rename && old_mode == new_mode {
            return failure(PatchFailure::Empty);
        }
        let mut patch = canonical.into_bytes();
        if binary {
            // Literal sizes are available without inflating untrusted data. Delta
            // headers only bound instruction bytes, not the allocated result;
            // refuse them until a bounded decoder can inspect their target size.
            let mut blocks=0;
            for line in &section[body+1..] {
                let l=line.strip_suffix(b"\n").unwrap_or(line);
                if l.starts_with(b"delta "){return Err(Error::Unsupported("Binary delta patches need Git: Rocket currently accepts size-bounded literal binary patches only.".into()));}
                if let Some(size)=l.strip_prefix(b"literal ") {
                    let size=std::str::from_utf8(size).ok().and_then(|s|s.parse::<usize>().ok()).ok_or(Error::Patch(PatchFailure::Format))?;
                    if size>PATCH_LIMIT{return failure(PatchFailure::Unsupported);}
                    expanded_binary=expanded_binary.checked_add(size).ok_or(Error::Patch(PatchFailure::Unsupported))?;
                    if expanded_binary>TOTAL_LIMIT{return failure(PatchFailure::Unsupported);}blocks+=1;
                }
            }
            if !(1..=2).contains(&blocks){return failure(PatchFailure::Format);}
        }
        for l in &section[body..] {
            patch.extend_from_slice(l);
        }
        out.push(FilePatch {
            path: new,
            original: rename.then_some(old),
            bytes: patch,
            added,
            deleted,
            binary,
            before_mode: if !added&&!deleted&&new_mode.is_some(){old_mode}else{None},
            mode: if !deleted && (old_mode != new_mode || new_mode == Some(0o100755)) {
                new_mode
            } else {
                None
            },
        });
    }
    Ok(out)
}
