//! Pin a new regular text file without writing intent-to-add entries or executing
//! filters. Git remains responsible for the final cached patch application.
use super::*;

impl Repository {
    pub fn review_untracked(
        &self,
        path: &RepoPath,
        cancel: &CancellationToken,
    ) -> Result<ChangeReview> {
        self.require_worktree(cancel)?;
        let mut a = args(&["ls-files", "--stage", "-z", "--"]);
        a.push(path.argument()?);
        if !self.read_diff("review-new-file", &a, cancel)?.is_empty() {
            return Err(Error::InvalidInput(
                "This file is now tracked. Refresh its diff before staging selected lines.".into(),
            ));
        }
        // check-ignore accepts filenames, not pathspecs, and rejects the runner's
        // default literal-pathspec mode. -- still protects option-like names.
        let mut a = args(&[
            "--no-literal-pathspecs",
            "check-ignore",
            "--no-index",
            "-q",
            "--",
        ]);
        a.push(path.argument()?);
        let ignored = self.runner.run(self.cwd(), &a, vec![], cancel)?;
        if ignored.status.success() {
            return Err(Error::InvalidInput(
                "This file is ignored. Review its ignore rules before staging.".into(),
            ));
        }
        if ignored.status.code() != Some(1) {
            ignored.success("check new-file ignore rules")?;
        }
        let preview = self.preview_untracked(path, cancel)?;
        let (attrs, config) = self.conversion_settings(path, cancel)?;
        let reason = if preview.truncated {
            Some("This new file is too large for partial staging. Stage the whole file.")
        } else if preview.binary
            || std::str::from_utf8(&preview.bytes).is_err()
            || preview
                .bytes
                .iter()
                .any(|b| *b < 32 && !matches!(b, b'\t' | b'\r' | b'\n'))
        {
            Some("Partial staging of new files requires plain UTF-8 text. Stage the whole file.")
        } else if crate::file_actions::checkout_bytes(vec![], &attrs, &config).is_err() {
            Some(
                "Partial staging cannot safely run this file's filter, encoding or ident conversion. Stage the whole file.",
            )
        } else {
            None
        };
        let raw = preview.bytes;
        let mut bytes = raw.clone();
        let fields: Vec<_> = attrs.split(|b| *b == 0).collect();
        let attr = |name: &[u8]| {
            fields
                .as_chunks::<3>().0.iter()
                .find(|v| v[1] == name)
                .map(|v| v[2])
                .unwrap_or(b"unspecified")
        };
        let auto = config
            .split(|b| *b == 0)
            .filter_map(|r| {
                r.iter()
                    .position(|b| *b == b'\n')
                    .map(|i| (&r[..i], &r[i + 1..]))
            })
            .filter(|(k, _)| *k == b"core.autocrlf")
            .map(|(_, v)| v.to_ascii_lowercase())
            .next_back()
            .unwrap_or_default();
        let normalize = attr(b"text") != b"unset"
            && (matches!(attr(b"text"), b"set" | b"auto")
                || matches!(attr(b"eol"), b"lf" | b"crlf")
                || matches!(auto.as_slice(), b"true" | b"yes" | b"on" | b"1" | b"input"));
        // Git's automatic text detection preserves files containing bare CRs.
        // Refuse that ambiguous new-file case rather than inventing blob bytes.
        let reason = reason.or_else(|| {
            (normalize
                && bytes
                    .iter()
                    .enumerate()
                    .any(|(i, b)| *b == b'\r' && bytes.get(i + 1) != Some(&b'\n')))
            .then_some("Mixed or bare CR line endings require whole-file staging.")
        });
        if normalize && reason.is_none() {
            bytes = bytes
                .iter()
                .enumerate()
                .filter_map(|(i, b)| {
                    (*b != b'\r' || bytes.get(i + 1) != Some(&b'\n')).then_some(*b)
                })
                .collect();
        }
        let mode = {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let root = self
                    .info()
                    .worktree
                    .as_ref()
                    .ok_or(Error::WorktreeRequired)?;
                if std::fs::metadata(root.join(path.argument()?))
                    .map_err(preview_io)?
                    .permissions()
                    .mode()
                    & 0o111
                    != 0
                {
                    0o100755
                } else {
                    0o100644
                }
            }
            #[cfg(not(unix))]
            {
                0o100644
            }
        };
        let a = crate::patches::parse::quote_path(b"a/", path);
        let b = crate::patches::parse::quote_path(b"b/", path);
        let count = bytes.split_inclusive(|b| *b == b'\n').count();
        let mut patch = format!("diff --git {a} {b}\nnew file mode {mode:o}\n--- /dev/null\n+++ {b}\n@@ -0,0 +1,{count} @@\n").into_bytes();
        if !preview.binary {
            for line in bytes.split_inclusive(|b| *b == b'\n') {
                patch.push(b'+');
                patch.extend_from_slice(line);
            }
            if !bytes.ends_with(b"\n") {
                patch.extend_from_slice(b"\n\\ No newline at end of file\n");
            }
        }
        let (hunks, kind, unavailable) = match parse_hunks(&patch) {
            Ok((h,k)) if reason.is_none() => (h,k,None),
            _ => (vec![],ChangeKind::Added(mode),reason.or(Some("Stage this new file as a whole; partial staging requires nonempty text with a final newline."))),
        };
        cancel.check()?;
        Ok(ChangeReview {
            git_dir: self.info().git_dir.clone(),
            path: path.clone(),
            paths: vec![path.clone()],
            staged: false,
            patch,
            hunks,
            kind,
            unavailable,
            untracked: Some((raw, attrs, config)),
        })
    }
}
