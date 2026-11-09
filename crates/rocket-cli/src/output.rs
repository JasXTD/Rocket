use rocket_git::*;
use serde_json::{Value, json};
use std::{io, path::Path};

const PREFIXED_CODES: &[&str] = &[
    "workflow_review_stale", "commit_push_remote", "commit_push_branch", "commit_push_conflicts",
    "commit_push_empty", "commit_push_limit", "commit_push_stale", "commit_push_unconfirmed",
    "commit_push_commit_changed", "commit_push_destination_changed", "commit_push_incomplete",
    "saved_work_branch", "saved_work_conflicts", "saved_work_index", "saved_work_limit",
    "saved_work_same_branch", "saved_work_stale", "saved_work_unknown", "saved_work_changed",
    "saved_work_switch_changed", "saved_work_paused",
    "stash_control_stale", "stash_selection_stale", "branch_fast_forward",
    "remote_cleanup_stale", "remote_cleanup_mapping", "remote_cleanup_response",
    "remote_branch_changed", "remote_branch_default", "remote_branch_diagnostics",
    "remote_branch_missing", "remote_branch_response",
    "remote_rename_collision", "remote_rename_name", "remote_rename_stale",
];

#[derive(Debug)]
pub struct Problem {
    pub value: Value,
    pub exit: u8,
}
impl Problem {
    pub fn new(code: &str, message: impl Into<String>, guidance: &str, exit: u8) -> Self {
        Self {
            value: json!({"code":code,"message":message.into(),"guidance":guidance}),
            exit,
        }
    }
    pub fn usage(message: &str) -> Self {
        Self::new(
            "usage",
            message,
            "Run rocket-cli --help for syntax. No Git operation was started.",
            2,
        )
    }
    pub fn backend(error: &Error) -> Self {
        if let Some(f) = error.clone_failure() {
            return Self::new(f.code(), f.summary(), f.guidance(), 1);
        }
        if let Some(f) = error.fetch_failure() {
            return Self::new(f.code(), f.summary(), f.guidance(), 1);
        }
        if let Some(f) = error.push_failure() {
            return Self::new(f.code(), f.summary(), f.guidance(), 1);
        }
        if let Some(f) = error.pull_failure() {
            return Self::new(f.code(), f.summary(), f.guidance(), 1);
        }
        if let Some(f) = error.lfs_lock_failure() {
            return Self::new(f.code(), "The LFS lock request did not complete.", f.guidance(), 1);
        }
        if let Some(reason) = rocket_git::pattern_error(error) {
            let mut problem = Self::new(
                "invalid_pattern",
                format!("Not a valid regular expression: {reason}"),
                "Fix the expression, or leave out --regex to search for the text as typed. Nothing was searched.",
                1,
            );
            problem.value["reason"] = json!(reason);
            return problem;
        }
        match error {
            Error::ForcePush(f) => Self::new(f.code(), "Reviewed push could not complete.", f.guidance(), 1),
            Error::AtomicPush(f) => Self::new(f.code(), "Atomic push could not complete.", f.guidance(), 1),
            Error::Tag(f) => Self::new(f.code(), "Tag operation could not complete.", f.guidance(), 1),
            Error::Worktree(f) => Self::new(f.code(), "Worktree operation could not complete.", f.guidance(), 1),
            Error::Bisect(f) => Self::new(f.code(), "Bisect could not continue.", f.guidance(), 1),
            Error::Conflict(f) => Self::new(f.code(), "Conflict resolution could not complete.", f.guidance(), 1),
            Error::Patch(f) => Self::new(f.code(), "The selected changes could not be applied.", f.guidance(), 1),
            Error::FileEdit(f) => Self::new(f.code(), "The working file could not be updated.", f.guidance(), 1),
            Error::CommitAction(f) => Self::new(f.code(), "The commit operation could not complete.", f.guidance(), 1),
            Error::Recovery(f) => Self::new(f.code(), "Recovery could not complete.", f.guidance(), 1),
            Error::Signing(f) => Self::new(f.code(), "Signing or verification could not complete.", f.guidance(), 1),
            Error::Lfs(f) => Self::new(f.code(), "Git LFS could not complete the operation.", f.guidance(), 1),
            Error::Ssh(f) => Self::new(f.code(), "The SSH key operation could not complete.", f.guidance(), 1),
            Error::Forge(f) => Self::new(f.code(), "The hosting operation could not complete.", f.guidance(), 1),
            Error::SecretProtection(failure) => {
                let mut problem = Self::new(failure.code(), failure.to_string(), failure.guidance(), 1);
                problem.value["operation_started"] = json!(false);
                problem.value["retry_requires_changes"] = json!(true);
                problem.value["location"] = json!(failure.location());
                problem.value["line"] = json!(failure.line());
                problem
            }
            Error::Cancelled => Self::new(
                "cancelled",
                "Cancellation completed.",
                "Inspect the final observations before retrying. Cancellation does not roll back writes.",
                130,
            ),
            Error::TimedOut => Self::new(
                "timed_out",
                "Git exceeded its command time limit.",
                "Inspect repository/remote state before retrying. Use --timeout-secs to change the limit.",
                124,
            ),
            Error::InvalidInput(message) => Self::new(
                // Some backend refusals start with a fixed code, as in "stash_control_stale: ..."
                // or "[undo_head_changed] ...".
                rocket_git::undo_failure_code(message)
                    .or_else(|| PREFIXED_CODES.iter().copied().find(|code| {
                        message.strip_prefix(code).is_some_and(|rest| rest.starts_with(':'))
                    }))
                    .unwrap_or("invalid_input"),
                message,
                "Correct the input and refresh the repository before retrying.",
                1,
            ),
            Error::Unsupported(message) => Self::new(
                "unsupported",
                message,
                "Use a supported operation or configuration; no automatic fallback was used.",
                1,
            ),
            Error::RepositoryBusy => Self::new(
                "repository_busy",
                "Another Rocket window or rocket-cli is writing to this repository or a linked worktree.",
                "Wait for that operation to finish, then refresh and retry. This write did not start; do not delete Git lock files.",
                1,
            ),
            Error::WorktreeRequired => Self::new(
                "worktree_required",
                "This operation needs a working tree.",
                "Choose a non-bare repository with --repo.",
                1,
            ),
            Error::Io { action, source } => {
                let mut p = Self::new(
                    "io_error",
                    format!("Could not {action}."),
                    "Check the selected path, permissions and Git executable.",
                    1,
                );
                p.value["os_error"] = json!(source.raw_os_error());
                p.value["io_kind"] = json!(format!("{:?}", source.kind()));
                p
            }
            Error::Git {
                operation,
                code,
                stdout,
                stderr,
            } => {
                // Classify only fixed Git diagnostics. Never echo hook/remote output,
                // which can contain credentials or terminal escape sequences.
                let raw = String::from_utf8_lossy(stderr).to_ascii_lowercase();
                let out = String::from_utf8_lossy(stdout).to_ascii_lowercase();
                let (kind, message, guidance) = if raw.contains("author identity unknown")
                    || raw.contains("unable to auto-detect email")
                    || raw.contains("committer identity unknown")
                {
                    (
                        "identity_required",
                        "Git author/committer identity is missing.",
                        "Set repository-local user.name and user.email, then retry the commit.",
                    )
                } else if out.contains("nothing to commit")
                    || out.contains("no changes added to commit")
                {
                    (
                        "nothing_to_commit",
                        "There are no staged changes to commit.",
                        "Use status and stage the intended files first.",
                    )
                } else if raw.contains(".lock") && raw.contains("file exists") {
                    (
                        "repository_locked",
                        "Git reported an existing lock.",
                        "Wait for the owning Git operation. Do not automatically delete lock files.",
                    )
                } else if raw.contains("would be overwritten") {
                    (
                        "local_changes",
                        "Git refused to overwrite local changes.",
                        "Commit or stash the affected changes, then retry.",
                    )
                } else if raw.contains("not fully merged") {
                    (
                        "branch_not_merged",
                        "The branch has unmerged commits.",
                        "Review and merge or preserve its commits before deleting it.",
                    )
                } else if *operation == "open" || raw.contains("not a git repository") {
                    (
                        "not_repository",
                        "The selected folder is not a Git repository.",
                        "Choose an existing Git repository with --repo, or use init.",
                    )
                } else {
                    (
                        "git_failed",
                        "Git refused or failed the operation.",
                        "Inspect status and the operation's structured observations. Raw helper/hook output is omitted; do not retry writes automatically.",
                    )
                };
                let mut p = Self::new(kind, message, guidance, 1);
                p.value["operation"] = json!(operation);
                p.value["git_exit_code"] = json!(code);
                p
            }
            Error::Rebase(f) => Self::new(f.code(), "Rebase needs attention.", f.guidance(), 1),
            Error::Merge(f) => Self::new(f.code(), "Merge could not complete.", f.guidance(), 1),
            Error::OutputLimit { stream, limit } => {
                let mut p = Self::new(
                    "output_limit",
                    "Git output exceeded its bounded limit.",
                    "Narrow the diff/history selection. Writes may already have changed the repository.",
                    1,
                );
                p.value["stream"] = json!(stream);
                p.value["limit_bytes"] = json!(limit);
                p
            }
            Error::InvalidOutput { operation, .. } => {
                let mut p = Self::new(
                    "invalid_git_output",
                    "Git returned output Rocket could not parse.",
                    "Inspect repository state before retrying. Parsing failure does not undo a write.",
                    1,
                );
                p.value["operation"] = json!(operation);
                p
            }
            Error::WorkerPanicked | Error::StatePoisoned(_) => Self::new(
                "internal_error",
                "An internal worker failed.",
                "Preserve the diagnostics and inspect repository state before retrying.",
                1,
            ),
            _ => Self::new(
                "backend_error",
                "The backend could not complete this operation.",
                "Inspect the structured report and repository state before retrying.",
                1,
            ),
        }
    }
}

