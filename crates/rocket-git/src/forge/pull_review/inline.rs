//! Comment locations come from complete, bounded provider diff hunks, not typed paths.
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ForgeDiffSide {
    Left,
    Right,
}
impl ForgeDiffSide {
    pub fn key(self) -> &'static str {
        match self {
            Self::Left => "LEFT",
            Self::Right => "RIGHT",
        }
    }
}
#[derive(Clone, Debug)]
pub struct ForgePullLine {
    pub old: Option<u64>,
    pub new: Option<u64>,
    pub text: String,
}
#[derive(Clone, Debug)]
pub struct ForgeInlineComment {
    repository: ForgeRepository,
    session: u64,
    number: u64,
    head: String,
    base: String,
    pub(super) path: String,
    pub(super) line: u64,
    start: u64,
    pub(super) side: ForgeDiffSide,
    pub(super) body: String,
    pub(super) anchor: Anchor,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Anchor {
    pub(super) old_path: String,
    pub(super) old: Option<u64>,
    pub(super) new: Option<u64>,
    refs: Option<DiffRefs>,
    pub(super) provider: Option<FileAnchor>,
    revision: Option<String>,
}
/// Untrusted persisted text/location. Restoring never restores a live capability:
/// a fresh PR snapshot must validate both revisions and the complete diff range.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ForgeInlineDraft {
    pub path: String,
    pub start: u64,
    pub line: u64,
    pub side: ForgeDiffSide,
    pub head: String,
    pub base: String,
    pub body: String,
    /// Provider diff/iteration version, in addition to the reviewed head/base.
    pub diff_revision: Option<String>,
}
impl ForgeInlineComment {
    pub(super) fn review_binding(&self) -> String {
        format!("{:?}:{:?}", self.draft(), self.anchor)
    }
    pub fn start_line(&self) -> u64 { self.start }
    pub fn draft(&self) -> ForgeInlineDraft {
        ForgeInlineDraft { path: self.path.clone(), start: self.start, line: self.line,
            side: self.side, head: self.head.clone(), base: self.base.clone(), body: self.body.clone(),
            diff_revision: self.anchor.revision.clone() }
    }
    pub fn path(&self) -> &str {
        &self.path
    }
    pub fn line(&self) -> u64 {
        self.line
    }
    pub fn side(&self) -> ForgeDiffSide {
        self.side
    }
    pub fn body(&self) -> &str {
        &self.body
    }
    pub fn head(&self) -> &str {
        &self.head
    }
    pub fn edit_body(&mut self, body: &str) -> Result<()> {
        validate_body(body)?;
        self.body = body.into();
        Ok(())
    }
}
fn validate_body(body: &str) -> Result<()> {
    if body.trim().is_empty() || body.len() > 8192 || body.contains('\0') {
        return Err(invalid(
            "Inline comments need a message of at most 8 KiB without NUL.",
        ));
    }
    Ok(())
}
impl ForgePullDetails {
    pub fn inline_diff_revision(&self) -> Option<String> {
        match self.repository.config().provider() {
            ForgeProvider::BitbucketServer => self.summary.revision.map(|v| format!("bitbucket-server:{v}")),
            p if p.is_azure() => self.files.items.iter().find_map(|f| match &f.anchor {
                Some(FileAnchor::Azure { iteration: r, .. }) => Some(format!("azure:{}:{}:{}:{}", r.id, r.common, r.source, r.target)),
                _ => None,
            }),
            _ => self.summary.diff_refs.as_ref().map(diff_revision),
        }
    }
    pub fn supports_inline_review(&self) -> bool {
        match self.repository.config().provider() {
            p if p.is_github() => true,
            ForgeProvider::Gitea => true,
            ForgeProvider::Bitbucket => true,
            ForgeProvider::BitbucketServer => self.summary.revision.is_some(),
            p if p.is_azure() => self.inline_diff_revision().is_some(),
            ForgeProvider::GitLab => self.summary.diff_refs.as_ref().is_some_and(|r| r.head == self.summary.head),
            _ => false,
        }
    }
    pub fn supports_inline_ranges(&self) -> bool { self.repository.config().provider().is_github() }
    pub fn inline_comment_limit(&self) -> usize {
        if !self.supports_inline_review() { 0 }
        else if self.repository.config().provider().is_github() || self.repository.config().provider() == ForgeProvider::Gitea { 32 } else { 1 }
    }
    pub fn inline_comment(
        &self,
        file: usize,
        line: u64,
        side: ForgeDiffSide,
        body: &str,
    ) -> Result<ForgeInlineComment> {
        self.inline_comment_range(file, line, line, side, body)
    }
    pub fn restore_inline_comment(&self, draft: &ForgeInlineDraft) -> Result<ForgeInlineComment> {
        if draft.head != self.summary.head || draft.base != self.summary.base
            || draft.diff_revision != self.inline_diff_revision() {
            return fail(ForgeFailure::PullReviewStale);
        }
        let file = self.files.items.iter().position(|f| f.raw_path.as_deref() == Some(&draft.path))
            .ok_or_else(|| invalid("The saved comment's file is absent from the current diff. Keep its text and select a new location."))?;
        self.inline_comment_range(file, draft.start, draft.line, draft.side, &draft.body)
    }
    pub fn inline_comment_range(
        &self, file: usize, start: u64, line: u64, side: ForgeDiffSide, body: &str,
    ) -> Result<ForgeInlineComment> {
        if !self.supports_inline_review() {
            return fail(ForgeFailure::Unsupported);
        }
        if start != line && !self.supports_inline_ranges() {
            return Err(invalid("This provider currently supports single-line comments. Select one line."));
        }
        validate_body(body)?;
        let file = self
            .files
            .items
            .get(file)
            .ok_or_else(|| invalid("Select a loaded PR file."))?;
        let path = file.raw_path.as_ref().ok_or_else(|| {
            invalid("This path cannot safely identify an inline comment. Open it on the provider.")
        })?;
        if !valid_range(&file.lines, start, line, side) {
            return Err(invalid("Choose available lines on the same side of one complete diff hunk. Missing lines, reversed ranges and ranges crossing hunks are refused."));
        }
        let row = file.lines.iter().find(|r| match side { ForgeDiffSide::Left => r.old, ForgeDiffSide::Right => r.new } == Some(line))
            .ok_or_else(|| invalid("The selected line is absent from this diff."))?;
        let old_path = if file.previous.is_some() {
            file.raw_previous.as_ref().ok_or_else(|| invalid("The original path cannot safely identify this renamed file."))?
        } else { path };
        Ok(ForgeInlineComment {
            repository: self.repository.clone(),
            session: self.session,
            number: self.summary.number,
            head: self.summary.head.clone(),
            base: self.summary.base.clone(),
            path: path.clone(),
            line,
            start,
            side,
            body: body.into(),
            anchor: Anchor { old_path: old_path.clone(), old: row.old, new: row.new, refs: self.summary.diff_refs.clone(), provider: file.anchor.clone(), revision: self.inline_diff_revision() },
        })
    }
}
pub(super) fn validate_comments(
    details: &ForgePullDetails,
    comments: &[ForgeInlineComment],
    body_bytes: usize,
) -> Result<()> {
    if comments.len() > details.inline_comment_limit()
        || comments.iter().map(|c| c.body.len()).sum::<usize>() + body_bytes > 128 * 1024
    {
        return Err(invalid(
            "Use at most 32 inline comments and 128 KiB of total review text.",
        ));
    }
    let mut locations = HashSet::new();
    for comment in comments {
        if comment.repository != details.repository
            || comment.session != details.session
            || comment.number != details.summary.number
            || comment.head != details.summary.head
            || comment.base != details.summary.base
        {
            return fail(ForgeFailure::PullReviewStale);
        }
        validate_body(&comment.body)?;
        let file = details
            .files
            .items
            .iter()
            .position(|f| f.raw_path.as_deref() == Some(&comment.path))
            .ok_or_else(|| {
                invalid(
                    "A commented file is no longer available. Refresh and re-select the location.",
                )
            })?;
        let restored = details.inline_comment_range(file, comment.start, comment.line, comment.side, &comment.body)?;
        if restored.anchor != comment.anchor { return fail(ForgeFailure::PullReviewStale); }
        if !locations.insert((&comment.path, comment.side, comment.start, comment.line)) {
            return Err(invalid(
                "There is already a draft comment on this side and line. Edit that comment instead.",
            ));
        }
    }
    Ok(())
}
fn diff_revision(r: &DiffRefs) -> String { format!("{}:{}:{}", r.base, r.head, r.start) }
pub(super) fn gitlab_payload(c: &ForgeInlineComment) -> Result<String> {
    let r = c.anchor.refs.as_ref().ok_or(Error::Forge(ForgeFailure::PullReviewStale))?;
    let mut lines = String::new();
    if let Some(n) = c.anchor.old { lines.push_str(&format!(",\"old_line\":{n}")); }
    if let Some(n) = c.anchor.new { lines.push_str(&format!(",\"new_line\":{n}")); }
    Ok(format!("{{\"body\":{},\"position\":{{\"position_type\":\"text\",\"base_sha\":{},\"head_sha\":{},\"start_sha\":{},\"old_path\":{},\"new_path\":{}{lines}}}}}",
        quote(&c.body), quote(&r.base), quote(&r.head), quote(&r.start), quote(&c.anchor.old_path), quote(&c.path)))
}
pub(super) fn gitlab_matches(j: &Json, c: &ForgeInlineComment) -> bool {
    let Some(p) = j.get("position") else { return false };
    let Some(r) = &c.anchor.refs else { return false };
    j.str("body") == Some(&c.body) && p.str("position_type") == Some("text")
        && p.str("base_sha") == Some(&r.base) && p.str("head_sha") == Some(&r.head)
        && p.str("start_sha") == Some(&r.start) && p.str("old_path") == Some(&c.anchor.old_path)
        && p.str("new_path") == Some(&c.path) && p.u64("old_line") == c.anchor.old && p.u64("new_line") == c.anchor.new
}
pub(super) fn gitea_payload(comments: &[ForgeInlineComment]) -> String {
    if comments.is_empty() { return String::new(); }
    format!(",\"comments\":[{}]", comments.iter().map(|c| {
        let (old, new) = match c.side { ForgeDiffSide::Left => (c.line,0), ForgeDiffSide::Right => (0,c.line) };
        format!("{{\"body\":{},\"path\":{},\"old_position\":{old},\"new_position\":{new}}}",quote(&c.body),quote(&c.path))
    }).collect::<Vec<_>>().join(","))
}
pub(super) fn payload(comments: &[ForgeInlineComment]) -> String {
    if comments.is_empty() {
        return String::new();
    }
    format!(
        ",\"comments\":[{}]",
        comments
            .iter()
            .map(|c| format!(
                "{{\"path\":{},\"line\":{},\"side\":{},\"body\":{}{}}}",
                quote(&c.path),
                c.line,
                quote(c.side.key()),
                quote(&c.body),
                if c.start != c.line { format!(",\"start_line\":{},\"start_side\":{}",c.start,quote(c.side.key())) } else { String::new() }
            ))
            .collect::<Vec<_>>()
            .join(",")
    )
}
fn valid_range(rows: &[ForgePullLine], start: u64, end: u64, side: ForgeDiffSide) -> bool {
    if start == 0 || start > end || end - start >= 2000 { return false; }
    let mut next = start;
    for row in rows {
        if row.old.is_none() && row.new.is_none() {
            if next != start { return false; }
            continue;
        }
        let number = match side { ForgeDiffSide::Left => row.old, ForgeDiffSide::Right => row.new };
        if number == Some(next) {
            if next == end { return true; }
            next += 1;
        }
    }
    false
}
pub(super) fn safe_path(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= 8192
        && !path.starts_with('/')
        && !path
            .split('/')
            .any(|s| s.is_empty() || matches!(s, "." | ".."))
        && crate::display_text::bounded(path, 8192).text == path
}
fn range(text: &str) -> Option<(u64, u64)> {
    let (start, count) = text.split_once(',').unwrap_or((text, "1"));
    let (start, count) = (start.parse::<u64>().ok()?, count.parse::<u64>().ok()?);
    (start <= u32::MAX as u64 && count <= 2000 && (start > 0 || count == 0))
        .then_some((start, count))
}
/// Reject malformed/truncated hunks instead of inventing line numbers. The
/// bounded raw patch is parsed before display escaping can alter its text.
pub(super) fn parse_lines(patch: &str) -> Option<Vec<ForgePullLine>> {
    let (mut old, mut new, mut left, mut right) = (0u64, 0u64, 0u64, 0u64);
    let (mut end_old, mut end_new) = (0u64, 0u64);
    let mut rows = Vec::new();
    let mut started = false;
    let mut had_line = false;
    for line in patch.lines() {
        if rows.len() >= 2000 {
            return None;
        }
        if let Some(header) = line.strip_prefix("@@ -") {
            if left != 0 || right != 0 {
                return None;
            }
            let (old_range, rest) = header.split_once(" +")?;
            let (new_range, _) = rest.split_once(" @@")?;
            (old, left) = range(old_range)?;
            (new, right) = range(new_range)?;
            if started && (old < end_old || new < end_new) {
                return None;
            }
            end_old = old.checked_add(left)?;
            end_new = new.checked_add(right)?;
            started = true;
            had_line = false;
            rows.push(ForgePullLine {
                old: None,
                new: None,
                text: bounded(line, 8192),
            });
            continue;
        }
        if !started {
            return None;
        }
        if line == "\\ No newline at end of file" {
            if !had_line {
                return None;
            }
            had_line = false;
            continue;
        }
        let (at_old, at_new) = match line.as_bytes().first()? {
            b' ' if left > 0 && right > 0 => {
                let v = (Some(old), Some(new));
                old += 1;
                new += 1;
                left -= 1;
                right -= 1;
                v
            }
            b'-' if left > 0 => {
                let v = (Some(old), None);
                old += 1;
                left -= 1;
                v
            }
            b'+' if right > 0 => {
                let v = (None, Some(new));
                new += 1;
                right -= 1;
                v
            }
            _ => return None,
        };
        rows.push(ForgePullLine {
            old: at_old,
            new: at_new,
            text: bounded(line, 8192),
        });
        had_line = true;
    }
    (started && left == 0 && right == 0).then_some(rows)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn multi_line_range_stays_inside_one_complete_hunk() {
        let rows = parse_lines("@@ -1,2 +1,3 @@\n one\n-old\n+two\n+three\n@@ -3 +4 @@\n four").unwrap();
        assert!(valid_range(&rows,1,3,ForgeDiffSide::Right));
        assert!(valid_range(&rows,1,2,ForgeDiffSide::Left));
        for (start,end) in [(0,1),(3,2),(2,4),(9,9),(1,2001)] {
            assert!(!valid_range(&rows,start,end,ForgeDiffSide::Right));
        }
    }
    #[test]
    fn maps_add_delete_context_and_multiple_hunks_without_guessing() {
        let rows = parse_lines("@@ -10,2 +10,2 @@ fn\n context\n-old\n+new 界\n@@ -20,0 +21,1 @@\n+extra\n\\ No newline at end of file").unwrap();
        assert_eq!((rows[1].old, rows[1].new), (Some(10), Some(10)));
        assert_eq!((rows[2].old, rows[2].new), (Some(11), None));
        assert_eq!((rows[3].old, rows[3].new), (None, Some(11)));
        assert_eq!(rows.last().unwrap().new, Some(21));
        for patch in [
            "@@ -1,2 +1 @@\n-old\n+new",
            "@@ -1 +1 @@\n-new",
            "@@ -0 +1 @@\n-a\n+b",
            "@@ -1 +1 @@\n a\n@@ -1 +1 @@\n a",
        ] {
            assert!(parse_lines(patch).is_none(), "{patch}");
        }
    }
    #[test]
    fn path_display_cannot_change_the_comment_destination() {
        assert!(safe_path("資料/é 🚀.rs"));
        for path in ["../a", "/a", "a\nb", "a\u{202e}b", ""] {
            assert!(!safe_path(path));
        }
    }
}
