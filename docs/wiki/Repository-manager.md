# Repository manager

Open **File → Repository manager** to organize saved repositories into folders
and workspaces. Saved entries are shortcuts: organizing or removing them does not
move or delete repositories on disk.

## Add several repositories

1. Choose **Scan folder** and pick a parent folder such as `D:\Projects`. The scan
   starts at once and runs in the background; **Stop** cancels it without adding
   anything.
2. Results are grouped by the folders they sit in on disk, the chosen folder's own
   repositories first. Each row shows the repository's branch and when its last
   commit was made; a repository without commits shows neither. Existing shortcuts
   show **In library** and can't be added again. Linked worktrees have separate
   entries, even when they share Git metadata with another repository, and bare
   repositories are marked too.
3. New repositories start ticked. A group's box changes only the rows the list
   shows, so search by name or path first to pick part of a group; ticked rows
   hidden by the search stay ticked and the footer counts them. **All**, **New**,
   **Saved** and **Skipped** switch the list. Hover a row for its full location.
4. **Save in** decides where they go. **Match disk folders**, the default when the
   results have subfolders, makes one saved folder for each disk folder and reuses
   a saved folder with the same name. Choose a saved folder, or none, to put them
   all in one place.
5. Choose **Add N repositories**. Rocket rechecks the selection before saving it
   together. It does not open tabs, change Git configuration, stage files or move
   folders.

**Subfolders** in the toolbar searches deeper; turning it off checks only the chosen
folder and its immediate children, and scans again. The folder button picks another
folder and the refresh button scans the same one again. Search, the views and
**Save in** stay put while the results scroll. An empty result offers **Choose
another folder**.

**Stop adding** cancels validation and leaves the saved library unchanged. If a selected repository
disappears or becomes invalid, no part of that batch is added; correct its location
and scan again. A scan also refuses to overwrite library changes made while its
review was open. Existing shortcut labels and saved folders stay unchanged.

Folder links, junctions and Git metadata are not traversed. A directly selected
folder link is refused; choose its actual folder instead. **Skipped** lists up to 20
entries the scan passed over and why: folder links, unreadable entries and invalid
repositories. An unreadable parent folder is an error, not an empty scan.
A scan is bounded to 50,000
folders, 500,000 directory entries, 64 nested levels and 1,024 discovered repositories.
If a limit is reached, Rocket says the results are partial; choose a narrower parent
folder to find the remainder. The saved library also allows at most 1,024 entries.

Select a saved repository and choose **Open**, or save a set of repositories as a
workspace to open them together. A saved folder only groups shortcuts; it does not
create a folder on disk.

## See where each repository stands

Each row shows the repository's branch, how many files it has changed, and where it
stands against its upstream as of its last fetch: **↑2** commits to push, **↓4** to
pull, **↑1 ↓3** when it has diverged. **Folder missing**, **Fetch failed** and
**Couldn't check** show in red. Rocket reads this locally in the background, four
repositories at a time, for the folder, workspace or list you select; a row says
**Not checked yet** until its turn. **Needs attention** narrows the list to the
repositories that do, and a dot marks folders and workspaces holding one.

## Update many repositories

**Fetch all** fetches every remote of the listed repositories, two at a time, with the
rules of automatic fetch: it never prunes, merges or asks for a password. **Pull all**
then moves only a branch that can fast-forward cleanly: one with an upstream, no
changed tracked files and no merge or rebase in progress. Everything else is skipped
and listed with the reason, never forced. The result card lists failures and skips
first; **Open in tabs** opens them so you can sort them out one by one. **Stop** skips
the repositories that haven't started.
