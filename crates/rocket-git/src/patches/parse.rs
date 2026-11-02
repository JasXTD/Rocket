use super::*;
use std::collections::HashSet;

pub(crate) struct FilePatch {
    pub path: RepoPath,
    pub bytes: Vec<u8>,
    pub added: bool,
    pub deleted: bool,
    pub original: Option<RepoPath>,
    pub binary: bool,
    pub mode: Option<u32>,
    pub before_mode: Option<u32>,
}

pub(crate) fn parse(bytes: &[u8], cancel: &CancellationToken) -> Result<Vec<FilePatch>> {
    if bytes.starts_with(b"From ") || bytes.split(|b| *b==b'\n').any(|l| l==b"GIT binary patch" || l.starts_with(b"rename from ") || l.starts_with(b"old mode ") || l==b"new file mode 100755") || !bytes.windows(4).any(|w| w==b"@@ -") {
        return super::extended::parse(bytes,cancel);
    }
    if bytes.is_empty() {
        return failure(PatchFailure::Empty);
    }
    if bytes.len() > PATCH_LIMIT || bytes.contains(&0) || std::str::from_utf8(bytes).is_err() {
        return failure(PatchFailure::Unsupported);
    }
    let lines: Vec<_> = bytes.split_inclusive(|b| *b == b'\n').collect();
    if lines.len() > 200_000 || !lines[0].starts_with(b"diff --git ") {
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
    let mut files = Vec::new();
    let mut seen = HashSet::new();
    for range in starts.windows(2) {
        cancel.check()?;
        let section = &lines[range[0] + 1..range[1]];
        let mut old = None;
        let mut new = None;
        let mut body_start = None;
        let (mut added, mut deleted) = (false, false);
        for (i, line) in section.iter().enumerate() {
            let line = line.strip_suffix(b"\n").unwrap_or(line);
            if line.starts_with(b"@@ ") {
                body_start = Some(i);
                break;
            }
            if let Some(v) = line.strip_prefix(b"--- ") {
                if old.is_some() {
                    return failure(PatchFailure::Format);
                }
                old = Some(header_path(v, b"a/")?);
            } else if let Some(v) = line.strip_prefix(b"+++ ") {
                if new.is_some() {
                    return failure(PatchFailure::Format);
                }
                new = Some(header_path(v, b"b/")?);
            } else if line == b"new file mode 100644" {
                added = true;
            } else if matches!(
                line,
                b"deleted file mode 100644" | b"deleted file mode 100755"
            ) {
                deleted = true;
            } else if let Some(v) = line.strip_prefix(b"index ") {
                let fields: Vec<_> = v.split(|b| *b == b' ').collect();
                if fields.len() > 2
                    || fields
                        .get(1)
                        .is_some_and(|mode| !matches!(*mode, b"100644" | b"100755"))
                {
                    return failure(PatchFailure::Unsupported);
                }
                if fields[0].is_empty()
                    || !fields[0]
                        .iter()
                        .all(|b| b.is_ascii_hexdigit() || *b == b'.')
                {
                    return failure(PatchFailure::Format);
                }
            } else {
                return failure(PatchFailure::Unsupported);
            }
        }
        let (Some(old), Some(new), Some(body_start)) = (old, new, body_start) else {
            return failure(PatchFailure::Unsupported);
        };
        if added != old.is_none() || deleted != new.is_none() || added && deleted {
            return failure(PatchFailure::Format);
        }
        let path = match (&old, &new) {
            (Some(a), Some(b)) if a == b => a.clone(),
            (None, Some(p)) | (Some(p), None) => p.clone(),
            _ => return failure(PatchFailure::Unsupported),
        };
        // Case aliases cannot be independent changes on the Windows filesystem.
        if !seen.insert(path.display_lossy().to_lowercase()) {
            return failure(PatchFailure::Format);
        }
        validate_patch_path(&path)?;
        let mut patch = format!(
            "diff --git {} {}\n",
            quote_path(b"a/", &path),
            quote_path(b"b/", &path)
        )
        .into_bytes();
        if added {
            patch.extend_from_slice(b"new file mode 100644\n");
        }
        if deleted {
            patch.extend_from_slice(b"deleted file mode 100644\n");
        }
        patch.extend_from_slice(
            format!(
                "--- {}\n+++ {}\n",
                if added {
                    "/dev/null".into()
                } else {
                    quote_path(b"a/", &path)
                },
                if deleted {
                    "/dev/null".into()
                } else {
                    quote_path(b"b/", &path)
                }
            )
            .as_bytes(),
        );
        let body = &section[body_start..];
        validate_hunks(body, cancel)?;
        for line in body {
            patch.extend_from_slice(line);
        }
        files.push(FilePatch {
            path,
            bytes: patch,
            added,
            deleted,
            original: None,
            binary: false,
            mode: None,
            before_mode: None,
        });
    }
    Ok(files)
}

pub(super) fn validate_hunks(lines: &[&[u8]], cancel: &CancellationToken) -> Result<()> {
    let (mut old, mut new) = (0usize, 0usize);
    let mut saw = false;
    for (i, line) in lines.iter().enumerate() {
        if i % 1024 == 0 {
            cancel.check()?;
        }
        if line.starts_with(b"@@ -") {
            if old != 0 || new != 0 {
                return failure(PatchFailure::Format);
            }
            let fields: Vec<_> = std::str::from_utf8(line)
                .unwrap()
                .split_whitespace()
                .collect();
            if fields.len() < 4 || fields[3] != "@@" {
                return failure(PatchFailure::Format);
            }
            let count = |s: &str, prefix: char| -> Result<usize> {
                let s = s
                    .strip_prefix(prefix)
                    .ok_or(Error::Patch(PatchFailure::Format))?;
                let (start, count) = s.split_once(',').unwrap_or((s, "1"));
                let start: usize = start
                    .parse()
                    .map_err(|_| Error::Patch(PatchFailure::Format))?;
                let count: usize = count
                    .parse()
                    .map_err(|_| Error::Patch(PatchFailure::Format))?;
                if start > 10_000_000 || count > 200_000 {
                    return failure(PatchFailure::Unsupported);
                }
                Ok(count)
            };
            old = count(fields[1], '-')?;
            new = count(fields[2], '+')?;
            saw = true;
        } else if *line == b"\\ No newline at end of file\n"
            || *line == b"\\ No newline at end of file"
        {
            if !saw || i == 0 || lines[i - 1].starts_with(b"@@") || lines[i - 1].starts_with(b"\\")
            {
                return failure(PatchFailure::Format);
            }
        } else {
            let (a, b) = match line.first() {
                Some(b' ') => (1, 1),
                Some(b'-') => (1, 0),
                Some(b'+') => (0, 1),
                _ => return failure(PatchFailure::Format),
            };
            old = old
                .checked_sub(a)
                .ok_or(Error::Patch(PatchFailure::Format))?;
            new = new
                .checked_sub(b)
                .ok_or(Error::Patch(PatchFailure::Format))?;
        }
    }
    if !saw || old != 0 || new != 0 {
        return failure(PatchFailure::Format);
    }
    Ok(())
}

pub(super) fn header_path(bytes: &[u8], prefix: &[u8]) -> Result<Option<RepoPath>> {
    let bytes = bytes.strip_suffix(b"\t").unwrap_or(bytes);
    if bytes == b"/dev/null" {
        return Ok(None);
    }
    let mut decoded = Vec::new();
    if bytes.first() == Some(&b'"') {
        if bytes.last() != Some(&b'"') || bytes.len() < 2 {
            return failure(PatchFailure::Format);
        }
        let mut i = 1;
        while i < bytes.len() - 1 {
            let c = bytes[i];
            i += 1;
            if c != b'\\' {
                if c == b'"' {
                    return failure(PatchFailure::Format);
                }
                decoded.push(c);
                continue;
            }
            if i >= bytes.len() - 1 {
                return failure(PatchFailure::Format);
            }
            let c = bytes[i];
            i += 1;
            decoded.push(match c {
                b'"' | b'\\' => c,
                b't' => b'\t',
                b'n' => b'\n',
                b'r' => b'\r',
                b'a' => 7,
                b'b' => 8,
                b'f' => 12,
                b'v' => 11,
                b'0'..=b'3' => {
                    if i + 1 >= bytes.len() - 1
                        || !bytes[i..i + 2].iter().all(|b| (b'0'..=b'7').contains(b))
                    {
                        return failure(PatchFailure::Format);
                    }
                    let value = (c - b'0') * 64 + (bytes[i] - b'0') * 8 + (bytes[i + 1] - b'0');
                    i += 2;
                    value
                }
                _ => return failure(PatchFailure::Format),
            });
        }
    } else {
        decoded.extend_from_slice(bytes);
    }
    let relative = decoded
        .strip_prefix(prefix)
        .ok_or(Error::Patch(PatchFailure::Format))?;
    let path =
        RepoPath::new(relative.to_vec()).map_err(|_| Error::Patch(PatchFailure::UnsafePath))?;
    validate_patch_path(&path)?;
    Ok(Some(path))
}
pub(crate) fn quote_path(prefix: &[u8], path: &RepoPath) -> String {
    let mut s = String::from("\"");
    for b in prefix.iter().chain(path.as_bytes()) {
        match b {
            b'"' => s.push_str("\\\""),
            b'\\' => s.push_str("\\\\"),
            32..=126 => s.push(*b as char),
            _ => s.push_str(&format!("\\{b:03o}")),
        }
    }
    s.push('"');
    s
}
fn validate_patch_path(path: &RepoPath) -> Result<()> {
    if path.as_bytes().len() > 4096 || std::str::from_utf8(path.as_bytes()).is_err() {
        return failure(PatchFailure::UnsafePath);
    }
    for part in path.as_bytes().split(|b| *b == b'/') {
        let name = String::from_utf8_lossy(part).to_lowercase();
        if name == ".git"
            || name == ".gitattributes"
            || name == ".gitmodules"
            || name.starts_with("git~")
            || name.ends_with('.')
            || name.ends_with(' ')
            || part.iter().any(|b| *b < 32)
        {
            return failure(PatchFailure::UnsafePath);
        }
    }
    Ok(())
}
