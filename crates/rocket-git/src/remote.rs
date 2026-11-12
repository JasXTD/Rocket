use crate::{model::bytes_to_os, runner::args, *};
use std::{collections::BTreeMap, ffi::OsString, sync::mpsc::SyncSender};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FetchTags {
    #[default]
    None,
    /// Git's usual reachable-tag behavior, respecting remote.<name>.tagOpt.
    RemoteDefault,
    /// All remote tags; never force replacement of a different local tag.
    All,
}
impl FetchTags {
    /// Sets this choice in a prepared fetch, which names `--no-tags` explicitly.
    pub(crate) fn apply(self, arguments: &mut Vec<OsString>) {
        let no_tags = arguments.iter().position(|a|a == "--no-tags").expect("explicit fetch tag mode");
        match self {
            FetchTags::None => (),
            FetchTags::RemoteDefault => { arguments.remove(no_tags); }
            FetchTags::All => arguments[no_tags] = "--tags".into(),
        }
    }
}

impl Repository {
    /// List remotes from effective Git configuration (including includes).
    /// Names and values retain their bytes. Legacy .git/remotes and .git/branches
    /// files are not supported. No transport or credential helper is contacted.
    pub fn remotes(&self, cancel: &CancellationToken) -> Result<Vec<Remote>> {
        let output = self.runner.run(
            self.cwd(),
            &args(&[
                "config",
                "--null",
                "--get-regexp",
                r"^remote\..*\.(url|pushurl|fetch)$",
            ]),
            Vec::new(),
            cancel,
        )?;
        if output.status.code() == Some(1) && output.stdout.is_empty() {
            return Ok(Vec::new());
        }
        parse_remotes(&output.success("list remotes")?)
    }

    /// Fetch one explicitly selected, configured remote. This first slice accepts
    /// branch mappings refs/heads/X:refs/remotes/<name>/X (optional + and wildcard),
    /// plus negative branch refspecs. It rejects other mappings before transport.
    /// It does not prune, fetch tags/submodules, merge, or change upstream config.
    /// Git's configured URL rewriting, transports, helpers, and hooks still apply.
    ///
    /// Use WriteTask off the UI thread. Optional progress comes through a bounded
    /// std::sync::mpsc::sync_channel; full/disconnected queues lose updates without
    /// blocking Git. Use a fresh channel per request and the returned report for
    /// completion. Timeout/output limits come from this repository's GitClient.
    pub fn fetch(
        &self,
        name: impl AsRef<[u8]>,
        progress: Option<&SyncSender<FetchProgress>>,
        cancel: &CancellationToken,
    ) -> Result<FetchReport> {
        self.fetch_with_tags(name, FetchTags::None, progress, cancel)
    }
    pub fn fetch_with_tags(
        &self, name: impl AsRef<[u8]>, tags: FetchTags,
        progress: Option<&SyncSender<FetchProgress>>, cancel: &CancellationToken,
    ) -> Result<FetchReport> {
        let (result, (status, branches)) = self.fetch_with_refresh(
            name, tags, progress, cancel, |repo| {
                let branches = repo.runner.observe("refresh", "branches", || {
                    repo.branches(&CancellationToken::new())
                });
                let status = repo.info().worktree.as_ref().map(|_| {
                    repo.runner.observe("refresh", "status", || {
                        repo.status(&CancellationToken::new())
                    })
                });
                (status, branches)
            },
        )?;
        Ok(FetchReport { result, status, branches })
    }

    /// Reconcile once using the caller's fresh read, while still holding the
    /// repository write permit. Called after every started transport, including
    /// cancellation/failure; validation failures before transport return early.
    /// The callback must be read-only (no nested write permit) and use an
    /// independent token so a cancelled fetch still reports its actual state.
    /// Ordinary callers can use `fetch`.
    pub fn fetch_with_refresh<T>(
        &self, name: impl AsRef<[u8]>, tags: FetchTags,
        progress: Option<&SyncSender<FetchProgress>>, cancel: &CancellationToken,
        refresh: impl FnOnce(&Self) -> T,
    ) -> Result<(Result<FetchOutput>, T)> {
        let name = name.as_ref();
        let (prefix, remote_argument) = self.validate_fetch_selection(name, cancel)?;
        let _permit = self.acquire_write(cancel)?;
        let (mut arguments, input) =
            self.runner
                .step("fetch", "validate_remote_and_refspecs", || {
                    self.prepare_fetch(name, &prefix, remote_argument, cancel)
                })?;
        tags.apply(&mut arguments);
        let result = self.execute_fetch(&arguments, input, progress, cancel);
        Ok((result, refresh(self)))
    }

