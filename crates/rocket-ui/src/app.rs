use crate::{
    backend::{Action, Backend, NetworkRead, NetworkRefresh, Outcome, Refresh, RefreshMode, Snapshot, user_error},
    diff::Patch,
    graph,
    persistence::{Preferences, SavedState, Store},
    style,
};
use eframe::egui::*;
use rocket_git::*;
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        mpsc::{self, Receiver},
    },
    time::{Duration, Instant},
};

// Reopened native windows must not accept an old window's expected_tab IDs.
static NEXT_TAB_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
/// Most closed repositories Ctrl+Shift+T can bring back.
const CLOSED_TABS: usize = 20;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    Summary,
    Commits,
    Compare,
    Map,
    BranchReview,
    /// The repository's CI runs from its host.
    Actions,
    FileHistory,
    Tree,
    Conflict,
    Changes,
    Branches,
    Remotes,
    Stashes,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DetailTab {
    Details,
    Files,
    Checks,
}

pub struct Tab {
    pub undo_commit: Option<Arc<UndoCommit>>,
    /// A branch switch finished with Update submodules on checkout on.
    pub(crate) submodule_update: bool,
    pub id: u64,
    pub path: PathBuf,
    pub title: String,
    /// Independent native review tabs expose only read actions.
    pub review_window: bool,
    pub snapshot: Option<Snapshot>,
    pub view: View,
    pub search: String,
    /// What the commit search compares `search` with.
    pub search_kind: HistorySearchKind,
    /// Changed code only: a regular expression (git log -G) instead of exact text.
    pub search_regex: bool,
    /// Changed code only: letters must match exactly.
    pub search_case: bool,
    /// Changed code only: the file or folder typed in the path field.
    pub search_path: String,
    /// The diff find text a Changed code search filled in, so it can clear it again.
    pub(crate) search_find: Option<String>,
    pub branch_filter: String,
    pub worktree_view: crate::worktree_view::Settings,
    pub history_ref: Option<String>,
    pub history_all: bool,
    pub history_order: HistoryOrder,
    pub search_state: crate::history::SearchState,
    pub file_history: crate::file_history::State,
    pub tree: crate::tree::State,
    pub map: crate::repository_map::State,
    pub compare: crate::compare::State,
    pub branch_review: crate::branch_review::State,
    pub stash: crate::stashes::State,
    pub conflict: crate::conflicts::State,
    pub selected_commit: Option<ObjectId>,
    /// Extra commits chosen with Ctrl/Shift-click for a cherry-pick or revert
    /// sequence; it always contains `selected_commit` when it is not empty.
    pub selected_commits: Vec<ObjectId>,
    /// Where a Shift-click range starts: the last plain or Ctrl-clicked row.
    pub selection_anchor: Option<ObjectId>,
    /// The commit whose details popup is open (Space in the commit list).
    pub peek: Option<ObjectId>,
    pub detail_tab: DetailTab,
    pub selected_change: Option<(RepoPath, bool)>,
    /// A rebase or import just paused: select its first conflict once status shows it.
    pub show_conflict: bool,
    pub change_selection: crate::change_drag::Selection,
    pub patch: Option<Patch>,
    pub patch_file: usize,
    pub split_diff: bool,
    pub diff_tools: crate::diff_tools::State,
    pub image: crate::image_view::State,
    pub(crate) image_reload: Option<(ObjectId, RepoPath)>,
    pub conflict_side: u8,
    pub terminal: Option<crate::terminal::TerminalPane>,
    pub patch_error: Option<String>,
    pub draft: String,
    pub error: Option<String>,
    pub graph: Vec<graph::Row>,
    /// Collapsed merge commits in the history list, and the rows left shown.
    pub folding: graph::Folding,
    folded: Vec<Commit>,
    /// How many loaded commits each shown collapsed merge hides.
    pub behind: std::collections::HashMap<ObjectId, usize>,
    /// For each hidden commit, the shown collapsed merge that hides it.
    hidden_in: std::collections::HashMap<ObjectId, ObjectId>,
    pub refresh: Option<ReadTask<Refresh>>,
    pub more: Option<ReadTask<HistoryPage>>,
    pub diff: Option<ReadTask<Patch>>,
    pub background_refresh: bool,
    pub refresh_pending: Option<Instant>,
    last_change: Instant,
    pub(crate) draft_loaded: bool,
    /// Commit template and recent messages for the Changes composer.
    pub composer: crate::composer::Composer,
    pub refresh_count: u64,
    /// Issue-tracker link rules for commit messages.
    pub issue_links: crate::issue_links::State,
    /// CI marks for pushed commits and branches, read from the hosting account.
    pub ci: crate::ci_checks::State,
    /// Someone else's repository opened read-only from Rocket's preview cache.
    pub preview: Option<Box<crate::preview_tab::Preview>>,
    /// The Actions page: the repository's runs from its host.
    pub actions: crate::actions::State,
}

impl Tab {
    pub(crate) fn new(path: PathBuf, log: Option<rocket_git::DiagnosticLog>) -> Self {
        let id = NEXT_TAB_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let title = path.file_name().unwrap_or(path.as_os_str()).to_string_lossy().into_owned();
        Self {
            id,
            undo_commit: None,
            submodule_update: false,
            path,
            title,
            review_window: false,
            snapshot: None,
            view: View::Commits,
            search: String::new(),
            search_kind: HistorySearchKind::Text,
            search_regex: false,
            search_case: false,
            search_path: String::new(),
            search_find: None,
            branch_filter: String::new(),
            worktree_view: Default::default(),
            history_ref: None,
            history_all: false,
            history_order: Default::default(),
            search_state: Default::default(),
            file_history: Default::default(),
            tree: Default::default(),
            map: Default::default(),
            compare: Default::default(),
            branch_review: Default::default(),
            stash: Default::default(),
            conflict: Default::default(),
            selected_commit: None,
            selected_commits: Vec::new(),
            selection_anchor: None,
            peek: None,
            detail_tab: DetailTab::Details,
            selected_change: None,
            show_conflict: false,
            change_selection: crate::change_drag::Selection::default(),
            patch: None,
            patch_file: 0,
            split_diff: false,
            diff_tools: crate::diff_tools::State::with_log(
                log.clone(),
            ),
            image: crate::image_view::State::with_log(log.clone()),
            image_reload: None,
            conflict_side: 0,
            terminal: None,
            patch_error: None,
            draft: String::new(),
            error: None,
            graph: vec![],
            folding: Default::default(),
            folded: Vec::new(),
            behind: std::collections::HashMap::new(),
            hidden_in: std::collections::HashMap::new(),
            refresh: None,
            more: None,
            diff: None,
            background_refresh: false,
            refresh_pending: None,
            last_change: Instant::now(),
            draft_loaded: false,
            composer: Default::default(),
            refresh_count: 0,
            issue_links: Default::default(),
            ci: Default::default(),
            preview: None,
            actions: Default::default(),
        }
    }
    /// A read-only preview tab: no writes, terminal, tools or extensions.
    pub fn is_preview(&self) -> bool {
        self.preview.is_some()
    }
    pub fn loading(&self) -> bool {
        self.refresh.is_some() && !self.background_refresh
    }
    pub fn worktree(&self) -> bool {
        self.snapshot
            .as_ref()
            .is_some_and(|s| s.info.worktree.is_some() && s.status.is_some())
    }
    /// The Changed code options in the search bar, or why the path can't be used.
    pub(crate) fn content_match(&self) -> std::result::Result<ContentMatch, String> {
        Ok(ContentMatch {
            regex: self.search_regex,
            match_case: self.search_case,
            path: crate::history::search_path(&self.search_path)?,
        })
    }
    /// Start the commit search over from the bar. An exact Changed code search also
    /// fills the diff's Find, so the selected commit shows where the text changed.
    pub(crate) fn restart_search(&mut self) {
        self.search_state
            .reset(&self.search, self.search_kind, self.content_match());
        let exact = self.search_kind == HistorySearchKind::Content
            && !self.search_regex
            && !self.search.trim().is_empty();
        let mine = self
            .search_find
            .as_ref()
            .is_some_and(|filled| *filled == self.diff_tools.query);
        if exact && (self.diff_tools.query.is_empty() || mine) {
            let text: String = self.search.chars().take(512).collect();
            self.diff_tools.query = text.clone();
            self.search_find = Some(text);
        } else if !exact && mine {
            self.diff_tools.query.clear();
            self.search_find = None;
        }
    }
    pub fn commits(&self) -> &[Commit] {
        if !self.search.trim().is_empty() {
            return &self.search_state.commits;
        }
        if self.folding.active() {
            return &self.folded;
        }
        self.loaded_commits()
    }
    /// Every loaded history commit, including those a collapsed merge hides.
    pub fn loaded_commits(&self) -> &[Commit] {
        self.snapshot
            .as_ref()
            .and_then(|s| s.history.as_ref())
            .map_or(&[], |h| h.commits.as_slice())
    }
    /// Recompute the shown history rows and their graph after the loaded history
    /// or the collapsed merges change.
    pub fn relayout(&mut self) {
        let commits = self
            .snapshot
            .as_ref()
            .and_then(|s| s.history.as_ref())
            .map_or(&[][..], |h| h.commits.as_slice());
        if self.folding.active() {
            let folded = graph::fold(commits, &self.folding);
            self.folded = folded.visible.iter().map(|&i| commits[i].clone()).collect();
            self.graph = graph::layout(&self.folded, &folded.hidden);
            self.behind = folded.behind;
            self.hidden_in = folded.owner;
        } else {
            self.folded = Vec::new();
            self.behind = std::collections::HashMap::new();
            self.hidden_in = std::collections::HashMap::new();
            self.graph = graph::layout(commits, &std::collections::HashSet::new());
        }
    }
    /// The commits a cherry-pick or revert should use, in application order:
    /// a multi-selection oldest first for cherry-pick and newest first for revert
    /// (the list shows newest first), or else the single selected commit.
    pub fn commit_targets(&self, operation: CommitOperation) -> Vec<ObjectId> {
        if self.selected_commits.len() < 2 {
            return self.selected_commit.iter().cloned().collect();
        }
        let mut chosen: Vec<_> = self
            .commits()
            .iter()
            .filter(|c| self.selected_commits.contains(&c.id))
            .map(|c| c.id.clone())
            .collect();
        if operation == CommitOperation::CherryPick {
            chosen.reverse();
        }
        chosen
    }
    /// Commit & Push can start here: there is a remote to push to, and a branch to push.
    pub(crate) fn commit_pushes(&self) -> bool {
        self.snapshot.as_ref().is_some_and(|s| {
            !s.remotes.is_empty() && matches!(s.head, Some(Head::Branch { .. } | Head::Unborn { .. }))
        })
    }

    /// The current branch as the window names it. Git leaves a branch while it
    /// rebases it, so a paused rebase names it from its own state.
    pub fn branch_label(&self) -> String {
        let snapshot = self.snapshot.as_ref();
        let rebasing = snapshot
            .and_then(|s| s.state.as_ref())
            .and_then(|s| s.step.as_ref())
            .and_then(|s| s.branch.as_deref());
        match rebasing {
            Some(branch) => format!("{} · rebasing", style::display(branch)),
            None => style::head_name(snapshot.and_then(|s| s.head.as_ref())),
        }
    }

    /// Where a rebase or import ends up. Paused: Changes, on its first conflict.
    /// Finished: Commits, on the new tip, which the next refresh selects.
    pub fn after_sequence(&mut self, paused: bool) {
        self.show_conflict = paused;
        if paused {
            self.view = View::Changes;
        } else {
            self.view = View::Commits;
            self.selected_commit = None;
            self.selected_commits.clear();
        }
    }

    /// The review of Continue, once a paused rebase or import can go on as it stands.
    pub fn continue_paused(&self) -> Option<Intent> {
        let snapshot = self.snapshot.as_ref()?;
        let state = snapshot.state.as_ref()?;
        let recover = crate::chrome::Recover::of(state, snapshot.sequence.as_ref())?;
        crate::chrome::pause_text(state, snapshot.status.as_ref(), recover).ready.then(|| recover.intent(CommitRecovery::Continue))
    }

    pub fn can_commit(&self) -> bool {
        let Some(snapshot) = &self.snapshot else {
            return false;
        };
        let (Some(status), Some(state)) = (&snapshot.status, &snapshot.state) else {
            return false;
        };
        let merge = !state.merge_heads.is_empty();
        !self.loading()
            && !self.draft.trim().is_empty()
            // A running Claude Code draft will replace the message.
            && !self.composer.draft.running()
            && (state.is_idle()
                || (merge
                    && !state.bisect
                    && !state.merge_committed
                    && !state.rebase
                    && !state.mail_import
                    && !state.cherry_pick
                    && !state.revert
                    && !state.sequencer
                    && state.merge_autostash.is_none()))
            && !status.entries.iter().any(|e| e.kind == EntryKind::Unmerged)
            && (merge
                || status
                    .entries
                    .iter()
                    .any(|e| e.kind != EntryKind::Untracked && e.index != Change::Unchanged))
    }
    fn cancel_reads(&mut self) {
        self.diff_tools.cancel();
        self.image.clear();
        self.image_reload = None;
        self.stash.stop();
        self.conflict.stop();
        self.file_history.stop();
        self.tree.stop();
        self.map.stop();
        self.compare.stop();
        self.branch_review.stop();
        self.search_state.stop();
        if let Some(task) = &self.refresh {
            task.cancel();
        }
        if let Some(task) = &self.more {
            task.cancel();
        }
        if let Some(task) = &self.diff {
            task.cancel();
        }
    }
}

pub enum Intent {
    GuidedSetup,
    Tool(crate::external_tools::ToolIntent),
    Conflict(crate::conflicts::ConflictIntent),
    Stash(crate::stashes::StashIntent),
    File(crate::file_history::FileIntent),
    Tree(crate::tree::TreeIntent),
    Map(crate::repository_map::MapIntent),
    Actions(crate::actions::ActionsIntent),
    Compare(crate::compare::CompareIntent),
    BranchReview(crate::branch_review::ReviewIntent),
    Open(PathBuf),
    /// Open Explore, searching for or showing this text when given.
    Explore(Option<String>),
    Dialog(DialogKind),
    RunContextCommand { index: usize, context: rocket_git::CommandContext },
    /// Preferences, opened at this page.
    Preferences(crate::preferences::Page),
    /// An enabled extension's page.
    Extension(String),
    /// A command from a menu, the palette or a right-click menu.
    ExtensionCommand(crate::integrations::CommandRun),
    /// The row that stands in for an extension's commands until this is cleared.
    ExtensionFix(String, crate::integrations::Fix),
    /// Preferences → Extensions with the file picker open.
    InstallExtension,
    Playground,
    KeepLocal { snapshot: bool, paths: Vec<RepoPath> },
    /// Open the existing reviewed Sync or Finish workflow from the toolbar.
    BranchWorkflow(rocket_git::BranchWorkflowKind),
    MergeBranch(String),
    /// Open the reviewed rebase of the current branch on top of this branch.
    RebaseOnto(String),
    /// Open Export on Commit series, with the first and last commit when they're chosen.
    ExportSeries(Option<(String, String)>),
    /// A branch or commit was dropped; show its choices.
    Drop(crate::drag::PendingDrop),
    /// Open the interactive rebase review with a plan prepared by a drop.
    ArrangeCommits(crate::drag::Arrangement),
    View(View),
    SelectCommit(ObjectId),
    /// Ctrl-click: add or remove one commit from a multi-selection.
    ToggleCommit(ObjectId),
    /// Space in the commit list: open or close this commit's details popup.
    PeekCommit(ObjectId),
    HistoryOrder(HistoryOrder),
    OpenReview(crate::reviews::Request),
    CommitLink(ObjectId),
    /// Collapse (true) or expand a merge commit in the history list.
    FoldMerge(ObjectId, bool),
    /// Show a commit's CI checks beside where its mark was clicked.
    CiChecks(String, Rect),
    /// Collapse or expand every merge commit in the history list.
    FoldAllMerges(bool),
    /// Shift-click: select the listed commits between the anchor and this one.
    SelectCommitRange(ObjectId),
    SelectChange(StatusEntry, bool),
    ConflictSide(u8),
    Terminal,
    CommitPush,
    SavedSwitch(String),
    /// Switch to this repository tab and open its terminal.
    TerminalFor(u64),
    /// Show this folder in File Explorer.
    ShowFolder(PathBuf),
    /// Open an issue-tracker link from a commit message.
    OpenIssueLink(String),
    /// Review a dropped patch file or pasted patch text.
    ImportPatch(crate::patches::PatchSource),
    /// Apply Patch from Clipboard: ask for the clipboard's text.
    PastePatch,
    /// Reopen the most recently closed repository (Ctrl+Shift+T).
    ReopenTab,
    Refresh,
    ReloadImage,
    More,
    History(Option<String>),
    AllBranches,
    Search,
    /// Open Commits with a Changed code search for the Files search text in this file.
    FindInCommits(Vec<u8>),
    StopSearch,
    Execute(u64, Action),
    CloseTab(u64),
    Exit,
}

