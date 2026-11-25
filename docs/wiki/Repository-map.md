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

**Folders** is the code map. Folders start closed: open one with the arrow on its card,
or use **Expand all / Collapse all**, and it becomes a box holding its folders and
files. A folder's own files show one by one up to eight; more share one dashed card,
which opens the same way.

To trace an embedded Rust resource, expand its folder and the folder containing the
Rust source. For example, Rocket's `app_update.rs` points to the `extract.ps1` script
it includes. In **Modules**, select the script to see **Used by**, then select a
caller to trace the connection back. A shared file lists each calling module.

In **Folders** or **Modules**, select either the source file or its asset, then choose
**Go to reference** in Details. A single reference opens immediately. With several,
choose a source path and line from the list. The highlighted line shows the Rust
macro call or the HTML/CSS asset URL as it appeared in the map’s selected commit.
You can keep local edits open while reading it. Choose **Close** to return to your
map selection.

To trace a Rust native call, expand the crate in **Folders**. Select the Rust file
and follow its arrow to the C-family source. The arrow lists matching linker names.
For example, Rocket's `rocket_spell_check` connects `macos.rs` to `macos.m`.
The map follows `extern "C"` declarations, literal `#[link_name]` aliases and uses
that resolve to those declarations. It reads the committed Cargo build script and
recognizes direct `cc::Build::new().file(…).compile(…)` chains and Rocket's Cocoa
build helper with literal source paths. A C++ or Objective-C++ definition needs
explicit C linkage. Dynamic build logic, explicitly linked external libraries,
file-private functions and ambiguous definitions stay unresolved.

Arrows show which files use which. The map reads every package of the snapshot the way
Modules reads one (Rust crates with their tests, examples and programs, JavaScript and
TypeScript projects, Python packages, Go modules, Java and Kotlin, C, C++, Objective-C, Objective-C++, C#
projects, Lua, PowerShell and Bash scripts, GitHub Actions workflows, and HTML pages and stylesheets) and connects packages through the names
they import: a Cargo package's crate
name, a package.json name, another Python package or Go module of the repository, a Java
package or C# namespace another project declares (or whose folders it is laid out in),
or a C or C++ file of another project. Script loads can connect individual files across
folders. An arrow between two cards gathers every link
between the files inside them, and its number counts the files on the using side.
At Fit zoom, a count moves along its arrow if a card is in the way. When space runs
out, the arrow stays visible and you can select the card to inspect its connections.
In a compact window, fitted cards show a single-line filename when there is room.
What uses sits above what it uses; when two use each other, the arrow pointing up is dashed.
Cards that connect to nothing at their level sit under **Not connected**. One read takes
up to 512 packages, 10,000 code files and 64 MiB. Empty packages count
toward the package limit. A deferred package is read once its folder is open,
by its arrow, **Expand all** or **Show folder**, and a package with more code than one
read takes has no arrows. Go packages are read whole: an arrow from one counts packages,
and a package that uses a change has all its Go files hatched. **Stop** ends the code
reads too; **Refresh** starts them again.

**Colour** on the toolbar colours the cards; the key at the bottom left says what the
colours mean:

- **Kind**: code, tests, docs, and config and data, by size; a band along the top of a
  card shows the mix.
- **Commit activity**: commits in 30 days, 90 days or all time, merges left out, shaded
  against the busiest card shown. The newest 10,000 commits are read.
- **Owner**: the snapshot's CODEOWNERS (`.github/`, the top folder, `docs/`, then
  `.gitlab/`), read as GitHub or GitLab reads it (GitLab's rules for `.gitlab/CODEOWNERS`
  and for any file with sections); without one, the person with the most
  commits to each file, as the snapshot's `.mailmap` names them. Three owners get
  colours, the most files first; the rest are Others, and files no rule covers belong to
  Nobody. Most commits says who changed the files, not who answers for them.
- **Changes**: what the snapshot changes since it split from the branch, tag or commit
  typed in the key, as a pull request compares them. A, M and R are as in the Changes
  list; a hatched card uses a changed file, or a package that holds one. **Changed only**
  keeps those files and the folders on the way.

