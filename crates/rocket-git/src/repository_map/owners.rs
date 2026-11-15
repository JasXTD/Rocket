//! CODEOWNERS rules for the code map's Owner colours, read the way the hosts read them.
//!
//! GitHub reads `.github/CODEOWNERS`, then `CODEOWNERS`, then `docs/CODEOWNERS`. Its patterns
//! follow gitignore without `!` or `[ ]`: a slash at the start or in the middle anchors a
//! pattern to the top, the last matching line wins, and a line without owners leaves its
//! files unowned. GitLab reads `CODEOWNERS`, `docs/CODEOWNERS`, then `.gitlab/CODEOWNERS`.
//! Its `[Section]` headers (names case-insensitive, equal names combined, default owners
//! after the header) each add their owners; inside a section the last match wins, `!`
//! excludes, a pattern without a leading slash matches at any depth, and a trailing `/**`
//! is one level. The file found first is read; its location or a section header decides
//! which host's rules apply.

/// Files over this size are not read; GitHub ignores them too.
pub const CODEOWNERS_BYTES: usize = 3 * 1024 * 1024;
/// Where the file may be, in the order a GitHub repository is searched; GitLab's own
/// folder comes last.
pub const CODEOWNERS_FILES: [&str; 4] = [".github/CODEOWNERS", "CODEOWNERS", "docs/CODEOWNERS", ".gitlab/CODEOWNERS"];
/// Rules kept from one file; past this the rest of the file is skipped.
const RULES: usize = 10_000;

/// One snapshot's matching work, including wildcard backtracking and byte comparisons.
/// Checking inside the matcher prevents one adversarial pattern from monopolizing it.
pub(super) struct MatchWork<'a> {
    remaining: usize,
    cancel: &'a crate::CancellationToken,
}
impl<'a> MatchWork<'a> {
    pub(super) fn new(cancel: &'a crate::CancellationToken) -> Self {
        Self { remaining: 8_000_000, cancel }
    }
    fn step(&mut self) -> Option<()> {
        self.charge(1)
    }
    fn charge(&mut self, amount: usize) -> Option<()> {
        if amount > self.remaining || self.cancel.is_cancelled() {
            self.remaining = 0;
            return None;
        }
        self.remaining -= amount;
        Some(())
    }
}

/// Who owns a file under the rules.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ownership {
    /// Matching was cancelled or exhausted its work budget; no decision is available.
    Limited,
    /// No line matches it.
    Nobody,
    /// The line that decides names no owner.
    Unowned,
    Owners(Vec<String>),
}

#[derive(Clone, Debug, Default)]
pub struct CodeOwners {
    /// Where the rules were read from.
    pub file: Vec<u8>,
    /// GitLab's rules (sections, exclusions, floating paths) instead of GitHub's.
    pub gitlab: bool,
    /// Lines left out: a `!` or `[` pattern on GitHub, a broken section header, owners
    /// that are neither `@name` nor an email address, or lines past the limit.
    pub skipped: usize,
    sections: Vec<Section>,
}

#[derive(Clone, Debug)]
struct Section {
    rules: Vec<Rule>,
}

#[derive(Clone, Debug)]
struct Rule {
    pattern: Pattern,
    owners: std::sync::Arc<[String]>,
    exclude: bool,
}

#[derive(Clone, Debug)]
struct Pattern {
    segments: Vec<Vec<u8>>,
    /// Ends in `/`: it names folders only.
    folder: bool,
}

