//! One issue's text and its comments, read whole so work can start from it (a new
//! agent session made from an issue). Bounded like pull request details: at most 200
//! comments and 64 KiB of text each. URLs in responses are data: a next page is only
//! requested after it is checked to stay on the same endpoint, and Azure's never is.
use super::*;
use std::collections::HashSet;

const READ: &str = "hosting-issue-text";
/// Comments read at most, as for each collection of a pull request.
const LIMIT: usize = 200;
/// `Link` pages of 50 comments, at most four.
const PAGES: u32 = 4;
const TITLE: usize = 1024;
const TEXT: usize = 64 * 1024;
const AUTHOR: usize = 256;
const LABEL: usize = 64;
const UNKNOWN: &str = "Unknown author";
/// Ends a description or comment cut at the bound, within the bound.
const CUT: &str = "\n[Truncated; open the issue for the full text.]";
/// The work item fields read. System.TeamProject keeps the read to this project.
const FIELDS: &str = "System.Title,System.Description,System.State,System.CreatedBy,System.CreatedDate,System.TeamProject";

/// One issue (an Azure DevOps work item) with its text and comments, oldest comment first.
#[derive(Clone, Debug)]
pub struct ForgeIssueText {
    pub number: u64,
    pub title: String,
    /// As the host reports it: "open"/"closed" (GitLab "opened"), or Azure's state name.
    pub state: String,
    pub author: String,
    /// Created, as the host gives it.
    pub date: String,
    /// Plain text. Azure's HTML is turned into text (block tags become line breaks, tags
    /// dropped, entities decoded). Other hosts' Markdown stays as written.
    pub body: String,
    /// Web page of the issue: `repository.item_url(ForgeKind::Issues, number)`.
    pub url: String,
    pub comments: Vec<ForgeIssueComment>,
    /// More comments exist than were read (Rocket's bound, like pull requests' 200).
    pub limited: bool,
    /// The host returned a pull request for this number (GitHub's issues endpoint does).
    pub is_pull: bool,
}

/// A comment on an issue, its body as plain as the issue's.
#[derive(Clone, Debug)]
pub struct ForgeIssueComment {
    pub author: String,
    pub date: String,
    pub body: String,
}

impl ForgeSession {
    /// Reads issue `number` (an Azure DevOps work item) and up to 200 of its comments.
    /// Bitbucket Cloud and Server have no issues to read; malformed responses fail as
    /// `InvalidResponse`.
    pub fn issue_text(&self, repo: &ForgeRepository, number: u64, cancel: &CancellationToken) -> Result<ForgeIssueText> {
        self.check_repository(repo)?;
        let p = self.config.provider();
        if matches!(p, ForgeProvider::Bitbucket | ForgeProvider::BitbucketServer) {
            // Cloud retired its issue tracker and Server has no issues API (see Hosting).
            return fail(ForgeFailure::Unsupported);
        }
        if number == 0 || number > i32::MAX as u64 {
            return Err(invalid("Select a valid issue."));
        }
        let mut issue = if p.is_azure() { self.work_item(repo, number, cancel)? } else { self.issue(repo, number, cancel)? };
        issue.url = repo.item_url(ForgeKind::Issues, number);
        Ok(issue)
    }

