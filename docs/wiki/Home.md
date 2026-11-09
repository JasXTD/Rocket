# Rocket wiki

[Project overview](../../README.md) · [Full reference](../../crates/rocket-ui/src/guide.md)

Start here if Rocket's menus feel overwhelming. You can do ordinary Git work
with just **Changes**, **Commits**, **Fetch**, **Pull** and **Push**. Open the
advanced tools when you need a particular operation.

These pages cover **Rocket 0.92.1**. Windows examples use Ctrl and File Explorer;
on macOS, use the native menus and Command shortcuts where shown. Linux uses
your installed Git and desktop shell. Press **F1** for the guide inside Rocket,
or use the toolbar search to find a command without remembering its menu location.

## What do you want to do?

| Your goal | Read this |
| --- | --- |
| Open a project or make your first commit | [Getting started](Getting-started.md) |
| Scan a folder and organize saved repositories | [Repository manager](Repository-manager.md) |
| Explore code structure, Git ancestry and contributor connections | [Repository map](Repository-map.md) |
| Save changes, create a branch or switch tasks | [Everyday workflows](Everyday-workflows.md) |
| Connect GitHub, fetch, pull, push or publish a project | [Remotes and accounts](Remotes-and-accounts.md) |
| Fix a conflict, failed operation or cancelled push | [Recovery and troubleshooting](Recovery-and-troubleshooting.md) |
| Find history, stashes, worktrees, LFS or automation tools | [Advanced tools](Advanced-tools.md) |
| Install extensions or create a native service panel | [Extensions](Integrations.md) |
| Check for a newer Rocket version or recover an interrupted update | [App updates](App-updates.md) |
| Understand “staged,” “upstream,” “rebase” or other Git terms | [Glossary](Glossary.md) |

## The workflow to learn first

**Edit files → stage the changes you want → commit locally → push when ready.**

| Action | What changes? | Does it contact a remote? |
| --- | --- | --- |
| Edit | Working files on your computer | No |
| Stage / unstage | The selection for your next commit | No |
| Commit | Your local branch history | No |
| Fetch | Downloaded objects and remote-tracking branches | Yes |
| Pull | Fetches first, then integrates into your local branch | Yes |
| Push | Sends committed history to a remote branch | Yes |

This table describes Rocket's ordinary Git operations. Repository hooks and
custom commands can run other programs. Committing does not itself upload your
work to GitHub; **Commit & Push** explicitly combines those two steps.

## Find your way around

- **Help → Setup Guide:** a walkthrough with buttons for Git, opening/creating/cloning, identity and your first commit.
- **Summary:** local change counts, tracking-ref comparison and a suggested next step.
- **Changes:** inspect edits, stage files and write a commit message.
- **Commits:** browse saved history and select a commit to inspect it.
- **Map:** explore committed code, Git history and contributor connections locally.
- **Branch dropdown:** create a branch or find branch workflow actions.
- **Remotes:** inspect the saved destinations for fetch and push.
- **Activity:** see the running operation and its result in the bottom drawer.
- **Terminal:** use a shell in the selected repository when you need it: Windows PowerShell, Git Bash, Command Prompt, PowerShell 7 or your own, chosen in Preferences → Terminal.
- **Edit → Preferences:** Git selection, global defaults, the theme and other app settings.
- **Repository → Settings → Repository Settings:** settings for the open repository.

Start with the [local practice walkthrough](Getting-started.md#make-your-first-local-commit).
It needs no GitHub account and sends nothing to a server.

## Read the result before trying again

Some operations prepare a review before you confirm a write. If you change a
field or another program changes the repository, make a fresh review.
**Cancel/Stop is not undo:** a completed commit or remote update may remain.
Follow the [recovery guide](Recovery-and-troubleshooting.md) when an operation
fails, pauses or reports an unknown result.

The Wiki source is maintained in `docs/wiki` alongside Rocket. The GitHub Wiki is
generated from these pages; changes should be made in the repository first.
