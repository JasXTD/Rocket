//! Operations in words, for the status bar, the Activity drawer and its history.
//! `Action::label()` keys stay what diagnostics, automation and saved history record;
//! this module only names them for people.
use crate::backend::Action;

/// A key, what it shows while it runs, and its title once it has run.
const NAMES: &[(&str, &str, &str)] = &[
    ("benchmark", crate::i18n::mark("Benchmarking repository"), crate::i18n::mark("Repository benchmark")),
    ("benchmark-prepare", crate::i18n::mark("Preparing benchmark"), crate::i18n::mark("Prepare benchmark")),
    ("benchmark-refresh", crate::i18n::mark("Measuring refresh"), crate::i18n::mark("Benchmark refresh")),
    ("benchmark-history", crate::i18n::mark("Measuring history"), crate::i18n::mark("Benchmark history")),
    ("benchmark-diff", crate::i18n::mark("Measuring commit diff"), crate::i18n::mark("Benchmark commit diff")),
    ("review-window", crate::i18n::mark("Loading pinned review"), crate::i18n::mark("Open review window")),
    ("review-close", crate::i18n::mark("Closing review"), crate::i18n::mark("Close review window")),
    ("commit-link", crate::i18n::mark("Reading commit links"), crate::i18n::mark("Read commit links")),
    ("commit-link-open", crate::i18n::mark("Opening commit link"), crate::i18n::mark("Open commit link")),
    ("series-preview", crate::i18n::mark("Reviewing commit series"), crate::i18n::mark("Review commit series")),
    ("series-import", crate::i18n::mark("Importing commit series"), crate::i18n::mark("Import commit series")),
    ("series-continue", crate::i18n::mark("Continuing import"), crate::i18n::mark("Continue import")),
    ("series-skip", crate::i18n::mark("Skipping patch"), crate::i18n::mark("Skip patch")),
    ("series-abort", crate::i18n::mark("Restoring original checkout"), crate::i18n::mark("Abort import")),
    ("github-publish-preview", crate::i18n::mark("Reviewing GitHub publication"), crate::i18n::mark("Review GitHub publication")),
    ("github-publish", crate::i18n::mark("Publishing to GitHub"), crate::i18n::mark("Publish to GitHub")),
    ("stage", crate::i18n::mark("Staging"), crate::i18n::mark("Stage")),
    ("unstage", crate::i18n::mark("Unstaging"), crate::i18n::mark("Unstage")),
    ("stage-selection", crate::i18n::mark("Staging selection"), crate::i18n::mark("Stage selection")),
    ("unstage-selection", crate::i18n::mark("Unstaging selection"), crate::i18n::mark("Unstage selection")),
    ("discard-selection", crate::i18n::mark("Discarding selection"), crate::i18n::mark("Discard selection")),
    ("mark-resolved", crate::i18n::mark("Marking resolved"), crate::i18n::mark("Mark resolved")),
    ("conflict-save", crate::i18n::mark("Saving resolution"), crate::i18n::mark("Save resolution")),
    ("conflict-keep", crate::i18n::mark("Keeping one version"), crate::i18n::mark("Keep one version")),
    ("commit", crate::i18n::mark("Committing"), crate::i18n::mark("Commit")),
    ("undo-commit", crate::i18n::mark("Undoing commit"), crate::i18n::mark("Undo commit")),
    ("amend", crate::i18n::mark("Amending commit"), crate::i18n::mark("Amend commit")),
    ("cherry-pick", crate::i18n::mark("Cherry-picking"), crate::i18n::mark("Cherry-pick")),
    ("cherry-pick-continue", crate::i18n::mark("Continuing cherry-pick"), crate::i18n::mark("Continue cherry-pick")),
    ("cherry-pick-skip", crate::i18n::mark("Skipping commit"), crate::i18n::mark("Skip commit")),
    ("cherry-pick-abort", crate::i18n::mark("Stopping cherry-pick"), crate::i18n::mark("Abort cherry-pick")),
    ("revert", crate::i18n::mark("Reverting"), crate::i18n::mark("Revert")),
    ("revert-continue", crate::i18n::mark("Continuing revert"), crate::i18n::mark("Continue revert")),
    ("revert-skip", crate::i18n::mark("Skipping commit"), crate::i18n::mark("Skip commit")),
    ("revert-abort", crate::i18n::mark("Stopping revert"), crate::i18n::mark("Abort revert")),
    ("reset-soft", crate::i18n::mark("Resetting (soft)"), crate::i18n::mark("Reset (soft)")),
    ("reset-mixed", crate::i18n::mark("Resetting (mixed)"), crate::i18n::mark("Reset (mixed)")),
    ("reset-hard", crate::i18n::mark("Resetting (hard)"), crate::i18n::mark("Reset (hard)")),
    ("recover-branch", crate::i18n::mark("Recovering branch"), crate::i18n::mark("Recover branch")),
    ("rebase", crate::i18n::mark("Rebasing"), crate::i18n::mark("Rebase")),
    ("interactive-rebase", crate::i18n::mark("Rebasing"), crate::i18n::mark("Interactive rebase")),
    ("rebase-continue", crate::i18n::mark("Continuing rebase"), crate::i18n::mark("Continue rebase")),
    ("rebase-skip", crate::i18n::mark("Skipping commit"), crate::i18n::mark("Skip commit")),
    ("rebase-abort", crate::i18n::mark("Stopping rebase"), crate::i18n::mark("Abort rebase")),
    ("bisect-start", crate::i18n::mark("Starting bisect"), crate::i18n::mark("Start bisect")),
    ("bisect-auto", crate::i18n::mark("Running bisect tests"), crate::i18n::mark("Automated bisect")),
    ("bisect-good", crate::i18n::mark("Marking good"), crate::i18n::mark("Mark good")),
    ("bisect-bad", crate::i18n::mark("Marking bad"), crate::i18n::mark("Mark bad")),
    ("bisect-skip", crate::i18n::mark("Skipping commit"), crate::i18n::mark("Skip commit")),
    ("bisect-resume", crate::i18n::mark("Resuming bisect"), crate::i18n::mark("Resume bisect")),
    ("bisect-restore", crate::i18n::mark("Ending bisect"), crate::i18n::mark("End bisect")),
    ("tag-create", crate::i18n::mark("Creating tag"), crate::i18n::mark("Create tag")),
    ("tag-delete", crate::i18n::mark("Deleting tag"), crate::i18n::mark("Delete tag")),
    ("tag-delete-batch", crate::i18n::mark("Deleting tags"), crate::i18n::mark("Delete tags")),
    ("remote-tag-publish", crate::i18n::mark("Publishing tag"), crate::i18n::mark("Publish tag")),
    ("remote-tag-publish-batch", crate::i18n::mark("Publishing tags"), crate::i18n::mark("Publish tags")),
    ("remote-tag-delete", crate::i18n::mark("Deleting remote tag"), crate::i18n::mark("Delete remote tag")),
    ("worktree-create", crate::i18n::mark("Creating worktree"), crate::i18n::mark("Create worktree")),
    ("worktree-remove", crate::i18n::mark("Removing worktree"), crate::i18n::mark("Remove worktree")),
    ("worktree-move", crate::i18n::mark("Moving worktree"), crate::i18n::mark("Move worktree")),
    ("worktree-repair", crate::i18n::mark("Repairing worktrees"), crate::i18n::mark("Repair worktrees")),
    ("worktree-prune", crate::i18n::mark("Pruning worktrees"), crate::i18n::mark("Prune worktrees")),
    ("submodule-update", crate::i18n::mark("Updating submodule"), crate::i18n::mark("Update submodule")),
    ("submodule-init", crate::i18n::mark("Initializing submodule"), crate::i18n::mark("Initialize submodule")),
    ("submodule-add", crate::i18n::mark("Adding submodule"), crate::i18n::mark("Add submodule")),
    ("submodule-remove", crate::i18n::mark("Removing submodule"), crate::i18n::mark("Remove submodule")),
    ("submodule-deinitialize", crate::i18n::mark("Deinitializing submodule"), crate::i18n::mark("Deinitialize submodule")),
    ("submodule-sync", crate::i18n::mark("Syncing submodule URL"), crate::i18n::mark("Sync submodule URL")),
    ("submodule-set-url", crate::i18n::mark("Updating submodule URL"), crate::i18n::mark("Update submodule URL")),
    ("submodule-move", crate::i18n::mark("Moving submodule"), crate::i18n::mark("Move submodule")),
    ("submodule-batch", crate::i18n::mark("Updating submodules"), crate::i18n::mark("Update submodules")),
    ("lfs-fetch", crate::i18n::mark("Fetching LFS objects"), crate::i18n::mark("Fetch LFS objects")),
    ("lfs-push", crate::i18n::mark("Pushing LFS objects"), crate::i18n::mark("Push LFS objects")),
    ("lfs-checkout", crate::i18n::mark("Checking out LFS files"), crate::i18n::mark("Check out LFS files")),
    ("lfs-track", crate::i18n::mark("Tracking with LFS"), crate::i18n::mark("Track with LFS")),
    ("lfs-untrack", crate::i18n::mark("Untracking from LFS"), crate::i18n::mark("Untrack from LFS")),
    ("lfs-lock", crate::i18n::mark("Locking file"), crate::i18n::mark("Lock file")),
    ("lfs-unlock", crate::i18n::mark("Unlocking file"), crate::i18n::mark("Unlock file")),
    ("lfs-force-unlock", crate::i18n::mark("Unlocking file"), crate::i18n::mark("Force unlock")),
    ("lfs-prune", crate::i18n::mark("Cleaning the LFS cache"), crate::i18n::mark("Clean LFS cache")),
    ("file-discard", crate::i18n::mark("Discarding changes"), crate::i18n::mark("Discard changes")),
    ("file-restore", crate::i18n::mark("Restoring file"), crate::i18n::mark("Restore file")),
    ("file-recover", crate::i18n::mark("Recovering file"), crate::i18n::mark("Recover file")),
    ("patch-import", crate::i18n::mark("Applying patch"), crate::i18n::mark("Apply patch")),
    ("patch-export", crate::i18n::mark("Saving patch"), crate::i18n::mark("Save patch")),
    ("patch-series-export", crate::i18n::mark("Exporting commit series"), crate::i18n::mark("Export commit series")),
    ("signing-save", crate::i18n::mark("Saving signing settings"), crate::i18n::mark("Save signing settings")),
    ("identity-save", crate::i18n::mark("Saving identity"), crate::i18n::mark("Save identity")),
    ("issue-links-save", crate::i18n::mark("Saving issue links"), crate::i18n::mark("Save issue links")),
    ("branch-create", crate::i18n::mark("Creating branch"), crate::i18n::mark("Create branch")),
    ("branch-switch", crate::i18n::mark("Switching branch"), crate::i18n::mark("Switch branch")),
    ("branch-checkout-remote", crate::i18n::mark("Checking out branch"), crate::i18n::mark("Check out branch")),
    ("submodule-checkout", crate::i18n::mark("Updating submodules"), crate::i18n::mark("Update submodules")),
    ("saved-switch", crate::i18n::mark("Saving, switching and restoring"), crate::i18n::mark("Save, switch and restore")),
    ("commit-push", crate::i18n::mark("Committing and pushing"), crate::i18n::mark("Commit & Push")),
    ("branch-rename", crate::i18n::mark("Renaming branch"), crate::i18n::mark("Rename branch")),
    ("branch-delete", crate::i18n::mark("Deleting branch"), crate::i18n::mark("Delete branch")),
    ("branch-delete-batch", crate::i18n::mark("Deleting branches"), crate::i18n::mark("Delete branches")),
    ("branch-sync", crate::i18n::mark("Syncing branch"), crate::i18n::mark("Sync branch")),
    ("branch-finish", crate::i18n::mark("Finishing branch"), crate::i18n::mark("Finish branch")),
    ("branch-fast-forward", crate::i18n::mark("Fast-forwarding branches"), crate::i18n::mark("Fast-forward branches")),
    ("custom-command", crate::i18n::mark("Running command"), crate::i18n::mark("Custom command")),
    ("remote-add", crate::i18n::mark("Adding remote"), crate::i18n::mark("Add remote")),
    ("remote-edit", crate::i18n::mark("Changing remote URL"), crate::i18n::mark("Edit remote URL")),
    ("remote-remove", crate::i18n::mark("Removing remote"), crate::i18n::mark("Remove remote")),
    ("remote-rename", crate::i18n::mark("Renaming remote"), crate::i18n::mark("Rename remote")),
    ("remote-prune", crate::i18n::mark("Removing tracking refs"), crate::i18n::mark("Remove tracking refs")),
    ("remote-branch-delete", crate::i18n::mark("Deleting remote branch"), crate::i18n::mark("Delete remote branch")),
    ("fetch", crate::i18n::mark("Fetching"), crate::i18n::mark("Fetch")),
    ("pull", crate::i18n::mark("Pulling"), crate::i18n::mark("Pull")),
    ("pull-fetch", crate::i18n::mark("Fetching for pull"), crate::i18n::mark("Fetch for pull")),
    ("pull-apply", crate::i18n::mark("Pulling"), crate::i18n::mark("Pull")),
    ("push", crate::i18n::mark("Pushing"), crate::i18n::mark("Push")),
    ("force-push", crate::i18n::mark("Force pushing"), crate::i18n::mark("Force push")),
    ("atomic-push", crate::i18n::mark("Pushing branches together"), crate::i18n::mark("Push branches together")),
    ("ignore-save", crate::i18n::mark("Saving .gitignore"), crate::i18n::mark("Save .gitignore")),
    ("ignore-tracked", crate::i18n::mark("Ignoring tracked files"), crate::i18n::mark("Ignore tracked files")),
    ("stash-push", crate::i18n::mark("Saving stash"), crate::i18n::mark("Save stash")),
    ("stash-apply", crate::i18n::mark("Applying stash"), crate::i18n::mark("Apply stash")),
    ("stash-drop", crate::i18n::mark("Dropping stash"), crate::i18n::mark("Drop stash")),
    ("stash-rename", crate::i18n::mark("Renaming stash"), crate::i18n::mark("Rename stash")),
    ("stash-control", crate::i18n::mark("Updating stashes"), crate::i18n::mark("Update stashes")),
    ("stash-selection", crate::i18n::mark("Stashing selection"), crate::i18n::mark("Stash selection")),
    ("stash-apply-selection", crate::i18n::mark("Applying saved selection"), crate::i18n::mark("Apply saved selection")),
    ("stash-apply-preview", crate::i18n::mark("Reviewing saved selection"), crate::i18n::mark("Review saved selection")),
    ("stash-snapshot", crate::i18n::mark("Saving snapshot"), crate::i18n::mark("Save snapshot")),
    ("saved-work", crate::i18n::mark("Saving work"), crate::i18n::mark("Save work")),
    ("merge", crate::i18n::mark("Merging"), crate::i18n::mark("Merge")),
    ("merge-continue", crate::i18n::mark("Finishing merge"), crate::i18n::mark("Continue merge")),
    ("merge-abort", crate::i18n::mark("Stopping merge"), crate::i18n::mark("Abort merge")),
    ("merge-finish", crate::i18n::mark("Clearing merge state"), crate::i18n::mark("Clear merge state")),
    ("gitflow-init", crate::i18n::mark("Setting up Git Flow"), crate::i18n::mark("Set up Git Flow")),
    ("gitflow-start", crate::i18n::mark("Starting Git Flow branch"), crate::i18n::mark("Start Git Flow branch")),
    ("gitflow-finish", crate::i18n::mark("Finishing Git Flow branch"), crate::i18n::mark("Finish Git Flow branch")),
    ("clone", crate::i18n::mark("Cloning repository"), crate::i18n::mark("Clone repository")),
    ("keep-preview", crate::i18n::mark("Keeping preview"), crate::i18n::mark("Keep preview")),
    ("discard-preview", crate::i18n::mark("Discarding preview"), crate::i18n::mark("Discard preview")),
    ("init", crate::i18n::mark("Creating repository"), crate::i18n::mark("Create repository")),
];