    /// GitHub, GitHub Enterprise Server, GitLab and Gitea: the issue, then its comments.
    fn issue(&self, repo: &ForgeRepository, n: u64, c: &CancellationToken) -> Result<ForgeIssueText> {
        let p = self.config.provider();
        let path = format!("{}/issues/{n}", self.prefix(repo));
        let j = self.request(READ, "read_issue", "GET", &path, None, false, c)?.0;
        let (id, user, login, text) = if p == ForgeProvider::GitLab {
            ("iid", "author", "username", "description")
        } else {
            ("number", "user", "login", "body")
        };
        if j.u64(id) != Some(n) {
            return fail(ForgeFailure::InvalidResponse);
        }
        let mut issue = ForgeIssueText {
            number: n,
            title: title(j.str("title"))?,
            state: bounded(need(j.str("state"))?, LABEL),
            author: name(j.get(user), login),
            date: bounded(j.str("created_at").unwrap_or_default(), LABEL),
            body: plain(optional(j.get(text))?, TEXT),
            url: String::new(),
            comments: Vec::new(),
            limited: false,
            // GitHub and Gitea answer a pull request's number too, with `pull_request`
            // set; Gitea sends it as null for an issue.
            is_pull: matches!(j.get("pull_request"), Some(Json::Object(_))),
        };
        (issue.comments, issue.limited) = match p {
            ForgeProvider::GitLab => self.linked_comments(
                repo,
                &format!("{path}/notes"),
                "activity_filter=only_comments&sort=asc&order_by=created_at&per_page=50&page=1",
                c,
                |j| {
                    // Notes GitLab writes itself, such as label or assignee changes.
                    if j.bool("system") == Some(true) {
                        return Ok(None);
                    }
                    comment(j, "author", "username").map(Some)
                },
            )?,
            ForgeProvider::Gitea => self.gitea_comments(&format!("{path}/comments"), c)?,
            _ => self.linked_comments(repo, &format!("{path}/comments"), "per_page=50&page=1", c, |j| {
                comment(j, "user", "login").map(Some)
            })?,
        };
        Ok(issue)
    }

    /// Comments over `Link` pages, as a pull request's discussion is read: pages of 50,
    /// at most four, each next page checked to stay on this endpoint before it is asked.
    fn linked_comments(
        &self,
        repo: &ForgeRepository,
        endpoint: &str,
        query: &str,
        c: &CancellationToken,
        mut parse: impl FnMut(&Json) -> Result<Option<ForgeIssueComment>>,
    ) -> Result<(Vec<ForgeIssueComment>, bool)> {
        let filter = ForgeFilter::default();
        let mut comments = Vec::new();
        let mut path = format!("{endpoint}?{query}");
        let mut seen = HashSet::new();
        let mut page = 1;
        loop {
            c.check()?;
            if !seen.insert(path.clone()) {
                return fail(ForgeFailure::InvalidPagination);
            }
            let (j, headers) = self.request(READ, "read_issue_comments", "GET", &path, None, false, c)?;
            // A hostile endpoint must not bypass the advertised bound.
            let rows = need(j.array().filter(|rows| rows.len() <= 100))?;
            for row in rows {
                if let Some(comment) = parse(row)? {
                    if comments.len() == LIMIT {
                        return Ok((comments, true));
                    }
                    comments.push(comment);
                }
            }
            let Some(next) = pagination::next_link(&headers)?.flatten() else {
                return Ok((comments, false));
            };
            let cursor = self.linked_cursor(repo, &filter, page, &path, next)?;
            if page == PAGES || comments.len() == LIMIT {
                return Ok((comments, true));
            }
            path = self
                .continuation_path(repo, &filter, &cursor)?
                .ok_or(Error::Forge(ForgeFailure::InvalidPagination))?
                .to_owned();
            page += 1;
        }
    }

    /// Gitea lists all of an issue's comments in one response, with no page or limit
    /// (`since` and `before` only), and counts them in X-Total-Count.
    fn gitea_comments(&self, path: &str, c: &CancellationToken) -> Result<(Vec<ForgeIssueComment>, bool)> {
        let (j, headers) = self.request(READ, "read_issue_comments", "GET", path, None, false, c)?;
        let rows = need(j.array())?;
        let comments = rows.iter().take(LIMIT).map(|row| comment(row, "user", "login")).collect::<Result<Vec<_>>>()?;
        let total = crate::hosting::header(&headers, "x-total-count").and_then(|v| v.parse::<u64>().ok());
        let limited = rows.len() > LIMIT || total.is_some_and(|total| total > comments.len() as u64);
        Ok((comments, limited))
    }

