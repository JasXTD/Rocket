# Rocket Lua API 1

This is the developer contract for Rocket's extensions. User setup: [Extensions
wiki](docs/wiki/Integrations.md). Examples: [PlayGround](crates/rocket-integrations/examples/playground/)
and [Jira Cloud](crates/rocket-integrations/examples/jira/), which both ship with Rocket. The
manifest and script are UTF-8 text. In Preferences → Extensions choose **Install from file**
and pick the package's `integration.json`, or copy the folder into the extensions folder
shown under Developer tools and reload; either way review and enable it. Names, labels and
values support Unicode; control IDs use ASCII letters, digits, `_` and `-`, up to 64 bytes.

The on-disk names keep their first spelling: `integration.json`, the `integrations` folder,
`integration-accounts.dpapi`, the `integration_*` error codes and the `int-*` automation IDs.

## API stability

`"api": 1` is a promise to people writing extensions. API 1 can grow, but nothing in this
document is removed or changes meaning under that number, and a later Rocket keeps loading
API 1 packages. A change that would break a package takes a new number: Rocket refuses a
package for a newer API with `integration_api`, naming the API it needs, rather than running
it in part. The manifest refuses unknown fields, so a package using an addition shows as
invalid in a Rocket older than that addition.

**Open Lua playground** (Developer tools in the extensions list, or **Lua playground** in the
command palette) runs source directly in the bounded worker with an offline
host. It renders the same `Node` schema as installed packages. `rocket.http`
returns a request description for inspection; the scratch host never supplies
credentials or makes an HTTP request. Keep network-dependent logic in an installed
package with explicit permissions. Source saves are protected locally, separate
from package connection settings; external modifications reject replacement.

## Package

```json
{
  "api": 1,
  "id": "my-dashboard",
  "name": "My dashboard",
  "version": "1.0.0",
  "description": "A local native panel",
    "permissions": {"network": false, "write": false, "repository": false},
  "settings": [{"id":"project","label":"Project","default":"DEMO"}],
  "commands": [{"id":"refresh","label":"Refresh dashboard"}]
}
```

Unknown manifest fields/permissions and duplicate IDs are rejected. Maximum 64
folder entries, 32 KiB manifest, 64 KiB source and 16 settings. Symlinks/junctions
are refused. Invalid packages are listed independently; healthy packages stay
available. Every copy of a duplicate ID stays off until one is removed. A package has
no icon: Rocket draws the first letter of its name. Each ID supports up to 16 named
connections. Installation and discovery do not execute the script. Scripts are never
installed from repository contents.

## Install, update and remove

A package is the two files. **Install from file** reads the chosen `integration.json` and
the `main.lua` beside it, and nothing is copied until the review is accepted. What lands
is the bytes the review showed, in one rename, whatever the chosen files say by then. A
package installs into a folder named after its ID.

Approval is exact and covers the extension: one review writes the same manifest and source
snapshot to each connection that is enabled or has a server, or to the selected connection
when none has. Editing either file, or updating, asks again before the extension can run.
**Allow writes** and **Share branch, commit IDs and change counts** stay per connection. A
review shows new permissions, counts changed lines against the approved copy and shows them.
Turning an extension off keeps its approval, so turning it on again asks only if files
changed. The Enable switch reads the connections' saved state, so the storage format is
unchanged: an extension is on when any connection is enabled.

A file whose ID matches an installed package opens the update review, which replaces that
package's folder and keeps its connections, on or off as they were; a copy that changed since
the review stops the update. A file cannot use the ID of a bundled extension, so it cannot inherit saved tokens:
give a modified copy its own ID. A newer bundled copy offers **Update to** its version and
never replaces files by itself. **Remove** deletes the folder and the extension's
connections, tokens and drafts, and waits while a request may have reached the service.

Optional `permissions.repository` requests read-only metadata. It also needs the
native **Share read-only branch, commit IDs and change counts** setting for each
connection; old connections default to off. Turning the grant off invalidates saved
draft bindings; turning it on keeps them, because sharing more can't expose what a
draft holds. An enabled network package can transmit shared values to its
configured origin, so grant only to trusted integrations.

## Callback and state