#[derive(Clone)]
pub enum DialogKind {
    UndoCommit,
    RepositoryManager,
    /// The Explore page: search GitHub or paste an address.
    Explore,
    /// Keep the preview in tab N as a normal clone.
    KeepPreview(u64),
    Open,
    NewRepository,
    RepositorySettings,
    Signing(Option<SignatureTarget>),
    Clone,
    NewBranch,
    /// New branch from a remote-tracking branch (`upstream`, a refs/remotes/ ref): it
    /// starts there, tracks it, and you switch to it. `name` is the suggested name.
    TrackBranch { upstream: Vec<u8>, name: String },
    Rename(Vec<u8>),
    /// Delete several local branches; `Dialog::first` filters the checklist.
    DeleteBranches,
    /// Confirm running custom command N; the dialog shows the exact command line.
    RunCommand(usize),
    AddRemote,
    EditRemote(Remote, RemoteUrlKind),
    Fetch,
    Pull,
    Push,
    AtomicPush,
    Ignore(Vec<RepoPath>),
    Workflows,
    Stash,
    Merge,
    CommitAction(CommitActionRequest),
    Rebase(RebaseAction),
    MailImport(rocket_git::MailImportAction),
    Bisect,
    Recovery,
    Tags,
    Worktrees,
    Submodules,
    Lfs,
    GitHub,
    Hosting,
    Integrations,
    Statistics,
    Benchmark,
    /// Git Flow: Initialize, Start or Finish, each a reviewed dialog.
    GitFlow(crate::gitflow::Request),
    /// The first-use explanation before a Claude Code draft for this tab.
    DraftNotice(u64),
    FileEdit(Option<(RepoPath, FileEditSource)>),
    Patch(crate::patches::Mode),
    Confirm(Action, String),
    ConflictReload(RepoPath),
    Preferences,
    About,
    /// About › Legal: Rocket's license and the notices of what it includes.
    Legal,
    Changelog,
    CloseTab(u64),
    Quit,
}

pub struct Dialog {
    pub undo: Option<crate::guided::UndoState>,
    pub kind: DialogKind,
    pub owner: Option<u64>,
    pub first: String,
    pub second: String,
    pub flag: bool,
    pub fetch_tags: rocket_git::FetchTags,
    /// What a space in a new branch name becomes.
    pub branch_space: crate::general::BranchSpace,
    pub remotes: Vec<Vec<u8>>,
    pub remote: usize,
    pub branch: Vec<u8>,
    pub mode: MergeMode,
    pub error: Option<String>,
    pub identity: Option<RepositoryIdentity>,
    pub identity_task: Option<ReadTask<RepositoryIdentity>>,
    pub signing: Option<crate::signing::DialogState>,
    pub commit: Option<crate::commit_actions::DialogState>,
    pub merge: Option<crate::merge::DialogState>,
    pub pull: Option<crate::pull::DialogState>,
    pub force_push: Option<crate::force_push::DialogState>,
    pub atomic_push: Option<crate::atomic_push::DialogState>,
    pub ignore: Option<crate::ignore::DialogState>,
    pub workflow: Option<crate::workflows::DialogState>,
    pub rebase: Option<crate::rebase::DialogState>,
    pub mail_import: Option<crate::mail_import::DialogState>,
    pub bisect: Option<crate::bisect::DialogState>,
    pub recovery: Option<crate::recovery::DialogState>,
    pub tags: Option<crate::tags::DialogState>,
    pub worktrees: Option<crate::worktrees::DialogState>,
    pub submodules: Option<crate::submodules::DialogState>,
    pub lfs: Option<crate::lfs::DialogState>,
    pub github: Option<crate::hosting::DialogState>,
    pub file_edit: Option<crate::file_actions::DialogState>,
    pub patch: Option<crate::patches::DialogState>,
    /// Save stash: `Some` saves only these changed files; `None` saves everything.
    pub stash_paths: Option<Vec<RepoPath>>,
    /// Delete branches: the checked local branch names.
    pub selected_branches: Vec<Vec<u8>>,
    /// Preferences → Custom commands: the working copy of the list.
    pub commands: Option<crate::custom_commands::Editor>,
    pub command_context: Option<rocket_git::CommandContext>,
    pub command_values: std::collections::BTreeMap<String, String>,
    /// Repository Settings: the Issue links page.
    pub issue_links: Option<crate::issue_links::Editor>,
    /// Add/Edit remote and Clone: the last Test connection.
    pub connection: Option<crate::remote_check::ConnectionTest>,
    /// Repository Statistics: commit activity and the activity treemap.
    pub statistics: Option<crate::statistics::State>,
    /// Git Flow: settings, branches and the review.
    pub gitflow: Option<crate::gitflow::DialogState>,
    /// File → Open: the folder browser.
    pub folder: Option<crate::folder_browser::Browser>,
    /// New repository and Clone: the folder fields and their check.
    pub create: Option<crate::repository_forms::Form>,
    /// The small dialogs' own reads (merged branches, what a push sends).
    pub small: crate::small_dialogs::State,
}

pub struct Activity {
    pub title: String,
    pub outcome: Outcome,
    pub elapsed: Duration,
    /// When the operation finished, for the drawer's "3 min ago".
    pub at: std::time::SystemTime,
    /// The Activity drawer has shown this entry; its review markers stop asking.
    pub seen: bool,
}
pub struct ActiveWrite {
    pub owner: Option<u64>,
    pub label: String,
    /// What it is doing, in words ("Pushing to origin"); `label` stays the key.
    pub title: String,
    pub task: WriteTask<Outcome>,
    pub fetch_progress: Receiver<FetchProgress>,
    pub clone_progress: Receiver<CloneProgress>,
    pub force_push_progress: Receiver<ForcePushProgress>,
    pub force_push_phase: Option<ForcePushProgress>,
    pub progress: String,
    pub fraction: Option<f32>,
    pub cancelling: bool,
    pub started: Instant,
    pub submitted_draft: Option<String>,
}

impl ActiveWrite {
    pub fn checking_remote(&self) -> bool {
        matches!(
            self.force_push_phase,
            Some(ForcePushProgress::CheckingRemote { .. })
        )
    }
    pub fn can_cancel(&self) -> bool {
        !self.cancelling && !self.checking_remote()
    }
    /// The Stop button's label; button labels carry no trailing ellipsis.
    pub fn cancel_label(&self) -> &'static str {
        if self.checking_remote() {
            "Checking result"
        } else if self.cancelling {
            "Stopping"
        } else {
            "Stop"
        }
    }
    pub fn force_push_phase_code(&self) -> Option<&'static str> {
        if !matches!(self.label.as_str(), "force-push" | "atomic-push") {
            None
        } else if self.cancelling && !self.checking_remote() {
            Some("stopping")
        } else {
            Some(
                self.force_push_phase
                    .unwrap_or(ForcePushProgress::CheckingLease)
                    .code(),
            )
        }
    }
}

#[derive(Clone, Copy)]
pub struct UiControl {
    pub rect: Rect,
    pub enabled: bool,
}
impl From<&Response> for UiControl {
    fn from(response: &Response) -> Self {
        Self {
            rect: response.rect,
            enabled: response.enabled(),
        }
    }
}

struct Startup {
    backend: Backend,
    state: Result<(Arc<Store>, SavedState)>,
}

/// HEAD of each recent repository, for the welcome screen. Reads run only while no
/// repository is open; opening one or quitting drops them, which cancels their Git
/// commands, and the next welcome screen reads again. The reads are Git's read-only
/// HEAD queries, so the repositories are not changed.
#[derive(Default)]
pub struct RecentHeads {
    reads: Vec<(PathBuf, ReadTask<Head>)>,
    /// `None` when HEAD could not be read (moved, deleted, not a repository).
    heads: std::collections::HashMap<PathBuf, Option<Head>>,
}

impl RecentHeads {
    /// `None` until the read finishes; `Some(None)` when HEAD could not be read.
    pub fn get(&self, path: &Path) -> Option<Option<&Head>> {
        self.heads.get(path).map(Option::as_ref)
    }
    pub fn running(&self) -> bool {
        !self.reads.is_empty()
    }
    /// Keep finished results for listed paths, drop reads and results for paths no
    /// longer listed, and start a read for each listed path that has neither.
    fn update(
        &mut self,
        recent: &[PathBuf],
        mut spawn: impl FnMut(&Path) -> Result<ReadTask<Head>>,
    ) {
        let heads = &mut self.heads;
        self.reads.retain_mut(|(path, task)| {
            if !recent.contains(path) {
                return false; // Dropping a read cancels it without waiting.
            }
            match task.try_finish() {
                Some(result) => {
                    heads.insert(path.clone(), result.ok());
                    false
                }
                None => true,
            }
        });
        heads.retain(|path, _| recent.contains(path));
        for path in recent {
            if self.heads.contains_key(path) || self.reads.iter().any(|(p, _)| p == path) {
                continue;
            }
            match spawn(path) {
                Ok(task) => self.reads.push((path.clone(), task)),
                // No worker means no result will arrive; do not retry every frame.
                Err(_) => {
                    self.heads.insert(path.clone(), None);
                }
            }
        }
    }
    fn clear(&mut self) {
        self.reads.clear();
        self.heads.clear();
    }
}

pub struct Rocket {
    pub updates: crate::app_updates::State,
    pub setup: crate::guided::Setup,
    pub library: crate::repository_library::Library,
    pub manager: crate::repository_manager::State,
    pub activity_history: crate::activity_history::Panel,
    pub crash_reports: crate::crash_reports::State,
    pub benchmark: crate::benchmark::State,
    pub external: crate::external_tools::State,
    pub mcp: crate::mcp::Panel,
    pub integrations: crate::integrations::State,
    /// Preferences: the page shown and the settings search.
    pub preferences: crate::preferences::State,
    pub git_settings: crate::git_settings::State,
    /// A Browse picker drawn over Preferences.
    pub picker: Option<crate::folder_browser::Picker>,
    /// Set only by the isolated native MCP host: folder browsing stays inside it.
    pub browse_root: Option<PathBuf>,
    pub backend: Option<Backend>,
    transport_scope: Option<rocket_git::TransportScope>,
    startup: Option<ReadTask<Startup>>,
    pending_paths: Vec<PathBuf>,
    pub startup_error: Option<String>,
    pub log_path: PathBuf,
    pub tabs: Vec<Tab>,
    pub reviews: Vec<crate::reviews::Window>,
    /// MCP: review windows open without taking the foreground.
    pub background_reviews: bool,
    pub pending_review: Option<(PathBuf, crate::reviews::Request)>,
    pub commit_link: Option<crate::commit_links::State>,
    /// CI marks' accounts, the checks popover and the re-run review.
    pub ci: crate::ci_checks::Shared,
    pub active: usize,
    pub recent: Vec<PathBuf>,
    pub recent_heads: RecentHeads,
    pub dialog: Option<Dialog>,
    pub write: Option<ActiveWrite>,
    pub activities: Vec<Activity>,
    pub show_console: bool,
    /// Session preference: review, never automatically mutate, child checkouts after switching.
    pub review_submodules_after_switch: bool,
    pub show_terminal: bool,
    /// The bottom drawer shows only its Terminal/Activity strip. Operations that reveal
    /// Activity keep it collapsed; asking for Activity or the terminal opens it.
    pub drawer_collapsed: bool,
    /// Optional test-owned PSReadLine history, configured before a terminal opens.
    pub terminal_history_file: Option<PathBuf>,
    /// Set only by the isolated native MCP host, never by repository contents.
    pub submodule_fixture_root: Option<PathBuf>,
    pub lfs_source_scope: Option<rocket_git::LfsSourceScope>,
    pub lfs_lock_source_scope: Option<rocket_git::LfsSourceScope>,
    /// GitHub sign-in, reused until GitHub refuses it or Rocket exits. Memory only.
    pub github: Option<Arc<GitHubSession>>,
    pub forge: crate::forge::State,
    pub ssh: crate::ssh_keys::State,
    /// Where GitHub requests go; `None` turns them off (the MCP host's default).
    pub github_endpoint: Option<GitHubEndpoint>,
    /// Set only by the isolated native MCP host and tests: GitHub links and folders
    /// for File Explorer are recorded here instead of opening the browser or Explorer.
    pub opened_urls: Option<Vec<String>>,
    pub(crate) closing_terminals: Vec<crate::terminal::TerminalPane>,
    pub(crate) terminal_log: crate::terminal::EventLog,
    /// Repositories closed this session, most recent last, for Ctrl+Shift+T.
    pub closed_tabs: Vec<PathBuf>,
    pub keybindings: crate::shortcuts::State,
    pub show_sidebar: bool,
    /// Persisted: off shows the sidebar as an icon rail.
    pub sidebar_labels: bool,
    /// Session only: labels were turned on explicitly while the window was narrow.
    pub sidebar_labels_narrow: bool,
    /// Persisted: list open repositories in the sidebar instead of tabs above it.
    pub repository_list: bool,
    /// Persisted: spell-check commit messages (Windows and Linux).
    pub spell_check: bool,
    /// Persisted: the repository, diff and commit list settings on Preferences → General.
    pub general: crate::general::Settings,
    /// Persisted: the shell chosen on Preferences → Terminal.
    pub terminal_shell: crate::terminal_settings::State,
    /// Persisted: drafting commit messages with Claude Code (off by default).
    pub drafting: crate::commit_draft::Settings,
    pub(crate) spelling: crate::spell::Spelling,
    pub palette: crate::palette::Palette,
    /// Help → Guide: the open topic, drawn above any dialog; `None` while closed.
    pub(crate) guide: Option<crate::guide::Topic>,
    pub show_details: bool,
    pub row_height: f32,
    /// Persisted: the commit message area's height after its handle was dragged;
    /// None follows the message's length.
    pub commit_height: Option<f32>,
    /// Persisted: the interface language picked on Preferences → General; None matches
    /// the system's. [`Rocket::update_language`] shows it.
    pub language: Option<crate::i18n::Language>,
    /// Persisted: the theme chosen in Preferences → Appearance.
    pub theme: crate::theme::Settings,
    /// The Custom theme's picture behind the main window's panes.
    pub(crate) backdrop: crate::theme::picture::Backdrop,
    /// The palette applied last, so a frame applies only a change.
    pub(crate) look: Option<crate::theme::Look>,
    /// A chosen picture being copied into Rocket's storage.
    pub(crate) theme_import: Option<std::sync::mpsc::Receiver<std::result::Result<String, String>>>,
    /// Why the last chosen picture could not be used.
    pub(crate) theme_import_error: Option<String>,
    pub notice: Option<String>,
    /// When Apply Patch from Clipboard asked for the clipboard's text.
    pub paste_patch: Option<Instant>,
    /// The text field the Edit menu acts on: the one focused when the menu opened.
    pub(crate) edit_target: Option<crate::menus::EditTarget>,
    /// An Edit menu command for the next frame, with the field it acts on.
    pub(crate) text_command: Option<(crate::menus::EditTarget, crate::menus::TextCommand)>,
    /// A dropped branch or commit waiting for its menu choice.
    pub pending_drop: Option<crate::drag::PendingDrop>,
    /// Automatic background fetch of open repositories (off unless turned on).
    pub auto_fetch: crate::auto_fetch::AutoFetch,
    /// User-defined commands, in menu order.
    pub commands: Vec<rocket_git::CustomCommand>,
    pub quitting: bool,
    pub frame_widgets: std::collections::HashMap<String, UiControl>,
    pub(crate) context: Context,
    pub auto_refresh: bool,
    pub secret_protection: bool,
    pub guard_ai_api_keys: bool,
    /// The native application permits this user preference; MCP-hosted windows do not automate it.
    pub secret_protection_editable: bool,
    pub dependencies: crate::dependency_security::State,
    previous_auto_refresh: bool,
    pub state_directory: PathBuf,
    /// Explore's search, results and limits; kept while previews are open.
    pub explore: crate::explore::State,
    pub(crate) preview_cache: Option<rocket_git::PreviewCache>,
    /// Where previews stop downloading unless lifted for one preview.
    pub preview_limit: u64,
    /// The preview tab Keep is running for.
    pub(crate) keeping_preview: Option<u64>,
    pub state_error: Option<String>,
    state_store: Option<Arc<Store>>,
    saved: SavedState,
    written: SavedState,
    save_task: Option<WriteTask<SavedState>>,
    save_dirty_since: Option<Instant>,
    save_last_change: Instant,
}