/// Operations that only read, by the name diagnostics show. Keys ending in `-preview`,
/// `-read` or `-policy` read too, and are worded from the key unless named here. A key
/// that is in neither counts as an action, so diagnostics never hide a write.
const READS: &[(&str, &str)] = &[
    ("status", crate::i18n::mark("Read status")),
    ("github-search", crate::i18n::mark("Search GitHub")),
    ("diff", crate::i18n::mark("Read diff")),
    ("diff-words", crate::i18n::mark("Read diff words")),
    ("history", crate::i18n::mark("Read history")),
    ("directory-history", crate::i18n::mark("Read folder history")),
    ("tags", crate::i18n::mark("Read tags")),
    ("tag-inspect", crate::i18n::mark("Read tag")),
    ("tag-preview", crate::i18n::mark("Review tag")),
    ("remote-tags", crate::i18n::mark("Read remote tags")),
    ("remote-tag-preview", crate::i18n::mark("Review remote tag")),
    ("remote-tag-policy", crate::i18n::mark("MCP input check")),
    ("force-push-policy", crate::i18n::mark("MCP input check")),
    ("atomic-push-policy", crate::i18n::mark("MCP input check")),
    ("pull-policy", crate::i18n::mark("MCP input check")),
    ("mcp-ui", crate::i18n::mark("MCP request")),
    ("watch", crate::i18n::mark("Watch repository")),
    ("auto_refresh", crate::i18n::mark("Refresh repository")),
    ("save_state", crate::i18n::mark("Save app state")),
    ("load_state", crate::i18n::mark("Load app state")),
    ("commit-template", crate::i18n::mark("Read commit template")),
    ("commit-activity", crate::i18n::mark("Read commit activity")),
    ("activity-treemap", crate::i18n::mark("Read file activity")),
    ("branch-merge-state", crate::i18n::mark("Read merge state")),
    ("integration-discover", crate::i18n::mark("Find extensions")),
    ("hosting-drafts-load", crate::i18n::mark("Load pull request drafts")),
    ("library-check", crate::i18n::mark("Check saved repositories")),
    ("repository-summary", crate::i18n::mark("Read repository status")),
    ("lfs-status", crate::i18n::mark("Read LFS status")),
    ("lfs-locks", crate::i18n::mark("Read LFS locks")),
    ("lost-commits", crate::i18n::mark("Find lost commits")),
    ("reflog", crate::i18n::mark("Read reflog")),
    ("remote-check", crate::i18n::mark("Check remote")),
    ("push-outlook", crate::i18n::mark("Check what will push")),
    ("signature-verify", crate::i18n::mark("Verify signature")),
    ("submodules", crate::i18n::mark("Read submodules")),
    ("submodule-open", crate::i18n::mark("Open submodule")),
    ("worktrees", crate::i18n::mark("Read worktrees")),
    ("worktree-open", crate::i18n::mark("Open worktree")),
    ("file-preview", crate::i18n::mark("Read file")),
    ("file-backups", crate::i18n::mark("Read file backups")),
    ("bisect", crate::i18n::mark("Read bisect state")),
    ("bisect-tests", crate::i18n::mark("Read bisect tests")),
    ("compare", crate::i18n::mark("Compare")),
    ("compare-diff", crate::i18n::mark("Read comparison diff")),
    ("diff-search", crate::i18n::mark("Search diff")),
    ("file-history", crate::i18n::mark("Read file history")),
    ("history-search", crate::i18n::mark("Search history")),
    ("head", crate::i18n::mark("Read current commit")),
    ("tree-list", crate::i18n::mark("Read files")),
    ("tree-content", crate::i18n::mark("Read file")),
    ("file-search", crate::i18n::mark("Search in files")),
    ("file-search-open", crate::i18n::mark("Open a search match")),
    ("map-code", crate::i18n::mark("Read code map")),
    ("map-export", crate::i18n::mark("Export Mermaid map")),
    ("map-history", crate::i18n::mark("Read history map")),
    ("map-people", crate::i18n::mark("Read people map")),
    ("image-preview", crate::i18n::mark("Read image")),
    ("image-difference", crate::i18n::mark("Compare images")),
    ("hosting-accounts", crate::i18n::mark("Read hosting accounts")),
    ("hosting-list", crate::i18n::mark("Read hosting list")),
    ("hosting-ci-marks", crate::i18n::mark("Read CI marks")),
    ("hosting-ci-details", crate::i18n::mark("Read checks")),
    ("hosting-ci-rerun-preview", crate::i18n::mark("Review re-run")),
    ("hosting-ci-runs", crate::i18n::mark("Read CI runs")),
    ("hosting-ci-run", crate::i18n::mark("Read CI run")),
    ("managed-git-verify", crate::i18n::mark("Verify bundled Git")),
];