impl CodeOwners {
    /// Rules from `text`, read from `file`. GitLab's rules apply to `.gitlab/CODEOWNERS`
    /// and to any file with a section header.
    pub fn parse(text: &str, file: &[u8]) -> Self {
        // An editor's byte-order mark is not part of the first pattern.
        let text = text.strip_prefix('\u{feff}').unwrap_or(text);
        let gitlab = file == b".gitlab/CODEOWNERS" || text.lines().any(|l| header(l.trim()).is_some());
        let mut out = Self {
            file: file.to_vec(),
            gitlab,
            skipped: 0,
            sections: vec![Section { rules: Vec::new() }],
        };
        let mut current = 0;
        // The default owners of the header above: each header's own, even when sections
        // of one name combine.
        let mut defaults: std::sync::Arc<[String]> = Vec::new().into();
        let mut section_ids = std::collections::HashMap::from([(String::new(), 0)]);
        let mut headers = 0;
        let mut kept = 0;
        for raw in text.lines() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if kept >= RULES || headers >= RULES {
                out.skipped += 1;
                continue;
            }
            if gitlab && (line.starts_with('[') || line.starts_with("^[")) {
                headers += 1;
                match header(line) {
                    Some((name, rest)) => {
                        let key = name.to_lowercase();
                        defaults = owners(rest, true).unwrap_or_default().into();
                        // Equal names combine into one section.
                        current = *section_ids.entry(key).or_insert_with(|| {
                            out.sections.push(Section { rules: Vec::new() });
                            out.sections.len() - 1
                        });
                    }
                    None => out.skipped += 1,
                }
                continue;
            }
            let (token, rest) = pattern_token(line);
            let exclude = token.starts_with('!');
            if exclude && !gitlab || token.starts_with('[') && !gitlab {
                out.skipped += 1;
                continue;
            }
            let body = if exclude { &token[1..] } else { token.as_str() };
            let Some(pattern) = Pattern::new(body, gitlab) else {
                out.skipped += 1;
                continue;
            };
            let owners = if exclude { Some(Vec::new()) } else { owners(rest, gitlab) };
            let Some(owners) = owners else {
                out.skipped += 1;
                continue;
            };
            // GitLab: a line without owners takes its header's default owners.
            let owners = if gitlab && !exclude && owners.is_empty() { defaults.clone() } else { owners.into() };
            out.sections[current].rules.push(Rule { pattern, owners, exclude });
            kept += 1;
        }
        out
    }

    /// How many rules were kept.
    pub fn rules(&self) -> usize {
        self.sections.iter().map(|s| s.rules.len()).sum()
    }

    /// Who owns the committed file at `path` (a path from the top, without a leading `/`).
    pub fn owners(&self, path: &[u8]) -> Ownership {
        self.owners_bounded(path, &mut MatchWork::new(&crate::CancellationToken::new()))
    }

    pub(super) fn owners_bounded(&self, path: &[u8], work: &mut MatchWork<'_>) -> Ownership {
        self.evaluate(path, work).unwrap_or(Ownership::Limited)
    }

    fn evaluate(&self, path: &[u8], work: &mut MatchWork<'_>) -> Option<Ownership> {
        work.step()?;
        let parts: Vec<&[u8]> = path.split(|b| *b == b'/').filter(|p| !p.is_empty()).collect();
        if parts.is_empty() {
            return Some(Ownership::Nobody);
        }
        if !self.gitlab {
            for r in self.sections[0].rules.iter().rev() {
                work.step()?;
                if r.pattern.matches(&parts, work)? {
                    work.charge(r.owners.iter().map(String::len).sum::<usize>().saturating_add(r.owners.len()))?;
                    return Some(if r.owners.is_empty() { Ownership::Unowned } else { Ownership::Owners(r.owners.to_vec()) });
                }
            }
            return Some(Ownership::Nobody);
        }
        let mut matched = false;
        let mut all: Vec<String> = Vec::new();
        for section in &self.sections {
            work.step()?;
            // An exclusion anywhere in its section keeps that section's owners away.
            let mut excluded = false;
            for r in &section.rules {
                work.step()?;
                if r.exclude && r.pattern.matches(&parts, work)? {
                    excluded = true;
                    break;
                }
            }
            if excluded {
                matched = true;
                continue;
            }
            let mut found = None;
            for r in section.rules.iter().rev() {
                work.step()?;
                if !r.exclude && r.pattern.matches(&parts, work)? {
                    found = Some(r);
                    break;
                }
            }
            let Some(rule) = found else {
                continue;
            };
            matched = true;
            for name in rule.owners.iter() {
                work.charge(name.len().saturating_add(1))?;
                let mut present = false;
                for existing in &all {
                    work.step()?;
                    if existing == name { present = true; break; }
                }
                if !present {
                    all.push(name.clone());
                }
            }
        }
        Some(match (matched, all.is_empty()) {
            (false, _) => Ownership::Nobody,
            (true, true) => Ownership::Unowned,
            (true, false) => Ownership::Owners(all),
        })
    }
}

