use crate::{parse, runner::args, *};
use std::{fs::File, io::Read};

/// Git's state files name one commit or list a plan: far under this, even for long plans.
const STATE_LIMIT: u64 = 1024 * 1024;

/// The first non-blank line, kept short and with invisible characters shown.
fn first_line(text: &str) -> String {
    let line = text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or_default();
    crate::display_text::bounded(line, 240).text
}

/// Plan lines that make a commit. The rest are comments, commands, breaks, labels and
/// branch moves.
fn commit_lines(plan: &str) -> usize {
    plan.lines()
        .filter(|l| {
            matches!(
                l.split_whitespace().next(),
                Some("pick" | "p" | "reword" | "r" | "edit" | "e" | "squash" | "s" | "fixup" | "f" | "merge" | "m")
            )
        })
        .count()
}

/// The last commit line's subject, which Git writes after a `#`.
fn planned_subject(done: &str) -> String {
    done.lines()
        .rev()
        .find(|l| commit_lines(l) == 1)
        .and_then(|l| l.split_once(" # "))
        .map(|(_, subject)| first_line(subject))
        .unwrap_or_default()
}

impl Repository {
    /// Read operation markers from Git's discovered per-worktree directory, not
    /// from an assumed `.git` folder or the shared common directory.
    pub fn operation_state(&self, cancel: &CancellationToken) -> Result<OperationState> {
        self.require_worktree(cancel)?;
        let merge_heads = self.operation_ids("MERGE_HEAD", cancel)?;
        let merge_committed = if merge_heads.is_empty() {
            false
        } else {
            let output = self
                .runner
                .run(
                    self.cwd(),
                    &args(&["--no-replace-objects", "cat-file", "--batch"]),
                    b"HEAD\n".to_vec(),
                    cancel,
                )?
                .success("inspect merge parents")?;
            let id = parse::oid(
                output.split(|b| *b == b' ').next().unwrap_or_default(),
                "merge parents",
            )?;
            let commit = parse::commits(&output, &[id])?.remove(0);
            // Inspect actual commit headers, not a history walk affected by
            // grafts/replace refs. Skip the original checkout's first parent.
            commit.parents.len() >= 2
                && merge_heads
                    .iter()
                    .all(|head| commit.parents[1..].contains(head))
        };
        // These are refs, potentially stored in reftable. MERGE_HEAD itself is a
        // multi-ID operation file, not an ordinary ref; keep that read separate.
        let [merge_autostash_exists, cherry_pick, revert] = self.operation_refs(cancel)?;
        let merge_autostash = if merge_autostash_exists {
            let output = self
                .runner
                .run(
                    self.cwd(),
                    &args(&["show-ref", "--verify", "--hash", "--", "MERGE_AUTOSTASH"]),
                    Vec::new(),
                    cancel,
                )?
                .success("operation state")?;
            Some(parse::oid(parse::line(&output), "operation state")?)
        } else {
            None
        };
        let exists = |name: &str| -> Result<bool> {
            cancel.check()?;
            self.info()
                .git_dir
                .join(name)
                .try_exists()
                .map_err(|e| Error::io("read operation marker", e))
        };
        let mail_import = exists("rebase-apply/applying")?;
        let rebase = exists("rebase-merge")? || (exists("rebase-apply")? && !mail_import);
        let sequencer = exists("sequencer")?;
        let step = if mail_import {
            self.import_step()
        } else if rebase {
            self.rebase_step(cancel)
        } else if cherry_pick || revert || sequencer {
            self.sequence_step(cherry_pick || revert)
        } else {
            None
        };
        let state = OperationState {
            bisect: self.is_bisecting(cancel)?,
            merge_heads,
            merge_committed,
            merge_autostash,
            rebase,
            mail_import,
            cherry_pick,
            revert,
            sequencer,
            step,
        };
        cancel.check()?;
        Ok(state)
    }

