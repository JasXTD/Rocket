# Repository map

Open **Map** in the sidebar, or find **Repository map** in the command palette.
The three views use your local repository. They do not fetch, switch branches,
run project code, or change working files or the staging area.

## Code

Enter a commit, branch or tag and choose **Refresh**. The file view connects
committed folders to their contents, including Unicode paths. **Cargo packages**
shows packages with literal local `path` dependencies from committed Cargo
manifests. An arrow points from the dependent package to its dependency.

This first version does not infer every language's imports. Workspace-inherited,
escaped or otherwise unsupported Cargo declarations stay unlinked. Symlinks and
submodules are listed as references; their targets are not traversed. Uncommitted
and untracked files do not appear.

In the file view, **Collapse folders** groups files under their folder/package
directories. Select a folder and use **Collapse group / Expand group** in Details,
or **Expand all** to restore the tree. Connections crossing a collapsed folder are
shown between the groups; hidden internal edges are omitted. Search can find a
file inside a collapsed folder and show its containing group for expansion.

## Git history

Arrows connect parents to children, including multiple parents of a merge.
**All branches** includes local and remote-tracking branch tips already present
on disk. **Merge context** focuses on merge commits and their visible neighbors.
Search finds subjects, authors, object IDs and recorded changed paths in the
loaded history. Select a commit and choose **Open commit** for its diff.

Open **Scope** to choose an optional excluded base and a literal file/folder path.
The scope contains commits reachable from the revision (or selected all-branch
tips) excluding the base and its ancestors. The path selects commits touching that
path; it does not act as a wildcard. Commit details still list all changed paths
in each selected commit. Choose **Apply scope** to read the new scope.

**Load more** adds up to 500 commits to the loaded snapshot, with a 2,000-commit
budget. It uses the original pinned tips, base and branch labels even if another
app moves a branch. **Refresh** starts a new snapshot. Changing scope fields
disables Load more until that scope is applied. Parent links outside a filtered
or unloaded scope are counted explicitly; they are not replaced with invented
ancestor connections. Search only examines loaded data.

History scopes with more than 12,000 parent links are refused, including during
Load more. The previous completed snapshot remains visible. Narrow the branch,
base or literal path scope before trying again.

The graph shows up to 240 nodes per section. **Previous nodes / Next nodes**
browse all loaded nodes without another Git read. Search resets to its first
section; Load more extends the loaded data without changing the pinned snapshot.

## People

Authors connect to the top-level areas they changed. **Shared files** connects
authors who changed the same recorded paths. Select an author or connection to
see counts and paths. **Last 30 days** filters by author timestamp within the
loaded history; identities use the selected commit's `.mailmap`.

People supports the same Scope and Load more controls. The 30-day filter applies
to author timestamps in the loaded commit scope; a partial history may omit
activity within those dates. The counts update when more commits are loaded.

Hover over a connection in Details to see its recorded changed/shared paths.

Shared paths mean overlapping work, not direct collaboration, ownership or a
productivity ranking. Renames count as the old and new path. Merge-only
resolutions are not assigned to a contributor. Names can be shared by different
people; **Find author in history** is a name search, not an identity filter.

## Navigation, limits and stopping

- Drag to pan, scroll or use **+ / −** to zoom, **Focus** to read the selected
  node, and **Fit** to see the overview. With the canvas focused, arrow keys pan
  and **F** fits. Search can find nodes beyond those initially drawn.
- **Neighbors** shows the selected node and its direct connections. **All
  connections** restores the current scope. Choosing a connection in Details
  follows that node's neighborhood. Filtering keeps retained nodes in their lanes.
- On compact windows, **Details** opens a drawer. Close it to use the graph.
- **Stop** cancels the current read. Previously completed results remain visible.
  A failed refresh also keeps the last result and shows an error.
- Results are pinned to Git object IDs. External changes mark an existing map
  for review; choose **Refresh** to read the new snapshot. Editing the revision
  field alone does not replace the displayed snapshot.
- Reads are bounded: initially 500 commits (Load more up to 2,000), 4,096 file/folder/people nodes, 100,000 changed
  paths, 64 Cargo manifests (128 KiB each, 4 MiB total), and shared-file edges for
  up to 128 authors. History snapshots allow at most 12,000 parent links. At most
  240 nodes are drawn at once. Partial-map indicators describe incomplete scopes;
  a refused read keeps the previous result and displays an error.
- Labels over 4 KiB are shortened before rendering. Full paths and object IDs
  remain intact. The inspector shows the first 16 references and 24 changed paths.
- Diagnostics record operation names, steps, durations, cancellation and errors.
  They omit repository paths, author names, commit messages and file contents.

## Mermaid export

Choose **Export Mermaid** to review the visible nodes and connections as plain
Mermaid text. **Copy text** puts it on the clipboard. To save, enter the absolute
path to a new `.mmd` file in an existing folder, then choose **Save new .mmd**.
Existing files are never overwritten. **Close** cancels the review without writing;
**Stop export** cancels preparation before publication. If publication has already
completed, Rocket reports the saved file instead of claiming cancellation.

Exports include pinned commit IDs, scope, partial-data notices, and the current
visible connections. They can contain repository names, paths, subjects and
contributor names: review before sharing. Labels are escaped, with no active
links, callbacks or embedded configuration. Mermaid is an export format; Rocket's
map remains native Rust and does not bundle or run a Mermaid renderer.

[Back to the wiki](Home.md)