pub struct Outcome {
    pub data: Value,
    pub error: Option<Problem>,
    pub warnings: Vec<Value>,
}
impl Outcome {
    pub fn success(data: Value) -> Self {
        Self {
            data,
            error: None,
            warnings: vec![],
        }
    }
    pub fn failure(error: Problem) -> Self {
        Self {
            data: Value::Null,
            error: Some(error),
            warnings: vec![],
        }
    }
    pub fn primary<T>(&mut self, result: &Result<T>) {
        if let Err(error) = result {
            self.error = Some(Problem::backend(error));
        }
    }
    pub fn observe<T>(
        &mut self,
        label: &str,
        result: &Result<T>,
        project: impl FnOnce(&T) -> Value,
    ) -> Value {
        if let Err(error) = result {
            self.warnings.push(json!({"code":"observation_failed","observation":label,"error":Problem::backend(error).value,
                "message":"The command outcome is unchanged; inspect state before retrying."}));
        }
        observation(result, project)
    }
    pub fn advisory(&mut self, yes: bool) {
        if yes {
            self.warnings.push(json!({"code":"git_diagnostics","message":"Git reported error diagnostics despite a successful command; inspect local and remote state before retrying."}));
        }
    }
    pub fn envelope(&self, command: &str, diagnostics: Value) -> Value {
        json!({"schema_version":1,"version":env!("CARGO_PKG_VERSION"),"command":command,
            "ok":self.error.is_none(),"data":self.data,"error":self.error.as_ref().map(|e| &e.value),
            "warnings":self.warnings,"diagnostics":diagnostics})
    }
    pub fn exit(&self) -> u8 {
        self.error.as_ref().map_or(0, |p| p.exit)
    }
}

