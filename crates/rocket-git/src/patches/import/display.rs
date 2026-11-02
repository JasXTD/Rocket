use crate::{
    CancellationToken, Error, RepoPath, Result,
    display_text::{DisplayText, bounded},
};

pub const PREVIEW_LINES: usize = 160;
pub const PREVIEW_BYTES: usize = 16 * 1024;
pub const PREVIEW_PATHS: usize = 64;
/// Lines before a hunk that name paths, which Git quotes when they're unusual.
const PATH_LINES: [&str; 7] = ["diff --git ", "--- ", "+++ ", "rename from ", "rename to ", "copy from ", "copy to "];

/// A path from a patch with invisible characters shown, for a title in a review.
pub fn visible_path(path: &str) -> String {
    bounded(path, 1024).text
}

/// Built once on the review worker. Rendering never scans or decodes raw mail.
#[derive(Clone, Debug)]
pub struct MailImportDisplay {
    pub label: String,
    pub paths: Vec<String>,
    pub patch: String,
    pub total_lines: usize,
    pub shown_lines: usize,
    pub limited: bool,
    pub escaped: bool,
    /// Lines the whole patch adds and removes.
    pub added: usize,
    pub removed: usize,
}

impl MailImportDisplay {
    pub(super) fn new(
        position: usize,
        headers: [&DisplayText; 3],
        paths: &[RepoPath],
        patch: &[u8],
        cancel: &CancellationToken,
    ) -> Result<Self> {
        cancel.check()?;
        let mut out = Self {
            label: format!("{position:02}  {}  ·  {}", headers[0].text, headers[1].text),
            paths: Vec::new(),
            patch: String::new(),
            total_lines: 0,
            shown_lines: 0,
            limited: headers.iter().any(|h| h.truncated) || paths.len() > PREVIEW_PATHS,
            escaped: headers.iter().any(|h| h.escaped),
            added: 0,
            removed: 0,
        };
        for path in paths.iter().take(PREVIEW_PATHS) {
            cancel.check()?;
            let text = bounded(&path.display_lossy(), 1024);
            out.limited |= text.truncated || std::str::from_utf8(path.as_bytes()).is_err();
            out.escaped |= text.escaped;
            out.paths.push(text.text);
        }
        let patch = std::str::from_utf8(patch).map_err(|_| {
            Error::InvalidInput("series_format: The decoded patch must be UTF-8 for review.".into())
        })?;
        // format-patch ends with "-- " and Git's version, which would read as a removed line.
        let signature = patch.rfind("\n-- \n").filter(|at| {
            let mut rest = patch[at + 5..].lines().filter(|l| !l.is_empty());
            rest.next().is_some_and(|version| version.starts_with(|c: char| c.is_ascii_digit())) && rest.next().is_none()
        });
        let (mut in_hunk, mut offset) = (false, 0);
        // Preserve CR and other invisible bytes as escapes. Only LF separates rows.
        for line in patch.split_terminator('\n') {
            cancel.check()?;
            out.total_lines += 1;
            // Inside a hunk, "---" can be a removed line, not a file header.
            in_hunk = line.starts_with("@@") || (in_hunk && !line.starts_with("diff --git"));
            if in_hunk && signature.is_none_or(|at| offset <= at) {
                out.added += usize::from(line.starts_with('+'));
                out.removed += usize::from(line.starts_with('-'));
            }
            offset += line.len() + 1;
            // Git's own quoting of unusual paths stays readable to the diff viewer.
            let header = !in_hunk && PATH_LINES.iter().any(|start| line.starts_with(start));
            let separator = usize::from(out.shown_lines > 0);
            let remaining = PREVIEW_BYTES.saturating_sub(out.patch.len() + separator);
            if out.shown_lines == PREVIEW_LINES || remaining < 8 {
                out.limited = true;
                continue;
            }
            let text = bounded(line, remaining.min(1024));
            out.limited |= text.truncated;
            // Only hunks reach the screen as text: the rest is parsed, or left out.
            out.escaped |= text.escaped && in_hunk;
            if separator > 0 {
                out.patch.push('\n');
            }
            if header {
                out.patch.push_str(&text.text.replace("\\\\", "\\"));
            } else {
                out.patch.push_str(&text.text);
            }
            out.shown_lines += 1;
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_lines_many_lines_and_display_controls_stay_bounded() {
        let header = bounded("ordinary 文", 512);
        for patch in [
            "+x\n".repeat(2000),
            format!("+{}\r\u{202e}\n", "a".repeat(1024 * 1024)),
        ] {
            let preview = MailImportDisplay::new(
                1,
                [&header; 3],
                &[],
                patch.as_bytes(),
                &CancellationToken::new(),
            )
            .unwrap();
            assert!(preview.limited);
            assert!(preview.patch.len() <= PREVIEW_BYTES);
            assert!(preview.shown_lines <= PREVIEW_LINES);
            assert_eq!(preview.total_lines, patch.split_terminator('\n').count());
        }
        let preview = MailImportDisplay::new(
            1,
            [&header; 3],
            &[],
            b"@@ -0,0 +1,2 @@\n+safe\r\n+\tvalue\n",
            &CancellationToken::new(),
        )
        .unwrap();
        assert_eq!(preview.patch, "@@ -0,0 +1,2 @@\n+safe\\r\n+\\tvalue");
        assert!(!preview.limited);
        assert!(preview.escaped);
        let stopped = CancellationToken::new();
        stopped.cancel();
        assert!(matches!(
            MailImportDisplay::new(1, [&header; 3], &[], b"+line\n", &stopped),
            Err(Error::Cancelled)
        ));
    }

    #[test]
    fn git_quoted_paths_stay_as_git_wrote_them() {
        let header = bounded("ordinary", 512);
        let patch = "diff --git \"a/\\350\\263\\207.txt\" \"b/\\350\\263\\207.txt\"\n--- \"a/\\350\\263\\207.txt\"\n+++ \"b/\\350\\263\\207.txt\"\n@@ -1 +1 @@\n-a\\b\n+c\n";
        let preview = MailImportDisplay::new(1, [&header; 3], &[], patch.as_bytes(), &CancellationToken::new()).unwrap();
        assert!(preview.patch.starts_with("diff --git \"a/\\350\\263\\207.txt\""), "{}", preview.patch);
        assert!(preview.patch.contains("-a\\\\b"), "code keeps its visible escape: {}", preview.patch);
    }

    #[test]
    fn the_signature_is_not_a_removed_line() {
        let header = bounded("ordinary", 512);
        let patch = "@@ -1 +1 @@\n-old\n+new\n-- \n2.56.0\n\n";
        let preview = MailImportDisplay::new(1, [&header; 3], &[], patch.as_bytes(), &CancellationToken::new()).unwrap();
        assert_eq!((preview.added, preview.removed), (1, 1));
        let code = "@@ -1,2 +1 @@\n-- \n-old\n+new\n";
        let preview = MailImportDisplay::new(1, [&header; 3], &[], code.as_bytes(), &CancellationToken::new()).unwrap();
        assert_eq!((preview.added, preview.removed), (1, 2), "a removed \"- \" line stays one");
    }

    #[test]
    fn counts_cover_the_whole_patch_and_only_its_hunks() {
        let header = bounded("ordinary", 512);
        let patch = format!(
            "---\n a | 2 +-\n\ndiff --git a/a b/a\n--- a/a\n+++ b/a\n@@ -1,2 +1,2 @@\n keep\n-- old\n+new\n{}",
            "+more\n".repeat(400)
        );
        let preview = MailImportDisplay::new(1, [&header; 3], &[], patch.as_bytes(), &CancellationToken::new()).unwrap();
        assert_eq!((preview.added, preview.removed), (401, 1));
        assert!(preview.limited, "the preview itself stops early");
    }

    #[test]
    fn path_count_and_width_limits_are_explicit() {
        let header = bounded("ordinary", 512);
        let paths: Vec<_> = (0..80)
            .map(|n| {
                RepoPath::new(format!("{n}-{}\u{202e}", "界".repeat(500)).into_bytes()).unwrap()
            })
            .collect();
        let preview =
            MailImportDisplay::new(1, [&header; 3], &paths, b"", &CancellationToken::new())
                .unwrap();
        assert_eq!(preview.paths.len(), PREVIEW_PATHS);
        assert!(preview.paths.iter().all(|p| p.len() <= 1024));
        assert!(preview.limited);
        assert_eq!(preview.total_lines, 0);
        assert!(preview.patch.is_empty());
    }
}