    pub(crate) fn validate_fetch_selection(
        &self,
        name: &[u8],
        cancel: &CancellationToken,
    ) -> Result<(Vec<u8>, OsString)> {
        let (prefix, remote_argument) = self.runner.step("fetch", "validate_selection", || {
            cancel.check()?;
            if name.is_empty() || name.len() > 4096 || name.contains(&0) || name.starts_with(b"-") {
                return Err(Error::InvalidInput(
                    "select a configured remote name of 1..4096 bytes without NUL or a leading dash"
                        .into(),
                ));
            }
            let mut prefix = b"refs/remotes/".to_vec();
            prefix.extend_from_slice(name);
            prefix.push(b'/');
            let mut probe = prefix.clone();
            probe.extend_from_slice(b"rocket-name-check");
            self.check_fetch_ref(&probe, false, cancel)?;
            let remote_argument = bytes_to_os(name)?;
            Ok((prefix, remote_argument))
        })?;
        Ok((prefix, remote_argument))
    }

    // Caller holds the common-directory write gate through command and refreshes.
    pub(crate) fn execute_fetch(
        &self,
        arguments: &[OsString],
        input: Vec<u8>,
        progress: Option<&SyncSender<FetchProgress>>,
        cancel: &CancellationToken,
    ) -> Result<FetchOutput> {
        let fetched = self.runner.observe("write_result", "fetch", || {
            let output = self
                .runner
                .fetch(self.cwd(), arguments, input, cancel, progress)?;
            self.fetch_output(output)
        })?;
        self.index_fetched_commits(&fetched, cancel);
        Ok(fetched)
    }

    /// Where the repository keeps a commit index up to date, add what this fetch
    /// brought. Nothing runs when every branch was already up to date.
    fn index_fetched_commits(&self, fetched: &FetchOutput, cancel: &CancellationToken) {
        let brought = fetched.updates.as_ref().map_or(true, |updates| {
            updates.iter().any(|update| update.kind != FetchUpdateKind::UpToDate)
        });
        if brought {
            self.update_history_index_after_fetch(cancel);
        }
    }

    /// [`Repository::execute_fetch`] for automatic fetches, which may never prompt.
    pub(crate) fn execute_quiet_fetch(
        &self,
        arguments: &[OsString],
        input: Vec<u8>,
        cancel: &CancellationToken,
    ) -> Result<FetchOutput> {
        let fetched = self.runner.observe("write_result", "fetch", || {
            let output = self
                .runner
                .quiet_fetch(self.cwd(), arguments, input, cancel)?;
            self.fetch_output(output)
        })?;
        self.index_fetched_commits(&fetched, cancel);
        Ok(fetched)
    }

    fn fetch_output(&self, output: crate::runner::Output) -> Result<FetchOutput> {
        if !output.status.success() {
            return Err(Error::Git {
                operation: "fetch",
                code: output.status.code(),
                stdout: output.stdout,
                stderr: output.stderr,
            });
        }
        Ok(FetchOutput {
            updates: self.runner.observe("parse", "fetch", || {
                parse_updates(&output.stdout, self.info().object_format)
            }),
            output: WriteOutput {
                stdout: output.stdout,
                stderr: output.stderr,
            },
        })
    }

