//! Commit message helpers: the file named by Git's `commit.template`.
use crate::{runner::args, *};
use std::{
    fs,
    io::Read,
    path::{Component, Path, PathBuf},
};

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
        let text = read_template(&path, self.cwd(), self.runner.transport_scope.as_ref())?;
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

fn read_template(path: &Path, repository: &Path, scope: Option<&TransportScope>) -> Result<String> {
    let refused = || {
        Error::InvalidInput(
            "The commit template must be a regular file inside the active fixture.".into(),
        )
    };
    let invalid =
        || Error::Unsupported("commit.template must name a text file of at most 64 KiB".into());
    let io = |e| Error::io("read the commit.template file", e);
    let mut options = fs::OpenOptions::new();
    options.read(true);
    // A FIFO substituted before open must not block the composer.
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        use std::os::unix::fs::OpenOptionsExt;
        #[cfg(target_os = "linux")]
        let (nonblock, nofollow) = (0o4000, 0o400000);
        #[cfg(target_os = "macos")]
        let (nonblock, nofollow) = (0x4, 0x100);
        options.custom_flags(nonblock | if scope.is_some() { nofollow } else { 0 });
    }
    let file = if let Some(scope) = scope {
        // Check the configured spelling before touching it, including UNC paths on Windows.
        let relative = path
            .strip_prefix(repository)
            .or_else(|_| path.strip_prefix(Path::new(&crate::runner::git_path(repository))))
            .map_err(|_| refused())?;
        if !repository.starts_with(scope.root())
            || relative.as_os_str().is_empty()
            || relative
                .components()
                .any(|c| !matches!(c, Component::Normal(_)))
        {
            return Err(refused());
        }
        let path = repository.join(relative);
        let _parents =
            local_files::pin_directories(scope.root(), path.parent().ok_or_else(refused)?)?;
        let leaf = fs::symlink_metadata(&path).map_err(io)?;
        if !leaf.is_file() || local_files::redirected(&leaf) {
            return Err(refused());
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.share_mode(1).custom_flags(0x00200000); // FILE_FLAG_OPEN_REPARSE_POINT
        }
        let file = options.open(&path).map_err(io)?;
        crate::review::verify_handle_under(&file, repository, &path)?;
        if !local_files::single_link(&file)? {
            return Err(refused());
        }
        file
    } else {
        if !fs::metadata(path).map_err(io)?.is_file() {
            return Err(invalid());
        }
        options.open(path).map_err(io)?
    };
    let metadata = file.metadata().map_err(io)?;
    if !metadata.is_file() || metadata.len() > TEMPLATE_LIMIT {
        return Err(invalid());
    }
    let mut bytes = Vec::new();
    file.take(TEMPLATE_LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(io)?;
    if bytes.len() as u64 > TEMPLATE_LIMIT {
        return Err(invalid());
    }
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
