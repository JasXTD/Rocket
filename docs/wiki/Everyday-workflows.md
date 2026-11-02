# Everyday workflows

[Wiki home](Home.md) · [Getting started](Getting-started.md) · [Remotes](Remotes-and-accounts.md)

## Save a small, reviewable change

1. Confirm the repository tab and current branch.
2. Edit and save your files in your editor.
3. Open **Changes**, select each changed file and read its diff.
4. Drag the intended files into **Staged**. To undo that selection, drag them
   back to **Unstaged**; the files themselves stay as edited.
5. Write a message describing the result, then click **Commit**.
6. Check **Commits** and **Activity**. Push separately when you are ready to share.

For supported text diffs, hunk or line actions let you stage part of a file.
If a partial action is unavailable for a binary, rename, encoding or filter,
use a whole-file action or inspect the [documented limits](../../README.md#current-limits).
Do not use **Discard** to unstage: discard changes the working content.

## Work on a separate branch

1. Start from the branch whose contents you want to use. If it tracks a remote,
   fetch and inspect incoming commits before starting new work.
2. Open the **Branch** dropdown and choose **New Branch**. Enter a descriptive
   name, such as `feature/readme-update`, and review the starting commit.
3. Create the branch. If creating did not switch to it, use the branch's switch
   action. Confirm the current branch name before editing.
4. Make and commit your change as above. A new local branch is not published
   until you push it.

Switching branches changes the checked-out files. If Rocket refuses because
local changes would be affected, commit or save the work first. Read the
[save-and-switch options](#switch-tasks-with-uncommitted-work) below.

## Bring in someone else's changes

Use **Fetch** to update your view of the remote without changing the checked-out
files. Inspect **Commits**, then use **Pull** when you want to integrate the
incoming history into your current branch.

[The pull guide](Remotes-and-accounts.md#pull-into-your-current-branch) explains
fast-forward, merge and rebase. A rejected fast-forward pull usually means there
are commits on both sides; choose the strategy deliberately rather than forcing
an update.

## Switch tasks with uncommitted work

Choose the option that matches what you want to keep on screen:

| Option | Working files afterward | Saved work |
| --- | --- | --- |
| **Save stash** | Saved changes are cleaned from the working tree | A stash you can apply later |
| **Snapshot** on the Stashes page | Files and staging remain as they are | A saved snapshot |
| **Save, switch & restore** | Changes are restored onto the reviewed local branch, if possible | Recovery stash is kept |

For an ordinary stash:

1. Open **Stash** from the toolbar and review what will be saved. Include untracked
   files explicitly if you need them; ignored files are excluded.
2. Save the stash, then confirm it appears on the **Stashes** page.
3. Switch to the other branch and do your work.
4. Return to the intended branch, select the saved stash, inspect it and choose
   **Apply**. Applying keeps the stash.
5. Review **Changes** and resolve any conflicts. Only drop the saved stash after
   confirming the restored result is what you need.

Staged files are part of Git's saved index even when you choose a smaller file
selection. Read the preview. Never apply the same stash twice just because the
first attempt reported a conflict.

## Commit and push in one workflow

Once you are comfortable with separate commits and pushes, **Changes → Commit &
Push** reviews the staged changes and publishing destination together.

It still has two outcomes: a local commit, followed by a push. If the push is
rejected or cancelled after the commit succeeds, **keep that commit** and use
**Push** to retry after fixing the problem. Do not make a duplicate commit.

## Look up an earlier change

- Select a commit in **Commits** to inspect its message, files and diff.
- Use file history to follow changes to one path, including renames.
- Use **Blame** to find which commit last changed a line; it is an investigation
  aid, not an indication of who caused a bug.
- Use **Compare** to inspect two commits or branches without switching checkout.

See [Advanced tools](Advanced-tools.md) for links to these features and their limits.