impl Rocket {
    pub fn new(ctx: &Context, paths: Vec<PathBuf>) -> Self {
        #[cfg(windows)]
        let log_path = std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir)
            .join("Rocket/logs");
        // Without a home folder, a temporary one, as without LOCALAPPDATA on Windows.
        #[cfg(not(windows))]
        let log_path = rocket_git::local_storage::directory()
            .unwrap_or_else(|| std::env::temp_dir().join("rocket"))
            .join("logs");
        let state_path = log_path.parent().expect("Rocket directory").join("state");
        let mut app=Self::with_storage(ctx, paths, log_path, state_path);
        app.updates.endpoint=Some(rocket_git::app_update::UpdateEndpoint::Public);
        app
    }
    pub fn with_log_directory(ctx: &Context, paths: Vec<PathBuf>, log_path: PathBuf) -> Self {
        let state_path = log_path.join("state");
        Self::with_storage(ctx, paths, log_path, state_path)
    }
    pub fn with_storage(
        ctx: &Context,
        paths: Vec<PathBuf>,
        log_path: PathBuf,
        state_directory: PathBuf,
    ) -> Self {
        Self::with_storage_scoped(ctx, paths, log_path, state_directory, None)
    }
    pub fn with_storage_scoped(
        ctx: &Context,
        paths: Vec<PathBuf>,
        log_path: PathBuf,
        state_directory: PathBuf,
        transport_scope: Option<rocket_git::TransportScope>,
    ) -> Self {
        let mut app = Self {
            updates: Default::default(),
            library: Default::default(),
            manager: Default::default(),
            activity_history: Default::default(),
            crash_reports: crate::crash_reports::State::new(rocket_git::crash_reports::directory().map(PathBuf::from).unwrap_or_else(||log_path.join("crashes"))),
            benchmark: Default::default(),
            external: Default::default(),
            mcp: crate::mcp::Panel::new(state_directory.join("mcp")),
            integrations: crate::integrations::State::new(state_directory.clone()),
            preferences: Default::default(),
            git_settings: Default::default(),
            picker: None,
            browse_root: None,
            backend: None,
            transport_scope,
            startup: None,
            pending_paths: paths,
            startup_error: None,
            log_path,
            tabs: vec![],
            active: 0,
            recent: vec![],
            recent_heads: Default::default(),
            dialog: None,
            write: None,
            activities: vec![],
            show_console: false,
            review_submodules_after_switch: false,
            show_terminal: false,
            drawer_collapsed: false,
            terminal_history_file: None,
            submodule_fixture_root: None,
            lfs_source_scope: None,
            lfs_lock_source_scope: None,
            github: None,
            github_endpoint: Some(GitHubEndpoint::Public),
            forge: crate::forge::State::default(),
            ssh: crate::ssh_keys::State::default(),
            opened_urls: None,
            closing_terminals: Vec::new(),
            terminal_log: Default::default(),
            closed_tabs: Vec::new(),
            keybindings: Default::default(),
            show_sidebar: true,
            sidebar_labels: true,
            sidebar_labels_narrow: false,
            repository_list: false,
            spell_check: false,
            general: Default::default(),
            terminal_shell: Default::default(),
            drafting: Default::default(),
            spelling: Default::default(),
            palette: Default::default(),
            guide: None,
            show_details: true,
            row_height: 33.0,
            commit_height: None,
            language: None,
            theme: Default::default(),
            backdrop: Default::default(),
            look: None,
            theme_import: None,
            theme_import_error: None,
            notice: None,
            paste_patch: None,
            edit_target: None,
            text_command: None,
            pending_drop: None,
            auto_fetch: Default::default(),
            commands: Vec::new(),
            quitting: false,
            frame_widgets: Default::default(),
            setup: Default::default(),
            reviews: Vec::new(),
            background_reviews: false,
            pending_review: None,
            commit_link: None,
            ci: Default::default(),
            context: ctx.clone(),
            auto_refresh: true,
            secret_protection: true,
            guard_ai_api_keys: true,
            secret_protection_editable: true,
            dependencies: Default::default(),
            previous_auto_refresh: true,
            state_directory,
            explore: Default::default(),
            preview_cache: None,
            preview_limit: rocket_git::PREVIEW_LIMIT,
            keeping_preview: None,
            state_error: None,
            state_store: None,
            saved: SavedState::default(),
            written: SavedState::default(),
            save_task: None,
            save_dirty_since: None,
            save_last_change: Instant::now(),
        };
        app.start_backend();
        app.remove_stale_previews();
        // Font discovery stays complete before the first frame, while Git and
        // saved-state discovery can run alongside this cold-start disk work.
        style::install(ctx);
        app
    }
    /// Previews a crashed Rocket left behind: delete those no running Rocket holds.
    /// Only looks when the folder exists, on a worker.
    fn remove_stale_previews(&self) {
        let root = self.preview_root();
        if root.is_dir() {
            let _ = std::thread::Builder::new()
                .name("rocket-preview-cleanup".into())
                .spawn(move || {
                    if let Ok(cache) = rocket_git::PreviewCache::open(root) {
                        cache.remove_stale();
                    }
                });
        }
    }
    pub fn start_backend(&mut self) {
        if self.startup.is_some() {
            return;
        }
        self.startup_error = None;
        let directory = self.log_path.clone();
        let state_directory = self.state_directory.clone();
        let transport_scope = self.transport_scope.clone();
        match crate::read_task::spawn(&self.context, move |cancel| {
            let selected = crate::git_settings::selection(&state_directory)?;
            let backend = Backend::discover_selected(&directory, transport_scope, selected.as_deref(), &cancel)?;
            let state = backend.log.request("load_state", |_| {
                Store::open(&state_directory)
                    .map(|(store, state)| (Arc::new(store), state))
                    .map_err(|source| Error::Io {
                        action: "load drafts and settings",
                        source,
                    })
            });
            Ok(Startup { backend, state })
        }) {
            Ok(task) => self.startup = Some(task),
            Err(error) => self.startup_error = Some(user_error(&error)),
        }
    }
    pub fn tab(&self) -> Option<&Tab> {
        self.tabs.get(self.active)
    }
    pub fn can_write(&self) -> bool {
        self.write.is_none()
            && !self.quitting
            && self.tab().is_some_and(|t| t.worktree() && !t.loading() && !t.is_preview())
    }
    /// Metadata operations such as tags and linked worktrees also support bare repositories.
    /// Keep working-file actions on can_write(), including staging, commits and checkout.
    pub fn can_write_repository(&self) -> bool {
        self.write.is_none() && !self.quitting
            && self.tab().is_some_and(|t| t.snapshot.is_some() && !t.loading() && !t.is_preview())
    }
    pub fn busy(&self) -> bool {
        self.working() || self.state_pending() || self.forge.drafts.saving()
            || (self.quitting && self.integrations.exit_pending())
            || (self.quitting && self.terminal_log.busy())
    }
    pub(crate) fn profile_storage_ready(&self) -> bool { self.state_store.is_some() }
    /// Running work, not counting a drafts/settings save that waits out its debounce.
    pub(crate) fn working(&self) -> bool {
        self.git_settings.running()
            || self.benchmark.running()
            || self.setup.running()
            || self.mcp.busy()
            || (self.quitting && self.integrations.running())
            || self.forge.running()
            || self.ssh.running()
            || (self.quitting && self.external.running())
            || (self.quitting && self.dependencies.running())
            || (self.quitting && self.updates.running())
            || (self.quitting && self.crash_reports.exporting())
            || (self.quitting && self.spelling.waiting())
            || (self.quitting && self.manager.status.batch.is_some())
            || self.dialog.as_ref().is_some_and(|d| {
                d.identity_task.is_some()
                    || d.undo.as_ref().is_some_and(|s| s.task.is_some())
                    || d.signing.as_ref().is_some_and(|s| s.running())
                    || d.commit.as_ref().is_some_and(|c| c.task.is_some())
                    || d.merge.as_ref().is_some_and(|m| m.task.is_some())
                    || d.pull.as_ref().is_some_and(|s| s.running())
                    || d.force_push.as_ref().is_some_and(|s| s.running())
                    || d.atomic_push.as_ref().is_some_and(|s|s.running())
                    || d.ignore.as_ref().is_some_and(|s|s.running())
                    || d.workflow.as_ref().is_some_and(|s|s.running())
                    || d.rebase.as_ref().is_some_and(|r| r.task.is_some())
                    || d.mail_import.as_ref().is_some_and(|r| r.task.is_some())
                    || d.bisect.as_ref().is_some_and(|s| s.task.is_some())
                    || d.recovery.as_ref().is_some_and(|r| r.reading())
                    || d.small.running()
                    || d.tags.as_ref().is_some_and(|t| t.running())
                    || d.worktrees.as_ref().is_some_and(|t| t.running() || t.reading_statuses())
                    || d.submodules.as_ref().is_some_and(|t| t.running())
                    || d.lfs.as_ref().is_some_and(|t| t.running())
                    || d.file_edit.as_ref().is_some_and(|t| t.running())
                    || d.patch.as_ref().is_some_and(|t| t.running())
                    || d.issue_links.as_ref().is_some_and(|e| e.running())
                    || d.connection.as_ref().is_some_and(|c| c.running())
                    || d.gitflow.as_ref().is_some_and(|s| s.running())
                    || d.create.as_ref().is_some_and(|f| f.checking())
            })
            || (self.quitting
                && (self.tabs.iter().any(|t| {
                    t.terminal
                        .as_ref()
                        .is_some_and(|p| !p.session.status().closed)
                }) || !self.closing_terminals.is_empty()))
            || self.manager.running()
            || self.startup.is_some()
            || self.recent_heads.running()
            || self.paste_patch.is_some()
            || self.write.is_some()
            || self.tabs.iter().any(|t| {
                t.refresh.is_some()
                    || t.more.is_some()
                    || t.diff.is_some()
                    || t.diff_tools.running()
                    || t.diff_tools.words_running()
                    || t.image.running()
                    || t.search_state.task.is_some()
                    || t.file_history.running()
                    || t.tree.running()
                    || t.map.running()
                    || t.compare.running()
                    || t.branch_review.running()
                    || t.stash.running()
                    || t.conflict.running()
                    || t.issue_links.running()
                    || t.composer.draft.running()
            })
            || (!self.quitting && self.tab().is_some_and(|t| t.search_state.due.is_some()))
            || (self.dialog.is_none()
                && self
                    .tab()
                    .is_some_and(|t| self.auto_refresh && t.refresh_pending.is_some()))
    }

    /// Background refresh and saving must never prevent typing or navigation.
    pub fn interaction_busy(&self) -> bool {
        self.startup.is_some()
            || self.write.is_some()
            || self
                .tabs
                .iter()
                .any(|t| t.loading() || t.more.is_some() || t.diff.is_some())
    }
    pub fn state_pending(&self) -> bool {
        self.save_task.is_some() || self.save_dirty_since.is_some()
    }
    /// The drafts/settings write is running, not just waiting out its debounce.
    pub(crate) fn saving(&self) -> bool {
        self.save_task.is_some()
    }

    pub(crate) fn can_retry_state_save(&self) -> bool {
        self.state_store.is_some() && self.state_error.is_some() && self.save_task.is_none()
    }

    /// Retry a failed write using the current drafts/settings. A failed initial
    /// load has no owned store and must never overwrite the unread state file.
    pub fn retry_state_save(&mut self) -> bool {
        if !self.can_retry_state_save() {
            return false;
        }
        self.state_error = None;
        self.save_dirty_since = Some(Instant::now());
        true
    }

    pub fn open(&mut self, path: PathBuf) {
        self.external.open = false;
        self.activity_history.close();
        if self.quitting {
            return;
        }
        if self.backend.is_none() {
            self.pending_paths.push(path);
            return;
        }
        if let Some(index) = self.tabs.iter().position(|t| t.path == path) {
            self.active = index;
            return;
        }
        let tab = Tab::new(path, self.backend.as_ref().map(|b| b.log.clone()));
        let id = tab.id;
        self.tabs.push(tab);
        self.active = self.tabs.len() - 1;
        if let Some(view) = self.saved.worktree_views.get(&self.tabs[self.active].path).cloned() {
            view.restore(&mut self.tabs[self.active]);
        }
        self.refresh(id);
    }

    pub(crate) fn inherit_worktree_view(&mut self, owner: Option<u64>, path: &Path) {
        if let Some(tab) = self.tabs.iter().find(|t| Some(t.id) == owner) {
            self.saved.worktree_views.entry(path.to_owned())
                .or_insert_with(|| crate::worktree_view::Settings::capture(tab));
        }
    }

    pub fn refresh(&mut self, id: u64) {
        self.refresh_inner(id, false);
    }
    fn refresh_inner(&mut self, id: u64, background: bool) {
        self.refresh_after_network(id, background, None);
    }
    fn refresh_after_network(&mut self, id: u64, background: bool, completed: Option<NetworkRefresh>) {
        if self.quitting {
            return;
        }
        let Some(backend) = self.backend.clone() else {
            return;
        };
        if self.write.as_ref().is_some_and(|w| w.owner == Some(id)) {
            return;
        }
        let Some(tab) = self.tabs.iter_mut().find(|t| t.id == id) else {
            return;
        };
        if tab.preview.as_ref().is_some_and(|p| !p.ready()) {
            return;
        }
        if tab.refresh.is_some() {
            if !background && tab.background_refresh {
                tab.refresh = None; // Explicit refresh/history changes supersede automatic reads.
            } else { return; }
        }
        if !background {
            tab.search_state.stop();
            tab.more = None;
            tab.diff = None; // Dropped readers cancel; old results have no receiver in this tab.
            tab.patch = None;
        }
        tab.background_refresh = background;
        tab.refresh_pending = None;
        tab.error = None;
        let path = tab.path.clone();
        let start = if tab.history_all {
            HistoryStart::AllBranches
        } else {
            tab.history_ref
                .clone()
                .map_or(HistoryStart::Head, HistoryStart::Revision)
        };
        let context = self.context.clone();
        let order = tab.history_order;
        let mode = if background { RefreshMode::Automatic(tab.snapshot.as_ref().and_then(Snapshot::refresh_base)) } else { RefreshMode::Manual };
        let wake: Option<Arc<dyn Fn() + Send + Sync>> = self
            .auto_refresh
            .then(|| Arc::new(move || context.request_repaint_of(eframe::egui::ViewportId::ROOT)) as _);
        match crate::read_task::spawn(&self.context, move |cancel| {
            cancel.check()?;
            if let Some(snapshot) = completed.and_then(|read| read.into_matching(&path, &start, order, wake.is_some())) {
                // Same handoff as an ordinary full read: its live watcher also
                // covers changes between reconciliation and UI publication.
                return Ok(Refresh::Full(snapshot));
            }
            backend.refresh_ordered(&path, start, order, &cancel, wake, mode)
        }) {
            Ok(task) => tab.refresh = Some(task),
            Err(error) => tab.error = Some(user_error(&error)),
        }
    }

    fn load_diff(&mut self, id: u64, target: DiffTarget, paths: Vec<RepoPath>) {
        let Some(backend) = self.backend.clone() else {
            return;
        };
        let Some(tab) = self.tabs.iter_mut().find(|t| t.id == id) else {
            return;
        };
        if !matches!(&target, DiffTarget::Commit { id } if tab.image_reload.as_ref().is_some_and(|(commit, _)| commit == id))
        {
            tab.image_reload = None;
        }
        tab.diff = None;
        tab.image.clear();
        tab.diff_tools.cancel();
        tab.patch = None;
        tab.patch_error = None;
        tab.patch_file = 0;
        let Some(repository) = tab.snapshot.as_ref().map(|s| s.repository.clone()) else {
            return;
        };
        match crate::read_task::spawn(&self.context, move |cancel| {
            backend.diff(
                &repository,
                &DiffOptions {
                    target,
                    paths,
                    ..Default::default()
                },
                &cancel,
            )
        }) {
            Ok(task) => tab.diff = Some(task),
            Err(error) => tab.patch_error = Some(user_error(&error)),
        }
    }

    pub(crate) fn current_diff(&mut self, id: u64) {
        let Some(tab) = self.tabs.iter_mut().find(|t| t.id == id) else {
            return;
        };
        // Compare owns pinned revisions and its selected-file worker. A status
        // refresh must not replace that patch with the current HEAD's diff.
        if matches!(tab.view, View::Compare | View::BranchReview) {
            return;
        }
        tab.diff = None;
        tab.image.clear();
        tab.diff_tools.cancel();
        tab.patch = None;
        tab.patch_error = None;
        if matches!(tab.view, View::Summary | View::FileHistory | View::Conflict | View::Tree | View::Map | View::Actions) {
            return;
        }
        if tab.view == View::Changes {
            if let Some((path, staged)) = &tab.selected_change {
                if tab
                    .snapshot
                    .as_ref()
                    .and_then(|s| s.status.as_ref())
                    .is_some_and(|s| {
                        s.entries
                            .iter()
                            .any(|e| e.path == *path && e.kind == EntryKind::Unmerged)
                    })
                {
                    let path = path.clone();
                    let side = tab.conflict_side;
                    let repository = tab.snapshot.as_ref().unwrap().repository.clone();
                    if let Some(backend) = self.backend.clone() {
                        match crate::read_task::spawn(&self.context, move |cancel| {
                            backend.conflict_preview(&repository, &path, side, &cancel)
                        }) {
                            Ok(task) => tab.diff = Some(task),
                            Err(error) => tab.patch_error = Some(user_error(&error)),
                        }
                    }
                    return;
                }
                if !staged
                    && tab
                        .snapshot
                        .as_ref()
                        .and_then(|s| s.status.as_ref())
                        .is_some_and(|s| {
                            s.entries
                                .iter()
                                .any(|e| e.path == *path && e.kind == EntryKind::Untracked)
                        })
                {
                    let path = path.clone();
                    let repository = tab.snapshot.as_ref().unwrap().repository.clone();
                    if let Some(backend) = self.backend.clone() {
                        match crate::read_task::spawn(&self.context, move |cancel| {
                            backend.preview(&repository, &path, &cancel)
                        }) {
                            Ok(task) => tab.diff = Some(task),
                            Err(error) => tab.patch_error = Some(user_error(&error)),
                        }
                    }
                    return;
                }
                let mut paths = vec![path.clone()];
                if let Some(original) = tab
                    .snapshot
                    .as_ref()
                    .and_then(|s| s.status.as_ref())
                    .and_then(|s| s.entries.iter().find(|e| e.path == *path))
                    .and_then(|e| e.original_path.as_ref())
                {
                    paths.push(original.clone());
                }
                let staged = *staged;
                self.load_diff(
                    id,
                    if staged {
                        DiffTarget::Staged
                    } else {
                        DiffTarget::Unstaged
                    },
                    paths,
                );
            }
        } else if let Some(commit) = tab.selected_commit.clone() {
            self.load_diff(id, DiffTarget::Commit { id: commit }, vec![]);
        }
    }

    pub fn execute(&mut self, owner: u64, action: Action) {
        if self.write.is_some() || self.quitting || self.dependency_push_review_open() {
            return;
        }
        self.sync_dependency_push_check();
        // A background fetch steps aside so the write gets the repository at once.
        self.auto_fetch.stand_aside(owner);
        let Some(backend) = self.backend.clone() else {
            return;
        };
        let Some(tab) = self.tabs.iter_mut().find(|t| t.id == owner) else {
            return;
        };
        if tab.loading() {
            self.notice = Some("Wait for repository refresh before starting a write.".into());
            return;
        }
        tab.cancel_reads();
        tab.refresh = None;
        tab.refresh_pending = None;
        tab.more = None;
        tab.diff = None;
        let path = tab.path.clone();
        let label = action.label().to_owned();
        // Another operation ends the one-action Undo offer. A failed Undo can
        // be reviewed again; its fresh backend guard still checks every boundary.
        if !matches!(action, Action::UndoCommit(_)) { tab.undo_commit = None; }
        let title = crate::operation_names::running_action(&action);
        let submitted_draft = match &action {
            Action::Commit(m) | Action::ContinueMerge(m) => Some(m.clone()),
            Action::Workflow {review,..} => match review.as_ref() {crate::workflows::Review::CommitPush(p)=>Some(p.message().into()),_=>None},
            _ => None,
        };
        let (tx, rx) = mpsc::sync_channel(32);
        let (_, clone_rx) = mpsc::sync_channel(1);
        let (force_push_tx, force_push_rx) = mpsc::sync_channel(4);
        let network = matches!(action, Action::Fetch(_) | Action::FetchWithTags(..) | Action::PullFetch).then(|| {
            let context = self.context.clone();
            NetworkRead {
                start: if tab.history_all { HistoryStart::AllBranches }
                    else { tab.history_ref.clone().map_or(HistoryStart::Head, HistoryStart::Revision) },
                order: tab.history_order,
                wake: self.auto_refresh.then(|| Arc::new(move || context.request_repaint_of(ViewportId::ROOT)) as _),
            }
        });
        match WriteTask::spawn(move |cancel| {
            backend.execute_with_network_refresh(
                &path,
                &action,
                &tx,
                Some(&force_push_tx),
                network.as_ref(),
                &cancel,
            )
        }) {
            Ok(task) => {
                let initial_progress = if label.starts_with("submodule-") {
                    "Checking, preparing and verifying the selected checkout…"
                } else {
                    "Starting…"
                };
                self.write = Some(ActiveWrite {
                    owner: Some(owner),
                    label,
                    title,
                    task,
                    fetch_progress: rx,
                    clone_progress: clone_rx,
                    force_push_progress: force_push_rx,
                    force_push_phase: None,
                    progress: initial_progress.into(),
                    fraction: None,
                    cancelling: false,
                    started: Instant::now(),
                    submitted_draft,
                })
            }
            Err(error) => self.notice = Some(user_error(&error)),
        }
    }

    pub fn clone_repository(&mut self, source: CloneSource, destination: PathBuf) {
        if self.write.is_some() || self.quitting {
            return;
        }
        let Some(backend) = self.backend.clone() else {
            return;
        };
        let (tx, rx) = mpsc::sync_channel(32);
        let (_, fetch_rx) = mpsc::sync_channel(1);
        let (_, force_push_rx) = mpsc::sync_channel(1);
        match WriteTask::spawn(move |cancel| {
            backend.clone_repository(source, destination, &tx, &cancel)
        }) {
            Ok(task) => {
                self.write = Some(ActiveWrite {
                    owner: None,
                    label: "clone".into(),
                    title: crate::operation_names::running("clone"),
                    task,
                    fetch_progress: fetch_rx,
                    clone_progress: rx,
                    force_push_progress: force_push_rx,
                    force_push_phase: None,
                    progress: "Preparing…".into(),
                    fraction: None,
                    cancelling: false,
                    started: Instant::now(),
                    submitted_draft: None,
                })
            }
            Err(error) => self.notice = Some(user_error(&error)),
        }
    }

    pub fn init_repository(&mut self, destination: PathBuf, branch: String) {
        if self.write.is_some() || self.quitting {
            return;
        }
        let Some(backend) = self.backend.clone() else {
            return;
        };
        let (_, fetch_rx) = mpsc::sync_channel(1);
        let (_, clone_rx) = mpsc::sync_channel(1);
        let (_, force_push_rx) = mpsc::sync_channel(1);
        match WriteTask::spawn(move |cancel| {
            backend.init_repository(&destination, &branch, &cancel)
        }) {
            Ok(task) => {
                self.write = Some(ActiveWrite {
                    owner: None,
                    label: "init".into(),
                    title: crate::operation_names::running("init"),
                    task,
                    fetch_progress: fetch_rx,
                    clone_progress: clone_rx,
                    force_push_progress: force_push_rx,
                    force_push_phase: None,
                    progress: "Preparing repository…".into(),
                    fraction: None,
                    cancelling: false,
                    started: Instant::now(),
                    submitted_draft: None,
                })
            }
            Err(error) => self.notice = Some(user_error(&error)),
        }
    }

    pub fn cancel_write(&mut self) {
        if let Some(write) = &mut self.write {
            if !write.can_cancel() {
                return;
            }
            write.cancelling = true;
            write.task.cancel();
            write.progress = if matches!(write.label.as_str(), "force-push" | "atomic-push") {
                "Stopping…"
            } else {
                "Stopping; waiting for Git and repository checks…"
            }
            .into();
            write.fraction = None;
        }
    }

    pub fn show_dialog(&mut self, kind: DialogKind) {
        if self.quitting {
            return;
        }
        if !matches!(kind, DialogKind::RepositoryManager) {
            if self.manager.running() { self.manager.cancel(); }
            self.manager.cancel_form();
        } else {
            self.external.open = false; self.activity_history.close();
        }
        // Reopening the page that is already shown keeps its selection and reads.
        if let Some(page) = crate::pages::Page::of(&kind)
            && self.page() == Some(page)
        {
            return;
        }
        let tab = self.tab();
        let owner = if matches!(kind, DialogKind::RepositoryManager | DialogKind::Explore) { None } else { tab.map(|t| t.id) };
        let remotes = tab
            .and_then(|t| t.snapshot.as_ref())
            .map_or_else(Vec::new, |s| {
                s.remotes.iter().map(|r| r.name.clone()).collect()
            });
        let branch = match tab
            .and_then(|t| t.snapshot.as_ref())
            .and_then(|s| s.head.as_ref())
        {
            Some(Head::Branch { name, .. }) | Some(Head::Unborn { branch: name }) => name.clone(),
            _ => vec![],
        };
        let second = if matches!(kind, DialogKind::NewRepository) {
            "main".into()
        } else if matches!(kind, DialogKind::Push) {
            String::from_utf8(branch.clone()).unwrap_or_default()
        } else {
            String::new()
        };
        let (first, flag) = if let DialogKind::EditRemote(remote, direction) = &kind {
            let values = if *direction == RemoteUrlKind::Fetch {
                &remote.urls
            } else {
                &remote.push_urls
            };
            (
                values
                    .first()
                    .and_then(|v| String::from_utf8(v.clone()).ok())
                    .unwrap_or_default(),
                *direction == RemoteUrlKind::Push && values.is_empty(),
            )
        } else if let DialogKind::TrackBranch { name, .. } = &kind {
            (name.clone(), false)
        } else {
            (String::new(), false)
        };
        let mut identity_error = None;
        let identity_task = if matches!(kind, DialogKind::RepositorySettings) {
            match (self.backend.clone(), self.tab().map(|t| t.path.clone())) {
                (Some(backend), Some(path)) => {
                    match crate::read_task::spawn(&self.context, move |cancel| backend.identity(&path, &cancel)) {
                        Ok(task) => Some(task),
                        Err(error) => {
                            identity_error = Some(user_error(&error));
                            None
                        }
                    }
                }
                _ => {
                    identity_error = Some("Open a repository first".into());
                    None
                }
            }
        } else {
            None
        };
        let issue_links = matches!(kind, DialogKind::RepositorySettings).then(|| {
            crate::issue_links::Editor::start(
                self.backend.as_ref(),
                self.tab().map(|t| t.path.as_path()),
            )
        });
        self.dialog = Some(Dialog {
            fetch_tags: self.general.fetch_tags,
            branch_space: self.general.branch_space,
            kind,
            owner,
            first,
            second,
            flag,
            remotes,
            remote: 0,
            branch,
            mode: MergeMode::FastForwardOnly,
            error: identity_error,
            identity: None,
            identity_task,
            signing: None,
            commit: None,
            merge: None,
            pull: None,
            force_push: None,
            atomic_push: None,
            ignore: None,
            workflow: None,
            rebase: None,
            mail_import: None,
            bisect: None,
            recovery: None,
            undo: None,
            tags: None,
            worktrees: None,
            submodules: None,
            lfs: None,
            github: None,
            file_edit: None,
            patch: None,
            stash_paths: None,
            selected_branches: Vec::new(),
            commands: None,
            command_values: Default::default(),
            command_context: None,
            issue_links,
            connection: None,
            statistics: None,
            gitflow: None,
            folder: None,
            create: None,
            small: Default::default(),
        });
        self.start_atomic_dialog();
        self.start_ignore_dialog();
        self.start_workflow_dialog();
        self.start_commit_preview();
        self.start_pull_preview();
        self.start_rebase_preview();
        self.start_mail_preview();
        self.start_bisect_read(None);
        self.start_recovery_read();
        self.start_undo_preview();
        self.start_signing_read();
        self.start_tag_read();
        self.start_worktree_read();
        self.start_submodule_read();
        self.start_lfs_read();
        self.start_github_read(false, false);
        self.start_forge_page();
        self.start_ssh_page();
        self.start_file_edit();
        self.start_patch_dialog();
        self.start_statistics_read();
        self.start_gitflow_read();
    }

    pub fn dispatch(&mut self, intent: Intent) {
        if self.quitting {
            return;
        }
        if self.tab().is_some_and(Tab::is_preview) && crate::preview_tab::refused(&intent) {
            self.notice = Some("This is a read-only preview. Keep it to change it.".into());
            return;
        }
        // Searching changed code reads every version of every file, which a preview
        // doesn't have; Git would otherwise fail on the first missing one.
        if matches!(intent, Intent::FindInCommits(_)) && self.tab().is_some_and(Tab::is_preview) {
            self.notice = Some("Changed code search needs every file version. Keep this preview to search it.".into());
            return;
        }
        match intent {
            Intent::GuidedSetup => {
                self.close_browsing_page();
                self.setup.open = true;
                self.setup.identity = None;
                self.setup.error = None;
                self.activity_history.open = false;
            }
            Intent::OpenReview(request) => self.open_review(request),
            Intent::CommitLink(id) => self.open_commit_link(id),
            Intent::HistoryOrder(order) => {
                if let Some(tab) = self.tabs.get_mut(self.active)
                    && tab.history_order != order {
                        tab.history_order = order;
                        tab.refresh = None;
                        tab.more = None;
                        tab.search_state = Default::default();
                        let id = tab.id;
                        self.refresh(id);
                    }
            }
            Intent::Conflict(intent) => self.conflict_intent(intent),
            Intent::Tool(intent) => self.external_intent(intent),
            Intent::File(intent) => self.file_intent(intent),
            Intent::Tree(intent) => self.tree_intent(intent),
            Intent::Map(intent) => self.map_intent(intent),
            Intent::Actions(intent) => self.actions_intent(intent),
            Intent::Compare(intent) => self.compare_intent(intent),
            Intent::BranchReview(intent) => self.branch_review_intent(intent),
            Intent::Stash(intent) => self.stash_intent(intent),
            Intent::KeepLocal { snapshot, paths } => self.open_keep_local(snapshot,paths),
            Intent::BranchWorkflow(kind) => self.open_branch_workflow(kind),
            Intent::Open(path) => self.open(path),
            Intent::Terminal => self.open_terminal(),
            Intent::CommitPush => self.open_commit_switch(None),
            Intent::SavedSwitch(branch) => self.open_commit_switch(Some(branch)),
            Intent::TerminalFor(id) => {
                if let Some(index) = self.tabs.iter().position(|t| t.id == id) {
                    self.active = index;
                    self.open_terminal();
                }
            }
            Intent::ShowFolder(path) => self.show_folder(&path),
            Intent::OpenIssueLink(url) => self.open_issue_link(&url),
            Intent::ImportPatch(source) => self.import_patch(source),
            Intent::PastePatch => {
                // The text arrives as a paste event in a later frame. A focused text
                // field or terminal would take it too, so focus is released first.
                self.context.memory_mut(|m| m.stop_text_input());
                self.paste_patch = Some(Instant::now());
                self.context
                    .send_viewport_cmd(ViewportCommand::RequestPaste);
                self.context.request_repaint();
            }
            Intent::PeekCommit(id) => {
                if let Some(tab) = self.tabs.get_mut(self.active) {
                    tab.peek = (tab.peek.as_ref() != Some(&id)).then_some(id);
                }
            }
            Intent::FoldMerge(id, collapse) => self.fold_merges(Some(id), collapse),
            Intent::CiChecks(commit, anchor) => {
                if let Some(id) = self.tab().map(|t| t.id) {
                    self.open_ci_checks(id, commit, anchor);
                }
            }
            Intent::FoldAllMerges(collapse) => self.fold_merges(None, collapse),
            Intent::ReopenTab => self.reopen_closed_tab(),
            Intent::RunContextCommand { index, context } => {
                if self.tab().is_some_and(|t| t.path == context.repository) {
                    self.show_dialog(DialogKind::RunCommand(index));
                    if let Some(d) = &mut self.dialog { d.command_context = Some(context); }
                }
            }
            Intent::Explore(query) => self.open_explore(query),
            Intent::Dialog(kind) => self.show_dialog(kind),
            Intent::Preferences(page) => self.show_preferences(page),
            Intent::Extension(id) => self.open_extension(id),
            Intent::ExtensionCommand(run) => self.run_extension_command(run),
            Intent::ExtensionFix(id, fix) => self.fix_extension(id, fix),
            Intent::InstallExtension => {
                self.show_preferences(crate::preferences::Page::Integrations);
                self.browse(crate::folder_browser::PickTarget::ExtensionFile, "");
            }
            Intent::Playground => self.open_playground(),
            Intent::MergeBranch(reference) => {
                self.show_dialog(DialogKind::Merge);
                if let Some(dialog) = &mut self.dialog {
                    dialog.first = reference;
                }
                self.start_merge_preview();
            }
            Intent::RebaseOnto(reference) => self.rebase_onto(reference),
            Intent::ExportSeries(range) => self.export_series(range),
            Intent::Drop(pending) => self.pending_drop = Some(pending),
            Intent::ArrangeCommits(arrangement) => self.arrange_commits(arrangement),
            Intent::Execute(id, action) => self.execute(id, action),
            Intent::View(view) => {
                self.setup.close();
                if view == View::BranchReview {
                    self.branch_review_intent(crate::branch_review::ReviewIntent::Open);
                    return;
                }
                if view == View::Compare {
                    self.compare_intent(crate::compare::CompareIntent::Open);
                    return;
                }
                self.activity_history.close();
                // Navigating away leaves a browsing management page.
                self.close_browsing_page();
                if let Some(tab) = self.tabs.get_mut(self.active) {
                    tab.view = view;
                    let id = tab.id;
                    self.current_diff(id);
                }
                if view == View::Tree
                    && self
                        .tab()
                        .is_some_and(|t| t.tree.listing.is_none() && !t.tree.running())
                {
                    self.tree_intent(crate::tree::TreeIntent::OpenSelected);
                }
                if view == View::Map { self.map_intent(crate::repository_map::MapIntent::Open); }
            }
            Intent::Refresh => {
                if let Some(tab) = self.tab() {
                    self.refresh(tab.id);
                }
            }
            Intent::ReloadImage => {
                if self.tab().is_some_and(|t| t.view == View::Compare) {
                    self.compare_intent(crate::compare::CompareIntent::Retry);
                    return;
                }
                if let Some(tab) = self.tabs.get_mut(self.active) {
                    // Refresh still reconciles status/refs. Remember the exact
                    // image path only within this commit, never a display label.
                    tab.image_reload = (tab.view == View::Commits)
                        .then(|| {
                            tab.selected_commit.clone().zip(
                                tab.patch
                                    .as_ref()
                                    .and_then(|p| p.files.get(tab.patch_file))
                                    .and_then(|f| f.image.as_ref())
                                    .map(|image| image.path().clone()),
                            )
                        })
                        .flatten();
                    let id = tab.id;
                    self.refresh(id);
                }
            }
            Intent::SelectCommit(commit) => {
                if let Some(tab) = self.tabs.get_mut(self.active) {
                    tab.image_reload = None;
                    tab.view = View::Commits;
                    tab.selected_commit = Some(commit.clone());
                    tab.selected_commits.clear();
                    tab.selection_anchor = Some(commit.clone());
                    let id = tab.id;
                    self.load_diff(id, DiffTarget::Commit { id: commit }, vec![]);
                }
            }
            Intent::ToggleCommit(commit) => {
                if let Some(tab) = self.tabs.get_mut(self.active) {
                    tab.selection_anchor = Some(commit.clone());
                    if tab.selected_commits.is_empty()
                        && let Some(current) = &tab.selected_commit
                        && *current != commit
                    {
                        tab.selected_commits.push(current.clone());
                    }
                    if let Some(i) = tab.selected_commits.iter().position(|c| *c == commit) {
                        tab.selected_commits.remove(i);
                        // The details pane follows the most recently chosen commit.
                        if tab.selected_commit.as_ref() == Some(&commit) {
                            tab.selected_commit = tab.selected_commits.last().cloned();
                        }
                    } else {
                        tab.selected_commits.push(commit.clone());
                        tab.selected_commit = Some(commit);
                    }
                    if tab.selected_commits.len() < 2 {
                        tab.selected_commits.clear();
                    }
                    if let Some(commit) = tab.selected_commit.clone() {
                        let id = tab.id;
                        self.load_diff(id, DiffTarget::Commit { id: commit }, vec![]);
                    }
                }
            }
            Intent::SelectCommitRange(commit) => {
                if let Some(tab) = self.tabs.get_mut(self.active) {
                    let ids: Vec<_> = tab.commits().iter().map(|c| c.id.clone()).collect();
                    let end = ids.iter().position(|c| *c == commit);
                    // The anchor stays put, so repeated Shift-clicks resize one range.
                    let listed = |s: &ObjectId| ids.iter().position(|c| c == s);
                    let start = tab
                        .selection_anchor
                        .as_ref()
                        .and_then(listed)
                        .or_else(|| tab.selected_commit.as_ref().and_then(listed))
                        .or(end);
                    if let (Some(start), Some(end)) = (start, end) {
                        let (low, high) = (start.min(end), start.max(end));
                        tab.selected_commits = ids[low..=high].to_vec();
                        if tab.selected_commits.len() < 2 {
                            tab.selected_commits.clear();
                        }
                        tab.selected_commit = Some(commit.clone());
                        let id = tab.id;
                        self.load_diff(id, DiffTarget::Commit { id: commit }, vec![]);
                    }
                }
            }
            Intent::SelectChange(entry, staged) => {
                if let Some(tab) = self.tabs.get_mut(self.active) {
                    tab.view = View::Changes;
                    tab.selected_change = Some((entry.path.clone(), staged));
                    tab.conflict_side = 0;
                    let id = tab.id;
                    tab.patch_file = 0;
                    self.current_diff(id);
                }
            }
            Intent::ConflictSide(side) => {
                if side <= 3
                    && let Some(tab) = self.tabs.get_mut(self.active)
                {
                    tab.conflict_side = side;
                    let id = tab.id;
                    self.current_diff(id);
                }
            }
            Intent::History(reference) => {
                if let Some(tab) = self.tabs.get_mut(self.active) {
                    tab.refresh = None;
                    tab.history_ref = reference;
                    tab.history_all = false;
                    tab.selected_commit = None;
                    tab.selected_commits.clear();
                    let id = tab.id;
                    self.refresh(id);
                }
            }
            Intent::AllBranches => {
                if let Some(tab) = self.tabs.get_mut(self.active) {
                    tab.refresh = None;
                    tab.history_all = true;
                    tab.history_ref = None;
                    tab.selected_commit = None;
                    tab.selected_commits.clear();
                    let id = tab.id;
                    self.refresh(id);
                }
            }
            Intent::Search => {
                if let Some(tab) = self.tabs.get_mut(self.active) {
                    tab.restart_search();
                    tab.diff = None;
                    tab.patch = None;
                    tab.selected_commit = None;
                    tab.selected_commits.clear();
                    if tab.search.trim().is_empty() {
                        tab.selected_commit = tab.commits().first().map(|c| c.id.clone());
                        let id = tab.id;
                        self.current_diff(id);
                    }
                }
            }
            Intent::FindInCommits(path) => {
                let Some(tab) = self.tabs.get_mut(self.active) else { return };
                let search = &tab.tree.search;
                let (text, regex, case) = match &search.ran {
                    Some((query, _)) => (query.text.clone(), query.regex, query.match_case),
                    None => (search.query.clone(), search.regex, search.match_case),
                };
                tab.search = text;
                tab.search_kind = HistorySearchKind::Content;
                tab.search_regex = regex;
                tab.search_case = case;
                tab.search_path = String::from_utf8_lossy(&path).into_owned();
                self.dispatch(Intent::View(View::Commits));
                self.dispatch(Intent::Search);
            }
            Intent::StopSearch => {
                if let Some(tab) = self.tabs.get_mut(self.active) {
                    tab.search_state.stop();
                }
            }
            Intent::More => {
                if let Some(tab) = self.tabs.get_mut(self.active)
                    && !tab.search.trim().is_empty()
                {
                    tab.search_state.more();
                    return;
                }
                if let (Some(backend), Some(tab)) =
                    (self.backend.clone(), self.tabs.get_mut(self.active))
                {
                    if tab.more.is_some() || tab.refresh.is_some() {
                        return;
                    }
                    if let Some(cursor) = tab
                        .snapshot
                        .as_ref()
                        .and_then(|s| s.history.as_ref())
                        .and_then(|h| h.next.clone())
                    {
                        let repository = tab.snapshot.as_ref().unwrap().repository.clone();
                        match crate::read_task::spawn(&self.context, move |cancel| {
                            backend.history(&repository, HistoryStart::Continue(cursor), &cancel)
                        }) {
                            Ok(task) => tab.more = Some(task),
                            Err(error) => tab.error = Some(user_error(&error)),
                        }
                    }
                }
            }
            Intent::CloseTab(id) => {
                if self.write.as_ref().is_some_and(|w| w.owner == Some(id)) {
                    self.notice = Some(
                        "Wait for this repository's operation to finish before closing it.".into(),
                    );
                    return;
                }
                if self
                    .tabs
                    .iter()
                    .any(|t| t.id == id && (!t.draft.trim().is_empty() || t.conflict.dirty))
                {
                    self.show_dialog(DialogKind::CloseTab(id));
                } else {
                    self.remove_tab(id);
                }
            }
            Intent::Exit => self.show_dialog(DialogKind::Quit),
        }
    }

    pub fn remove_tab(&mut self, id: u64) {
        if self.external.owner == Some(id) {
            self.external.cancel();
        }
        if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == id)
            && let Some(mut terminal) = tab.terminal.take()
        {
            terminal.stop();
            self.closing_terminals.push(terminal);
        }
        // Closing a repository retains its draft for the next open.
        if let Some(tab) = self.tabs.iter().find(|t| t.id == id)
            && !tab.is_preview()
        {
            if tab.draft_loaded {
                self.saved.worktree_views.insert(tab.path.clone(), crate::worktree_view::Settings::capture(tab));
                // An unopened or failed tab has not restored its saved draft yet.
                if !tab.draft.is_empty() {
                    self.saved.set_draft(&tab.path, &tab.draft, &self.written);
                } else {
                    self.saved.drafts.remove(&tab.path);
                }
            }
            self.closed_tabs.retain(|p| *p != tab.path);
            self.closed_tabs.push(tab.path.clone());
            if self.closed_tabs.len() > CLOSED_TABS {
                self.closed_tabs.remove(0);
            }
        }
        let active_id = self.tab().map(|t| t.id);
        if let Some(preview) = self.tabs.iter_mut().find(|t| t.id == id).and_then(|t| t.preview.take()) {
            if self.keeping_preview == Some(id) {
                self.keeping_preview = None;
            }
            self.note_discard(&preview);
            self.release_preview(preview);
        }
        self.tabs.retain(|t| t.id != id);
        self.active = self
            .tabs
            .iter()
            .position(|t| Some(t.id) == active_id)
            .unwrap_or_else(|| self.active.min(self.tabs.len().saturating_sub(1)));
    }
    /// Reopen the most recently closed repository that is not open again (Ctrl+Shift+T).
    pub(crate) fn reopen_closed_tab(&mut self) {
        while let Some(path) = self.closed_tabs.pop() {
            if !self.tabs.iter().any(|t| t.path == path) {
                self.open(path);
                return;
            }
        }
        self.notice = Some("No closed repository to reopen.".into());
    }
    /// Window-level drops and pastes. A dropped folder opens as a repository; a
    /// dropped file, or patch text pasted from the clipboard, opens the reviewed
    /// patch import for the active repository, one at a time. Runs every frame,
    /// so an unanswered paste request always expires.
    pub(crate) fn dropped_or_pasted(&mut self, ctx: &Context, intents: &mut Vec<Intent>) {
        let pasted = self.pasted_patch(ctx);
        if self.blocking_dialog() || self.quitting {
            return;
        }
        let mut patch = None;
        for file in ctx.input(|i| i.raw.dropped_files.clone()) {
            let path = file.path().to_path_buf();
            if path.is_file() {
                patch.get_or_insert(path);
            } else {
                intents.push(Intent::Open(path));
            }
        }
        if let Some(path) = patch {
            intents.push(Intent::ImportPatch(crate::patches::PatchSource::File(path)));
        }
        if let Some(text) = pasted {
            intents.push(Intent::ImportPatch(crate::patches::PatchSource::Text(text)));
        }
    }
    /// Patch text from the clipboard: Ctrl+V in the main window while no text field,
    /// terminal or dialog has it, or the text Apply Patch from Clipboard asked for.
    /// Text that does not look like a patch only gets a notice.
    fn pasted_patch(&mut self, ctx: &Context) -> Option<String> {
        let requested = self
            .paste_patch
            .is_some_and(|t| t.elapsed() < Duration::from_secs(2));
        if self.paste_patch.is_some() && !requested {
            // An empty clipboard sends no paste event.
            self.paste_patch = None;
            self.notice = Some("The clipboard holds no text to apply as a patch.".into());
        } else if requested {
            ctx.request_repaint_after(Duration::from_millis(50));
        }
        let text = ctx.input(|i| {
            i.events.iter().find_map(|e| match e {
                Event::Paste(text) => Some(text.clone()),
                _ => None,
            })
        })?;
        let typing = ctx
            .memory(|m| m.focused())
            .is_some_and(|id| TextEdit::load_state(ctx, id).is_some());
        let blocked = self.blocking_dialog() || self.quitting || self.tab().is_none();
        if blocked
            || (!requested && (typing || self.dialog.is_some() || self.terminal_screen_focused()))
        {
            if requested {
                self.paste_patch = None;
            }
            return None;
        }
        self.paste_patch = None;
        if crate::patches::looks_like_patch(&text) {
            Some(text)
        } else {
            self.notice = Some(
                "The clipboard text is not a patch. Copy a Git diff, then paste it again.".into(),
            );
            None
        }
    }
    /// Collapse or expand one merge commit, or all of them, in the history list. A
    /// selected commit that is no longer shown hands the selection to the merge
    /// that now hides it.
    fn fold_merges(&mut self, merge: Option<ObjectId>, collapse: bool) {
        let Some(tab) = self.tabs.get_mut(self.active) else {
            return;
        };
        match &merge {
            Some(id) => tab.folding.set(id, collapse),
            None => tab.folding.set_all(collapse),
        }
        tab.relayout();
        let hidden_in = &tab.hidden_in;
        tab.selected_commits
            .retain(|id| !hidden_in.contains_key(id));
        if tab.selected_commits.len() < 2 {
            tab.selected_commits.clear();
        }
        let owner = tab
            .selected_commit
            .as_ref()
            .and_then(|id| tab.hidden_in.get(id))
            .cloned();
        if let Some(owner) = owner {
            self.dispatch(Intent::SelectCommit(owner));
        }
    }
    /// Open an issue link from a commit message in the default browser, after
    /// checking it again. The isolated MCP host and tests record it instead.
    pub(crate) fn open_issue_link(&mut self, url: &str) {
        self.open_reviewed_url(url, "issue-link-open");
    }
    pub(crate) fn open_reviewed_url(&mut self, url: &str, operation: &'static str) {
        if !rocket_git::https_url(url) {
            self.notice = Some("Rocket opens only plain HTTPS links.".into());
            return;
        }
        let opened = &mut self.opened_urls;
        let mut open = || match opened {
            Some(list) => {
                list.push(url.to_owned());
                Ok(())
            }
            None => crate::hosting::launch_browser(url),
        };
        let result = match &self.backend {
            Some(backend) => backend.log.request(operation, |_| open()),
            None => open(),
        };
        if let Err(error) = result {
            self.notice = Some(format!(
                "Could not open the browser link: {}",
                user_error(&error)
            ));
        }
    }
    /// Show a folder in File Explorer. The isolated MCP host and tests record it instead.
    pub(crate) fn show_folder(&mut self, path: &std::path::Path) {
        let folder = crate::views::display_path(path);
        if let Some(opened) = &mut self.opened_urls {
            opened.push(format!("explorer:{folder}"));
            return;
        }
        if let Err(e) = launch_explorer(&folder) {
            self.notice = Some(format!("Could not open File Explorer: {e}"));
        }
    }
    /// Shared by the native close event and localhost MCP. Programmatic close
    /// must not bypass unsaved drafts or reconciliation of an active write.
    pub fn request_close(&mut self) {
        if self.quitting {
            return;
        }
        if self.external.running()
            || self.integrations.running()
            || self.integrations.exit_pending()
            || self.forge.running()
            || self.ci.rerun.as_ref().is_some_and(|r| r.sending())
            || self.forge.pull.has_draft()
            || self.ssh.running()
            || self.write.is_some()
            || self.manager.status.batch.is_some()
            || self
                .tabs
                .iter()
                .any(|t| !t.draft.trim().is_empty() || t.conflict.dirty)
        {
            self.show_dialog(DialogKind::Quit);
            // A taskbar close can arrive while minimized. Make the confirmation
            // reachable instead of leaving the user waiting on an invisible dialog.
            self.context.send_viewport_cmd(ViewportCommand::Minimized(false));
        } else {
            self.begin_quit();
        }
        self.context.request_repaint_of(ViewportId::ROOT);
    }
    pub fn begin_quit(&mut self) {
        if let Err(error)=self.updates.approve_restart(){self.updates.error=Some(user_error(&error));self.updates.open=true;return;}
        self.quitting = true;
        // Clipboard delivery/expiry is handled by the drawn UI. It must not keep
        // shutdown busy when only logic runs (for example after minimizing).
        self.paste_patch = None;
        self.context.request_repaint_of(ViewportId::ROOT);
        self.setup.close();
        if let Some(task) = self.dialog.as_ref().and_then(|d| d.undo.as_ref()).and_then(|s| s.task.as_ref()) { task.cancel(); }
        self.spelling.enable(false);
        self.pending_review=None;
        self.commit_link=None;
        self.updates.shutdown();
        for window in &self.reviews {window.lock().unwrap_or_else(|e|e.into_inner()).close();}
        self.manager.cancel();
        self.benchmark.cancel();
        self.manager.cancel_form();
        self.forge.cancel();
        self.integrations.prepare_exit();
        self.ssh.cancel();
        self.dependencies.cancel();
        self.cancel_dependency_push_review();
        self.activity_history.close();
        self.crash_reports.shutdown();
        self.external.cancel();
        self.cancel_write();
        if let Some(task) = self
            .dialog
            .as_ref()
            .and_then(|d| d.bisect.as_ref())
            .and_then(|s| s.task.as_ref())
        {
            task.cancel();
        }
        if let Some(task) = self
            .dialog
            .as_ref()
            .and_then(|d| d.merge.as_ref())
            .and_then(|m| m.task.as_ref())
        {
            task.cancel();
        }
        self.cancel_gitflow_reads();
        if let Some(dialog) = &self.dialog {
            if let Some(task) = dialog.atomic_push.as_ref().and_then(|s| s.task.as_ref()) {
                task.cancel();
            }
            if let Some(state) = &dialog.ignore {
                if let Some(task) = &state.read { task.cancel(); }
                if let Some(task) = &state.task { task.cancel(); }
            }
            if let Some(task)=dialog.workflow.as_ref().and_then(|s|s.task.as_ref()){task.cancel();}
        }
        for tab in &mut self.tabs {
            tab.cancel_reads();
            tab.composer.draft.cancel();
            tab.refresh_pending = None;
            if let Some(terminal) = &mut tab.terminal {
                terminal.stop();
            }
        }
        if let Some(startup) = &self.startup {
            startup.cancel();
        }
        self.recent_heads.clear();
    }

    pub fn poll(&mut self, ctx: &Context) {
        self.crash_reports.poll();
        self.general.publish(ctx);
        self.poll_guided(ctx);
        self.poll_git_settings(ctx);
        self.poll_manager(ctx);
        self.poll_external(ctx);
        self.mcp.poll(ctx);
        let repository_context = self.tab().map(|tab| (tab.path.clone(), tab.selected_commit.clone()));
        self.integrations.repository_selection(repository_context);
        self.integrations.poll(ctx, self.backend.clone(), self.quitting);
        self.integrations.publish_commands(ctx, self.backend.is_some() && !self.interaction_busy());
        if self.quitting && let Some(error) = self.integrations.draft_save_error() {
            self.quitting = false;
            self.notice = Some(format!("Rocket stayed open because an extension draft could not be saved: {error}"));
            // begin_quit closes content pages. Return to the retained draft so
            // the user can copy it or retry the failed save without hunting for it.
            self.dispatch(Intent::Dialog(DialogKind::Integrations));
        }
        self.poll_commit_preview();
        self.poll_merge_preview();
        if let Some(d) = &mut self.dialog {
            d.small.poll(ctx);
        }
        self.poll_pull_preview();
        self.poll_force_push_preview();
        self.poll_atomic_preview();
        self.poll_ignore();
        self.poll_workflow();
        self.poll_rebase_preview();
        self.poll_mail_preview();
        self.poll_bisect_read();
        self.poll_recovery_read();
        self.poll_signing_read();
        self.poll_tag_read();
        self.poll_worktree_read();
        self.poll_submodule_read();
        self.poll_lfs_read();
        self.poll_github();
        self.poll_explore(ctx);
        self.poll_previews(ctx);
        self.poll_forge();
        self.poll_ci(ctx);
        self.poll_actions(ctx);
        self.poll_hosting_drafts();
        self.poll_ssh();
        self.poll_composer();
        self.poll_auto_fetch();
        self.poll_file_edit();
        self.poll_patch_dialog();
        self.poll_statistics();
        self.poll_benchmark(ctx);
        self.poll_gitflow();
        // Spell checking follows its preference; answers come from its own thread.
        self.spelling.enable(self.spell_check && !self.quitting);
        if self.spelling.poll() {
            ctx.request_repaint();
        } else if self.spelling.waiting() {
            ctx.request_repaint_after(std::time::Duration::from_millis(30));
        }
        for tab in &mut self.tabs {
            tab.diff_tools.poll();
            tab.image.poll(&self.context);
            tab.issue_links.poll();
        }
        if let Some(dialog) = &mut self.dialog
            && let Some(settings) = dialog
                .issue_links
                .as_mut()
                .and_then(crate::issue_links::Editor::poll)
            && let Some(tab) = self.tabs.iter_mut().find(|t| Some(t.id) == dialog.owner)
        {
            // The commit details use what the editor just read.
            tab.issue_links.set(&settings);
        }
        if let Some(test) = self.dialog.as_mut().and_then(|d| d.connection.as_mut()) {
            test.poll();
        }
        if let Some(dialog) = &mut self.dialog
            && let Some(result) = dialog.identity_task.as_mut().and_then(ReadTask::try_finish)
        {
            dialog.identity_task = None;
            match result {
                Ok(identity) => {
                    dialog.first = identity.local.name.clone().unwrap_or_default();
                    dialog.second = identity.local.email.clone().unwrap_or_default();
                    dialog.identity = Some(identity);
                }
                Err(error) => dialog.error = Some(user_error(&error)),
            }
        }
        self.poll_terminals();
        if let Some(result) = self.startup.as_mut().and_then(ReadTask::try_finish) {
            self.startup = None;
            match result {
                Ok(startup) => {
                    match startup.state {
                        Ok((store, saved)) => {
                            let p = &saved.preferences;
                            self.show_sidebar = p.sidebar;
                            self.show_details = p.details;
                            self.show_console = p.console;
                            self.auto_refresh = p.auto_refresh;
                            self.secret_protection = p.secret_protection;
                            self.guard_ai_api_keys = p.guard_ai_api_keys;
                            self.dependencies.enabled = p.dependency_warnings;
                            self.dependencies.before_push = p.dependency_before_push;
                            self.previous_auto_refresh = p.auto_refresh;
                            self.row_height = p.row_height;
                            self.commit_height = p.commit_height;
                            self.language = p.language;
                            self.theme = p.theme.clone();
                            self.sidebar_labels = p.sidebar_labels;
                            self.repository_list = p.repository_list;
                            self.spell_check = p.spell_check;
                            self.general = p.general.clone();
                            self.terminal_shell.load(p.shell.clone());
                            self.drafting = p.drafting.clone();
                            self.keybindings.bindings = p.shortcuts.clone();
                            self.auto_fetch.minutes = p.auto_fetch_minutes;
                            self.commands = saved.commands.clone();
                            self.library = saved.library.clone();
                            ctx.set_zoom_factor(p.zoom);
                            self.recent = saved.recent.clone();
                    self.forge.searches = saved.hosting_searches.clone();
                            self.external.profiles = saved.tool_profiles.clone().unwrap_or_else(||
                                crate::tool_profiles::Profiles::from_legacy(&saved.tools));
                            self.external.reset_edits();
                            self.written = saved.clone();
                            self.saved = saved;
                            self.state_store = Some(store);
                        }
                        Err(error) => {
                            self.state_error = Some(format!(
                                "Drafts/settings could not be loaded or saved: {}. Existing state is preserved.",
                                user_error(&error)
                            ))
                        }
                    }
                    startup.backend.set_secret_protection(self.secret_protection);
                    startup.backend.set_guard_ai_api_keys(self.guard_ai_api_keys);
                    self.backend = Some(startup.backend);
                    if !self.quitting {
                        for path in std::mem::take(&mut self.pending_paths) {
                            self.open(path);
                        }
                    }
                }
                Err(error) => self.startup_error = Some(user_error(&error)),
            }
        }
        self.poll_dependency_security(ctx);
        self.poll_app_updates(ctx);
        self.poll_recent_heads();
        let mut refresh_diffs = Vec::new();
        let mut restored_history = Vec::new();
        let mut canonical_tabs: std::collections::HashMap<_, _> = self
            .tabs
            .iter()
            .filter(|t| t.draft_loaded)
            .map(|t| (t.path.clone(), t.id))
            .collect();
        let mut duplicates = Vec::new();
        for tab in &mut self.tabs {
            if let Some(result) = tab.refresh.as_mut().and_then(ReadTask::try_finish) {
                tab.refresh = None;
                match result {
                    Ok(update) => {
                        let (mut snapshot, worktree_only) = match update {
                            Refresh::Full(snapshot) => (*snapshot, false),
                            Refresh::Worktree(update) => {
                                if !tab.snapshot.as_ref().is_some_and(|snapshot| update.matches(snapshot)) {
                                    tab.refresh_pending = Some(Instant::now());
                                    if let Some(snapshot) = &mut tab.snapshot { snapshot.refresh_guard = None; }
                                    continue;
                                }
                                let mut snapshot = tab.snapshot.take().unwrap();
                                update.apply(&mut snapshot);
                                (snapshot, true)
                            }
                        };
                        if let Some(path) = snapshot.info.worktree.as_ref() {
                            tab.path = path.clone();
                        } else {
                            tab.path = snapshot.info.git_dir.clone();
                        }
                        if let Some(&existing) = canonical_tabs.get(&tab.path)
                            && existing != tab.id
                        {
                            duplicates.push((tab.id, existing));
                            continue; // Opening another spelling must not create competing drafts.
                        }
                        canonical_tabs.insert(tab.path.clone(), tab.id);
                        if !tab.draft_loaded {
                            if let Some(view) = self.saved.worktree_views.get(&tab.path).cloned() {
                                if view.history_all != tab.history_all || view.history_ref != tab.history_ref || view.history_order != tab.history_order {
                                    restored_history.push(tab.id);
                                }
                                view.restore(tab);
                            }
                            if tab.draft.is_empty() {
                                tab.draft = self
                                    .saved
                                    .drafts
                                    .get(&tab.path)
                                    .map(|text| text.to_string())
                                    .unwrap_or_default();
                            }
                            tab.composer.recent = self
                                .saved
                                .recent_messages
                                .get(&tab.path)
                                .cloned()
                                .unwrap_or_default();
                            tab.draft_loaded = true;
                        }
                        if !tab.is_preview() && !self.recent.contains(&tab.path) {
                            self.recent.insert(0, tab.path.clone());
                            self.recent.truncate(12);
                        }
                        let background = tab.background_refresh;
                        let previous_selected = tab.selected_commit.clone();
                        let mut history_retained = worktree_only;
                        if !worktree_only && background
                            && let Some(old) = &mut tab.snapshot
                            && snapshot
                                .history
                                .as_ref()
                                .zip(old.history.as_ref())
                                .is_some_and(|(new, old)| new.same_snapshot(old))
                        {
                            snapshot.history = old.history.take(); // Keep already-loaded pages.
                            history_retained = true;
                        }
                        let commits = snapshot
                            .history
                            .as_ref()
                            .map_or(&[][..], |h| h.commits.as_slice());
                        if !commits
                            .iter()
                            .any(|c| Some(&c.id) == tab.selected_commit.as_ref())
                        {
                            tab.selected_commit = commits.first().map(|c| c.id.clone());
                        }
                        // A sequence selection keeps only commits still listed.
                        tab.selected_commits
                            .retain(|id| commits.iter().any(|c| c.id == *id));
                        if tab.selected_commits.len() < 2 {
                            tab.selected_commits.clear();
                        }
                        if let Some((path, staged)) = &tab.selected_change
                            && !snapshot.status.as_ref().is_some_and(|s| {
                                s.entries.iter().any(|e| {
                                    &e.path == path
                                        && (if *staged {
                                            e.index != Change::Unchanged
                                        } else {
                                            e.worktree != Change::Unchanged
                                                || e.kind == EntryKind::Untracked
                                        })
                                })
                            })
                        {
                            tab.selected_change = None;
                        }
                        if !history_retained {
                            tab.more = None; // A late page belongs to the old set of tips.
                            if !tab.search.trim().is_empty() {
                                tab.restart_search();
                                tab.selected_commit = None;
                            }
                        } else if !tab.search.trim().is_empty() {
                            tab.selected_commit = previous_selected.clone();
                        }
                        tab.snapshot = Some(snapshot);
                        if !history_retained {
                            tab.relayout();
                        }
                        tab.refresh_count += 1;
                        if let (Some(backend), Some(s)) = (&self.backend, &tab.snapshot) {
                            tab.issue_links
                                .reload(backend, s, !background, &self.context);
                        }
                        if !background
                            || !worktree_only
                            || tab.view == View::Changes
                            || tab.selected_commit != previous_selected
                        {
                            // The diff on screen stays while it reloads only if it still
                            // belongs to the selection; a new commit's diff starts empty.
                            let keep = background
                                && (tab.view == View::Changes
                                    || tab.selected_commit == previous_selected);
                            refresh_diffs.push((tab.id, keep));
                        }
                    }
                    Err(error) => tab.error = Some(user_error(&error)),
                }
            }
            if let Some(result) = tab.more.as_mut().and_then(ReadTask::try_finish) {
                tab.more = None;
                match result {
                    Ok(page) => {
                        if let Some(history) =
                            tab.snapshot.as_mut().and_then(|s| s.history.as_mut())
                        {
                            history.commits.extend(page.commits);
                            history.next = page.next;
                        }
                        tab.relayout();
                    }
                    Err(error) => tab.error = Some(user_error(&error)),
                }
            }
            if let Some(result) = tab.diff.as_mut().and_then(ReadTask::try_finish) {
                tab.diff = None;
                let image_reload = tab.image_reload.take();
                match result {
                    Ok(patch) => {
                        if let Some((commit, path)) = image_reload
                            && tab.view == View::Commits
                            && tab.selected_commit.as_ref() == Some(&commit)
                            && let Some(index) = patch.files.iter().position(|file| {
                                file.image
                                    .as_ref()
                                    .is_some_and(|image| image.path() == &path)
                            })
                        {
                            tab.patch_file = index;
                        }
                        tab.patch = Some(patch);
                    }
                    Err(error) => tab.patch_error = Some(user_error(&error)),
                }
            }
        }
        let mut active_id = self.tab().map(|t| t.id);
        for (duplicate, existing) in duplicates {
            if active_id == Some(duplicate) {
                active_id = Some(existing);
            }
            self.tabs.retain(|t| t.id != duplicate);
        }
        self.active = self
            .tabs
            .iter()
            .position(|t| Some(t.id) == active_id)
            .unwrap_or(0);
        if !self.quitting {
            for id in restored_history { self.refresh(id); }
            for (id, keep) in refresh_diffs {
                let old = self
                    .tabs
                    .iter_mut()
                    .find(|t| t.id == id)
                    .map(|t| (t.patch.take(), t.patch_file));
                self.current_diff(id);
                if keep
                    && let Some(tab) = self.tabs.iter_mut().find(|t| t.id == id)
                    && tab.diff.is_some()
                    && let Some((patch, file)) = old
                {
                    tab.patch = patch;
                    tab.patch_file = file;
                }
            }
        }
        if let Some(write) = &mut self.write {
            while let Ok(phase) = write.force_push_progress.try_recv() {
                write.force_push_phase = Some(phase);
                if !write.cancelling || write.checking_remote() {
                    write.progress = match phase {
                        ForcePushProgress::CheckingLease => "Checking the reviewed lease…",
                        ForcePushProgress::Pushing if write.label == "atomic-push" => "Publishing the selected branches atomically…",
                        ForcePushProgress::Pushing => "Pushing the reviewed commit…",
                        ForcePushProgress::CheckingRemote {
                            push_cancelled: true,
                        } => "Push stopped—checking the remote result…",
                        ForcePushProgress::CheckingRemote {
                            push_cancelled: false,
                        } => "Checking the remote result…",
                    }
                    .into();
                    write.fraction = None;
                }
            }
            let transfer = |write: &mut ActiveWrite, progress: FetchProgress| {
                if progress.phase == rocket_git::FetchPhase::CheckingRepository {
                    write.progress = "Checking repository state…".into();
                    write.fraction = None;
                    return;
                }
                write.progress = format!(
                    "{}: {}{} objects",
                    progress.phase.label(),
                    progress.current,
                    progress
                        .total
                        .map_or_else(String::new, |t| format!(" / {t}"))
                );
                write.fraction = progress
                    .total
                    .filter(|t| *t > 0)
                    .map(|t| (progress.current as f32 / t as f32).clamp(0.0, 1.0));
            };
            while let Ok(progress) = write.fetch_progress.try_recv() {
                if !write.cancelling {
                    transfer(write, progress);
                }
            }
            while let Ok(progress) = write.clone_progress.try_recv() {
                if write.cancelling {
                    continue;
                }
                match progress {
                    CloneProgress::Phase(phase) => {
                        write.progress = match phase {
                            ClonePhase::Preparing => "Preparing…",
                            ClonePhase::Cloning => "Cloning…",
                            ClonePhase::Verifying => "Verifying…",
                            ClonePhase::Publishing => "Publishing…",
                            ClonePhase::Opening => "Opening…",
                            ClonePhase::DownloadingFiles => "Downloading files…",
                        }
                        .into();
                        write.fraction = None;
                    }
                    CloneProgress::Files { done, total } => {
                        write.progress = format!("Downloading files: {} of {}", crate::explore::group(done), crate::explore::group(total));
                        write.fraction = (total > 0).then(|| (done as f32 / total as f32).clamp(0.0, 1.0));
                    }
                    CloneProgress::Transfer(progress) => transfer(write, progress),
                    CloneProgress::Checkout { current, total } => {
                        write.progress = format!(
                            "Checkout: {current}{} files",
                            total.map_or_else(String::new, |t| format!(" / {t}"))
                        );
                        write.fraction = total
                            .filter(|t| *t > 0)
                            .map(|t| (current as f32 / t as f32).clamp(0.0, 1.0));
                    }
                }
            }
        }
        if let Some(result) = self.write.as_mut().and_then(|w| w.task.try_finish()) {
            let write = self.write.take().expect("completed write");
            let errored = result.is_err();
            let mut outcome = result.unwrap_or_else(|error| {
                let mut raw = String::new();
                if let Error::Git { stdout, stderr, .. } = &error {
                    raw = format!(
                        "{}{}",
                        String::from_utf8_lossy(stdout),
                        String::from_utf8_lossy(stderr)
                    );
                }
                Outcome {
                    summary: match &error {
                        Error::SecretProtection(failure) => failure.summary(),
                        _ => format!("{} did not complete", crate::operation_names::done(&write.label)),
                    },
                    details: vec![user_error(&error)],
                    raw_output: raw,
                    ..Default::default()
                }
            });
            let quiet = outcome.quiet;
            if !quiet {
                self.notice = Some(outcome.summary.clone());
            }
            if let Some(tab) = self.tabs.iter_mut().find(|t| Some(t.id) == write.owner) {
                if outcome.command_succeeded { tab.undo_commit = outcome.undo_commit.clone(); }
                else if write.label != "undo-commit" { tab.undo_commit = None; }
                tab.submodule_update |= self.general.submodules_on_checkout
                    && outcome.command_succeeded
                    && matches!(write.label.as_str(), "branch-switch" | "branch-checkout-remote" | "saved-switch");
            }
            self.finish_undo(write.owner, &outcome);
            self.finish_commit_dialog(write.owner, &outcome);
            self.finish_merge_dialog(write.owner, &outcome);
            self.finish_pull_dialog(write.owner, &mut outcome);
            self.finish_force_push_dialog(write.owner, &outcome);
            self.finish_atomic_dialog(write.owner, &outcome);
            self.finish_ignore_dialog(write.owner, &outcome);
            self.finish_workflow_dialog(write.owner, &outcome);
            self.finish_rebase_dialog(write.owner, &outcome);
            self.finish_mail_dialog(write.owner, &outcome);
            self.finish_bisect_dialog(write.owner, &outcome);
            self.finish_recovery_dialog(write.owner, &outcome);
            self.finish_signing_dialog(write.owner, &outcome);
            self.finish_tag_dialog(write.owner, &outcome);
            self.finish_worktree_dialog(write.owner, &outcome);
            self.finish_submodule_dialog(write.owner, &outcome);
            self.finish_lfs_dialog(write.owner, &outcome);
            self.finish_file_edit(write.owner, &outcome);
            self.finish_patch_dialog(write.owner, &outcome);
            self.finish_gitflow_dialog(write.owner, &outcome);
            if write.label == "conflict-save" {
                self.finish_conflict_write(write.owner, &outcome);
            }
            if write.label == "conflict-keep" {
                self.finish_conflict_keep(write.owner, &outcome);
            }
            if !outcome.details.is_empty() || !outcome.command_succeeded {
                self.show_console = true;
            }
            // Never retain its watcher in activity/last-outcome storage when the
            // tab closed or the app is quitting instead of accepting this read.
            let completed_refresh = outcome.network_refresh.take();
            if let Some(id) = write.owner {
                if (outcome.command_succeeded || outcome.local_commit_created)
                    && let Some(tab) = self.tabs.iter_mut().find(|t| t.id == id)
                    && let Some(message) = &write.submitted_draft
                {
                    tab.composer.committed(Some(message));
                    if *message == tab.draft {
                        tab.draft.clear();
                    }
                }
                if !self.quitting && !quiet {
                    if matches!(write.label.as_str(), "fetch" | "pull-fetch" | "push") {
                        // Reconcile every section when metadata changed, but keep
                        // navigation responsive and unchanged derived rules intact.
                        self.refresh_after_network(id, true, completed_refresh);
                    } else {
                        self.refresh(id);
                    }
                }
            }
            if write.label == "keep-preview" {
                // A Keep that errored says why; its summary only says that it failed.
                let reason = match outcome.details.first() {
                    Some(detail) if errored => detail.clone(),
                    _ => outcome.summary.clone(),
                };
                self.finish_keep(outcome.command_succeeded, &reason);
            }
            if !self.quitting
                && let Some(path) = outcome.open_repository.clone()
            {
                self.open(path);
            }
            // With the setting on, the page this session asked for waits for the update.
            let reviewed = if self.general.submodules_on_checkout {
                write.label == "submodule-checkout"
            } else {
                matches!(write.label.as_str(), "branch-switch" | "branch-checkout-remote")
            };
            if !self.quitting && outcome.command_succeeded && reviewed && self.review_submodules_after_switch && self.tab().map(|t|t.id)==write.owner && self.dialog.is_none() {
                self.show_dialog(DialogKind::Submodules);
            }
            if outcome.raw_output.chars().count() > 32_000 {
                outcome.raw_output = outcome.raw_output.chars().take(32_000).collect::<String>()
                    + "\n[Local display limited to 32,000 characters]";
            }
            if !quiet {
                self.activities.push(Activity {
                    title: write.label,
                    outcome,
                    elapsed: write.started.elapsed(),
                    at: std::time::SystemTime::now(),
                    seen: false,
                });
                self.activity_history.mark_dirty();
                if self.activities.len() > 40 {
                    self.activities.remove(0);
                }
            }
        }
        self.poll_state(ctx);
        self.poll_refresh(ctx);
        self.poll_submodule_update();
        self.poll_history_search(ctx);
        self.poll_file_history(ctx);
        self.poll_tree(ctx);
        self.poll_repository_map(ctx);
        self.poll_compare(ctx);
        self.poll_branch_reviews(ctx);
        self.poll_stashes(ctx);
        self.poll_conflicts(ctx);
        if self.busy() {
            ctx.request_repaint_after(Duration::from_millis(50));
        }
        if self.quitting && !self.busy() {
            self.release_all_previews();
            ctx.send_viewport_cmd(ViewportCommand::Close);
        }
    }

    /// The welcome screen shows each recent repository's branch. Its reads run only
    /// while no repository is open and stop when one opens or Rocket quits.
    fn poll_recent_heads(&mut self) {
        if !self.tabs.is_empty() || self.quitting {
            self.recent_heads.clear();
            return;
        }
        let Some(backend) = &self.backend else {
            return;
        };
        self.recent_heads.update(&self.recent, |path| {
            let backend = backend.clone();
            let path = path.to_path_buf();
            crate::read_task::spawn(&self.context, move |cancel| {
                backend.head(&path, &cancel)
            })
        });
    }

    fn poll_refresh(&mut self, ctx: &Context) {
        if self.quitting {
            return;
        }
        let now = Instant::now();
        let enabled_now = self.auto_refresh && !self.previous_auto_refresh;
        self.previous_auto_refresh = self.auto_refresh;
        for tab in &mut self.tabs {
            if !self.auto_refresh {
                tab.refresh_pending = None;
                if let Some(s) = &mut tab.snapshot {
                    s.watcher = None;
                }
                continue;
            }
            if enabled_now {
                tab.refresh_pending = Some(now);
                tab.last_change = now;
            }
            if let Some(watch) = tab.snapshot.as_ref().and_then(|s| s.watcher.as_ref()) {
                match watch.take_changed() {
                    Ok(true) => {
                        tab.refresh_pending.get_or_insert(now);
                        tab.last_change = now;
                    }
                    Ok(false) => {}
                    Err(error) => {
                        tab.error = Some(format!(
                            "Automatic refresh stopped: {}. Use F5.",
                            user_error(&error)
                        ));
                        if let Some(s) = &mut tab.snapshot {
                            s.watcher = None;
                        }
                    }
                }
            }
        }
        if self.write.is_none()
            && self.dialog.is_none()
            && let Some(tab) = self.tab()
            && let Some(since) = tab.refresh_pending
            && tab.refresh.is_none()
            && tab.diff.is_none()
            && tab.more.is_none()
        {
            if now.duration_since(tab.last_change) >= Duration::from_millis(350)
                || now.duration_since(since) >= Duration::from_secs(2)
            {
                self.refresh_inner(tab.id, true);
            } else {
                ctx.request_repaint_after(Duration::from_millis(350));
            }
        }
    }

    fn poll_state(&mut self, ctx: &Context) {
        if let Some(result) = self.save_task.as_mut().and_then(WriteTask::try_finish) {
            self.save_task = None;
            match result {
                Ok(saved) => self.written = saved,
                Err(error) => {
                    self.state_error = Some(format!(
                        "Drafts/settings could not be saved: {}. Keep Rocket open or copy your draft before exiting.",
                        user_error(&error)
                    ));
                    self.save_dirty_since = None;
                    self.quitting = false;
                }
            }
        }
        if self.state_store.is_none() || self.state_error.is_some() {
            return;
        }
        let preferences = Preferences {
            sidebar: self.show_sidebar,
            details: self.show_details,
            console: self.show_console,
            auto_refresh: self.auto_refresh,
            secret_protection: self.secret_protection,
            guard_ai_api_keys: self.guard_ai_api_keys,
            dependency_warnings: self.dependencies.enabled,
            dependency_before_push: self.dependencies.before_push,
            row_height: self.row_height,
            zoom: ctx.zoom_factor(),
            sidebar_labels: self.sidebar_labels,
            auto_fetch_minutes: self.auto_fetch.minutes,
            repository_list: self.repository_list,
            spell_check: self.spell_check,
            drafting: self.drafting.clone(),
            shortcuts: self.keybindings.bindings.clone(),
            theme: self.theme.clone(),
            general: self.general.clone(),
            shell: self.terminal_shell.saved.clone(),
            commit_height: self.commit_height,
            language: self.language,
        };
        let mut changed = self.saved.preferences != preferences
            || self.saved.recent != self.recent
            || self.saved.tool_profiles.as_ref() != Some(&self.external.profiles)
            || self.saved.commands != self.commands
            || self.saved.library != self.library
            || self.saved.hosting_searches != self.forge.searches;
        self.saved.hosting_searches = self.forge.searches.clone();
        if self.saved.library != self.library { self.saved.library = self.library.clone(); }
        if self.saved.commands != self.commands {
            self.saved.commands = self.commands.clone();
        }
        if self.saved.tool_profiles.as_ref() != Some(&self.external.profiles) {
            self.saved.tools = self.external.profiles.defaults();
            self.saved.tool_profiles = Some(self.external.profiles.clone());
        }
        self.saved.preferences = preferences;
        self.saved.recent = self.recent.clone();
        for tab in &self.tabs {
            if !tab.draft_loaded || tab.is_preview() {
                continue;
            }
            let view = crate::worktree_view::Settings::capture(tab);
            if self.saved.worktree_views.get(&tab.path) != Some(&view) {
                self.saved.worktree_views.insert(tab.path.clone(), view);
                changed = true;
            }
            changed |= self.saved.set_draft(&tab.path, &tab.draft, &self.written);
            if tab.composer.recent.is_empty() {
                changed |= self.saved.recent_messages.remove(&tab.path).is_some();
            } else if self.saved.recent_messages.get(&tab.path) != Some(&tab.composer.recent) {
                self.saved
                    .recent_messages
                    .insert(tab.path.clone(), tab.composer.recent.clone());
                changed = true;
            }
        }
        changed |= self.saved.retain_worktree_views(
            self.tabs.get(self.active).into_iter().map(|tab| &tab.path)
                .chain(self.tabs.iter().rev().map(|tab| &tab.path))
                .chain(self.recent.iter()),
        );
        // Keep recent messages only for open and recently opened repositories.
        if self.saved.recent_messages.len() > crate::persistence::MESSAGE_REPOSITORIES {
            self.saved.recent_messages.retain(|path, _| {
                self.recent.contains(path) || self.tabs.iter().any(|t| &t.path == path)
            });
            while self.saved.recent_messages.len() > crate::persistence::MESSAGE_REPOSITORIES {
                self.saved.recent_messages.pop_first();
            }
            changed = true;
        }
        let now = Instant::now();
        if changed {
            self.save_last_change = now;
        }
        if self.saved != self.written {
            self.save_dirty_since.get_or_insert(now);
        } else if self.save_task.is_none() {
            self.save_dirty_since = None;
        }
        if let Some(since) = self.save_dirty_since
            && self.save_task.is_none()
        {
            if self.quitting
                || now.duration_since(self.save_last_change) >= Duration::from_millis(400)
                || now.duration_since(since) >= Duration::from_secs(2)
            {
                let state = self.saved.clone();
                let store = self.state_store.clone().unwrap();
                let log = self.backend.as_ref().unwrap().log.clone();
                match WriteTask::spawn(move |_| {
                    log.request("save_state", |_| {
                        store.save(&state).map_err(|source| Error::Io {
                            action: "save drafts and settings",
                            source,
                        })?;
                        Ok(state)
                    })
                }) {
                    Ok(task) => self.save_task = Some(task),
                    Err(error) => {
                        self.state_error = Some(format!(
                            "Drafts/settings could not be saved: {}. Keep Rocket open or copy your draft before exiting.",
                            user_error(&error)
                        ));
                        self.save_dirty_since = None;
                        self.quitting = false;
                    }
                }
            } else {
                ctx.request_repaint_after(Duration::from_millis(400));
            }
        }
    }

    /// Shows the chosen theme: again whenever it, Windows' light or dark mode or the
    /// Custom picture's colour changes.
    fn update_theme(&mut self, ctx: &Context) {
        self.poll_theme_import();
        let accent = self.backdrop.update(ctx, self.theme.custom(), &self.state_directory);
        let look = crate::theme::resolve(&self.theme, ctx.system_theme(), accent);
        if self.look != Some(look) {
            style::apply(ctx, &look.palette, look.light);
            self.look = Some(look);
        }
    }

    /// Copies a chosen picture into Rocket's storage on a worker; the Custom theme
    /// shows it once it is kept.
    pub(crate) fn import_theme_picture(&mut self, path: &std::path::Path) {
        let (sender, receiver) = std::sync::mpsc::channel();
        let (source, storage, ctx) = (path.to_path_buf(), self.state_directory.clone(), self.context.clone());
        self.theme_import_error = None;
        let spawned = std::thread::Builder::new().name("theme-import".into()).spawn(move || {
            let _ = sender.send(crate::theme::picture::import(&source, &storage));
            ctx.request_repaint();
        });
        match spawned {
            Ok(_) => self.theme_import = Some(receiver),
            Err(e) => self.theme_import_error = Some(format!("The picture could not be copied: {e}.")),
        }
    }

    fn poll_theme_import(&mut self) {
        let Some(receiver) = &self.theme_import else { return };
        let result = match receiver.try_recv() {
            Ok(result) => result,
            Err(std::sync::mpsc::TryRecvError::Empty) => return,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => Err("The picture could not be copied.".into()),
        };
        self.theme_import = None;
        match result {
            Ok(name) => {
                self.theme.choice = crate::theme::Choice::Custom;
                self.theme.custom.picture = crate::theme::Picture::Own(name);
            }
            Err(message) => self.theme_import_error = Some(message),
        }
    }
}