    /// A small file of Git's operation state. These only label what is on screen,
    /// so a missing, unreadable or oversized file reads as nothing.
    fn state_text(&self, name: &str) -> Option<String> {
        let file = File::open(self.info().git_dir.join(name)).ok()?;
        let mut bytes = Vec::new();
        file.take(STATE_LIMIT + 1).read_to_end(&mut bytes).ok()?;
        (bytes.len() as u64 <= STATE_LIMIT).then(|| String::from_utf8_lossy(&bytes).into_owned())
    }

    fn import_step(&self) -> Option<OperationStep> {
        let number = |name: &str| self.state_text(name)?.trim().parse::<usize>().ok();
        let (at, total) = (number("rebase-apply/next")?, number("rebase-apply/last")?);
        (at > 0 && at <= total).then(|| OperationStep {
            at,
            total,
            kind: StepKind::Commit,
            subject: self.state_text("rebase-apply/final-commit").as_deref().map(first_line).unwrap_or_default(),
            branch: None,
        })
    }

    /// Counts commits, not plan lines: a plan also holds commands, breaks and branch moves.
    fn rebase_step(&self, cancel: &CancellationToken) -> Option<OperationStep> {
        let done = self.state_text("rebase-merge/done")?;
        let todo = self.state_text("rebase-merge/git-rebase-todo").unwrap_or_default();
        let at = commit_lines(&done);
        let total = at + commit_lines(&todo);
        let branch = self
            .state_text("rebase-merge/head-name")
            .and_then(|name| name.trim().strip_prefix("refs/heads/").map(|b| b.as_bytes().to_vec()));
        // A folded commit's message starts with Git's comments, so the plan's line comes first.
        let subject = Some(planned_subject(&done)).filter(|s| !s.is_empty()).unwrap_or_else(|| {
            let message = self.state_text("rebase-merge/message").unwrap_or_default();
            first_line(&message.lines().filter(|l| !l.starts_with('#')).collect::<Vec<_>>().join("\n"))
        });
        (total > 0).then(|| OperationStep { at: at.max(1), total, kind: self.rebase_stop(cancel), subject, branch })
    }

    /// A stopped cherry-pick or revert. Git has prepared the commit's message, and only a
    /// range Rocket reviewed has a count.
    fn sequence_step(&self, stopped: bool) -> Option<OperationStep> {
        let git_dir = &self.info().git_dir;
        let position = crate::sequence::read_journal(git_dir).ok().flatten().and_then(|(journal, _)| {
            let sequencer = crate::sequence::read_sequencer(git_dir).ok().flatten()?;
            let left = crate::sequence::remaining(&journal, &sequencer)?.len();
            Some((journal.commits.len() - left + 1, journal.commits.len()))
        });
        let (at, total) = position.unwrap_or((1, 1));
        let message = self.state_text("MERGE_MSG").unwrap_or_default();
        let line = first_line(&message);
        let subject = line.strip_prefix("Revert \"").and_then(|s| s.strip_suffix('"')).map(str::to_owned).unwrap_or(line);
        Some(OperationStep {
            at,
            total,
            kind: if stopped { StepKind::Commit } else { StepKind::Between },
            subject: if stopped { subject } else { String::new() },
            branch: None,
        })
    }

    /// Whether the branch left the tip this rebase saved, which the review refuses. Asks
    /// Git because the branch may be packed or in reftable; a failed read reads as nothing.
    fn rebase_branch_moved(&self, cancel: &CancellationToken) -> bool {
        let (Some(name), Some(saved)) = (self.state_text("rebase-merge/head-name"), self.state_text("rebase-merge/orig-head")) else {
            return false;
        };
        let name = name.trim();
        name.starts_with("refs/heads/")
            && self
                .runner
                .run(self.cwd(), &args(&["show-ref", "--verify", "--hash", "--", name]), Vec::new(), cancel)
                .is_ok_and(|output| output.status.success() && parse::line(&output.stdout) != saved.trim().as_bytes())
    }

