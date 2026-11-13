//! Bitbucket and Azure positions are validated data, never filesystem paths.
//! Every mutation is one POST. An unverified acknowledgement is not retried.
use super::*;

pub(super) fn safe_provider_path(p: ForgeProvider, path: &str) -> bool {
    if p.is_azure() {
        path.strip_prefix('/').is_some_and(inline::safe_path)
    } else {
        inline::safe_path(path)
    }
}
fn positive(n: Option<u64>) -> Option<u64> {
    n.filter(|n| *n > 0 && *n <= i32::MAX as u64)
}
fn truncated(j: &Json) -> Result<bool> {
    match j.get("truncated") {
        None | Some(Json::Null) => Ok(false),
        _ if j.bool("truncated") == Some(false) || j.str("truncated") == Some("false") => Ok(false),
        _ if j.bool("truncated") == Some(true) || j.str("truncated") == Some("true") => Ok(true),
        _ => fail(ForgeFailure::InvalidResponse),
    }
}
fn server_path<'a>(j: &'a Json, key: &str) -> Option<&'a str> {
    j.str(key).or_else(|| txt(j, &[key, "toString"]))
}

pub(super) fn attach_server_diff(
    files: &mut [ForgePullFile],
    j: &Json,
    head: &str,
    c: &CancellationToken,
) -> Result<bool> {
    if truncated(j)? {
        return Ok(true);
    }
    let diffs = need(
        j.get("diffs")
            .and_then(Json::array)
            .filter(|r| r.len() <= 200),
    )?;
    let mut mapped = Vec::new();
    let mut seen = HashSet::new();
    let mut limited = false;
    for d in diffs {
        c.check()?;
        if truncated(d)? {
            limited = true;
            continue;
        }
        let old = server_path(d, "source");
        let new = server_path(d, "destination");
        let name = need(new.or(old))?;
        if !seen.insert(name) {
            return fail(ForgeFailure::InvalidResponse);
        }
        let Some((i, f)) = files
            .iter()
            .enumerate()
            .find(|(_, f)| f.raw_path.as_deref() == Some(name))
        else {
            continue;
        };
        let old_expected = if f.status == "ADD" {
            None
        } else {
            Some(f.raw_previous.as_deref().unwrap_or(name))
        };
        let new_expected = if f.status == "DELETE" {
            None
        } else {
            Some(name)
        };
        if old != old_expected || new != new_expected {
            return fail(ForgeFailure::InvalidResponse);
        }
        let from = need(
            d.str("fromHash")
                .or_else(|| j.str("fromHash"))
                .and_then(oid),
        )?;
        let to = need(d.str("toHash").or_else(|| j.str("toHash")).and_then(oid))?;
        if to != head {
            return fail(ForgeFailure::PullChanged);
        }
        let hunks = need(
            d.get("hunks")
                .and_then(Json::array)
                .filter(|r| r.len() <= 200),
        )?;
        let mut patch = String::new();
        let mut complete = true;
        for h in hunks {
            c.check()?;
            if truncated(h)? {
                complete = false;
                break;
            }
            let (s, sn, t, tn) = (
                need(h.u64("sourceLine"))?,
                need(h.u64("sourceSpan"))?,
                need(h.u64("destinationLine"))?,
                need(h.u64("destinationSpan"))?,
            );
            if sn > 2000 || tn > 2000 || s > u32::MAX as u64 || t > u32::MAX as u64 {
                complete = false;
                break;
            }
            patch.push_str(&format!("@@ -{s},{sn} +{t},{tn} @@\n"));
            let (mut left, mut right) = (s, t);
            for seg in need(
                h.get("segments")
                    .and_then(Json::array)
                    .filter(|a| a.len() <= 2000),
            )? {
                if truncated(seg)? {
                    complete = false;
                    break;
                }
                let kind = match seg.str("type") {
                    Some("CONTEXT") => ' ',
                    Some("ADDED") => '+',
                    Some("REMOVED") => '-',
                    _ => return fail(ForgeFailure::InvalidResponse),
                };
                for line in need(
                    seg.get("lines")
                        .and_then(Json::array)
                        .filter(|a| a.len() <= 2000),
                )? {
                    if truncated(line)? {
                        complete = false;
                        break;
                    }
                    if kind != '+' {
                        if line.u64("source") != Some(left) {
                            return fail(ForgeFailure::InvalidResponse);
                        }
                        left += 1;
                    }
                    if kind != '-' {
                        if line.u64("destination") != Some(right) {
                            return fail(ForgeFailure::InvalidResponse);
                        }
                        right += 1;
                    }
                    let text = need(line.str("line"))?;
                    if text.contains(['\n', '\0']) {
                        return fail(ForgeFailure::InvalidResponse);
                    }
                    patch.push(kind);
                    patch.push_str(text);
                    patch.push('\n');
                    if patch.len() > 16384 {
                        complete = false;
                        break;
                    }
                }
                if !complete {
                    break;
                }
            }
            if !complete {
                break;
            }
            if left - s != sn || right - t != tn {
                return fail(ForgeFailure::InvalidResponse);
            }
        }
        if !complete {
            limited = true;
            continue;
        }
        if let Some(lines) = inline::parse_lines(&patch) {
            mapped.push((i, patch, lines, FileAnchor::BitbucketServer { from, to }));
        } else if !patch.is_empty() {
            return fail(ForgeFailure::InvalidResponse);
        }
    }
    for (i, patch, lines, anchor) in mapped {
        files[i].patch = Some(multiline(&patch, 16384));
        files[i].lines = lines;
        files[i].anchor = Some(anchor);
    }
    Ok(limited)
}