impl eframe::App for Rocket {
    fn raw_input_hook(&mut self, ctx: &Context, input: &mut RawInput) {
        self.shortcut_raw_input(ctx, input);
    }
    fn logic(&mut self, ctx: &Context, _frame: &mut eframe::Frame) {
        let was_quitting = self.quitting;
        self.poll(ctx);
        if was_quitting && !self.quitting {
            // A failed save keeps Rocket open; show its recovery/error UI even
            // when the close was requested from the taskbar.
            ctx.send_viewport_cmd(ViewportCommand::Minimized(false));
        }
        // eframe skips ui() for hidden/minimized windows, including native close
        // events. Poll first so this frame's saves/errors remain an exit barrier.
        if ctx.input(|i| i.viewport().close_requested()) {
            if !was_quitting {
                ctx.send_viewport_cmd(ViewportCommand::CancelClose);
                self.request_close();
            } else if !self.quitting || self.busy() {
                // A failed save may just have aborted shutdown. Another X in
                // this frame must not silently restart it or bypass that error.
                ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            }
        }
        if self.quitting {
            self.activity_history.close();
        }
        let directory = self.backend.as_ref().map_or_else(
            || self.log_path.clone(),
            |b| b.log.directory().to_path_buf(),
        );
        self.activity_history.poll(directory, ctx);
        self.update_theme(ctx);
        self.update_language(ctx);
    }
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        ctx.data_mut(|data| data.insert_temp(Id::new("rocket-shortcut-hints"), self.keybindings.bindings.clone()));
        self.frame_widgets.clear();
        self.apply_text_command(&ctx);
        // F1 opens the guide; while it is open it takes Esc and Ctrl+Enter.
        self.guide_input(&ctx);
        crate::change_drag::cancel_on_navigation(&ctx, self.tab(),
            self.can_write() && !self.blocking_dialog() && self.guide.is_none() && !self.quitting);
        let mut intents = Vec::new();
        // Browsing pages keep shortcuts and file drops; reviews and modals own input.
        self.dropped_or_pasted(&ctx, &mut intents);
        if !self.blocking_dialog() && !self.quitting && self.guide.is_none() {
            self.shortcuts(&ctx, &mut intents);
        }
        // Full-bleed window: panes meet the window edge, separated only by hairlines.
        let bounds = ui.max_rect();
        ui.painter().rect_filled(bounds, 0.0, style::BG());
        // A Custom theme's picture sits under the panes, which let part of it through.
        if let Some(custom) = self.theme.custom() {
            self.backdrop.paint(ui.painter(), bounds, custom);
        }
        ui.scope_builder(UiBuilder::new().max_rect(bounds), |ui| {
            self.shell(ui, &mut intents)
        });
        if !self.blocking_dialog() && !self.quitting && self.guide.is_none() {
            self.command_palette(&ctx, &mut intents);
        } else {
            self.palette.open = false;
        }
        for intent in intents {
            self.dispatch(intent);
        }
        self.dialog_ui(&ctx);
        self.extension_review_dialog(&ctx);
        // Crash reports open their Browse picker, which must draw above them.
        self.crash_reports_ui(&ctx);
        // A Browse picker sits above Preferences; the tool dialog shows only alone.
        self.picker_dialog(&ctx);
        self.tool_launch_dialog(&ctx);
        self.guide_ui(&ctx);
        self.commit_link_ui(&ctx);
        self.ci_ui(&ctx);
        self.app_updates_ui(&ctx);
        self.dependency_push_review_ui(&ctx);
        if let Some((path, _)) = &self.pending_review
            && let Some(index) = self.tabs.iter().position(|t| &t.path == path && t.snapshot.is_some()) {
                let (_, request) = self.pending_review.take().expect("pending review");
                self.open_review_for(index, request);
            }
        self.review_windows(&ctx);
        // Capture this frame's edits before shutdown or an idle frame. I/O stays
        // on workers; unchanged strings are not cloned or serialized each frame.
        self.poll_state(&ctx);
    }
}

