# Remotes and accounts

[Wiki home](Home.md) · [Everyday workflows](Everyday-workflows.md) · [Troubleshooting](Recovery-and-troubleshooting.md)

**Goal:** understand which connection you need, then fetch, pull or publish your
committed work. Local staging and commits work without a hosting account.

## Three settings that do different jobs

| Setting | Purpose | Where to find it |
| --- | --- | --- |
| Remote, such as `origin` | The repository address Git reads from or pushes to | **Repository → Remotes & Hosting → Manage Remotes** |
| Git credentials or SSH key | Permission to fetch and push at that address | Installed credential helper or **Edit → Preferences → SSH keys** |
| Hosting account | API features such as listing pull requests and creating a hosted repository | **Edit → Preferences → Accounts**, or **Accounts** on the Hosting page |

Signing in to a hosting account does not automatically sign Git in, change a
remote URL or replace an SSH key. Your commit author name/email is another
separate setting: it identifies the author of commits, not your login.

## Choose HTTPS or SSH

- **HTTPS:** use the HTTPS clone address from your hosting service. Rocket runs
  Git with your configured credential helper; Git Credential Manager is the
  supported route for browser authentication. Complete sign-in in its own prompt
  or browser window.
- **SSH:** use the SSH clone address. The corresponding public key must be
  authorized by the hosting service, and the private key must be usable locally.
  An encrypted key may need unlocking in your SSH agent after a restart.

