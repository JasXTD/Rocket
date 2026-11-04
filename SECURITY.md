# Rocket security boundaries

This security remediation follows the findings in Codex chat
`01a14039-a14f-7c23-a3ce-1f9a89ebbcc6`. It applies to the current source;
see the reports for the first pass in `work/security-remediation-20261023/REPORT.md`
and the follow-up in `work/security-remediation-20261024-v2/REPORT.md` for tested
artifacts and limits.

## History map availability (0.91.1, 2026-11-04)

An aggregate 12,000-parent-link budget now bounds each history snapshot before
its connections can reach the UI. This covers initial reads and pinned Load more;
the existing per-commit parent and total commit limits alone did not bound the
combined graph tightly enough. An excessive scope returns an explicit error and
retains the last completed map. Narrowing the branch, base or literal path scope
can reduce it. Parent links are never silently dropped to make a scope fit.

History layout indexes parent connections and active/free lanes, preserving
first-parent ordering without scanning the whole edge collection for every node.
Regression evidence, including synthetic dense Git history and native acceptance,
is in `tools/maintenance/security-fixes-20261104/REPORT.md`.

## Privilege and npm coverage follow-up (0.89.2, 2026-11-02)

One Windows primary-token check now protects the desktop and MCP entry points,
operational CLI commands, independent helpers and every child-process launcher.
It rejects elevation, high/system integrity and failed or malformed token queries.
Normal filtered administrator-account tokens remain usable. The shared std-only
implementation is also compiled directly into the terminal helper, with an
explicit build dependency so edits cannot leave that helper stale. Git/custom
commands, automatic bisect, external tools, terminal/OpenSSH sessions, browser and
Explorer launches, MCP startup, Lua worker creation and update handoff/apply all
use this boundary. The built-in Command Prompt passes `/d` to disable AutoRun.

The desktop shows a small refusal window before initializing profiles,
repositories or update plans. Reopen it normally. This does not require signing
Rocket executables; unsigned local builds retain the existing Lua LPAC sandbox
and the worker SHA-256 embedded in their hosts. Owners can still modify/rebuild
the application; this policy is not a boundary against the machine owner.

Before reading a scan cache or starting OSV, npm coverage now rejects source
metadata that the pinned OSV 2.7.0/SCALIBR extractor would classify as unevaluated
local data for a registry-shaped dependency. Nonempty text alone is insufficient.
HTTP(S) URLs require a valid authority and escapes; validation does not repair or
rewrite the lockfile. Recognized Git categories and explicit workspace/local
dependencies retain their existing behavior. This is coverage validation, not an
allowlist for executing or downloading those URLs; scans remain offline. Some
unusual URLs outside the conservative validator's supported subset report
incomplete coverage instead of a clean result.

An enabled optional pre-push check applies the same rule to the committed tip.
Unsupported source metadata stops that attempt before publication, including when
the working copy contains a valid replacement. Existing scan preferences and
warning approval policy are unchanged. Evidence and runtime limits are recorded
in `tools/maintenance/security-fixes-20261102/REPORT.md`.

## AI API-key guard

**Edit → Preferences → Security → Guard AI API keys** is a separate, default-on
setting. Disabling general Secret protection does not disable this guard, and
disabling this guard does not disable general credential-assignment checks.
Each desktop profile saves the choice; existing profiles start with the new
guard enabled. Explicit opt-out uses profile format v25, which older Rocket
builds reject instead of silently losing the setting. MCP reports both settings
but cannot change them. The standalone CLI enables both and has no bypass flag.

The shared content scan checks messages, staged index contents, imported commit
series and all outgoing immutable history, including deleted files in ancestors.
The same policy is applied before GitHub publication creates the remote repository.
A refusal uses `ai_api_key_detected`; reports contain the location and rule, not
the matched value. Cancellation, object/byte limits and index/history rechecks
are shared with general Secret protection. Matching is offline: Rocket never
submits a candidate to an AI provider to determine whether it works.

Distinctive complete formats are recognized for OpenAI legacy/project/service/
admin keys, Anthropic API/admin keys, Groq, xAI, OpenRouter, Hugging Face and
standard Google `AIza` API keys (also used by Gemini). Prefix, alphabet, payload
length, marker and boundary checks prevent matching a prefix alone or a slice of
a larger identifier. Low-variation repeated filler is ignored. Ordinary environment
references and placeholders are not treated as complete keys; realistic synthetic
keys intentionally remain blocked, even in tests, comments or examples. UTF-8 and
UTF-16 text use the same checks. There is no project-name or test-file allowlist.

