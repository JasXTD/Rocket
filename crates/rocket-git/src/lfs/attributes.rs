//! Byte-preserving root attribute edits. No filters, staging, or file touching.
use super::*;
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    ops::Range,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};
const LIMIT: usize = 1024 * 1024;
static NEXT: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LfsTrackingRule {
    pub line: usize,
    /// Exact source line, without its line ending. Required to untrack a listed rule.
    pub text: String,
}
#[derive(Clone)]
pub struct LfsAttributeEdit {
    before: Option<Vec<u8>>,
    after: Vec<u8>,
    modified: Option<SystemTime>,
    removed: String,
    added: String,
}
impl LfsAttributeEdit {
    pub fn before(&self) -> Option<&[u8]> {
        self.before.as_deref()
    }
    pub fn after(&self) -> &[u8] {
        &self.after
    }
    pub fn removed(&self) -> &str {
        &self.removed
    }
    pub fn added(&self) -> &str {
        &self.added
    }
}
fn path(repo: &Repository) -> PathBuf {
    repo.cwd().join(".gitattributes")
}
fn read(file: &mut File) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    file.take((LIMIT + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|e| Error::io("read LFS attributes", e))?;
    if bytes.len() > LIMIT || std::str::from_utf8(&bytes).is_err() || bytes.contains(&0) {
        return fail(LfsFailure::Attributes);
    }
    Ok(bytes)
}
fn snapshot(repo: &Repository) -> Result<(Option<Vec<u8>>, Option<SystemTime>)> {
    let _guard = local_files::pin_directories(repo.cwd(), repo.cwd())?;
    let Some(mut f) = disk::open(&path(repo), false)? else {
        return Ok((None, None));
    };
    crate::review::verify_preview_handle(&f, repo.cwd(), &path(repo))?;
    let meta = f
        .metadata()
        .map_err(|e| Error::io("inspect LFS attributes", e))?;
    if meta.permissions().readonly() {
        return fail(LfsFailure::ReadOnly);
    }
    Ok((
        Some(read(&mut f)?),
        Some(
            meta.modified()
                .map_err(|e| Error::io("inspect attributes modification time", e))?,
        ),
    ))
}
fn index_check(context: &Context, present: bool) -> Result<()> {
    for record in context.index.split(|b| *b == 0) {
        if let Some((meta, name)) = record.split_once_byte(b'\t')
            && name.eq_ignore_ascii_case(b".gitattributes")
            && (!present
                || name != b".gitattributes"
                || !(meta.starts_with(b"100644 ") || meta.starts_with(b"100755 "))
                || !meta.ends_with(b" 0"))
        {
            return fail(LfsFailure::Attributes);
        }
    }
    if context
        .flags
        .split(|b| *b == 0)
        .any(|r| r.len() > 2 && r[2..].eq_ignore_ascii_case(b".gitattributes") && r[0] != b'H')
    {
        return fail(LfsFailure::Attributes);
    }
    Ok(())
}
// Only the first token has Git's C-style quoting semantics. Attribute tokens
// retain their exact source spelling and spacing; no whole-file serialization.
fn tokens(line: &str) -> Result<Vec<Range<usize>>> {
    let b = line.as_bytes();
    let mut pos = 0;
    while pos < b.len() && matches!(b[pos], b' ' | b'\t') {
        pos += 1;
    }
    if pos == b.len() || b[pos] == b'#' {
        return Ok(Vec::new());
    }
    let start = pos;
    if b[pos] == b'"' {
        pos += 1;
        let mut closed = false;
        while pos < b.len() {
            if b[pos] == b'\\' {
                pos += 2;
                continue;
            }
            if b[pos] == b'"' {
                pos += 1;
                closed = true;
                break;
            }
            pos += 1;
        }
        if !closed || (pos < b.len() && !matches!(b[pos], b' ' | b'\t')) {
            return fail(LfsFailure::Attributes);
        }
    } else {
        while pos < b.len() && !matches!(b[pos], b' ' | b'\t') {
            pos += 1;
        }
    }
    let mut out: Vec<_> = std::iter::once(start..pos).collect();
    while pos < b.len() {
        while pos < b.len() && matches!(b[pos], b' ' | b'\t') {
            pos += 1;
        }
        if pos == b.len() {
            break;
        }
        let start = pos;
        while pos < b.len() && !matches!(b[pos], b' ' | b'\t') {
            pos += 1;
        }
        out.push(start..pos);
    }
    Ok(out)
}
fn content(bytes: &[u8]) -> Result<&str> {
    std::str::from_utf8(bytes)
        .map(|s| s.strip_prefix('\u{feff}').unwrap_or(s))
        .map_err(|_| Error::Lfs(LfsFailure::Attributes))
}
fn body(line: &str) -> &str {
    line.trim_end_matches('\n').trim_end_matches('\r')
}
fn decoded_pattern(token: &str) -> Result<Vec<u8>> {
    if !token.starts_with('"') {
        return Ok(token.as_bytes().to_vec());
    }
    let quoted = token
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .ok_or(Error::Lfs(LfsFailure::Attributes))?;
    let mut bytes = quoted.bytes();
    let mut out = Vec::new();
    while let Some(b) = bytes.next() {
        if b != b'\\' {
            out.push(b);
            continue;
        }
        let escape = bytes.next().ok_or(Error::Lfs(LfsFailure::Attributes))?;
        out.push(match escape {
            b'a' => 7,
            b'b' => 8,
            b't' => 9,
            b'n' => 10,
            b'v' => 11,
            b'f' => 12,
            b'r' => 13,
            b'\\' | b'"' => escape,
            b'0'..=b'3' => {
                let second = bytes
                    .next()
                    .filter(|b| matches!(b, b'0'..=b'7'))
                    .ok_or(Error::Lfs(LfsFailure::Attributes))?;
                let third = bytes
                    .next()
                    .filter(|b| matches!(b, b'0'..=b'7'))
                    .ok_or(Error::Lfs(LfsFailure::Attributes))?;
                (escape - b'0') * 64 + (second - b'0') * 8 + (third - b'0')
            }
            _ => return fail(LfsFailure::Attributes),
        });
    }
    Ok(out)
}
fn rules(bytes: &[u8]) -> Result<Vec<LfsTrackingRule>> {
    let mut out = Vec::new();
    for (i, line) in content(bytes)?.split_inclusive('\n').enumerate() {
        let line = body(line);
        let fields = tokens(line)?;
        if fields
            .iter()
            .skip(1)
            .any(|r| &line[r.clone()] == "filter=lfs")
        {
            // Macros affect other rules indirectly and are not path patterns.
            if decoded_pattern(&line[fields[0].clone()])?.starts_with(b"[attr]") {
                continue;
            }
            if out.len() >= ENTRY_LIMIT {
                return fail(LfsFailure::Limit);
            }
            out.push(LfsTrackingRule {
                line: i + 1,
                text: line.into(),
            });
        }
    }
    Ok(out)
}

