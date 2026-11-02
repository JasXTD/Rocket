use crate::{runner::args, *};
use std::collections::{HashMap, HashSet};

/// Text matching is cheap compared with starting Git and walking the same prefix.
/// Path/content scans still use smaller batches because they inspect file changes.
fn window(kind: HistorySearchKind) -> usize {
    match kind {
        HistorySearchKind::Text => 4096,
        HistorySearchKind::Path => 1000,
        HistorySearchKind::Content => 200,
    }
}

impl Repository {
    /// Search all reachable commits in bounded worker batches, not just displayed
    /// rows: message/author/email text or an ID prefix, a changed path, or changed
    /// content (see [`HistorySearchKind`]). A next cursor means the scan is
    /// incomplete even with zero matches. Queries/cursors remain data and are never
    /// passed to a shell or Git regex; content search uses Git's literal `-S`.
    pub fn search_history(
        &self,
        start: HistorySearchStart,
        cancel: &CancellationToken,
    ) -> Result<HistorySearchPage> {
        let (start, query, kind) = match start {
            HistorySearchStart::Query { start, query, kind } => (start, query, kind),
            HistorySearchStart::Continue(cursor) => (
                HistoryStart::Continue(cursor.history),
                cursor.query,
                cursor.kind,
            ),
        };
        if query.trim().is_empty() || query.len() > 1024 || query.contains('\0') {
            return Err(Error::InvalidInput(
                "enter a search of 1..1024 bytes without NUL".into(),
            ));
        }
        if kind != HistorySearchKind::Text && query.chars().any(char::is_control) {
            return Err(Error::InvalidInput(
                "path and content searches take one line of text".into(),
            ));
        }
        let offset = match &start {
            HistoryStart::Continue(c) => c.offset,
            _ => 0,
        };
        let page = match self.history_batch(start.clone(), window(kind), cancel) {
            // Large messages can exhaust the unchanged output-byte limit. Retry
            // only that bounded read with the former batch size; cancellation,
            // malformed data and all other failures must propagate unchanged.
            Err(Error::OutputLimit {
                stream: "stdout", ..
            }) if kind == HistorySearchKind::Text => {
                cancel.check()?;
                self.history(start, 1000, cancel)?
            }
            result => result?,
        };
        let total = page.commits.len();
        let needle = query.to_lowercase();
        let matched = match kind {
            HistorySearchKind::Text => None,
            HistorySearchKind::Path => Some(self.commits_changing_path(
                &page.commits,
                &needle.replace('\\', "/"),
                cancel,
            )?),
            HistorySearchKind::Content => {
                Some(self.commits_changing_content(&page.commits, &query, cancel)?)
            }
        };
        let mut commits = Vec::new();
        let mut scanned = 0;
        for commit in page.commits {
            cancel.check()?;
            scanned += 1;
            let found = match &matched {
                Some(ids) => ids.contains(&commit.id),
                None => {
                    commit.id.as_str().starts_with(&needle)
                        || String::from_utf8_lossy(&commit.message)
                            .to_lowercase()
                            .contains(&needle)
                        || String::from_utf8_lossy(&commit.author.name)
                            .to_lowercase()
                            .contains(&needle)
                        || String::from_utf8_lossy(&commit.author.email)
                            .to_lowercase()
                            .contains(&needle)
                }
            };
            if found {
                commits.push(commit);
                if commits.len() == 200 {
                    break;
                }
            }
        }
        let next = if scanned < total || page.next.is_some() {
            Some(HistorySearchCursor {
                history: HistoryCursor {
                    repository: self.info().git_dir.clone(),
                    tips: page.tips,
                    order: page.order,
                    offset: offset
                        .checked_add(scanned)
                        .ok_or_else(|| Error::InvalidInput("search cursor overflow".into()))?,
                },
                query,
                kind,
            })
        } else {
            None
        };
        Ok(HistorySearchPage {
            commits,
            next,
            scanned,
        })
    }

    /// Commits among `commits` whose own change (against the first parent, or the
    /// empty tree for a root commit) touches a path containing `needle`. Renames
    /// count as both paths. Merges show no change, as in `git log`.
    fn commits_changing_path(
        &self,
        commits: &[Commit],
        needle: &str,
        cancel: &CancellationToken,
    ) -> Result<HashSet<ObjectId>> {
        if commits.is_empty() {
            return Ok(HashSet::new());
        }
        let output = self
            .runner
            .run(
                self.cwd(),
                &args(&[
                    "diff-tree",
                    "--stdin",
                    "-r",
                    "-z",
                    "--name-only",
                    "--no-renames",
                    "--no-color",
                    "--root",
                    "--always",
                ]),
                commit_lines(commits),
                cancel,
            )?
            .success("history-search")?;
        let ids: Vec<&ObjectId> = commits.iter().map(|c| &c.id).collect();
        changed_path_matches(&output, &ids, needle)
    }

