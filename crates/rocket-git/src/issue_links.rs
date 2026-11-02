//! Issue-tracker links in commit messages. Rules live in Git config as
//! `rocket.issuelink.<name>.pattern` and `.url`: the repository's own config is
//! edited by Rocket, and global or system config can supply shared rules that a
//! repository rule of the same name replaces. A link is offered only as a
//! validated https URL; the matched ID is percent-encoded into the template.
mod pattern;

use crate::{local_files, runner::args, *};
use pattern::Pattern;
use std::{ffi::OsString, ops::Range};

pub const MAX_ISSUE_LINK_RULES: usize = 20;
/// Only this much of a message is searched for issue IDs.
pub const ISSUE_LINK_TEXT_LIMIT: usize = 16 * 1024;
const NAME_LIMIT: usize = 32;
const URL_LIMIT: usize = 2048;
const MATCH_BUDGET: usize = 2_000_000;
const KEY_PREFIX: &[u8] = b"rocket.issuelink.";

/// One rule: IDs that `pattern` finds link to `url`, where `{id}` is the whole
/// match and `{1}`…`{9}` are its groups.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IssueLinkRule {
    pub name: String,
    pub pattern: String,
    pub url: String,
}

/// Where an inherited rule comes from, for display.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IssueLinkScope {
    Repository,
    Worktree,
    Global,
    System,
}

/// The rules Rocket can edit (the repository's own config file) and the
/// read-only rules that apply as well. Never contains credentials.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IssueLinkSettings {
    /// Every rule in the repository's own config file, in file order. An
    /// incomplete or invalid one is listed too (missing fields are empty), so
    /// the editor can fix or remove it; it links nothing until it is valid.
    pub local: Vec<IssueLinkRule>,
    /// Rules from other config files (global, system, worktree or included
    /// files) that still apply: a local rule with the same name replaces one.
    pub inherited: Vec<(IssueLinkScope, IssueLinkRule)>,
    /// Incomplete or invalid rules, which are ignored.
    pub problems: Vec<String>,
    // The exact local config bytes guard against edits made since reading.
    config: Vec<u8>,
}

pub struct IssueLinkWriteReport {
    pub result: Result<WriteOutput>,
    pub settings: Result<IssueLinkSettings>,
}

/// One linked issue ID in a message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IssueLink {
    /// Byte range of the ID in the searched text.
    pub range: Range<usize>,
    pub url: String,
    pub rule: String,
}

/// Compiled rules, in priority order: the first rule wins where IDs overlap.
#[derive(Clone, Debug, Default)]
pub struct IssueLinks {
    rules: Vec<(IssueLinkRule, Pattern)>,
}

impl IssueLinkRule {
    pub fn validate(&self) -> Result<()> {
        self.compile().map(drop)
    }

    fn compile(&self) -> Result<Pattern> {
        let invalid = |message: String| Err(Error::InvalidInput(message));
        if self.name.is_empty()
            || self.name.len() > NAME_LIMIT
            || !self
                .name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        {
            return invalid(format!(
                "Rule names use 1–{NAME_LIMIT} letters, digits, - or _ (\"{}\")",
                self.name
            ));
        }
        let pattern = Pattern::new(&self.pattern)
            .or_else(|reason| invalid(format!("Rule {}: pattern: {reason}", self.name)))?;
        if let Err(reason) = check_template(&self.url, pattern.groups()) {
            return invalid(format!("Rule {}: URL: {reason}", self.name));
        }
        Ok(pattern)
    }
}