/// A GitLab section header: `[Name]`, `^[Name]`, `[Name][2]`, then default owners.
fn header(line: &str) -> Option<(&str, &str)> {
    let line = line.strip_prefix('^').unwrap_or(line);
    let rest = line.strip_prefix('[')?;
    let end = rest.find(']')?;
    let name = rest[..end].trim();
    if name.is_empty() {
        return None;
    }
    let mut rest = &rest[end + 1..];
    // Required approvals: `[2]`.
    if let Some(count) = rest.strip_prefix('[') {
        let close = count.find(']')?;
        if count[..close].is_empty() || !count[..close].bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        rest = &count[close + 1..];
    }
    if !rest.is_empty() && !rest.starts_with(char::is_whitespace) {
        return None;
    }
    Some((name, rest))
}

/// The pattern at the start of a line, with `\ ` and other backslash escapes undone,
/// and what follows it.
fn pattern_token(line: &str) -> (String, &str) {
    let mut token = String::new();
    let mut chars = line.char_indices();
    while let Some((i, c)) = chars.next() {
        match c {
            '\\' => match chars.next() {
                Some((_, next)) => token.push(next),
                None => token.push('\\'),
            },
            c if c.is_whitespace() => return (token, &line[i..]),
            c => token.push(c),
        }
    }
    (token, "")
}

/// Owners after a pattern. GitHub stops at a `#` comment and refuses a line with an
/// owner that is neither `@name` nor an email address. GitLab has no inline comments
/// and reads the owners it finds anywhere on the line.
fn owners(rest: &str, gitlab: bool) -> Option<Vec<String>> {
    let mut out: Vec<String> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for word in rest.split_whitespace() {
        if !gitlab && word.starts_with('#') {
            break;
        }
        let owner = word.starts_with('@') && word.len() > 1 || is_email(word);
        if !owner {
            if gitlab {
                continue;
            }
            return None;
        }
        if seen.insert(word) {
            out.push(word.to_string());
        }
    }
    Some(out)
}

fn is_email(word: &str) -> bool {
    let Some((user, host)) = word.split_once('@') else { return false };
    !user.is_empty() && host.contains('.') && !host.starts_with('.') && !host.ends_with('.')
}

impl Pattern {
    fn new(text: &str, gitlab: bool) -> Option<Self> {
        if text.is_empty() || text.contains('\0') {
            return None;
        }
        let folder = text.ends_with('/');
        let anchored_start = text.starts_with('/');
        let trimmed = text.trim_matches('/');
        if trimmed.is_empty() {
            // `/` alone: everything from the top.
            return Some(Self { segments: vec![b"**".to_vec()], folder: false });
        }
        let mut segments: Vec<Vec<u8>> = trimmed.split('/').filter(|s| !s.is_empty()).map(|s| s.as_bytes().to_vec()).collect();
        // GitLab: a trailing `/**` matches one level, like `*`.
        if gitlab && segments.len() > 1 && segments.last().is_some_and(|s| s == b"**") && !folder {
            *segments.last_mut()? = b"*".to_vec();
        }
        // GitHub anchors a pattern with a slash at the start or in the middle; GitLab
        // only one with a leading slash. An unanchored pattern may start at any depth.
        let anchored = anchored_start || !gitlab && trimmed.contains('/');
        if !anchored && segments.first().is_none_or(|s| s != b"**") {
            segments.insert(0, b"**".to_vec());
        }
        Some(Self { segments, folder })
    }