/// Open `folder` in File Explorer.
#[cfg(windows)]
fn launch_explorer(folder: &str) -> std::io::Result<()> {
    rocket_git::ensure_unprivileged_process().map_err(std::io::Error::other)?;
    use std::os::windows::process::CommandExt;
    let explorer = std::env::var_os("SystemRoot")
        .map(|root| PathBuf::from(root).join("explorer.exe"))
        .ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::NotFound, "SystemRoot is not set")
        })?;
    // Explorer splits an unquoted path at commas, so the folder is always quoted;
    // Windows paths cannot contain quotes.
    std::process::Command::new(explorer)
        .raw_arg(format!("\"{folder}\""))
        .spawn()
        .map(drop)
}
#[cfg(not(windows))]
fn launch_explorer(_folder: &str) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "File Explorer is only available on Windows",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_common as common;
    use std::{
        cell::Cell,
        sync::atomic::{AtomicBool, Ordering},
        thread,
    };

    fn finish_reads(
        heads: &mut RecentHeads,
        recent: &[PathBuf],
        spawn: &mut impl FnMut(&Path) -> Result<ReadTask<Head>>,
    ) {
        let deadline = Instant::now() + Duration::from_secs(10);
        heads.update(recent, &mut *spawn);
        while heads.running() {
            assert!(
                Instant::now() < deadline,
                "recent HEAD reads did not finish"
            );
            thread::sleep(Duration::from_millis(5));
            heads.update(recent, &mut *spawn);
        }
    }

    #[test]
    fn recent_heads_keep_listed_results_and_cancel_dropped_reads() {
        let target = ObjectId::parse(&"1".repeat(40)).unwrap();
        let spawned = Cell::new(0);
        let mut spawn = |path: &Path| {
            spawned.set(spawned.get() + 1);
            let (path, target) = (path.to_path_buf(), target.clone());
            ReadTask::spawn(move |_| match path.to_str() {
                Some("main") => Ok(Head::Branch {
                    name: b"main".to_vec(),
                    target,
                }),
                Some("detached") => Ok(Head::Detached { target }),
                Some("unborn") => Ok(Head::Unborn {
                    branch: b"trunk".to_vec(),
                }),
                _ => Err(Error::InvalidInput("not a repository".into())),
            })
        };
        let [main, detached, unborn, gone] =
            ["main", "detached", "unborn", "gone"].map(PathBuf::from);
        let recent = vec![main.clone(), detached.clone(), unborn.clone(), gone.clone()];
        let mut heads = RecentHeads::default();
        finish_reads(&mut heads, &recent, &mut spawn);
        assert!(
            matches!(heads.get(&main), Some(Some(Head::Branch { name, .. })) if name == b"main")
        );
        assert!(matches!(
            heads.get(&detached),
            Some(Some(Head::Detached { .. }))
        ));
        assert_eq!(
            heads.get(&unborn),
            Some(Some(&Head::Unborn {
                branch: b"trunk".to_vec()
            }))
        );
        assert_eq!(
            heads.get(&gone),
            Some(None),
            "unreadable paths are Unavailable"
        );
        // Results are kept while a path stays listed; nothing is read twice.
        heads.update(&recent, &mut spawn);
        assert!(!heads.running());
        assert_eq!(spawned.get(), 4);
        // A path that leaves the list loses its result.
        heads.update(&recent[..2], &mut spawn);
        assert_eq!(heads.get(&unborn), None);
        assert_eq!(heads.get(&gone), None);
        assert!(heads.get(&main).is_some());
        // Clearing (a repository opened, or Rocket quits) drops a read in flight,
        // which cancels it, and forgets every result. Wait until the read runs: a
        // read cancelled before its worker starts is discarded without being called.
        let started = Arc::new(AtomicBool::new(false));
        let cancelled = Arc::new(AtomicBool::new(false));
        let (running, observed) = (started.clone(), cancelled.clone());
        heads.update(&[main.clone(), PathBuf::from("slow")], |_| {
            let (running, observed) = (running.clone(), observed.clone());
            ReadTask::spawn(move |cancel| {
                running.store(true, Ordering::SeqCst);
                loop {
                    if let Err(error) = cancel.check() {
                        observed.store(true, Ordering::SeqCst);
                        return Err(error);
                    }
                    thread::sleep(Duration::from_millis(5));
                }
            })
        });
        assert!(heads.running());
        let deadline = Instant::now() + Duration::from_secs(5);
        while !started.load(Ordering::SeqCst) {
            assert!(Instant::now() < deadline, "the slow read did not start");
            thread::sleep(Duration::from_millis(5));
        }
        heads.clear();
        assert!(!heads.running());
        assert_eq!(heads.get(&main), None);
        let deadline = Instant::now() + Duration::from_secs(5);
        while !cancelled.load(Ordering::SeqCst) {
            assert!(
                Instant::now() < deadline,
                "the dropped read was not cancelled"
            );
            thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn welcome_reads_recent_heads_until_a_repository_opens() {
        common::isolated(
            "ROCKET_RECENT_HEADS_CASE",
            "app::tests::welcome_reads_recent_heads_until_a_repository_opens",
            || {
                let born = common::Fixture::new();
                born.write("README.md", "recent\n");
                born.commit("first");
                let unborn = common::Fixture::new();
                let missing = born.root.join("missing");
                let ctx = Context::default();
                let mut app = Rocket::with_log_directory(&ctx, vec![], born.root.join("logs"));
                let settle = |app: &mut Rocket| {
                    let deadline = Instant::now() + Duration::from_secs(40);
                    app.poll(&ctx);
                    while app.busy() {
                        assert!(Instant::now() < deadline, "Rocket did not settle");
                        thread::sleep(Duration::from_millis(10));
                        app.poll(&ctx);
                    }
                };
                settle(&mut app);
                assert!(app.backend.is_some(), "{:?}", app.startup_error);
                app.recent = vec![
                    born.repository.clone(),
                    unborn.repository.clone(),
                    missing.clone(),
                ];
                settle(&mut app);
                assert!(matches!(
                    app.recent_heads.get(&born.repository),
                    Some(Some(Head::Branch { name, .. })) if name == b"main"
                ));
                assert_eq!(
                    app.recent_heads.get(&unborn.repository),
                    Some(Some(&Head::Unborn {
                        branch: b"main".to_vec()
                    }))
                );
                assert_eq!(app.recent_heads.get(&missing), Some(None));
                // Opening a repository drops the welcome screen's reads and results.
                app.open(born.repository.clone());
                app.poll(&ctx);
                assert!(!app.recent_heads.running());
                assert_eq!(app.recent_heads.get(&born.repository), None);
                settle(&mut app);
                assert!(!app.tabs.is_empty());
                assert_eq!(app.recent_heads.get(&unborn.repository), None);
            },
        );
    }
}