`main.lua` returns a function. Rocket calls it for Refresh (`event="open"`) and
button activation (`event` is that button's ID).

A click on a table row calls it with `event` set to the table's ID and `ctx.row` to that row's
key; choosing a segment calls it with `event` set to the segments' ID. A command calls it with
`event="command"`; see [Commands](#commands).

```lua
return function(ctx)
  local count = ctx.state.count or 0
  if ctx.event == "increment" then count = count + 1 end
  return {
    state = { count = count },
    view = ui.page { title = "My dashboard", children = {
      ui.input { id="name", label="Name", value="Rocket" },
      ui.button { id="increment", label="Update", primary=true },
      ui.text("Hello " .. (ctx.values.name or "Rocket") .. " · " .. count)
    } }
  }
end
```

`ctx.settings` contains manifest setting values. `ctx.values` contains the current
form controls. `ctx.state` is the last successful returned state, an object limited
to 32 KiB. `ctx.confirmed` is true from the moment the service confirms a reviewed request
until the next callback finishes, so a form can close. The last successful native view,
current form and script state are
saved in protected local storage for this exact package/connection. Switching
packages/connections or restarting restores cached data without calling Lua or
HTTP. Tokens, account identifiers, repository paths
and repository contents are not provided in this context. Return a UI node directly
when no custom state is needed. A failed/cancelled callback preserves the last view.

When repository access is both requested and granted, `ctx.repository` contains a
fresh action-start snapshot: `branch`, `head`, `selected_commit`, `detached`,
`unborn`, `bare`, and `changes` with `staged`, `unstaged`, `untracked`, `conflicted`
counts. Missing values become Lua nil. Bare repositories have no `changes`; no
open repository or no permission means no `repository` value. The selected commit
is the UI selection, not necessarily HEAD. Reads run off the UI thread, support
cancellation and reject a checkout change observed during the read. The result is
a snapshot, not an atomic lock over external file edits. Refresh to observe changes.
Paths, remote URLs, author identities, commit messages and file contents are absent.
The [offline repository example](crates/rocket-integrations/examples/repository/)
shows this API without a network permission or any Git write capability.

Drafts are bound to exact source/manifest bytes and connection identity/settings.
Changing these keeps the old draft protected but blocks execution with it; restore
the original files/settings or explicitly clear the draft. Renaming a connection
does not change its identity. Re-entering the same session-only token after restart
retains its binding through a protected SHA-256 digest; the token itself is not
stored unless Remember token is enabled. Credential replacement invalidates the
old draft binding. Request previews are never saved or replayed.

Form changes are saved off the UI thread after 400 ms without edits, or after two
seconds of continuous edits; navigation/actions flush pending changes first.
Inputs remain editable during the disk save. A normal exit drains pending saves;
failed saves keep the window open. A crash can lose edits made since the last
successful save. External/corrupt files are preserved and require explicit recovery.
Limits per draft: 128 KiB form values, 32 KiB state, 768 KiB including saved view
and source binding. The complete protected connection store remains bounded to
8 MiB. Saving a draft never performs a remote write.

Existing version-1 profiles load as Default connections; migration is written only
on a successful local save. Version-2 storage is not readable by older Rocket
builds; they must refuse it rather than overwrite it. Clearing/removing a draft is
explicit and local. Removal cannot erase an unresolved remote-outcome guard.

## Commands

A package can declare up to 8 commands in its manifest:

```json
"commands": [
  {"id": "create-issue", "label": "Create issue"},
  {"id": "find-commit", "label": "Find issues for this commit", "target": "commit"},
  {"id": "open-branch", "label": "Open issue for this branch", "target": "branch"}
]
```

`id` follows the rules for control IDs and is unique in the package. `label` is one line of
up to 64 bytes. `target` is optional and is `commit` or `branch`; it needs
`permissions.repository`. A package says what a command acts on and Rocket decides where it
shows: it can't choose a menu, a position or a key, and a script can't add or change a row.

Every command is in the command palette as `Name: Label`. Without a target it is also in
**Repository → Extensions**. With a target it is in the right-click menu of commits (Commits and
File history) or of branches (the sidebar and Branches), in a submenu named after the extension.
Files and submodules have no commands, because paths never reach Lua. A palette command uses the
selected commit, or the checked-out branch.

Choosing a command opens the extension's page and calls the callback there, in the same worker
with the same limits as a button, on the connection selected on that page:

```lua
return function(ctx)
  if ctx.command == "find-commit" then
    return find_issues(ctx.target.commit:sub(1, 7))
  end
  return main_view(ctx)
end
```

`ctx.event` is `"command"` and `ctx.command` the command's ID. `ctx.target` is
`{kind="commit", commit=<full ID>}` or `{kind="branch", branch=<short name>}`, and is absent for
a command without a target. It is the row that was chosen, kept from that moment, not the current
selection. A target is a commit ID or a branch name, so a command with one needs the connection's
**Share branch, commit IDs and change counts**; until then the row offers that setting instead.

A command ends like any callback: in a view, or in one write that stops at the request review.
Rocket never runs a command by itself, and opening a menu runs no Lua. Menus show the commands of
the approved copy, under its name. While the files differ, a network extension has no server, or
a saved draft no longer fits, each surface offers **Review**, **Connect** or the page with **Clear
draft** in their place, and the palette keeps the commands greyed with their reason. A review
compares commands by ID and marks them New, Changed or Removed.

## UI description

All functions below return data; Rocket owns rendering and event dispatch.

| Constructor | Fields |
| --- | --- |
| `ui.page`, `ui.column`, `ui.card` | `title`, `children`; a column with `small` keeps its children close |
| `ui.row` | `children`; wraps in narrow windows |
| `ui.split` | two `children`, side by side; optional `focus`, 1 or 2. A narrow window shows them as tabs named after their cards, opening on `focus` |
| `ui.heading` | `text`, or pass a string |
| `ui.text` | `text`, or pass a string; optional `tone`, `small` |
| `ui.code` | `text`, or pass a string; a monospace block |
| `ui.badge` | `text`, `tone`: neutral/accent/success/warning/error |
| `ui.button` | unique `id`, `label`, optional `primary`, `disabled` |
| `ui.input`, `ui.multiline` | unique `id`, `label`, initial `value`, optional `disabled`; `ui.input` also `placeholder`, and `submit`, an event Enter runs |
| `ui.segments` | unique `id`, up to 8 string `options`, `value` (the script decides what shows as chosen), optional `disabled` |
| `ui.checkbox` | unique `id`, `label`, boolean `value`, optional `disabled` |
| `ui.select` | unique `id`, `label`, string `options`, initial `value`, optional `disabled` |
| `ui.table` | string `columns`, array of `rows`; a cell is a string, or `{text, tone, mono}`, which draws a pill when it has a tone. With `id` and one unique string per row in `keys`, rows can be clicked; `selected` names the chosen key. In a narrow window a table becomes cards, and one with keys keeps its rows on two lines |
| `ui.separator` | no fields required |
| `ui.progress` | numeric `value` from 0 to 1, `label` |

A page with a `ui.split` uses the full width of the window; other pages stop at 960 points.

Values entered by the user remain until the panel resets; changing `value` in a
later callback does not overwrite user input. Removing a control from a returned
view removes its stored field value; use explicit `state` for cross-page data.
User input is limited to 4,096 characters per field. Select/checkbox/input changes are
submitted on the next callback. Limits: 256 nodes, 12 nested layout levels,
512 KiB view, 200 rows, 8 table columns, 64 select choices. Controls are namespaced
for automation. No script controls trusted permissions, review, stop or recovery UI.
If a callback changes a field's type or choices, an incompatible old value is
discarded before saving/dispatching the new view. Unchanged compatible inputs remain.

## HTTP and JSON

```lua
local response = rocket.http {
  method="GET", path="/api/items?project=" .. rocket.url_encode(ctx.settings.project)
}
local name = response.json.name
-- A write creates a proposal; the native host asks the user before sending it.
rocket.http { method="POST", path="/api/items", body={name="Example"} }
```

Methods: GET, POST, PUT, PATCH, DELETE. Paths must be ASCII relative paths beginning
with `/`, with no `//` origin override, whitespace, control characters, fragments
or backslashes. Use `rocket.url_encode` for UTF-8 path components/query values.
No custom headers, arbitrary URLs or redirects. GET has no body. The native host
adds JSON content headers and the configured bearer/basic credential.

The host replays a callback from its original context after each GET, caching the
exact request's response. Keep callbacks deterministic; random functions are absent.
At most eight GETs per action, 1 MiB per response, 4 MiB response cache, 256 KiB
write body. The response has `status` and parsed `json`. Non-success status/invalid
JSON stops the action with a native error. Only the approved HTTPS origin is used;
HTTP is permitted solely by the native marked loopback test host.

A write ends the action at a proposal. After confirmation, Rocket sends it once,
records the outcome and asks the user to refresh. Code after the write does not
execute as a post-submit callback. There is no automatic retry. Do not use mutating
GET endpoints. Remote services enforce their own authorization and concurrency;
this API does not automatically prevent a remote user editing the same item.

A write can include one optional preflight condition:

```lua
rocket.http {
  method="PUT", path="/rest/api/3/issue/ROCKET-42", body={fields={summary="Reviewed edit"}},
  check={path="/rest/api/3/issue/ROCKET-42?fields=updated",
         pointer="/fields/updated", expected=ctx.state.issue.updated}
}
```

`check.path` is a GET on the same approved origin; `pointer` is a JSON pointer of
up to 256 bytes and `expected` a non-null string/number/boolean of up to 4 KiB.
Rocket compares the response before showing a review and again before sending.
A missing/different value returns `integration_stale`, preserves draft fields and
sends no write. The first check consumes one of the action's eight reads; the
confirmation makes one additional check. Cancelling that read sends no write.
Once a write may have been sent, the existing uncertainty guard still applies.
This is **not an atomic conditional write**: another service user can change data
between the GET and the write. Use a service-side version check when its API supports
one. The native review shows the check as well as the exact write payload.

`rocket.json.encode(value)` / `decode(text)` support bounded plain data. Empty Lua
tables encode as JSON objects; a non-empty contiguous integer table encodes as an
array. JSON null becomes Lua nil; avoid relying on null-containing arrays retaining
their shape. Functions, userdata, cycles, mixed numeric/string tables and nonfinite
numbers cannot cross the boundary.

## Runtime and safety limits

Bundled Lua 5.5.1 runs in `rocket-lua-host.exe`, never in a UI callback. The worker
uses sanitized environment variables and has a 32 MiB Lua allocator limit, two
million instruction budget and five-second native deadline. Conversion to native
JSON has a separate 1 MiB aggregate budget so shared Lua data cannot expand without
bound. HTTP helpers have their own time/output limits and are cancelled with the
existing process owner.

Available standard helpers: assert, error, ipairs, next, pairs, pcall, select,
tonumber, tostring, type, rawequal, rawlen, table, string, math and utf8. No os, io,
debug, package, require, load/loadfile/dofile, coroutine, collectgarbage, metatable
access or bytecode. `string.dump`, math.random/randomseed and print are absent.
Windows also isolates every invocation in a fresh Less Privileged AppContainer
(LPAC), with no capabilities, a 128 MiB process memory limit, a five-second CPU
limit, and no child processes. Only input/output pipes and a read/execute copy of
the verified worker are supplied. Direct network access and private user-file
access are denied; approved HTTP requests still pass through Rocket's native broker.
The worker refuses direct execution outside this container. Unsupported Windows
policies and other platforms fail closed; there is no unrestricted fallback.

Rocket compares the worker to the SHA-256 embedded in the host during the build,
holds the source and staged image against modification, and rejects mismatches
before execution. Unsigned executables are supported without a certificate. A
hash binds the worker to its host; it cannot authenticate a host that was itself
replaced. Review scripts and their requested access: the sandbox cannot prevent
misuse of data deliberately supplied through the approved API or guarantee against
every native VM or operating-system vulnerability.

Settings are separate DPAPI-protected local data. Source/config errors, process
steps and HTTP failure categories enter diagnostics; credentials, Lua source,
request URLs/bodies and response payloads do not. Raw Lua errors are omitted because
they may include private values. Syntax/runtime errors identify the package phase
and numeric `main.lua` line when available;
check your script and API contract locally. Damaged or externally edited settings
are preserved and require an explicit reload/recovery.

Every settings save holds an OS lock on the persistent
`integration-accounts.lock` sidecar from the expected-byte comparison through
replacement. Competing Rocket windows receive `integration_store_busy` and retain
their drafts; retry after the other save finishes, or reload if the snapshot changed.
The lock is released on normal or abrupt process exit. Do not delete the sidecar
while Rocket is running. Close older Rocket processes before using the fixed build;
older builds do not participate in this locking protocol.

HTTP redirects are never followed. A redirect after POST, PUT, PATCH or DELETE is
`integration_outcome_unknown`: the service may already have applied the write.
The protected uncertainty guard survives restart and blocks another write until
you inspect the remote result and explicitly acknowledge it. GET redirects remain
`integration_redirect` errors.

## Development

Open **Developer tools** in an extension's details. **Validate Lua**
reads the current package files and compiles source in the bounded worker, even
when disabled. It never initializes the module, invokes its callback, reads Git or
uses HTTP. Syntax validity does not establish valid callback behavior or service
compatibility. Validation does not adopt changed source or renew approval; reload
and explicitly enable it separately. The local panel shows the latest 32 action
results across connections, filtered to the selected connection, with codes and
durations. Clearing this view does not delete durable diagnostic logs or drafts.
No raw Lua exception, credential, form or response payload enters the view/logs.

Build the worker with the app:

```powershell
.\tools\Build-Local.ps1 -Offline
cargo test --offline --locked -p rocket-integrations --features runtime
node tools/tests/test-mcp-lua-integrations.cjs
node tools/tests/test-mcp-integration-profiles.cjs
node tools/tests/test-mcp-integration-workflows.cjs
```

The MCP runner uses temporary dirty repositories, synthetic credentials and a
loopback Jira-shaped API. It does not authenticate to Jira or any real service.
Keep `rocket-lua-host.exe` beside Rocket/MCP when distributing a build. Ship
`THIRD-PARTY-NOTICES.txt`; Lua's C source license is included in generated notices.

## Native runtime trust review (2026-10-29)

The Lua 5.5.1 post-release GC-parameter and coroutine-close fixes reference APIs
that Rocket does not expose: collectgarbage, coroutine and metatable access.
The existing runtime regression checks their absence and rejects bytecode.
This narrows those particular entry points; it does not prove that every native
VM defect is unreachable. Numeric-loop/negative-zero correctness fixes remain
upstream. The Windows LPAC described above now adds OS isolation around the native
runtime; it does not claim to patch those upstream issues. Use a matching complete
bundle and review requested permissions. No signing certificate is required.
