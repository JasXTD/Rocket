//! Repository reads, local writes, and explicit fetch/push through bundled or installed Git.
//!
//! Methods are blocking: use [`ReadTask`] for reads and [`WriteTask`] for writes,
//! or a UI worker. Never run Git operations on the UI thread.
//! Paths and commit messages retain Git's bytes; lossy conversion is for display only.

mod auto_fetch;
mod process_privilege;
/// Refuse elevated/high-integrity Windows hosts, including an unverifiable token.
/// Call before reading startup state or preparing any child process. Normal UAC-
/// filtered users are supported; other platforms retain their existing behavior.
pub fn ensure_unprivileged_process() -> Result<()> {
    process_privilege::check().map_err(|error| Error::InvalidInput(error.to_string()))
}
pub mod app_update;
mod advisory_tool;
mod managed_git;
pub use managed_git::{MANAGED_GIT_VERSION, MANAGED_GIT_MANIFEST_SHA256, ManagedGitProgress, ManagedGitRuntime, managed_git_path, verify_managed_git};
pub use advisory_tool::{OSV_VERSION, OsvOutput, run_offline_osv, osv_file_digest, sha256_bytes, verify_osv_scanner};
mod bisect;
mod branch;
mod branch_merge;
pub use branch_merge::BranchMergeState;
mod branch_fast_forward;
pub use branch_fast_forward::{FastForwardBranch, FastForwardPreview, FastForwardReport};
mod branch_workflow;
pub use branch_workflow::{BranchWorkflowKind, BranchWorkflowOptions, BranchWorkflowPreview, BranchWorkflowReport};
mod clone;
mod clone_failure;
mod commit_actions;
mod commit_draft;
mod commit_message;
mod commit_prediction;
mod comparison;
mod branch_review;
pub use branch_review::{BranchReviewFailure, BranchReviewPreview};
mod conflict;
mod custom_commands;
mod diagnostics;
mod display_text;
mod error;
mod external_tools;
mod failure_text;
pub use failure_text::{FailureText, failure_text};
mod fetch_failure;
mod file_actions;
mod file_history;
mod force_push;
mod forge;
mod gitflow;
mod history_search;
mod hosting;
mod integration_transport;
mod integration_worker;
pub use integration_worker::IntegrationWorker;
pub use integration_transport::{IntegrationConnection, IntegrationHttpResponse, integration_credential_digest};
mod identity;
mod global_identity;
pub use global_identity::GlobalIdentity;
mod ignore;
pub use ignore::*;
mod keep_local;
pub use keep_local::{IgnoreTrackedPreview, IgnoreTrackedReport, StashSnapshotPreview, StashSnapshotReport};
mod image_compare;
mod submodule_diff;
pub use submodule_diff::{SubmoduleDiff, SubmoduleDiffCommit};
mod init;
mod issue_links;
mod lfs;
mod local_files;
mod local_time;
pub use local_time::{LocalTime, local_time};
mod merge;
mod merge_review;
mod model;
mod commit_links;
pub use commit_links::CommitBrowserLink;
mod operation;
mod parse;
mod patches;
mod process_tree;
mod progress;
mod pull;
mod pull_failure;
mod pull_review;
mod push;
mod push_check;
pub use push_check::PushCheck;
mod push_outlook;
pub use push_outlook::{PUSH_OUTLOOK_COMMITS, PushOutlook};
mod push_failure;
mod rebase;
mod rebase_plan;
mod rebase_merges;
mod recovery;
mod undo;
pub use undo::{UndoCommit, UndoCommitPreview, UndoCommitReport, UndoableCommitReport, failure_code as undo_failure_code};
mod remote;
pub use remote::FetchTags;
mod remote_check;
mod remote_cleanup;
pub use remote_cleanup::{ObsoleteTrackingRef, RemoteBranchDeletePreview, RemoteCleanupPreview, RemoteCleanupReport};
mod remote_management;
pub use remote_management::{RemoteRenamePreview, RemoteRenameReport};
mod repository;
mod review;
mod runner;
mod secret_guard;
pub use secret_guard::{SecretKind, SecretProtectionFailure};
mod sequence;
mod statistics;
mod signing;
mod ssh_keys;
mod stash;
mod stash_apply;
pub use stash_apply::{StashApplyFile, StashApplyReview, StashApplySelection, StashApplyPreview};
mod stash_controls;
pub use stash_controls::{StashControl, StashControlPreview, StashControlReport, StashSelectionPreview, StashSelectionReport};
mod stash_rename;
pub use stash_rename::StashRenamePreview;
mod saved_work;
mod commit_push;
pub use commit_push::{CommitPushPreview, CommitPushReport};
pub use saved_work::{SavedWorkAction,SavedWorkPreview,SavedWorkReport};
mod submodules;
mod tags;
mod task;
mod tree;
mod transport_scope;
pub use transport_scope::TransportScope;
mod watch;
mod worktrees;
mod worktree_status;
pub use worktree_status::WorktreeStatus;
mod write;
mod write_gate;