/// Other operations named for diagnostics that have no title as an action.
const OTHERS: &[(&str, &str)] = &[
    ("auto-fetch", crate::i18n::mark("Automatic fetch")),
    ("fetch-all", crate::i18n::mark("Fetch all")),
    ("pull-all", crate::i18n::mark("Pull all")),
    ("commit-draft", crate::i18n::mark("Draft commit message")),
    ("github-sign-in", crate::i18n::mark("GitHub sign-in")),
    ("preview-open", crate::i18n::mark("Open preview")),
    ("preview-sign-in", crate::i18n::mark("Sign in for a preview")),
    ("hosting-browser-sign-in", crate::i18n::mark("Browser sign-in")),
    ("hosting-connect", crate::i18n::mark("Connect hosting account")),
    ("hosting-create", crate::i18n::mark("Create pull request")),
    ("hosting-merge-submit", crate::i18n::mark("Merge pull request")),
    ("hosting-review-submit", crate::i18n::mark("Submit pull request review")),
    ("hosting-thread-submit", crate::i18n::mark("Reply in review thread")),
    ("hosting-ci-rerun", crate::i18n::mark("Re-run failed jobs")),
    ("global-identity-save", crate::i18n::mark("Save Git identity")),
];

/// What an operation shows while it runs, for example "Applying patch".
pub fn running(key: &str) -> String {
    NAMES
        .iter()
        .find(|(k, ..)| *k == key)
        .map_or_else(|| spelled(key), |(_, running, _)| crate::i18n::tr(running).to_owned())
}