pub fn observation<T>(result: &Result<T>, project: impl FnOnce(&T) -> Value) -> Value {
    match result {
        Ok(value) => json!({"ok":true,"data":project(value),"error":null}),
        Err(error) => json!({"ok":false,"data":null,"error":Problem::backend(error).value}),
    }
}
pub fn optional<T>(result: &Option<Result<T>>, project: impl FnOnce(&T) -> Value) -> Value {
    result
        .as_ref()
        .map_or(Value::Null, |r| observation(r, project))
}

/// Lossless Git bytes. Normal Unicode stays readable; invalid UTF-8 is explicit.
pub fn bytes(value: &[u8]) -> Value {
    match std::str::from_utf8(value) {
        Ok(text) => json!({"text":text,"hex":null}),
        Err(_) => {
            json!({"text":null,"hex":value.iter().map(|b| format!("{b:02x}")).collect::<String>()})
        }
    }
}
pub fn os_path(path: &Path) -> Value {
    // JSON cannot represent unpaired Windows surrogates. Keep exact UTF-16 units.
    if let Some(s) = path.to_str() {
        return json!({"text":s,"hex":null});
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        json!({"text":null,"utf16":path.as_os_str().encode_wide().collect::<Vec<_>>()})
    }
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        bytes(path.as_os_str().as_bytes())
    }
    #[cfg(not(any(windows, unix)))]
    {
        json!({"text":null})
    }
}
pub fn info(info: &RepositoryInfo) -> Value {
    json!({"worktree":info.worktree.as_deref().map(os_path),"git_dir":os_path(&info.git_dir),
        "common_dir":os_path(&info.common_dir),"object_format":match info.object_format {ObjectFormat::Sha1=>"sha1",ObjectFormat::Sha256=>"sha256"},"shallow":info.is_shallow})
}
pub fn head(h: &Head) -> Value {
    match h {
        Head::Unborn { branch } => json!({"kind":"unborn","name":bytes(branch),"target":null}),
        Head::Branch { name, target } => {
            json!({"kind":"branch","name":bytes(name),"target":target.as_str()})
        }
        Head::Detached { target } => {
            json!({"kind":"detached","name":null,"target":target.as_str()})
        }
    }
}
pub fn change(c: Change) -> &'static str {
    match c {
        Change::Unchanged => "unchanged",
        Change::Added => "added",
        Change::Modified => "modified",
        Change::Deleted => "deleted",
        Change::Renamed => "renamed",
        Change::Copied => "copied",
        Change::TypeChanged => "type_changed",
        Change::Unmerged => "unmerged",
    }
}
pub fn status(s: &Status) -> Value {
    json!({"head":head(&s.head),"upstream":s.upstream.as_deref().map(bytes),
        "ahead_behind":s.ahead_behind.map(|(ahead,behind)|json!({"ahead":ahead,"behind":behind})),
        "entries":s.entries.iter().map(|e|json!({"path":bytes(e.path.as_bytes()),"original_path":e.original_path.as_ref().map(|p|bytes(p.as_bytes())),
            "kind":match e.kind {EntryKind::Tracked=>"tracked",EntryKind::RenameOrCopy=>"rename_or_copy",EntryKind::Unmerged=>"unmerged",EntryKind::Untracked=>"untracked",EntryKind::Ignored=>"ignored"},
            "index":change(e.index),"worktree":change(e.worktree),"similarity":e.similarity,
            "submodule":e.submodule.map(|s|json!({"commit_changed":s.commit_changed,"tracked_changes":s.tracked_changes,"untracked_changes":s.untracked_changes}))})).collect::<Vec<_>>()})
}
pub fn branches(values: &[Branch]) -> Value {
    json!(values.iter().map(|b|json!({"reference":bytes(&b.reference),"target":b.target.as_str(),"kind":match b.kind {BranchKind::Local=>"local",BranchKind::Remote=>"remote"},"is_head":b.is_head,
        "upstream":b.upstream.as_deref().map(bytes),"symbolic_target":b.symbolic_target.as_deref().map(bytes),"checked_out_at":b.checked_out_at.as_deref().map(os_path)})).collect::<Vec<_>>())
}
fn signature(s: &Signature) -> Value {
    json!({"name":bytes(&s.name),"email":bytes(&s.email),"timestamp":s.timestamp,"offset_minutes":s.offset_minutes})
}
pub fn commit(c: &Commit) -> Value {
    json!({"id":c.id.as_str(),"tree":c.tree.as_str(),"parents":c.parents.iter().map(ObjectId::as_str).collect::<Vec<_>>(),"author":signature(&c.author),"committer":signature(&c.committer),"encoding":c.encoding.as_deref().map(bytes),"message":bytes(&c.message)})
}
pub fn history(page: &HistoryPage) -> Value {
    json!({"order":page.order.key(),"commits":page.commits.iter().map(commit).collect::<Vec<_>>(),"has_more":page.next.is_some()})
}
/// A cherry-pick, revert or rebase content prediction, as `pull` reports it.
pub fn prediction(p: &CommitPrediction) -> Value {
    json!({"outcome":p.kind.code(),"checked":p.checked,"total":p.total,"stopped_at":p.stopped_at,
        "paths":p.paths.iter().map(|path|bytes(path.as_bytes())).collect::<Vec<_>>(),"reason":p.reason.map(|v|v.code()),"guidance":p.reason.map(|v|v.guidance())})
}
pub fn stashes(values: &[StashEntry]) -> Value {
    json!(
        values
            .iter()
            .map(|s| json!({"index":s.index,"id":s.id.as_str(),"subject":bytes(&s.subject)}))
            .collect::<Vec<_>>()
    )
}
pub fn operation(s: &OperationState) -> Value {
    json!({"idle":s.is_idle(),"bisect":s.bisect,"merge_heads":s.merge_heads.iter().map(ObjectId::as_str).collect::<Vec<_>>(),"merge_committed":s.merge_committed,"merge_autostash":s.merge_autostash.as_ref().map(ObjectId::as_str),"rebase":s.rebase,"mail_import":s.mail_import,"cherry_pick":s.cherry_pick,"revert":s.revert,"sequencer":s.sequencer})
}
pub fn remotes(values: &[Remote]) -> Value {
    // URLs may contain credentials, including custom helper formats. Do not try
    // to redact an unknown URL grammar; expose names/mappings and counts only.
    json!(values.iter().map(|r|json!({"name":bytes(&r.name),"fetch_url_count":r.urls.len(),"push_url_count":r.push_urls.len(),"fetch_refspecs":r.fetch_refspecs.iter().map(|s|bytes(s)).collect::<Vec<_>>(),"urls_omitted":true})).collect::<Vec<_>>())
}
pub fn write_output(output: &WriteOutput) -> Value {
    json!({"stdout_bytes":output.stdout.len(),"stderr_bytes":output.stderr.len(),"raw_output_omitted":true})
}