    /// Whether the file at `parts` falls under this pattern: the pattern names the file
    /// itself, or a folder holding it. A pattern ending in a wildcard (`docs/*`) names
    /// files at that level only, as GitHub's own example has it; one ending in a plain
    /// name or `/` also names a folder of that name.
    fn matches(&self, parts: &[&[u8]], work: &mut MatchWork<'_>) -> Option<bool> {
        work.step()?;
        if !self.folder && glob(&self.segments, parts, work)? {
            return Some(true);
        }
        let last = self.segments.last().map(Vec::as_slice).unwrap_or_default();
        let names_folder = self.folder || last == b"**" || !last.iter().any(|b| matches!(b, b'*' | b'?'));
        if names_folder {
            for k in 1..parts.len() {
                if glob(&self.segments, &parts[..k], work)? { return Some(true); }
            }
        }
        Some(false)
    }
}

/// Pattern segments against path segments; `**` stands for any number of folders.
fn glob(pattern: &[Vec<u8>], parts: &[&[u8]], work: &mut MatchWork<'_>) -> Option<bool> {
    // Iterative backtracking over the last `**` seen keeps this linear for ordinary
    // patterns and avoids deep recursion.
    let (mut p, mut s) = (0usize, 0usize);
    let mut star: Option<(usize, usize)> = None;
    while s < parts.len() {
        work.step()?;
        if p < pattern.len() && pattern[p] == b"**" {
            star = Some((p, s));
            p += 1;
            continue;
        }
        if p < pattern.len() && segment(&pattern[p], parts[s], work)? {
            p += 1;
            s += 1;
            continue;
        }
        match star {
            Some((sp, ss)) => {
                p = sp + 1;
                s = ss + 1;
                star = Some((sp, ss + 1));
            }
            None => return Some(false),
        }
    }
    while p < pattern.len() && pattern[p] == b"**" {
        work.step()?;
        p += 1;
    }
    Some(p == pattern.len())
}