    /// Azure DevOps: the work item's fields, then one bounded page of its comments.
    fn work_item(&self, repo: &ForgeRepository, n: u64, c: &CancellationToken) -> Result<ForgeIssueText> {
        let project = repo.parts().0;
        let version = self.config.azure_api_version();
        let base = format!("/{}/_apis/wit", encode(project));
        let path = format!("{base}/workitems/{n}?fields={FIELDS}&api-version={version}");
        let j = self.request(READ, "read_issue", "GET", &path, None, false, c)?.0;
        let fields = need(j.get("fields").filter(|f| matches!(f, Json::Object(_))))?;
        if j.u64("id") != Some(n) {
            return fail(ForgeFailure::InvalidResponse);
        }
        // Work item IDs run across a collection's projects. Read only this repository's,
        // as the Azure DevOps Server list keeps to it.
        if !need(fields.str("System.TeamProject"))?.eq_ignore_ascii_case(project) {
            return fail(ForgeFailure::NotFound);
        }
        // Rich text is HTML unless the last editor saved the field as Markdown.
        let markdown = j
            .get("multilineFieldsFormat")
            .and_then(|f| f.str("System.Description"))
            .is_some_and(|f| f.eq_ignore_ascii_case("markdown"));
        let mut item = ForgeIssueText {
            number: n,
            title: title(fields.str("System.Title"))?,
            state: bounded(need(fields.str("System.State"))?, LABEL),
            author: name(fields.get("System.CreatedBy"), "displayName"),
            date: bounded(fields.str("System.CreatedDate").unwrap_or_default(), LABEL),
            body: rich(optional(fields.get("System.Description"))?, markdown),
            url: String::new(),
            comments: Vec::new(),
            limited: false,
            is_pull: false,
        };
        let path = format!("{base}/workItems/{n}/comments?$top={LIMIT}&order=asc&api-version={}", comments_version(version));
        let list = self.request(READ, "read_issue_comments", "GET", &path, None, false, c)?.0;
        let rows = need(list.get("comments").and_then(Json::array).filter(|rows| rows.len() <= LIMIT))?;
        for row in rows {
            if row.get("workItemId").is_some() && row.u64("workItemId") != Some(n) {
                return fail(ForgeFailure::InvalidResponse);
            }
            // The reference names the ID `id`; its examples show `commentId`.
            need(row.u64("id").or_else(|| row.u64("commentId")).filter(|id| *id > 0))?;
            if row.bool("isDeleted") == Some(true) {
                continue;
            }
            let markdown = row.str("format").is_some_and(|f| f.eq_ignore_ascii_case("markdown"));
            item.comments.push(ForgeIssueComment {
                author: name(row.get("createdBy"), "displayName"),
                date: bounded(row.str("createdDate").unwrap_or_default(), LABEL),
                body: rich(optional(row.get("text"))?, markdown),
            });
        }
        // The next page's link and token only say that there is more.
        let more = |key: &str| list.str(key).is_some_and(|v| !v.is_empty());
        item.limited = more("continuationToken")
            || more("nextPage")
            || list.u64("totalCount").is_some_and(|total| total > rows.len() as u64);
        Ok(item)
    }
}

/// Work item comments are a preview resource in every REST version Rocket offers, and
/// Azure refuses one asked for without its preview: Server 2020 (6.0) and 2022 (7.0)
/// serve them as `-preview.3`, 7.1 as `-preview.4`.
fn comments_version(version: &str) -> &'static str {
    match version {
        "6.0" => "6.0-preview.3",
        "7.0" => "7.0-preview.3",
        _ => "7.1-preview.4",
    }
}

fn need<T>(value: Option<T>) -> Result<T> {
    value.ok_or(Error::Forge(ForgeFailure::InvalidResponse))
}

/// Text a host may leave out or send as null, such as an issue without a description.
fn optional(value: Option<&Json>) -> Result<&str> {
    match value {
        None | Some(Json::Null) => Ok(""),
        Some(Json::String(text)) => Ok(text.as_str()),
        Some(_) => fail(ForgeFailure::InvalidResponse),
    }
}