Details lists what the selection uses and what uses it, with the files on the using side,
and facts that follow the colour: kinds, commits and the most changed files, the owner,
or the change and its lines.

In Packages, workspace-inherited, escaped or otherwise unsupported Cargo declarations
stay unlinked. Which function calls which is not shown; macros, `cfg` and other build
constraints are not evaluated, as in Modules. Symlinks and submodules are listed as
references; their targets are not traversed. Uncommitted and untracked files do not
appear.

### Modules

**Modules** shows the modules of one package and which module uses which: a Rust
crate, a JavaScript or TypeScript project, a Python package, the packages of a Go module
or of a Java or Kotlin project, the namespaces of a C# project, the files of a C-family
project, a Lua group, the PowerShell or Bash scripts in a top-level folder, GitHub Actions workflows,
or the repository's HTML and CSS files. The button beside
**Packages · Folders · Modules** lists the snapshot's packages by language. Choose
one there, or select **Show modules** in a Cargo
package's Details. Without a choice it reads the largest package of the language with
the most files. A repository without Cargo packages shows **Folders · Modules**.

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
files past the read limits and files the reader cannot decode. Source is read as UTF-8.
PowerShell also accepts UTF-16 files with a byte-order mark. A module's Details give its file and size, what it uses and
what uses it, and **In this file**: what the file declares at its top level, public
items first, each type with its functions counted. In a narrow window the list folds
into a button above the picture.

Modules reads the committed `.rs` files as text, up to 2,000 files of 1 MiB each and
32 MiB in all. It follows `mod` declarations from `src/lib.rs` (or `src/main.rs`),
including `#[path]` files elsewhere in the repository, and connects a module to another
through `use` lines, paths starting `crate::`, `super::` or `self::`, and paths into its
own modules. The map also reads literal `include_str!` and `include_bytes!` paths
relative to their source file. A regular file in the selected commit can be a target
regardless of its size or encoding. Computed paths, symlinks and calls inside
other macros' input are skipped. Names a `use …::*` line brings in are matched by name. Macros and `cfg` are
not expanded, other packages are not followed, and which function calls which is not
shown: that needs the compiler. Nothing is compiled or run.

The other languages are read the same way, as text within the same limits:

- **JavaScript and TypeScript.** A project is a folder with `package.json`; files in
  no project make one project per top-level folder. Each file is a module and its
  folders group their files, as Rust modules do. Files are connected by `import`,
  `export … from`, `require()` and `import()` with a fixed relative path, and by the
  project's tsconfig.json (or jsconfig.json) `paths`; `./view.js` also finds
  `view.ts`. The names a card shows are the names imported, with `default` and `*`, and
  the names used through a namespace (`ns.card()`). `node_modules` and hidden folders
  are skipped.
- **Python.** A package is a folder with `__init__.py` that no other package holds; its
  `__init__.py` is the root, like Rust's `lib.rs`. Python files in no package make one
  folder of scripts per top-level folder. Modules are connected by `import` and
  `from … import` lines, absolute and relative, including those inside functions, so
  two modules can use each other.
- **Go.** A module is a folder with `go.mod`. Each package (a folder of Go files)
  stands alone: a folder inside it is another package, nothing counts as inside
  another, and the list only nests them by folder. Packages are connected by imports
  that start with the module's path and the names used through them
  (`http.Client`). Test files, `testdata`, `vendor` and files marked
  `//go:build ignore` are left out; the last are listed under Not linked. Other build
  constraints are not evaluated, so files for every platform are read together.
