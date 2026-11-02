# Git terms in plain language

[Wiki home](Home.md) · [First commit](Getting-started.md) · [Remotes](Remotes-and-accounts.md)

| Term | Meaning in Rocket |
| --- | --- |
| Repository / repo | A project with Git history and configuration |
| Working tree | The project files currently checked out on your computer |
| Untracked file | A file Git has not added to its tracked set |
| Unstaged change | A working-file edit not included in the current staging snapshot |
| Staged / index | The prepared snapshot that an ordinary commit will record |
| Commit | A saved snapshot with metadata and a message, created locally |
| Commit ID / hash | The identifier for that saved commit; rewriting creates a different ID |
| Branch | A name that points to a line of commits; new commits advance the current branch |
| HEAD | What you currently have checked out, usually your current branch |
| Detached HEAD | A specific commit is checked out without a current local branch; create a branch to retain new work clearly |
| Remote | A saved name and address for another repository; `origin` is a convention, not a special server |
| Remote-tracking branch | Your local record of a remote branch, such as `origin/main`, updated by fetch |
| Upstream | The branch your local branch normally tracks for pull and ahead/behind information |
| Ahead / behind | Commits found only locally or only in the tracking branch; based on the last known remote state |
| Fetch | Download remote history and update remote-tracking refs without integrating into your branch |
| Pull | Fetch, then integrate the selected upstream history using a strategy |
| Push | Send committed history and request an update of a remote branch or tag |
| Fast-forward | Move a branch to a descendant commit without creating a merge commit |
| Merge | Combine histories, potentially recording a new merge commit |
| Rebase | Replay commits on a different starting point, changing their IDs |
| Cherry-pick | Apply a selected commit's changes as a new commit on the current branch |
| Revert | Record a new commit that reverses an earlier commit's changes |
| Amend | Replace the latest commit with a revised commit |
| Conflict | Git cannot choose the combined result automatically; you must resolve it |
| Hunk | One section of a text diff, with nearby context |
| Stash | Saved unfinished work that can be applied later; it is separate from ordinary branch history |
| Tag | A named reference, often used to mark a release |
| Worktree | Another checked-out folder sharing the repository's history, often on a different branch |
| Submodule | A separate repository recorded inside a parent repository at a particular commit |
| Git LFS | An extension that stores large file contents separately while Git records small pointers |
| Reflog | Local records of previous reference positions, useful for recovery; not a permanent remote backup |
| PR / MR | A pull/merge request on a hosting service asking to review and integrate a branch |
| Force with lease | Replace a remote ref only if it still matches an expected value; still a history-replacement operation |

## Three common mix-ups

- **Unstage versus discard:** unstage changes the next commit's selection;
  discard changes or removes working content.
- **Commit versus push:** commit saves locally; push publishes committed history
  to another repository.
- **Git versus GitHub:** Git stores and manipulates repository history; GitHub is
  one service that can host a remote and provide pull requests and permissions.

When a term appears in a failed operation, use the
[troubleshooting table](Recovery-and-troubleshooting.md#what-to-do-next) to find
the next action.