    /// Commits among `commits` whose change adds or removes occurrences of `text`.
    /// External diff and textconv programs are not run.
    fn commits_changing_content(
        &self,
        commits: &[Commit],
        text: &str,
        cancel: &CancellationToken,
    ) -> Result<HashSet<ObjectId>> {
        if commits.is_empty() {
            return Ok(HashSet::new());
        }
        let output = self
            .runner
            .run(
                self.cwd(),
                &args(&[
                    "log",
                    "--no-walk=unsorted",
                    "--stdin",
                    "--no-color",
                    "--no-show-signature",
                    "--no-ext-diff",
                    "--no-textconv",
                    "--regexp-ignore-case",
                    &format!("-S{text}"),
                    "--format=%H",
                ]),
                commit_lines(commits),
                cancel,
            )?
            .success("history-search")?;
        let wanted: HashMap<&str, &ObjectId> =
            commits.iter().map(|c| (c.id.as_str(), &c.id)).collect();
        String::from_utf8(output)
            .map_err(|_| Error::parse("history-search", "non-UTF-8 commit ID"))?
            .lines()
            .map(|line| match wanted.get(line) {
                Some(id) => Ok((*id).clone()),
                None => Err(Error::parse("history-search", "unexpected commit ID")),
            })
            .collect()
    }
}

fn commit_lines(commits: &[Commit]) -> Vec<u8> {
    let mut input = Vec::with_capacity(commits.len() * 65);
    for commit in commits {
        input.extend_from_slice(commit.id.as_str().as_bytes());
        input.push(b'\n');
    }
    input
}

/// Parse `diff-tree --stdin -z --name-only --always`: each commit ID in input
/// order, then the NUL-terminated paths it changed.
fn changed_path_matches(
    output: &[u8],
    ids: &[&ObjectId],
    needle: &str,
) -> Result<HashSet<ObjectId>> {
    let mut matches = HashSet::new();
    let mut next = 0;
    let mut current: Option<&ObjectId> = None;
    let tokens = output.strip_suffix(b"\0").unwrap_or(output);
    if !tokens.is_empty() {
        for token in tokens.split(|b| *b == 0) {
            if let Some(id) = ids.get(next)
                && token == id.as_str().as_bytes()
            {
                current = Some(id);
                next += 1;
                continue;
            }
            let Some(id) = current else {
                return Err(Error::parse("history-search", "path before a commit ID"));
            };
            if String::from_utf8_lossy(token)
                .to_lowercase()
                .contains(needle)
            {
                matches.insert(id.clone());
            }
        }
    }
    if next != ids.len() {
        return Err(Error::parse(
            "history-search",
            "missing commit in changed paths",
        ));
    }
    Ok(matches)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn changed_paths_are_grouped_by_commit_in_input_order() {
        let [a, b, c] = ["a", "b", "c"].map(|x| ObjectId::parse(&x.repeat(40)).unwrap());
        let ids = [&a, &b, &c];
        let output = format!(
            "{a}\0src/Main.rs\0docs/文.md\0{b}\0{c}\0README\0",
            a = a.as_str(),
            b = b.as_str(),
            c = c.as_str()
        );
        let found = changed_path_matches(output.as_bytes(), &ids, "main").unwrap();
        assert_eq!(found, HashSet::from([a.clone()]));
        let found = changed_path_matches(output.as_bytes(), &ids, "文").unwrap();
        assert_eq!(found, HashSet::from([a.clone()]));
        // A merge (b) lists no paths, so it never matches.
        assert!(
            changed_path_matches(output.as_bytes(), &ids, "zzz")
                .unwrap()
                .is_empty()
        );
        let missing = format!("{}\0x\0{}\0", a.as_str(), b.as_str());
        assert!(changed_path_matches(missing.as_bytes(), &ids, "x").is_err());
        assert!(changed_path_matches(b"stray\0", &[], "x").is_err());
        assert!(changed_path_matches(b"", &[], "x").unwrap().is_empty());
    }
}
