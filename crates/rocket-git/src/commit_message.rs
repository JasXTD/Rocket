//! Commit message helpers: the file named by Git's `commit.template`.
use crate::{runner::args, *};
use std::path::PathBuf;

/// Largest template file Rocket reads.
const TEMPLATE_LIMIT: u64 = 64 * 1024;

/// The configured commit template, split the way Git's editor flow treats it:
/// the text a new message starts with, and the comment lines Git would strip.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommitTemplate {
    pub path: PathBuf,
    /// Non-comment lines, without leading or trailing blank lines.
    pub message: String,
    /// Comment lines without their comment prefix, for showing as guidance.
    pub guidance: String,
}

impl Repository {
    /// Read `commit.template`, or `None` when it is not configured. Relative paths
    /// are resolved from the work tree, as Git does when committing there.
    pub fn commit_template(&self, cancel: &CancellationToken) -> Result<Option<CommitTemplate>> {
        let Some(path) = self.config_get("commit.template", true, cancel)? else {
            return Ok(None);
        };
        let path = PathBuf::from(path);
        let path = if path.is_absolute() {
            path
        } else {
            self.cwd().join(path)
        };
        let text = read_template(&path)?;
        let mut comment = self.config_get("core.commentString", false, cancel)?;
        if comment.is_none() {
            comment = self.config_get("core.commentChar", false, cancel)?;
        }
        let comment = comment
            .filter(|c| !c.is_empty() && c != "auto" && !c.contains(['\n', '\r']))
            .unwrap_or_else(|| "#".into());
        Ok(Some(split_template(path, &text, &comment)))
    }

    /// One configuration value, or `None` when the key is not set.
    fn config_get(
        &self,
        key: &str,
        path: bool,
        cancel: &CancellationToken,
    ) -> Result<Option<String>> {
        let mut arguments = args(&["config", "--null"]);
        if path {
            arguments.extend(args(&["--type=path"]));
        }
        arguments.extend(args(&["--get", key]));
        let output = self
            .runner
            .run(self.cwd(), &arguments, Vec::new(), cancel)?;
        if output.status.code() == Some(1) && output.stdout.is_empty() {
            return Ok(None);
        }
        let value = output.success("commit-template")?;
        let value = value.strip_suffix(b"\0").unwrap_or(&value);
        String::from_utf8(value.to_vec())
            .map(Some)
            .map_err(|_| Error::parse("commit-template", "configuration value is not UTF-8"))
    }
}

fn read_template(path: &std::path::Path) -> Result<String> {
    let metadata =
        std::fs::metadata(path).map_err(|e| Error::io("read the commit.template file", e))?;
    if !metadata.is_file() || metadata.len() > TEMPLATE_LIMIT {
        return Err(Error::Unsupported(
            "commit.template must name a text file of at most 64 KiB".into(),
        ));
    }
    let bytes = std::fs::read(path).map_err(|e| Error::io("read the commit.template file", e))?;
    String::from_utf8(bytes)
        .map_err(|_| Error::Unsupported("commit.template is not UTF-8 text".into()))
}

fn split_template(path: PathBuf, text: &str, comment: &str) -> CommitTemplate {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut message = Vec::new();
    let mut guidance = Vec::new();
    for line in text.lines() {
        let line = line.trim_end();
        match line.strip_prefix(comment) {
            Some(rest) => guidance.push(rest.strip_prefix(' ').unwrap_or(rest)),
            None => message.push(line),
        }
    }
    CommitTemplate {
        path,
        message: trim_blank_lines(&message),
        guidance: trim_blank_lines(&guidance),
    }
}

fn trim_blank_lines(lines: &[&str]) -> String {
    let start = lines.iter().position(|l| !l.is_empty());
    let end = lines.iter().rposition(|l| !l.is_empty());
    match (start, end) {
        (Some(start), Some(end)) => lines[start..=end].join("\n"),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn templates_split_message_and_comment_guidance() {
        let t = split_template(
            "t".into(),
            "\u{feff}# Summary: what changed\r\n#\r\n\r\nfeat: \r\n\r\nWhy:\r\n# Explain why 文\r\n\r\n",
            "#",
        );
        assert_eq!(t.message, "feat:\n\nWhy:");
        assert_eq!(t.guidance, "Summary: what changed\n\nExplain why 文");
        // A custom comment string; `#` is then ordinary text.
        let t = split_template("t".into(), "// note\n#123 fix\n", "//");
        assert_eq!(
            (t.message.as_str(), t.guidance.as_str()),
            ("#123 fix", "note")
        );
        // Indented comment characters are not comments, as in Git.
        let t = split_template("t".into(), "  # kept\n", "#");
        assert_eq!(t.message, "  # kept");
        let t = split_template("t".into(), "# only guidance\n", "#");
        assert_eq!(
            (t.message.as_str(), t.guidance.as_str()),
            ("", "only guidance")
        );
    }
}