DeepSeek and GLM/Z.ai do not have equally distinctive public fingerprints. This
guard therefore checks their conservative shapes only in credential assignments,
including `DEEPSEEK_API_KEY`, `ZAI_API_KEY`, `GLM_API_KEY`, `ZHIPUAI_API_KEY` and
generic `api_key`/`token`/`secret` fields. DeepSeek matching uses `sk-` followed by
32 hexadecimal characters; GLM/Z.ai matching uses a hexadecimal ID and a URL-safe
secret separated by one dot. These are bounded heuristics, not provider contracts
or proof of ownership. Hashes and dotted identifiers elsewhere are left alone.

Provider formats change. Unrecognized versions, computed/escaped credentials,
short or low-variation values and newer Google authorization-key formats can be
missed. General Secret protection supplies overlapping coverage for credential
assignments. Neither switch guarantees that a repository contains no secrets.

Format evidence: [OpenAI key safety](https://help.openai.com/en/articles/5112595-best-practices-for-api-key-safety),
[Anthropic authentication](https://platform.claude.com/docs/en/cli-sdks-libraries/cli/authentication),
[DeepSeek API](https://api-docs.deepseek.com/),
[Z.ai authentication](https://docs.z.ai/api-reference/introduction),
[GLM HTTP authentication](https://docs.bigmodel.cn/cn/guide/develop/http/introduction),
[xAI authentication](https://docs.x.ai/developers/rest-api-reference/management/auth),
[Hugging Face tokens](https://huggingface.co/docs/hub/security-tokens),
[Google API keys](https://aistudio.google.com/docs/api-key) and
[OpenRouter authentication](https://openrouter.ai/docs/api_reference/authentication).
Published detector rules from [Gitleaks](https://github.com/gitleaks/gitleaks/tree/master/cmd/generate/config/rules)
and [TruffleHog](https://github.com/trufflesecurity/trufflehog/tree/main/pkg/detectors)
were used as format references, not installed or copied as dependencies.

## Local secret protection

Protection is **on by default**. Users can switch **Edit → Preferences → Security →
Secret protection** off or on. The setting is saved per desktop
profile, applies to already-open repositories and cannot change while a write is
running. Turning it off shows an amber warning. Existing saved profiles migrate
with protection on. MCP exposes the current setting but cannot toggle it. Its
isolated profile is separate from the normal desktop profile. The standalone
CLI also defaults on; it has no per-command bypass and does not read desktop
settings.

Rocket checks explicit commit/amend messages and the **staged index**, including
staged content that differs from the working copy. Push checks traverse the raw
objects reachable from each pinned source, including parents, trees, blobs and
annotated tags. Replacement refs, shallow boundaries and deleting a secret from
the latest snapshot do not hide older content. Regular, force, atomic and tag
pushes use the same check. Ref deletion publishes no objects. Already-remote
ancestry is scanned conservatively too, so an older secret can block a later push.

Commit messages use a narrow prose rule for ambiguous `token:` labels: bare
40/64-character hexadecimal object IDs and PascalCase type references may pass.
Explicit assignments such as `token=...`, specific labels such as `github_token:`
or `password:`, and recognized code/config snippets retain the credential check.
GitHub-token and private-key rules still apply throughout the message. The same
policy applies to historical commit bodies on push; files, filenames and commit
headers keep the stricter content rules. A hash-like or type-like unprefixed secret
in that ambiguous prose form cannot be reliably distinguished from a reference.

Detection covers current prefixed GitHub token families, PEM private-key material
and conservative credential assignments, locally without contacting a provider.
Generic assignments require a credential-like literal with sufficient character
variation; names such as `token` or `password` alone are not evidence. Ordinary
type/variable references, UI labels and template references are ignored. This
reduces accidental blocks but can miss short, low-variation or word-like generic
passwords. Quoted literals admit all non-whitespace ASCII punctuation, including
colons and escaped quotes. A leading `$`, `%` or `<` alone is not a placeholder
exemption. Complete references such as `${NAME}`, `${NAME:-}`, `$env:NAME`,
`%NAME%`, `<name>` and identifier-based `{{ ... }}` / `${{ ... }}` templates
remain supported. Bare `$NAME` remains a reference in unquoted source; quoted
bare-dollar values receive the normal literal heuristic. The check is lexical,
not a language parser, and does not evaluate templates or environment variables.
The bounded assignment matcher recognizes common colon type annotations and Go
`var name TYPE` / `:=` bindings. Literal matching includes Rust/Python raw and
byte prefixes, Rust hash delimiters, backticks and C# verbatim strings. Lookahead
is limited to 512 bytes, with bounded type nesting; this is not complete language
syntax support. It does not follow arbitrary expressions or computed credentials.
Quoted configuration values are not reinterpreted as type annotations, and an
annotation cannot consume an adjacent field, comment or statement. Incomplete
literals remain candidates; CRLF line boundaries do not become credential bytes.
For ambiguous `token` fields, complete quoted protocol identifiers such as
`REQUEST_ACCEPTED_V2:local-worker` are classified by structure and component
entropy, without application/event/provider allowlists. The form requires at least
two uppercase tag words, an optional short version, and lowercase label words.
Each word retains the 3.5-bit character-entropy bound; the combined words use a
separate 4.0-bit bound because ordinary compound names contain more distinct
letters. The credential threshold remains 3.5. Both bounds apply only after full
case, separator and component-length matching; they are not a dictionary of names.
One terminal escaped `\n` or `\r\n` is allowed inside the complete literal for
line-oriented protocol messages. This does not decode other escapes, trim
arbitrary whitespace or accept repeated terminators; values that do not match
are checked unchanged by the credential heuristic.
Long or high-entropy components, mixed-case/numeric payloads, malformed separators,
concatenations and incomplete literals do not receive this classification.
Password/auth/API-key fields retain their credential checks. This is a heuristic,
not proof of a value's purpose. Other findings in the same file still scan.
The same rule applies to staged content and reachable history; there is no
test-file exemption, history rewrite or already-public-history bypass.
Ambiguous `secret`/`token` constant declarations also recognize short diagnostic
markers: at least two uppercase alphabetic words, separated consistently by
hyphens or underscores, followed by a 4–8-character lowercase hexadecimal ID
containing both a letter and a digit. Label words have 2–24 characters and use
the same component/aggregate entropy bounds above. Word-only labels also qualify
when they contain one contiguous group of at least two uppercase words, with
lowercase namespace or annotation words before and/or after it, and use a single
separator throughout. Their aggregate bound applies to case-folded letters; a mixed-case
word, numeric component or opaque alphabet does not gain this classification.
Both marker forms require an explicit
line-leading `const` declaration (optionally Rust visibility or `export`), a
complete quoted literal and a statement boundary within the bounded lookahead.
Ordinary assignments, configuration fields, password/API-key fields, longer or
opaque payloads and concatenated/incomplete literals do not gain this exception.
No application name, marker word or test-file location is recognized. A real
unprefixed secret deliberately shaped like such a marker in that exact constant
context is indistinguishable to this lexical rule; this is not semantic proof
that the value is harmless. Provider-token and private-key checks remain independent.
GitHub tokens (including stateless installation tokens) and private
keys use independent rules: examples, comments and templates do not exempt them.
Synthetic values that look like credentials can still trigger a refusal.
UTF-8 BOMs are ignored and BOM-marked UTF-16 is normalized. Private-key detection
recognizes common OpenPGP and encrypted-PEM metadata headers, examining up to
16 KiB after each key header; it is not a complete key parser. More than 2 MiB
of cumulative key-armor lookahead in one scan causes an incomplete-scan refusal.
`.env`, `.env.*` and `*.env` paths are blocked;
`.env.example`, `.env.sample` and `.env.template` may be committed only if their
content passes the same scan. Untracked files are not inspected for commits.
Refusals include a fixed reason, sanitized location and corrective guidance, never
the matching value. Diagnostic logs retain only fixed failure codes. CLI JSON
includes `operation_started: false` and `retry_requires_changes: true`; refused
normal pushes report `remote_outcome: "not_attempted"`.

The scanner refuses incomplete checks, missing objects or limits: 100,000 objects,
128 MiB total raw object bytes, and the configured operation timeout. The 8 MiB
batch target groups smaller objects; larger individual objects are read separately
within the same total budget. That budget includes unchanged staged files for a
commit and all reachable history for a push, with duplicate objects counted once.
Object reads use a size-derived output bound and remain buffered; this is not an
unlimited or constant-memory scan. Content inspection checks cancellation and the
shared deadline as it runs. A scan refusal or cancellation never starts the commit
or push process. False positives are possible;
no pattern scanner recognizes every secret, legacy unprefixed token or arbitrary
encoding. Archives and external Git LFS payloads are not inspected; the scanner
checks Git objects, including LFS pointer files. Submodule histories are checked
through their own push operations. Git history operations that create commits internally
(such as rebase or automatic merge commits) are checked when Rocket pushes the
result. Commit hooks or other local processes may change content after preflight;
the push check inspects the resulting immutable objects.

This is a Rocket client guard. Raw Git, other clients and custom shell commands
are outside it, and an agent with unrestricted machine access can bypass local
controls. GitHub push protection supplies a separate server-side check; it is not
configured by this feature. An existing leak requires revoking the credential and
reviewing/removing it from history; simply deleting the newest file is insufficient.
See GitHub's [push protection documentation](https://docs.github.com/en/code-security/concepts/secret-security/push-protection).

## Git LFS

Rocket requires **Git LFS 3.8.1 or newer** and refuses older, malformed and
prerelease version strings before repository operations. The version probe runs
beside the installed executable, outside the selected repository. This supersedes
the 3.7.1 minimum recorded in earlier milestone documentation. No helper is
downloaded or installed by Rocket.

The [official 3.8.1 release](https://github.com/git-lfs/git-lfs/releases/tag/v3.8.1)
fixes CVE-2026-69260, CVE-2026-69261, CVE-2026-105802 and CVE-2026-105803.
Blocking an old helper protects Rocket operations; updating the installed helper
remains necessary for other Git clients and for restoring Rocket LFS functionality.

## MCP fixture scope

The MCP backend propagates its transport scope to every Git client and request.
Immediately before fetch, quiet fetch, push, clone or remote advertisement, it
reads effective configuration (including includes), rejects URL rewrite rules and
custom remote transport commands, and checks all effective destinations against
the canonical fixture root and exact approved network URLs. It finds the Git verb
behind global options and refuses a global option it doesn't recognize, so a flag
can't skip this check. Transport protocols
are restricted to file/HTTPS/SSH, intersected with any narrower inherited list;
HTTP redirects are disabled. Existing operation-specific branch and remote rules
remain in force. Normal desktop and CLI clients retain ordinary Git behavior.

This is a disposable-fixture guard, not an operating-system sandbox against a
hostile local process that can replace files or executables during execution.

General MCP state omits terminal tail/grid text, conflict drafts, external-tool
arguments/results and their free-text errors by default. Counts and process
status remain available. An operator running **synthetic fixtures only** may
create `<test-root>/rocket-private-state` containing exactly
`synthetic-content-only-v1` followed by one LF newline. This opts the test window
into private text inspection; every full state reports `private_state_opt_in`.
Removing the marker revokes the opt-in on the next state read. There is no MCP
tool that grants this consent. Do not use the marker with real/private content.
Explicit screenshots still show whatever is visible in the test window.

## Resource and output limits

- Commit parsing refuses more than 1,024 parents rather than silently changing
  ancestry used by merge/rebase operations. History reports an unsupported read.
- Graph rendering limits each commit to 32 parents and the live layout to 64
  lanes. A visible notice identifies simplified ancestry; normal graph folding
  and first-parent colors are preserved.
- The developer `inspect` example escapes terminal controls and bidirectional
  formatting in every output/error path, including displayed patches. Its diff
  output is human-readable; use the patch export API for byte-exact patches.

## Diagnostic durability

The existing five 1 MiB rotated logs remain. A writer blocked on the rotation
lock for 500 ms saves its complete sanitized record in a fixed disk overflow
slot and flushes it. There are at most 256 slots of 16 KiB (4 MiB total).
Readers include these slots immediately. The next successful rotation-lock
holder replays complete records, flushes the destination, then removes recovery
copies. Readers deduplicate `(session, seq)` after interrupted recovery.

Incomplete crash records remain inspectable and are not replayed as valid JSON.
Disk failures, invalid retention paths and exhausted overflow capacity still
increment `dropped_events`; logs never change an already observed Git result.
`overflow_events` counts records preserved through contention. The total normal
retention budget is 9 MiB, independent of the number of cooperating processes.

## Unsigned distribution and optional signing

Rocket supports **unsigned builds and releases**. No certificate is required
to build, distribute or run Rocket's Lua worker. The
current verifier accepts unsigned Rocket binaries by default while retaining
exact-content/hash checks and the existing Microsoft ConPTY verification.

Current bundles require all six Rocket executables, including `rocket-lua-host.exe`.
The verifier also checks that the worker digest appears in both host images, so
an incomplete or accidentally mixed build fails even with a regenerated manifest.
Use `tools/Package-Release.ps1 -Offline` after choosing a new workspace version;
it verifies the candidate before giving it the versioned release directory.
Historical bundles retain their original bytes and their own verifier.

SHA-256 manifests detect changed bytes but do not authenticate the publisher.
The worker's digest is embedded into Rocket/MCP by `tools/Build-Local.ps1`, not read
from a mutable sidecar or runtime environment. This prevents a changed worker
from being accepted by that host. It does not protect a replaced host or verifier.
No trust-store changes or certificates are installed. Optional future publisher
verification uses `-PublisherThumbprint`; signing is not a prerequisite here.

## Follow-up hardening (0.69.2)

The second audit pass is recorded in
`work/security-remediation-20261024-v2/REPORT.md`. Scoped transport also refuses
nonempty effective `core.sshCommand`, `core.askPass`, `core.gitProxy` and all
`credential.helper` commands, including URL-specific helpers and includes. It
rejects inherited SSH/askpass/proxy/helper-executable overrides before transport.
LFS subprocesses enforce the same command policy. Use disposable Git global/system
configuration and a clean child environment for MCP fixtures. Ordinary desktop/CLI
authentication and the separately authorized browser sign-in path keep their
existing behavior. This guard is not an operating-system process sandbox.

Scoped Open/Browse checks existing path components again on its worker, refuses
junctions and other reparse points, and holds directory handles while reading.
Git metadata and HEAD reads remain inside the fixture root. Missing destinations
remain valid for New/Clone previews when their existing ancestors are safe.

The authoritative verifier is `tools/Verify-Release.ps1`; new bundles carry a
copy. It requires exact agreement between directory contents and the hash manifest,
including hidden files, and refuses links, duplicates and unexpected executables
or DLLs. The bundled ConPTY DLL and OpenConsole executable require valid timestamped
Microsoft signatures and a reviewed matching architecture hash pair.

For an unsigned build or release:

```powershell
pwsh -File .\tools\Verify-Release.ps1 -Bundle <release-directory>
```

This verifies contents and the Microsoft runtime, not a Rocket publisher
identity. `-AllowUnsignedLocal` remains a compatibility alias for `-AllowUnsigned`.
If optional `-PublisherThumbprint` is supplied, all Rocket executables must have
valid timestamped signatures from that publisher.
Historical release directories are retained unchanged; their older verifiers do
not provide the new exact-content checks.

## Scoped hooks and HTTPS policy (0.70.1)

Every scoped Git invocation forces `core.hooksPath=/dev/null` after caller config
options. LFS receives that policy through its child-Git environment as well. This
does not edit repository/global configuration or remove hook files. Ordinary
desktop/CLI operations retain their configured hooks. Hook-driven workflows must
be exercised outside the restricted MCP fixture mode.

Git clears client configuration overrides before spawning a local receive-pack.
Scoped local pushes therefore refuse receivers whose effective hooks directory
contains active entries, lies outside the fixture root, or uses a link/reparse
point. Missing/empty hook directories and Git's disabled-hooks setting are accepted;
sample hook files are inert. The receiver check is bounded and repeated at execution.

Scoped HTTPS uses direct connections, certificate verification, default trust and
no redirects. Effective config (including includes and URL-specific settings)
cannot select proxies, alternate TLS trust/backends, weakened TLS settings or DNS
routing overrides. Proxy/TLS environment overrides are rejected before transport
and removed from scoped Git/LFS processes, including helper version probes.
Safe explicit verification values and empty proxy resets remain accepted.

These controls constrain the disposable fixture process. They are not an OS
sandbox against a separate hostile process changing repository files during
execution. Live authentication and actual supported Git LFS transfers were not
exercised by this follow-up; the helper test uses a compiled fixture plus real Git.

## Restricted Windows SSH acceptance (0.71.1)

The local MCP operator may launch `rocket-mcp --test-root ROOT
--windows-ssh-agent ROOT/identity.pub` (or append the same option to
`--http-profile PROFILE`). There is no MCP tool that enables this mode. The public
key must be a regular Ed25519 `.pub` file inside the fixture root, without linked
parent directories. Private keys remain in the existing Windows SSH agent.

This profile selects Windows OpenSSH from the operating system's system directory,
ignores user/system SSH configuration, and permits only the existing exact SSH URLs
for `JasXTD/rocket-auth-test`. It pins GitHub's published Ed25519 host key, disables
proxy commands, forwarding, password prompts and connection sharing, and selects
only the supplied public identity. Generated configuration, public key and host
trust files stay pinned against replacement and writes for the scope's lifetime.
Public-key comments are omitted. Host-key rotation requires a reviewed update.

Repository/inherited SSH, askpass and helper command overrides are still refused;
the profile does not weaken hook, URL, proxy or TLS restrictions. Normal desktop
and CLI authentication behavior is unchanged. A push refused before its process
starts reports `not_attempted`. Errors after process start remain `unknown` without
a conclusive Git ref status; stderr wording alone cannot prove no update occurred.

Policy sources: [GitHub host keys](https://docs.github.com/en/authentication/keeping-your-account-and-data-secure/githubs-ssh-key-fingerprints)
and [OpenSSH configuration](https://man.openbsd.org/ssh_config).

## Offline dependency warnings — 2026-10-25

Dependency warnings default on and can be turned off in Preferences > Security.
Background warnings do not participate in commit, push or publish policy. The
separate optional pre-push check is described below. Existing secret protection
is independent. Preference state v13 preserves older settings and
migrates dependency warnings to on; older binaries cannot read the new state.

First-run setup in Security lets the user choose the optional OSV executable.
Selection verifies the pinned checksum without running the chosen file, then
atomically saves a bounded per-profile path. A failed/cancelled selection keeps
the previous choice; concurrent changes require reloading setup. Saved choices
are checked again on startup and every launch. An invalid saved choice never
silently falls back to a different scanner. Automatic runtime-folder discovery
remains available when the profile has no saved choice. The restricted MCP host
cannot select a scanner or initiate advisory downloads.

With a verified scanner, the active repository is checked on open and periodically;
manual scanning is also available. Discovery and scanning run on a worker. Only
recognized dependency files are copied into a temporary snapshot with Rocket's
empty scanner configuration. Repository OSV ignores, source files, build scripts,
package-manager configuration and credentials are not passed to the helper.
The child receives a cleared environment, `--offline`, `--no-resolve` and disabled
call analysis. This is a restricted invocation, not an OS network sandbox.
Actual dependency reachability and project-specific exploitability are not assessed.

The result cache compares dependency bytes and advisory archive hashes. Changes
during scanning invalidate the result. Missing/invalid data, unsupported or absent
lockfiles, parser failures, cancellation, size limits and stale databases are
reported; none is presented as proof of safety. Scans permit at most 256 files,
8 MiB per file, 64 MiB total, 50,000 directory entries and 24 nested directories.
Generated dependency folders, symbolic links and junctions are skipped. Parsed
output is bounded to 16 MiB, 50,000 packages and 5,000 findings. A normal scan has
a 90-second timeout; an explicit data update has a 180-second timeout. Cancellation
uses Rocket's existing child-process ownership and cleanup. Quit waits for cleanup.

An explicit data update downloads only public ecosystem-wide databases using a
synthetic inventory. It receives no repository paths, dependency names or versions.
Background scans and updates are serialized. Downloads are staged and checked before archive
replacement; an error during publication can leave some ecosystems updated. Local
state and temporary lockfiles are private application data, not encrypted storage.
Raw OSV JSON, stderr, package names and paths are not copied into activity logs or
MCP metadata. The isolated MCP host cannot change the preference or initiate a data
update. Advisory links use validated identifiers at `https://osv.dev/vulnerability/`.

## Mail and hosting review display boundaries — 0.76.1

Commit-mail previews build a separate display cache on the cancellable review
worker. Subjects, authors and dates have output budgets of 512, 256 and 128 UTF-8
bytes. Each selected preview shows at most 64 paths (1,024 bytes each), 160 patch
lines and 16 KiB of patch text, with at most 1,024 bytes per line. Counts are cached;
rendering does not decode or rescan the original mail. Invisible control and bidi
characters are shown as visible escapes, including in paths and branch labels.
Authors, messages, paths, secret scans and Git input retain the original bytes.

Any omitted applied content requires explicit acknowledgement. Consent belongs to
one immutable preview; refreshing or reopening a review clears it. The backend
also refuses an unacknowledged limited preview. Before writing, it compares every
source byte and the repository/recovery state against the reviewed snapshot; stale
inputs require a fresh review. Abort remains available without accepting omitted
content, and Skip only requires acceptance for patches it will actually apply.

Hosting results use the same visible escaping before constructing `ForgeItem`.
Title, author, state and each branch have output budgets of 1,024, 256, 64 and
1,024 UTF-8 bytes, including any ellipsis. This covers all five providers and Azure
work items. Display strings are not used as write arguments; destination links
remain locally reconstructed from the validated repository and numeric item ID.

## Imported custom-command safety

Imported command definitions are data only. Opening, accepting, selecting or editing
one never checks its executable on disk. The editor offers an explicit **Check**
action for the exact current path; changing that path clears the check. Browse is
also explicit and invalid stored executable paths do not seed its starting folder.
Validation is lexical and runs before any executable metadata access. On Windows,
UNC/SMB/WebDAV path spellings, non-drive device namespaces and alternate streams
are rejected. Normal and extended local drive paths are supported. A mapped drive
or junction can still resolve elsewhere; only an explicit Check, Browse or confirmed
run can resolve it. The existing regular-file and reparse checks remain at execution.

Definitions reject the shared invisible control/bidi/formatting character set in
names, groups, executable/URL text, argument contents and input labels/defaults/flags.
CRLF/LF argument separators and ordinary Unicode text remain supported. Every import
schema uses the same validator. Existing invalid saved definitions remain editable,
but cannot be saved or executed unchanged. Menu, palette, import, tooltip and execution
review strings visibly escape hidden characters and backslashes; original contextual
paths/branches and literal arguments are preserved for execution. Repeated defaults
cannot inflate the preview beyond the runner's existing 32000-byte argument limit.

## Windows Git HTTP runtime gate (2026-10-29)

Git for Windows bundles its own curl/Brotli decoder. Rocket permits built-in
HTTP(S) Git operations only when the Windows build identifies itself as
2.56.0.windows.4 or newer. Earlier and unrecognized builds receive a final
`GIT_ALLOW_PROTOCOL` restriction to file/SSH, intersected with existing limits.
This follows URL rewrites and child Git processes, including submodules and LFS
child Git calls; repository configuration cannot restore HTTP. Unknown remote
helpers are also excluded. Local Git and SSH remain usable. Preferences preserves
the complete build number and shows the HTTP restriction. Restart Rocket after
updating Git. Non-Windows version requirements are unchanged.

This is a known-version safety gate, not binary authentication or protection
against replacing an installation after discovery. Explicit external/custom tools
remain separately trusted. Git for Windows 2.56.0.windows.4 updates Brotli to 1.2.1
for CVE-2026-111497; see the [official security release](https://github.com/git-for-windows/git/releases/tag/v2.56.0.windows.4).

On Windows, hosting/integration HTTP uses only curl from `GetSystemDirectoryW`.
`SystemRoot`, PATH and a curl beside Git cannot select this credential-bearing
helper; reparse files are refused and a missing OS helper fails closed. This
relies on the OS installation's protection and does not authenticate arbitrary
local Rocket builds. Rocket releases may be unsigned; the Lua worker uses the
embedded digest and OS isolation described below.

## Lua worker identity and OS isolation (2026-10-29)

All package execution, syntax validation and playground calls use the same
hash-bound launcher. It rejects unbound builds, hash changes, oversized files,
network paths and reparse points. Read-only image handles deny writes/deletion
through verification and launch. A fresh random LPAC identity receives no
capabilities; only a private staged executable is granted read/execute access.
No repository files, credentials or general inherited handles enter the worker.

The job is attached atomically during suspended process creation. Before resuming,
Rocket verifies the AppContainer token, empty capabilities and LPAC access behavior:
its own SID grants access but ALL APPLICATION PACKAGES alone does not. Windows
blocks child processes, dynamic executable memory and Win32k calls, with 128 MiB
process memory and five-second CPU limits. Existing wall-clock, input/output and
Lua VM limits remain. Network access goes only through the reviewed native broker.

Every invocation creates temporary per-user AppContainer storage and a separate
read/execute staging directory, then removes them after reaping the worker.
Cancellation and failures terminate the owned job; closing the host's job handle
also kills the process. An abrupt host/OS crash can leave inert temporary profile
or staging files; they are never adopted by a later invocation. No global ACL,
firewall, antivirus or trust-store policy is changed.

This is defense in depth around an unsigned native worker. It relies on Windows
and the trusted Rocket host, does not authenticate a replacement distribution,
and cannot guarantee against unknown VM/OS flaws or misuse of API permissions
the user deliberately grants. Unsupported sandbox setup fails closed.

## Codex configuration and cached Git runtime checks (2026-10-31)

All Codex commit-draft, branch-review and conflict-proposal runs use
`--strict-config` and explicit `--disable` feature arguments. Unknown settings or
feature names must fail before the CLI starts the requested AI turn. Older CLIs
that do not support these controls fail closed. Rocket also refuses an otherwise
successful result if CLI diagnostics report an unknown or unsupported setting;
generated message content is not treated as a diagnostic. Existing read-only,
no-tools, isolated-directory and no-session controls remain in effect.

The bundled Git lease retains file handles that deny writes and deletion and
directory handles that deny deletion. Since these handles do not prevent new
directory entries, cached verification and every Git/LFS launch now enumerate
the retained directories again, checking exact names, types and membership.
Added files, directories and reparse points are refused. These checks are
cancellable and do not hash the locked files again. Removing an unexpected
entry restores use without weakening the reviewed inventory.

This reduces the cached-runtime gap; it does not make a user-writable directory
immutable. A same-user process can still race an insertion after the check, and
replacement of the complete unsigned application remains outside this boundary.
No system ACL, antivirus policy, certificate requirement or installed Git choice
is changed. New local packages use 0.87.3; archived 0.87.2 bundles are preserved.

## Bundled Git launch checks (2026-10-31)

This changes how the check above runs, because reading all 871 runtime directories
took about 190 ms per Git launch, 63 to 73% of Git time. On a fixed NTFS volume with
an active USN change journal, each Git/LFS launch now reads the journal from where
the last one stopped. The directories are read in full, exactly as before, when a
record names one of them, when the journal gives any error, when more than 16 MiB
waits to be read, and at most two minutes after the last full read. That read alone
decides, and a refused read stays due until one passes. Every other volume reads
before every launch, as it did, and Rocket never creates or changes a journal.

The journal replaced a `ReadDirectoryChangesW` watch during review. A change made
through a handle opened by file ID produces no directory change report, however the
watch is set up, while a full read still lists it. NTFS journals that change. Every
route tried was recorded before the changing call returned, with its handle still
open: plain, relative and by-ID creation of files and folders, renames and hard links
in from outside, repeat renames and links on one handle, a folder moved in with
contents, junctions, a stream, and a transaction at its commit. A file's own empty
DACL does not hide its record, and Git could not load a file shut that way. Rocket
reads records for the whole volume to find its own; the unprivileged read leaves
their names out, and nothing from them is kept or logged.

The full read now also checks each held directory handle. A listing shows a new
reparse point only once the handle that set it closes, and the journal has nothing
more to say then. Without this check, a junction stayed invisible while a program
kept that handle open, which was also true of the earlier per-launch read.

Limits: this trusts NTFS to write the record. Code in the kernel, or a filter that
changes what a directory shows, could hide a change from the journal. The two-minute
read finds what is still there, but not something removed before it. The same applies
to a reparse point set on a held file through a handle that stays open, which needs
the symbolic link privilege and was not tested. A same-user process can still race
an insertion after the check, as before, with a window of microseconds instead of up
to about 200 ms.

## Elevated launches, placeholder boundaries and npm coverage (2026-11-01)

The shared backend process launcher checks the current Windows primary token
before creating a child process. Elevated tokens and integrity levels at or above
High are refused, including Git, Git LFS and external backend helpers. A failed
token query also refuses the launch. Reopen Rocket without Run as administrator.
Normal UAC-filtered use retains configured Git hooks; administrator group
membership alone is not a reason for refusal. The check has no release-build
environment override and does not alter Windows privileges or system settings.

Generic credential detection accepts complete placeholder values and recognized
reference syntax, rather than arbitrary strings beginning with `example`, `dummy`,
`fixture`, `redacted`, `process.env` or similar markers. A quoted expression is
still a literal unless it has explicit template/reference syntax. These checks
cover commit messages, staged content and outgoing history. This remains bounded,
heuristic detection, not a guarantee that every possible secret will be found.

Before an offline scan or cache reuse, Rocket checks npm package-lock/shrinkwrap
source metadata. Registry-shaped dependency entries with missing, empty or invalid
`resolved` values produce an incomplete result. Explicit local/workspace entries
remain supported. This guards the pinned OSV 2.7 [upstream coverage issue](https://github.com/google/osv-scanner/issues/3177).
No package manager runs and no lockfile is rewritten. Correct the resolution
metadata and rescan. Background scans remain warnings; an enabled pre-push check
refuses incomplete scanner results. Existing findings may remain visible as previous results,
while current coverage is incomplete.

## Optional pre-push dependency review — 0.89.0

Preferences > Security > **Scan dependencies before pushing** defaults off and
is independent of background warnings and secret protection. When on, the native
application installs a shared Git push check after the existing secret guard and
before the push process starts. It covers branch, force, atomic, remote-tag,
commit-and-push and GitHub publication pushes; delete-only refspecs send no tree
and skip the check. Standalone CLI/MCP processes do not inherit this UI preference.

The scan reads raw, immutable pushed objects, including annotated-tag targets,
without a checkout, replacements, filters, package-manager execution or lazy
fetching. It scans supported dependency files at each tip, not every historical
dependency version. Worktree/index edits cannot change the scanned bytes. Each
push forces a fresh offline OSV run. Complete, warning-free reports continue;
advisories, stale data, skipped content or missing coverage require a native user
decision. Scanner/data/parser/limit failures stop the push without a bypass.
The restricted MCP UI cannot enable the option or approve a warning review.

An approval belongs to one pinned push only. Cancellation or shutdown releases
the waiting worker without starting Git push. Advisory fingerprints are checked
again after review; changed Git configuration also refuses the attempt, and the
transport scope is revalidated. Existing Git leases and remote-outcome reporting
still apply. A local commit or newly created GitHub repository can already exist
when publication is stopped. Cancellation after approval is not rollback of a
push that has started.

Per-file/inventory/parser bounds above remain. One request allows at most 32
unique tips, 64 MiB of combined dependency inputs and 5,000 combined findings.
Background data updates can overlap a pre-push check; changed archive fingerprints
invalidate that check rather than approving an outdated review. Opt-in persists
in profile v24; old profiles default off, and older binaries reject v24.
