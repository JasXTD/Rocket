use super::{SecretKind, incomplete, patterns, refusal};
use crate::{
    CancellationToken, Error, Result,
    runner::{Runner, args},
};
use std::{
    collections::{HashSet, VecDeque},
    ffi::OsString,
    path::Path,
    time::Instant,
};

const OBJECT_LIMIT: usize = 100_000;
const OBJECT_BYTES: usize = 128 * 1024 * 1024;
// Group small objects to amortize Git startup. A single larger object is read
// separately within OBJECT_BYTES; the configurable total is never an allocation.
const BATCH_BYTES: usize = 8 * 1024 * 1024;

pub(super) fn oid(bytes: &[u8]) -> Option<String> {
    if !matches!(bytes.len(), 40 | 64)
        || !bytes.iter().all(u8::is_ascii_hexdigit)
        || bytes.iter().all(|b| *b == b'0')
    {
        return None;
    }
    String::from_utf8(bytes.to_ascii_lowercase()).ok()
}

pub(super) struct Scan<'a> {
    runner: &'a Runner,
    cwd: &'a Path,
    operation: &'static str,
    cancel: &'a CancellationToken,
    started: Instant,
    queue: VecDeque<(String, String)>,
    seen: HashSet<String>,
    bytes: usize,
    byte_limit: usize,
    policy: patterns::Policy,
}
impl<'a> Scan<'a> {
    pub(super) fn new(
        runner: &'a Runner,
        cwd: &'a Path,
        operation: &'static str,
        cancel: &'a CancellationToken,
    ) -> Self {
        Self {
            runner,
            cwd,
            operation,
            cancel,
            started: Instant::now(),
            queue: VecDeque::new(),
            seen: HashSet::new(),
            bytes: 0,
            byte_limit: runner.secret_scan_bytes.load(std::sync::atomic::Ordering::Acquire),
            policy: patterns::Policy {
                secrets: runner
                    .secret_protection
                    .load(std::sync::atomic::Ordering::Acquire),
                ai_keys: runner
                    .guard_ai_api_keys
                    .load(std::sync::atomic::Ordering::Acquire),
            },
        }
    }
    fn check(&self) -> Result<()> {
        self.cancel.check()?;
        if self.started.elapsed() >= self.runner.limits.timeout {
            return Err(incomplete(self.operation));
        }
        Ok(())
    }
    pub(super) fn read(&self, arguments: &[OsString], input: Vec<u8>) -> Result<Vec<u8>> {
        self.read_bounded(
            arguments,
            input,
            self.runner.limits.stdout_bytes.min(BATCH_BYTES + 64 * 1024),
        )
    }
    fn read_bounded(
        &self,
        arguments: &[OsString],
        input: Vec<u8>,
        output_bytes: usize,
    ) -> Result<Vec<u8>> {
        self.check()?;
        let mut runner = self.runner.clone();
        runner.limits.timeout = self
            .runner
            .limits
            .timeout
            .saturating_sub(self.started.elapsed());
        // Only this scan's immutable object read gets a size-derived allowance.
        // The caller's ordinary command/diff output limits remain unchanged.
        runner.limits.stdout_bytes = output_bytes;
        match runner.run(self.cwd, arguments, input, self.cancel) {
            Ok(output) if output.status.success() => Ok(output.stdout),
            Err(Error::Cancelled) => Err(Error::Cancelled),
            // Never include raw Git errors or partial object contents in a refusal.
            _ => Err(incomplete(self.operation)),
        }
    }
    pub(super) fn resolve(&self, reference: &str) -> Result<String> {
        let bytes = self.read(
            &args(&[
                "--no-replace-objects",
                "rev-parse",
                "--verify",
                "--end-of-options",
                reference,
            ]),
            vec![],
        )?;
        oid(bytes.trim_ascii()).ok_or_else(|| incomplete(self.operation))
    }
    pub(super) fn content(&self, bytes: &[u8], location: String) -> Result<()> {
        self.check()?;
        if let Some((kind, line)) =
            patterns::scan_with_policy(bytes, false, self.policy, &mut || self.check())?
        {
            return Err(refusal(self.operation, kind, location, Some(line)));
        }
        Ok(())
    }
    pub(super) fn message(&self, bytes: &[u8], location: String) -> Result<()> {
        self.check()?;
        if let Some((kind, line)) =
            patterns::scan_with_policy(bytes, true, self.policy, &mut || self.check())?
        {
            return Err(refusal(self.operation, kind, location, Some(line)));
        }
        Ok(())
    }
    pub(super) fn path(&self, path: &[u8]) -> Result<()> {
        self.content(path, "a filename (hidden)".into())?;
        if self.policy.secrets && path.split(|b| *b == b'/').any(patterns::environment_file) {
            return Err(refusal(
                self.operation,
                SecretKind::EnvironmentFile,
                patterns::safe_path(path),
                None,
            ));
        }
        Ok(())
    }
    pub(super) fn enqueue(&mut self, id: String, location: String) -> Result<()> {
        self.check()?;
        if self.seen.insert(id.clone()) {
            if self.seen.len() > OBJECT_LIMIT {
                return Err(incomplete(self.operation));
            }
            self.queue.push_back((id, location));
        }
        Ok(())
    }
    pub(super) fn single(&mut self, id: &str, location: String, follow: bool) -> Result<()> {
        self.batch(vec![(id.to_owned(), location)], follow)
    }
    pub(super) fn finish(&mut self) -> Result<()> {
        while !self.queue.is_empty() {
            self.check()?;
            let batch = (0..64).filter_map(|_| self.queue.pop_front()).collect();
            self.batch(batch, true)?;
        }
        Ok(())
    }
    fn batch(&mut self, requested: Vec<(String, String)>, follow: bool) -> Result<()> {
        let input = requested
            .iter()
            .map(|(id, _)| format!("{id}\n"))
            .collect::<String>()
            .into_bytes();
        let metadata = self.read(
            &args(&[
                "--no-replace-objects",
                "cat-file",
                "--batch-check=%(objectname) %(objecttype) %(objectsize)",
            ]),
            input,
        )?;
        let rows = metadata
            .split(|b| *b == b'\n')
            .filter(|r| !r.is_empty())
            .collect::<Vec<_>>();
        if rows.len() != requested.len() {
            return Err(incomplete(self.operation));
        }
        // Validate and charge the complete metadata response before reading any
        // payload. A known over-budget batch must not spend the scan deadline
        // decoding its first large object before reaching the later size row.
        let mut objects = Vec::with_capacity(requested.len());
        for (row, (id, location)) in rows.into_iter().zip(requested) {
            let fields = row.split(|b| *b == b' ').collect::<Vec<_>>();
            if fields.len() != 3 || fields[0] != id.as_bytes() {
                return Err(incomplete(self.operation));
            }
            let kind = std::str::from_utf8(fields[1])
                .map_err(|_| incomplete(self.operation))?
                .to_owned();
            if !matches!(kind.as_str(), "blob" | "tree" | "commit" | "tag") {
                return Err(incomplete(self.operation));
            }
            let length: usize = std::str::from_utf8(fields[2])
                .ok()
                .and_then(|n| n.parse().ok())
                .ok_or_else(|| incomplete(self.operation))?;
            if length > OBJECT_BYTES {
                return Err(refusal(
                    self.operation,
                    SecretKind::Incomplete,
                    "the 128 MiB per-object limit (independent of the total scan budget)".into(),
                    None,
                ));
            }
            if length > self.byte_limit.saturating_sub(self.bytes) {
                return Err(refusal(
                    self.operation,
                    SecretKind::Incomplete,
                    format!("the configured {} MiB total Git object scan budget (uncompressed, including reachable history)", self.byte_limit / (1024 * 1024)),
                    None,
                ));
            }
            self.bytes += length;
            objects.push((id, location, kind, length));
        }
        let mut group = Vec::new();
        let mut size = 0;
        for (id, location, kind, length) in objects {
            if !group.is_empty() && size + length > BATCH_BYTES {
                self.read_group(std::mem::take(&mut group), follow)?;
                size = 0;
            }
            size += length;
            group.push((id, location, kind, length));
        }
        if !group.is_empty() {
            self.read_group(group, follow)?;
        }
        Ok(())
    }
    fn read_group(
        &mut self,
        group: Vec<(String, String, String, usize)>,
        follow: bool,
    ) -> Result<()> {
        let input = group
            .iter()
            .map(|(id, ..)| format!("{id}\n"))
            .collect::<String>()
            .into_bytes();
        // Metadata already charged every object's raw bytes to the total budget.
        // Also bound protocol overhead exactly; oversized/truncated output fails.
        let output_bytes = group
            .iter()
            .try_fold(0usize, |total, (id, _, kind, length)| {
                total
                    .checked_add(format!("{id} {kind} {length}\n").len())
                    .and_then(|n| n.checked_add(*length))
                    .and_then(|n| n.checked_add(1))
                    .ok_or_else(|| incomplete(self.operation))
            })?;
        let bytes = self.read_bounded(
            &args(&["--no-replace-objects", "cat-file", "--batch"]),
            input,
            output_bytes,
        )?;
        let mut at = 0;
        for (id, location, kind, length) in group {
            self.check()?;
            let header = format!("{id} {kind} {length}\n");
            if !bytes
                .get(at..)
                .is_some_and(|b| b.starts_with(header.as_bytes()))
            {
                return Err(incomplete(self.operation));
            }
            at += header.len();
            let end = at
                .checked_add(length)
                .ok_or_else(|| incomplete(self.operation))?;
            let content = bytes
                .get(at..end)
                .ok_or_else(|| incomplete(self.operation))?;
            if bytes.get(end) != Some(&b'\n') {
                return Err(incomplete(self.operation));
            }
            if kind == "commit" {
                let header_end = content
                    .windows(2)
                    .position(|w| w == b"\n\n")
                    .ok_or_else(|| incomplete(self.operation))?;
                // Only the commit body is prose. Keep object headers strict and
                // use the same message policy for immediate commits, reused
                // amend messages and every historical commit checked on push.
                self.content(&content[..header_end], location.clone())?;
                self.message(
                    &content[header_end + 2..],
                    format!("commit message in {location}"),
                )?;
            } else {
                self.content(content, location)?;
            }
            if follow {
                self.children(&id, &kind, content)?;
            }
            at = end + 1;
        }
        if at != bytes.len() {
            return Err(incomplete(self.operation));
        }
        Ok(())
    }
    fn children(&mut self, id: &str, kind: &str, bytes: &[u8]) -> Result<()> {
        match kind {
            "commit" | "tag" => {
                let header_end = bytes
                    .windows(2)
                    .position(|w| w == b"\n\n")
                    .ok_or_else(|| incomplete(self.operation))?;
                let mut target_found = false;
                for line in bytes[..header_end].split(|b| *b == b'\n') {
                    let target = if kind == "commit" {
                        line.strip_prefix(b"tree ")
                    } else {
                        line.strip_prefix(b"object ")
                    };
                    if let Some(target) = target {
                        let child = oid(target).ok_or_else(|| incomplete(self.operation))?;
                        self.enqueue(
                            child.clone(),
                            format!("object {child} referenced by {kind} {id}"),
                        )?;
                        target_found = true;
                    } else if kind == "commit"
                        && let Some(parent) = line.strip_prefix(b"parent ")
                    {
                        let parent = oid(parent).ok_or_else(|| incomplete(self.operation))?;
                        self.enqueue(parent.clone(), format!("commit {parent}"))?;
                    }
                }
                if !target_found {
                    return Err(incomplete(self.operation));
                }
            }
            "tree" => {
                let mut at = 0;
                while at < bytes.len() {
                    self.check()?;
                    let space = bytes[at..]
                        .iter()
                        .position(|b| *b == b' ')
                        .map(|n| n + at)
                        .ok_or_else(|| incomplete(self.operation))?;
                    let nul = bytes[space + 1..]
                        .iter()
                        .position(|b| *b == 0)
                        .map(|n| n + space + 1)
                        .ok_or_else(|| incomplete(self.operation))?;
                    let mode = &bytes[at..space];
                    let name = &bytes[space + 1..nul];
                    if name.is_empty() || name.contains(&b'/') {
                        return Err(incomplete(self.operation));
                    }
                    self.path(name)?;
                    let end = nul + 1 + id.len() / 2;
                    let raw_id = bytes
                        .get(nul + 1..end)
                        .ok_or_else(|| incomplete(self.operation))?;
                    if mode != b"160000" {
                        if !matches!(mode, b"40000" | b"100644" | b"100755" | b"120000") {
                            return Err(incomplete(self.operation));
                        }
                        let child = raw_id
                            .iter()
                            .map(|b| format!("{b:02x}"))
                            .collect::<String>();
                        self.enqueue(
                            child.clone(),
                            format!("{} (object {child})", patterns::safe_path(name)),
                        )?;
                    }
                    at = end;
                }
            }
            "blob" => {}
            _ => return Err(incomplete(self.operation)),
        }
        Ok(())
    }
}
