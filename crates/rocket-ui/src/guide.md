# Rocket guide

<!-- Shown in Help → Guide (F1). Each `## ` heading is a topic and must match a title in
guide.rs; `### ` is a subheading and `- ` a bullet. Other lines join into paragraphs until
a blank line. Dialogs keep only what a decision needs; background explanations live here. -->

## Getting started

Rocket is a native Git workspace. Each open repository has its own tab. The sidebar switches between Summary, Changes, Commits, Compare, AI review and Files, your branches, remotes and stashes, and the pages for tags, worktrees, submodules, Git LFS, path history and statistics. Each extension you enable adds its own entry. Hosting is under Repository → Remotes & Hosting.

Operations that change a repository open a review first. The dialog shows what will happen, and nothing is written until you confirm. Press F1, or the ? in a dialog's header, to open the guide at the topic for what is on screen.

### Crash reports

- Open Help → Crash Reports after an unexpected failure. Reports stay on this PC; Rocket never uploads them. The list groups them into Today and Earlier. Select one to see what stopped, when, which version and the error; Refresh reads the list again without blocking repository work.
- If Rocket or the MCP server stopped, or recorded an internal error, in the 15 minutes before this start, a notice in the corner offers that report once: View opens it, Dismiss hides the notice. Reports from the command-line tool are listed but not offered.
- An internal error is a Rust panic that may have been caught in a worker, so the whole app didn't necessarily stop. A report that says Rocket stopped is an unhandled Windows exception, named in plain words such as Access violation or Stack overflow. When it failed in the graphics driver's OpenGL, the report says so; a driver update often helps. Forced termination and some severe failures can prevent a report.
- Copy report copies a structured summary. Save as saves it as a new .json file: Browse picks the folder, keep or change the name, then Save. Existing files are never replaced. Stop cancels a save before it is published; a file already saved stays saved. Code offsets show where in Rocket's code it failed, for a bug report.
- Reports omit panic messages, repository contents, credentials and memory dumps. Review a saved file before sharing it. Open folder shows the storage folder, by default %LOCALAPPDATA%\Rocket\crashes (on Linux, ~/.local/state/rocket/crashes); old inactive session files rotate automatically.

### Saved repositories

- File → Repository manager saves repository shortcuts in folders and workspaces. Scan folder asks for a folder, then lists the repositories beneath it without opening tabs or changing files. Subfolders looks deeper; turn it off to check only the chosen folder and its direct children, which scans again.
- Results are grouped by the folders they sit in on disk. Each row shows the repository's branch and when its last commit was made. In library marks repositories already saved, which can't be added twice; linked worktrees stay separate.
- New repositories start ticked. A group's box changes only the rows the list shows, so search first to pick part of a group; ticked rows hidden by the search stay ticked and the footer counts them. All, New, Saved and Skipped switch the list, and Skipped says why each entry was passed over.
- Save in decides where they go. Match disk folders, the default when the results have subfolders, makes one saved folder for each disk folder and reuses one with the same name. Choose a saved folder, or none, to put them all in one place.
- Add rechecks the selection and saves it together, so a failed or stopped check adds nothing. Stop cancels the scan or the check. Git metadata and folder links are never followed; when a scan reaches its limit, choose a narrower folder.

### Repository map

- Map in the sidebar shows how the repository fits together in Code, History and People. Each reads a pinned local snapshot of the revision on the button at the right of the toolbar; Refresh reads it again. Reads never switch the checkout or change the index, and uncommitted files aren't included.
- The revision button lists HEAD, the checked-out branch and the other branches, and a row maps that one. Type any commit, branch or tag in the box above them and press Enter to map it.
- Code opens on Packages when the repository has Cargo packages: what a package uses sits below it and arrows point to what is used. Folders shows the folder tree, folders closed at first; open one with the arrow on its card, or use Expand all and Collapse all. A folder's own files are summed up on one card, named when there are three or fewer, and listed in Details.
- Details, beside the map or as a drawer in a narrow window, says what the selection is and what it connects to. Show folders opens a package's folder, Commits here shows the commits that touched a folder or file, and a row in its lists selects that item.
- History opens on Branches & merges: the main line of the revision, each merged branch on one line, and runs of plain commits folded into a count. With All branches, branches ahead of the main line show above it. Every commit lists them all with their lanes. Path and Leave out narrow History and People to a literal path, or leave out what a commit or branch already has.
- People opens on Who works where: a row per person, a column per folder, and their commits touching it in each cell. A folder holding most of the changes is split into its subfolders. All time, 90 days and 30 days filter by author time. Shared files joins people who changed the same files. Counts use recorded authors and the snapshot's .mailmap; they describe overlapping work, not productivity or collaboration.
- Search finds names, paths, authors and commit IDs. In Packages, Folders and Shared files, drag to pan, scroll or use + and − to zoom, Fit shows everything and Focus centres the selection; Connected only keeps the selection and what it connects to. Stop and read errors keep the previous snapshot.
- Export as Mermaid shows the text of what the map shows. Copy copies it; Save writes a new .mmd file to the folder you choose and never replaces a file. It holds names and paths only, never file contents.
- History reads the newest 500 commits; Load older adds more, up to 2,000. A map holds up to 4,096 nodes and draws up to 240 cards at once. When a map is partial it says so: narrow the scope or search.

### Command palette

- Ctrl+K, or the search field in the toolbar, opens the command palette. With nothing typed it lists what you can do now: commands you ran from it this session, suggestions such as Pull and Push with their commit counts, views, branches to switch to and the rest by group. Without a repository it offers opening, cloning or creating one, and your recent repositories.
- Type to search every command, branch and view; the best matches come first. A command that cannot run right now stays in the results with the reason, and the arrow keys skip it. Enter runs the selected command and Esc closes the palette.

### Menus

- Repository keeps Fetch, Pull, Push and Push Branches Together on top and groups the rest by task: Commit, Branch, Git Flow, Stash, Patches, History & Recovery, Custom Commands, Remotes & Hosting and Settings. Extensions joins them when an enabled extension has commands. The command palette finds every command by name.
- View → Go to lists the sidebar’s pages, with a tick on the one you’re on, so they stay reachable when the sidebar is hidden or shows icons only.
- Edit → Undo, Redo, Cut, Copy, Paste and Select All act on the text field you were typing in, like their shortcuts, and are grey when no field has focus; Cut and Copy also need selected text. Outside a text field, Ctrl+V applies a patch from the clipboard (Repository → Patches). Edit → Undo Last Commit is separate: it undoes a commit, never typing.

### Setup Guide and repository summary

- Help > Setup Guide, or Setup Guide on the welcome screen, walks through finding Git, opening/creating/cloning a repository, reviewing the commit identity and making a first local commit. Its buttons open the normal controls; the guide does not stage, commit or publish anything automatically.
- Repository Settings saves identity overrides for this repository only. Your commit author name/email are separate from a hosting login.
- Summary in the sidebar shows staged, unstaged, untracked and conflicted files, the current branch and the next suggested step. A file can count as both staged and unstaged. It uses the same repository snapshot as Changes, including automatic refresh after external edits.
- Incoming/outgoing counts use local tracking refs, usually updated by your last fetch. Missing status or an unavailable comparison is explicit, not proof that the checkout is clean or up to date. A bare repository has no working-file summary.

### Credentials

- Edit > Preferences > Accounts holds your hosting accounts, and Preferences > SSH keys lists your SSH keys. Both work before you open a repository; the command palette (Ctrl+K) finds them as Accounts and SSH keys.
- Git uses your configured credential helpers and SSH agent.
- Hosting accounts and the local MCP access token are stored separately in Rocket's application settings.
- On Linux, remembered tokens, saved SSH passphrases and saved drafts are encrypted with a key Rocket keeps in your desktop keyring, "Rocket credential protection key". It needs libsecret (Ubuntu: libsecret-1-0) and an unlocked keyring such as GNOME Keyring or KeePassXC; desktop sessions have both. WSL has neither: install gnome-keyring and libsecret-1-0, then start the keyring before Rocket, for example with `gnome-keyring-daemon --unlock --components=secrets`, which reads its password from the terminal. Data encrypted with the key can't be read without it, so removing it from the keyring loses what was saved.

### Diff overview

- Long text diffs have a narrow overview at the right: green for added rows, red for removed rows, and a blue outline for the current viewport. It describes the loaded diff, not omitted context or content beyond a preview limit. Unified and side-by-side views use their own row positions.
- Click or drag the overview to scroll. With it focused, use arrows, Page Up/Down, Home or End. It never selects lines for staging or changes files. It hides when the diff fits; image and binary previews keep their existing controls.
- A diff's header follows the pane's width, not its height: one row from about 760 px, two rows below that. Find (Ctrl+F) shows its match count, ↑ ↓ (F3 and Shift+F3) and × inside the box. Under 440 px Side by side reads Split and Words moves into the ··· menu.

### Compare local contents

- In Compare, choose Revisions to compare two branches, tags or commits, or Staging area or Working tree to compare a committed base with your local contents. Working tree includes tracked files and staged additions; it excludes untracked files.
- Revisions is a direct comparison: it shows how Target's contents differ from Base's, not only what Target added since the two split.
- Results are snapshots. Branch tips that move later and staged or working contents that change aren't followed; run Compare again before opening another diff. Click a commit ID in the result line to copy it. Reads are cancellable and do not stage files.
- A file that was only renamed or copied shows where it moved instead of a diff.
- Submodule diffs list locally available added and removed commits. Missing checkouts or objects keep the original commit IDs with an explanation; Rocket does not fetch automatically.

### Files

- Files shows the repository at HEAD, a branch, a tag or a commit. Type one and choose Browse revision, or use Repository → History & Recovery → Browse Files at Commit. It only reads: nothing in your checkout changes.
- Text files preview up to 50,000 lines and 8 KiB per line; a binary file shows only its size. Folder history opens the history of the folder you chose.

### AI tools

- Rocket has three AI tools. Each one runs only when you ask, and none of them commits, pushes or saves for you.
- AI drafts write a commit message from your staged changes. Turn them on in Edit → Preferences → Commit messages; Draft with Claude or Draft with Codex then shows in the commit box in Changes. See Commit message drafts under Commits.
- AI review checks the commits on a branch. Open AI review in the sidebar, View → Go to → AI review, or AI review in the command palette.
- AI resolve proposes how to solve the conflicts in one file. It shows in Changes and in the conflict editor, only while a text conflict is open. See Conflicts.
- AI drafts and AI review need Claude Code or Codex on this computer. AI resolve can also ask an AI app connected through the MCP server. Rocket stores no AI API key: the program you choose sends your code under its own account.

### AI review

- Base and Target choose what to review: the commits on Target since it split from Base. Opened on a branch, the page reviews it against main, or master. Opened on main itself, it asks which branch to review.
- Rocket reads the changes by itself when the page opens and when Base or Target change; a typed name is read once you stop typing. A name that matches no branch, tag or commit is marked on its field, and the last preview stays until a name is found. Nothing is sent yet.
- Changes lists the files and shows each diff exactly as it will be sent. Binary files are named only. A diff over 128 KiB is shortened to fit, and the files left out are named only. Show exact data shows the whole text that goes out.
- Send to Claude Code or Send to Codex sends that text through the program on this computer, under its account and its data and usage policies. Its tools are off, and Codex may add your global AGENTS instructions. When the program isn't found, Browse chooses it; commit drafts and AI resolve use the same program.
- Send checks Base and Target again first. If one moved, Rocket reads the changes again instead, and nothing is sent.
- Stop ends the program, and a review stops after two minutes. A request already sent may still be processed by the provider.
- Findings shows each one as Bug or Check with its file and line; click the place to see it in Changes. A reply that doesn't come as a list shows as written. Findings are suggestions to check: they aren't saved and change no files, and no findings is not proof the code is right. Copy findings copies them as text.

## Reviews and safety

Every operation that writes follows the same rules, so dialogs do not repeat them.

- Review first. A dialog reads the current state and shows the result before anything is written. Editing a field after a review discards it; review again.
- Checked again before writing. Branch tips, files and settings from the review are compared with the repository right before the change. If anything changed in between, Rocket stops instead of guessing.
- Recovery copies. Before a working file is replaced or removed, Rocket keeps a recovery copy in the repository's Git directory. Operations that rewrite history keep the original tip at a recovery reference, which Reflog and recovery can bring back.
- No silent fallbacks. Rocket does not add force, autostash or automatic retries to an operation, and does not switch to a different kind of operation when the requested one fails.
- Stopping is not undo. Stop ends only work that has not happened yet. Changes already written, locally or on a remote, stay. Rocket then refreshes and verifies what actually happened; inspect that result before trying again.
- Long previews are shortened on screen only. The operation always uses the complete reviewed content.

## Repositories

### Open, create and clone

- Open lists folders with Git repositories marked by their current branch. Double-click a repository to open it, or a folder to look inside; the arrow at the end of a row goes into any folder. Ctrl+L, or the pencil, takes a typed or pasted path. Hidden and system folders are not listed, as in File Explorer.
- New repository works in a new folder or in an existing project folder; its parent folder must exist. Under the folder, Rocket says what creating will do before you create. Your project files stay as they are: nothing is staged or committed. Existing repositories, and folders inside another repository, cannot be initialized here; Open it opens the repository instead.
- Clone accepts an HTTPS URL, an SSH address such as ssh://git@host/owner/repo.git, or a local repository path. The clone goes into a new folder: choose its parent folder, and its name comes from the source until you change it. The parent folder must exist. Rocket refuses existing destinations, including empty folders.
- On Linux, creating, cloning, adding a worktree and other steps that must never replace a file need a folder on a local filesystem such as ext4, XFS or Btrfs. NFS, some FUSE mounts and WSL's /mnt drives can't promise that, so Rocket refuses there and says why.

### Automatic refresh

- Rocket refreshes a repository when its files or Git data change outside Rocket; F5 refreshes at any time. When automatic refresh can't run, Rocket shows a note saying why.
- On Linux, each folder of the working folder and the Git folder is watched separately. Linux limits watched folders per user (fs.inotify.max_user_watches), and Rocket uses at most half of them across its repositories, so other programs keep theirs. A repository with more folders shows the note: raise the limit, for example with sudo sysctl fs.inotify.max_user_watches=1048576 (a file in /etc/sysctl.d keeps it), then press F5.
- Linux isn't told about changes made through a network share, or by Windows programs in WSL's /mnt drives; press F5 after those.