/// A title is required; as in list rows, one past 16 KiB is malformed rather than cut.
fn title(value: Option<&str>) -> Result<String> {
    need(value.filter(|t| !t.is_empty() && t.len() <= 16 * 1024)).map(|t| bounded(t, TITLE))
}

fn name(user: Option<&Json>, key: &str) -> String {
    bounded(user.and_then(|u| u.str(key)).filter(|s| !s.is_empty()).unwrap_or(UNKNOWN), AUTHOR)
}

fn bounded(text: &str, max: usize) -> String {
    crate::display_text::bounded(text, max).text
}

/// A GitHub, GitLab or Gitea comment: a positive ID, its author and its Markdown.
fn comment(j: &Json, user: &str, login: &str) -> Result<ForgeIssueComment> {
    need(j.u64("id").filter(|id| *id > 0))?;
    Ok(ForgeIssueComment {
        author: name(j.get(user), login),
        date: bounded(j.str("created_at").unwrap_or_default(), LABEL),
        body: plain(optional(j.get("body"))?, TEXT),
    })
}

/// Azure DevOps rich text, as Markdown or as HTML.
fn rich(text: &str, markdown: bool) -> String {
    if markdown { plain(text, TEXT) } else { plain(&html_text(text, TEXT), TEXT) }
}

/// Host text as written: line endings become `\n`, and text past `max` bytes is cut
/// and ends with a note, within `max`. Control, bidirectional and other invisible
/// characters stay, so the caller can tell what the host's page doesn't show: whoever
/// shows or sends this text removes or escapes them first. (An agent session's first
/// message leaves them out and lists them, as it does HTML comments.)
fn plain(text: &str, max: usize) -> String {
    let mut out = String::with_capacity(text.len().min(max));
    // The longest part that still leaves room for the note.
    let mut fit = 0;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\r' => {
                chars.next_if_eq(&'\n');
                out.push('\n');
            }
            c => out.push(c),
        }
        if out.len() > max {
            out.truncate(fit);
            out.push_str(CUT);
            return out;
        }
        if out.len() + CUT.len() <= max {
            fit = out.len();
        }
    }
    out
}

/// Azure DevOps keeps rich text as HTML. Read it as the page shows it: block elements
/// and `<br>` end lines, other tags and comments are dropped, character references are
/// decoded, and runs of whitespace collapse as a browser collapses them, except inside
/// `<pre>`. Stops past `max` bytes, which `plain` then cuts.
fn html_text(html: &str, max: usize) -> String {
    let mut out = String::new();
    let (mut pre, mut space, mut tags) = (0usize, false, true);
    let mut rest = html;
    while let Some(c) = rest.chars().next() {
        if out.len() > max {
            break;
        }
        if c == '<' && tags {
            if let Some(after) = rest.strip_prefix("<!--") {
                rest = after.find("-->").map_or("", |end| &after[end + 3..]);
                continue;
            }
            match markup(rest) {
                Markup::Tag { name, closing, rest: after } => {
                    rest = after;
                    match name.as_str() {
                        "br" => {
                            line_break(&mut out, pre > 0);
                            space = false;
                        }
                        "pre" => {
                            end_line(&mut out);
                            space = false;
                            pre = if closing { pre.saturating_sub(1) } else { pre + 1 };
                        }
                        "td" | "th" => space = true,
                        other if block(other) => {
                            end_line(&mut out);
                            space = false;
                        }
                        _ => {}
                    }
                    continue;
                }
                // No tag ends after this: the rest is text, which keeps reading linear.
                Markup::Unterminated => tags = false,
                Markup::Text => {}
            }
        }
        if c == '&'
            && let Some((decoded, after)) = entity(rest)
        {
            rest = after;
            push(&mut out, &mut space, decoded);
            continue;
        }
        rest = &rest[c.len_utf8()..];
        if pre == 0 && matches!(c, ' ' | '\t' | '\n' | '\r' | '\x0c') {
            space = true;
        } else {
            push(&mut out, &mut space, c);
        }
    }
    out.trim().to_owned()
}

