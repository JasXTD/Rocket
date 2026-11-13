# Repository map

Open **Map** in the sidebar, or find **Repository map** in the command palette.
**Code**, **History** and **People** each read a pinned snapshot of your local
repository. They do not fetch, switch branches, run project code, or change working
files or the staging area.

The revision field at the right of the toolbar names the commit, branch or tag to
map; press Enter or choose **Refresh** to read it. The header says which snapshot is
on screen. **Details**, beside the map or as a drawer in a narrow window, says what
the selection is, what it connects to and where to go from it.

## Code

**Packages** opens first when the repository has Cargo packages. It shows packages
with literal local `path` dependencies from committed Cargo manifests, in layers:
what a package uses sits below it, and arrows point to what is used. Selecting a
package keeps its connections bright and dims the rest; the packages that use it
connect with dashed lines. Details lists its folder, file count, what it uses and
what uses it. **Show folders** opens its folder in the tree; **Commits here** shows
the commits that touched it in History.

**Folders** shows the committed folder tree, read left to right. Folders start
closed: open one with the arrow on its card, or use **Expand all / Collapse all**.
A folder's own files are summed up on one dashed card beside its subfolders, named
when there are three or fewer, and files at the top of the repository on one card of
their own. The tree shows folders inside folders only; a Cargo package's folder keeps
its folder name, and dependencies show in Packages. A selected folder's
Details lists its folders and files; choose a file there to see the commits that
changed it. Search can find a file inside a closed folder and selects the folder
that shows it.

This version does not infer every language's imports. Workspace-inherited, escaped
or otherwise unsupported Cargo declarations stay unlinked. Symlinks and submodules
are listed as references; their targets are not traversed. Uncommitted and untracked
files do not appear.

### Modules

**Modules** shows the modules of one Rust package and which module uses which. Choose
the package on the button beside **Packages · Folders · Modules**, or **Show modules**
in a package's Details. Without a choice it reads the package with the most Rust
files.

The list on the left names every module of the package with how many modules use it,
most used first; **Most used** switches to A to Z. A module with modules inside it
opens with its arrow, and search opens what holds a match. The picture puts the chosen
module in the middle, the modules that use it on the left and the ones it uses on the
right: eight a side, the rest counted on one card that opens them in Details. Point at a
card to see the names it uses; choose it to move it to the middle. A module inside the
chosen one counts as part of it; its parent shows as itself, and a module elsewhere
shows as the module that holds it beside the chosen one.

**Whole package**, for a package of 40 top-level modules or fewer, draws them all in
layers, what a module uses below it, with modules that have no connection apart. Its
Details give the folder, Rust files, modules and connections, the most used modules,
and the files no module of the library is read from: `src/main.rs` beside a library
and files in `src/bin` (their own program), files no `mod` names, files over 1 MiB,
files past the read limits and files that are not UTF-8. A module's Details give its file and size, what it uses and
what uses it, and **In this file**: what the file declares at its top level, public
items first, each type with its functions counted. In a narrow window the list folds
into a button above the picture.

Modules reads the committed `.rs` files as text, up to 2,000 files of 1 MiB each and
32 MiB in all. It follows `mod` declarations from `src/lib.rs` (or `src/main.rs`),
including `#[path]` files elsewhere in the repository, and connects a module to another
through `use` lines, paths starting `crate::`, `super::` or `self::`, and paths into its
own modules. Names a `use …::*` line brings in are matched by name. Macros and `cfg` are
not expanded, other packages are not followed, and which function calls which is not
shown: that needs the compiler. Nothing is compiled or run.

## History

**Branches & merges** opens first. It follows the revision's first parents as the
main line. Each merge shows the branch it brought in (named from the merge's
subject) with a count of its commits and their authors; select it to list those
commits in Details and open any of them. Runs of plain commits fold into one line
with a count. A commit with a branch or tag, or where a branch started, keeps its
own line. With **All branches**, branches ahead of the main line show above it with
how many commits they're ahead.

**Every commit** lists every loaded commit with its lanes, subject, author, ID and
date. Select a commit and choose **Open commit** for its diff, or **Find
contributor** to see the author in People.

