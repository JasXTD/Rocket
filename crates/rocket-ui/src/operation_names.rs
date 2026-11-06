//! Operations in words, for the status bar, the Activity drawer and its history.
//! `Action::label()` keys stay what diagnostics, automation and saved history record;
//! this module only names them for people.
use crate::backend::Action;

/// A key, what it shows while it runs, and its title once it has run.
const NAMES: &[(&str, &str, &str)] = &[
    ("benchmark", "Benchmarking repository", "Repository benchmark"),
    ("benchmark-prepare", "Preparing benchmark", "Prepare benchmark"),
    ("benchmark-refresh", "Measuring refresh", "Benchmark refresh"),
    ("benchmark-history", "Measuring history", "Benchmark history"),
    ("benchmark-diff", "Measuring commit diff", "Benchmark commit diff"),
    ("review-window", "Loading pinned review", "Open review window"),
    ("review-close", "Closing review", "Close review window"),
    ("commit-link", "Reading commit links", "Read commit links"),
    ("commit-link-open", "Opening commit link", "Open commit link"),
    ("series-preview", "Reviewing commit series", "Review commit series"),
    ("series-import", "Importing commit series", "Import commit series"),
    ("series-continue", "Continuing import", "Continue import"),
    ("series-skip", "Skipping patch", "Skip patch"),
    ("series-abort", "Restoring original checkout", "Abort import"),
    ("github-publish-preview", "Reviewing GitHub publication", "Review GitHub publication"),
    ("github-publish", "Publishing to GitHub", "Publish to GitHub"),
    ("stage", "Staging", "Stage"),
    ("unstage", "Unstaging", "Unstage"),
    ("stage-selection", "Staging selection", "Stage selection"),
    ("unstage-selection", "Unstaging selection", "Unstage selection"),
    ("discard-selection", "Discarding selection", "Discard selection"),
    ("mark-resolved", "Marking resolved", "Mark resolved"),
    ("conflict-save", "Saving resolution", "Save resolution"),
    ("conflict-keep", "Keeping one version", "Keep one version"),
    ("commit", "Committing", "Commit"),
    ("undo-commit", "Undoing commit", "Undo commit"),
    ("amend", "Amending commit", "Amend commit"),
    ("cherry-pick", "Cherry-picking", "Cherry-pick"),
    ("cherry-pick-continue", "Continuing cherry-pick", "Continue cherry-pick"),
    ("cherry-pick-skip", "Skipping commit", "Skip commit"),
    ("cherry-pick-abort", "Stopping cherry-pick", "Abort cherry-pick"),
    ("revert", "Reverting", "Revert"),
    ("revert-continue", "Continuing revert", "Continue revert"),
    ("revert-skip", "Skipping commit", "Skip commit"),
    ("revert-abort", "Stopping revert", "Abort revert"),
    ("reset-soft", "Resetting (soft)", "Reset (soft)"),
    ("reset-mixed", "Resetting (mixed)", "Reset (mixed)"),
    ("reset-hard", "Resetting (hard)", "Reset (hard)"),
    ("recover-branch", "Recovering branch", "Recover branch"),
    ("rebase", "Rebasing", "Rebase"),
    ("interactive-rebase", "Rebasing", "Interactive rebase"),
    ("rebase-continue", "Continuing rebase", "Continue rebase"),
    ("rebase-skip", "Skipping commit", "Skip commit"),
    ("rebase-abort", "Stopping rebase", "Abort rebase"),
    ("bisect-start", "Starting bisect", "Start bisect"),
    ("bisect-auto", "Running bisect tests", "Automated bisect"),
    ("bisect-good", "Marking good", "Mark good"),
    ("bisect-bad", "Marking bad", "Mark bad"),
    ("bisect-skip", "Skipping commit", "Skip commit"),
    ("bisect-resume", "Resuming bisect", "Resume bisect"),
    ("bisect-restore", "Ending bisect", "End bisect"),
    ("tag-create", "Creating tag", "Create tag"),
    ("tag-delete", "Deleting tag", "Delete tag"),
    ("tag-delete-batch", "Deleting tags", "Delete tags"),
    ("remote-tag-publish", "Publishing tag", "Publish tag"),
    ("remote-tag-publish-batch", "Publishing tags", "Publish tags"),
    ("remote-tag-delete", "Deleting remote tag", "Delete remote tag"),
    ("worktree-create", "Creating worktree", "Create worktree"),
    ("worktree-remove", "Removing worktree", "Remove worktree"),
    ("worktree-move", "Moving worktree", "Move worktree"),
    ("worktree-repair", "Repairing worktrees", "Repair worktrees"),
    ("worktree-prune", "Pruning worktrees", "Prune worktrees"),
    ("submodule-update", "Updating submodule", "Update submodule"),
    ("submodule-init", "Initializing submodule", "Initialize submodule"),
    ("submodule-add", "Adding submodule", "Add submodule"),
    ("submodule-remove", "Removing submodule", "Remove submodule"),
    ("submodule-deinitialize", "Deinitializing submodule", "Deinitialize submodule"),
    ("submodule-sync", "Syncing submodule URL", "Sync submodule URL"),
    ("submodule-set-url", "Updating submodule URL", "Update submodule URL"),
    ("submodule-move", "Moving submodule", "Move submodule"),
    ("submodule-batch", "Updating submodules", "Update submodules"),
    ("lfs-fetch", "Fetching LFS objects", "Fetch LFS objects"),
    ("lfs-push", "Pushing LFS objects", "Push LFS objects"),
    ("lfs-checkout", "Checking out LFS files", "Check out LFS files"),
    ("lfs-track", "Tracking with LFS", "Track with LFS"),
    ("lfs-untrack", "Untracking from LFS", "Untrack from LFS"),
    ("lfs-lock", "Locking file", "Lock file"),
    ("lfs-unlock", "Unlocking file", "Unlock file"),
    ("lfs-force-unlock", "Unlocking file", "Force unlock"),
    ("lfs-prune", "Cleaning the LFS cache", "Clean LFS cache"),
    ("file-discard", "Discarding changes", "Discard changes"),
    ("file-restore", "Restoring file", "Restore file"),
    ("file-recover", "Recovering file", "Recover file"),
    ("patch-import", "Applying patch", "Apply patch"),
    ("patch-export", "Saving patch", "Save patch"),
    ("patch-series-export", "Exporting commit series", "Export commit series"),
    ("signing-save", "Saving signing settings", "Save signing settings"),
    ("identity-save", "Saving identity", "Save identity"),
    ("issue-links-save", "Saving issue links", "Save issue links"),
    ("branch-create", "Creating branch", "Create branch"),
    ("branch-switch", "Switching branch", "Switch branch"),
    ("branch-checkout-remote", "Checking out branch", "Check out branch"),
    ("submodule-checkout", "Updating submodules", "Update submodules"),
    ("saved-switch", "Saving, switching and restoring", "Save, switch and restore"),
    ("commit-push", "Committing and pushing", "Commit & Push"),
    ("branch-rename", "Renaming branch", "Rename branch"),
    ("branch-delete", "Deleting branch", "Delete branch"),
    ("branch-delete-batch", "Deleting branches", "Delete branches"),
    ("branch-sync", "Syncing branch", "Sync branch"),
    ("branch-finish", "Finishing branch", "Finish branch"),
    ("branch-fast-forward", "Fast-forwarding branches", "Fast-forward branches"),
    ("custom-command", "Running command", "Custom command"),
    ("remote-add", "Adding remote", "Add remote"),
    ("remote-edit", "Changing remote URL", "Edit remote URL"),
    ("remote-remove", "Removing remote", "Remove remote"),
    ("remote-rename", "Renaming remote", "Rename remote"),
    ("remote-prune", "Removing tracking refs", "Remove tracking refs"),
    ("remote-branch-delete", "Deleting remote branch", "Delete remote branch"),
    ("fetch", "Fetching", "Fetch"),
    ("pull", "Pulling", "Pull"),
    ("pull-fetch", "Fetching for pull", "Fetch for pull"),
    ("pull-apply", "Pulling", "Pull"),
    ("push", "Pushing", "Push"),
    ("force-push", "Force pushing", "Force push"),
    ("atomic-push", "Pushing branches together", "Push branches together"),
    ("ignore-save", "Saving .gitignore", "Save .gitignore"),
    ("ignore-tracked", "Ignoring tracked files", "Ignore tracked files"),
    ("stash-push", "Saving stash", "Save stash"),
    ("stash-apply", "Applying stash", "Apply stash"),
    ("stash-drop", "Dropping stash", "Drop stash"),
    ("stash-rename", "Renaming stash", "Rename stash"),
    ("stash-control", "Updating stashes", "Update stashes"),
    ("stash-selection", "Stashing selection", "Stash selection"),
    ("stash-apply-selection", "Applying saved selection", "Apply saved selection"),
    ("stash-apply-preview", "Reviewing saved selection", "Review saved selection"),
    ("stash-snapshot", "Saving snapshot", "Save snapshot"),
    ("saved-work", "Saving work", "Save work"),
    ("merge", "Merging", "Merge"),
    ("merge-continue", "Finishing merge", "Continue merge"),
    ("merge-abort", "Stopping merge", "Abort merge"),
    ("merge-finish", "Clearing merge state", "Clear merge state"),
    ("gitflow-init", "Setting up Git Flow", "Set up Git Flow"),
    ("gitflow-start", "Starting Git Flow branch", "Start Git Flow branch"),
    ("gitflow-finish", "Finishing Git Flow branch", "Finish Git Flow branch"),
    ("clone", "Cloning repository", "Clone repository"),
    ("keep-preview", "Keeping preview", "Keep preview"),
    ("discard-preview", "Discarding preview", "Discard preview"),
    ("init", "Creating repository", "Create repository"),
];