    pub(crate) fn prepare_fetch(
        &self,
        name: &[u8],
        prefix: &[u8],
        remote_argument: std::ffi::OsString,
        cancel: &CancellationToken,
    ) -> Result<(Vec<std::ffi::OsString>, Vec<u8>)> {
        let remote = self
            .remotes(cancel)?
            .into_iter()
            .find(|r| r.name == name)
            .ok_or_else(|| {
                Error::InvalidInput(
                    "remote is not configured; select a name returned by remotes()".into(),
                )
            })?;
        if remote.urls.is_empty() || remote.urls.iter().any(|url| url.is_empty()) {
            return Err(Error::Unsupported("fetch requires a configured, nonempty remote URL; URL reset entries are not supported yet".into()));
        }
        self.ensure_no_remote_group(name, cancel)?;
        if remote.fetch_refspecs.is_empty() || remote.fetch_refspecs.len() > 128 {
            return Err(Error::Unsupported(
                "fetch requires 1..128 configured branch refspecs".into(),
            ));
        }
        let mut input = Vec::new();
        let mut positive = false;
        for spec in &remote.fetch_refspecs {
            let (source, destination) = branch_refspec(spec, prefix)?;
            self.check_fetch_ref(source, true, cancel)?;
            if let Some(destination) = destination {
                positive = true;
                if !destination.contains(&b'*') {
                    // Wildcard expansion skips existing symbolic destinations in
                    // Git. Exact refspecs dereference them, even dangling symrefs,
                    // and could write outside the selected remote's namespace.
                    let mut arguments = args(&["symbolic-ref", "--quiet"]);
                    arguments.push(bytes_to_os(destination)?);
                    let output = self
                        .runner
                        .run(self.cwd(), &arguments, Vec::new(), cancel)?;
                    match output.status.code() {
                        Some(1) => (),
                        Some(0) => return Err(Error::Unsupported("an exact fetch destination is a symbolic ref; refusing to update its target".into())),
                        _ => { output.success("check fetch destination")?; }
                    }
                }
            }
            input.extend_from_slice(spec);
            input.push(b'\n');
        }
        if !positive {
            return Err(Error::InvalidInput(
                "fetch requires at least one positive branch refspec".into(),
            ));
        }
        let mut arguments = args(&[
            "fetch",
            "--atomic",
            "--no-all",
            "--no-tags",
            "--no-prune",
            "--no-prune-tags",
            "--no-recurse-submodules",
            "--no-auto-maintenance",
            "--no-write-commit-graph",
            "--porcelain",
            "--verbose",
            "--progress",
            "--show-forced-updates",
            "--write-fetch-head",
            "--refmap=",
            "--stdin",
            "--",
        ]);
        arguments.push(remote_argument);
        // The validated refspecs are pinned on stdin, not read again from config.
        // --refmap= prevents additional opportunistic mappings from configuration.
        // --atomic protects the ref transaction, not downloaded objects/FETCH_HEAD.
        cancel.check()?;
        Ok((arguments, input))
    }

    pub(crate) fn ensure_no_remote_group(
        &self,
        name: &[u8],
        cancel: &CancellationToken,
    ) -> Result<()> {
        // Git also accepts remotes.<group>. Refuse a collision rather than letting
        // the selected remote silently mean a different set of repositories.
        let mut group_key = b"remotes.".to_vec();
        group_key.extend_from_slice(name);
        // The final component of a config key is case-insensitive. Read key
        // names instead of querying a constructed key: valid remote names can
        // contain characters (e.g. '=' or '/') invalid in a config variable.
        let variable = group_key.iter().rposition(|b| *b == b'.').unwrap() + 1;
        group_key[variable..].make_ascii_lowercase();
        let group = self.runner.run(
            self.cwd(),
            &args(&[
                "config",
                "--null",
                "--name-only",
                "--get-regexp",
                r"^remotes\.",
            ]),
            Vec::new(),
            cancel,
        )?;
        match group.status.code() {
            Some(1) if group.stdout.is_empty() => (),
            Some(0) if group.stdout.split(|b| *b == 0).any(|key| key == group_key) => {
                return Err(Error::Unsupported(
                    "remote name also names a remote group; select one unambiguous remote".into(),
                ));
            }
            _ => {
                group.success("check remote group")?;
            }
        }
        Ok(())
    }