pub(super) fn list(repo: &Repository, context: &Context) -> Result<Vec<LfsTrackingRule>> {
    let (before, _) = snapshot(repo)?;
    index_check(context, before.is_some())?;
    rules(before.as_deref().unwrap_or_default())
}
fn quoted_pattern(pattern: &str, literal: bool) -> Result<String> {
    if pattern.is_empty()
        || pattern.len() > 4096
        || pattern.trim() != pattern
        || pattern.chars().any(char::is_control)
        || pattern.contains(['\\', ':', '"', '<', '>', '|'])
        || pattern.starts_with(['/', '!', '#'])
        || pattern.starts_with("[attr]")
        || pattern.ends_with('/')
        || pattern
            .split('/')
            .any(|s| s.is_empty() || matches!(s, "." | "..") || s.eq_ignore_ascii_case(".git"))
    {
        return fail(LfsFailure::Pattern);
    }
    if literal {
        RepoPath::new(pattern.as_bytes().to_vec())?;
    }
    let mut escaped = String::new();
    // Anchor literal basenames too: 'asset.dat' alone matches every directory.
    if literal {
        escaped.push('/');
    }
    for ch in pattern.chars() {
        if literal && matches!(ch, '*' | '?' | '[' | ']') {
            escaped.push('\\');
        }
        escaped.push(ch);
    }
    // Always quote, escaping the backslash for Git's first-token C decoder.
    Ok(format!("\"{}\"", escaped.replace('\\', "\\\\")))
}
pub(super) fn preview(
    repo: &Repository,
    context: &Context,
    action: &LfsAction,
    c: &CancellationToken,
) -> Result<LfsAttributeEdit> {
    c.check()?;
    let (before, modified) = snapshot(repo)?;
    index_check(context, before.is_some())?;
    let source = before.as_deref().unwrap_or_default();
    let existing = rules(source)?;
    let mut after = source.to_vec();
    let (removed, added) = match action {
        LfsAction::Track { pattern, literal } => {
            let pattern = quoted_pattern(pattern, *literal)?;
            let line = format!("{pattern} filter=lfs diff=lfs merge=lfs -text");
            // An identical earlier rule can be overridden by later patterns.
            // Only suppress an exact final rule; otherwise the append is meaningful.
            if content(source)?
                .lines()
                .rev()
                .find(|s| !s.trim().is_empty() && !s.trim_start().starts_with('#'))
                == Some(line.as_str())
            {
                return fail(LfsFailure::NoChange);
            }
            let newline: &[u8] = if source.windows(2).any(|w| w == b"\r\n") {
                b"\r\n"
            } else {
                b"\n"
            };
            if !after.is_empty() && !after.ends_with(b"\n") && after != b"\xef\xbb\xbf" {
                after.extend_from_slice(newline);
            }
            after.extend_from_slice(line.as_bytes());
            after.extend_from_slice(newline);
            (String::new(), line)
        }
        LfsAction::Untrack { line, expected } => {
            let selected = existing
                .iter()
                .find(|r| r.line == *line && r.text == *expected)
                .ok_or(Error::Lfs(LfsFailure::Stale))?;
            let text = &selected.text;
            let fields = tokens(text)?;
            let mut updated = text.clone();
            let removable =
                |s: &str| matches!(s, "filter=lfs" | "diff=lfs" | "merge=lfs" | "-text");
            let retain = fields.iter().skip(1).any(|r| !removable(&text[r.clone()]));
            if retain {
                for pair in fields.windows(2).rev() {
                    if removable(&text[pair[1].clone()]) {
                        updated.replace_range(pair[0].end..pair[1].end, "");
                    }
                }
            } else {
                updated.clear();
            }
            let text_source = content(source)?;
            let prefix = source.len() - text_source.len();
            let start = prefix
                + text_source
                    .split_inclusive('\n')
                    .take(line - 1)
                    .map(str::len)
                    .sum::<usize>();
            let old_line = text_source
                .split_inclusive('\n')
                .nth(line - 1)
                .ok_or(Error::Lfs(LfsFailure::Stale))?;
            let replacement = if retain {
                format!("{}{}", updated, &old_line[text.len()..])
            } else {
                String::new()
            };
            after.splice(start..start + old_line.len(), replacement.bytes());
            (text.clone(), updated)
        }
        _ => return fail(LfsFailure::Pattern),
    };
    if after.len() > LIMIT {
        return fail(LfsFailure::Attributes);
    }
    c.check()?;
    Ok(LfsAttributeEdit {
        before,
        after,
        modified,
        removed,
        added,
    })
}
pub(super) fn recheck(repo: &Repository, edit: &LfsAttributeEdit) -> Result<()> {
    let (bytes, modified) = snapshot(repo)?;
    if bytes != edit.before || modified != edit.modified {
        return fail(LfsFailure::Stale);
    }
    Ok(())
}
pub(super) fn verify(repo: &Repository, edit: &LfsAttributeEdit) -> Result<()> {
    if snapshot(repo)?.0.as_deref() != Some(edit.after()) {
        return fail(LfsFailure::Verification);
    }
    Ok(())
}
fn save(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut f = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| Error::io("create LFS attributes recovery file", e))?;
    f.write_all(bytes)
        .and_then(|()| f.sync_all())
        .map_err(|e| Error::io("flush LFS attributes recovery file", e))
}
pub(super) fn apply(
    repo: &Repository,
    p: &LfsPreview,
    c: &CancellationToken,
    recovery: &mut Option<PathBuf>,
    completed: &mut usize,
) -> Result<()> {
    let edit = p
        .attributes
        .as_ref()
        .ok_or(Error::Lfs(LfsFailure::Attributes))?;
    let mut guards = local_files::pin_directories(repo.cwd(), repo.cwd())?;
    guards.extend(local_files::pin_directories(
        &repo.info().git_dir,
        &repo.info().git_dir,
    )?);
    let base = repo.info().git_dir.join("rocket-lfs-attributes");
    match fs::create_dir(&base) {
        Ok(()) => (),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => (),
        Err(e) => return Err(Error::io("create LFS attributes recovery root", e)),
    }
    guards.extend(local_files::pin_directories(&repo.info().git_dir, &base)?);
    let id = format!(
        "{}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| Error::Lfs(LfsFailure::Verification))?
            .as_nanos(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    );
    let workspace = base.join(id);
    fs::create_dir(&workspace)
        .map_err(|e| Error::io("create LFS attributes recovery workspace", e))?;
    guards.extend(local_files::pin_directories(
        &repo.info().git_dir,
        &workspace,
    )?);
    *recovery = Some(workspace.clone());
    let prepared = workspace.join("prepared.gitattributes");
    repo.runner.step(p.action.label(), "backup_lfs_attributes", || {
        c.check()?;
        if let Some(before) = &edit.before { save(&workspace.join("before.gitattributes"), before)?; }
        save(&workspace.join("RECOVERY.md"), format!("Rocket LFS tracking recovery\n\nTarget: root .gitattributes\nOriginally present: {}\n\nbefore.gitattributes, when present, is the reviewed original including uncommitted edits. original.gitattributes, when present, is the actual file moved from the worktree. prepared.gitattributes is the reviewed replacement if it has not been published. Inspect current .gitattributes and Git status before restoring anything. Do not overwrite later edits. Staging, tracked file contents and history were not changed by this operation. Cancellation is not rollback.\n", edit.before.is_some()).as_bytes())?;
        save(&prepared, &edit.after)
    })?;
    repo.runner
        .step(p.action.label(), "publish_lfs_attributes", || {
            repo.lfs_recheck(p, c)?;
            let mut prepared_guard =
                disk::open(&prepared, true)?.ok_or(Error::Lfs(LfsFailure::Stale))?;
            if read(&mut prepared_guard)? != edit.after {
                return fail(LfsFailure::Stale);
            }
            let target = path(repo);
            // Keep the original deny-write handle alive across both renames, including
            // writes through hardlinks. Publication never replaces a concurrent arrival.
            let mut original = disk::open(&target, true)?;
            match original.as_mut() {
                Some(f) => {
                    crate::review::verify_preview_handle(f, repo.cwd(), &target)?;
                    if Some(read(f)?) != edit.before {
                        return fail(LfsFailure::Stale);
                    }
                    let meta = f
                        .metadata()
                        .map_err(|e| Error::io("inspect LFS attributes for save", e))?;
                    if meta.permissions().readonly() {
                        return fail(LfsFailure::ReadOnly);
                    }
                    fs::set_permissions(&prepared, meta.permissions())
                        .map_err(|e| Error::io("preserve LFS attributes permissions", e))?;
                    c.check()?;
                    local_files::move_path(
                        &target,
                        &workspace.join("original.gitattributes"),
                        false,
                    )?;
                    *completed = 1;
                    let mut moved = disk::open(&workspace.join("original.gitattributes"), false)?
                        .ok_or(Error::Lfs(LfsFailure::Stale))?;
                    if Some(read(&mut moved)?) != edit.before {
                        return fail(LfsFailure::Stale);
                    }
                }
                None if edit.before.is_some() => return fail(LfsFailure::Stale),
                None => (),
            }
            c.check()?;
            local_files::move_path(&prepared, &target, false)?;
            *completed = 2;
            Ok(())
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lfs_tracking_quoted_macros_are_not_editable_path_rules() {
        let source = b"[attr]plain filter=lfs\n\"[attr]quoted\" filter=lfs\n\"\\133attr]escaped\" filter=lfs\n\"asset.dat\" filter=lfs\n";
        let listed = rules(source).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].line, 4);
        assert!(rules(b"\"bad\\q\" filter=lfs\n").is_err());
        assert_eq!(decoded_pattern("\"\\377\"").unwrap(), [255]);
    }
}