/// Operations that only read, by the name diagnostics show. Keys ending in `-preview`,
/// `-read` or `-policy` read too, and are worded from the key unless named here. A key
/// that is in neither counts as an action, so diagnostics never hide a write.
const READS: &[(&str, &str)] = &[
    ("status", "Read status"),
    ("github-search", "Search GitHub"),
    ("diff", "Read diff"),
    ("diff-words", "Read diff words"),
    ("history", "Read history"),
    ("directory-history", "Read folder history"),
    ("tags", "Read tags"),
    ("tag-inspect", "Read tag"),
    ("tag-preview", "Review tag"),
    ("remote-tags", "Read remote tags"),
    ("remote-tag-preview", "Review remote tag"),
    ("remote-tag-policy", "MCP input check"),
    ("force-push-policy", "MCP input check"),
    ("atomic-push-policy", "MCP input check"),
    ("pull-policy", "MCP input check"),
    ("mcp-ui", "MCP request"),
    ("watch", "Watch repository"),
    ("auto_refresh", "Refresh repository"),
    ("save_state", "Save app state"),
    ("load_state", "Load app state"),
    ("commit-template", "Read commit template"),
    ("commit-activity", "Read commit activity"),
    ("activity-treemap", "Read file activity"),
    ("branch-merge-state", "Read merge state"),
    ("integration-discover", "Find extensions"),
    ("hosting-drafts-load", "Load pull request drafts"),
    ("library-check", "Check saved repositories"),
    ("repository-summary", "Read repository status"),
    ("lfs-status", "Read LFS status"),
    ("lfs-locks", "Read LFS locks"),
    ("lost-commits", "Find lost commits"),
    ("reflog", "Read reflog"),
    ("remote-check", "Check remote"),
    ("push-outlook", "Check what will push"),
    ("signature-verify", "Verify signature"),
    ("submodules", "Read submodules"),
    ("submodule-open", "Open submodule"),
    ("worktrees", "Read worktrees"),
    ("worktree-open", "Open worktree"),
    ("file-preview", "Read file"),
    ("file-backups", "Read file backups"),
    ("bisect", "Read bisect state"),
    ("bisect-tests", "Read bisect tests"),
    ("compare", "Compare"),
    ("compare-diff", "Read comparison diff"),
    ("diff-search", "Search diff"),
    ("file-history", "Read file history"),
    ("history-search", "Search history"),
    ("head", "Read current commit"),
    ("tree-list", "Read files"),
    ("tree-content", "Read file"),
    ("file-search", "Search in files"),
    ("file-search-open", "Open a search match"),
    ("map-code", "Read code map"),
    ("map-export", "Export Mermaid map"),
    ("map-history", "Read history map"),
    ("map-people", "Read people map"),
    ("image-preview", "Read image"),
    ("image-difference", "Compare images"),
    ("hosting-accounts", "Read hosting accounts"),
    ("hosting-list", "Read hosting list"),
    ("hosting-ci-marks", "Read CI marks"),
    ("hosting-ci-details", "Read checks"),
    ("hosting-ci-rerun-preview", "Review re-run"),
    ("hosting-ci-runs", "Read CI runs"),
    ("hosting-ci-run", "Read CI run"),
    ("managed-git-verify", "Verify bundled Git"),
];