/// Text after collapsed whitespace gets one space, unless it starts a line.
fn push(out: &mut String, space: &mut bool, c: char) {
    if std::mem::take(space) && !out.is_empty() && !out.ends_with('\n') {
        out.push(' ');
    }
    out.push(c);
}

/// Ends the current line, unless the text is empty or already at a line's start.
fn end_line(out: &mut String) {
    if !out.is_empty() && !out.ends_with('\n') {
        line_break(out, false);
    }
}

fn line_break(out: &mut String, keep_spaces: bool) {
    if !keep_spaces {
        out.truncate(out.trim_end_matches([' ', '\t']).len());
    }
    out.push('\n');
}

fn block(name: &str) -> bool {
    matches!(
        name,
        "address" | "article" | "aside" | "blockquote" | "caption" | "dd" | "details" | "div" | "dl" | "dt"
            | "fieldset" | "figcaption" | "figure" | "footer" | "form" | "h1" | "h2" | "h3" | "h4" | "h5"
            | "h6" | "header" | "hr" | "li" | "main" | "nav" | "ol" | "p" | "section" | "summary" | "table"
            | "tbody" | "tfoot" | "thead" | "tr" | "ul"
    )
}

enum Markup<'a> {
    /// A `<` that starts no tag, as in `a < b`.
    Text,
    /// A tag that never ends.
    Unterminated,
    /// A tag's lowercase name (empty for `<!…>` and `<?…>`) and the text after it.
    Tag { name: String, closing: bool, rest: &'a str },
}

/// The markup at the start of `html`, which starts with `<`. A tag ends at the first `>`
/// outside a quoted attribute value.
fn markup(html: &str) -> Markup<'_> {
    let inner = &html[1..];
    let (closing, inner) = match inner.strip_prefix('/') {
        Some(inner) => (true, inner),
        None => (false, inner),
    };
    let declaration = inner.starts_with(['!', '?']);
    if !declaration && !inner.starts_with(|c: char| c.is_ascii_alphabetic()) {
        return Markup::Text;
    }
    let name = if declaration { "" } else { &inner[..inner.bytes().take_while(u8::is_ascii_alphanumeric).count()] };
    // A quote opens a value only after `=`, as `title="a > b"`.
    let (mut quote, mut value) = (None, false);
    for (i, b) in inner.bytes().enumerate() {
        if let Some(q) = quote {
            if b == q {
                quote = None;
            }
            continue;
        }
        match b {
            b'>' => return Markup::Tag { name: name.to_ascii_lowercase(), closing, rest: &inner[i + 1..] },
            b'"' | b'\'' if value => quote = Some(b),
            b'=' => {
                value = true;
                continue;
            }
            b if b.is_ascii_whitespace() => continue,
            _ => {}
        }
        value = false;
    }
    Markup::Unterminated
}

/// A character reference at the start of `html`, which starts with `&`: the named ones
/// editors write, and decimal or hexadecimal ones. Others stay as text.
fn entity(html: &str) -> Option<(char, &str)> {
    let body = &html[1..];
    let end = body.bytes().take(16).position(|b| b == b';')?;
    let (name, after) = (&body[..end], &body[end + 1..]);
    let c = match name {
        "amp" => '&',
        "lt" => '<',
        "gt" => '>',
        "quot" => '"',
        "apos" => '\'',
        // Plain text has no use for a non-breaking space.
        "nbsp" => ' ',
        _ => {
            let number = name.strip_prefix('#')?;
            let (digits, radix) = match number.strip_prefix(['x', 'X']) {
                Some(hex) => (hex, 16),
                None => (number, 10),
            };
            if digits.is_empty() || !digits.chars().all(|d| d.is_digit(radix)) {
                return None;
            }
            // As a browser reads them, NUL, surrogates and values past U+10FFFF are U+FFFD.
            char::from_u32(u32::from_str_radix(digits, radix).ok()?).filter(|c| *c != '\0').unwrap_or('\u{fffd}')
        }
    };
    Some((c, after))
}
