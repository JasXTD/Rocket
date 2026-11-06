use std::{collections::BTreeSet, ffi::OsString, path::PathBuf};

pub const HELP: &str = "Rocket CLI — bundled or installed Git, Rocket's guarded backend

Usage: rocket-cli [GLOBAL OPTIONS] COMMAND [OPTIONS]

Global options (also accepted after a command, before --):
  --repo PATH          Repository to open (default: current directory)
  --json               One schema_version=1 JSON result on stdout
  --git PATH           Explicit Git executable (overrides bundled/PATH)
  --timeout-secs N     Per-Git-command limit, 1..86400 (default: 600)
  --cancel-after-ms N  Request cancellation after N ms; wait for final result
  --log-dir PATH       Dedicated diagnostics directory (absolute path)
  --no-log             Disable diagnostic logging
  --help, -h           Show this help without opening a repository
  --version, -V        Show version without opening a repository

Read commands:
  info | status | operation
  history [--all | --revision REV] [--order topo|commit-date|author-date] [--limit N]
  commit-url REV [--remote NAME] [--provider PROVIDER --server HTTPS_URL]
  review commit REV | review file REV --path PATH [--blame]
  diff [--staged | --base REV --target REV] [--context N] [-- PATH...]
  hunks [--staged] -- PATH (changed-line IDs and patch SHA-256 of one file)
  blame REV --path PATH [--first N] [--size N]
  file-history PATH [--revision REV] [--limit N]
  search TEXT [--kind text|path|content] [--all | --revision REV]
    [--regex] [--match-case] [--path PATH] (these three need --kind content)
  grep PATTERN [--revision REV] [--regex] [--match-case] [--path PATH] (lines in files)
  compare BASE (TARGET | --staged | --worktree)
  reflog [--scope head|all|deleted-branches|retained|unreachable] [--limit N]
  verify commit REV | verify tag NAME [--require-valid]
  signing show
  identity show

Local writes:
  init DESTINATION [--branch NAME] (default main; protects existing files)
  stage (--all | -- PATH...)
  stage --lines ID,... --expect-patch SHA256 -- PATH
  unstage (--all | -- PATH...)
  unstage --lines ID,... --expect-patch SHA256 -- PATH
  commit --message MESSAGE (JSON undo: values for undo-commit)
  undo-commit --branch NAME --expect COMMIT_ID --index-sha256 SHA256 [--preview]
  amend --message MESSAGE --expect HEAD_ID [--preview]
  cherry-pick | revert ID... [--mainline N] [--expect HEAD_ID] [--preview]
  cherry-pick | revert continue | skip | abort [--preview]
  reset --soft | --mixed | --hard ID --expect HEAD_ID [--preview]
  identity set [--name NAME] [--email EMAIL] [--preview] (repository-local user.name/email)
  branch list
  branch create NAME [--start REV]
  branch switch NAME
  branch rename OLD NEW
  branch delete NAME (merged branches only)
  branch fast-forward [--preview] [-- BRANCH...] (branches behind their upstream)
  stash list
  stash show FULL_ID [--part tracked|index|untracked]
  stash save [--message MESSAGE] [--include-untracked] [-- PATH...]
  stash save --message MESSAGE --lines ID,... --expect-patch SHA256 [--preview] -- PATH
  stash apply FULL_ID [--index]
  stash apply FULL_ID [--part PART] [--hunks N,... | --lines ID,... --expect-patch SHA256]
    [--preview] -- PATH... (applies selected changes; index and stash are kept)
  stash pop INDEX --expect FULL_ID [--preview] (clean working tree; keeps a backup ref)
  stash drop INDEX --expect FULL_ID
  merge start REV [--ff-only | --no-ff | --squash | --no-commit]
  merge continue --message MESSAGE
  merge abort
  rebase start TARGET [--update-refs | --preserve-merges] [--interactive] [--plan FILE]
    (--preview | --expect-original ID --expect-onto ID) (--plan implies --interactive)
  rebase continue | skip | abort [--preview] (recovery; skip discards stopped changes)

Conflicts:
  conflict show PATH
  conflict resolve PATH --expect-sha256 SHA256 (--choose N=SIDE,... | --choose all=SIDE
    | --result FILE) [--stage] [--preview] (SIDE is ours, theirs or both)
  conflict keep PATH --side ours|theirs|deletion --expect BLOB_ID|missing [--preview]
  conflict mark-resolved -- PATH (refuses remaining conflict markers)

Bisect:
  bisect status
  bisect start GOOD BAD [--preview]
  bisect good | bad | skip --expect COMMIT_ID [--preview]
  bisect resume | restore [--preview]
  bisect run (GOOD BAD | --continue) --test-timeout SECS (--preview | --expect-sha256 SHA256)
    -- PROGRAM [ARG...] (exit 0 good, 1-124 bad, 125 skip; absolute .exe outside the checkout)

Remotes:
  remote list
  remote add NAME URL
  remote set-url NAME URL [--push]
  remote remove NAME (local config/tracking refs only)
  remote rename OLD NEW [--preview]
  remote prune REMOTE [--preview] [-- REF...] (tracking refs the server no longer has)
  branch delete-remote REMOTE BRANCH --expect ID [--preview]
  fetch REMOTE [--tags] (--tags: all remote tags, never replacing a different local tag)
  pull [--ff-only | --merge | --rebase] [--preview]
    Configured upstream; default --ff-only. --preview fetches without integration.
  push REMOTE LOCAL_BRANCH REMOTE_BRANCH [--set-upstream]
  push-lease REMOTE LOCAL_BRANCH REMOTE_BRANCH --preview
  push-lease REMOTE LOCAL_BRANCH REMOTE_BRANCH --source LOCAL_ID --expect REMOTE_ID
  push-atomic REMOTE --preview -- LOCAL:REMOTE...
  push-atomic REMOTE -- LOCAL:REMOTE:SOURCE_ID:EXPECTED_ID_OR_missing...
  clone (--local PATH | --url URL) DESTINATION

Tags and worktrees:
  tag list [--limit N] [--order name|newest]
  tag inspect NAME
  tag create NAME REV [--message MESSAGE] [--sign] [--preview] (--sign needs --message)
  tag delete NAME --expect TAG_OBJECT_ID [--preview]
  tag remote REMOTE
  tag publish NAME --remote REMOTE --source TAG_OBJECT_ID [--preview]
  tag delete-remote NAME --remote REMOTE --expect REMOTE_TAG_ID [--preview]
  worktree list
  worktree create PATH --branch NAME [--start REV] [--existing] [--preview]
  worktree remove PATH --expect HEAD_ID [--preview]
  worktree move PATH DESTINATION --expect HEAD_ID [--preview]
  worktree repair PATH --at FOLDER --expect HEAD_ID [--preview] (registered folder moved)
  worktree prune PATH --expect HEAD_ID [--preview] (registered folder gone)