**Path** and **Leave out** open the scope row: an optional excluded base (commits a
commit or branch already has) and a literal file or folder path. The scope contains
commits reachable from the revision (or the selected all-branch tips) excluding the
base and its ancestors. The path selects commits touching that path; it is not a
wildcard. Commit details still list all changed paths in each selected commit.
Choose **Apply** to read the new scope.

History reads the newest 500 commits. **Load older** adds 500 more at a time, up to
2,000, using the original pinned tips, base and branch labels even if another app
moves a branch. **Refresh** starts a new snapshot. Changing scope fields disables
Load older until that scope is applied. Parent links outside a filtered or unloaded
scope are counted explicitly; they are not replaced with invented connections.
Search only examines loaded data.

History scopes with more than 12,000 parent links are refused, including while
loading older commits. The previous completed snapshot remains visible. Narrow the
branch, base or literal path scope before trying again. Every commit shows up to 240
commits at a time; **Previous / Next** browse the rest without another Git read.

## People

**Who works where** opens first: one row per person, one column per folder, and the
number of their commits touching that folder in each cell; a stronger colour means
more commits. A folder holding most of the changes is split into its subfolders so the
columns stay useful. Select a cell to see that person's commits there and the files
they changed in it; **Show commits** opens them in History, **Show folder** opens
the folder in Code. Select a person or a folder for their totals.

**All time**, **90 days** and **30 days** filter by author time within the loaded
history. **Shared files** connects people who changed the same recorded paths;
hover a connection's row in Details to see those paths. People supports the same
Path, Leave out and Load older controls, and the counts update when older commits
are loaded. Identities use the snapshot's `.mailmap`. A person's total includes
their merge commits, which change no files of their own, so it can exceed the sum of
their cells.

Shared paths mean overlapping work, not direct collaboration, ownership or a
productivity ranking. Renames count as the old and new path. Merge-only
resolutions are not assigned to a contributor. Names can be shared by different
people; **Show commits** for a person is a name search, not an identity filter.

## Navigation, limits and stopping

- In Packages, Folders, Modules and Shared files, drag to pan, scroll or use **+ / −** to
  zoom, **Focus** to centre the selection and **Fit** to see everything. With the
  map focused, arrow keys pan and **F** fits.
- **Connected only** keeps the selection and what it directly connects to.
  Choosing a connection in Details follows it. Filtering keeps cards in place.
- **Stop** cancels the current read. Previously completed results remain visible.
  A failed refresh also keeps the last result and shows the error with **Retry**.
- Results are pinned to Git object IDs. When the repository changes, the map says
  so; choose **Refresh** to read the new snapshot. Editing the revision field alone
  does not replace the snapshot on screen.
- Reads are bounded: 500 commits at a time up to 2,000, 4,096 file/folder/people
  nodes, 100,000 changed paths, 64 Cargo manifests (128 KiB each, 4 MiB total), and
  shared-file connections for up to 128 authors. History snapshots allow at most
  12,000 parent links, and at most 240 cards draw at once. A partial map says so; a
  refused read keeps the previous result and shows an error.
- Labels over 4 KiB are shortened before drawing. Full paths and object IDs remain
  intact. Details lists up to 100 rows per section and says how many more there are.
- Diagnostics record operation names, steps, durations, cancellation and errors.
  They omit repository paths, author names, commit messages and file contents.

## Export as Mermaid

The export button in the header opens the text of what the map shows, as Mermaid,
with line numbers. **Copy** puts it on the clipboard. **Save** writes a new `.mmd`
file: **Save in** starts at your Documents folder (**Browse** picks another) and the
name follows the view, such as `rocket-packages.mmd`. Existing files are never
replaced. While saving, **Stop** cancels before publication; if publication has
already completed, Rocket reports the saved file instead of claiming cancellation.

Exports include pinned commit IDs, scope, partial-data notices and the connections
in view. They hold names and paths only, never file contents, but they can contain
repository names, paths, subjects and contributor names: review before sharing.
Labels are escaped, with no active links, callbacks or embedded configuration.
Mermaid is an export format; Rocket's map remains native Rust and does not bundle or
run a Mermaid renderer.

[Back to the wiki](Home.md)