/// One segment: `*` for any run of characters, `?` for one; everything else literal.
fn segment(pattern: &[u8], name: &[u8], work: &mut MatchWork<'_>) -> Option<bool> {
    let (mut p, mut n) = (0usize, 0usize);
    let mut star: Option<(usize, usize)> = None;
    while n < name.len() {
        work.step()?;
        if p < pattern.len() && pattern[p] == b'*' {
            star = Some((p, n));
            p += 1;
        } else if p < pattern.len() && (pattern[p] == b'?' || pattern[p] == name[n]) {
            p += 1;
            n += 1;
        } else if let Some((sp, sn)) = star {
            p = sp + 1;
            n = sn + 1;
            star = Some((sp, sn + 1));
        } else {
            return Some(false);
        }
    }
    while p < pattern.len() && pattern[p] == b'*' {
        work.step()?;
        p += 1;
    }
    Some(p == pattern.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn who(rules: &CodeOwners, path: &str) -> Ownership {
        rules.owners(path.as_bytes())
    }
    fn owned(names: &[&str]) -> Ownership {
        Ownership::Owners(names.iter().map(|n| n.to_string()).collect())
    }

    #[test]
    fn matching_budget_spans_files_and_stops_inside_wildcards() {
        let cancel = crate::CancellationToken::new();
        let rules = CodeOwners::parse("/unmatched @owner\n", b"CODEOWNERS");
        let mut work = MatchWork { remaining: 100, cancel: &cancel };
        let mut completed = 0;
        for _ in 0..1000 {
            if rules.owners_bounded(b"src/main.rs", &mut work) == Ownership::Limited { break; }
            completed += 1;
        }
        assert!(completed > 0 && completed < 1000);
        assert_eq!(work.remaining, 0);
        let rules = CodeOwners::parse(&format!("/{}b @owner\n", "*a".repeat(200)), b"CODEOWNERS");
        let mut work = MatchWork { remaining: 100, cancel: &cancel };
        assert_eq!(rules.owners_bounded(&vec![b'a'; 1000], &mut work), Ownership::Limited);
        cancel.cancel();
        let mut work = MatchWork::new(&cancel);
        assert_eq!(rules.owners_bounded(b"file", &mut work), Ownership::Limited);
        assert_eq!(work.remaining, 0, "cancelled work cannot continue");
    }

    #[test]
    fn github_examples_from_the_documentation() {
        let text = "\
# A comment
*       @global-owner1 @global-owner2
*.js    @js-owner #This is an inline comment.
*.go docs@example.com
*.txt @octo-org/octocats
/build/logs/ @doctocat
docs/* docs@example.com
apps/ @octocat
/docs/ @doctocat
/scripts/ @doctocat @octocat
**/logs @octocat
/apps/ @octocat
/apps/github
/apps/ @octocat
/apps/github @doctocat
";
        let rules = CodeOwners::parse(text, b".github/CODEOWNERS");
        assert!(!rules.gitlab);
        assert_eq!(rules.skipped, 0);
        assert_eq!(who(&rules, "README.md"), owned(&["@global-owner1", "@global-owner2"]));
        assert_eq!(who(&rules, "src/app.js"), owned(&["@js-owner"]));
        assert_eq!(who(&rules, "cmd/main.go"), owned(&["docs@example.com"]));
        // `/build/logs/` comes before `**/logs`, so the later line wins there too.
        assert_eq!(who(&rules, "build/logs/a/b.log"), owned(&["@octocat"]));
        assert_eq!(who(&rules, "build/other.log"), owned(&["@global-owner1", "@global-owner2"]));
        // docs/* covers the folder's own files, not deeper ones; /docs/ covers both and
        // comes later.
        assert_eq!(who(&rules, "docs/getting-started.md"), owned(&["@doctocat"]));
        assert_eq!(who(&rules, "deep/apps/x.rb"), owned(&["@octocat"]));
        assert_eq!(who(&rules, "deeply/nested/logs/today.txt"), owned(&["@octocat"]));
        // The last line for /apps/github wins over the empty one before it.
        assert_eq!(who(&rules, "apps/github/readme.md"), owned(&["@doctocat"]));
        assert_eq!(who(&rules, "apps/other.rb"), owned(&["@octocat"]));
    }

    #[test]
    fn github_lines_without_owners_unown_and_wildcards_stay_at_their_level() {
        let rules = CodeOwners::parse("/apps/ @octocat\n/apps/github\ndocs/* @docs\n", b"CODEOWNERS");
        assert_eq!(who(&rules, "apps/github/x.rb"), Ownership::Unowned);
        assert_eq!(who(&rules, "apps/x.rb"), owned(&["@octocat"]));
        assert_eq!(who(&rules, "docs/a.md"), owned(&["@docs"]));
        assert_eq!(who(&rules, "docs/build-app/troubleshooting.md"), Ownership::Nobody);
        assert_eq!(who(&rules, "nested/docs/a.md"), Ownership::Nobody, "a slash in the middle anchors");
        assert_eq!(who(&rules, "src/main.rs"), Ownership::Nobody);
    }

    #[test]
    fn github_refuses_negation_ranges_and_bad_owners() {
        let rules = CodeOwners::parse("!secret @a\n[abc].rs @b\n*.rs owner\n*.md @docs # trailing words\n*.toml @c\n", b"CODEOWNERS");
        assert_eq!(rules.skipped, 3);
        assert_eq!(rules.rules(), 2);
        assert_eq!(who(&rules, "a.md"), owned(&["@docs"]));
        assert_eq!(who(&rules, "Cargo.toml"), owned(&["@c"]));
        assert_eq!(who(&rules, "src/lib.rs"), Ownership::Nobody);
    }

    #[test]
    fn paths_are_case_sensitive_and_escaped_spaces_are_part_of_a_pattern() {
        let rules = CodeOwners::parse("/My\\ Folder/ @a\n/docs/ @b\n", b"CODEOWNERS");
        assert_eq!(who(&rules, "My Folder/x.txt"), owned(&["@a"]));
        assert_eq!(who(&rules, "Docs/x.md"), Ownership::Nobody);
    }

    #[test]
    fn gitlab_sections_combine_and_each_adds_its_owners() {
        let text = "\
* @admin
[README Owners]
README.md @user1 @user2
internal/README.md @user4
[README other owners]
README.md @user3
[readme OWNERS]
docs/README.md @user5
";
        let rules = CodeOwners::parse(text, b"CODEOWNERS");
        assert!(rules.gitlab);
        assert_eq!(who(&rules, "README.md"), owned(&["@admin", "@user1", "@user2", "@user3"]));
        // Inside one section the last match wins; relative paths float.
        assert_eq!(who(&rules, "app/internal/README.md"), owned(&["@admin", "@user4", "@user3"]));
        assert_eq!(who(&rules, "docs/README.md"), owned(&["@admin", "@user5", "@user3"]));
    }

    #[test]
    fn a_byte_order_mark_is_not_a_pattern_and_each_header_keeps_its_defaults() {
        let rules = CodeOwners::parse("\u{feff}/first.md @first\n", b"CODEOWNERS");
        assert_eq!(who(&rules, "first.md"), owned(&["@first"]));
        // Two headers of one name make one section, but each line takes the default
        // owners of the header above it.
        let rules = CodeOwners::parse("[Docs] @a\n/docs/a.md\n[docs] @b\n/docs/b.md\n", b".gitlab/CODEOWNERS");
        assert_eq!(rules.rules(), 2);
        assert_eq!(who(&rules, "docs/a.md"), owned(&["@a"]));
        assert_eq!(who(&rules, "docs/b.md"), owned(&["@b"]));
    }

    #[test]
    fn gitlab_defaults_exclusions_and_one_level_globstar() {
        let text = "\
[Docs][2] @docs-team
/docs/
/docs/api/ @api
!/docs/drafts/
^[Ruby] @rubyists
!/config/**/*.rb
*.rb
[Go]
/cmd/** @go
";
        let rules = CodeOwners::parse(text, b".gitlab/CODEOWNERS");
        assert_eq!(who(&rules, "docs/guide.md"), owned(&["@docs-team"]));
        assert_eq!(who(&rules, "docs/api/x.md"), owned(&["@api"]));
        assert_eq!(who(&rules, "docs/drafts/x.md"), Ownership::Unowned);
        assert_eq!(who(&rules, "lib/a.rb"), owned(&["@rubyists"]));
        assert_eq!(who(&rules, "config/env/prod.rb"), Ownership::Unowned);
        assert_eq!(who(&rules, "cmd/main.go"), owned(&["@go"]));
        assert_eq!(who(&rules, "cmd/tool/main.go"), Ownership::Nobody, "a trailing /** is one level on GitLab");
    }

    #[test]
    fn gitlab_reads_owners_inside_trailing_comments_and_skips_broken_headers() {
        let rules = CodeOwners::parse("[Section\n[Ok] @a\n*.md @b # also @c\n", b"CODEOWNERS");
        assert!(rules.gitlab);
        assert_eq!(rules.skipped, 1);
        assert_eq!(who(&rules, "x.md"), owned(&["@b", "@c"]));
    }

    #[test]
    fn globs_cross_folders_only_with_double_stars() {
        let rules = CodeOwners::parse("/src/**/mod.rs @a\n/lib/*.rs @b\n", b"CODEOWNERS");
        assert_eq!(who(&rules, "src/mod.rs"), owned(&["@a"]));
        assert_eq!(who(&rules, "src/x/y/mod.rs"), owned(&["@a"]));
        assert_eq!(who(&rules, "lib/a.rs"), owned(&["@b"]));
        assert_eq!(who(&rules, "lib/x/a.rs"), Ownership::Nobody);
    }
}