    pub(crate) fn check_fetch_ref(
        &self,
        reference: &[u8],
        pattern: bool,
        cancel: &CancellationToken,
    ) -> Result<()> {
        let mut arguments = args(&["check-ref-format"]);
        if pattern {
            arguments.push("--refspec-pattern".into());
        }
        arguments.push(bytes_to_os(reference)?);
        let output = self
            .runner
            .run(self.cwd(), &arguments, Vec::new(), cancel)?;
        match output.status.code() {
            Some(0) => Ok(()),
            Some(1) => Err(Error::InvalidInput(
                "invalid remote name or branch refspec".into(),
            )),
            _ => output.success("validate fetch reference").map(|_| ()),
        }
    }
}

pub(crate) fn parse_remotes(bytes: &[u8]) -> Result<Vec<Remote>> {
    let mut remotes = BTreeMap::new();
    if bytes.is_empty() {
        return Ok(Vec::new());
    }
    let records = bytes
        .strip_suffix(b"\0")
        .ok_or_else(|| Error::parse("remotes", "unterminated config record"))?;
    for record in records.split(|b| *b == 0) {
        // Config subsection names cannot contain newline or NUL. Values can
        // contain newlines, so only the first newline separates key from value.
        let split = record
            .iter()
            .position(|b| *b == b'\n')
            .unwrap_or(record.len());
        let key = &record[..split];
        let value = record.get(split + 1..).unwrap_or_default();
        let key = key
            .strip_prefix(b"remote.")
            .ok_or_else(|| Error::parse("remotes", "invalid config key"))?;
        let dot = key
            .iter()
            .rposition(|b| *b == b'.')
            .ok_or_else(|| Error::parse("remotes", "missing remote field"))?;
        let name = &key[..dot];
        if name.is_empty() {
            return Err(Error::parse("remotes", "empty remote name"));
        }
        let remote = remotes.entry(name.to_vec()).or_insert_with(|| Remote {
            name: name.to_vec(),
            urls: Vec::new(),
            push_urls: Vec::new(),
            fetch_refspecs: Vec::new(),
        });
        match &key[dot + 1..] {
            b"url" => remote.urls.push(value.to_vec()),
            b"pushurl" => remote.push_urls.push(value.to_vec()),
            b"fetch" => remote.fetch_refspecs.push(value.to_vec()),
            _ => return Err(Error::parse("remotes", "unexpected remote field")),
        }
    }
    Ok(remotes.into_values().collect())
}

pub(crate) fn pattern_matches(pattern: &[u8], reference: &[u8]) -> bool {
    match pattern.iter().position(|b| *b == b'*') {
        None => pattern == reference,
        Some(index) => {
            reference.len() >= pattern.len() - 1
                && reference.starts_with(&pattern[..index])
                && reference.ends_with(&pattern[index + 1..])
        }
    }
}

pub(crate) fn branch_refspec<'a>(
    spec: &'a [u8],
    prefix: &[u8],
) -> Result<(&'a [u8], Option<&'a [u8]>)> {
    let unsupported = || {
        Error::Unsupported("tracking supports only refs/heads/X:refs/remotes/<selected-remote>/X (optional + and wildcard) and negative refs/heads/X; mirror, tag, renamed, and local-branch mappings are not supported".into())
    };
    if spec.is_empty()
        || spec.len() > 4096
        || spec.iter().any(|b| b.is_ascii_whitespace() || *b == 0)
    {
        return Err(unsupported());
    }
    if let Some(source) = spec.strip_prefix(b"^") {
        if !source.starts_with(b"refs/heads/") {
            return Err(unsupported());
        }
        return Ok((source, None));
    }
    let mapping = spec.strip_prefix(b"+").unwrap_or(spec);
    let colon = mapping
        .iter()
        .position(|b| *b == b':')
        .ok_or_else(unsupported)?;
    let (source, destination) = (&mapping[..colon], &mapping[colon + 1..]);
    let branch = source
        .strip_prefix(b"refs/heads/")
        .ok_or_else(unsupported)?;
    if branch.is_empty() || destination.strip_prefix(prefix) != Some(branch) {
        return Err(unsupported());
    }
    Ok((source, Some(destination)))
}