pub fn display_safe(text: &str) -> String {
    text.chars()
        .flat_map(|c| {
            if (c.is_control() && c != '\n' && c != '\t')
                || matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
            {
                c.escape_default().to_string().chars().collect::<Vec<_>>()
            } else {
                vec![c]
            }
        })
        .collect()
}

fn field(value: &Value) -> String {
    value["text"]
        .as_str()
        .map(|s| display_safe(s).replace('\n', "\\n").replace('\t', "\\t"))
        .unwrap_or_else(|| "[non-Unicode value; use --json]".into())
}

fn render_status(out: &mut impl io::Write, data: &Value) -> io::Result<()> {
    match data["head"]["kind"].as_str() {
        Some("unborn") => writeln!(
            out,
            "On branch {} (no commits yet)",
            field(&data["head"]["name"])
        )?,
        Some("branch") => writeln!(
            out,
            "On branch {} at {}",
            field(&data["head"]["name"]),
            data["head"]["target"].as_str().unwrap_or("")
        )?,
        Some("detached") => writeln!(
            out,
            "Detached HEAD at {}",
            data["head"]["target"].as_str().unwrap_or("")
        )?,
        _ => (),
    }
    if let Some(entries) = data["entries"].as_array() {
        if entries.is_empty() {
            writeln!(out, "Working tree clean")?;
        } else {
            writeln!(out, "{:<14} {:<14} PATH", "INDEX", "WORKTREE")?;
            for entry in entries {
                let working = if entry["kind"] == "untracked" {
                    "untracked"
                } else {
                    entry["worktree"].as_str().unwrap_or("")
                };
                writeln!(
                    out,
                    "{:<14} {:<14} {}",
                    entry["index"].as_str().unwrap_or(""),
                    working,
                    field(&entry["path"])
                )?;
            }
        }
    }
    Ok(())
}