Git LFS and SSH keys:
  lfs status | lfs locks REMOTE
  lfs track PATTERN [--literal] [--preview] | lfs untrack --line N --expect TEXT [--preview]
  lfs fetch REMOTE | lfs push REMOTE | lfs checkout -- PATH [--preview]
  lfs lock PATH --remote REMOTE [--preview]
  lfs unlock PATH --remote REMOTE --expect LOCK_ID [--preview] (your own locks; no force)
  ssh keys | ssh check ALIAS (public keys and Rocket profiles; nothing is unlocked)
  ssh use ALIAS | ssh restore (--preview | --expect-config SHA256) (this repository's sshCommand)

Use -- before literal paths/names beginning with '-'. All path arguments to
stage/unstage/diff are relative to the repository root and never wildcards.
No unconditional force pushes, automatic stashing, shell commands or automatic retries.
bisect run starts only the reviewed PROGRAM, with literal arguments and no shell.
Ctrl+C/Ctrl+Break requests cancellation; a write may already have succeeded.
Exit: 0 success (check warnings), 1 operation failure, 2 usage, 124 timeout,
130 cancelled. JSON errors use the same envelope as successful results.
";

#[derive(Debug)]
pub struct Options {
    pub repo: PathBuf,
    pub json: bool,
    pub git: Option<PathBuf>,
    pub timeout_secs: u64,
    pub cancel_after_ms: Option<u64>,
    pub log_dir: Option<PathBuf>,
    pub no_log: bool,
    pub command: Command,
}

#[derive(Debug)]
pub enum Command {
    Help,
    Version,
    Info,
    Status,
    Operation,
    History {
        all: bool,
        revision: Option<String>,
        limit: usize,
        order: rocket_git::HistoryOrder,
    },
    CommitUrl { revision: String, remote: Option<String>, config: Option<rocket_git::ForgeConfig> },
    Review { revision: String, path: Option<OsString>, blame: bool },
    Diff {
        staged: bool,
        base: Option<String>,
        target: Option<String>,
        context: u16,
        paths: Vec<OsString>,
    },
    Hunks { path: OsString, staged: bool },
    Blame { revision: String, path: OsString, first: usize, size: usize },
    FileHistory { path: OsString, revision: Option<String>, limit: usize },
    /// `regex`, `match_case` and `path` apply to content searches only; the parser
    /// refuses them for the other kinds rather than ignoring them.
    Search {
        query: String, kind: rocket_git::HistorySearchKind, all: bool, revision: Option<String>,
        regex: bool, match_case: bool, path: Option<OsString>,
    },
    /// Lines in files: the checkout's files, or a commit's with `revision`.
    Grep { pattern: String, revision: Option<String>, regex: bool, match_case: bool, path: Option<OsString> },
    /// Without a target, compare BASE with the index (`staged`) or the working tree.
    Compare { base: String, target: Option<String>, staged: bool },
    Reflog { scope: rocket_git::ReflogScope, limit: usize },
    Verify { tag: bool, name: String, require_valid: bool },
    SigningShow,
    IdentityShow,
    /// Fields left out keep their current repository-local values.
    IdentitySet { name: Option<String>, email: Option<String>, preview: bool },
    Init {
        destination: PathBuf,
        branch: String,
    },
    Stage {
        all: bool,
        paths: Vec<OsString>,
        unstage: bool,
    },
    StageLines { path: OsString, lines: Vec<usize>, expected: String, unstage: bool },
    Commit(String),
    UndoCommit { branch: String, expected: String, index: String, preview: bool },
    /// `expected` pins the amended commit, or HEAD for cherry-pick and revert.
    CommitAction { request: rocket_git::CommitActionRequest, expected: Option<String>, message: Option<String>, preview: bool },
    Reset { mode: rocket_git::ResetMode, target: String, expected: String, preview: bool },
    BranchList,
    BranchCreate {
        name: OsString,
        start: Option<String>,
    },
    BranchSwitch(OsString),
    BranchRename(OsString, OsString),
    BranchDelete(OsString),
    StashList,
    StashSave {
        message: Option<String>,
        untracked: bool,
        paths: Vec<OsString>,
    },
    StashApply {
        id: String,
        index: bool,
    },
    StashDrop {
        index: usize,
        expected: String,
    },
    StashShow { id: String, part: rocket_git::StashPart },
    StashApplySelection {
        id: String, part: rocket_git::StashPart, paths: Vec<OsString>,
        hunks: Vec<usize>, lines: Vec<usize>, expected: Option<String>, preview: bool,
    },
    StashSaveLines { message: String, path: OsString, lines: Vec<usize>, expected: String, preview: bool },
    StashPop { index: usize, expected: String, preview: bool },
    MergeStart {
        revision: String,
        mode: rocket_git::MergeMode,
    },
    MergeContinue(String),
    MergeAbort,
    RebaseRecovery {
        action: rocket_git::RebaseAction,
        preview: bool,
    },
    /// `expected` is the reviewed (original branch tip, destination) pair. An
    /// interactive review lists every commit in the range, as a plan of picks.
    RebaseStart {
        target: String, interactive: bool, plan: Option<PathBuf>, update_refs: bool, preserve_merges: bool,
        expected: Option<(String, String)>, preview: bool,
    },
    ConflictShow(OsString),
    ConflictResolve { path: OsString, expected: String, choice: crate::conflicts::Choice, stage: bool, preview: bool },
    /// `expected` is the kept side's blob ID; None for a deletion ("missing").
    ConflictKeep { path: OsString, keep: rocket_git::ConflictKeep, expected: Option<String>, preview: bool },
    ConflictMarkResolved(OsString),
    BisectStatus,
    /// `expected` names the checked-out commit a good/bad/skip decision was tested on.
    Bisect { action: rocket_git::BisectAction, good: String, bad: String, expected: Option<String>, preview: bool },
    BisectRun {
        range: Option<(String, String)>, program: PathBuf, arguments: Vec<String>,
        timeout: u64, expected: Option<String>, preview: bool,
    },
    RemoteList,
    RemoteAdd(OsString, OsString),
    RemoteSetUrl {
        name: OsString,
        url: OsString,
        push: bool,
    },
    RemoteRemove(OsString),
    RemoteRename { old: String, new: String, preview: bool },
    /// Selected obsolete tracking refs, by full name; required unless previewing.
    RemotePrune { remote: String, refs: Vec<String>, preview: bool },
    /// `expected` is the server branch's advertised object, the deletion's lease.
    BranchDeleteRemote { remote: String, branch: String, expected: String, preview: bool },
    BranchFastForward { branches: Vec<String>, preview: bool },
    Fetch { remote: OsString, tags: bool },
    Pull {
        strategy: rocket_git::PullStrategy,
        preview: bool,
    },
    Push {
        remote: OsString,
        local: OsString,
        destination: OsString,
        upstream: bool,
    },
    PushLease {
        remote: String, local: String, destination: String,
        source: Option<String>, expected: Option<String>, preview: bool,
    },
    PushAtomic { remote: String, branches: Vec<crate::advanced::AtomicSelection>, preview: bool },
    TagList { limit: usize, order: rocket_git::TagOrder },
    TagInspect(String),
    TagCreate { name: String, revision: String, message: Option<String>, sign: bool, preview: bool },
    TagDelete { name: String, expected: String, preview: bool },
    TagRemote(String),
    TagPublish { name: String, remote: String, source: String, preview: bool },
    TagDeleteRemote { name: String, remote: String, expected: String, preview: bool },
    WorktreeList,
    WorktreeCreate { path: PathBuf, branch: String, start: String, existing: bool, preview: bool },
    WorktreeRemove { path: PathBuf, expected: String, preview: bool },
    /// Move needs the new folder; repair needs the folder the worktree now lives in.
    WorktreeMaintain { kind: rocket_git::WorktreeMaintenance, path: PathBuf, destination: Option<PathBuf>, expected: String, preview: bool },
    LfsStatus,
    /// Fetch, push, track and untrack; a checkout names a repository path instead.
    LfsChange { action: rocket_git::LfsAction, preview: bool },
    LfsCheckout { path: OsString, preview: bool },
    LfsLocks(String),
    /// `unlock` holds the expected lock ID; None locks the path.
    LfsLock { path: String, remote: String, unlock: Option<String>, preview: bool },
    SshKeys,
    SshCheck(String),
    /// None restores the setting this repository had before Rocket's last binding.
    SshBinding { alias: Option<String>, expected: Option<String>, preview: bool },
    Clone {
        source: rocket_git::CloneSource,
        destination: PathBuf,
    },
}

impl Command {
    /// Fixed names also match the backend's diagnostic allowlist.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Help => "help",
            Self::Version => "version",
            Self::Info => "info",
            Self::Status => "status",
            Self::Operation => "operation",
            Self::History { .. } => "history",
            Self::CommitUrl { .. } => "commit-url",
            Self::Review { .. } => "review",
            Self::Diff { .. } => "diff",
            Self::Hunks { .. } => "hunks",
            Self::Blame { .. } => "file-blame",
            Self::FileHistory { .. } => "file-history",
            Self::Search { .. } => "history-search",
            Self::Grep { .. } => "file-search",
            Self::Compare { .. } => "compare",
            Self::Reflog { .. } => "reflog",
            Self::Verify { .. } => "signature-verify",
            Self::SigningShow => "signing-read",
            // A preview of `identity set` only reads.
            Self::IdentityShow => "identity-read",
            Self::IdentitySet { preview, .. } => if *preview { "identity-read" } else { "identity-save" },
            Self::Init { .. } => "init",
            Self::Stage { all, unstage, .. } => match (*unstage, *all) {
                (false, false) => "stage",
                (false, true) => "stage-all",
                (true, false) => "unstage",
                (true, true) => "unstage-all",
            },
            Self::StageLines { unstage, .. } => if *unstage { "unstage-selection" } else { "stage-selection" },
            Self::Commit(_) => "commit",
            Self::UndoCommit { preview, .. } => if *preview { "undo-preview" } else { "undo-commit" },
            Self::CommitAction { request, preview, .. } => if *preview { "commit-preview" } else { request.label() },
            Self::Reset { mode, preview, .. } => if *preview { "recovery-preview" } else { mode.label() },
            Self::BranchList => "branches",
            Self::BranchCreate { .. } => "branch-create",
            Self::BranchSwitch(_) => "branch-switch",
            Self::BranchRename(..) => "branch-rename",
            Self::BranchDelete(_) => "branch-delete",
            Self::StashList => "stashes",
            Self::StashSave { .. } => "stash-push",
            Self::StashApply { .. } => "stash-apply",
            Self::StashDrop { .. } => "stash-drop",
            Self::StashShow { .. } => "stash-show",
            Self::StashApplySelection { preview, .. } => if *preview { "stash-apply-preview" } else { "stash-apply-selection" },
            Self::StashSaveLines { preview, .. } => if *preview { "stash-selection-preview" } else { "stash-selection" },
            Self::StashPop { preview, .. } => if *preview { "stash-control-preview" } else { "stash-pop" },
            Self::MergeStart { .. } => "merge",
            Self::MergeContinue(_) => "merge-continue",
            Self::MergeAbort => "merge-abort",
            Self::RebaseRecovery { action, .. } => action.label(),
            Self::RebaseStart { interactive, preview, .. } => match (preview, interactive) {
                (true, _) => "rebase-preview",
                (false, true) => "interactive-rebase",
                (false, false) => "rebase",
            },
            Self::ConflictShow(_) => "conflict-show",
            Self::ConflictResolve { preview, .. } => if *preview { "conflict-preview" } else { "conflict-save" },
            Self::ConflictKeep { preview, .. } => if *preview { "conflict-preview" } else { "conflict-keep" },
            Self::ConflictMarkResolved(_) => "mark-resolved",
            Self::BisectStatus => "bisect-status",
            Self::Bisect { action, preview, .. } => if *preview { "bisect-preview" } else { action.label() },
            Self::BisectRun { preview, .. } => if *preview { "bisect-auto-preview" } else { "bisect-auto" },
            Self::RemoteList => "remotes",
            Self::RemoteAdd(..) => "remote-add",
            Self::RemoteSetUrl { .. } => "remote-edit",
            Self::RemoteRemove(_) => "remote-remove",
            Self::RemoteRename { preview, .. } => if *preview { "remote-rename-preview" } else { "remote-rename" },
            Self::RemotePrune { preview, .. } => if *preview { "remote-cleanup-preview" } else { "remote-prune" },
            Self::BranchDeleteRemote { preview, .. } => if *preview { "remote-branch-delete-preview" } else { "remote-branch-delete" },
            Self::BranchFastForward { preview, .. } => if *preview { "branch-fast-forward-preview" } else { "branch-fast-forward" },
            Self::Fetch { .. } => "fetch",
            Self::Pull { .. } => "pull",
            Self::Push { .. } => "push",
            Self::PushLease { preview, .. } => if *preview { "force-push-preview" } else { "force-push" },
            Self::PushAtomic { preview, .. } => if *preview { "atomic-push-preview" } else { "atomic-push" },
            Self::TagList { .. } => "tags",
            Self::TagInspect(_) => "tag-inspect",
            Self::TagCreate { preview, sign, .. } => match (preview, sign) {
                (true, _) => "tag-preview",
                (false, true) => "tag-sign",
                (false, false) => "tag-create",
            },
            Self::TagDelete { preview, .. } => if *preview { "tag-preview" } else { "tag-delete" },
            Self::TagRemote(_) => "remote-tags",
            Self::TagPublish { preview, .. } => if *preview { "remote-tag-preview" } else { "remote-tag-publish" },
            Self::TagDeleteRemote { preview, .. } => if *preview { "remote-tag-preview" } else { "remote-tag-delete" },
            Self::WorktreeList => "worktrees",
            Self::WorktreeCreate { preview, .. } => if *preview { "worktree-preview" } else { "worktree-create" },
            Self::WorktreeRemove { preview, .. } => if *preview { "worktree-preview" } else { "worktree-remove" },
            Self::WorktreeMaintain { kind, preview, .. } => if *preview { "worktree-preview" } else { kind.label() },
            Self::LfsStatus => "lfs-status",
            Self::LfsChange { action, preview } => if *preview { "lfs-preview" } else { action.label() },
            Self::LfsCheckout { preview, .. } => if *preview { "lfs-preview" } else { "lfs-checkout" },
            Self::LfsLocks(_) => "lfs-locks",
            Self::LfsLock { unlock, preview, .. } => match (preview, unlock.is_some()) {
                (true, _) => "lfs-lock-preview",
                (false, true) => "lfs-unlock",
                (false, false) => "lfs-lock",
            },
            Self::SshKeys => "ssh-discover",
            Self::SshCheck(_) => "ssh-check",
            Self::SshBinding { preview, .. } => if *preview { "ssh-binding-review" } else { "ssh-binding-save" },
            Self::Clone { .. } => "clone",
        }
    }
}