- **Java and Kotlin**, read together. A project is a folder with `pom.xml`,
  `build.gradle(.kts)` or `settings.gradle(.kts)` holding the files no deeper build holds,
  as Maven's modules and Gradle's subprojects are; files in no project make one per
  top-level folder. Kotlin names a project with more Kotlin files than Java ones. Each
  package (its files' `package` line) stands alone, as Go's do, nested in the list under
  the name above it; the list starts below the part every name shares. Packages are
  connected by imports (`static` and Kotlin's `as` too), by the names a file uses through a
  `*` import, and by names written out with their package. Test source sets (`src/test`,
  `src/androidTest` and the like), `module-info.java` and Kotlin scripts are left out.
- **C, C++, Objective-C and Objective-C++**, read together. The reader recognizes
  `.m` and `.mm` source files by extension. A literal `#import "Bridge.h"` or
  `#include "Bridge.h"` connects to the committed header. An SDK import such as
  `#import <AppKit/AppKit.h>` appears under **Outside** when the header is external.
  A project is the top-most folder with a CMake, Meson,
  Make or similar build (the builds deeper down are parts of it); files no build holds make
  one project. Each file is a module and its folders group their files. `#include "…"` is
  looked for beside the file, then, as include folders would find it, by the end of a path
  in the project, nearest first, then elsewhere in the repository (`<…>` only when it names
  a folder). The names on a connection are those the header declares and the file uses.
  The preprocessor is not run: macros are not expanded, `#include` lines count in every
  branch but `#if 0`, and declarations only in an `#if`'s first branch.
- **C#.** A project is a folder with a `.csproj` (`bin` and `obj` skipped); files in no
  project make one per top-level folder. Each namespace stands alone, as Java's packages
  do. Namespaces are connected by `using` lines (`static` and aliases too), by the type
  names a file uses, looked for as C# does (its own namespace, its parents, then the
  namespaces its `using` lines and the project's `global using` lines open), and by names
  written out in full. A partial class is one, its methods added up.
- **HTML / CSS.** In **Modules**, select **HTML / CSS** and choose a page to see
  the stylesheets, scripts and images it loads. Select a shared stylesheet, image or
  font to see its callers under **Used by**, including pages in other folders.
  The same arrows appear in **Folders**. The map reads literal asset URLs in committed
  UTF-8 `.html`, `.htm` and `.css` files, including image candidates in `srcset`, inline
  styles, CSS `@import` and `url()`. CSS paths resolve from the stylesheet's folder.
  A local HTML `<base href>` changes the base for that page. Query strings and fragments
  are stripped before matching, so an encoded `%23` still names a literal `#` in a file.
  Root-relative URLs require a committed `index.html` or `index.htm` at the repository
  root and a matching path there. Deployment mounts, external origins and generated
  URLs can leave references unresolved. Browsing the map reads Git objects locally.
- **GitHub Actions.** In **Folders**, open `.github/workflows`. In **Modules**,
  choose **GitHub Actions** and select a workflow or helper to follow its arrows in either direction.
  A `run` arrow points to a literal script called by a Bash or PowerShell step.
  This includes scripts passed to Node or Python and respects step, job and workflow
  working-directory settings. A `uses` arrow points to a local action's `action.yml`
  or `action.yaml`, or to a reusable workflow in the same repository. Both `./` and
  `$/` references are recognized. Workspace links under an explicitly redirected
  checkout are skipped. The `$/` form still identifies files in the workflow's commit.
  The map reads block-form jobs and steps from the selected commit. Remote actions,
  computed paths, custom shell interpreters and commands following a directory change are outside
  this lookup. [GitHub workflow syntax](https://docs.github.com/en/actions/reference/workflows-and-actions/workflow-syntax)
- **Lua.** In **Folders** or **Modules**, follow `require("ui.panel")` or
  `require 'ui.panel'` to `ui/panel.lua` or `ui/panel/init.lua`. Ordinary scripts
  share the repository root. An arrow requires a single committed match. Computed
  names, escaped strings, custom search paths and binary modules are outside this
  lookup. Files that redefine `require` or `_ENV` are skipped conservatively.
  A Rocket integration has its own group. The map recognizes API 1 manifests by
  their identity fields and permissions, and links `integration.json` directly to
  its sibling `main.lua`. The integration sandbox
  disables `require`. Browsing the map never runs the module or integration.
- **PowerShell.** Choose a PowerShell group to see which `.ps1` and `.psm1` files
  dot-source a helper, invoke a script with `&`, or use `Import-Module` or `using module`
  with a path. Selecting a script shows the actual helper files, including helpers in
  nested folders. For example, `. (Join-Path $PSScriptRoot 'lib/Build.ps1')` links to that
  helper beside the calling script. A `.psd1` manifest can be a target. Rocket reads
  it as data and leaves module names looked up on `PSModulePath` unresolved.
- **Bash and shell.** Choose a Bash group for `.sh`, `.bash` and common Bash startup
  files. Arrows follow `source` and `.`, including quoted paths and helpers located
  with `BASH_SOURCE[0]` or `dirname "$0"`. Rocket recognizes a directory variable
  assigned directly from these forms, including `cd … && pwd`. An unrelated command
  ends that inference. A bare filename searched on `PATH` has no arrow. **Folders**
  can show an explicitly sourced extensionless helper. Detecting an extensionless
  entry script from its shebang alone is outside this reader's scope.

For either language, an explicit working-directory-relative path gets an arrow when
exactly one committed file matches at the repository root or beside the caller.
After a recognized directory change only script-anchored loads are followed. Dynamic
expressions and ambiguous paths are skipped. **Outside** lists known helpers beyond
the selected group. Open **Folders** to follow those connections across the repository.
A group with more than 12,000 literal loads is marked partial. Analysis uses
committed Git objects and never runs a script.

Imports from outside the package (other crates, Node built-ins and npm packages,
Python's and PyPI's modules, other Go modules, Java packages, C and C++ libraries and
headers, C# namespaces) are listed under **Outside** in a
module's Details, and counted in the package's. They are not drawn. Paths built while a
program runs, bundler aliases other than tsconfig paths and `sys.path` changes are not
followed.

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

**Who works where** opens first: one row per folder, one column per person, and the
number of the person's commits touching that folder in each cell; a stronger colour
means more commits, and the key beside the title gives the range. A folder holding most
of the changes is split into its subfolders, each shown with its path (for example
`crates/rocket-git`), so the rows stay useful. With two or more people the folder column
also gives each folder's commits, by everyone in the period. The folder column stays in
place while the table scrolls, and so does the people row when there is room for rows
under it; pointing at a cell lights its row and column. Select a cell to see
that person's commits there and the files they changed in it; **Show commits** opens
them in History, **Show folder** opens the folder in Code. Select a person or a folder
for their totals.

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

Rust `#[path]` follow-ups share the same 10,000-file / 64 MiB source-read budget.
CODEOWNERS has a separate global matching-work budget, checked inside wildcard
matching as well as between files. If it is exhausted, Rocket reports the limit
and falls back to commit-based ownership; incomplete ownership is not displayed.
Cancellation interrupts matching and follow-up reads.

- In Packages, Folders, Modules and Shared files, drag to pan, scroll or use **+ / −** to
  zoom, **Focus** to centre the selection and **Fit** to see everything. With the
  map focused, arrow keys pan and **F** fits.
- **Connected only** keeps the selection and what it directly connects to.
  Choosing a connection in Details follows it. Filtering keeps cards in place, except
  in Folders, which lays the kept cards out again.
- **Stop** cancels the current read. Previously completed results remain visible.
  A failed refresh also keeps the last result and shows the error with **Retry**.
- Results are pinned to Git object IDs. When the repository changes, the map says
  so; choose **Refresh** to read the new snapshot. Editing the revision field alone
  does not replace the snapshot on screen.
- Reads are bounded: 500 commits at a time up to 2,000, 4,096 file/folder/people
  nodes, 100,000 changed paths, 64 Cargo manifests (128 KiB each, 4 MiB total), and
  shared-file connections for up to 128 authors. History snapshots allow at most
  12,000 parent links, and at most 240 cards draw at once (400 cards and boxes in
  Folders, which reads up to 10,000 code files and 64 MiB at a time, every manifest
  up to 128 KiB and 16 MiB together, and the newest 10,000 commits). A partial map
  says so; a refused read keeps the previous result and shows an error.
- Labels over 4 KiB are shortened before drawing. Full paths and object IDs remain
  intact. Details lists up to 100 rows per section and says how many more there are.
- Diagnostics record operation names, steps, durations, cancellation and errors.
  They omit repository paths, author names, commit messages and file contents.

## Export as Mermaid

**Export** at the right of the header opens the text of what the map shows, as Mermaid,
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