### Repository settings

- Commit identity overrides are saved in this repository's Git config and are shared by its linked worktrees. Leave a field blank to remove its local override. Global settings stay unchanged.
- Author and committer overrides can also affect commits. Signing settings are preserved.

### Saved repositories

- Opening a saved workspace adds its repositories to your session. Existing tabs, terminal sessions and drafts stay open.
- In File → Repository manager, the + beside Workspaces creates an empty group and Save open tabs captures your open repositories. Select a workspace and use New child, or change Parent when editing, to organize up to eight levels.
- Workspaces open their own repositories by default. Turn on Include nested workspaces to preview and open all descendants, with duplicates removed and a maximum of 64 repositories per opening. Stop keeps tabs that have already opened.
- Add repository checks that the location holds a Git repository before saving it; Browse picks the folder. Everything you change here is saved in Rocket's settings straight away.
- In a workspace's form, tick the repositories it opens. The circle in the Active tab column picks the tab shown once the workspace has opened. A workspace's list marks that repository, and each repository that comes from a nested workspace names it.
- While repositories open, the selected card says which one is opening and how many are left. The result stays at the foot of the card, and a repository that couldn't open shows why in its own card: correct its location with Edit, then open it again.
- Each row shows the repository's branch, its uncommitted changes and where it stands against its upstream: ↑ commits to push, ↓ commits to pull, both when it has diverged. Not checked yet means Rocket hasn't read it on this visit.
- Reading status is local: one git status per repository and the time of its last fetch, four at a time and at most ten seconds each, for the repositories the selection lists. Nothing is fetched and no lock is taken, so the counts are as of the last fetch. Rocket reads again when you open the page, with Check again, and after Fetch all or Pull all.
- Needs attention lists repositories with changes, commits to push or pull, a merge or rebase in progress, a failed fetch, or a folder that is gone or can't be read. A dot on a folder or workspace means one of its repositories already checked needs attention.
- Fetch all fetches every remote of each listed repository, two at a time, by automatic fetch's rules: tags as Preferences → General says, never pruning or merging, and never asking for a password. A remote that needs a sign-in fails instead; open that repository and Fetch to sign in.
- Pull all moves a repository only where Pull would fast-forward it: a branch with an upstream, no changed tracked files and no merge or rebase in progress. New files Git doesn't track yet don't stop it, as with Pull, and Git still refuses to overwrite one. Everything else is skipped and listed, never forced: no merge commit, rebase, stash or reset. A repository with a write running in its tab, or whose folder is gone, is skipped too.
- Fetch all and Pull all act on every repository the selection lists, whatever the filter shows. Stop skips the ones that haven't started: a fetch stops at once, and a pull already running finishes. The result lists failures and skips first, and Open in tabs opens those so you can deal with them. The ×, picking a repository or another selection closes it.
- Removing a workspace moves its children to its parent; it never deletes child workspaces, repository files or open tabs. Names must be unique under the same parent. If removal would create a duplicate, rename or move the child first.
- Existing flat workspaces load at the top level. Nesting saves a newer settings format, which older builds refuse to read. Keep the whole state folder, including its draft archive, together when backing it up.
- Folders organize saved repositories. Removing a repository from Rocket, or a folder, never deletes repositories or working files; a removed folder's repositories become Ungrouped.

### Closing

- Closing a repository saves its commit message draft for the next time you open it. Working files and staged changes stay on disk.
- Drafts support the commit writer's full 1 MiB UTF-8 message limit. An edit above that limit is rejected with an explanation, preserving the previous text. Retained drafts are independent of the presentation-cache limit.
- Close Rocket before backing up its whole state folder, which Preferences → Storage shows: settings are in state.bin and drafts in the drafts-…bin archive it points to. Existing state migrates on save. Older builds refuse a format they don't know instead of losing data.
- If a drafts/settings write fails, keep Rocket open and use Retry save in the status bar after resolving the cause. A failed initial state-file load keeps the original file untouched and cannot be retried as a write.

## Exploring other repositories

### Search

- File → Explore Repositories (Ctrl+Shift+E), the welcome screen's Explore box, the + beside the tabs and the command palette all open Explore. Type words to search GitHub, or paste a repository's address from any host to preview or clone it without a search. A local repository folder works too.
- A search runs only when you press Enter or Search; typing alone sends nothing. GitHub allows 30 searches a minute when you're signed in and 10 without, and Rocket says when only a few are left. When the limit is reached, the results you have stay and Rocket counts down to when you can search again.
- Explore signs in the way the GitHub page does, with the github.com sign-in Git already stores. Without one it searches public repositories only; Sign in adds the private ones your account can see.
- GitHub's own search words work in the box, such as language:rust, stars:>500, user:name or topic:cli. The Language list adds language: for you. GitHub shows the first 1,000 results of any search; narrow the search to see others.
- Size on GitHub is GitHub's estimate of the whole repository with every file version. A preview usually downloads much less.

### Preview

- Preview opens a read-only copy in a new tab, marked Preview. Rocket downloads the history (every commit and folder, without old file versions) into its own cache, then the files of the default branch, so Files, Map and Insights work at once.
- Older file versions download when a view needs them: a commit's changes, a file at another revision, Compare, path history, blame, or the Map on another branch. While they download, the view says how many have arrived and how much, with Stop. Content search in history is off in a preview, because it needs every version of every file.
- Viewing branch at the top left picks the branch Commits shows; clicking a branch in the sidebar does the same. Files and the Map pick their own revision.
- Nothing from the repository runs in a preview: there is no checkout, and no hooks, submodules or Git LFS downloads. Git never fetches by itself behind a read; Rocket asks the repository's host for exactly the files a view needs. A preview has no Changes, terminal, custom commands, external tools or extensions, and nothing in it can be changed.
- A preview stops at 1 GB. GitHub's size warns before a large one starts; an address from another host stops when it reaches the limit. Continue without a limit lifts it for that preview. Some hosts can't leave file versions out; their previews download everything, still within the limit.
- If the host can't be reached, what is already downloaded keeps working, and a view that needs more says Not downloaded yet with Try again. A private repository needs a sign-in Git can use for that host; it is the same one cloning would use. When the host refuses, Sign in asks Git's credential helper to sign in to that host, then tries again. Rocket stores nothing itself.

### Keep and Discard

- Keep makes a normal clone in a folder you choose, with the same checks as Clone. Rocket first downloads the file versions the preview doesn't have yet, then copies it with Rocket's verified clone and points origin at the original address. The kept repository replaces the preview's tab, and Activity notes where it went. Stopping Keep, or a Keep that can't download the rest, leaves the preview as it was; Try again picks up from there.
- Discard, or closing the preview's tab, deletes it straight away, and Activity notes the space freed; it can always be downloaded again. Quitting Rocket deletes every preview. If Rocket closes unexpectedly, the next start deletes previews no running Rocket holds.
- Previews live in the previews folder next to Rocket's state folder (%LOCALAPPDATA%\Rocket\previews; on Linux, ~/.local/state/rocket/previews). They never appear in Recent repositories.

## Commits

- The commit message in Changes has a one-line summary, counted against 72 characters, and a description that opens at 7 lines, or 4 when the window is short. It grows with what you write up to 12 lines, then scrolls. Drag the handle above the summary to make the message area taller or shorter; Rocket remembers that height, and double-clicking the handle goes back to following the text.
- Drag files between the Staged and Unstaged groups in Changes. Ctrl-click toggles files and Shift-click selects a range within one group. Drag a selected file to move that selection; Escape or dropping outside the opposite group cancels. Staging uses current whole-file contents, while unstaging keeps working files intact. Renames include both paths, and conflict rows still require explicit resolution. A refreshed or changed repository requires a fresh drag.
- Amend replaces the last commit with the current index and a new message, so its commit ID changes. Unstaged files stay unchanged, and the original author and parents are kept. Coordinate with collaborators if the commit has already been shared. A recovery reference keeps the original commit, and an amended commit that ends up empty is kept, never dropped. Your regular commit draft is kept separately.
- Cherry-pick adds a copy of the selected commits to your branch, oldest first. Revert adds a commit that undoes each one, newest first, and the originals stay in history. Both are in a commit's right-click menu, in Repository → Commit and in the command palette. The review shows what each new commit changes. Both are checked first in temporary Git storage; your checkout, staging and branches stay unchanged by the check.
- A conflict pauses the work and the banner says which commit it stopped at, with Continue and Abort, and with Skip when other commits come after it. A single commit has no Skip, because Abort would do the same. Once your fix is staged, the main button in Changes becomes Continue revert or Continue cherry-pick. If you commit a fix yourself partway through several commits, Continue carries on with the rest and Abort ends the work without moving your branch. Continue commits the staged resolutions, leaves unstaged edits in the working tree, applies the remaining reviewed commits and pauses again at the next conflict. Skip and Abort discard the operation's conflict resolutions and tracked changes, and the review shows those edits first; untracked files are kept.
- Reverting a merge asks which side stays, and starts on the branch the merge went into, which undoes what came in from the other one. Merging that branch again later won't bring those changes back, because Git counts them as merged already; to get them back, revert the revert.

### Finding code

- The search above the commit list looks in messages, authors and IDs, in changed file paths, or in changed code. It reads 200 commits at a time and pauses after 200 matches; Load more carries on, and after Stop, Resume picks up where it stopped.
- Changed code with exact text finds the commits that change how often the text appears, as git log -S does. A commit that only edits the line around it, such as RETRY_LIMIT = 3 becoming = 5, isn't one of them. Turn on .* to search with a regular expression instead: it finds commits whose added or removed lines match, as git log -G does. Aa makes letters match exactly; with Aa off, the commit search ignores case for A to Z only, while Search in files ignores it for other letters too. In path keeps one file or folder, written as in the repository with / or \, and case matters there. Exact text also fills Find in the selected commit's diff.
- Regular expressions are POSIX extended, as git log -G and git grep -E read them: . * + ? [ ] ( ) | ^ $. When Git can't read one, its reason shows and nothing runs.
- Search in files on the Files page runs git grep. Open it with Ctrl+Shift+F, Repository → History & Recovery → Search in Files, or Search files at this commit in a commit's right-click menu. In: Checkout searches the files on disk now, new files Git doesn't track yet included and ignored files skipped. In: Revision shown searches the commit the folders show.
- Results are grouped by file in path order, with line numbers; files with more than 20 matches start collapsed. Click a line to open the file there with its matches marked, then use ↑ and ↓ to step through every match. Find in commits opens the commit search for the same text in that file.
- Limits keep large repositories responsive: 2,000 matches, 100 per file, 400 characters of each line and 30 seconds. When one is reached, the matches found so far stay and the strip says which limit stopped the search. Stop ends it at once. There's no resume, because git grep can't continue where it stopped, so narrow the search with a path or more text.
- Binary files are skipped. At a commit, a Git LFS file is only a small pointer, so its content isn't searched; a match inside a pointer is labelled. Files over 2 MiB open around the match, 200 lines either side. Text that isn't UTF-8 is searched as its own bytes and shown with replacement marks, also when you just browse it.
- A commit whose file contents aren't downloaded, as in a partial clone, can't be searched here; Rocket says so instead of fetching them.

### Commit message drafts

- Drafting with Claude Code or Codex is off until you turn it on in Edit → Preferences → Commit messages. Draft with Claude or Draft with Codex then shows in the commit box in Changes, and works once something is staged. The first draft with each tool explains exactly what is sent and asks first. Nothing is committed: the draft goes into the message fields for you to edit.
- Check spelling as you type is in Edit → Preferences → Commit messages and uses the Windows spell checker. Spell checking discards obsolete queued text as you edit. Disabling it stays responsive; an accepted dictionary change finishes in the background. Text checks are limited to 128 KiB, matching the saved-message limit.
- On Linux, spelling is checked against the Hunspell dictionaries installed on the system (Ubuntu: hunspell-en-us for English), in your language or else English (US). Words that look like code aren't checked: paths, URLs, file names, identifiers and words with capitals inside. Add to dictionary keeps the word in Rocket's own list for that language, ~/.local/state/rocket/dictionary; other applications don't share it. Ignore lasts until Rocket restarts.

## Branches

- Repository → Branch → Workflows & Cleanup adds Sync and Finish. Sync rebases the current feature on top of a reviewed local base; Finish switches to that base and makes a merge commit. Both branches stay, and fetching, publishing or deleting them remains separate.
- These helpers require a clean checkout, including untracked files. Changed tips, configuration or files invalidate the review. Original tips are retained under refs/rocket/branch-workflow and a phase journal is kept in the checkout's Git directory. Check the observed checkout after cancellation; completed steps are not undone. Use Changes to continue or abort conflicts. After aborting Finish, switch back to the feature if desired.
- Each worktree remembers history scope, branch filtering, sidebar sections, hidden Tags/Stashes, diff layout and graph folding. New worktrees inherit the current view; drafts and later view changes are independent. View → Reset This Worktree’s View resets only the active checkout's presentation.
- Presentation settings are retained for up to 256 worktrees, with active, open and recently opened repositories preferred. Evicting an older view does not remove its draft or its saved repository shortcut.