impl ForgeSession {
    fn azure_iteration(&self, path: &str, c: &CancellationToken) -> Result<AzureIteration> {
        let rows = self.first_page(
            &format!("{path}/iterations"),
            Some("value"),
            "read_pull_iterations",
            c,
            |j| Some(j.clone()),
        )?;
        if rows.limited {
            return fail(ForgeFailure::ReviewStateUnavailable);
        }
        if let Some(e) = rows.error {
            return fail(e);
        }
        let mut ids = HashSet::new();
        let mut latest: Option<AzureIteration> = None;
        for row in rows.items {
            let id = need(positive(row.u64("id")))?;
            if !ids.insert(id) {
                return fail(ForgeFailure::InvalidResponse);
            }
            let iteration = AzureIteration {
                id,
                common: need(txt(&row, &["commonRefCommit", "commitId"]).and_then(oid))?,
                source: need(txt(&row, &["sourceRefCommit", "commitId"]).and_then(oid))?,
                target: need(txt(&row, &["targetRefCommit", "commitId"]).and_then(oid))?,
            };
            if latest.as_ref().is_none_or(|r| r.id < id) {
                latest = Some(iteration);
            }
        }
        need(latest)
    }
    fn azure_content(
        &self,
        repo: &ForgeRepository,
        path: &str,
        commit: &str,
        c: &CancellationToken,
    ) -> Result<Option<String>> {
        let endpoint = self.versioned(&format!("{}/items?path={}&versionDescriptor.version={}&versionDescriptor.versionType=commit&includeContent=true&includeContentMetadata=true&resolveLfs=false&$format=json", self.prefix(repo), encode(path), commit));
        let j = self
            .request(
                READ,
                "read_pull_file_content",
                "GET",
                &endpoint,
                None,
                false,
                c,
            )?
            .0;
        if j.str("path") != Some(path) || j.str("commitId") != Some(commit) {
            return fail(ForgeFailure::PullChanged);
        }
        Ok(azure_text(&j))
    }
    pub(super) fn azure_files(
        &self,
        repo: &ForgeRepository,
        path: &str,
        head: &str,
        c: &CancellationToken,
    ) -> Result<ForgePullCollection<ForgePullFile>> {
        let iteration = self.azure_iteration(path, c)?;
        if iteration.source != head {
            return fail(ForgeFailure::PullChanged);
        }
        let entries = self.first_page(
            &format!(
                "{path}/iterations/{}/changes?$top=50&$compareTo=0",
                iteration.id
            ),
            Some("changeEntries"),
            "read_pull_files",
            c,
            |j| Some(j.clone()),
        )?;
        let mut files = ForgePullCollection {
            items: vec![],
            limited: entries.limited,
            error: entries.error,
        };
        let mut tracking_ids = HashSet::new();
        for (index, entry) in entries.items.iter().enumerate() {
            c.check()?;
            let mut f = need(file(entry, self.config.provider()))?;
            let tracking = need(positive(entry.u64("changeTrackingId")))?;
            if !tracking_ids.insert(tracking) {
                return fail(ForgeFailure::InvalidResponse);
            }
            f.anchor = Some(FileAnchor::Azure {
                iteration: iteration.clone(),
                tracking,
            });
            if index >= 8
                || f.raw_path.is_none()
                || (f.previous.is_some() && f.raw_previous.is_none())
            {
                files.limited = true;
                files.items.push(f);
                continue;
            }
            let name = f.raw_path.as_deref().expect("checked");
            let kinds: Vec<_> = f.status.split(',').map(str::trim).collect();
            if kinds
                .iter()
                .any(|k| !matches!(*k, "add" | "edit" | "delete" | "rename" | "encoding"))
            {
                files.items.push(f);
                continue;
            }
            let old = if kinds.contains(&"add") {
                Some(String::new())
            } else {
                self.azure_content(
                    repo,
                    f.raw_previous.as_deref().unwrap_or(name),
                    &iteration.common,
                    c,
                )?
            };
            let new = if kinds.contains(&"delete") {
                Some(String::new())
            } else {
                self.azure_content(repo, name, &iteration.source, c)?
            };
            if let (Some(old), Some(new)) = (old, new) {
                if let Some(patch) = azure_diff::patch(&old, &new, c)? {
                    if let Some(lines) = inline::parse_lines(&patch) {
                        f.lines = lines;
                        f.additions =
                            f.lines.iter().filter(|r| r.text.starts_with('+')).count() as u64;
                        f.deletions =
                            f.lines.iter().filter(|r| r.text.starts_with('-')).count() as u64;
                        f.patch = Some(multiline(&patch, 16384));
                    } else {
                        files.limited = true;
                    }
                } else if old != new {
                    files.limited = true;
                }
            } else {
                files.limited = true;
            }
            files.items.push(f);
        }
        Ok(files)
    }
    pub(in crate::forge::pull_review) fn validate_other_inline_snapshot(
        &self,
        repo: &ForgeRepository,
        number: u64,
        comments: &[ForgeInlineComment],
        c: &CancellationToken,
    ) -> Result<()> {
        if !self.config.provider().is_azure() || comments.is_empty() {
            return Ok(());
        }
        let path = self.provider_path(repo, number)?;
        let current = self.azure_iteration(&path, c)?;
        for comment in comments {
            let Some(FileAnchor::Azure {
                iteration,
                tracking,
            }) = &comment.anchor.provider
            else {
                return fail(ForgeFailure::PullReviewStale);
            };
            if &current != iteration {
                return fail(ForgeFailure::PullChanged);
            }
            let entries = self.first_page(
                &format!(
                    "{path}/iterations/{}/changes?$top=50&$compareTo=0",
                    current.id
                ),
                Some("changeEntries"),
                "verify_pull_comment_location",
                c,
                |j| Some(j.clone()),
            )?;
            if let Some(e) = entries.error {
                return fail(e);
            }
            if entries
                .items
                .iter()
                .filter(|j| {
                    j.u64("changeTrackingId") == Some(*tracking)
                        && txt(j, &["item", "path"]) == Some(&comment.path)
                        && j.str("originalPath").unwrap_or(&comment.path) == comment.anchor.old_path
                })
                .count()
                != 1
            {
                return fail(ForgeFailure::PullChanged);
            }
        }
        Ok(())
    }
    pub(super) fn other_discussions(
        &self,
        path: &str,
        c: &CancellationToken,
    ) -> Result<(
        ForgePullCollection<ForgePullComment>,
        ForgePullCollection<ForgePullComment>,
    )> {
        let p = self.config.provider();
        let (endpoint, key) = match p {
            ForgeProvider::Bitbucket => (format!("{path}/comments?pagelen=50&page=1"), "values"),
            ForgeProvider::BitbucketServer => {
                (format!("{path}/activities?limit=50&start=0"), "values")
            }
            _ => (format!("{path}/threads"), "value"),
        };
        let page = self.first_page(&endpoint, Some(key), "read_pull_discussion", c, |j| {
            Some(j.clone())
        })?;
        let mut general = ForgePullCollection {
            items: vec![],
            limited: page.limited,
            error: page.error,
        };
        let mut inline = ForgePullCollection {
            items: vec![],
            limited: page.limited,
            error: page.error,
        };
        for entry in page.items {
            c.check()?;
            let (rows, location) = if p.is_azure() {
                (
                    need(entry.get("comments").and_then(Json::array))?
                        .iter()
                        .collect::<Vec<_>>(),
                    azure_location(&entry),
                )
            } else if p == ForgeProvider::BitbucketServer {
                (
                    entry.get("comment").into_iter().collect(),
                    entry.get("commentAnchor").and_then(server_location),
                )
            } else {
                (vec![&entry], cloud_location(&entry))
            };
            for row in rows {
                if row.bool("isDeleted") == Some(true) || row.bool("deleted") == Some(true) {
                    continue;
                }
                let result = if location.is_some() {
                    &mut inline
                } else {
                    &mut general
                };
                if result.items.len() >= 200 {
                    result.limited = true;
                    break;
                }
                let mut n = need(note(row, p))?;
                if let Some((name, line)) = &location {
                    n.path = Some(bounded(name, 8192));
                    n.line = Some(*line);
                }
                result.items.push(n);
            }
        }
        Ok((general, inline))
    }
    pub(super) fn submit_other_inline(
        &self,
        path: &str,
        pv: &ForgePullReviewPreview,
        c: &CancellationToken,
    ) -> Result<ForgePullReview> {
        let p = self.config.provider();
        let comment = need(pv.comments.first())?;
        let body = inline_payload(p, comment)?;
        let endpoint = self.versioned(&format!(
            "{path}/{}",
            if p.is_azure() { "threads" } else { "comments" }
        ));
        let j = self
            .request(
                SUBMIT,
                "publish_pull_inline",
                "POST",
                &endpoint,
                Some(&body),
                true,
                c,
            )
            .map_err(review_error)?
            .0;
        // A server may have accepted the write even if its response is malformed.
        // Cancellation/read failure after POST must preserve the user's draft.
        let confirmed = (|| -> Result<ForgePullComment> {
            let row = if p.is_azure() {
                need(
                    j.get("comments")
                        .and_then(Json::array)
                        .filter(|a| a.len() == 1)
                        .and_then(|a| a.first()),
                )?
            } else {
                &j
            };
            let actor = row
                .get(if p == ForgeProvider::Bitbucket {
                    "user"
                } else {
                    "author"
                })
                .and_then(|a| identity(a, p));
            let n = need(note(row, p))?;
            if n.id == 0
                || actor.as_deref() != self.account_identity()
                || raw_body(row, p) != Some(&comment.body)
            {
                return fail(ForgeFailure::PullReviewUnknown);
            }
            let mut matches = inline_matches(p, &j, comment);
            if p == ForgeProvider::BitbucketServer && j.get("anchor").is_none() {
                let activities = self.first_page(
                    &format!("{path}/activities?limit=50&start=0"),
                    Some("values"),
                    "verify_pull_inline",
                    c,
                    |j| Some(j.clone()),
                )?;
                matches = activities.error.is_none()
                    && activities
                        .items
                        .iter()
                        .filter(|a| {
                            a.get("comment").and_then(|r| r.u64("id")) == Some(n.id)
                                && a.get("commentAnchor")
                                    .is_some_and(|anchor| server_matches(anchor, comment))
                        })
                        .count()
                        == 1;
            }
            if !matches {
                return fail(ForgeFailure::PullReviewUnknown);
            }
            // Cloud has no commit condition on comments. Also catch a changed
            // server PR/iteration after POST; never describe a racing write as fresh.
            let current = self.current_pull(SUBMIT, &pv.repository, pv.summary.number, c)?;
            if !same_revision(&pv.summary, &current) {
                return fail(ForgeFailure::PullReviewUnknown);
            }
            self.validate_other_inline_snapshot(
                &pv.repository,
                pv.summary.number,
                &pv.comments,
                c,
            )?;
            Ok(n)
        })()
        .map_err(|_| Error::Forge(ForgeFailure::PullReviewUnknown))?;
        Ok(ForgePullReview {
            id: confirmed.id,
            author: confirmed.author,
            body: confirmed.body,
            state: "COMMENTED".into(),
            commit: Some(pv.summary.head.clone()),
            date: confirmed.date,
        })
    }
}