/// Other operations named for diagnostics that have no title as an action.
const OTHERS: &[(&str, &str)] = &[
    ("auto-fetch", "Automatic fetch"),
    ("fetch-all", "Fetch all"),
    ("pull-all", "Pull all"),
    ("commit-draft", "Draft commit message"),
    ("github-sign-in", "GitHub sign-in"),
    ("preview-open", "Open preview"),
    ("preview-sign-in", "Sign in for a preview"),
    ("hosting-browser-sign-in", "Browser sign-in"),
    ("hosting-connect", "Connect hosting account"),
    ("hosting-create", "Create pull request"),
    ("hosting-merge-submit", "Merge pull request"),
    ("hosting-review-submit", "Submit pull request review"),
    ("hosting-thread-submit", "Reply in review thread"),
    ("hosting-ci-rerun", "Re-run failed jobs"),
    ("global-identity-save", "Save Git identity"),
];

/// What an operation shows while it runs, for example "Applying patch".
pub fn running(key: &str) -> String {
    NAMES
        .iter()
        .find(|(k, ..)| *k == key)
        .map_or_else(|| spelled(key), |(_, running, _)| (*running).to_owned())
}

/// An operation's title once it has run, for example "Apply patch".
pub fn done(key: &str) -> String {
    NAMES
        .iter()
        .find(|(k, ..)| *k == key)
        .map_or_else(|| spelled(key), |(.., done)| (*done).to_owned())
}