- New branch starts at the current commit. Creating a branch does not switch to it.
- The branch button in the toolbar lists the current branch first, then local branches newest first, each with when it last changed and how far it is ahead (↑) or behind (↓) its upstream. Type to find a branch; ↑ ↓ move, Enter switches and Esc closes. Manage branches opens the Branches page.
- Remote branches stay folded under one row until you type or open it. Choosing one that a local branch tracks switches to that branch. For one that none tracks, Check out as local opens New branch starting from it and tracking it, and Create and switch checks it out. It happens all at once or not at all: when Git refuses the upstream (no remote's fetch settings cover it, or two do) or the switch (local changes in the way), the new branch is removed again and the reason is shown. A local branch with the same name is never replaced; choose another name.
- Delete branches: Git deletes a branch only when it is merged into its upstream, or into the current branch when it has none, and keeps the others. Remote branches are unaffected. The result lists each deleted branch's last commit.
- Delete branches marks each branch Merged, or says how many of its commits are not merged, by that same rule. Select merged picks the merged ones; Git still checks each again when it deletes.
- Dragging a commit onto another offers to move it above or below, or to squash or fix it up into the target. Each choice opens the interactive rebase review first.

## Remotes

- Repository → Branch → Workflows & Cleanup reviews obsolete tracking refs. Select exactly which to prune; old tips are retained under refs/rocket/remote-prune. Local branches with missing upstreams are listed for inspection and kept. Only standard complete fetch mappings are supported.
- Remote branch deletion pins the exact server tip and uses a lease. A changed tip or server policy rejects it; the default branch is refused, and the server must identify that branch. An interrupted deletion may already have happened: inspect the reported remote observation before retrying.
- Branch workflows & cleanup lists its operations by what they touch: Branches, Remote, Saved work and Storage. Choose one, review it, then apply the operation the button names; changing a choice discards the review. A local branch listed as kept is a candidate for inspection, not proof that it is merged.

- Adding a remote saves it in this repository's configuration. Nothing is downloaded until you fetch.
- Editing a remote URL changes it locally. Ref mappings and other settings are preserved, and no connection is made. Without a separate push URL, pushes use the fetch URL.
- Removing a remote deletes its remote-tracking branches from this repository. Local branches and working files remain, and nothing is deleted from the server.

## Fetch and pull

### Fetch

- Fetch downloads new objects and updates remote-tracking branches. Your local branches and working files stay as they are.
- Automatic fetch runs in the background for open repositories and steps aside when you start an operation. It never prunes, merges or asks for credentials: a remote that needs them shows a quiet failure and is retried at the next interval.
- Preferences → General → Tags in fetch sets where the Fetch dialog starts and which tags automatic fetch brings.

### Pull

- Pull uses the current branch's configured upstream. It fetches first, shows the incoming commits, and then applies the strategy you choose: fast-forward only, merge, or rebase. Fetching alone does not change your branch.
- Commit or stash tracked changes first. Untracked and ignored files that would be overwritten are protected.
- There is no autostash, forced update or automatic retry. Conflicts continue in Changes.
- Rebase moves your local commits on top of the fetched commits, so their commit IDs change; a recovery reference keeps the original tip. Do not rewrite shared work without coordinating.

### Conflict prediction

- Before a pull, rebase, cherry-pick or revert, Rocket checks for content conflicts in temporary Git storage. The check covers file content only. Originally empty commits are kept and newly empty commits pause. Your checkout and branches are unchanged by the check.
- A clean result does not guarantee the operation: hooks, signing and later changes can still stop it. After a predicted conflict you can still start, then resolve the conflict with Continue, Skip or Abort.

## Push

- Push sends one branch to the selected remote branch normally. It never replaces remote history.
- Replacing remote history is a separate, reviewed force push with lease.
- To publish several branches at once, all or nothing, use Push branches together.
- What will be pushed compares your branch with the remote branch as of your last fetch; the remote may have moved since, and Git decides at push time. When the remote has commits you lack, pull first or review a force push with lease.
- Git checks for unpublished submodule commits before publishing the parent. Initialize submodules and fetch their remotes first for a complete check; Rocket never pushes submodules automatically.

## Force push with lease

Force push with lease replaces one remote branch only if it still points to the commit you reviewed.

- Review remote reads the push destination and compares its history with your local branch. Opening the dialog does not fetch or push.
- The review lists the commits that will no longer be reachable from the remote branch. Coordinate with collaborators before replacing shared history.
- Only the reviewed commit is pushed. Your index and working files are not sent, and upstream settings stay unchanged.
- If the remote branch changed after your review, the lease is stale and the push stops.
- Cancelling is not rollback; the remote may already have changed. Inspect the observed result before retrying.

## Push branches together

Push branches together publishes several branches in one atomic push: every selected remote branch is updated, or none is.

- The server must support atomic pushes. A server without support rejects the whole push; Rocket never falls back to separate pushes.
- There is no force, no extra tags and no upstream change. A branch that cannot fast-forward is blocked until you fetch or integrate the remote history.
- Local and remote branch tips are checked again right before sending.
- Cancelling is not rollback. After the Git process stops, Rocket checks every selected remote branch and reports what actually happened.

## Merge

- Squash changes prepares changes without moving HEAD or creating merge continuation state. Resolve any conflicts, then make an ordinary commit; reviewed discard/restore can undo prepared changes.
- Merge without committing leaves a prepared merge for review, even when a fast-forward was possible. Continue commits it; Abort restores the pre-merge checkout.

- Prepare a review first. Editing the branch or the merge mode needs a new review.
- Tracked changes must be committed or stashed. Git protects untracked and ignored files that would be overwritten.
- Hooks and signing settings apply to the merge commit.
- A review does not guarantee a conflict-free merge. Conflicts stay in Changes for you to resolve; then continue the merge from there.
- Clearing a completed merge state keeps HEAD, the index and working files. It does not create another commit or undo the merge.
- The three usual ways to merge are shown as cards. More options holds Squash, Merge without committing and Allow unrelated histories; it opens by itself when a review stops at unrelated histories, and while closed it names a choice made there.
- The review draws both histories: commits only on your branch above, incoming commits below, joined by the new commit. When the branch can simply move forward, it shows one line instead.

## Rebase

- Rebase moves your branch's commits on top of another branch or commit. From is your branch. On top of is the one you choose, which Git calls onto. Rocket makes a new copy of each commit on top of it, then moves your branch to the last copy. The branch you chose doesn't change: merging into it or pushing is a separate step.
- Rebasing changes commit IDs. Coordinate before rewriting shared history. The original tip is kept at a recovery reference.
- A rebase is local: no push, autostash or merge flattening, and other branches move only if you choose to move them with it. A commit that becomes empty pauses for review.
- Choose Keep merge commits to recreate the branch's merge structure on top of what you chose. Previous manual merge resolutions may need to be repeated. This mode supports Continue, Skip and Abort, but not conflict prediction, interactive row editing or dependent-branch updates.
- Branches that are not moved keep pointing at the original commits, which stay reachable from them and from the recovery reference.
- Keep merge commits is under More options in the Rebase dialog. Other branches on the commits that move are listed with whether they move along.

### Interactive rebase

- The plan lists commits oldest first. Move commits with Up and Down.
- Squash combines a commit with the one before it and asks for a new message; fixup keeps the earlier message; edit replays the commit and then stops so you can amend it or split the work.
- Select a row to add a stop or a command after it. Each planned stop appears as a paused rebase; continue it from the banner or the main button in Changes.
- If a command fails or leaves changes, Git stops so you can fix them. Continue moves on without running the command again. Commands run with your permissions in Git's shell at the repository root.
- Dropped commits are omitted from the branch, and reordering can cause conflicts. Review every action and replacement message before starting.

### Continue, skip and abort

- A paused rebase shows which commit it stopped at and what to do next. While the branch is being moved, the window shows it as your branch · rebasing.
- If the branch has moved from where the rebase started, the banner says to finish it with Git. That happens when Stop lands at the very end of a rebase, or when another tool moves the branch during a pause. Continue, Skip and Abort explain why they can't go on, and the original tip stays in a recovery reference.
- While a rebase is moving commits, ours is what your commit goes on top of, with your earlier commits already there, and theirs is the commit being moved.
- Continue needs conflicts resolved and staged first. Once they are, the main button in Changes becomes Continue rebase.
- Skip omits the stopped commit. Later squash or fixup steps may then combine with a different preceding commit, so review the remaining plan. At a planned stop, Skip discards only the tracked changes made since the stop, and the replayed commits are kept.
- Skip and Abort discard the tracked edits shown in their review. The original-tip recovery reference stays.

## Conflicts

- AI resolve asks for a proposal for one file, from Changes or the conflict editor. Choose who answers in Ask for a proposal; the page starts on the first program it finds. It sends the whole file: base, ours, theirs, your editor draft and your instructions. Show exact data shows that text. Rocket stores no AI API key.
- Claude Code and Codex: Send runs the program on this computer once, from an empty folder, with no tools or project settings. Claude Code sends to Anthropic under your Claude account, and your Claude plan or API usage applies. Codex sends to OpenAI under the account it is signed in with, together with your global AGENTS.md if you have one. Rocket uses the program set under AI drafts in Preferences → Commit messages or chosen with Browse on this page, otherwise claude.exe in your user folder's .local\bin, or the Codex it finds. Found means the program is there; sign-in shows only once you send. Stop ends it; a request stops after 5 minutes.
- Connected agent: Share with agent makes this one snapshot available for 15 minutes through Rocket's authenticated localhost server. Connect your AI app in Preferences → MCP server and copy the agent instructions. The connected app's provider may receive the shared code and charge for model usage.
- Proposals include one resolution per conflict, an explanation and an estimated confidence. Changed lines are green; click a conflict to move both panes to it. Ask again drops the proposal and keeps your instructions. Apply to editor draft does not write or stage: each conflict lands in the editor as resolved, marked AI, with its own Undo. Save file / Save and stage retain the usual backups and external-file/context checks. Stop revokes a shared request; late, incomplete or malformed proposals are rejected. Confidence is not proof of correctness: inspect and test the result.

- In the result, each conflict is tinted (ours blue, theirs violet) beside its line numbers, and its first line offers Use both, plus Use ours and Use theirs when the side panes are hidden. ‹ › or Alt+↑/↓ move between conflicts; an amber bar marks the selected one. You can also edit the text directly.
- In a wide window, Ours, Result and Theirs sit side by side and each pane's header says what it is: during a merge, Ours is your branch (HEAD) and Theirs the incoming branch; during a rebase, Ours is what your commit goes on top of and Theirs your commit; during a commit series import, Ours is your branch and Theirs the patch. Each conflict has the same number in all three panes, and a band joins its lines across the gaps. Choose Use ours on the Ours lines or Use theirs on the Theirs lines; once a conflict is resolved, each side says Used or Not used. Text in Ours and Theirs can be selected and copied.
- A conflict resolved with one of those choices stays tinted green, with its name and Undo on its first line; Undo puts the original conflict back and selects it. Typing elsewhere keeps Undo. Typing on the resolved lines ends it, and so do Reload, a merge-tool result and a failed save. While Undo is available, the header counts what is left, such as 1 of 2 left.
- The result pane's overview marks unresolved conflicts in amber and choices resolved in this draft in green. Its blue outline shows the viewport. Click a conflict to select it, or drag the rail to scroll. Linked scrolling moves the side panes; unlinked scrolling moves Result only. Focus the rail for arrows, Page Up/Down, Home or End. Navigation does not resolve or save anything.
- The conflict editor writes your result into the working file. Save and stage also stages this path only. Neither commits nor continues the merge or rebase.
- Before writing, Rocket checks the file and its conflict stages again and keeps recovery copies in the repository's Git directory. Dismissing the confirmation writes nothing. Cancelling an in-flight save stops remaining work; a file already replaced stays changed, with recovery and staging status reported.
- Ours is Git index stage 2, theirs is stage 3 and the base is stage 1. During a rebase these are Git's index sides (see Rebase).
- When Git cannot merge a file line by line, for example a binary file or a file deleted on one side, keep one whole version. It replaces the working file and resolves only this path's conflict in the index.
- LFS conflicts can keep a verified local object and stage its exact pointer. Missing or corrupt objects cannot be selected. Fetch them separately, then reload. Standard filter=lfs with -text is supported up to 16 MiB; other conversions require Git.
- Submodule conflicts change only the parent's recorded commit or deletion. Child checkouts, local commits and dirty or nested files stay untouched. The child may still appear modified; review any checkout update separately in Submodules.
- LFS and submodule resolutions also retain the original index and stages. Split indexes or indexes over 64 MiB are refused before writing; resolve those with Git.
- External merge tools work on temporary copies. Stopping a tool keeps those copies, but changes the tool has already saved stay.
- Reloading a conflict replaces your editor draft with the working file and conflict stages; copy any text you want to keep first. Closing the repository or exiting Rocket discards unsaved conflict-editor text.

## Stashes

- Apply selected on a saved stash opens Apply part of a stash for the current section (All tracked, Saved index or Untracked). Tick whole files in the list, or pick a file and tick its changes or single lines. Each choice is checked against your current files at once; Apply selected changes applies only a checked selection. The original stash and staging are kept. If files or HEAD change after the check, Apply is refused; Check again checks the same selection anew.
- Overlapping edits stop the check. New files need existing parent folders. File recovery copies are kept; an interrupted multi-file save reports completed files instead of claiming rollback. Binary deltas, executable-mode changes, metadata files, custom filters and non-regular files need the full stash/Git workflow. Single changes and lines require regular text changes with unchanged mode and a final newline; other files apply whole.

- Snapshot on the Stashes page saves staged and working versions without cleaning files or changing the index. Include untracked files explicitly; ignored files stay excluded. The reviewed files, checkout, index and stash tip are checked again before saving. A recovery ref is published with the stash. After cancellation, read the reported publication state before retrying or applying it.
- Snapshot supports normal files up to 16 MiB each, 256 paths and 64 MiB total. Resolve conflicts and dirty submodules first. Custom filters, encoding conversions, sparse/hidden index entries and split indexes need a normal checkout or Git outside this workflow.

- Rename stash in Repository → Branch → Workflows & Cleanup changes the display name while retaining its commit and position. The previous file-backed reflog is backed up; reftable is not supported for rename.
- Stash + pull/rebase explicitly saves tracked changes, integrates a reviewed target and reapplies the staged/unstaged split. Pull review fetches first. Untracked files stay in place. The stash is always retained; inspect before dropping it.
- A stopped or conflicted workflow records its phase and stash ID under .git/rocket-saved-work. Finish or abort active integration, then inspect and explicitly apply the retained stash. Do not repeat the whole workflow blindly or apply a stash twice.

- Saving a stash records your changes and then cleans them from the working tree. Ignored files are excluded.
- You can save only selected files. Staged files are always saved, because Git keeps the whole index in a stash, and unselected changes stay in the working tree. Git can stash a staged deletion, rename or conflict only together with every other change.
- Applying keeps the stash, including when conflicts occur. Review Changes before retrying a failed apply.
- Dropping a stash can make its changes unrecoverable. Working files stay unchanged. If the stash list changed since you selected the stash, the drop is rejected; refresh and select it again.

## Reflog and recovery

### Undo the last local commit

- After an ordinary Commit in this Rocket session, use Edit > Undo Last Commit, Summary > Undo last commit or the command palette. The dialog shows the commit being undone, the commit the branch moves back to and what it checked; Copy recovery name copies the recovery reference. Then choose Undo commit.
- Undo moves the same branch back to that commit's parent and leaves the index and working files untouched. The committed changes become staged again, alongside any newer unstaged/untracked files. Undoing a first commit returns the branch to having no commits.
- The original commit is retained under the shown refs/rocket/undo reference before the branch moves. Activity records that reference. Reflog and recovery can make a recovery branch; do not delete recovery data merely to clear an error.
- This first Undo supports only the latest ordinary commit made here in the current session. Another Rocket write clears the offer. Restarting does not retain the offer. Amend, merge, Commit & Push and earlier operations use their existing recovery tools; this is not a multi-step Undo/Redo history. Ctrl+Z keeps its text-editing meaning.
- Undo refuses a changed branch/tip, later staging changes, an active Git operation, another Git lock or an external edit after review. Git refuses a review older than two minutes, so an open dialog checks again by itself before then; after a refusal, Check again reviews it anew. Dirty working-file previews are bounded to 16 MiB and refuse changed submodules or special files.
- Only Rocket's copies of remote branches are checked for publication. Don't undo a commit you shared some other way; use Revert instead.
- A commit contained in a known remote-tracking branch is refused. No server is queried, so a commit shared through another clone may not be detected; use Revert for shared history.
- Stop cancels work still in progress, then checks the actual branch, index and recovery reference. A branch move already completed is reported as completed even if Stop interrupted a later hook. An unconfirmed result needs inspection, not an automatic retry.

### Earlier history and other operations

- Reflog and recovery lists reflog entries, deleted branches, kept and unreachable commits, newest first. Actions use full commit IDs.
- Reflogs can expire, and deleting a branch removes its own log. A lost commit can still be recovered if another retained log contains it.
- Deleted branches lists branches this worktree switched away from that no longer exist, at the tip each had when you left it. Later commits to such a branch, and branches never checked out here, are not listed.
- Unreachable reads the local object database to find commits no ref or reflog reaches. It never prunes objects or changes your checkout; commits that were already pruned cannot be recovered.
- A recovery branch is a new local branch at the selected commit. Your current branch, index and working files stay in place.
- Reset moves the current branch to the selected commit and changes local history; coordinate before rewriting shared history. The original-tip reference protects committed history only.
- Mixed reset requires no staged changes. Hard reset also requires clean tracked files and no untracked or ignored files in the way.
- Entries no branch reaches are marked Not on any branch. Select an entry to see how it compares with your branch, then Make a branch here or Move your branch here; Keep them staged, Keep them unstaged and Replace them are soft, mixed and hard reset. Each is reviewed before it runs.

## Tags

- Creating a tag never replaces an existing one. The reviewed target commit is pinned, even if its branch moves.
- Tag controls also work when the opened repository is bare. Commands that change working files remain unavailable until you open a worktree.
- An annotated tag's preview pins Git's tagger identity and time. A signed tag's object ID is known only after signing, and the reviewed signing settings must still match.
- Deleting a tag keeps the exact tag object at a recovery reference. Only the local tag is deleted; fetching may bring it back from a remote.
- Batch tag actions check each tag again right before it changes. Cancelling stops before the next tag; completed changes stay, and the result lists each tag.
- Tags are listed newest first: by tagger date, or by the commit's date for a lightweight tag. Name sorts them by name. The filter searches the tags already loaded; Show more loads up to 2,000.
- "Signed · not checked" means the tag object carries a signature; Verify signature checks it. Raw tag object shows the tag exactly as Git stores it.
- Publish to a remote from a tag's details opens the remote tab, reads the remote and keeps the tag selected. Publishing still has its own review.

### Remote tags

- Inspecting a remote reads the tag IDs it advertises. It does not download annotations or verify signatures; local annotations are under Local tags.
- Publishing never replaces a remote tag whose object differs from the local one. An indirect (symbolic) remote tag is never published or deleted, because Git could change its target branch instead.
- Deleting a remote tag removes it only if it still matches the reviewed object. Local tags are kept. There is no remote undo or backup, and other collaborators may still have copies.
- Git uses your configured authentication helpers and hooks. Cancelling after transmission may leave the remote changed; inspect the result before retrying.
- The remote tab contacts the remote only when you choose Read tags or Read again. Each tag then shows where it stands: only here, the same on both, different on the remote, only on the remote, or indirect. The count buttons show one kind at a time.
- Where it points gives the commit on each side. A commit on the remote is named only if this repository already has it; Rocket doesn't download anything to find out.
- The read stays while you look at This repository. If tags change here, the tab says so, and publishing and deleting wait for Read again.
- After a verified publish or delete, the tag's row updates without another read. If Rocket can't verify the result, it says so, and changes wait for Read again.
- Publish and Delete show only for tags they can act on. To replace a tag that differs, delete it from the remote, then publish yours.

## Worktrees

- A new worktree is prepared in a sibling folder and then published without replacing the destination. Existing paths, and branches checked out in another worktree, are refused. If creation fails, the preparation data is kept for inspection.
- From a bare repository, create and open a linked worktree to edit files. The bare repository itself cannot be removed through worktree controls.
- Before removal, Rocket checks that there are no tracked edits, staged, untracked or ignored files, active operation, submodules or hidden index flags, and checks again right before removing.
- Removing a worktree keeps its branch, and its committed tip stays at a recovery reference. Close editors using the folder; there is no force option. Cancelling after removal starts is not an undo.
- The main worktree holds the repository and cannot be removed. A worktree that is open in Rocket must be closed first.
- Each row shows the worktree's branch, whether its folder has changes, and its last commit. A new worktree's folder is suggested next to the repository from the branch name until you change it.

### Move, repair and prune

- Move needs a clean folder: staged, unstaged, untracked and ignored files block it. The whole folder moves without replacing an existing path; close editors using it first. Registration and metadata are checked again before applying, and a recovery copy is kept in Git's common folder. A move that has started finishes updating its links even if you cancel.
- Repair reconnects a missing registration to its moved folder; the folder's .git file must identify that registration. Working files and staging are preserved. Repairing a moved main repository or a broken .git file requires Git.
- Prune removes only the registration of a folder that was deleted. Its metadata and committed tip are kept for recovery, and no working files are deleted. If the folder was moved instead, use Repair.
- In a worktree's details, Find moved folder is Repair and Forget it is Prune; they appear only when its folder is missing.

## Agents

- A session runs Claude Code, Codex, GitHub Copilot or your own program in its own worktree, on its own branch, with a terminal of its own. Install the program and sign in to it yourself: Rocket uses your existing login, and does not install agents or store an API key.
- A program Rocket does not find shows Not found, and the dialog opens the field for its path. Type the path, or use Browse. A program chosen with Browse is where Rocket finds it from then on: for Claude Code and Codex that is the Program setting under Edit → Preferences → Commit messages → AI drafts, which AI drafts, AI review and AI resolve use too. A path you type applies to that session only.
- Your checkout stays as it is. Its staged, unstaged, untracked and ignored files are not copied into the new worktree, and neither are credentials or environment files.
- A worktree separates files, not permissions. The program runs with your account's access and keeps its own permission prompts; Rocket never adds an option that bypasses them. Arguments are saved with the session, so never put a password or token in them. Terminal input and output are not saved or logged.
- Review session checks the base commit, the folder, the program's SHA-256 and its arguments, and holds the program's file until the session ends. A program that changed since the review needs a new review. Arguments are passed as written, one per line, with no shell.
- A setup command is optional and runs first. The agent starts only after it exits successfully, and a failed setup keeps the worktree for you to look at. It is off again on every restart until you choose it.
- The list groups sessions by what they need from you: Needs you, Running and Not running. A group folds when you click its heading; searching shows every match.
- Stop ends the program and keeps the worktree and its changes. Closing Rocket stops its sessions; reopening restores them without starting anything.

### Activity and resuming

- Track conversation activity lets the program report when it is working, has finished a turn or is waiting for you. It does not change the program's own configuration or login. Codex may ask you to review these hooks in /hooks.
- These are the program's reports. A finished turn or an exit code of zero does not mean the work is correct. A program that reports nothing shows Activity not connected.
- A session that waits for you is listed under Needs you until you mark it as seen or it starts working again.
- Resume conversation reopens the same conversation after checking the worktree, branch, program and arguments again. It never runs setup and never picks another conversation. Review and restart starts a new one.

### Bringing the work back

- Commit what you want to keep in the worktree: Open worktree shows it in a tab of its own. Rocket commits nothing for you.
- Check integration needs the session stopped, both checkouts clean and the original checkout on a branch. It names the branch that will receive the work and how many commits come in. Nothing is saved, stashed or discarded for you.
- Compare committed changes opens the comparison. Review merge opens the usual merge review; applying it is your decision, checks both branch tips again and uses the conflict editor if needed. The session's branch and worktree stay, and nothing is pushed.

### Removing

- Remove worktree works only on a clean, unlocked worktree, and checks again right before removing. The branch and a recovery reference stay.
- Forget session removes only Rocket's record of it. The folder and the branch stay; manage them under Worktrees.

## Submodules

- Move / rename moves a clean submodule within its immediate parent repository. Choose an unused path with an existing parent folder. Close tabs for the moved subtree first. Initialized nested checkouts move with it and their Git links are updated; module names, URLs and history stay. The gitlink path and .gitmodules are staged, while unrelated local work stays.
- Dirty or ignored files, existing metadata drafts, redirected paths, embedded .git directories and occupied destinations stop the move. Recovery metadata is saved first. Cancellation stops preparation; once the directory moves, the prepared filesystem updates finish before observation. After a partial disk/lock failure, inspect completed steps and MOVE-RECOVERY.md before retrying or restoring anything.

- Edit URL reviews and stages the .gitmodules URL, then syncs the affected local configuration. Existing .gitmodules edits must be handled first. Unrelated staging and dirty child files are preserved, and metadata backups are retained. Nested modules are edited separately.
- Branch pushes check for unpublished child commits without pushing children automatically. The check uses locally available remote information; initialize and fetch submodules for a complete local assessment. A missing checkout is not proof that its commits are published.

- Submodule operations need clean local files in the submodule and in its initialized nested modules. The source, the parent index and the checkout are checked again before writing, and the parent's staging stays unchanged.
- Nested modules are reviewed separately. A new parent checkout may reveal more nested entries.
- Adding prepares the submodule separately and then publishes it into the new folder. Nested modules are initialized separately, and unrelated staged and working files stay unchanged.
- Removing moves the clean checkout into recovery storage under the parent's Git directory. Local commits and Git metadata are kept. Close tools using the folder; there is no force or recursive deletion.
- Sync updates only repository-local URL settings: no fetch, checkout or staging. Local file edits and push URLs stay unchanged, and nested modules are synced separately.
- Cancelling stops Git; completed steps, preparation data and recovery copies may remain. Inspect the result before retrying.
- Every submodule review shows the submodule first, then each change: the old value struck through above the new one. Nothing changes until you choose the operation's button; editing an input asks for the review again.

## Git LFS

- Repository → Branch → Workflows & Cleanup → LFS cache reviews objects and bytes before cleanup. Tracked files must be clean, with one worktree and the default cache. Current, recent, stashed and unpushed objects are retained. Remote verification is repeated before deletion; local recovery copies are independently verified and held open. Cancellation keeps remaining objects and reports the count removed.
- One LFS cache review covers up to 1,000 cleanup candidates of up to 512 MiB each. The dialog lists the first 100; deleting covers every reviewed object.
- On Linux, Git LFS 3.8.1 or later must be installed beside Git. Cleanup removes an object only while it is still the file Rocket checked. Linux can't keep another program from removing a recovery copy during cleanup, as Windows can.

- Git LFS 3.8.1 or newer is required. Update an older installed helper, then refresh.

### Objects

- The objects list reads the current index; file contents are not hashed while listing.
- Fetch downloads a commit's objects into the LFS cache. Files and staging are preserved.
- Upload offers the objects in one commit's file tree, HEAD's from the page. Objects used only in older commits are not included, and the server may already have some. Publish the branch separately. Upload progress counts objects in the current phase; completion is checked separately.
- Checking out a file verifies its SHA-256 and size, keeps the original pointer and preserves staging. Only cached, unchanged pointers can be checked out. The file and index are checked again before writing, and recovery data is kept if writing is interrupted.
- Git LFS may save its format marker and the endpoint's authentication mode in the local config; credentials come from your Git or SSH setup. Objects that finished downloading or uploading stay after cancellation or failure. Before an upload, the destination, HEAD and staging are checked again; working files are preserved.

### Tracking rules

- Rules are saved only in the root .gitattributes file; nested, global and info attributes may override them. All other bytes of the file are kept, and a recovery copy is saved before writing. Appending to a file without a final newline adds a line separator.
- Tracking changes future staging only. Stage .gitattributes and the intended file conversions yourself; existing pointers and history are not converted.
- Untracking removes the filter=lfs, diff=lfs, merge=lfs and -text fields from the selected line; other fields stay. It does not turn existing pointers back into files, and overlapping rules may still apply.
- The attributes file, index and repository state are checked again before saving. If a rule change is cancelled after the original was kept, inspect the reported phase; it may need recovery.

### Locks

- Locks are shared by the repository; ownership is checked against your signed-in account.
- Locking can make a read-only file writable. Removing another account's lock needs the server's permission; coordinate with its owner first.
- The lock, owner, endpoint, repository and file are checked again before sending. Cancelling after sending is not rollback: an uncertain result needs an explicit refresh, and Rocket does not retry automatically.

## Patches

- Export saves staged changes, unstaged changes (untracked files excluded) or a commit's diff to a new patch file; existing output files are never overwritten. A commit export saves the diff only, not the author or a new commit.
- Export Commit Series saves the commits from First to Last, both included, as numbered patch files, oldest first, with their authors, dates and messages. Pick each end from its list, type a commit ID, branch or tag, or right-click selected commits in Commits and choose Export as commit series. Choose a new folder. A series is one straight line of up to 64 commits with no merges, up to 16 MiB per patch and 64 MiB in all, and can't include a repository's first commit. Empty commits are kept. Working files, staging and checkout stay unchanged.
- Series refs and prepared files are checked again before publication. An interrupted export may retain a preparation folder: the result states its location and whether the final folder was published. Inspect it before cleanup; cancellation is not rollback. Git mail-patch message-delimiter limitations still apply.
- Import Commit Series makes each numbered patch in a folder into a commit, keeping authors, dates and messages. Review the folder and its patches first; tracked edits must be committed or stashed. This version accepts consecutive numbered text patches, renames, executable modes and empty commits, up to the same series limits; binary mail patches, links, submodules and cover letters are refused.
- A paused import shows which patch it stopped at and offers Continue, Skip and Abort, from the banner or the command palette. Resolve and stage conflicts before Continue; once they're staged, the main button in Changes becomes Continue import. If nothing is staged for a patch, Git has nothing to save: stage what it should change, or Skip it. Abort lists every patch: what comes off, where it stopped and what never arrived. Your patch files stay as they are, so you can import them again. Skip and Abort retain recovery references and copies of tracked resolutions plus the staged patch; read the reported backup folder's manifest before manually restoring files. Cancellation stops Git and checks what completed; it cannot undo commits already created. Externally modified import metadata requires manual Git inspection.
- Import accepts regular UTF-8 text and bounded Git literal binary patches, renames and executable modes: up to 100 files, 16 MiB per patch or file and 64 MiB total, into existing parent folders. Binary delta patches, links, submodules, custom filters and Git-configuration paths need Git or an external tool.
- Pasted text uses LF line endings, because Windows converts copied CRLF line endings. For a patch that changes CRLF lines, import the patch file instead.
- Every target is checked again before starting and backed up before it is replaced. If an import is interrupted, inspect the changed-file list and the recovery copies; there is no automatic rollback.
- Selective discard removes only the selected lines; unselected changes stay.
- An import review lists the patch's files as changed, new or deleted, and shows each file's changes as the patch describes them, with changed words highlighted.
- Export counts the files with unstaged and staged changes. A commit to export can be picked from recent history or typed as an ID, branch or tag.
- Stash selected changes saves the chosen lines as a stash under the message you give (it starts as the file's name), then takes them out of the working file. The review marks the changes that go into the stash; editing the message needs a new review.

## Restoring files

- Restoring or removing a file changes only the working file. Staging and commits stay as they are.
- A recovery copy is kept before every replacement or removal. File backups lists them, newest first.
- Paths are exact repository-relative paths, and the parent folder must exist.
- Cancelling waits for the outcome; it cannot undo a replacement that has already completed.

## Ignore rules

- Rules are written only to the root .gitignore; nested .gitignore files can override them.
- Ignore files shows the rules with the lines the file does not have yet in green. Add rules from new files, an exact path or a starter template, review, then save. Stop tracking files opens the separate untrack preview described below.
- Editing rules alone keeps tracked files tracked. Use Ignore tracked files for a separate preview that appends exact rules and removes only reviewed files from the index. Local files stay. Selected staged changes must be committed or unstaged first; unrelated staging is retained. The root .gitignore is not staged automatically.
- Index and rule backups are saved before untracking. If nested ignore rules override the new rules, the index stays unchanged and the result clearly says whether rules were already saved. Inspect recovery information before restoring over newer work.

## Bisect

- Start with a known good commit and a later bad commit. Both must exist locally, and the good commit must be an ancestor of the bad one.
- Commit or stash tracked edits first. Each checkout protects untracked and ignored files. Rocket does not change branch history.
- At each step, run your test in the terminal and record the result: Good means the bug is absent, Bad means it is present, and Skip means this revision cannot be tested. Skipping can make the final result inconclusive.
- When you are done, restore the original checkout. Restoring stops if there are new local edits or the original branch has moved.
- To automate testing, enable Run a test command automatically. Choose a trusted executable outside the working folder, enter one literal argument per line and review the range, command and per-revision timeout before running. Commands have your account permissions.
- Exit 0 means Good, 1–124 means Bad and 125 means Skip. Other codes, timeouts and cancellation stop without classifying that revision. Rocket checks for tracked edits before accepting each result and never deletes generated files. Preserve edits before resuming or restoring.
- Test summaries survive reopening; raw output is bounded and kept only in the open dialog. After interruption, review again to resume testing or use the manual controls.

## Faster history

- In a large repository, Git spends most of a History refresh putting the commits in order. Git can keep an index of the commits (its commit-graph) that makes this several times faster. Rocket offers it above the History list when ordering took more than a second and the repository has none. Repository → History & Recovery → Speed Up History opens it at any time.
- Writing the index adds files under .git/objects/info and nothing else: commits, branches, working files and remotes stay as they are. Git reads the index by itself from then on, in Rocket and on the command line. It takes about 60 bytes for each commit.
- Git reads every commit once to write it, which takes seconds in most repositories and longer in very large ones. Stop ends Rocket's write and preserves commits and working files. Rocket keeps Git locks and temporary graph files because an external Git writer may own them. If a later attempt reports an interrupted-writer lock, first confirm no Git process is still writing; only then remove the named commit-graph lock. Do not delete unrelated Git locks. A fetch-setting change made before Stop remains saved.
- Commits made or fetched later are not in the index until it is updated. Git still shows them; only that part goes without the speed-up. Keep it up to date when Rocket fetches adds the fetched commits after each fetch Rocket runs in this repository, and Update index adds what is missing on request. The choice is stored in the repository's own config as rocket.commitGraphOnFetch.
- Remove index deletes those files again. Git writes no index for a shallow clone, and reads none while the repository has replaced or grafted commits or its core.commitGraph setting is off; the dialog says so instead of writing one.
- The × on the notice hides it for that repository. The menu entry stays.

## Git Flow

- Git Flow settings are written to this repository's own config, where the git-flow command line and Fork read them too. Initializing does not switch branches or push.
- Starting a branch creates it locally. Nothing is pushed.
- If a merge stops with conflicts during Finish, Rocket stops there and leaves the merge in Changes. Resolve it, choose Continue merge, then Finish again: completed steps are skipped. Hooks and signing settings apply to the merges.

## Signing

- A verification result describes that object and the trust settings used for that check. Verify again after changing keys or trust settings.
- Signing settings are saved in this repository's config. Existing keys, trust files and global settings are not modified.
- Use an independently checked trust file. Rocket reads it and never adds trusted keys automatically.
- New tags have an explicit Sign tag choice. Unlock keys through your signer or agent; Rocket does not ask for passphrases.

## Custom commands

- Edit → Preferences → Custom commands lists your commands under the menu each one shows in: the Repository menu, or the right-click menu of commits, files, branches or submodules. Repository → Custom Commands → Edit Custom Commands opens the same page. Click a command to edit it under its row; Find narrows the list by name, submenu, program or arguments.
- Save commands in the footer keeps your changes and Discard drops them; Preferences stays open while changes are unsaved. Duplicate copies a command and Delete removes it. Check looks for the program on disk only when you click it.
- A submenu groups commands in their menu; use / for nested submenus. Asks first lists the questions a command asks before each run: text, branch, file and checkbox inputs use {input:key}. Input values become literal arguments after one expansion. A checkbox occupies its whole argument line and either inserts its checked value or omits that line. The Run review shows the exact result; per-run values are not saved.
- JSON import reviews programs, arguments and defaults before adding definitions to the editor; Save commands keeps them. Import never executes a command. Export writes a new file and refuses existing filenames. Review local paths and defaults before sharing. At most 20 commands and eight inputs per command are supported.

- Each line in Arguments is one argument, passed without quoting or a shell.
- {repo}, {branch}, {commit} and {file} are replaced by the repository folder, the current branch, the selected commit and the selected file.
- To run a script, choose cmd.exe or powershell.exe as the program. That shell then interprets its arguments, including replaced values such as branch names.
- Every run shows the exact command first and runs in the repository folder. Output appears in Activity. Stop ends the program and everything it started; programs it opens, such as an editor, keep running.
- In the editor, Insert puts a placeholder at the cursor; a checkbox's placeholder gets a line of its own. Runs as shows the command line with each input's default.
- Import marks each definition New or Same name. One with the same group and name as a command you already have is skipped; rename one of them to keep both. Export says before you click whether the file is new.
- A required input that the arguments use must be filled before Run. What will run shows each argument exactly as it is passed.

## Issue links

- Open Repository → Settings → Repository Settings and choose Issue links. Issue links turn issue IDs in commit messages into links, for Jira, Redmine or any tracker with https pages. Rules are saved in this repository's Git config (rocket.issuelink.*). A rule with the same name as an inherited one replaces it.
- Patterns support literals, . [a-z] [^…] \d \w \s \b ^ $ ( ) (?: ) | * + ? {n,m}, and a leading (?i) to ignore case.
- A link is an https URL where {id} is the whole match and {1} to {9} are its groups.

## Hosting and pull requests

Accounts live in Edit → Preferences → Accounts. On the Hosting page, Accounts
beside the sign-in status opens them; after an account connects, or when you
close Preferences, you return to Hosting. Cloud services and self-hosted servers
are listed separately; choosing another provider starts a new account.

Each service needs its own kind of token:

- GitHub: read access to repository metadata, issues and pull requests; write
  access to create, review or merge them. Sign in with browser uses Git
  Credential Manager and GitHub's consent screen.
- GitHub Enterprise Server: a token issued by that server and its HTTPS address
  without /api/v3 or a repository path. Server policy controls access; GitHub.com
  browser sign-in and repository publishing are separate.
- GitLab: read_api to browse; api to create merge requests.
- Bitbucket Cloud: an Atlassian API token with repository and pull request read
  access, plus write access to create. App passwords are no longer supported.
  Browser sign-in uses Git Credential Manager; its one-hour session is not saved,
  and Remember applies only to API tokens.
- Bitbucket Server / Data Center: a personal HTTP access token. Include any context
  path in the HTTPS server URL. Username is optional and must match the token's
  account. Repository is PROJECT/repository or ~user/repository.
- Azure DevOps Services: Code (Read) and Work Items (Read); Code (Read & write) to
  create pull requests. Organization policy may require another method.
- Gitea: repository and issue read access, plus repository write access to create.

PR Details and reviews is available for all hosting providers. Read the
loaded commits/files/discussion, then Write review and review the preview
before submitting. GitLab supports comments and approvals; Bitbucket and Azure
also support change-request votes. Their votes cannot include a message, so
send a separate comment if needed. Gitea accepts review messages. Available merge
methods are shown in the preview; server policy is always authoritative.

GitHub.com and Enterprise also support multiline inline comments and discussion
thread replies, resolution and reopening. Select the first and last line in the
same hunk/side, or open Discussion → Load / refresh threads. A changed commit
or thread requires a new preview. A reply draft must be sent or explicitly
discarded before switching to a resolution action.

Draft text and uncertain review/merge/thread outcomes are protected for your
Windows user (on Linux, encrypted with Rocket's key in your desktop keyring) and
retained across restarts. Saved drafts restores them after
connecting the same account. Restored line comments require their original
revision, and previews are never restored or submitted automatically. If a write
was interrupted, refresh and inspect the PR on the server before acknowledging
the result. An error saving recovery data blocks sending the request.

- Azure DevOps Server: choose it under Self-hosted in Accounts, enter the HTTPS
  collection URL (including any virtual directory), and use a PAT issued by that
  server. Select REST API 6.0 for Server 2020, 7.0 for 2022 or 7.1 for 2022.1+.
  Connect verifies an active user, not anonymous API availability. Remember
  protects the PAT and version separately from Azure DevOps Services profiles.
- Enter project/repository for Server PR lists and reviewed PR creation. Work
  items cover the project and use its exact state names. Code Read and Work Items
  Read permit browsing; Code Read & write permits PR creation, subject to policy.
  Changing collection/version requires reconnecting. Reads retain previous results
  on cancellation; interrupted creation requires checking the remote outcome.
  Windows authentication/browser SSO, work-item editing and GitHub-style inline
  reviews/merging are outside this adapter. TLS checks remain enabled. Local mock
  tests do not establish live Server/PAT/CA/proxy compatibility.

- GitHub Enterprise Server: choose that provider in Accounts, enter its HTTPS
  server address (optional port, no /api/v3 or repository path), and connect with
  a personal access token issued by the server. Remember uses a separate protected
  profile. The server's account and reported version are shown after verification.
  GitHub.com browser sign-in and new-repository publishing are not GHES features.
- GHES supports the PR/issue lists, server search and reviewed PR workflows below.
  It uses REST API version 2022-11-28. Missing capabilities and permissions produce
  visible errors; nothing is retried with a different API version. Certificate
  verification stays enabled and redirects are refused. Changing the server
  invalidates the session. Local mock testing does not establish live GHES/SSO
  compatibility; Git credentials, SSH keys and local repository files are separate.

- On a GitHub PR row, choose Details and reviews. Overview, Commits, Files,
  Discussion, Reviews and Checks use bounded remote data. Unicode renames keep
  their old paths; unavailable patches and checks are explicit. Check results
  apply to the loaded head and do not evaluate required branch rules.
- Write a review, choose Comment, Approve or Request changes, then Review before
  sending. Submit rechecks the PR and pins the review to the shown commit.
  A changed head/base needs refreshed details and a new preview. Your draft stays
  intact on failure. Review previews expire after two minutes.
- Cancelling a possible submission does not undo it. Refresh the same PR's reviews
  and check GitHub before acknowledging an unknown result. Rocket never retries
  automatically. Drafts and uncertainty state survive restarts in protected local
  storage; after restarting, refresh and inspect GitHub before trying again.
- In Files, click a left (old) or right (new) line number to add an inline comment.
  Save to review draft, then edit/remove comments in the composer. Submit up to
  32 comments as one review. These drafts are stored locally, not as server-side
  pending reviews. Refresh never relocates a stale comment; review its saved text
  and re-select the location.
- Merge on provider offers GitHub merge commit, squash and rebase. Submit/discard review
  drafts first, then Review merge. Confirm the target and commit IDs. Rocket
  rechecks permission, method and clean mergeability and sends the reviewed head
  condition. The base can still change after its final check. Local files stay
  unchanged; no branch is deleted. GitHub enforces the merge rules.
- An interrupted merge may have succeeded. Refresh that same PR and inspect
  GitHub; a merged state resolves the uncertainty, otherwise acknowledge before
  another preview. No automatic retry, auto-merge or merge queue is included.
- For a cross-fork PR, Create pull request → Use a different source repository
  (fork). Repository is the target; Source repository is the existing fork.
  Push both branches beforehand. Preview checks the fork network and both tips;
  creation cannot atomically lock them. Inspect an uncertain result before retrying.

- GitHub supports Sign in with browser through installed Git Credential Manager. Review GitHub's consent screen. GCM credential storage is disabled for this invocation; Rocket keeps the token in memory unless Remember is selected. Stopping a helper cannot revoke consent already granted in the browser. Other providers keep their token flow.

- Hosting accounts are separate from Git credentials and SSH keys. An account you do not choose to remember stays in the current Rocket session only.
- In pull request and issue lists, state filters query the provider. GitHub also supports a separate server search by text, author and assignee, including further result pages. Partial results and GitHub's 1,000-result cap are shown explicitly. Press Enter in a search field or choose Search. Recent successful searches are saved on this device for the server, repository and list kind; the clock button beside the search lists them, and Clear recent searches ends that list. The local text filter still searches only the pages already loaded. Other providers keep their existing lists and filters.
- Pull requests are created between branches already pushed to the repository. Rocket does not push, merge or change local files. Both branch IDs are checked again before submission, and a review expires after two minutes.
- Azure DevOps work items belong to the whole project, across repositories, and state names are project-defined. Bitbucket Cloud retired its native issue tracker in August 2026; pull requests remain available.
- Reading GitHub notifications in Rocket does not mark them read.
- A problem names what failed and what to do. Its code, such as hosting_invalid_pagination, identifies it in diagnostics and support requests.
- New pull request offers local branches for From and the branch names the repository knows for Into. The review shows both branches at the commits the server reported.

## CI checks

- When a repository's remote is on GitHub, GitHub Enterprise Server, GitLab, Gitea, Bitbucket Cloud or Azure DevOps, and a hosting account for that server is connected or remembered, Commits shows a mark at the end of each pushed commit's message and the sidebar shows one beside each branch: a check for passed, a cross for failed, a dot for running or queued, and a ring when the commit is on the host but no check ran. While no loaded commit has a check, no rings show. Unpushed commits have no mark. A local branch's mark is for its upstream's last pushed commit.
- A remembered account signs in by itself when its repository opens, for reads only. Rocket asks about the remote's branch tips and the newest 50 pushed commits when the repository opens, after a fetch, pull or push, every 30 seconds while a check runs and Rocket is in front, and every 5 minutes otherwise. GitHub and GitLab answer in one request and Azure DevOps in two; Gitea and Bitbucket answer up to 20 commits, one request each. History never waits for these reads.
- Click a mark, or Show checks in the commit details, for that commit's checks with failures first. Open goes to a failed job on the host. A status that links to another site shows that site's name first. Open in browser opens the failed run, or the commit's page.
- Re-run failed jobs (Retry failed jobs on GitLab) opens a review naming the host, repository, account, run, commit and failed jobs. Rocket offers it only when the run has finished and failed and the host says the account can write to the repository, and checks both again before sending. The host also runs the jobs that depend on the failed ones; GitLab also retries canceled jobs. GitHub re-runs a run only within 30 days of its first run.
- If the answer to a re-run is lost, the review says it may have started: refresh the checks before trying again. Rocket never retries automatically, and a refused re-run names the permission when the host does.
- Bitbucket and Azure DevOps show results only, because their APIs can't re-run failed jobs; Open in browser goes to the run, where the host can. Bitbucket Server has no CI read here.
- The status bar says Checks hidden when the account can't read checks, Checks paused until a time when the rate limit is used up, and Checks from a time when the last refresh failed; marks are faded then. Click Checks hidden or Checks paused to open Actions, which says what to do, or Accounts when the host rejected the token; click Checks from to try again.
- Actions, in the sidebar after AI review, lists the repository's runs from its host, newest first: every workflow, branch, pull request and schedule. GitLab, Bitbucket and Azure DevOps call them pipelines and builds. A red dot on Actions means the current branch's newest run failed; an amber dot, that it is running.
- Actions reads the newest 30 runs when it opens, again when it opens after more than a minute, the next 30 for each Load more (up to 150), and the newest again every 30 seconds while a run runs and Rocket is in front. Choose All branches or the current branch; Failed and Running filter the runs already read. Choose a run to read its jobs; a wide window chooses the newest failed run. Show in Commits selects its commit when history has it.
- Re-run failed jobs in Actions re-runs only the chosen run, after the same review; that run is checked again before sending. All runs in a mark's checks opens Actions on the run's branch with the run chosen; All runs for a branch does the same from the PR Checks tab.
- Without an account for the remote's host, Actions names the host and offers Connect, which opens Accounts on it. Actions also says when the repository has no workflow, when the token can't read runs and which permission it needs, when the rate limit is used up, and when no remote is on a host Rocket reads. Bitbucket Server doesn't report CI to Rocket; its builds show on the server.
- Token permissions for marks, Actions and re-runs:
- GitHub fine-grained token: Actions Read and Commit statuses Read for marks, Actions Read for Actions; Actions Read and write to re-run. Classic token or browser sign-in: repo for private repositories, and repo to re-run. Re-running also needs write access to the repository. Some tokens can't read the batch query; marks then come from Actions runs.
- GitLab: read_api for marks and Actions; api and the Developer role to retry, or Maintainer on a protected branch.
- Gitea: read:repository for marks and Actions; write:repository to re-run, on a Gitea with the Actions API.
- Bitbucket Cloud: read:repository:bitbucket and read:pipeline:bitbucket; Actions needs read:pipeline:bitbucket.
- Azure DevOps: Code (Read) and Build (Read); Code (Status) also reads commit statuses.
- The PR Checks tab shows the head's checks the same way. Checks that ran in a fork may not appear in the base repository, and the list does not evaluate required branch rules or merge readiness.

## Insights

- Insights shows who commits, when, which files change most and how pull requests move. It only reads: nothing changes in your repository or on its host.

### Activity and Files

- Activity counts commits over the last 30 days, 90 days, 1 year or All, on This branch or All branches: commits over time, authors grouped by email, and the weekdays and hours they commit.
- Files is a map of the files at HEAD, coloured by how often they changed in the period. Choose a folder to look inside it; small entries are combined.

### Pull requests

- Pull requests come from the Hosting account matched to this repository's remote. Rocket reads 50 at a time. Load more adds the next 50, up to 500 a refresh; it never reads every pull request by itself.
- The period sets what Opened, Merged, Median merge time and Time to first review count. Open now, Median open age and Waiting for review always cover every loaded pull request. The author and the reviewer set both the numbers and the rows. All, Open, Waiting and Merged change only the rows, which list every loaded date.
- A median uses only pull requests whose dates the host reported: creation to merge for Median merge time, creation to the read for Median open age. A row marked No dates or No merge date has a missing, future or backwards date and is left out of the timing that needs it. A number that can't be worked out shows a dash, never 0. The time a pull request was last updated is never taken as its merge time.
- Partial sample means the host has more pull requests than were loaded, or its list was incomplete or changed while it was read. The numbers then describe the loaded pull requests, not the whole repository. Read again starts a new sample.
- Choose a row to read that pull request's description, commits, reviews, files, discussion and checks. It is a snapshot: read it again to see changes on the host. Descriptions and comments show as plain text, up to 8,192 characters each. A missing check is never shown as passed, and reported checks don't say whether merging is allowed.
- Stop ends a read and keeps the sample you had. So does a failed read; the page then says when its numbers are from.

### Reviews

- Waiting for review counts open pull requests with a review request still waiting, and the requests on them. The requests come with the pull request list, so they cost no extra read. GitHub and GitHub Enterprise Server list the people and teams asked; a person drops off once they review. Azure DevOps lists reviewers who have not voted, Bitbucket Server and Bitbucket Cloud reviewers who have not approved or asked for changes. GitLab's and Gitea's lists name the reviewers but not who still waits, so they have no Waiting for review and the Waiting filter is off.
- The Reviewer field lists everyone the author's pull requests name as a reviewer, people before teams, with how many pull requests wait on each and how many each reviewed. Choosing one shows the pull requests they were asked to review or reviewed. A team counts as one reviewer; its members are not looked up. On GitHub and GitHub Enterprise Server, who already reviewed is known only after Read reviews. The reviewer narrows the list; Trends and its comparison count every reviewer. The chosen reviewer stays while the repository's tab is open but is not saved with its view.
- Time to first review is the median time from when a pull request was opened to its first review by someone other than its author. Time as a draft counts. Reviews by bots, such as Copilot or a CI app, don't count. With a reviewer chosen it is still each pull request's first review by anyone.
- Review times need each pull request's reviews, at least one request per pull request, so Rocket reads them only when you choose Read reviews: the pull requests opened in the period, the chosen author's and reviewer's first, then newest first, up to 100 at a time. Read more continues; Stop keeps what was read. Refresh starts a new sample without them; Read reviews reads them again. Only GitHub, GitHub Enterprise Server and Gitea say when a review was submitted; on other hosts Time to first review says it is not measured there.
- What the median leaves out is named: pull requests whose reviews were not read, could not be read (the row says Reviews unavailable), have a review without a usable time (No review date), have more reviews than Rocket reads without one that counts (Some reviews not read), or have no review yet. GitHub returns reviews oldest first, so Rocket reads its pages, up to three of 100, only until a review counts; on Gitea it reads two pages of 50. A review time more than five minutes ahead of this computer's clock counts only when no earlier review does.
### Trends

- Trends shows how many pull requests were opened and merged each week of the period, weeks starting on Monday (UTC). Past a year, as in All, the bars are months, then years. The bars add up to the Opened and Merged tiles and follow the author you chose.
- Beside the chart the period is compared with the one of the same length before it: Opened, Merged and Median merge time, with how many merged pull requests each median comes from. The tiles show the same change.
- Pull requests are compared only when both periods are in the loaded sample. GitHub, GitLab, Gitea and Bitbucket Cloud list the most recently updated pull requests first, so the sample holds every pull request opened or merged after the oldest update in it. Weeks before that are marked Not loaded instead of showing too few, and Load more reads further back. Azure DevOps and Bitbucket Server don't list by update, so they are compared only once every pull request is loaded. A list that changed on the host while it was read isn't compared; Read again starts a new sample.

### Saved views

- Each repository remembers its Insights tab and choices: Activity's period, branches and table, the pull requests' period, author, state and List or Trends, and Files' period. They come back the next time you open Insights, also after Rocket restarts.
- While the repository's tab stays open, the numbers stay too: leaving Insights and coming back shows them at once, and each row says when they were refreshed (the exact time is in its tooltip). Nothing reads again by itself; Refresh does. When HEAD has moved since, or for all branches any branch, Activity and Files read again as you come back, so their numbers are never another commit's. A choice that needs a new read keeps the old numbers, said for what they were read for, until the new ones replace them. Numbers are not saved to disk: after a restart they are read again.
- Leaving the page stops its reads and closes an open export review.

### Export

- Export saves the rows as they were listed when you chose it: later reads and filters don't change what is saved. It never replaces a file, also not one another program created meanwhile; choose a new name instead.
- CSV is for spreadsheets: UTF-8 with quoted fields, and text that looks like a formula gets a leading apostrophe. JSON keeps the exact text and adds the summary numbers with how many pull requests each comes from; times are Unix seconds and unknown dates are null.
- Both hold repository and pull request details that may be private. Descriptions, comments, reviews, file contents and credentials are left out.
- Each row also says who it waits on, what is known of its reviews (read, not_read, unavailable, no_usable_time, incomplete or not_measured) and its first review's time and reviewer. These columns come last in CSV, after the ones earlier exports had. Only an open row waits on anyone. A row without reviewers in its list reads not_in_list, and one whose list names reviewers but not who waits (GitLab, Gitea) reads no_review_state.
- Export on the Activity row saves the commit activity shown. A CSV holds one table: Contributors (each name after .mailmap with its commits, most first) or Commits per week (every week from the first to the last, empty weeks included, by the Monday of each commit's own date). JSON holds every table: totals, contributors, weeks, months, weekdays, hours and the weekday-by-hour grid, with the scope that was read.
- Email addresses are left out unless you tick Include email addresses. The file names the repository's folder, never its path.

## SSH keys

- Choose Windows agent or Direct key for the selected key. The list/selection stays; its tick means membership in Windows ssh-agent. Direct key needs no agent and requires a verified host key. For an encrypted key, Save passphrase verifies it with OpenSSH, then saves Windows-user-encrypted data bound to this exact profile and key. Saving is opt-in and persists across restarts; Forget passphrase removes only the saved credential. Rocket never passes the passphrase as a command argument or stores it in plaintext. Review Use only this key to apply the chosen method; Restore previous setting works for both. Keep rocket-ssh.exe beside Rocket. Unknown/changed host keys are rejected, never automatically trusted.

- Edit → Preferences → SSH keys lists every key once: the key pairs in %HOME%\.ssh (when HOME names an existing folder, otherwise %USERPROFILE%\.ssh) and the keys Rocket made or set up. Choose a key to see where it is, its fingerprint and its public key. Copy public key copies the public key only; paste it into your Git host's SSH key settings.
- Rescan reads that folder again and shows what ssh-agent holds. A scan reads public keys only: it does not recurse into subfolders, read SSH config or unlock keys. It stops at 512 folder entries or 128 keys and says what it skipped. A key needs its private file beside its .pub; PEM, hardware/security-token keys and keys without a .pub are not supported. Use Add key file for a supported key outside the folder.
- The tick before a key shows whether it is unlocked in Windows ssh-agent. Tick it to unlock the key: ssh-add asks for the passphrase, which goes straight to OpenSSH and is never saved, logged or passed as an argument. Untick it to remove only that key from ssh-agent. A key found in the folder is unlocked or removed only while its files are the ones the scan read.
- New key makes an encrypted Ed25519 key, or RSA 4096 for hosts that require RSA, in %USERPROFILE%\.ssh\rocket-keys. ssh-keygen asks for the passphrase twice; use at least 10 characters. Add key file sets up a key that stays where it is. Existing files are never overwritten.
- Use only this key makes the repository that is open use the chosen key and no other. Rocket shows a review first and backs up the repository's config when you apply it. For a key found in the folder, Rocket first asks for its host, user and port, taken from the repository's SSH remote when it has one, and checks the key again before setting it up; the key is never copied or unlocked. Restore previous setting brings back the repository's earlier SSH setting. Remote URLs and working files stay unchanged, and external edits to the config invalidate the review.
- A key set up this way uses its own SSH config: user and system SSH proxy and host settings are not inherited. Shared worktrees use the repository's setting. Check settings shows what OpenSSH reads from that config; it does not connect to the host.
- Your main .ssh/config and existing keys are preserved; Rocket does not delete key files or change services.
- On Linux, Rocket uses OpenSSH from /usr/bin (Ubuntu: openssh-client), and Windows agent is called ssh-agent. Rocket reaches the agent that SSH_AUTH_SOCK names; desktop sessions set it. In WSL or another session without it, start Rocket from a terminal after `eval "$(ssh-agent)"`, or after `export SSH_AUTH_SOCK=$XDG_RUNTIME_DIR/openssh_agent` where systemd's ssh-agent socket runs. Rocket starts no agent itself.
- On Linux, a private key must be yours and readable by you only (chmod 600), as OpenSSH requires: Rocket lists one that isn't but won't unlock or use it until you fix it. Folders Rocket makes are 0700 and its files 0600, and links to keys are refused. Linux can't lock a key's files while they are used, as Windows does; Rocket checks them again afterwards instead.

## File history

- In Blame, click a line's commit to select it, and Shift-click another line to select a range; then see the history of those lines.
- Line history lists the commits that changed the selected lines, newest first, each with those lines' changes. Git follows the lines through edits and renames.
- Ctrl-click a second commit in the list to compare the file between the two commits.
- File history lists up to 10,000 commits. Inspect an older commit to continue further back.
- Commit, file and blame review windows are read only and pinned to one commit; close one with its window's close button. A commit review lists its changed files beside the diff (a menu in narrow windows). Stop reading ends a slow read; Retry loads it again.
- Image previews load before/after first. Pixel differences are generated when selected. Tabs and review windows share a 192 MiB texture budget; a released preview offers Reload. Decoding buffers and graphics-driver overhead are separate from that budget.

## Activity and diagnostics

- The Activity drawer lists this session's Git operations, newest first. Details open with the arrow at the end of a row. Git output is shown only when you ask for it, because it can contain private repository data; Copy details leaves it out.
- Clear empties the drawer only. All diagnostics in the drawer opens Activity & diagnostics, which keeps reading the records Rocket writes on this computer, including earlier sessions, and never includes raw Git output, arguments, paths or repository names. Search covers operation names, steps and error codes.
- The page lists what you did and what went wrong. Reads Rocket does by itself stay out unless one fails or is cancelled; Show reads lists them, and a search always looks at them.
- Times are on this computer's clock. Hover over a time to see it in UTC.
- A failed operation shows Rocket's explanation and next step where it has them, then the error code and where it stopped. Steps lists each step once, with setup folded. Records is everything that was logged, with where each Git process spent its time.
- Live reads new records every few seconds. Stop appears if a read runs over a second, and stopping one pauses Live.
- Logs rotate across five files of up to 1 MiB each; the oldest records go first. A gap or a partial record means part of the log was rotated away or cut short.
- Incomplete means the records end before the operation finished, so its outcome cannot be confirmed; it is listed with Needs attention. A Git version check that exits on its own does not decide whether an operation succeeded.
- Copy summary copies up to 200 structured records of the selected operation, with a count of any left out.

### Repository benchmark

Open View → Repository Benchmark and choose Run benchmark after other
repository operations settle. Rocket measures three passes of full refresh,
the first 200 HEAD commits and preparation of the starting HEAD commit diff.
First uses existing caches; Repeat median excludes the first pass. No cache is
cleared. Painting, image decoding, network and external tools are not measured.

Full refresh includes discovery, changes, refs, remotes and history. While other
repository operations or scheduled fetches run, Run benchmark waits and the Timings
card says Waiting for other operations. Results are observations, not a speed
score: other programs, antivirus scanning and operating-system caches affect them.

Stop immediately shows Stopping while the active Git processes exit. Finished
samples remain visible; failed or cancelled reads are not counted as successful
timings. Close also stops a running benchmark before leaving. Unborn repositories
skip history/diff. An external-change or unavailable-observation warning means
you should rerun before comparing measurements. Detection is not an atomic snapshot.

The benchmark does not alter your files, index or refs. Scheduled fetches wait
until it finishes. Copy summary copies JSON containing times, counts and
outcome codes, excluding paths, refs, author data and file content. The report is
kept only in memory; Activity & diagnostics retains the individual read events.

### Drawer and terminal

- The arrow at the right of the drawer's tab strip collapses the drawer to that strip, which keeps the last operation, or the terminal's state, in view. The arrow, that line or a tab opens the drawer again at the height you left it.
- The toolbar's Activity and Terminal buttons and Ctrl+` hide the drawer completely. On a collapsed drawer the first press opens it.
- A running operation has one spinner and one Stop: in the footer of the dialog it came from while that is open, otherwise in its Activity row while the drawer is open, otherwise in the status bar. Anywhere else it is only named. The Terminal menu acts only on the terminal.
- The light at the right of the status bar means Rocket is reading in the background, for example refreshing or loading history, and says which. Pages and dialogs show their own reads where their data appears.
- The terminal runs the shell chosen in Edit → Preferences → Terminal, in the repository folder: Windows PowerShell, or on Linux your login shell, unless you change it. Type in the command field under the screen and press Enter, or click the screen for direct keyboard input, arrows, Tab and Ctrl+C.
- Interrupt sends Ctrl+C to the running command. Clear scrollback removes the lines above the screen; what is on screen stays. End session stops this session; Terminal → New Terminal starts another.
- A shell that can't start says why in the drawer, with Terminal settings to change it. Rocket never starts another shell in its place. A shell that stops with an error right after starting keeps its own output below that message, because it usually says what went wrong.

## Preferences and tools

### General

- Everything on Edit → Preferences → General applies right away, and each new setting starts at Rocket's usual behaviour.
- Source code folder is where Clone's Parent folder and New repository's Project folder start, and where their Browse opens. New repository still needs the project's folder name: Create stays off for the source code folder itself. Clear the field to go back to the folder of the latest repository. File → Open keeps starting beside the latest repository.
- If the folder is missing, or the path is a file, the page says so and Clone starts next to the latest repository. Rocket doesn't test whether it can write there, because only writing a file proves that; Clone reports it itself.
- Tags in fetch sets where the Fetch dialog starts and what automatic fetch brings: No tags for commits only, Usual tags for tags on fetched commits, All tags for every tag on the remote.
- All tags never replaces a tag you have. If a tag moved on the remote, that fetch fails, and automatic fetch shows its quiet failure note at each interval until you delete your copy of the tag or choose Usual tags.
- Space in branch names changes spaces to - or _ as you type or paste in New branch, Rename, New worktree and branch recovery. Not allowed keeps the check that asks for another name.
- Update submodules on checkout: after Switch to branch and Save, switch & restore, Rocket updates submodules that are initialized and clean to the commit the branch records, nested ones after their parents. The setting is your go-ahead, so no review opens. Each previous commit is kept in its submodule, under the same recovery reference a reviewed update leaves.
- Submodules with local changes, or that need attention, are left alone and named in Activity; nothing is forced. Uninitialized submodules stay out, so a branch can't make Rocket clone a new URL. Pull, merge and rebase don't update submodules. Review submodules after branch switching on the Submodules page still opens that page, once the update has finished.
- Push after commit makes Commit & Push the main button in Changes and gives it Ctrl+Enter, with Commit beside it. The usual review still opens before anything runs, and leaving it commits nothing. Without a remote, or on a detached HEAD, Commit stays the main button.
- The Diff view card previews its options. Show − and + marks hides the column that holds them, Font size runs from 11 to 16 and rows grow with it, and Syntax highlighting sets every tab; the Syntax checkbox in a diff's toolbar still flips one tab until the setting changes. These apply to the main diff in Changes, Commits, Compare and review windows. File history, Stashes and patch previews keep their own small viewers.
- Compact branch labels shortens the commit list only. A branch and the remote branch it tracks on the same commit share one chip with a cloud mark. A remote branch drops the remote's name when the repository has one remote. Folders shorten to their first letter, so rocket-ui-test/commands reads r/commands. Hover a chip for the full names; branches that don't fit become +2.

### Secret protection

- Edit → Preferences → Security → Secret protection is on by default. You can turn it off and back on; Rocket remembers your choice for this profile. An amber warning appears while protection is off.
- Rocket checks staged content before explicit commits and checks reachable history before pushes. Recognized GitHub tokens, private keys, likely credential literals and .env files cause a refusal with a location and instructions. Deleting a secret only from the newest commit does not remove it from history.
- Use placeholder-only .env.example, .env.sample or .env.template files. Keep real secrets outside Git. Revoke credentials that have already been exposed.
- MCP cannot change this preference. This guard covers Rocket operations; raw Git and custom shell commands are outside it. Detection is bounded and cannot recognize every secret or encoding. An incomplete scan also refuses the write.
- **Total scan budget** on the same page defaults to **256 MiB**. Enter a positive whole number of MiB and select **Apply**, or reset to the default. This counts uncompressed objects, including unchanged staged content for a commit and reachable older history for a push; it is not the compressed upload size. Raising it can take longer. The separate 128 MiB per-object cap, time limit and cancellation remain active. Changes apply to future scans and are saved for this desktop profile; the CLI uses its own 256 MiB default.

### Guard AI API keys

- **Guard AI API keys** is a separate switch in Edit → Preferences → Security, on by default. It checks supported OpenAI, Anthropic, DeepSeek, GLM/Z.ai, Google, Groq, xAI, OpenRouter and Hugging Face key shapes locally, without contacting providers. DeepSeek and GLM/Z.ai shapes require a credential assignment because ordinary IDs can look similar. Unknown formats can be missed.
- The two guards are independent: switching off Secret protection leaves the AI guard on, and switching off the AI guard leaves general credential checks on. Both inspect outgoing history. Only the user in the normal desktop app can change them; MCP cannot. The CLI always enables both.

### Dependency advisories

- Edit → Preferences → Security → Dependency checks scans the open repository in the background with local advisory data; Scan now runs it immediately. Scanner & data opens scanner selection and advisory downloads. Expand a finding for its details and advisory links. Background scans only warn. Scans run offline and do not execute project scripts.
- Scan dependencies before pushing is a separate option, off by default. When enabled, OSV scans the committed versions being pushed, including tags and multi-branch pushes. Complete results with no warnings continue automatically; findings or limited coverage pause for Push anyway or Cancel push. Missing scanner/data or a failed scan stops the attempt. Cancelled commit-and-push keeps the local commit. Existing secret protection runs first and cannot be bypassed by this review.
- The checking popup shows completed checks, the active dependency scan and the waiting push. Review groups findings by package version; expand a package for advisory details or Scan details for coverage. Cancel push keeps local commits. Push anyway approves this attempt only.
- Update advisory data downloads public ecosystem databases from OSV. Repository files and package lists stay on this computer.
- On Linux, download osv-scanner_linux_amd64 from the release Get OSV-Scanner opens, make it executable (`chmod +x`), then choose it. Rocket accepts only the file itself, not a link, and only one that no other user can change. It runs the exact file it checked.

### Git program and identity

- Edit → Preferences → Git lists Automatic, Bundled Git, installed copies and Custom path. Automatic prefers the Git included with Rocket; if this build has no bundle, it finds Git on PATH. Explicit installed/custom choices take priority. Choosing a copy checks its version (Git 2.46 or later is required) and saves it only when the check passes. Rocket switches after a restart. Only choose a trusted Git installation.
- Bundled Git travels with the app and includes its supporting tools. Its files are verified before use. Verify files repeats the full check with progress and Stop; a stopped or failed check leaves your saved choice intact. Missing/changed runtime files require a complete matching bundle or selecting another trusted installation. A choice changed by another writer must be reloaded before saving. This does not replace system Git or change PATH, credentials or SSH settings.
- Windows HTTP(S) Git operations require Git for Windows 2.56.0.windows.4 or newer. Preferences retains the full build suffix and shows HTTP blocked for older or unrecognized builds; local/SSH work remains available. Restart Rocket after updating Git.
- Commit identity sets the name and email for every repository on this computer, in your global Git config; a repository's own settings can override them. The info icon names the file. After an edit, Save writes it and Discard drops it. Leaving a field blank removes only that direct setting, and a value set elsewhere shows as the field's hint.

### Keyboard and mouse shortcuts

- Open Edit → Preferences → Shortcuts. Each command has a Shortcut and an Alternate cell: click one, then press the keys or mouse gesture, and it is saved at once. Esc cancels; Delete or Backspace clears the cell. Search finds a command by name or by its keys.
- To see what an input does, choose the keyboard button at the end of the search box, then press the keys or gesture: the box shows the input and the table lists only the command that uses it. Esc or a click cancels, and wheel scrolling still scrolls. Finding never changes a shortcut.
- Each of the 21 common commands supports two bindings. Mouse back/forward buttons work alone. Left, right and middle clicks and wheel directions require Ctrl, Alt or Shift. Keyboard bindings use Ctrl or Alt, or a function key. Ordinary typing, clipboard/navigation keys and Help stay standard; duplicate or reserved bindings are never saved.
- If the input belongs to another command, choose Use for this command (the other command loses it) or Keep. An input that cannot be used, such as a plain click or a reserved key, shows why under its row with Record again and Cancel. The reset button beside a changed command restores its default if that input is free; Restore defaults asks before resetting all bindings. Saved bindings belong to this Rocket profile, survive restarts and update menu/palette hints. They do not change repository files.
- Terminal input and modal reviews keep their own controls. A bound mouse gesture runs only when its command is available and does not also click, scroll or zoom beneath the pointer. Other gestures keep their normal behavior. Shortcuts named in this guide are defaults and may differ after customization.

### Terminal

- Edit → Preferences → Terminal chooses the shell the Terminal drawer runs. Save and Discard appear after an edit. A new choice applies to the next terminal; one that's open keeps its shell until you end it.
- Windows PowerShell is the default. Like PowerShell 7, it starts without your profile, with UTF-8 text and Rocket's two-line prompt. For your profile, use Custom.
- Git Bash is the bash of the Git Rocket uses, so the Git included with Rocket supplies it and nothing needs installing. It runs as a login shell: your .bash_profile and .bashrc run, and git in it is that same Git. A Git without bash shows Not available; pick a full Git for Windows in Preferences → Git, or use Custom.
- Git Bash's prompt runs Git in the repository each time it shows, as it does anywhere. If you have a .bashrc but no .bash_profile, Git Bash itself writes one into your home folder and shows a warning.
- Command Prompt runs at code page 65001, for UTF-8 text, which an old batch file may not expect. It can't use a network folder as its current folder, so in a repository on \\server\share Rocket doesn't start it; it would run your commands in the Windows folder. Use another shell there.
- Custom runs one program by its full .exe path, with one argument per line and no shell. It starts from the shell you had. Rocket checks that the file is there and runs nothing until New Terminal.
- Interrupt sends Ctrl+C, and Clear scrollback is Rocket's own, so both work on every shell; what Ctrl+C does in a custom program is up to that program. Arrow-up history is the shell's. Both PowerShells and Git Bash save it as you go, so End session keeps it. Command Prompt's lasts for the session.
- The program and arguments come only from your settings on this computer, never from a repository. MCP can't change them, and an MCP window always uses Windows PowerShell.
- On Linux the choices are Login shell (the program $SHELL names, the default), Bash, Zsh and Fish as /etc/shells lists them, and Custom: any program by its full path. They run as interactive shells with your own startup files and history, on a Linux terminal with job control, so Ctrl+C, Ctrl+Z, fg and bg work as usual. End session hangs up the shell and its jobs, as closing a terminal window does, and ends what is left a second later; a program that started its own session (setsid) keeps running. An MCP window always uses bash without your startup files.

### External tools

- Add lists the tools Rocket knows for that type: those on this computer, then the rest, then Empty profile for the plain form. Choosing one only fills in the form with its name, program and arguments. Nothing runs, and nothing is saved until Save profiles.
- If the selected profile is still empty, the tool goes in that one, so the first Default doesn't stay blank. Otherwise it's a new profile, and the default stays as it was. A tool already in your list just selects its profile.
- Rocket looks only for the file, in the tool's usual install folders. A tool elsewhere shows under Not found: choose it, then Browse to the program the form names. A saved profile whose program has gone says so on the form.
- If VS Code isn't already open, Rocket waits until that VS Code closes, not just the tab, so close its window when you're done. Three-way merging needs Beyond Compare Pro or Araxis Merge Professional. For WinMerge, turn on the Merge Result pane in its Compare/Merge options.
- Each tool type keeps up to 16 profiles; the default is used unless you choose another when launching. Profiles are saved on this computer when you click Save profiles.
- A profile runs one program, by its full .exe path, with one argument per line and no shell. Placeholders stand for the files Rocket prepares, each on its own line: {file} for an editor, {left} and {right} for a diff, {base}, {ours}, {theirs} and {result} for a merge.
- Use a program that stays open until its window closes. A launcher that hands the file to an app that is already running cannot be tracked or stopped.
- An editor works on the real working file, so its saves change your working tree and stopping it cannot undo them. A diff tool gets read-only copies of both versions, up to 16 MiB each; a missing side is an empty file. A merge tool edits a temporary result, which Rocket shows before anything is saved; saving checks the original file again and keeps recovery copies.
- Hide (or Esc) keeps a running tool, or a merge result you have not reviewed yet, while you work in Rocket; the status bar brings its dialog back. Discard result drops the result. Temporary copies contain file contents and stay on disk for recovery. Diagnostics record phases and outcomes, never paths, arguments or contents.

### MCP server

- The local MCP server lets your tools control Rocket test windows over Streamable HTTP, on this computer only and one controller at a time. It starts only when you click Start server. While Rocket checks the server, its card says Checking; Check status checks again.
- In a test window, a click on pull, force push, Push Branches Together or a remote tag runs an input check first. It shows only if it takes over a second. Stop or Esc drops the click, and so does using the window yourself; nothing has run by then.
- Only marked test repositories inside the test repositories folder can be controlled. Stop the server to change its port or folder. After an edit, Save keeps it and Discard drops it; Start server waits until you choose one.
- Connected tools need the access token. Copy token puts it on the clipboard; Rocket never shows it. Rotating the token disconnects clients that still use the old one.
- Close a controlled test window before stopping its server; closing a test window leaves the server running.
- On Linux the server's folder, ~/.local/state/rocket/mcp, is yours only (0700) and its settings and token are 0600. Rocket refuses a folder other users can change, or files they can open, and says how to fix it; Rotate token replaces a token others could read.

### Storage

- Edit → Preferences → Storage shows where Rocket keeps its diagnostic logs and saved state, with Copy path and Show in Explorer for each.

### Publish to GitHub

- Connect a GitHub account in Preferences → Accounts, open Repository → Remotes & Hosting → Hosting, then select Publish repository. Enter your login or an organization, a new repository name and an unused remote name. Private is the default; Public requires an explicit acknowledgement.
- Review publish checks the selected local branch and its committed history. Create repository and push creates an empty GitHub repository, attaches the remote and pushes only the reviewed branch. Staged, unstaged and untracked files stay local. No README, license or ignore rules are generated.
- GitHub API access and Git authentication are separate. HTTPS uses your existing Git credentials; SSH uses system SSH. The hosting token never enters Git configuration or arguments. Creation may succeed while Git push is denied; the result shows both steps separately.
- Stop shows Stopping immediately. A lost creation response can leave an unknown result: inspect GitHub before acknowledging a new review. Created repositories and configured remotes are retained on partial failure. Use a normal reviewed Push to finish after fixing access; nothing is automatically retried or deleted.
- Existing remotes, URL rewriting, config locks, stale branch/config reviews and already-configured upstreams are refused. Turn off Set upstream to preserve an existing tracking relationship. Reviews expire after two minutes.

### AI drafts

- Edit → Preferences → Commit messages → AI drafts turns on the Draft button and sets Draft with, Program, Message style and Language. While it's off, nothing is sent anywhere.
- A draft sends the staged changes, up to 128 KiB of diff with binary files named only, and Rocket's instructions. Claude Code drafts go to Anthropic under your Claude account and run without tools. Codex drafts go to OpenAI under the account Codex is signed in with, together with your global AGENTS.md if you have one, and run read-only without tools or your Codex settings. A draft is never committed for you.

## Themes

- Choose a theme in Edit → Preferences → Appearance, View → Appearance or the command palette. It applies at once and is saved for this profile. The window title bar keeps following Windows.
- Clay, the default, is Claude Code's colours: warm charcoal, cream text and a coral accent, or ivory and terracotta in its light mode. Its Mode follows Windows' app mode, as Claude's app does, or keeps Clay dark or light.
- Ocean is Rocket's original navy and cyan, the default before Clay. Dark is neutral graphite. Light is light. System shows Light or Dark, following Windows' app mode as it changes.
- Custom starts from Ocean, Dark, Light, Clay dark or Clay light. Its accent is a swatch, a colour code such as #F472B6, or From picture: the colour each built-in picture was drawn around, or your picture's most common vivid colour. Rocket lightens or darkens an accent until it reads well on the theme.
- A Custom theme can show a picture behind the main window: Astronaut, Space cat, Planet, Stars, or your own PNG, JPG or WebP picture of up to 32 MB. Rocket keeps a copy in its storage, at most 2,560 pixels on its long side, so moving or deleting the original changes nothing. Choosing another picture replaces the copy.
- The panes let the Visibility share of the picture through. Where a bright part (on Light, a dark part) would make text hard to read, Rocket dims or lightens it. Dialogs, menus and cards stay solid.
- Fill covers the window, and Position keeps the picture's left, centre or right in view. Fit shows the whole picture, its inner edge fading into the theme; that suits a picture of a character.
- Row density sets the height of list rows.

## Commit & Push and saved switching

- Changes → Commit & Push reviews the message, staged changes, configured remote and destination branch. It commits first, then pushes that commit normally. Unstaged/untracked files and existing upstream settings stay as they are.
- A rejected or cancelled push keeps the local commit. Read both outcomes, then use Push separately after addressing the error. An unknown remote outcome needs inspection before a retry; do not create another commit just to retry publishing.
- Save, switch & restore in a local branch's actions, or Save + switch under Repository → Branch → Workflows & Cleanup, saves tracked changes, switches to the reviewed existing local branch and restores staging too. Untracked/ignored files stay in place and are protected from overwrite.
- The recovery stash is always kept. On conflict, external change or cancellation, inspect the actual checkout and the displayed stash/journal before restoring anything. There is no automatic discard, rollback, duplicate apply or stash drop.

## Extensions

- Edit → Preferences → Extensions lists every extension. Jira and PlayGround ship with Rocket and start off. An extension runs in a sandboxed Lua worker and draws native panels; it cannot run Git or shell commands, read your files or see your tokens.
- On Linux, extensions need an x86_64 system with Landlock, which Linux 5.13 and later provide when it is switched on, as Ubuntu does. Without it Rocket runs no extension script and says why.
- Enable opens a review the first time: where the extension came from, its code and its three permissions. Network reaches only the server set in a connection. Reviewed writes and Repository details stay off until you allow them for a connection. Bundled Jira is copied into the extensions folder when you enable it. Enabling runs nothing: scripts run only from Refresh panel, a control in the panel or a command you choose.
- Install from file takes a package's integration.json, with main.lua in the same folder, and copies both after the review. Rocket can't confirm who made a file, so only install what you trust: an extension with Network can send what you share with it to the server you set.
- Each enabled extension gets its own entry in the sidebar, the icon rail, View → Go to and Preferences. An amber dot means it needs a review, or a check of a write that may have reached the service.
- An extension can add commands. Every command is in the palette under its extension's name. A command that needs no commit or branch is also in Repository → Extensions; one for a commit or branch is on that row's right-click menu, in a submenu named after the extension. A command opens the extension's page and runs there, and it uses the connection selected on that page. It gets the commit or branch you chose, not the selection, plus the repository details the connection shares. Writes still stop at the request review.
- Commit and branch commands need Share branch, commit IDs and change counts on the connection, and the right-click menu offers that setting until it is on. While an extension's files have changed, it has no server yet, or its saved draft no longer fits, one row takes you to the review, the connection or the page with Clear draft in place of its commands. Opening a menu runs nothing, and Rocket never runs a command by itself.
- Select an extension in the list for its details: Enable, permissions, Update and Remove, then its connection. Right-clicking a row offers the same actions. A connection holds the server, account, token and settings, plus Allow writes and Share branch, commit IDs and change counts. Save and Discard appear after an edit. An extension with no network has Settings instead.
- New connection adds another account or server, up to 16 per extension. It starts empty with a distinct name; Save brings it into use while the extension is on. The Connection selector on the page switches between them. Remove connection asks first and cannot bypass an uncertain-write guard.
- One review covers all of an extension's connections. Allow writes and Share stay per connection.
- A newer bundled Jira offers Update to its version, and it never replaces files on its own. Update from file takes a newer integration.json with the same ID. An update leaves the extension on or off as it was. Editing the files in the extensions folder shows Files changed, then Review after Reload folders. Each review marks new permissions and new, changed or removed commands, counts changed lines against the approved copy, shows them with View changes, and lists what stays. The extension can't run until you approve.
- A file can't use bundled Jira's ID, so it can't inherit saved tokens. Any other ID that is already installed opens the Update review. A file that can't be installed says why, and nothing is installed. A package for a newer Lua API asks you to update Rocket.
- Remove takes the extension's files, connections, tokens and drafts, and changes nothing on the service. Bundled Jira stays in the list, off. Remove waits while a request may have reached the service.
- Developer tools at the foot of the list has the extensions folder, Open Lua playground and the repository sample. Developer tools in an extension's details validates current Lua syntax without running it and shows recent action codes and durations. Syntax and runtime failures identify a source line where available; raw private Lua error values are omitted.
- Packages can request read-only repository context. Grant Share branch, commit IDs and change counts separately for each connection. No paths, remote URLs, author identities or file contents are shared. A network extension can send shared values to its configured service. The repository sample demonstrates this offline.
- Jira shows your issues beside the one you choose. One search box takes an issue key, words or JQL, and My issues, Project, This week and This branch switch the list; This branch needs Share and an issue key in the branch name. An issue shows its description and comments, with Move to, Edit and Add a comment. Create issue lists the project's issue types and names any required field it can't fill. Configure your own HTTPS server, account and token, and project key. Checked writes reread the update marker before review and send; external changes stop the request and retain edit fields, and Reload in the edit form reads the new marker. The check is not atomic with Jira's write. Projects may require custom fields, and unsupported required fields or transitions need Jira itself. Live Jira has not been tested; only mock services have.
- A classic Jira API token uses your site address as the server and leaves Cloud ID for scoped tokens empty. A token created with scopes needs https://api.atlassian.com as the server and your site's Cloud ID, which https://yoursite.atlassian.net/_edge/tenant_info shows while you're signed in. Give that token the Jira scopes read:jira-work and write:jira-work.
- PlayGround shows every control an extension can use, beside the Lua that draws it. Open Lua playground, or Lua playground in the command palette, opens a Lua editor with Run, Stop and interactive native controls; line numbers sit beside the code and the preview is framed on the right. Compact windows use Code and Preview tabs. HTTP requests are descriptions only; no account, repository or network access is provided. Errors and cancellation keep the last successful preview and callback state. A fresh Run resets callback state; editing source disables the old preview's actions until Run succeeds.
- Save protects scratch source on this device. Normal close saves dirty source and stays open on failure; a crash can lose edits since the last save. Restart restores code without running it. External edits refuse replacement; Reload saved code asks before discarding an unsaved draft.
- Lua arranges native cards, inputs, choices, buttons and tables inside its own panel. It cannot replace Rocket's permission or review controls. Scripts run in a separate bounded worker; no system Lua or Node installation is required. The developer contract is INTEGRATIONS.md in the project folder.
- Remembering a token is optional and uses separate protected local storage. Existing Git and hosting credentials and SSH settings are unchanged. Allow writes explicitly, inspect the exact request, then choose Send request. Back sends nothing.
- Stop shows Stopping while Rocket ends the worker or helper. A submitted write can still finish remotely; inspect that service, refresh, then acknowledge an uncertain result before sending another write. The guard survives restarts and crashes. No write is automatically retried.
- Forms, script state and the last panel save locally in protected storage. Switching or restarting restores cached data without executing Lua or contacting the service. Normal closing waits for pending saves; failures keep Rocket open. A crash can lose edits since the last successful save. A draft stays bound to the exact files and connection: after an update, or when a connection's server or account changes, it stays saved but blocked. Restore the originals or choose Clear draft.
- Folders that can't load show as rows with their reason, and healthy extensions stay available. When two folders use one ID, every copy stays off until you remove one from the list. Reload folders re-reads the extensions folder. See docs/wiki/Integrations.md for storage limits, migration and recovery.