type Parsed<T> = Result<T, &'static str>;

fn value_option(s: &str) -> bool {
    matches!(
        s,
        "-m" | "--message"
            | "--start"
            | "--revision"
            | "--limit"
            | "--context"
            | "--base"
            | "--target"
            | "--expect"
            | "--branch"
            | "--local"
            | "--url"
            | "--source" | "--order" | "--remote" | "--provider" | "--server" | "--path"
            | "--lines" | "--hunks" | "--part" | "--expect-patch" | "--expect-sha256"
            | "--choose" | "--result" | "--side" | "--test-timeout"
            | "--mainline" | "--plan" | "--expect-original" | "--expect-onto" | "--index-sha256"
            | "--scope" | "--kind" | "--first" | "--size"
            | "--at" | "--expect-config" | "--name" | "--email" | "--line"
    )
}

/// Track value positions and -- even for invalid input, so a message or filename
/// literally named --json never changes the output contract.
pub fn wants_json(args: &[OsString]) -> bool {
    let mut values = args.iter();
    let mut json = false;
    while let Some(value) = values.next() {
        let s = value.to_str().unwrap_or("");
        if s == "--" {
            break;
        }
        if s == "--json" {
            json = true;
        }
        if value_option(s)
            || matches!(
                s,
                "--repo" | "--git" | "--timeout-secs" | "--cancel-after-ms" | "--log-dir"
            )
        {
            values.next();
        }
    }
    json
}

pub fn parse(arguments: Vec<OsString>) -> Parsed<Options> {
    let mut options = Options {
        repo: ".".into(),
        json: false,
        git: None,
        timeout_secs: 600,
        cancel_after_ms: None,
        log_dir: None,
        no_log: false,
        command: Command::Help,
    };
    let mut seen = BTreeSet::new();
    let mut rest = Vec::new();
    let mut args = arguments.into_iter();
    let mut help = false;
    let mut version = false;
    while let Some(arg) = args.next() {
        let key = arg.to_str().unwrap_or("");
        if key == "--" {
            rest.push(arg);
            rest.extend(args);
            break;
        }
        if matches!(
            key,
            "--repo"
                | "--json"
                | "--git"
                | "--timeout-secs"
                | "--cancel-after-ms"
                | "--log-dir"
                | "--no-log"
                | "--help"
                | "-h"
                | "--version"
                | "-V"
        ) {
            let canonical = match key {
                "-h" => "--help",
                "-V" => "--version",
                _ => key,
            };
            if !seen.insert(canonical.to_owned()) {
                return Err("duplicate global option");
            }
            match key {
                "--repo" => options.repo = args.next().ok_or("--repo needs a path")?.into(),
                "--git" => options.git = Some(args.next().ok_or("--git needs a path")?.into()),
                "--log-dir" => {
                    options.log_dir = Some(args.next().ok_or("--log-dir needs a path")?.into())
                }
                "--timeout-secs" => {
                    options.timeout_secs = number(
                        &args.next().ok_or("--timeout-secs needs an integer")?,
                        1,
                        86400,
                    )?
                }
                "--cancel-after-ms" => {
                    options.cancel_after_ms = Some(number(
                        &args.next().ok_or("--cancel-after-ms needs an integer")?,
                        0,
                        86400000,
                    )?)
                }
                "--json" => options.json = true,
                "--no-log" => options.no_log = true,
                "--help" | "-h" => help = true,
                "--version" | "-V" => version = true,
                _ => unreachable!(),
            }
        } else {
            let takes_value = value_option(key);
            rest.push(arg);
            if takes_value {
                rest.push(args.next().ok_or("option requires a value")?);
            }
        }
    }
    if options.no_log && options.log_dir.is_some() {
        return Err("--no-log conflicts with --log-dir");
    }
    if options.log_dir.as_ref().is_some_and(|p| !p.is_absolute()) {
        return Err("--log-dir must be absolute");
    }
    if help && version {
        return Err("choose --help or --version");
    }
    options.command = if version {
        if !rest.is_empty() {
            return Err("--version does not accept a command");
        }
        Command::Version
    } else if help || rest.is_empty() {
        Command::Help
    } else {
        command(rest)?
    };
    Ok(options)
}

fn text(value: &OsString) -> Parsed<String> {
    value
        .to_str()
        .map(str::to_owned)
        .ok_or("text argument must be UTF-8")
}
fn number(value: &OsString, min: u64, max: u64) -> Parsed<u64> {
    let value = text(value)?
        .parse::<u64>()
        .map_err(|_| "expected a nonnegative decimal integer")?;
    if value < min || value > max {
        Err("numeric option is outside its documented range")
    } else {
        Ok(value)
    }
}

struct Tail {
    positional: Vec<OsString>,
    /// Number of positional values before `--`, when it was given.
    separator: Option<usize>,
    values: std::collections::BTreeMap<String, OsString>,
    flags: BTreeSet<String>,
}
impl Tail {
    fn parse(args: Vec<OsString>, flags: &[&str], values: &[&str]) -> Parsed<Self> {
        let mut tail = Self {
            positional: vec![],
            separator: None,
            values: Default::default(),
            flags: Default::default(),
        };
        let mut iter = args.into_iter();
        while let Some(arg) = iter.next() {
            let s = arg.to_str().unwrap_or("");
            if s == "--" {
                tail.separator = Some(tail.positional.len());
                tail.positional.extend(iter);
                break;
            }
            let key = if s == "-m" {
                "--message"
            } else if s == "-u" {
                "--include-untracked"
            } else {
                s
            };
            if flags.contains(&key) {
                if !tail.flags.insert(key.to_owned()) {
                    return Err("duplicate command option");
                }
            } else if values.contains(&key) {
                if tail
                    .values
                    .insert(
                        key.to_owned(),
                        iter.next().ok_or("option requires a value")?,
                    )
                    .is_some()
                {
                    return Err("duplicate command option");
                }
            } else if s.starts_with('-') {
                return Err(
                    "unknown option; use -- before literal paths or names beginning with '-'",
                );
            } else {
                tail.positional.push(arg);
            }
        }
        Ok(tail)
    }
    fn count(&self, n: usize) -> Parsed<()> {
        if self.positional.len() == n {
            Ok(())
        } else {
            Err("wrong number of arguments; see --help")
        }
    }
    fn text(&self, key: &str) -> Parsed<Option<String>> {
        self.values.get(key).map(text).transpose()
    }
    fn required(&self, key: &str) -> Parsed<String> {
        self.text(key)?.ok_or("missing required option; see --help")
    }
    fn flag(&self, key: &str) -> bool {
        self.flags.contains(key)
    }
}