/// Whether `key` only reads. Anything unknown doesn't, so it stays in view.
pub fn is_read(key: &str) -> bool {
    READS.iter().any(|(k, _)| *k == key)
        || ["-preview", "-read", "-policy"].iter().any(|end| key.ends_with(end))
}

/// A recorded operation in words: an action's title, or what a read does.
pub fn recorded(key: &str) -> String {
    if NAMES.iter().any(|(k, ..)| *k == key) {
        return done(key);
    }
    if let Some((_, name)) = READS.iter().chain(OTHERS).find(|(k, _)| *k == key) {
        return (*name).to_owned();
    }
    let worded = |verb: &str, rest: &str| {
        let words = spelled(rest);
        let mut chars = words.chars();
        match (chars.next(), chars.next()) {
            // An abbreviation such as LFS keeps its capitals.
            (Some(first), Some(second)) if !second.is_uppercase() => {
                format!("{verb} {}{second}{}", first.to_lowercase(), chars.as_str())
            }
            _ => format!("{verb} {words}"),
        }
    };
    match (key.strip_suffix("-preview"), key.strip_suffix("-read")) {
        (Some(rest), _) => worded("Review", rest),
        (_, Some(rest)) => worded("Read", rest),
        _ => spelled(key),
    }
}

/// What `action` shows while it runs, naming the remote or the command where it helps.
pub fn running_action(action: &Action) -> String {
    let remote = |name: &[u8]| String::from_utf8_lossy(name).into_owned();
    match action {
        Action::Push { remote: name, .. } => format!("Pushing to {}", remote(name)),
        Action::Fetch(name) | Action::FetchWithTags(name, _) => {
            format!("Fetching from {}", remote(name))
        }
        Action::RunCommand { name, .. } => format!("Running {}", name.trim()),
        _ => running(action.label()),
    }
}

