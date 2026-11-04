# Repository manager

Open **File → Repository manager** to organize saved repositories into folders
and workspaces. Saved entries are shortcuts: organizing or removing them does not
move or delete repositories on disk.

## Add several repositories

1. Choose **Scan folder**, then enter or browse to a parent folder such as
   `D:\Projects`.
2. Leave **Include subfolders** enabled to search deeper. With it disabled, Rocket
   checks the selected folder and its immediate children.
3. Choose **Scan folder**. Discovery runs in the background. **Stop scan** cancels the scan
   without adding anything.
4. Review the results. Existing shortcuts show **Already saved** and cannot be
   added again. Linked worktrees have separate entries, even when they share Git
   metadata with another repository. Bare repositories are identified too.
5. Filter by name or path, or hide already saved entries. **Select shown** changes
   only visible new entries; hidden selections stay selected and are counted in
   the footer. Relative paths keep the list compact; hover for the full location.
6. Select the repositories and the saved folder to put them in, then choose
   **Add repositories**. Rocket rechecks the selection before saving it together.
   It does not open tabs, change Git configuration, stage files or move folders.

Search, selection and **Save in** stay above the scrolling results. **Done** closes
an all-saved result; **Change folder** lets you retry an empty result.

**Stop adding** cancels validation and leaves the saved library unchanged. If a selected repository
disappears or becomes invalid, no part of that batch is added; correct its location
and scan again. A scan also refuses to overwrite library changes made while its
review was open. Existing shortcut labels and saved folders stay unchanged.

Folder links, junctions and Git metadata are not traversed. A directly selected
folder link is refused; choose its actual folder instead. Unreadable entries and
invalid repositories appear as counts in the result. **Review details** explains
up to 20 skipped entries; an unreadable parent folder is an error, not an empty scan.
A scan is bounded to 50,000
folders, 500,000 directory entries, 64 nested levels and 1,024 discovered repositories.
If a limit is reached, Rocket says the results are partial; choose a narrower parent
folder to find the remainder. The saved library also allows at most 1,024 entries.

Select a saved repository and choose **Open**, or save a set of repositories as a
workspace to open them together. A saved folder only groups shortcuts; it does not
create a folder on disk.