    /// Git keeps `amend` for a planned edit and `stopped-sha` for a commit that stopped.
    /// Neither means it stopped between commits.
    fn rebase_stop(&self, cancel: &CancellationToken) -> StepKind {
        if self.rebase_branch_moved(cancel) {
            return StepKind::BranchMoved;
        }
        let directory = self.info().git_dir.join("rebase-merge");
        let stopped = self.state_text("rebase-merge/stopped-sha");
        let pending = self.state_text(&format!("rebase-merge/{}", crate::rebase_plan::MESSAGE_FILE));
        let same = stopped.as_deref().map(str::trim).filter(|s| !s.is_empty()).zip(pending.as_deref()).is_some_and(|(id, marker)| marker.lines().next() == Some(id));
        if same {
            StepKind::Message
        } else if directory.join("amend").exists() {
            StepKind::Edit
        } else if stopped.is_some() {
            StepKind::Commit
        } else {
            StepKind::Between
        }
    }

    fn operation_refs(&self, cancel: &CancellationToken) -> Result<[bool; 3]> {
        // Exact checks are intentional: enumeration can omit dangling symbolic
        // root refs, and revision lookup can confuse these with branches/tags.
        // Only these three probes run concurrently, sharing cancellation. Scoped
        // workers are joined on every exit, including errors.
        std::thread::scope(|scope| {
            let autostash = std::thread::Builder::new()
                .name("rocket-operation-autostash".into())
                .spawn_scoped(scope, || {
                    self.operation_ref_exists("MERGE_AUTOSTASH", cancel)
                })
                .map_err(|e| Error::io("start operation state worker", e))?;
            let cherry = std::thread::Builder::new()
                .name("rocket-operation-cherry".into())
                .spawn_scoped(scope, || {
                    self.operation_ref_exists("CHERRY_PICK_HEAD", cancel)
                })
                .map_err(|e| Error::io("start operation state worker", e))?;
            let revert = self.operation_ref_exists("REVERT_HEAD", cancel);
            let autostash = autostash.join().map_err(|_| Error::WorkerPanicked)?;
            let cherry = cherry.join().map_err(|_| Error::WorkerPanicked)?;
            Ok([autostash?, cherry?, revert?])
        })
    }

    fn operation_ref_exists(&self, name: &str, cancel: &CancellationToken) -> Result<bool> {
        // Exact ref lookup: rev-parse would also resolve a branch or tag named
        // CHERRY_PICK_HEAD etc., incorrectly reporting an active operation.
        let output = self.runner.run(
            self.cwd(),
            &args(&["show-ref", "--exists", "--", name]),
            Vec::new(),
            cancel,
        )?;
        match output.status.code() {
            Some(0) => Ok(true),
            Some(2) => Ok(false),
            _ => output.success("operation state").map(|_| unreachable!()),
        }
    }

    fn operation_ids(&self, name: &str, cancel: &CancellationToken) -> Result<Vec<ObjectId>> {
        cancel.check()?;
        let file = match File::open(self.info().git_dir.join(name)) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(Error::io("open operation marker", error)),
        };
        // Normal markers contain one ID; allow octopus merges created by another
        // client, but never read an unbounded repository-controlled file.
        let mut bytes = Vec::new();
        file.take(64 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| Error::io("read operation marker", e))?;
        if bytes.is_empty() || bytes.len() > 64 * 1024 {
            return Err(Error::parse("operation state", "empty or oversized marker"));
        }
        let ids = parse::line(&bytes)
            .split(|b| *b == b'\n')
            .map(|line| parse::oid(line, "operation state"))
            .collect::<Result<Vec<_>>>()?;
        cancel.check()?;
        Ok(ids)
    }

    pub(crate) fn require_idle(&self, cancel: &CancellationToken) -> Result<()> {
        if !self.operation_state(cancel)?.is_idle() {
            return Err(Error::InvalidInput(
                "finish or abort the active Git operation first".into(),
            ));
        }
        Ok(())
    }
}