/// A key without a name of its own, in words: "lfs-checkout" reads "LFS checkout".
pub fn spelled(key: &str) -> String {
    let words: Vec<String> = key
        .split(['-', '_'])
        .filter(|w| !w.is_empty())
        .map(|w| match w {
            "lfs" => "LFS".to_owned(),
            "url" => "URL".to_owned(),
            "gitflow" => "Git Flow".to_owned(),
            other => other.to_owned(),
        })
        .collect();
    let text = words.join(" ");
    let mut chars = text.chars();
    chars
        .next()
        .map_or_else(String::new, |c| c.to_uppercase().chain(chars).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_read_as_words_and_unknown_ones_are_spelled_out() {
        assert_eq!(running("patch-import"), "Applying patch");
        assert_eq!(done("patch-import"), "Apply patch");
        assert_eq!(done("force-push"), "Force push");
        assert_eq!(running("lfs-something-new"), "LFS something new");
        assert_eq!(done("gitflow-publish"), "Git Flow publish");
        let mut keys: Vec<&str> = NAMES.iter().map(|(k, ..)| *k).collect();
        keys.sort_unstable();
        let before = keys.len();
        keys.dedup();
        assert_eq!(before, keys.len(), "each key is named once");
    }

    #[test]
    fn recorded_operations_read_as_words() {
        assert_eq!(recorded("fetch"), "Fetch");
        assert_eq!(recorded("tag-create"), "Create tag");
        assert_eq!(recorded("tag-preview"), "Review tag");
        assert_eq!(recorded("save_state"), "Save app state");
        assert_eq!(recorded("merge-preview"), "Review merge");
        assert_eq!(recorded("identity-read"), "Read identity");
        assert_eq!(recorded("lfs-preview"), "Review LFS");
        assert_eq!(recorded("something_new-here"), "Something new here");
        assert_eq!(recorded("hosting-merge-submit"), "Merge pull request");
        assert!(is_read("status") && is_read("merge-preview") && is_read("identity-read"));
        assert!(!is_read("push") && !is_read("hosting-merge-submit"));
        assert!(!is_read("something-new"), "an unknown operation stays in view");
        let mut keys: Vec<&str> = READS.iter().chain(OTHERS).map(|(k, _)| *k).collect();
        keys.sort_unstable();
        let before = keys.len();
        keys.dedup();
        assert_eq!(before, keys.len(), "each operation is named once");
        assert!(
            READS.iter().chain(OTHERS).all(|(k, _)| NAMES.iter().all(|(n, ..)| n != k)),
            "an action's title comes from NAMES"
        );
    }

    #[test]
    fn a_push_fetch_or_command_is_named_with_what_it_works_on() {
        let push = Action::Push {
            remote: b"origin".to_vec(),
            local: b"main".to_vec(),
            destination: b"main".to_vec(),
            set_upstream: false,
        };
        assert_eq!(running_action(&push), "Pushing to origin");
        assert_eq!(running_action(&Action::Fetch(b"upstream".to_vec())), "Fetching from upstream");
        let command = Action::RunCommand {
            name: " Build ".into(),
            program: "git.exe".into(),
            arguments: Vec::new(),
            storage: "commands".into(),
        };
        assert_eq!(running_action(&command), "Running Build");
        assert_eq!(running_action(&Action::Pull), "Pulling");
    }
}