Never put a token or password into the remote URL. Keep private keys and
passphrases out of logs, screenshots and support messages. Rocket's SSH manager
preserves your existing keys and main SSH configuration; it does not start the
Windows SSH service for you. See the [SSH reference](../../crates/rocket-ui/src/guide.md#ssh-keys)
for profile behavior.

## Use an existing SSH key

1. Open **Edit → Preferences → SSH keys**. Rocket lists every key once: the pairs in
   `%HOME%\.ssh` if HOME points to an existing folder, otherwise `%USERPROFILE%\.ssh`,
   and the keys Rocket made or set up. It reads public keys and checks for the adjacent
   private files; it does not read SSH config or unlock anything during discovery.
2. Choose the key, select **Copy public key** and authorize it at your host if needed.
3. Tick the key to unlock it in ssh-agent. OpenSSH asks for the passphrase, and Rocket
   never saves it.
4. To make the open repository use only this key, select **Use only this key**. Rocket
   asks for the host, user and port, taken from the repository's SSH remote when it has
   one, then shows a review. Your existing key stays at its original path.

**Rescan** finds keys added outside Rocket and reads ssh-agent again. Stop preserves the
previous list. The scan reports skipped pairs and stops at 512 entries or 128 keys. A
matching `.pub` is required; Rocket supports OpenSSH software keys, not PEM or
hardware/security-token keys. For a supported pair elsewhere, use **Add key file**.

## Add a remote to an existing local repository

1. Create the destination repository on your hosting service, or obtain its clone
   address from someone who has given you access. Adding a remote in Rocket does
   not create a repository on the server.
2. Open **Repository → Remotes & Hosting → Manage Remotes** and choose the add action.
3. Enter a name, commonly `origin`, and paste the plain clone URL. Review and save
   the remote settings.
4. Click **Fetch**, select that remote and run the operation.
5. Check **Activity** for success, then inspect its remote-tracking branches in
   **Commits**. An empty remote may have no branches yet; that is not an error.

Adding or editing a remote changes local configuration only. Removing it removes
the local remote configuration and associated remote-tracking refs, not the
server repository or your local branches.

## Fetch without changing your current files

Click **Fetch**, choose the remote and run it. Fetch downloads remote history and
refreshes remote-tracking branches. It does not merge into the checked-out branch.

Use this when you want to inspect incoming work before deciding what to do.
Ahead/behind information reflects the last successful fetch; a collaborator can
push again afterward.

## Pull into your current branch

1. Confirm the repository and current branch. Commit or stash tracked edits first.
2. Click **Pull**. Pull uses the branch's configured upstream. If it has none,
   establish the intended tracking relationship before proceeding.
3. Choose **Fetch and review**. Rocket fetches first and shows the incoming history.
4. Choose the strategy, inspect the preview and choose **Apply pull**.
5. Read the result. If it pauses at a conflict, use the
   [conflict workflow](Recovery-and-troubleshooting.md#resolve-a-conflict).

| Strategy | What happens | Use it when |
| --- | --- | --- |
| **Fast-forward only** | Advances to the incoming tip without merging or rewriting commits; refuses divergence | You want a straightforward update or a refusal you can inspect |
| **Merge** | Integrates both histories; may create a merge commit | Both sides have work and you want to retain the existing commits |
| **Rebase** | Replays your local commits on the fetched tip, changing their IDs | Your local commits are unpublished and this matches your team's workflow |

Pull does not silently stash your edits or switch strategies. Coordinate before
rebasing commits that other people already use.

## Push a local commit

1. Verify the commit exists in **Commits**. Unstaged and staged files are not
   uploaded by an ordinary push; only committed history is sent.
2. Click **Push** and select the intended remote, local branch and remote branch.
3. For a new branch, choose **Set upstream** if this should be its normal tracking
   destination. This controls what later pulls use.
4. Inspect the destination and commit preview, then confirm the push.
5. Check the reported result in **Activity** and the destination branch. A failed
   or uncertain result needs inspection before a retry.

If the server has new commits, fetch and inspect them. A rejection is not a
reason to enable force pushing. **Force push with lease** is a separate reviewed
history-replacement workflow; use it only when that replacement is intentional.

## Publish a local project as a new GitHub repository

Follow this only when you are ready to create a real remote and upload history.

1. Prepare and review the local commits, ignore rules and files you intend to share.
2. Open **Edit → Preferences → Accounts** and connect the intended GitHub account.
3. Open **Repository → Remotes & Hosting → Hosting** and choose **Publish repository**. Enter the owner, a new repository name and an
   unused remote name. Review **Private** versus **Public** deliberately.
4. Run **Review publish**, then read the selected branch, committed history,
   visibility, remote and authentication details.
5. Choose **Create repository and push** only after that review is correct.
6. Read both outcomes: repository creation can succeed even if pushing fails.
   Keep the created repository/remote and use an ordinary Push after fixing access;
   do not create a second repository just to retry.

Rocket does not generate a README, license or ignore rules during publication.
Cancellation cannot undo a remote repository that has already been created.

## Pull requests and other providers

All hosting adapters expose **Details and reviews** with their supported review
and merge actions. GitHub additionally offers multiline inline comments and
thread replies/resolve/reopen. Review the preview before every write. GitLab
approval and Bitbucket/Azure vote endpoints cannot carry accompanying messages;
publish a separate comment if necessary. Server permissions and merge policies
still apply, and missing or partial collections are shown explicitly.

Review/reply drafts and uncertain review, thread and merge results are saved in
the Windows user-protected local store. Connect the same account and use
**Saved drafts → Restore and refresh**. No saved request is automatically sent.
After cancellation or a lost response, inspect the same PR on the server,
refresh it in Rocket, then acknowledge the outcome before making a new preview.

Connect an account in **Preferences → Accounts**, open **Repository → Remotes & Hosting → Hosting**, select a repository, then choose the
pull request or issue view. A new PR/MR needs its branches already pushed; the
creation action does not push them for you. GitHub PR details also expose checks
and reviewed Comment, Approve and Request changes actions.

### Review changed lines on GitHub

1. Open a PR's **Details and reviews**, then **Files**.
2. Click an old (left) or new (right) line number. Write the comment and choose
   **Save to review draft**. Repeat for other lines; edit/remove comments in the composer.
3. Write the overall review, choose Comment, Approve or Request changes, and
   **Review before sending**. Inspect the comments and commit before submitting.

Drafts are protected locally and survive restarting Rocket. After new commits
arrive, refresh and re-select stale locations; Rocket does not silently move
comments to new lines.
Binary or incomplete patches cannot accept line comments. Open GitHub for those.

### Merge a reviewed GitHub PR

1. Finish or discard the review draft, then choose a method under **Merge on provider**.
2. Choose **Review merge** and inspect the remote target, method and commit IDs.
3. Choose **Merge reviewed PR**, then read the result and refresh the PR.

This changes the hosted target branch, not your local checkout. GitHub must
report clean mergeability and permit the method for your account. Rocket pins
the reviewed head; the base can still change after its final check. Branches are
not deleted. Cancelled requests may already have merged: refresh the same PR and
inspect GitHub before explicitly allowing a new attempt. The uncertainty guard
survives restarting Rocket and requires the same inspection after a restart.

### Create a PR from an existing fork

In **Create pull request**, enter the destination in **Repository**, enable
**Use a different source repository (fork)** and enter the source owner/repository.
Choose the two already-pushed branches, write the description and review. Rocket
checks the fork network and both tips again before creating the PR; it does not
create a fork or push a branch. This new workflow, inline publishing and merging
have isolated synthetic API/native tests; live GitHub acceptance is still separate.

### Connect GitHub Enterprise Server

In **Accounts**, choose **GitHub Enterprise Server** and enter its HTTPS server
address, without `/api/v3` or a repository path. Use a token issued by that
server, then **Connect and verify**. The authenticated user and reported server
version appear after verification. Enter `owner/repository` to browse PRs/issues,
search, review or prepare a PR merge. Existing PR preview/recovery rules apply.

Optional **Remember** stores this profile separately from GitHub.com accounts,
protected for your Windows user. This does not configure Git or SSH authentication.
Use a server-issued token: the GitHub.com browser sign-in and Publish repository
workflows do not apply to GHES. Certificate verification remains enabled; ask your
administrator about certificate trust or token policies when connection fails.

### Connect Azure DevOps Server

In **Accounts**, choose **Azure DevOps Server**, name the profile and enter the
HTTPS collection URL, including any virtual directory, such as
`https://devops.example.com/tfs/DefaultCollection`. Choose API **6.0** for Server
2020, **7.0** for 2022 or **7.1** for 2022.1+. Enter a PAT from that server and
**Connect and verify**. The selected API is saved with an optional protected
profile; changing it or the collection requires reconnecting.

Enter `project/repository` to list PRs or project-wide work items. Work-item state
filters use the exact names defined in that project. **Create pull request**
reviews two existing remote branches before sending. Cancellation after sending
can leave an unknown result: inspect the same repository's PR list before retrying.
This adapter does not implement Windows authentication, browser SSO, work-item
editing or inline reviews. Reviewed completion is available in PR details.
Existing Git/SSH credentials are separate.

GitHub has scoped live-test evidence. Additional adapters exist for GitHub Enterprise Server, GitLab,
Bitbucket Cloud, Bitbucket Server/Data Center, Azure DevOps Services/Server and Gitea,
but live acceptance for those services remains unverified. Consult
[Hosting](../../HOSTING.md) for authentication, search, pagination and provider limits.