fn command(mut args: Vec<OsString>) -> Parsed<Command> {
    let verb = text(&args.remove(0))?;
    let sub = if matches!(
        verb.as_str(),
        "branch" | "stash" | "merge" | "remote" | "rebase" | "review" | "tag" | "worktree"
            | "conflict" | "bisect" | "verify" | "signing" | "identity" | "lfs" | "ssh"
    ) {
        if args.is_empty() {
            return Err("missing subcommand; see --help");
        }
        text(&args.remove(0))?
    } else {
        String::new()
    };
    let (flags, values): (&[&str], &[&str]) = match (verb.as_str(), sub.as_str()) {
        ("history", _) => (&["--all"], &["--revision", "--limit", "--order"]),
        ("commit-url", _) => (&[], &["--remote", "--provider", "--server"]),
        ("review", "file") => (&["--blame"], &["--path"]),
        ("diff", _) => (&["--staged"], &["--base", "--target", "--context"]),
        ("hunks", _) => (&["--staged"], &[]),
        ("blame", _) => (&[], &["--path", "--first", "--size"]),
        ("file-history", _) => (&[], &["--revision", "--limit"]),
        ("search", _) => (&["--all", "--regex", "--match-case"], &["--kind", "--revision", "--path"]),
        ("grep", _) => (&["--regex", "--match-case"], &["--revision", "--path"]),
        ("compare", _) => (&["--staged", "--worktree"], &[]),
        ("reflog", _) => (&[], &["--scope", "--limit"]),
        ("verify", _) => (&["--require-valid"], &[]),
        ("stage" | "unstage", _) => (&["--all"], &["--lines", "--expect-patch"]),
        ("commit", _) | ("merge", "continue") => (&[], &["--message"]),
        ("undo-commit", _) => (&["--preview"], &["--branch", "--expect", "--index-sha256"]),
        ("amend", _) => (&["--preview"], &["--message", "--expect"]),
        ("cherry-pick" | "revert", _) => (&["--preview"], &["--mainline", "--expect"]),
        ("reset", _) => (&["--soft", "--mixed", "--hard", "--preview"], &["--expect"]),
        ("rebase", "start") => (&["--interactive", "--update-refs", "--preserve-merges", "--preview"], &["--plan", "--expect-original", "--expect-onto"]),
        ("init", _) => (&[], &["--branch"]),
        ("branch", "create") => (&[], &["--start"]),
        ("stash", "save") => (&["--include-untracked", "--preview"], &["--message", "--lines", "--expect-patch"]),
        ("stash", "show") => (&[], &["--part"]),
        ("stash", "apply") => (&["--index", "--preview"], &["--part", "--hunks", "--lines", "--expect-patch"]),
        ("stash", "drop") => (&[], &["--expect"]),
        ("stash", "pop") => (&["--preview"], &["--expect"]),
        ("conflict", "resolve") => (&["--stage", "--preview"], &["--expect-sha256", "--choose", "--result"]),
        ("conflict", "keep") => (&["--preview"], &["--side", "--expect"]),
        ("bisect", "start" | "resume" | "restore") => (&["--preview"], &[]),
        ("bisect", "good" | "bad" | "skip") => (&["--preview"], &["--expect"]),
        ("bisect", "run") => (&["--continue", "--preview"], &["--test-timeout", "--expect-sha256"]),
        ("tag", "publish") => (&["--preview"], &["--remote", "--source"]),
        ("tag", "delete-remote") => (&["--preview"], &["--remote", "--expect"]),
        ("merge", "start") => (&["--ff-only", "--no-ff", "--squash", "--no-commit"], &[]),
        ("remote", "set-url") => (&["--push"], &[]),
        ("remote", "rename" | "prune") | ("branch", "fast-forward") => (&["--preview"], &[]),
        ("branch", "delete-remote") => (&["--preview"], &["--expect"]),
        ("fetch", _) => (&["--tags"], &[]),
        ("identity", "set") => (&["--preview"], &["--name", "--email"]),
        ("worktree", "move" | "prune") => (&["--preview"], &["--expect"]),
        ("worktree", "repair") => (&["--preview"], &["--at", "--expect"]),
        ("lfs", "track") => (&["--literal", "--preview"], &[]),
        ("lfs", "untrack") => (&["--preview"], &["--line", "--expect"]),
        ("lfs", "fetch" | "push" | "checkout") => (&["--preview"], &[]),
        ("lfs", "lock") => (&["--preview"], &["--remote"]),
        ("lfs", "unlock") => (&["--preview"], &["--remote", "--expect"]),
        ("ssh", "use" | "restore") => (&["--preview"], &["--expect-config"]),
        ("push", _) => (&["--set-upstream"], &[]),
        ("push-lease", _) => (&["--preview"], &["--source", "--expect"]),
        ("push-atomic", _) => (&["--preview"], &[]),
        ("tag", "list") => (&[], &["--limit", "--order"]),
        ("tag", "create") => (&["--sign", "--preview"], &["--message"]),
        ("tag", "delete") | ("worktree", "remove") => (&["--preview"], &["--expect"]),
        ("worktree", "create") => (&["--existing", "--preview"], &["--branch", "--start"]),
        ("pull", _) => (&["--ff-only", "--merge", "--rebase", "--preview"], &[]),
        ("rebase", "continue" | "skip" | "abort") => (&["--preview"], &[]),
        ("clone", _) => (&[], &["--local", "--url"]),
        _ => (&[], &[]),
    };
    let t = Tail::parse(args, flags, values)?;
    let one = || -> Parsed<OsString> {
        t.count(1)?;
        Ok(t.positional[0].clone())
    };
    let result = match (verb.as_str(), sub.as_str()) {
        ("info", _) => {
            t.count(0)?;
            Command::Info
        }
        ("status", _) => {
            t.count(0)?;
            Command::Status
        }
        ("operation", _) => {
            t.count(0)?;
            Command::Operation
        }
        ("history", _) => {
            t.count(0)?;
            let revision = t.text("--revision")?;
            if t.flag("--all") && revision.is_some() {
                return Err("--all conflicts with --revision");
            }
            Command::History {
                order: rocket_git::HistoryOrder::parse(t.text("--order")?.as_deref().unwrap_or("topo"))
                    .ok_or("--order must be topo, commit-date or author-date")?,
                all: t.flag("--all"),
                revision,
                limit: t
                    .values
                    .get("--limit")
                    .map(|v| number(v, 1, 1000))
                    .transpose()?
                    .unwrap_or(25) as usize,
            }
        }
        ("commit-url", _) => {
            let revision = text(&one()?)?;
            let provider = t.text("--provider")?;
            let server = t.text("--server")?;
            let config = match (provider, server) {
                (None, None) => None,
                (Some(provider), Some(server)) => Some(rocket_git::ForgeConfig::new(
                    rocket_git::ForgeProvider::from_key(&provider).ok_or("unknown hosting provider")?, &server)
                    .map_err(|_| "invalid HTTPS hosting server")?),
                _ => return Err("use --provider and --server together for a custom host"),
            };
            Command::CommitUrl { revision, remote: t.text("--remote")?, config }
        }
        ("review", "commit" | "file") => {
            let path = if sub == "file" { Some(t.values.get("--path").cloned().ok_or("file review requires --path")?) } else { None };
            Command::Review { revision: text(&one()?)?, path, blame: t.flag("--blame") }
        }
        ("diff", _) => {
            let base = t.text("--base")?;
            let target = t.text("--target")?;
            if base.is_some() != target.is_some() || (base.is_some() && t.flag("--staged")) {
                return Err("use --staged, or both --base and --target");
            }
            Command::Diff {
                staged: t.flag("--staged"),
                base,
                target,
                context: t
                    .values
                    .get("--context")
                    .map(|v| number(v, 0, 1000))
                    .transpose()?
                    .unwrap_or(3) as u16,
                paths: t.positional,
            }
        }
        ("hunks", _) => Command::Hunks { path: one()?, staged: t.flag("--staged") },
        ("blame", _) => Command::Blame {
            revision: text(&one()?)?,
            path: t.values.get("--path").cloned().ok_or("blame needs --path PATH")?,
            first: t.values.get("--first").map(|v| number(v, 1, 100_000_000)).transpose()?.unwrap_or(1) as usize,
            size: t.values.get("--size").map(|v| number(v, 1, 500)).transpose()?.unwrap_or(500) as usize,
        },
        ("file-history", _) => Command::FileHistory {
            path: one()?,
            revision: t.text("--revision")?,
            limit: t.values.get("--limit").map(|v| number(v, 1, 200)).transpose()?.unwrap_or(50) as usize,
        },
        ("search", _) => {
            if t.flag("--all") && t.values.contains_key("--revision") {
                return Err("--all conflicts with --revision");
            }
            let kind = match t.text("--kind")?.as_deref() {
                None | Some("text") => rocket_git::HistorySearchKind::Text,
                Some("path") => rocket_git::HistorySearchKind::Path,
                Some("content") => rocket_git::HistorySearchKind::Content,
                _ => return Err("--kind must be text, path or content"),
            };
            let (regex, match_case, path) = (t.flag("--regex"), t.flag("--match-case"), t.values.get("--path").cloned());
            if kind != rocket_git::HistorySearchKind::Content && (regex || match_case || path.is_some()) {
                return Err("--regex, --match-case and --path need --kind content");
            }
            Command::Search {
                query: text(&one()?)?,
                kind,
                all: t.flag("--all"),
                revision: t.text("--revision")?,
                regex,
                match_case,
                path,
            }
        }
        ("grep", _) => Command::Grep {
            pattern: text(&one()?)?,
            revision: t.text("--revision")?,
            regex: t.flag("--regex"),
            match_case: t.flag("--match-case"),
            path: t.values.get("--path").cloned(),
        },
        ("compare", _) => {
            let local = usize::from(t.flag("--staged")) + usize::from(t.flag("--worktree"));
            match (t.positional.len(), local) {
                (2, 0) => Command::Compare { base: text(&t.positional[0])?, target: Some(text(&t.positional[1])?), staged: false },
                (1, 1) => Command::Compare { base: text(&t.positional[0])?, target: None, staged: t.flag("--staged") },
                _ => return Err("compare BASE TARGET, or compare BASE with --staged or --worktree"),
            }
        }
        ("reflog", _) => {
            t.count(0)?;
            Command::Reflog {
                scope: match t.text("--scope")?.as_deref() {
                    None | Some("head") => rocket_git::ReflogScope::Head,
                    Some("all") => rocket_git::ReflogScope::AllRefs,
                    Some("deleted-branches") => rocket_git::ReflogScope::DeletedBranches,
                    Some("retained") => rocket_git::ReflogScope::Retained,
                    Some("unreachable") => rocket_git::ReflogScope::Unreachable,
                    _ => return Err("--scope must be head, all, deleted-branches, retained or unreachable"),
                },
                limit: t.values.get("--limit").map(|v| number(v, 1, 2000)).transpose()?.unwrap_or(100) as usize,
            }
        }
        ("verify", "commit" | "tag") => Command::Verify { tag: sub == "tag", name: text(&one()?)?, require_valid: t.flag("--require-valid") },
        ("signing", "show") => {
            t.count(0)?;
            Command::SigningShow
        }
        ("init", _) => Command::Init {
            destination: one()?.into(),
            branch: t.text("--branch")?.unwrap_or_else(|| "main".into()),
        },
        ("stage" | "unstage", _) if t.values.contains_key("--lines") => {
            if t.flag("--all") || t.positional.len() != 1 {
                return Err("--lines selects changed lines of exactly one path");
            }
            Command::StageLines {
                path: t.positional[0].clone(),
                lines: ids(&t.values["--lines"])?,
                expected: checked_sha256(&t.required("--expect-patch")?)?,
                unstage: verb == "unstage",
            }
        }
        ("stage" | "unstage", _) => {
            let all = t.flag("--all");
            if t.values.contains_key("--expect-patch") {
                return Err("--expect-patch pins a --lines selection");
            }
            if (all && !t.positional.is_empty()) || (!all && t.positional.is_empty()) {
                return Err("select paths or explicitly use --all");
            }
            Command::Stage {
                all,
                paths: t.positional,
                unstage: verb == "unstage",
            }
        }
        ("commit", _) => {
            t.count(0)?;
            Command::Commit(t.required("--message")?)
        }
        ("undo-commit", _) => {
            t.count(0)?;
            let expected = t.required("--expect")?;
            checked_id(&expected)?;
            Command::UndoCommit {
                branch: t.required("--branch")?,
                expected: expected.to_ascii_lowercase(),
                index: checked_sha256(&t.required("--index-sha256")?)?,
                preview: t.flag("--preview"),
            }
        }
        ("amend", _) => {
            t.count(0)?;
            let expected = t.required("--expect")?;
            checked_id(&expected)?;
            Command::CommitAction {
                request: rocket_git::CommitActionRequest {
                    operation: rocket_git::CommitOperation::Amend, targets: vec![], mainline: None, recovery: None,
                },
                expected: Some(expected.to_ascii_lowercase()),
                message: Some(t.required("--message")?),
                preview: t.flag("--preview"),
            }
        }
        ("cherry-pick" | "revert", _) => {
            let operation = if verb == "revert" { rocket_git::CommitOperation::Revert } else { rocket_git::CommitOperation::CherryPick };
            let recovery = match t.positional.first().and_then(|s| s.to_str()) {
                Some("continue") => Some(rocket_git::CommitRecovery::Continue),
                Some("skip") => Some(rocket_git::CommitRecovery::Skip),
                Some("abort") => Some(rocket_git::CommitRecovery::Abort),
                _ => None,
            };
            let (targets, mainline, expected) = if recovery.is_some() {
                if t.positional.len() != 1 || !t.values.is_empty() {
                    return Err("continue, skip and abort take no commits, --mainline or --expect");
                }
                (vec![], None, None)
            } else {
                if t.positional.is_empty() {
                    return Err("name the commits to apply by full object ID, in order");
                }
                let targets = t.positional.iter().map(|id| {
                    let id = text(id)?.to_ascii_lowercase();
                    rocket_git::ObjectId::parse(&id).map_err(|_| "commits are full 40- or 64-character object IDs")
                }).collect::<Parsed<Vec<_>>>()?;
                let expected = t.text("--expect")?;
                if let Some(id) = &expected {
                    checked_id(id)?;
                }
                let mainline = t.values.get("--mainline").map(|v| number(v, 1, 64)).transpose()?.map(|n| n as usize);
                (targets, mainline, expected.map(|id| id.to_ascii_lowercase()))
            };
            Command::CommitAction {
                request: rocket_git::CommitActionRequest { operation, targets, mainline, recovery },
                expected,
                message: None,
                preview: t.flag("--preview"),
            }
        }
        ("reset", _) => {
            t.count(1)?;
            let modes = [("--soft", rocket_git::ResetMode::Soft), ("--mixed", rocket_git::ResetMode::Mixed), ("--hard", rocket_git::ResetMode::Hard)];
            let mut chosen = modes.iter().filter(|(flag, _)| t.flag(flag)).map(|(_, mode)| *mode);
            let mode = match (chosen.next(), chosen.next()) {
                (Some(mode), None) => mode,
                _ => return Err("choose one reset mode: --soft, --mixed or --hard"),
            };
            let target = text(&t.positional[0])?.to_ascii_lowercase();
            let expected = t.required("--expect")?;
            for id in [&target, &expected] {
                checked_id(id)?;
            }
            Command::Reset { mode, target, expected: expected.to_ascii_lowercase(), preview: t.flag("--preview") }
        }
        ("branch", "list") => {
            t.count(0)?;
            Command::BranchList
        }
        ("branch", "create") => Command::BranchCreate {
            name: one()?,
            start: t.text("--start")?,
        },
        ("branch", "switch") => Command::BranchSwitch(one()?),
        ("branch", "delete") => Command::BranchDelete(one()?),
        ("branch", "rename") => {
            t.count(2)?;
            Command::BranchRename(t.positional[0].clone(), t.positional[1].clone())
        }
        ("stash", "list") => {
            t.count(0)?;
            Command::StashList
        }
        ("stash", "save") if t.values.contains_key("--lines") => {
            if t.flag("--include-untracked") || t.positional.len() != 1 {
                return Err("stash save --lines selects changed lines of exactly one tracked path");
            }
            Command::StashSaveLines {
                message: t.text("--message")?.ok_or("stash save --lines needs --message")?,
                path: t.positional[0].clone(),
                lines: ids(&t.values["--lines"])?,
                expected: checked_sha256(&t.required("--expect-patch")?)?,
                preview: t.flag("--preview"),
            }
        }
        ("stash", "save") => {
            if t.values.contains_key("--expect-patch") || t.flag("--preview") {
                return Err("--expect-patch and --preview apply to stash save --lines");
            }
            Command::StashSave {
                message: t.text("--message")?,
                untracked: t.flag("--include-untracked"),
                paths: t.positional,
            }
        }
        ("stash", "show") => Command::StashShow { id: text(&one()?)?, part: stash_part(t.text("--part")?)? },
        ("stash", "apply") => {
            let (id, paths) = t.positional.split_first().ok_or("wrong number of arguments; see --help")?;
            let id = text(id)?;
            if paths.is_empty() {
                if t.flag("--preview") || ["--part", "--hunks", "--lines", "--expect-patch"].iter().any(|k| t.values.contains_key(*k)) {
                    return Err("select paths after -- to apply part of a stash");
                }
                Command::StashApply { id, index: t.flag("--index") }
            } else {
                if t.flag("--index") {
                    return Err("--index restores a whole stash; omit the paths");
                }
                let hunks = t.values.get("--hunks").map(ids).transpose()?.unwrap_or_default();
                let lines = t.values.get("--lines").map(ids).transpose()?.unwrap_or_default();
                if !hunks.is_empty() && !lines.is_empty() {
                    return Err("choose --hunks or --lines, not both");
                }
                let selection = !hunks.is_empty() || !lines.is_empty();
                let expected = t.text("--expect-patch")?.map(|s| checked_sha256(&s)).transpose()?;
                if selection != expected.is_some() || (selection && paths.len() != 1) {
                    return Err("--hunks or --lines select changes of exactly one path and need --expect-patch");
                }
                Command::StashApplySelection {
                    id, part: stash_part(t.text("--part")?)?, paths: paths.to_vec(),
                    hunks, lines, expected, preview: t.flag("--preview"),
                }
            }
        }
        ("stash", "drop") => Command::StashDrop {
            index: number(&one()?, 0, 1000000)? as usize,
            expected: t.required("--expect")?,
        },
        ("stash", "pop") => {
            let expected = t.required("--expect")?;
            checked_id(&expected)?;
            Command::StashPop {
                index: number(&one()?, 0, 1000000)? as usize,
                expected: expected.to_ascii_lowercase(),
                preview: t.flag("--preview"),
            }
        }
        ("merge", "start") => {
            if ["--ff-only", "--no-ff", "--squash", "--no-commit"].iter().filter(|flag|t.flag(flag)).count()>1 {
                return Err("choose one merge mode: --ff-only, --no-ff, --squash or --no-commit");
            }
            Command::MergeStart {
                revision: text(&one()?)?,
                mode: if t.flag("--ff-only") {
                    rocket_git::MergeMode::FastForwardOnly
                } else if t.flag("--no-ff") {
                    rocket_git::MergeMode::NoFastForward
                } else if t.flag("--squash") {
                    rocket_git::MergeMode::Squash
                } else if t.flag("--no-commit") {
                    rocket_git::MergeMode::NoCommit
                } else {
                    rocket_git::MergeMode::AllowFastForward
                },
            }
        }
        ("merge", "continue") => {
            t.count(0)?;
            Command::MergeContinue(t.required("--message")?)
        }
        ("merge", "abort") => {
            t.count(0)?;
            Command::MergeAbort
        }
        ("rebase", "continue" | "skip" | "abort") => {
            t.count(0)?;
            Command::RebaseRecovery {
                action: match sub.as_str() {
                    "continue" => rocket_git::RebaseAction::Continue,
                    "skip" => rocket_git::RebaseAction::Skip,
                    _ => rocket_git::RebaseAction::Abort,
                },
                preview: t.flag("--preview"),
            }
        }
        ("rebase", "start") => {
            t.count(1)?;
            let plan = t.values.get("--plan").map(PathBuf::from);
            let interactive = t.flag("--interactive") || plan.is_some();
            if t.flag("--preserve-merges") && (interactive || t.flag("--update-refs")) {
                return Err("--preserve-merges cannot be combined with --interactive, --plan or --update-refs");
            }
            let preview = t.flag("--preview");
            let expected = match (preview, t.text("--expect-original")?, t.text("--expect-onto")?) {
                (true, None, None) => None,
                (false, Some(original), Some(onto)) => {
                    checked_id(&original)?;
                    checked_id(&onto)?;
                    Some((original.to_ascii_lowercase(), onto.to_ascii_lowercase()))
                }
                _ => return Err("review with --preview, then start with --expect-original and --expect-onto from the review"),
            };
            Command::RebaseStart {
                target: text(&t.positional[0])?,
                interactive,
                plan,
                update_refs: t.flag("--update-refs"),
                preserve_merges: t.flag("--preserve-merges"),
                expected,
                preview,
            }
        }
        ("conflict", "show") => Command::ConflictShow(one()?),
        ("conflict", "resolve") => {
            let choice = match (t.text("--choose")?, t.values.get("--result")) {
                (Some(spec), None) => choices(&spec)?,
                (None, Some(file)) => crate::conflicts::Choice::Result(file.into()),
                _ => return Err("choose conflicts with --choose, or give the complete result with --result"),
            };
            Command::ConflictResolve {
                path: one()?,
                expected: checked_sha256(&t.required("--expect-sha256")?)?,
                choice,
                stage: t.flag("--stage"),
                preview: t.flag("--preview"),
            }
        }
        ("conflict", "keep") => {
            let keep = match t.required("--side")?.as_str() {
                "ours" => rocket_git::ConflictKeep::Ours,
                "theirs" => rocket_git::ConflictKeep::Theirs,
                "deletion" => rocket_git::ConflictKeep::Deletion,
                _ => return Err("--side must be ours, theirs or deletion"),
            };
            let expected = t.required("--expect")?;
            let expected = match (keep, expected.as_str()) {
                (rocket_git::ConflictKeep::Deletion, "missing") => None,
                (rocket_git::ConflictKeep::Deletion, _) | (_, "missing") => {
                    return Err("--expect is the kept side's blob ID, or missing when keeping the deletion");
                }
                (_, id) => {
                    checked_id(id)?;
                    Some(id.to_ascii_lowercase())
                }
            };
            Command::ConflictKeep { path: one()?, keep, expected, preview: t.flag("--preview") }
        }
        ("conflict", "mark-resolved") => Command::ConflictMarkResolved(one()?),
        ("bisect", "status") => {
            t.count(0)?;
            Command::BisectStatus
        }
        ("bisect", "start") => {
            t.count(2)?;
            Command::Bisect {
                action: rocket_git::BisectAction::Start,
                good: text(&t.positional[0])?,
                bad: text(&t.positional[1])?,
                expected: None,
                preview: t.flag("--preview"),
            }
        }
        ("bisect", "good" | "bad" | "skip" | "resume" | "restore") => {
            t.count(0)?;
            let (action, expected) = match sub.as_str() {
                "resume" => (rocket_git::BisectAction::Resume, None),
                "restore" => (rocket_git::BisectAction::Restore, None),
                decision => {
                    let expected = t.required("--expect")?;
                    checked_id(&expected)?;
                    let action = match decision {
                        "good" => rocket_git::BisectAction::Good,
                        "bad" => rocket_git::BisectAction::Bad,
                        _ => rocket_git::BisectAction::Skip,
                    };
                    (action, Some(expected.to_ascii_lowercase()))
                }
            };
            Command::Bisect { action, good: String::new(), bad: String::new(), expected, preview: t.flag("--preview") }
        }
        ("bisect", "run") => {
            let (before, after) = t.positional.split_at(t.separator.ok_or("bisect run needs -- before PROGRAM")?);
            let range = match (before.len(), t.flag("--continue")) {
                (2, false) => Some((text(&before[0])?, text(&before[1])?)),
                (0, true) => None,
                _ => return Err("bisect run needs GOOD BAD, or --continue for the active search"),
            };
            let (program, arguments) = after.split_first().ok_or("bisect run needs PROGRAM after --")?;
            let program = PathBuf::from(program);
            if !program.is_absolute() {
                return Err("bisect run needs an absolute PROGRAM path");
            }
            let preview = t.flag("--preview");
            let expected = t.text("--expect-sha256")?.map(|s| checked_sha256(&s)).transpose()?;
            if preview == expected.is_some() {
                return Err("review with --preview first, then run with --expect-sha256 and the reviewed digest");
            }
            Command::BisectRun {
                range,
                program,
                arguments: arguments.iter().map(text).collect::<Parsed<_>>()?,
                timeout: number(t.values.get("--test-timeout").ok_or("bisect run needs --test-timeout SECS")?, 1, 3600)?,
                expected,
                preview,
            }
        }
        ("remote", "list") => {
            t.count(0)?;
            Command::RemoteList
        }
        ("remote", "add") => {
            t.count(2)?;
            Command::RemoteAdd(t.positional[0].clone(), t.positional[1].clone())
        }
        ("remote", "set-url") => {
            t.count(2)?;
            Command::RemoteSetUrl {
                name: t.positional[0].clone(),
                url: t.positional[1].clone(),
                push: t.flag("--push"),
            }
        }
        ("remote", "remove") => Command::RemoteRemove(one()?),
        ("fetch", _) => Command::Fetch { remote: one()?, tags: t.flag("--tags") },
        ("remote", "rename") => {
            t.count(2)?;
            Command::RemoteRename { old: text(&t.positional[0])?, new: text(&t.positional[1])?, preview: t.flag("--preview") }
        }
        ("remote", "prune") => {
            let (remote, refs) = t.positional.split_first().ok_or("remote prune needs REMOTE")?;
            let preview = t.flag("--preview");
            if preview != refs.is_empty() {
                return Err("review with --preview, then name the tracking refs to remove after --");
            }
            Command::RemotePrune { remote: text(remote)?, refs: refs.iter().map(text).collect::<Parsed<_>>()?, preview }
        }
        ("branch", "delete-remote") => {
            t.count(2)?;
            let expected = t.required("--expect")?;
            checked_id(&expected)?;
            Command::BranchDeleteRemote {
                remote: text(&t.positional[0])?,
                branch: text(&t.positional[1])?,
                expected: expected.to_ascii_lowercase(),
                preview: t.flag("--preview"),
            }
        }
        ("branch", "fast-forward") => {
            let preview = t.flag("--preview");
            if preview != t.positional.is_empty() {
                return Err("review with --preview, then name the branches to fast-forward after --");
            }
            Command::BranchFastForward { branches: t.positional.iter().map(text).collect::<Parsed<_>>()?, preview }
        }
        ("identity", "show") => {
            t.count(0)?;
            Command::IdentityShow
        }
        ("identity", "set") => {
            t.count(0)?;
            let (name, email) = (t.text("--name")?, t.text("--email")?);
            if name.is_none() && email.is_none() {
                return Err("identity set needs --name, --email or both");
            }
            Command::IdentitySet { name, email, preview: t.flag("--preview") }
        }
        ("lfs", "status") => {
            t.count(0)?;
            Command::LfsStatus
        }
        ("lfs", "locks") => Command::LfsLocks(text(&one()?)?),
        ("lfs", "track") => Command::LfsChange {
            action: rocket_git::LfsAction::Track { pattern: text(&one()?)?, literal: t.flag("--literal") },
            preview: t.flag("--preview"),
        },
        ("lfs", "untrack") => {
            t.count(0)?;
            Command::LfsChange {
                action: rocket_git::LfsAction::Untrack {
                    line: number(t.values.get("--line").ok_or("lfs untrack needs --line N from lfs status")?, 0, 1_000_000)? as usize,
                    expected: t.required("--expect")?,
                },
                preview: t.flag("--preview"),
            }
        }
        ("lfs", "fetch" | "push") => {
            let remote = text(&one()?)?;
            let action = if sub == "fetch" { rocket_git::LfsAction::Fetch(remote) } else { rocket_git::LfsAction::Push(remote) };
            Command::LfsChange { action, preview: t.flag("--preview") }
        }
        ("lfs", "checkout") => Command::LfsCheckout { path: one()?, preview: t.flag("--preview") },
        ("lfs", "lock" | "unlock") => Command::LfsLock {
            path: text(&one()?)?.replace('\\', "/"),
            remote: t.required("--remote")?,
            unlock: if sub == "unlock" { Some(t.required("--expect")?) } else { None },
            preview: t.flag("--preview"),
        },
        ("ssh", "keys") => {
            t.count(0)?;
            Command::SshKeys
        }
        ("ssh", "check") => Command::SshCheck(text(&one()?)?),
        ("ssh", "use" | "restore") => {
            let alias = if sub == "use" { Some(text(&one()?)?) } else { t.count(0)?; None };
            let preview = t.flag("--preview");
            let expected = t.text("--expect-config")?.map(|s| checked_sha256(&s)).transpose()?;
            if preview == expected.is_some() {
                return Err("review with --preview, then run with --expect-config and the reviewed SHA-256");
            }
            Command::SshBinding { alias, expected, preview }
        }
        ("pull", _) => {
            t.count(0)?;
            if ["--ff-only", "--merge", "--rebase"]
                .iter()
                .filter(|f| t.flag(f))
                .count()
                > 1
            {
                return Err("choose one pull strategy: --ff-only, --merge or --rebase");
            }
            Command::Pull {
                strategy: if t.flag("--merge") {
                    rocket_git::PullStrategy::Merge
                } else if t.flag("--rebase") {
                    rocket_git::PullStrategy::Rebase
                } else {
                    rocket_git::PullStrategy::FastForward
                },
                preview: t.flag("--preview"),
            }
        }
        ("push", _) => {
            t.count(3)?;
            Command::Push {
                remote: t.positional[0].clone(),
                local: t.positional[1].clone(),
                destination: t.positional[2].clone(),
                upstream: t.flag("--set-upstream"),
            }
        }
        ("clone", _) => {
            let source = match (t.values.get("--local"), t.values.get("--url")) {
                (Some(local), None) => rocket_git::CloneSource::Local(local.into()),
                (None, Some(url)) => rocket_git::CloneSource::Url(text(url)?),
                _ => return Err("clone needs exactly one of --local PATH or --url URL"),
            };
            Command::Clone {
                source,
                destination: one()?.into(),
            }
        }
        ("push-lease", _) => {
            t.count(3)?;
            let source=t.text("--source")?;
            let expected=t.text("--expect")?;
            let preview=t.flag("--preview");
            if preview && (source.is_some() || expected.is_some()) { return Err("--preview cannot include confirmation IDs"); }
            if !preview {
                for id in [&source,&expected] { checked_id(id.as_deref().ok_or("push-lease needs --source and --expect from the preview")?)?; }
            }
            Command::PushLease { remote:text(&t.positional[0])?,local:text(&t.positional[1])?,destination:text(&t.positional[2])?,source:source.map(|s|s.to_ascii_lowercase()),expected:expected.map(|s|s.to_ascii_lowercase()),preview }
        }
        ("push-atomic", _) => {
            if !(2..=33).contains(&t.positional.len()) { return Err("push-atomic needs a remote and 1 to 32 branch mappings"); }
            let preview=t.flag("--preview");
            let mut destinations=BTreeSet::new();
            let mut branches=Vec::new();
            for value in &t.positional[1..] {
                let value=text(value)?;
                let fields:Vec<_>=value.split(':').collect();
                if fields.len()!=if preview {2} else {4} || fields.iter().any(|s|s.is_empty()) { return Err("atomic preview needs LOCAL:REMOTE; apply needs LOCAL:REMOTE:SOURCE_ID:EXPECTED_ID_OR_missing"); }
                if !destinations.insert(fields[1].to_owned()) { return Err("atomic destinations must be distinct"); }
                let (source,expected)=if preview {(None,None)} else {
                    checked_id(fields[2])?;
                    if fields[3]!="missing" {checked_id(fields[3])?;}
                    (Some(fields[2].to_ascii_lowercase()),if fields[3]=="missing" {None} else {Some(fields[3].to_ascii_lowercase())})
                };
                branches.push(crate::advanced::AtomicSelection { local:fields[0].into(),destination:fields[1].into(),source,expected });
            }
            Command::PushAtomic {remote:text(&t.positional[0])?,branches,preview}
        }
        ("tag", "list") => {t.count(0)?; Command::TagList{limit:t.values.get("--limit").map(|s|number(s,1,2000)).transpose()?.unwrap_or(200) as usize,
            order:match t.text("--order")?.as_deref() {None|Some("name")=>rocket_git::TagOrder::Name,Some("newest")=>rocket_git::TagOrder::Newest,_=>return Err("--order must be name or newest")}}}
        ("tag", "inspect") => Command::TagInspect(text(&one()?)?),
        ("tag", "create") => {t.count(2)?; let message=t.text("--message")?;
            if t.flag("--sign") && message.is_none() {return Err("--sign needs --message: a signed tag is annotated");}
            Command::TagCreate{name:text(&t.positional[0])?,revision:text(&t.positional[1])?,message,sign:t.flag("--sign"),preview:t.flag("--preview")}}
        ("tag", "delete") => {let expected=t.required("--expect")?;checked_id(&expected)?; Command::TagDelete{name:text(&one()?)?,expected,preview:t.flag("--preview")}}
        ("tag", "remote") => Command::TagRemote(text(&one()?)?),
        ("tag", "publish") => {let source=t.required("--source")?;checked_id(&source)?; Command::TagPublish{name:text(&one()?)?,remote:t.required("--remote")?,source:source.to_ascii_lowercase(),preview:t.flag("--preview")}}
        ("tag", "delete-remote") => {let expected=t.required("--expect")?;checked_id(&expected)?; Command::TagDeleteRemote{name:text(&one()?)?,remote:t.required("--remote")?,expected:expected.to_ascii_lowercase(),preview:t.flag("--preview")}}
        ("worktree", "list") => {t.count(0)?;Command::WorktreeList}
        ("worktree", "create") => {
            if t.flag("--existing") && t.values.contains_key("--start") {return Err("--start cannot override an existing branch");}
            Command::WorktreeCreate {path:one()?.into(),branch:t.required("--branch")?,start:t.text("--start")?.unwrap_or_else(||"HEAD".into()),existing:t.flag("--existing"),preview:t.flag("--preview")}
        }
        ("worktree", "remove") => {let expected=t.required("--expect")?;checked_id(&expected)?;Command::WorktreeRemove{path:one()?.into(),expected,preview:t.flag("--preview")}}
        ("worktree", "move" | "repair" | "prune") => {
            let expected = t.required("--expect")?;
            checked_id(&expected)?;
            let (kind, destination) = match sub.as_str() {
                "move" => {
                    t.count(2)?;
                    (rocket_git::WorktreeMaintenance::Move, Some(PathBuf::from(&t.positional[1])))
                }
                "repair" => {
                    t.count(1)?;
                    (rocket_git::WorktreeMaintenance::Repair, Some(PathBuf::from(t.values.get("--at").ok_or("worktree repair needs --at FOLDER")?)))
                }
                _ => {
                    t.count(1)?;
                    (rocket_git::WorktreeMaintenance::Prune, None)
                }
            };
            Command::WorktreeMaintain {
                kind, path: PathBuf::from(&t.positional[0]), destination,
                expected: expected.to_ascii_lowercase(), preview: t.flag("--preview"),
            }
        }
        _ => return Err("unknown command or subcommand; see --help"),
    };
    Ok(result)
}