/// The template must be an https URL with the placeholders after its host, and
/// a sample ID must produce a valid link.
fn check_template(template: &str, groups: usize) -> std::result::Result<(), String> {
    let Some(rest) = template.strip_prefix("https://") else {
        return Err("must start with https://".into());
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    if authority.contains('{') {
        return Err("the host cannot contain a placeholder".into());
    }
    let mut placeholders = 0;
    let mut tail = template;
    while let Some(open) = tail.find('{') {
        let close = tail[open..].find('}').ok_or("a { has no closing }")? + open;
        match &tail[open + 1..close] {
            "id" => {}
            n if n.len() == 1 && n.as_bytes()[0].is_ascii_digit() && n != "0" => {
                if usize::from(n.as_bytes()[0] - b'0') > groups {
                    return Err(format!("{{{n}}} needs a pattern with {n} group(s)"));
                }
            }
            other => {
                return Err(format!(
                    "unknown placeholder {{{other}}}; use {{id}} or {{1}}–{{9}}"
                ));
            }
        }
        placeholders += 1;
        tail = &tail[close + 1..];
    }
    if placeholders == 0 {
        return Err("must contain {id} or a group placeholder such as {1}".into());
    }
    let sample = fill(template, "ID-1", &[Some("1"); 9]);
    if !https_url(&sample) {
        return Err(
            "must be a plain https URL: a host name, then letters, digits and - . _ ~ / ? # = & % : @ + ! $ * ;"
                .into(),
        );
    }
    Ok(())
}

/// Replace `{id}` and `{n}` with percent-encoded values.
fn fill(template: &str, id: &str, groups: &[Option<&str>]) -> String {
    let mut out = String::with_capacity(template.len() + id.len());
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let close = rest[open..].find('}').map_or(rest.len(), |c| c + open);
        let value = match &rest[open + 1..close.min(rest.len())] {
            "id" => Some(id),
            n if n.len() == 1 && n.as_bytes()[0].is_ascii_digit() => n.as_bytes()[0]
                .checked_sub(b'1')
                .and_then(|i| groups.get(usize::from(i)).copied().flatten()),
            _ => None,
        };
        encode(value.unwrap_or_default(), &mut out);
        rest = rest.get(close + 1..).unwrap_or_default();
    }
    out.push_str(rest);
    out
}

fn encode(value: &str, out: &mut String) {
    for b in value.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
}

/// A conservative https URL check before anything is handed to the browser:
/// a DNS name or IPv4 host with an optional port, no user information, and a
/// path, query and fragment from a fixed ASCII set with valid `%XX` escapes.
pub fn https_url(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("https://") else {
        return false;
    };
    if url.len() > URL_LIMIT {
        return false;
    }
    let split = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let (authority, tail) = rest.split_at(split);
    let (host, port) = match authority.rsplit_once(':') {
        Some((host, port)) => (host, Some(port)),
        None => (authority, None),
    };
    let label = |l: &str| {
        !l.is_empty()
            && l.len() <= 63
            && !l.starts_with('-')
            && !l.ends_with('-')
            && l.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
    };
    if host.is_empty() || host.len() > 253 || !host.split('.').all(label) {
        return false;
    }
    if port.is_some_and(|p| p.is_empty() || p.len() > 5 || !p.bytes().all(|b| b.is_ascii_digit())) {
        return false;
    }
    let bytes = tail.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'%' {
            if !(bytes.get(i + 1).is_some_and(u8::is_ascii_hexdigit)
                && bytes.get(i + 2).is_some_and(u8::is_ascii_hexdigit))
            {
                return false;
            }
            i += 3;
            continue;
        }
        if !(b.is_ascii_alphanumeric() || b"-._~/?#=&:@+!$*;".contains(&b)) {
            return false;
        }
        i += 1;
    }
    true
}