pub use ssh_keys::{SshBindingReview, SshDiscoveredKey, SshDiscovery, SshFailure, SshInventory, SshKeyGuard, SshManager, SshPrepared, SshProfile};
pub use auto_fetch::QuietFetchReport;
pub use bisect::{
    BisectAction, BisectDecision, BisectFailure, BisectPhase, BisectPreview, BisectSession,
    BisectWriteReport,
    BisectTestCommand, BisectAutoPreview, BisectAutoProgress, BisectTestResult,
    BisectAutoReport, BisectTestOutcome,
};
pub use clone::{CloneDestination, check_clone_destination};
pub use clone_failure::CloneFailure;
pub use commit_actions::{
    CommitActionRequest, CommitFailure, CommitOperation, CommitPreview, CommitRecovery,
    CommitSequence, CommitWriteReport,
};
pub use commit_draft::{
    CONFLICT_PROPOSAL_TIMEOUT, CommitDraft, DRAFT_DIFF_LIMIT, DRAFT_LANGUAGE_LIMIT,
    DRAFT_READ_LIMIT, DRAFT_TIMEOUT, DraftFailure, DraftProvider, DraftRequest, DraftScope,
    DraftStyle, claude_executable, default_claude_path, default_codex_path,
};
pub use commit_message::CommitTemplate;
pub use commit_prediction::{CommitPrediction, PredictionKind, PredictionReason};
pub use comparison::{ComparedFile, Comparison};
pub use conflict::{
    CONFLICT_FILE_LIMIT, CONFLICT_TEXT_LIMIT, ConflictBlock, ConflictBlocks, ConflictFailure,
    ConflictKeep, ConflictKind, ConflictPreview, ConflictSaveReport, ConflictVersion, WholeConflictPreview,
    WholeConflictReport, WholeFileChange, conflict_blocks,
};
pub use custom_commands::{
    COMMAND_PLACEHOLDERS, CUSTOM_OUTPUT_LIMIT, CommandAction, CommandContext, CommandInput, CommandInputKind, CommandScope, CustomCommand, CustomCommandReport,
    MAX_CUSTOM_COMMANDS, CUSTOM_ARGUMENT_LIMIT, custom_command_display, valid_custom_program_path,
};
pub use diagnostics::{DiagnosticLog, InputValidationStep, LogHealth, McpRequestMetrics};
pub use error::{Error, Result};
pub use external_tools::*;
pub use fetch_failure::FetchFailure;
pub use file_actions::*;
pub use file_history::*;
pub use force_push::*;
pub use forge::*;
pub use gitflow::{
    GITFLOW_KEYS, GITFLOW_MESSAGE_LIMIT, GITFLOW_NAME_LIMIT, GitFlowBranch, GitFlowConfig,
    GitFlowDevelop, GitFlowFailure, GitFlowFinishOptions, GitFlowFinishPreview,
    GitFlowFinishReport, GitFlowInitPreview, GitFlowInitReport, GitFlowKeep, GitFlowKind,
    GitFlowNote, GitFlowStartPreview, GitFlowStartReport, GitFlowState, GitFlowStep,
    GitFlowStepReport,
};
pub use hosting::{
    GITHUB_NOTIFICATION_PAGE, GITHUB_REPOSITORY_PAGE, GitHubAccount, GitHubEndpoint, GitHubList,
    GitHubNotification, GitHubPullRequest, GitHubRemote, GitHubRepository, GitHubSession,
    HostingFailure, github_web_url,
};
pub use identity::{IdentityWriteReport, LocalIdentity, RepositoryIdentity};
pub use image_compare::{IMAGE_FILE_LIMIT, ImageComparison, ImageVersions, image_path};
pub use init::{INIT_ENTRY_LIMIT, InitDestination, InitReport, check_init_destination};
pub use issue_links::{
    ISSUE_LINK_TEXT_LIMIT, IssueLink, IssueLinkRule, IssueLinkScope, IssueLinkSettings,
    IssueLinkWriteReport, IssueLinks, MAX_ISSUE_LINK_RULES, https_url,
};
pub use lfs::{
    LFS_FILE_LIMIT, LfsAction, LfsAttributeEdit, LfsEntry, LfsFailure, LfsFileState, LfsLock,
    LfsLockAction, LfsLockFailure, LfsLockOutcome, LfsLockPreview, LfsLockReport, LfsLocks,
    LfsPointer, LfsPreview, LfsPrunePreview, LfsPruneReport, LfsPushFailure, LfsSourceScope, LfsStatus, LfsTrackingRule,
    LfsUploadOutcome, LfsWriteReport, is_lfs_lock_fixture_url,
};
pub use local_files::drive_letters;
pub use merge_review::{MergeFailure, MergePreview, MergeRelation};
pub use model::*;
pub use patches::*;
pub use progress::{ClonePhase, CloneProgress, FetchPhase, FetchProgress};
pub use pull_failure::PullFailure;
pub use pull_review::*;
pub use push_failure::PushFailure;
pub use rebase::{
    DependentBranch, RebaseAction, RebaseFailure, RebasePreview, RebaseStop, RebaseWriteReport,
};
pub use rebase_plan::{MAX_REBASE_COMMAND, RebaseInstruction, RebasePlanEntry, run_rebase_editor};
pub use recovery::{
    LostCommitProgress, LostCommitScan, MAX_LOST_COMMITS, RecoveryAction, RecoveryDetails,
    RecoveryFailure, RecoveryPreview, RecoveryWriteReport, ReflogEntry, ReflogPage, ReflogScope,
    ResetMode,
};
pub use remote_check::{REMOTE_CHECK_LIMIT, RemoteCheck};
pub use repository::{ConfigurationSnapshot, GitClient, Repository};
pub use review::{ChangeReview, FilePreview};
pub use runner::{CancellationToken, CommandLimits};
pub use sequence::MAX_SEQUENCE as MAX_COMMIT_SEQUENCE;
pub use signing::*;
pub use statistics::{
    ACTIVITY_COMMITS, ActivityScope, ActivityTreemap, AuthorActivity, CommitActivity,
    TREEMAP_COMMITS, TreemapNode,
};
pub use submodules::{
    SubmoduleBatchPreview, SubmoduleBatchReport,
    SubmoduleAction, SubmoduleActionPreview, SubmoduleActionReport, SubmoduleEntry,
    SubmoduleFailure, SubmodulePreview, SubmoduleWriteReport,
};
pub use tags::{
    MAX_TAG_BATCH, RemoteTagAction, RemoteTagBatchReport, RemoteTagCommit, RemoteTagEntry, RemoteTagPreview,
    RemoteTagSnapshot, RemoteTagWriteReport, TagAction, TagBatchReport, TagBatchReview, TagDetails,
    TagAnnotation, TagEntry, TagFailure, TagOrder, TagPage, TagPreview, TagSummary, TagWriteReport,
};
pub use task::{ReadTask, WriteTask};
pub use tree::{
    TREE_ENTRY_LIMIT, TREE_FILE_LIMIT, TreeDirectory, TreeEntry, TreeEntryKind, TreeListing,
};
pub use watch::RepositoryWatch;
pub use write::MAX_MESSAGE_BYTES as MAX_COMMIT_MESSAGE_BYTES;
pub use worktrees::{
    WorktreeCheckout, WorktreeEntry, WorktreeFailure, WorktreeMaintenance, WorktreePreview,
    WorktreeWriteReport,
};