fn parse_updates(bytes: &[u8], format: ObjectFormat) -> Result<Vec<FetchUpdate>> {
    let invalid = || Error::parse("fetch", "invalid porcelain ref update");
    if bytes.is_empty() {
        return Ok(Vec::new());
    }
    let bytes = bytes.strip_suffix(b"\n").ok_or_else(invalid)?;
    let width = if format == ObjectFormat::Sha1 { 40 } else { 64 };
    let oid = |raw: &[u8]| -> Result<ObjectId> {
        if raw.len() != width {
            return Err(invalid());
        }
        ObjectId::parse(std::str::from_utf8(raw).map_err(|_| invalid())?).map_err(|_| invalid())
    };
    bytes
        .split(|b| *b == b'\n')
        .map(|line| {
            let kind = match line.first() {
                Some(b' ') => FetchUpdateKind::FastForward,
                Some(b'+') => FetchUpdateKind::Forced,
                Some(b'*') => FetchUpdateKind::New,
                Some(b'=') => FetchUpdateKind::UpToDate,
                _ => return Err(invalid()),
            };
            let mut fields = line
                .get(2..)
                .filter(|_| line.get(1) == Some(&b' '))
                .ok_or_else(invalid)?
                .split(|b| *b == b' ');
            let old = fields.next().ok_or_else(invalid)?;
            let new = fields.next().ok_or_else(invalid)?;
            let reference = fields.next().ok_or_else(invalid)?;
            if fields.next().is_some()
                || !reference.starts_with(b"refs/remotes/")
                || reference.iter().any(|b| *b == 0 || b.is_ascii_whitespace())
            {
                return Err(invalid());
            }
            let old = oid(old)?;
            let old = (!old.as_str().bytes().all(|b| b == b'0')).then_some(old);
            let new = oid(new)?;
            if new.as_str().bytes().all(|b| b == b'0')
                || (kind == FetchUpdateKind::New) != old.is_none()
            {
                return Err(invalid());
            }
            Ok(FetchUpdate {
                kind,
                old,
                new,
                reference: reference.to_vec(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_records_preserve_case_dots_newlines_and_multiple_urls() {
        let parsed = parse_remotes(b"remote.Work.foo.url\nfirst\nline\0remote.Work.foo.url\nsecond\0remote.Work.foo.pushurl\npush\0remote.work.url\0").unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].name, b"Work.foo");
        assert_eq!(
            parsed[0].urls,
            [b"first\nline".to_vec(), b"second".to_vec()]
        );
        assert_eq!(parsed[0].push_urls, [b"push".to_vec()]);
        assert_eq!(parsed[1].urls, [Vec::<u8>::new()]);
        for invalid in [
            b"remote.a.url\nvalue".as_slice(),
            b"remote..url\nx\0",
            b"\0",
            b"remote.a.other\nx\0",
        ] {
            assert!(parse_remotes(invalid).is_err());
        }
    }

    #[test]
    fn porcelain_preserves_fast_forward_flag_and_rejects_corrupt_records() {
        let old = "a".repeat(40);
        let new = "b".repeat(40);
        let parsed = parse_updates(
            format!("  {old} {new} refs/remotes/origin/main\n").as_bytes(),
            ObjectFormat::Sha1,
        )
        .unwrap();
        assert_eq!(parsed[0].kind, FetchUpdateKind::FastForward);
        assert_eq!(parsed[0].old.as_ref().unwrap().as_str(), old);
        for bad in [
            format!("  {old} {new} refs/remotes/origin/main"),
            format!("! {old} {new} refs/remotes/origin/main\n"),
            format!("* {old} {new} refs/remotes/origin/main\n"),
            format!("  {old} {new} refs/heads/main\n"),
            format!("  {old} {new} refs/remotes/origin/bad ref\n"),
        ] {
            assert!(parse_updates(bad.as_bytes(), ObjectFormat::Sha1).is_err());
        }
        assert!(
            parse_updates(
                format!("  {old} {new} refs/remotes/origin/main\n").as_bytes(),
                ObjectFormat::Sha256
            )
            .is_err()
        );
    }
}