/// The text of an Azure Items read when it is a plain text blob. Azure leaves false
/// flags out of its JSON: Microsoft's samples show text files whose `contentMetadata`
/// has only encoding, contentType, fileName and extension, and files without
/// `isFolder`. So only an explicit `isBinary: true` marks a binary file; the content
/// must still be bounded and free of NUL.
fn azure_text(j: &Json) -> Option<String> {
    if j.str("gitObjectType") != Some("blob")
        || j.bool("isFolder") == Some(true)
        || j.bool("isSymLink") == Some(true)
        || at(j, &["contentMetadata"]).and_then(|m| m.bool("isBinary")) == Some(true)
    {
        return None;
    }
    j.str("content").filter(|s| s.len() <= 128 * 1024 && !s.contains('\0')).map(str::to_owned)
}
fn raw_body(j: &Json, p: ForgeProvider) -> Option<&str> {
    match p {
        ForgeProvider::Bitbucket => txt(j, &["content", "raw"]),
        ForgeProvider::BitbucketServer => j.str("text"),
        _ => j.str("content"),
    }
}
fn cloud_location(j: &Json) -> Option<(&str, u64)> {
    let a = j.get("inline")?;
    Some((
        a.str("path")?,
        positive(a.u64("to").or_else(|| a.u64("from")))?,
    ))
}
fn server_location(j: &Json) -> Option<(&str, u64)> {
    Some((server_path(j, "path")?, positive(j.u64("line"))?))
}
fn azure_location(j: &Json) -> Option<(&str, u64)> {
    let a = j.get("threadContext")?;
    Some((
        a.str("filePath")?,
        positive(
            at(a, &["rightFileStart"])
                .or_else(|| at(a, &["leftFileStart"]))
                .and_then(|l| l.u64("line")),
        )?,
    ))
}
fn inline_payload(p: ForgeProvider, c: &ForgeInlineComment) -> Result<String> {
    let side = if c.side == ForgeDiffSide::Left {
        "left"
    } else {
        "right"
    };
    match p {
        ForgeProvider::Bitbucket => Ok(format!(
            "{{\"content\":{{\"raw\":{}}},\"inline\":{{\"path\":{},\"{}\":{}}}}}",
            quote(&c.body),
            quote(&c.path),
            if c.side == ForgeDiffSide::Left {
                "from"
            } else {
                "to"
            },
            c.line
        )),
        ForgeProvider::BitbucketServer => {
            let Some(FileAnchor::BitbucketServer { from, to }) = &c.anchor.provider else {
                return fail(ForgeFailure::PullReviewStale);
            };
            Ok(format!(
                "{{\"text\":{},\"anchor\":{{\"diffType\":\"EFFECTIVE\",\"fromHash\":{},\"toHash\":{},\"path\":{},\"srcPath\":{},\"line\":{},\"lineType\":{},\"fileType\":{}}}}}",
                quote(&c.body),
                quote(from),
                quote(to),
                quote(&c.path),
                quote(&c.anchor.old_path),
                c.line,
                quote(line_type(c)),
                quote(if c.side == ForgeDiffSide::Left {
                    "FROM"
                } else {
                    "TO"
                })
            ))
        }
        _ if p.is_azure() => {
            let Some(FileAnchor::Azure {
                iteration,
                tracking,
            }) = &c.anchor.provider
            else {
                return fail(ForgeFailure::PullReviewStale);
            };
            Ok(format!(
                "{{\"comments\":[{{\"parentCommentId\":0,\"content\":{},\"commentType\":1}}],\"status\":1,\"threadContext\":{{\"filePath\":{},\"{side}FileStart\":{{\"line\":{},\"offset\":1}},\"{side}FileEnd\":{{\"line\":{},\"offset\":1}}}},\"pullRequestThreadContext\":{{\"changeTrackingId\":{tracking},\"iterationContext\":{{\"firstComparingIteration\":0,\"secondComparingIteration\":{}}}}}}}",
                quote(&c.body),
                quote(&c.path),
                c.line,
                c.line,
                iteration.id
            ))
        }
        _ => fail(ForgeFailure::Unsupported),
    }
}
fn line_type(c: &ForgeInlineComment) -> &'static str {
    if c.anchor.old.is_none() {
        "ADDED"
    } else if c.anchor.new.is_none() {
        "REMOVED"
    } else {
        "CONTEXT"
    }
}
fn server_matches(a: &Json, c: &ForgeInlineComment) -> bool {
    let Some(FileAnchor::BitbucketServer { from, to }) = &c.anchor.provider else {
        return false;
    };
    server_path(a, "path") == Some(&c.path)
        && (c.anchor.old_path == c.path || server_path(a, "srcPath") == Some(&c.anchor.old_path))
        && a.u64("line") == Some(c.line)
        && a.str("lineType") == Some(line_type(c))
        && a.str("fileType")
            == Some(if c.side == ForgeDiffSide::Left {
                "FROM"
            } else {
                "TO"
            })
        && a.str("diffType") == Some("EFFECTIVE")
        && a.str("fromHash") == Some(from)
        && a.str("toHash") == Some(to)
}
fn inline_matches(p: ForgeProvider, j: &Json, c: &ForgeInlineComment) -> bool {
    match p {
        ForgeProvider::Bitbucket => {
            j.get("inline").is_some_and(|a| {
                a.str("path") == Some(&c.path)
                    && a.u64(if c.side == ForgeDiffSide::Left {
                        "from"
                    } else {
                        "to"
                    }) == Some(c.line)
                    && a.u64(if c.side == ForgeDiffSide::Left {
                        "to"
                    } else {
                        "from"
                    })
                    .is_none()
            }) && j.bool("pending") != Some(true)
        }
        ForgeProvider::BitbucketServer => j.get("anchor").is_some_and(|a| server_matches(a, c)),
        _ if p.is_azure() => {
            let Some(FileAnchor::Azure {
                iteration,
                tracking,
            }) = &c.anchor.provider
            else {
                return false;
            };
            let Some(a) = j.get("threadContext") else {
                return false;
            };
            let (side, other) = if c.side == ForgeDiffSide::Left {
                ("left", "right")
            } else {
                ("right", "left")
            };
            let point = |name: &str| {
                a.get(name)
                    .is_some_and(|v| v.u64("line") == Some(c.line) && v.u64("offset") == Some(1))
            };
            positive(j.u64("id")).is_some()
                && a.str("filePath") == Some(&c.path)
                && point(&format!("{side}FileStart"))
                && point(&format!("{side}FileEnd"))
                && a.get(&format!("{other}FileStart"))
                    .is_none_or(|v| matches!(v, Json::Null))
                && a.get(&format!("{other}FileEnd"))
                    .is_none_or(|v| matches!(v, Json::Null))
                && at(j, &["pullRequestThreadContext"]).is_some_and(|ctx| {
                    ctx.u64("changeTrackingId") == Some(*tracking)
                        && ctx.get("iterationContext").is_some_and(|v| {
                            v.u64("firstComparingIteration") == Some(0)
                                && v.u64("secondComparingIteration") == Some(iteration.id)
                        })
                })
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn json(s: &str) -> Json {
        crate::hosting::json::parse(s.as_bytes()).unwrap()
    }
    #[test]
    fn azure_text_files_need_no_is_binary_flag() {
        // Microsoft's Items sample: a text file's contentMetadata has no isBinary key.
        let text = json(r#"{"gitObjectType":"blob","contentMetadata":{"encoding":65001,"contentType":"application/xml","fileName":"Web.config","extension":"config"},"content":"<a/>\n"}"#);
        assert_eq!(azure_text(&text).as_deref(), Some("<a/>\n"));
        assert_eq!(azure_text(&json(r#"{"gitObjectType":"blob","contentMetadata":{"isBinary":false},"content":"x"}"#)).as_deref(), Some("x"));
        assert_eq!(azure_text(&json(r#"{"gitObjectType":"blob","content":"x"}"#)).as_deref(), Some("x"));
        for refused in [
            r#"{"gitObjectType":"blob","contentMetadata":{"isBinary":true},"content":"x"}"#,
            r#"{"gitObjectType":"tree","isFolder":true,"content":"x"}"#,
            r#"{"gitObjectType":"blob","isSymLink":true,"content":"x"}"#,
            r#"{"gitObjectType":"blob","content":"a\u0000b"}"#,
            r#"{"gitObjectType":"blob"}"#,
        ] {
            assert_eq!(azure_text(&json(refused)), None, "{refused}");
        }
        let large = format!(r#"{{"gitObjectType":"blob","content":"{}"}}"#, "a".repeat(128 * 1024 + 1));
        assert_eq!(azure_text(&json(&large)), None);
    }
    fn files() -> Vec<ForgePullFile> {
        vec![file(&json(r#"{"path":{"toString":"資料/新.rs"},"srcPath":{"toString":"資料/旧.rs"},"type":"MOVE"}"#),ForgeProvider::BitbucketServer).unwrap()]
    }
    fn diff() -> String {
        format!(
            r#"{{"fromHash":"{}","toHash":"{}","diffs":[{{"source":{{"toString":"資料/旧.rs"}},"destination":{{"toString":"資料/新.rs"}},"hunks":[{{"sourceLine":1,"sourceSpan":2,"destinationLine":1,"destinationSpan":2,"segments":[{{"type":"CONTEXT","lines":[{{"source":1,"destination":1,"line":"same"}}]}},{{"type":"REMOVED","lines":[{{"source":2,"destination":2,"line":"old"}}]}},{{"type":"ADDED","lines":[{{"source":3,"destination":2,"line":"new"}}]}}]}}]}}]}}"#,
            "b".repeat(40),
            "a".repeat(40)
        )
    }
    #[test]
    fn server_diff_validates_hashes_paths_positions_and_truncation_before_enabling() {
        let c = CancellationToken::new();
        let head = "a".repeat(40);
        let source = diff();
        let mut f = files();
        assert!(!attach_server_diff(&mut f, &json(&source), &head, &c).unwrap());
        assert_eq!(f[0].lines[2].old, Some(2));
        assert_eq!(f[0].lines[3].new, Some(2));
        assert!(f[0].anchor.is_some());
        for bad in [
            source.replace("資料/旧.rs", "other.rs"),
            source.replace("\"source\":2", "\"source\":999"),
            source.replace("\"sourceSpan\":2", "\"sourceSpan\":3"),
            source.replace(&head, &"c".repeat(40)),
        ] {
            let mut f = files();
            assert!(attach_server_diff(&mut f, &json(&bad), &head, &c).is_err());
            assert!(f[0].lines.is_empty());
            assert!(f[0].anchor.is_none());
        }
        for needle in [
            "\"diffs\":[{",
            "\"hunks\":[{",
            "\"segments\":[{",
            "\"lines\":[{",
        ] {
            let mut f = files();
            let bad = source.replace(needle, &format!("{needle}\"truncated\":true,"));
            assert!(attach_server_diff(&mut f, &json(&bad), &head, &c).unwrap());
            assert!(f[0].lines.is_empty());
        }
        c.cancel();
        assert!(matches!(
            attach_server_diff(&mut files(), &json(&source), &head, &c),
            Err(Error::Cancelled)
        ));
    }
    #[test]
    fn azure_api_paths_allow_one_root_separator_but_never_traversal() {
        for p in [ForgeProvider::Azure, ForgeProvider::AzureServer] {
            assert!(safe_provider_path(p, "/資料/new.rs"));
            for bad in ["relative", "//x", "/../secret", "/a/./b", "/a\0b"] {
                assert!(!safe_provider_path(p, bad));
            }
        }
        assert!(!safe_provider_path(ForgeProvider::Bitbucket, "/root"));
    }
}