pub fn render(
    out: &mut impl io::Write,
    outcome: &Outcome,
    command: &str,
    diagnostics: Value,
    as_json: bool,
) -> io::Result<()> {
    if as_json {
        serde_json::to_writer(&mut *out, &outcome.envelope(command, diagnostics))?;
        return writeln!(out);
    }
    if let Some(error) = &outcome.error {
        writeln!(
            out,
            "{}: {}",
            error.value["code"].as_str().unwrap_or("error"),
            display_safe(
                error.value["message"]
                    .as_str()
                    .unwrap_or("Operation failed")
            )
        )?;
        writeln!(
            out,
            "{}",
            display_safe(error.value["guidance"].as_str().unwrap_or(""))
        )?;
    }
    if !outcome.data.is_null() {
        let data = &outcome.data;
        match command {
            "help" => write!(out, "{}", crate::args::HELP)?,
            "version" => writeln!(
                out,
                "rocket-cli {} (JSON schema 1)",
                env!("CARGO_PKG_VERSION")
            )?,
            "status" => render_status(out, data)?,
            "info" => {
                writeln!(
                    out,
                    "Worktree: {}",
                    if data["worktree"].is_null() {
                        "(bare)".into()
                    } else {
                        field(&data["worktree"])
                    }
                )?;
                writeln!(out, "Git directory: {}", field(&data["git_dir"]))?;
                writeln!(
                    out,
                    "Object format: {}",
                    data["object_format"].as_str().unwrap_or("")
                )?;
            }
            "branches" => {
                if let Some(values) = data.as_array() {
                    for b in values {
                        writeln!(
                            out,
                            "{} {} {}",
                            if b["is_head"] == true { "*" } else { " " },
                            field(&b["reference"]),
                            b["target"].as_str().unwrap_or("")
                        )?;
                    }
                }
            }
            "stashes" => {
                if let Some(values) = data.as_array() {
                    for s in values {
                        writeln!(
                            out,
                            "{} {} {}",
                            s["index"],
                            s["id"].as_str().unwrap_or(""),
                            field(&s["subject"])
                        )?;
                    }
                }
            }
            "remotes" => {
                if let Some(values) = data.as_array() {
                    for r in values {
                        writeln!(
                            out,
                            "{} ({} fetch / {} explicit push URLs; URLs omitted)",
                            field(&r["name"]),
                            r["fetch_url_count"],
                            r["push_url_count"]
                        )?;
                    }
                }
            }
            "diff" if data["patch"]["text"].is_string() => write!(
                out,
                "{}",
                display_safe(data["patch"]["text"].as_str().unwrap())
            )?,
            "history" => {
                if let Some(commits) = data["commits"].as_array() {
                    for c in commits {
                        writeln!(
                            out,
                            "{} {}",
                            c["id"].as_str().unwrap_or(""),
                            display_safe(
                                c["message"]["text"]
                                    .as_str()
                                    .unwrap_or("[non-UTF-8 message; use --json]")
                                    .lines()
                                    .next()
                                    .unwrap_or("")
                            )
                        )?;
                    }
                }
                if data["has_more"] == true {
                    writeln!(
                        out,
                        "More commits exist; increase --limit or select --revision."
                    )?;
                }
            }
            // One line per match, as git grep prints them.
            "file-search" => {
                for file in data["files"].as_array().into_iter().flatten() {
                    let path = field(&file["path"]);
                    for line in file["lines"].as_array().into_iter().flatten() {
                        writeln!(out, "{path}:{}: {}", line["line"], display_safe(line["preview"].as_str().unwrap_or("")))?;
                    }
                    if file["capped"] == true {
                        writeln!(out, "{path}: more matching lines; only the first {} are shown", rocket_git::FILE_SEARCH_FILE_MATCHES)?;
                    }
                }
                match data["end"].as_str() {
                    Some("too_many") => writeln!(out, "Stopped at the match limit; narrow the search with --path or a longer pattern.")?,
                    Some("timed_out") => writeln!(out, "Stopped at the time limit; only files before that were searched.")?,
                    _ => (),
                }
            }
            "clone" | "init" => {
                writeln!(
                    out,
                    "{}: {}",
                    if data["published"] == true {
                        "Published"
                    } else {
                        "Not published"
                    },
                    field(&data["destination"])
                )?;
                if data["published"] != true {
                    writeln!(
                        out,
                        "Retained workspace: {}",
                        field(&data["staging_directory"])
                    )?;
                }
            }
            _ if data.get("command_result").is_some() || command == "pull" => {
                writeln!(
                    out,
                    "{command}: {}",
                    if outcome.error.is_none() {
                        "succeeded"
                    } else {
                        "failed or interrupted; inspect final state"
                    }
                )?;
                let prediction = &data["preview"]["prediction"];
                if let Some(kind) = prediction["outcome"].as_str() {
                    writeln!(
                        out,
                        "Content prediction: {} ({}/{} commits checked)",
                        display_safe(kind),
                        prediction["checked"],
                        prediction["total"]
                    )?;
                    if let Some(index) = prediction["stopped_at"].as_u64() {
                        writeln!(out, "First stop: replay commit {}", index + 1)?;
                    }
                    if let Some(paths) = prediction["paths"].as_array() {
                        for path in paths {
                            writeln!(out, "  {}", field(path))?;
                        }
                    }
                    if let Some(guidance) = prediction["guidance"].as_str() {
                        writeln!(out, "{}", display_safe(guidance))?;
                    }
                }
                if let Some(remote) = data["remote_outcome"].as_str() {
                    writeln!(out, "Remote outcome: {remote}")?;
                }
                if data["status"]["ok"] == true {
                    render_status(out, &data["status"]["data"])?;
                }
                if data["operation"]["data"]["idle"] == false {
                    writeln!(
                        out,
                        "A Git operation remains in progress. Use operation --json for recovery state."
                    )?;
                }
            }
            _ => {
                let text = serde_json::to_string_pretty(data)?;
                writeln!(out, "{}", display_safe(&text))?;
            }
        }
    }
    for warning in &outcome.warnings {
        writeln!(out, "Warning: {}", display_safe(&warning.to_string()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invalid_utf8_is_lossless_and_terminal_controls_are_inert() {
        assert_eq!(bytes(&[b'a', 0xff]), json!({"text":null,"hex":"61ff"}));
        assert_eq!(bytes("性能 🚀 é".as_bytes())["text"], "性能 🚀 é");
        assert_eq!(
            display_safe("x\x1b]52;c;secret\x07\u{202e}"),
            "x\\u{1b}]52;c;secret\\u{7}\\u{202e}"
        );
    }
    #[test]
    fn raw_git_and_credentials_do_not_leak_through_unknown_errors() {
        let p = Problem::backend(&Error::Git {
            operation: "commit",
            code: Some(1),
            stdout: b"PRIVATE".to_vec(),
            stderr: b"https://user:SECRET@example.invalid\x1b[2J".to_vec(),
        });
        assert!(!p.value.to_string().contains("SECRET"));
        assert!(!p.value.to_string().contains("PRIVATE"));
        assert_eq!(p.value["code"], "git_failed");
    }
    #[test]
    fn failed_observation_never_changes_successful_write_exit_code() {
        let mut o = Outcome::success(json!({"command_result":{"ok":true}}));
        let observed: Result<()> = Err(Error::TimedOut);
        let state = o.observe("status", &observed, |_| Value::Null);
        assert_eq!(o.exit(), 0);
        assert_eq!(state["ok"], false);
        assert_eq!(o.warnings.len(), 1);
    }
}