impl IssueLinks {
    /// Compile valid rules; invalid ones are left out (settings list them).
    pub fn new<'a>(rules: impl IntoIterator<Item = &'a IssueLinkRule>) -> Self {
        Self {
            rules: rules
                .into_iter()
                .filter_map(|r| r.compile().ok().map(|p| (r.clone(), p)))
                .take(2 * MAX_ISSUE_LINK_RULES)
                .collect(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    /// How many valid rules apply.
    pub fn len(&self) -> usize {
        self.rules.len()
    }

    /// Issue IDs in the first [`ISSUE_LINK_TEXT_LIMIT`] bytes of `text`, in
    /// order. Where matches of different rules overlap, the earlier rule wins.
    pub fn find(&self, text: &str) -> Vec<IssueLink> {
        let mut end = text.len().min(ISSUE_LINK_TEXT_LIMIT);
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        let text = &text[..end];
        let mut links: Vec<IssueLink> = Vec::new();
        for (rule, pattern) in &self.rules {
            for captures in pattern.find_all(text, MATCH_BUDGET) {
                let (start, stop) = captures[0].expect("a match has a whole range");
                if links
                    .iter()
                    .any(|l| l.range.start < stop && start < l.range.end)
                {
                    continue;
                }
                let groups: Vec<Option<&str>> = captures[1..]
                    .iter()
                    .map(|g| g.map(|(a, b)| &text[a..b]))
                    .collect();
                let url = fill(&rule.url, &text[start..stop], &groups);
                if https_url(&url) {
                    links.push(IssueLink {
                        range: start..stop,
                        url,
                        rule: rule.name.clone(),
                    });
                }
            }
        }
        links.sort_by_key(|l| l.range.start);
        links
    }
}

impl IssueLinkSettings {
    /// Repository rules first, then inherited ones.
    pub fn links(&self) -> IssueLinks {
        IssueLinks::new(
            self.local
                .iter()
                .chain(self.inherited.iter().map(|(_, rule)| rule)),
        )
    }
}

/// A rule name in a key, `rocket.issuelink.<name>.<field>`, where Git
/// lower-cases the field.
fn split_key(key: &[u8]) -> Option<(&[u8], &[u8])> {
    let rest = key.strip_prefix(KEY_PREFIX)?;
    let dot = rest.iter().rposition(|b| *b == b'.')?;
    Some((&rest[..dot], &rest[dot + 1..]))
}

/// One config value: its scope (None for the repository's own file), key and value.
type ConfigEntry = (Option<IssueLinkScope>, Vec<u8>, Vec<u8>);

#[derive(Default)]
struct Partial {
    pattern: Option<String>,
    url: Option<String>,
}

/// Collect `(scope, name) -> fields` in config order; later values win.
fn collect(
    entries: Vec<ConfigEntry>,
    problems: &mut Vec<String>,
) -> Vec<(IssueLinkScope, String, Partial)> {
    let mut rules: Vec<(IssueLinkScope, String, Partial)> = Vec::new();
    for (scope, key, value) in entries {
        let Some((name, field)) = split_key(&key) else {
            continue;
        };
        let (Ok(name), Ok(value)) = (String::from_utf8(name.to_vec()), String::from_utf8(value))
        else {
            problems.push("A rule with non-UTF-8 text is ignored".into());
            continue;
        };
        let scope = scope.unwrap_or(IssueLinkScope::Repository);
        let index = match rules.iter().position(|(s, n, _)| *s == scope && *n == name) {
            Some(index) => index,
            None => {
                rules.push((scope, name, Partial::default()));
                rules.len() - 1
            }
        };
        match field {
            b"pattern" => rules[index].2.pattern = Some(value),
            b"url" => rules[index].2.url = Some(value),
            _ => {}
        }
    }
    rules
}

/// A rule with both fields, or `None` after noting the problem.
fn complete(
    scope: IssueLinkScope,
    name: String,
    partial: Partial,
    problems: &mut Vec<String>,
) -> Option<IssueLinkRule> {
    let where_ = match scope {
        IssueLinkScope::Repository => "repository",
        IssueLinkScope::Worktree => "worktree",
        IssueLinkScope::Global => "global",
        IssueLinkScope::System => "system",
    };
    let (Some(pattern), Some(url)) = (partial.pattern, partial.url) else {
        problems.push(format!(
            "Rule {name} in {where_} config needs both pattern and url; ignored"
        ));
        return None;
    };
    let rule = IssueLinkRule { name, pattern, url };
    if let Err(error) = rule.validate() {
        problems.push(format!("{error} ({where_} config); ignored"));
        return None;
    }
    Some(rule)
}

impl Repository {
    /// Read the rules: the repository's own config file (editable) and every
    /// other config file Git reads, with their scope.
    pub fn issue_link_settings(&self, cancel: &CancellationToken) -> Result<IssueLinkSettings> {
        self.runner
            .step("issue-links-read", "read_issue_links", || {
                let path = self.info().common_dir.join("config");
                let before = local_files::read_config(&path)?;
                let own = self.issue_link_values(true, cancel)?;
                let all = self.issue_link_values(false, cancel)?;
                if local_files::read_config(&path)? != before {
                    return Err(Error::InvalidInput(
                        "Repository settings changed while being read; reopen Repository Settings"
                            .into(),
                    ));
                }
                let mut problems = Vec::new();
                let mut local = Vec::new();
                for (_, name, partial) in collect(own, &mut problems) {
                    let rule = IssueLinkRule {
                        name,
                        pattern: partial.pattern.unwrap_or_default(),
                        url: partial.url.unwrap_or_default(),
                    };
                    if let Err(error) = rule.validate() {
                        problems.push(format!("{error}; it links nothing until fixed"));
                    }
                    local.push(rule);
                }
                // The own file's rules are listed above. Other values that Git
                // attributes to the repository come from included files.
                let mut ignored = Vec::new();
                let mut inherited: Vec<(IssueLinkScope, IssueLinkRule)> = Vec::new();
                for (scope, name, partial) in collect(all, &mut ignored) {
                    if local.iter().any(|r| r.name == name) {
                        continue;
                    }
                    if let Some(rule) = complete(scope, name, partial, &mut problems) {
                        // Git lists system, global, local, then worktree values:
                        // the later definition of a name replaces the earlier one.
                        inherited.retain(|(_, r)| r.name != rule.name);
                        inherited.push((scope, rule));
                    }
                }
                Ok(IssueLinkSettings {
                    local,
                    inherited,
                    problems,
                    config: before,
                })
            })
    }

    fn issue_link_values(&self, own: bool, cancel: &CancellationToken) -> Result<Vec<ConfigEntry>> {
        let mut arguments = args(&["config"]);
        arguments.extend(args(if own {
            &["--local", "--no-includes"][..]
        } else {
            &["--show-scope"][..]
        }));
        arguments.extend(args(&["--null", "--get-regexp", r"^rocket\.issuelink\."]));
        let output = self.runner.run(self.cwd(), &arguments, vec![], cancel)?;
        if output.status.code() == Some(1) && output.stdout.is_empty() {
            return Ok(vec![]);
        }
        let bytes = output.success("read issue link rules")?;
        let mut records = bytes.split(|b| *b == 0).filter(|r| !r.is_empty());
        let mut entries = Vec::new();
        while let Some(record) = records.next() {
            let (scope, record) = if own {
                (None, record)
            } else {
                let scope = match record {
                    b"local" | b"command" => IssueLinkScope::Repository,
                    b"worktree" => IssueLinkScope::Worktree,
                    b"global" => IssueLinkScope::Global,
                    b"system" => IssueLinkScope::System,
                    _ => return Err(Error::parse("issue-links", "unknown config scope")),
                };
                let record = records
                    .next()
                    .ok_or_else(|| Error::parse("issue-links", "missing config entry"))?;
                (Some(scope), record)
            };
            let separator = record
                .iter()
                .position(|b| *b == b'\n')
                .ok_or_else(|| Error::parse("issue-links", "missing config value"))?;
            entries.push((
                scope,
                record[..separator].to_vec(),
                record[separator + 1..].to_vec(),
            ));
        }
        Ok(entries)
    }

    /// Replace the repository's own rules through Git's config.lock. Inherited
    /// rules, includes and other config files are never edited.
    pub fn set_issue_links(
        &self,
        expected: &IssueLinkSettings,
        rules: &[IssueLinkRule],
        cancel: &CancellationToken,
    ) -> Result<IssueLinkWriteReport> {
        let _permit = self.acquire_write(cancel)?;
        self.runner
            .step("issue-links-save", "validate_issue_links", || {
                if rules.len() > MAX_ISSUE_LINK_RULES {
                    return Err(Error::InvalidInput(format!(
                        "At most {MAX_ISSUE_LINK_RULES} rules"
                    )));
                }
                for (i, rule) in rules.iter().enumerate() {
                    rule.validate()?;
                    if rules[..i]
                        .iter()
                        .any(|r| r.name.eq_ignore_ascii_case(&rule.name))
                    {
                        return Err(Error::InvalidInput(format!(
                            "Two rules are named {}",
                            rule.name
                        )));
                    }
                }
                Ok(())
            })?;
        let result = self.runner.observe("write_result", "issue-links-save", || {
            let mut edits: Vec<Vec<OsString>> = Vec::new();
            for rule in &expected.local {
                edits.push(args(&["--remove-section", &format!("rocket.issuelink.{}", rule.name)]));
            }
            for rule in rules {
                for (field, value) in [("pattern", &rule.pattern), ("url", &rule.url)] {
                    edits.push(args(&[
                        "--add",
                        &format!("rocket.issuelink.{}.{field}", rule.name),
                        value,
                    ]));
                }
            }
            self.rewrite_local_config(
                &expected.config,
                || {
                    if self.issue_link_settings(cancel)? != *expected {
                        return Err(Error::InvalidInput(
                            "Issue link rules changed since this dialog opened; reopen it before saving"
                                .into(),
                        ));
                    }
                    Ok(())
                },
                &edits,
                cancel,
            )
        });
        let settings = self.runner.observe("refresh", "issue-links-read", || {
            self.issue_link_settings(&CancellationToken::new())
        });
        Ok(IssueLinkWriteReport { result, settings })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn rule(name: &str, pattern: &str, url: &str) -> IssueLinkRule {
        IssueLinkRule {
            name: name.into(),
            pattern: pattern.into(),
            url: url.into(),
        }
    }

    #[test]
    fn links_fill_encoded_ids_and_earlier_rules_win() {
        let links = IssueLinks::new(&[
            rule(
                "jira",
                r"\b[A-Z]+-\d+\b",
                "https://jira.example.com/browse/{id}",
            ),
            rule(
                "redmine",
                r"#(\d+)",
                "https://redmine.example.com/issues/{1}?from=rocket",
            ),
            rule("all", r"[A-Z]+-\d+", "https://other.example.com/{id}"),
        ]);
        let text = "PROJ-12 fixes #7 (see 漢字 OPS-3)";
        let found = links.find(text);
        let urls: Vec<_> = found.iter().map(|l| l.url.as_str()).collect();
        assert_eq!(
            urls,
            [
                "https://jira.example.com/browse/PROJ-12",
                "https://redmine.example.com/issues/7?from=rocket",
                "https://jira.example.com/browse/OPS-3",
            ]
        );
        assert_eq!(&text[found[1].range.clone()], "#7");
        assert_eq!(found[2].rule, "jira");
        // The whole match is encoded: '#' and non-ASCII become %XX.
        let links = IssueLinks::new(&[rule("x", r"#\S+", "https://h.example/{id}")]);
        assert_eq!(links.find("#é/1")[0].url, "https://h.example/%23%C3%A9%2F1");
    }

    #[test]
    fn templates_and_urls_are_checked() {
        for (url, groups, ok) in [
            ("https://jira.example.com/browse/{id}", 0, true),
            ("https://example.com:8443/issues/{1}#top", 1, true),
            ("http://example.com/{id}", 0, false),
            ("https://{id}.example.com/", 0, false),
            ("https://example.com/browse", 0, false),
            ("https://example.com/{2}", 1, false),
            ("https://example.com/{name}", 0, false),
            ("https://user@example.com/{id}", 0, false),
            ("https://example.com/a b/{id}", 0, false),
            ("https://example.com/\"{id}", 0, false),
            ("https://exa_mple.com/{id}", 0, false),
            ("https://example.com/{id", 0, false),
        ] {
            assert_eq!(check_template(url, groups).is_ok(), ok, "{url}");
        }
        assert!(https_url("https://a.example/x?y=1&z=%2F#f"));
        assert!(!https_url("https://a.example/%zz"));
        assert!(!https_url("https://a.example/x,y"));
        assert!(!https_url("https://-a.example/"));
        assert!(!https_url(&format!(
            "https://a.example/{}",
            "x".repeat(URL_LIMIT)
        )));
        for name in ["", "two words", "é", &"x".repeat(NAME_LIMIT + 1)] {
            assert!(
                rule(name, "a", "https://x.example/{id}")
                    .validate()
                    .is_err()
            );
        }
    }

    #[test]
    fn long_messages_are_searched_up_to_the_limit_on_a_char_boundary() {
        let links = IssueLinks::new(&[rule("j", r"X-\d", "https://x.example/{id}")]);
        // The limit falls inside the "é" after X-1 (bytes 16383..16385).
        let mut text = "é".repeat(ISSUE_LINK_TEXT_LIMIT / 2 - 2);
        text.push_str("X-1é X-2");
        let found = links.find(&text);
        assert_eq!(found.len(), 1);
        assert_eq!(&text[found[0].range.clone()], "X-1");
    }
}