fn checked_id(id: &str) -> Parsed<()> {
    rocket_git::ObjectId::parse(id).map(|_| ()).map_err(|_| "expected a full 40- or 64-character object ID")
}
fn checked_sha256(value: &str) -> Parsed<String> {
    if value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit()) {
        Ok(value.to_ascii_lowercase())
    } else {
        Err("expected the 64-character SHA-256 from the review")
    }
}
/// Comma-separated decimal IDs from a review (line IDs or hunk numbers), each once.
fn ids(value: &OsString) -> Parsed<Vec<usize>> {
    let mut seen = BTreeSet::new();
    text(value)?
        .split(',')
        .map(|s| {
            let id = s.parse::<usize>().map_err(|_| "IDs are comma-separated decimal numbers")?;
            if id > 10_000_000 || !seen.insert(id) {
                return Err("IDs must be listed once and come from the review");
            }
            Ok(id)
        })
        .collect()
}
fn stash_part(value: Option<String>) -> Parsed<rocket_git::StashPart> {
    match value.as_deref() {
        None | Some("tracked") => Ok(rocket_git::StashPart::Tracked),
        Some("index") => Ok(rocket_git::StashPart::Index),
        Some("untracked") => Ok(rocket_git::StashPart::Untracked),
        _ => Err("--part must be tracked, index or untracked"),
    }
}
/// `N=SIDE[,N=SIDE...]` with conflicts numbered from 1 as `conflict show` lists them,
/// or `all=SIDE`.
fn choices(spec: &str) -> Parsed<crate::conflicts::Choice> {
    use crate::conflicts::{Choice, Side};
    let side = |s: &str| match s {
        "ours" => Ok(Side::Ours),
        "theirs" => Ok(Side::Theirs),
        "both" => Ok(Side::Both),
        _ => Err("conflict sides are ours, theirs or both"),
    };
    if let Some(s) = spec.strip_prefix("all=") {
        return Ok(Choice::All(side(s)?));
    }
    let mut seen = BTreeSet::new();
    let mut list = Vec::new();
    for item in spec.split(',') {
        let (number, s) = item.split_once('=').ok_or("--choose takes N=SIDE pairs or all=SIDE")?;
        let number = number
            .parse::<usize>()
            .ok()
            .filter(|n| (1..=1_000_000).contains(n))
            .ok_or("conflicts are numbered from 1")?;
        if !seen.insert(number) {
            return Err("a conflict is chosen twice");
        }
        list.push((number, side(s)?));
    }
    Ok(Choice::Blocks(list))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn args(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }
    #[test]
    fn values_and_literal_separator_do_not_become_global_options() {
        for case in [
            &["commit", "-m", "--json"][..],
            &["stage", "--", "--json"][..],
        ] {
            assert!(!wants_json(&args(case)));
            assert!(!parse(args(case)).unwrap().json);
        }
        assert!(
            parse(args(&["--json", "commit", "--message", "--repo"]))
                .unwrap()
                .json
        );
    }
    #[test]
    fn invalid_or_ambiguous_commands_are_rejected_before_git() {
        for case in [
            vec!["stage"],
            vec!["stage", "--all", "file"],
            vec!["commit", "-m", "x", "--force"],
            vec!["branch", "delete", "main", "--force"],
            vec!["push", "origin", "main", "main", "--force"],
            vec!["diff", "--base", "a"],
            vec!["history", "--all", "--revision", "HEAD"],
            vec!["history", "--limit", "0"],
            vec!["--repo", ".", "--repo", ".", "status"],
            vec!["--timeout-secs", "0", "status"],
            vec!["stash", "drop", "0"],
            vec!["pull", "--merge", "--rebase"],
            vec!["pull", "--ff-only", "--merge"],
            vec!["pull", "--rebase", "--rebase"],
            vec!["merge", "start", "topic", "--squash", "--no-commit"],
            vec!["merge", "start", "topic", "--squash", "--ff-only"],
            vec!["rebase", "start", "HEAD"],
            vec!["rebase", "skip", "--force"],
        ] {
            assert!(parse(args(&case)).is_err(), "{case:?}");
        }
    }
    #[test]
    fn merge_preparation_modes_are_explicit(){
        for(flag,wanted)in[("--squash",rocket_git::MergeMode::Squash),("--no-commit",rocket_git::MergeMode::NoCommit)] {
            assert!(matches!(parse(args(&["merge","start","topic",flag])).unwrap().command,Command::MergeStart{mode,..} if mode==wanted));
        }
    }
    #[test]
    fn reviewed_writes_require_unambiguous_full_ids() {
        for values in [
            vec!["push-lease","origin","main","main"],
            vec!["push-lease","origin","main","main","--source","abcd","--expect","abcd"],
            vec!["push-lease","origin","main","main","--preview","--expect","abcd"],
            vec!["push-atomic","origin","main:a"],
            vec!["push-atomic","origin","--preview","main:a","main:a"],
            vec!["tag","delete","v1"],vec!["tag","delete","v1","--expect","HEAD"],
            vec!["worktree","remove","somewhere"],
            vec!["worktree","create","new","--branch","topic","--existing","--start","HEAD"],
        ] {assert!(parse(args(&values)).is_err(),"{values:?}");}
        let id="A".repeat(40);
        let parsed=parse(args(&["--json","push-lease","origin","main","main","--source",&id,"--expect",&id])).unwrap();
        assert!(matches!(parsed.command,Command::PushLease {source:Some(s),..} if s==id.to_ascii_lowercase()));
        assert!(parse(args(&["push-atomic","origin","--preview","--","main:one","other:two"])).is_ok());
    }
    #[test]
    fn advanced_option_values_do_not_change_global_json_mode() {
        for values in [vec!["tag","create","v1","HEAD","--message","--json"],
            vec!["worktree","create","new","--branch","--json"],
            vec!["push-lease","origin","main","main","--source","--json"],
            vec!["commit-url","HEAD","--remote","--json"]] {
            assert!(!wants_json(&args(&values)),"{values:?}");
        }
    }
    #[test]
    fn selections_conflicts_bisect_and_remote_tags_need_their_review_pins() {
        let sha = "a".repeat(64);
        let sha = sha.as_str();
        let id = "b".repeat(40);
        let id = id.as_str();
        for case in [
            vec!["stage", "--lines", "1", "--", "a"],
            vec!["stage", "--lines", "1", "--expect-patch", sha, "--", "a", "b"],
            vec!["stage", "--all", "--lines", "1", "--expect-patch", sha],
            vec!["unstage", "--expect-patch", sha, "--", "a"],
            vec!["stage", "--lines", "1,1", "--expect-patch", sha, "--", "a"],
            vec!["stage", "--lines", "x", "--expect-patch", sha, "--", "a"],
            vec!["stage", "--lines", "1", "--expect-patch", "abc", "--", "a"],
            vec!["hunks", "--", "a", "b"],
            vec!["stash", "apply", id, "--hunks", "0", "--", "a"],
            vec!["stash", "apply", id, "--hunks", "0", "--lines", "1", "--expect-patch", sha, "--", "a"],
            vec!["stash", "apply", id, "--lines", "1", "--expect-patch", sha, "--", "a", "b"],
            vec!["stash", "apply", id, "--index", "--", "a"],
            vec!["stash", "apply", id, "--preview"],
            vec!["stash", "apply", id, "--part", "staged", "--", "a"],
            vec!["stash", "save", "--lines", "1", "--expect-patch", sha, "--", "a"],
            vec!["stash", "save", "-m", "x", "-u", "--lines", "1", "--expect-patch", sha, "--", "a"],
            vec!["stash", "save", "--preview"],
            vec!["stash", "pop", "0"],
            vec!["stash", "pop", "0", "--expect", "HEAD"],
            vec!["conflict", "resolve", "a", "--expect-sha256", sha],
            vec!["conflict", "resolve", "a", "--expect-sha256", sha, "--choose", "0=ours"],
            vec!["conflict", "resolve", "a", "--expect-sha256", sha, "--choose", "1=mine"],
            vec!["conflict", "resolve", "a", "--expect-sha256", sha, "--choose", "1=ours,1=theirs"],
            vec!["conflict", "resolve", "a", "--expect-sha256", sha, "--choose", "all=ours", "--result", "r.txt"],
            vec!["conflict", "resolve", "a", "--choose", "all=ours"],
            vec!["conflict", "keep", "a", "--side", "deletion", "--expect", id],
            vec!["conflict", "keep", "a", "--side", "ours", "--expect", "missing"],
            vec!["conflict", "keep", "a", "--side", "base", "--expect", id],
            vec!["bisect", "good"],
            vec!["bisect", "bad", "--expect", "HEAD"],
            vec!["bisect", "start", "a"],
            vec!["bisect", "restore", "--expect", id],
            vec!["bisect", "run", "a", "b", "--test-timeout", "10", "--preview", "C:/t.exe"],
            vec!["bisect", "run", "a", "b", "--test-timeout", "10", "--", "C:/t.exe"],
            vec!["bisect", "run", "a", "b", "--test-timeout", "10", "--preview", "--expect-sha256", sha, "--", "C:/t.exe"],
            vec!["bisect", "run", "--continue", "a", "b", "--test-timeout", "10", "--preview", "--", "C:/t.exe"],
            vec!["bisect", "run", "a", "b", "--preview", "--", "C:/t.exe"],
            vec!["bisect", "run", "a", "b", "--test-timeout", "3601", "--preview", "--", "C:/t.exe"],
            vec!["bisect", "run", "a", "b", "--test-timeout", "10", "--preview", "--", "t.exe"],
            vec!["bisect", "run", "a", "b", "--test-timeout", "10", "--preview", "--"],
            vec!["tag", "publish", "v1", "--remote", "origin"],
            vec!["tag", "publish", "v1", "--source", id],
            vec!["tag", "delete-remote", "v1", "--remote", "origin", "--expect", "HEAD"],
        ] {
            assert!(parse(args(&case)).is_err(), "{case:?}");
        }
        let upper = sha.to_ascii_uppercase();
        assert!(matches!(parse(args(&["unstage", "--lines", "3,1", "--expect-patch", &upper, "--", "a"])).unwrap().command,
            Command::StageLines { lines, expected, unstage: true, .. } if lines == [3, 1] && expected == sha));
        assert!(matches!(parse(args(&["stash", "apply", id, "--part", "index", "--", "a", "b"])).unwrap().command,
            Command::StashApplySelection { part: rocket_git::StashPart::Index, paths, expected: None, .. } if paths.len() == 2));
        assert!(matches!(parse(args(&["stash", "save", "-m", "x", "--", "a"])).unwrap().command,
            Command::StashSave { paths, .. } if paths.len() == 1));
        assert!(matches!(parse(args(&["conflict", "resolve", "a", "--expect-sha256", sha, "--choose", "2=both,1=theirs"])).unwrap().command,
            Command::ConflictResolve { choice: crate::conflicts::Choice::Blocks(list), .. } if list.len() == 2));
        assert!(matches!(parse(args(&["conflict", "keep", "a", "--side", "deletion", "--expect", "missing"])).unwrap().command,
            Command::ConflictKeep { expected: None, .. }));
        assert!(matches!(parse(args(&["bisect", "skip", "--expect", &id.to_ascii_uppercase()])).unwrap().command,
            Command::Bisect { action: rocket_git::BisectAction::Skip, expected: Some(e), .. } if e == id));
        assert!(matches!(parse(args(&["bisect", "run", "--continue", "--test-timeout", "5", "--expect-sha256", sha, "--", "C:/t.exe", "--flag", "-x"])).unwrap().command,
            Command::BisectRun { range: None, arguments, timeout: 5, .. } if arguments == ["--flag", "-x"]));
        for values in [
            vec!["stage", "--lines", "--json", "--expect-patch", sha, "--", "a"],
            vec!["conflict", "resolve", "a", "--result", "--json", "--expect-sha256", sha],
            vec!["bisect", "run", "a", "b", "--test-timeout", "--json"],
        ] {
            assert!(!wants_json(&args(&values)), "{values:?}");
        }
    }
    #[test]
    fn history_writes_need_full_ids_and_their_pins() {
        let sha = "c".repeat(64);
        let sha = sha.as_str();
        let id = "d".repeat(40);
        let id = id.as_str();
        for case in [
            vec!["undo-commit", "--branch", "main", "--expect", id],
            vec!["undo-commit", "--branch", "main", "--expect", "HEAD", "--index-sha256", sha],
            vec!["undo-commit", "--expect", id, "--index-sha256", sha],
            vec!["amend", "--message", "x"],
            vec!["amend", "--expect", id],
            vec!["cherry-pick"],
            vec!["cherry-pick", "HEAD"],
            vec!["cherry-pick", id, id, "--mainline", "0"],
            vec!["revert", "continue", id],
            vec!["revert", "abort", "--expect", id],
            vec!["cherry-pick", id, "-m", "1"],
            vec!["reset", id, "--expect", id],
            vec!["reset", "--soft", "--hard", id, "--expect", id],
            vec!["reset", "--soft", "HEAD~1", "--expect", id],
            vec!["reset", "--soft", id],
            vec!["rebase", "start", "main"],
            vec!["rebase", "start", "main", "--expect-original", id],
            vec!["rebase", "start", "main", "--preview", "--expect-original", id, "--expect-onto", id],
            vec!["rebase", "start", "main", "--preserve-merges", "--plan", "p.json", "--preview"],
            vec!["rebase", "start", "main", "--preserve-merges", "--update-refs", "--preview"],
            vec!["rebase", "start", "main", "--preserve-merges", "--interactive", "--preview"],
            vec!["compare", "a"],
            vec!["compare", "a", "b", "--staged"],
            vec!["compare", "a", "--staged", "--worktree"],
            vec!["search", "x", "--all", "--revision", "main"],
            vec!["search", "x", "--kind", "regex"],
            vec!["search", "x", "--regex"],
            vec!["search", "x", "--kind", "path", "--match-case"],
            vec!["search", "x", "--kind", "text", "--path", "src"],
            vec!["search", "x", "--kind", "content", "--path"],
            vec!["grep"],
            vec!["grep", "a", "b"],
            vec!["grep", "x", "--all"],
            vec!["grep", "x", "--kind", "content"],
            vec!["grep", "x", "--revision"],
            vec!["file-history", "a", "--limit", "201"],
            vec!["blame", "HEAD"],
            vec!["blame", "HEAD", "--path", "a", "--size", "501"],
            vec!["reflog", "--scope", "everything"],
            vec!["reflog", "--limit", "2001"],
            vec!["verify", "branch", "main"],
            vec!["signing", "show", "extra"],
        ] {
            assert!(parse(args(&case)).is_err(), "{case:?}");
        }
        let upper = id.to_ascii_uppercase();
        assert!(matches!(parse(args(&["cherry-pick", &upper, id, "--expect", &upper])).unwrap().command,
            Command::CommitAction { request, expected: Some(e), .. } if request.targets.len() == 2 && request.recovery.is_none() && e == id));
        assert!(matches!(parse(args(&["revert", "skip", "--preview"])).unwrap().command,
            Command::CommitAction { request, preview: true, .. } if request.recovery == Some(rocket_git::CommitRecovery::Skip)));
        assert!(matches!(parse(args(&["reset", "--hard", id, "--expect", id, "--preview"])).unwrap().command,
            Command::Reset { mode: rocket_git::ResetMode::Hard, preview: true, .. }));
        assert!(matches!(parse(args(&["rebase", "start", "main", "--plan", "p.json", "--expect-original", id, "--expect-onto", &upper])).unwrap().command,
            Command::RebaseStart { interactive: true, plan: Some(_), expected: Some((_, onto)), .. } if onto == id));
        assert!(matches!(parse(args(&["compare", "main", "--worktree"])).unwrap().command,
            Command::Compare { target: None, staged: false, .. }));
        assert!(matches!(parse(args(&["search", "a(b", "--kind", "content", "--regex", "--match-case", "--path", "src"])).unwrap().command,
            Command::Search { regex: true, match_case: true, path: Some(p), .. } if p == "src"));
        assert!(matches!(parse(args(&["grep", "--", "-x"])).unwrap().command,
            Command::Grep { pattern, revision: None, regex: false, .. } if pattern == "-x"));
        assert!(matches!(parse(args(&["grep", "x", "--revision", "main", "--path", "src"])).unwrap().command,
            Command::Grep { revision: Some(r), path: Some(_), .. } if r == "main"));
        assert!(matches!(parse(args(&["undo-commit", "--branch", "main", "--expect", id, "--index-sha256", &sha.to_ascii_uppercase()])).unwrap().command,
            Command::UndoCommit { index, .. } if index == sha));
        for values in [
            vec!["rebase", "start", "main", "--plan", "--json"],
            vec!["search", "x", "--kind", "--json"],
            vec!["blame", "HEAD", "--path", "a", "--first", "--json"],
        ] {
            assert!(!wants_json(&args(&values)), "{values:?}");
        }
    }
    #[test]
    fn maintenance_lfs_and_ssh_writes_review_first_and_pin_what_they_change() {
        let sha = "e".repeat(64);
        let sha = sha.as_str();
        let id = "f".repeat(40);
        let id = id.as_str();
        for case in [
            vec!["remote", "prune", "origin"],
            vec!["remote", "prune", "origin", "--preview", "--", "refs/remotes/origin/x"],
            vec!["remote", "rename", "origin"],
            vec!["branch", "delete-remote", "origin", "topic"],
            vec!["branch", "delete-remote", "origin", "topic", "--expect", "HEAD"],
            vec!["branch", "fast-forward"],
            vec!["branch", "fast-forward", "--preview", "--", "main"],
            vec!["fetch", "origin", "--all"],
            vec!["worktree", "move", "a", "--expect", id],
            vec!["worktree", "repair", "a", "--expect", id],
            vec!["worktree", "prune", "a", "b", "--expect", id],
            vec!["worktree", "prune", "a"],
            vec!["identity", "set"],
            vec!["identity", "set", "--name", "x", "extra"],
            vec!["tag", "create", "v1", "HEAD", "--sign"],
            vec!["tag", "list", "--order", "oldest"],
            vec!["lfs", "untrack", "--expect", "*.bin filter=lfs"],
            vec!["lfs", "lock", "a.bin"],
            vec!["lfs", "unlock", "a.bin", "--remote", "origin"],
            vec!["lfs", "unlock", "a.bin", "--remote", "origin", "--expect", "1", "--force"],
            vec!["ssh", "use", "work"],
            vec!["ssh", "use"],
            vec!["ssh", "restore", "--preview", "--expect-config", sha],
            vec!["ssh", "use", "work", "--expect-config", "abc"],
        ] {
            assert!(parse(args(&case)).is_err(), "{case:?}");
        }
        assert!(matches!(parse(args(&["fetch", "origin", "--tags"])).unwrap().command, Command::Fetch { tags: true, .. }));
        assert!(matches!(parse(args(&["branch", "fast-forward", "--", "main", "topic"])).unwrap().command,
            Command::BranchFastForward { branches, preview: false } if branches.len() == 2));
        assert!(matches!(parse(args(&["worktree", "repair", "a", "--at", "b", "--expect", &id.to_ascii_uppercase()])).unwrap().command,
            Command::WorktreeMaintain { kind: rocket_git::WorktreeMaintenance::Repair, destination: Some(_), expected, .. } if expected == id));
        assert!(matches!(parse(args(&["lfs", "lock", "art\\a.bin", "--remote", "origin"])).unwrap().command,
            Command::LfsLock { path, unlock: None, .. } if path == "art/a.bin"));
        assert!(matches!(parse(args(&["tag", "list", "--order", "newest"])).unwrap().command,
            Command::TagList { order: rocket_git::TagOrder::Newest, .. }));
        assert!(matches!(parse(args(&["ssh", "restore", "--expect-config", &sha.to_ascii_uppercase()])).unwrap().command,
            Command::SshBinding { alias: None, expected: Some(e), preview: false } if e == sha));
        for values in [
            vec!["identity", "set", "--name", "--json"],
            vec!["worktree", "repair", "a", "--at", "--json", "--expect", id],
            vec!["ssh", "use", "work", "--expect-config", "--json"],
            vec!["lfs", "untrack", "--line", "--json"],
        ] {
            assert!(!wants_json(&args(&values)), "{values:?}");
        }
    }
}