/// An operation's title once it has run, for example "Apply patch".
pub fn done(key: &str) -> String {
    NAMES
        .iter()
        .find(|(k, ..)| *k == key)
        .map_or_else(|| spelled(key), |(.., done)| crate::i18n::tr(done).to_owned())
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
        return crate::i18n::tr(name).to_owned();
    }
    let worded = |rest: &str| {
        let words = spelled(rest);
        let mut chars = words.chars();
        match (chars.next(), chars.next()) {
            // An abbreviation such as LFS keeps its capitals.
            (Some(first), Some(second)) if !second.is_uppercase() => {
                format!("{}{second}{}", first.to_lowercase(), chars.as_str())
            }
            _ => words,
        }
    };
    match (key.strip_suffix("-preview"), key.strip_suffix("-read")) {
        (Some(rest), _) => crate::i18n::trf!("Review {operation}", operation = worded(rest)),
        (_, Some(rest)) => crate::i18n::trf!("Read {operation}", operation = worded(rest)),
        _ => spelled(key),
    }
}

/// What `action` shows while it runs, naming the remote or the command where it helps.
pub fn running_action(action: &Action) -> String {
    let remote = |name: &[u8]| String::from_utf8_lossy(name).into_owned();
    match action {
        Action::Push { remote: name, .. } => crate::i18n::trf!("Pushing to {remote}", remote = remote(name)),
        Action::Fetch(name) | Action::FetchWithTags(name, _) => {
            crate::i18n::trf!("Fetching from {remote}", remote = remote(name))
        }
        Action::RunCommand { name, .. } => crate::i18n::trf!("Running {command}", command = name.trim()),
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
            "gitflow" => crate::i18n::tr("Git Flow").to_owned(),
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
    fn localized_names_preserve_keys_and_caller_supplied_names() {
        let action = Action::Fetch("origin/新".as_bytes().to_vec());
        for language in crate::i18n::Language::ALL.into_iter().skip(1) {
            crate::i18n::scoped(language, || {
                assert_ne!(running("patch-import"), "Applying patch", "{language:?}");
                assert_ne!(recorded("status"), "Read status", "{language:?}");
                assert_eq!(action.label(), "fetch");
                assert!(running_action(&action).contains("origin/新"));
                assert_eq!(running("extension-job"), "Extension job");
                assert!(is_read("identity-read") && !is_read("extension-job"));
            });
        }
    }

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
