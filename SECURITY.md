# Security

## Reporting a vulnerability

Please report security problems privately, not in a public issue. On GitHub, open the
repository's **Security** tab and choose **Report a vulnerability**. Include:

- the Rocket version (Help → About) and your operating system;
- what an attacker could do, with steps or a proof of concept;
- only files with tokens, passwords and private repository content removed.

You'll get a reply in the report. Please allow time for a fix before you disclose the
problem publicly.

## Supported versions

Rocket has no public release yet, so security fixes go to `main`. Once releases exist, the
latest release receives them.

## Scope

In scope: the desktop app, the CLI, the localhost MCP service, the Lua extension sandbox,
app updates, credential storage, and the components Rocket bundles (an outdated bundled
Git for Windows, Git LFS or terminal runtime counts). Problems in Git itself, OpenSSH or a
hosting service belong with those projects, unless Rocket makes them worse.

## Security boundaries

The rest of this file records how Rocket's safeguards work and where they stop, feature by
feature. The [README](README.md) gives an overview.

## macOS credential preparation (unreleased)

The macOS port seals local credentials with XChaCha20-Poly1305 and a wrapping key
in the default file-based Keychain. It uses Security.framework directly, disables
prompts for its serialized calls, restores that process setting, and fails visibly
when Keychain access is denied. It never replaces a pre-existing wrapping key,
creates a missing key during decryption, unlocks/resets a Keychain, or falls back
to plaintext. On first creation only, the item trusts the creating binary and
verified sibling Rocket helpers; existing item access lists are preserved.
Mac envelopes are separate from Windows DPAPI and Linux envelopes. This legacy
Keychain boundary supports development binaries, including ad-hoc signatures,
not a stable Developer ID identity or proof of signed/notarized helper upgrades.
Actual Mac interoperability still requires native acceptance.

Mac SSH uses installed `/usr/bin` tools and the current user's inherited agent
socket. Direct-key mode disables the agent and Apple's separate `UseKeychain`
lookup; private-key ACL grants are refused. No user key/config/agent settings are
edited automatically. Credential publication is exclusive on supported systems,
and Unix credential locks reject symlinks. See [the Mac port gates](docs/MacOS-port.md)
for the prepared disposable-Keychain and native OpenSSH tests and their limits.

## Earlier security remediation

This security remediation follows the findings in Codex chat
`01a14039-a14f-7c23-a3ce-1f9a89ebbcc6`. It applies to the current source;
see the reports for the first pass in `work/security-remediation-20261023/REPORT.md`
and the follow-up in `work/security-remediation-20261024-v2/REPORT.md` for tested
artifacts and limits.

## Linux state storage and preview ownership (unreleased, 2026-11-08)

Rocket fails closed when its XDG state root is unavailable or unsafe. It checks each
path component from the filesystem root before creating/using the next: real
directories only, owned by the effective user or root, with no group/other write
permission. Trusted sticky ancestors such as root-owned `/tmp` are allowed because
other users cannot rename the checked user/root-owned entry. The state root and
each new update job must belong to the effective user and have no group/other
permissions. Missing directories are created with 0700; failures propagate. No
existing data or permissions are repaired automatically and no temporary profile
fallback is used. This boundary trusts root and processes running as the same user.

Settings, logs and update-cache subfolders may inherit privacy from an owned 0700
ancestor, preserving earlier 0755 children within a private Rocket root. Directory
checks precede saved-state reads and update handoff/helper plan reads. The private
job and protected ancestry prevent a different local user from replacing the
hash-checked update helper between verification and execution. Signing remains
optional; Lua worker digest binding and sandbox policy are unchanged.

Preview slots take a nonblocking exclusive OS file lock on Unix as well as Windows.
Unix cleanup/discard retains that lock through deletion. A second process leaves
live previews alone; process exit/crash releases ownership so later cleanup can
remove stale slots. Windows retains its exclusive share-mode behavior.

## Direct SSH and Preview audit fixes (unreleased, 2026-11-06)

Direct SSH accepts exactly one fixed Git service and one shell-quoted repository
path; only Git's escaped apostrophe/exclamation continuations are permitted
outside quoted spans. LFS requests additionally require exactly `upload` or
`download`; simple unquoted LFS paths use a literal-character allowlist. Controls,
option-like/empty paths, extra arguments, trailing shell syntax and interactive
shell requests are refused. The no-connection `-G` configuration probe remains
available. Host/key checks, disabled forwarding and strict host trust remain active.

Profile binding refuses effective `remote.*.uploadpack`, `remote.*.receivepack`
and `remote.*.vcs` overrides. The helper repeats this check immediately before
launching OpenSSH, including effective global/system/includes/worktree config.
Whole-command validation remains the independent boundary against a config
change after review or an invocation-supplied service override. This closes the
audited path to the remote account's shell; it does not authenticate unsigned
Rocket binaries or make arbitrary user-replaced SSH/Git programs trusted.

Preview enumeration now retains at most 100,000 missing IDs, accepts at most
32 MiB of streamed object output (or the caller's smaller cap), and bounds each
partial SHA-1/SHA-256 record to 65 bytes. Oversized, invalid or truncated records
fail closed, and the runner stops/reaps Git on refusal. Open, on-demand reads and
Keep use the same limits, independently of the optional preview disk limit.
Oversized graphs can be cloned normally instead. No oversized enumeration is
passed to fetch or published as a kept repository.

Protocol references: [Git shell quoting](https://github.com/git/git/blob/v2.56.0/quote.c)
and [Git LFS SSH request construction](https://github.com/git-lfs/git-lfs/blob/main/ssh/ssh.go).

## Linux privilege and file publication (unreleased, 2026-11-06)

On Linux and other Unix systems, the shared privilege boundary refuses a process whose
real or effective user ID is 0 (root, `sudo` or a set-user-ID launch), as Windows refuses
an elevated token: the desktop, MCP, operational CLI commands, helpers and every
child-process launcher stop before acting. Capabilities granted to a non-root process are
not checked.

Publication that must not overwrite (repository creation, clone, worktree creation,
ignore and LFS attribute edits, patch files, submodules) uses `renameat2` with
`RENAME_NOREPLACE`. Conflict saves and checked whole-file replacements swap the reviewed
file with `RENAME_EXCHANGE` and keep the file actually displaced in the recovery folder,
as `ReplaceFileW` does. A filesystem that rejects either flag (EINVAL or ENOSYS, as on
NFS, some FUSE mounts and WSL's `/mnt` drives) is refused with that reason;
Rocket never retries with a rename that can replace a file. Symbolic links are refused
where Windows refuses reparse points. Linux has no handle that keeps folders from being
renamed during an operation, as Windows handles do; the checks before and after
publication still apply.

## Linux file checks: OSV-Scanner, bisect and LFS cleanup (unreleased, 2026-11-07)

**SHA-256.** On Linux, files are hashed with RustCrypto's sha2, with the same size limit
and cancellation as BCrypt on Windows.

**OSV-Scanner.** Rocket pins the SHA-256 of OSV-Scanner 2.7.0 for Linux x64, as for
Windows x64. Linux has no handle that stops other programs from changing a file, as
Windows' share mode does. Instead, Rocket checks only a regular file:
- opened without following a link;
- owned by you or by root;
- not writable by group or others;
- executable.

It hashes that open file. The scanner then runs from `/proc/self/fd/N`, that same open
file, so a path swapped after the check can't run instead. The file stays open until the
scan ends, and the system refuses writes to a running program. The scanner gets
a cleared environment: its scratch folder as `HOME` and `TMPDIR`, and no proxy, token or
user configuration.

**Automatic bisect.** The test program's SHA-256 is checked as on Windows. Each test runs
in its own process group; cancelling, a timeout, or the test's exit stops every process
left in that group.

**LFS cache cleanup.** Linux cannot delete a file by handle. Each object is opened inside
its own folder, reached one folder at a time from the Git folder with `openat` and
`O_NOFOLLOW`, so a renamed folder or a link put in its place cannot redirect the cleanup.
Rocket hashes the object through the file it opened. It removes the name only while it is
still that file, by device and inode; a file put in its place is kept and the review
reports the change. Recovery copies in a local remote are checked the same way and kept
open while the cleanup runs, but Linux cannot stop another program from removing them
meanwhile, as Windows can.

## Linux terminal, SSH keys, MCP control and credentials (unreleased, 2026-11-07)

**Terminal.** The terminal's pseudoterminal is opened with `posix_openpt` and
`TIOCGPTPEER`, both close-on-exec. The shell starts by its full path; a bare name is never
looked up on `PATH`. Before exec, its signal handlers are reset to their defaults, and it
gets a new session with the terminal as its controlling terminal. Ending a session sends
SIGHUP and SIGCONT to every process group in it, then SIGKILL a second later; the
shell's own group is signalled only while it is unreaped, so its ID can't have been reused.
A process that calls `setsid` itself leaves the session and is not followed. The reader
and writer wait with `poll`, so a process holding the terminal can't stall shutdown.

**SSH keys.** OpenSSH comes from `/usr/bin` only. A private key must belong to the user
and be unreadable by others (mode `077` clear), as OpenSSH's own check requires; links to
keys and profile files are refused, as reparse points are on Windows. Rocket's folders
are created 0700 and its files 0600. ssh-agent is reached only through an `SSH_AUTH_SOCK`
that names a socket owned by the user; Rocket starts no agent. Linux cannot share-lock
files as Windows does, so key files are checked again after use. A repository binding
replaces the config in one step (`RENAME_EXCHANGE`) and keeps its permissions.

**MCP control.** The profile folder must belong to the user and not be writable by
others. Its settings, token and lock files are created 0600, and Rocket refuses any that
other users can open. Locks are exclusive `flock`s, released when the process ends.
Tokens are 32 bytes from `getrandom(2)`. In the terminal, MCP types only fixed bash
probes, each matched exactly, as it types only fixed PowerShell probes on Windows; it
refuses any other text.

**Protected data.** Remembered tokens, saved SSH passphrases, drafts, conflict snapshots
and integration secrets are sealed locally with XChaCha20-Poly1305 (RustCrypto
`chacha20poly1305`), with a random 24-byte nonce for every seal and associated data that
binds each blob to this purpose. The 256-bit key comes from `getrandom(2)`. It is kept as
one Secret Service item ("Rocket credential protection key"), through libsecret, which
handles the keyring's encrypted session and unlock prompts.

libsecret is loaded at run time only from the system's own library folders, as a file
that root owns in folders root owns and no one else can write; a library offered by
`LD_LIBRARY_PATH` or a planted copy is never used. A lock file in Rocket's folder stops
two Rocket processes from creating the key at once. Decrypting never creates a key, and
a damaged key is reported, not replaced. Like DPAPI, this protects data at rest from other
users and offline copies, not from programs running as the same user.

**Spell check.** Words are checked by spellbook, a pure-Rust reader of the system's
Hunspell dictionaries, so no C dictionary parser runs in Rocket. Dictionaries are read
only from the system folders, and added words are kept in a 0600 file in Rocket's folder.

## Linux Lua worker isolation (unreleased, 2026-11-07)

On Linux x86_64, Lua runs in the same hash-bound worker as on Windows, with the same
input, output, time and Lua VM limits. Windows starts the worker inside its container.
On Linux the dynamic loader still has to open the worker's libraries after exec, so the
worker locks itself down at its entry point, before it reads any script. The system
dynamic loader, shared libraries and Rust startup run before that boundary; the
embedded hash binds the worker bytes, not those system components.

**Launch.** Rocket opens the worker, then checks the opened descriptor against the path
and rejects links, non-regular files, unexpected owners and group/other-writable files.
This is post-open validation, not an `O_NOFOLLOW` open. It copies the file into a memory file
and seals that against writes, growth and shrinking, then hashes the copy. What runs is
exactly what was hashed; no path or later write can change it. The worker gets:
- the sealed copy, run from `/proc/self/fd/N`;
- an empty environment, with `/` as its folder;
- its three pipes and nothing else: every other descriptor is closed on exec;
- its own session and process group, with no terminal;
- no new privileges (`PR_SET_NO_NEW_PRIVS`), cleared ambient, effective, permitted and
  inheritable capabilities, no core dumps and a five-second CPU soft limit (six hard);
- `SIGKILL` if the thread that started it ends, as closing the job kills the Windows
  worker.

Cancelling, a timeout or an output limit kills its process group.

**Lock-down.** Before it reads input, the worker:
- refuses to go on if it has more than one thread;
- closes every descriptor above standard error;
- clears capabilities again, including on direct execution;
- bounds further virtual-address growth at 128 MiB (`RLIMIT_AS`, including libraries
  and reservations), with a 128 MiB data limit, 2 MiB stack limit, no locked memory
  and a pending-signal limit of 32. Lower existing hard limits stay lower. This differs
  from Windows' 128 MiB committed-process-memory limit; neither is a machine-wide RAM
  quota. CPU soft/hard limits are five/six seconds; the host also enforces a five-second
  wall deadline. No core dumps, file writes or new descriptors are allowed;
- stops other programs of the same user from attaching to it (`PR_SET_DUMPABLE`);
- applies a Landlock ruleset with no rules. It denies every file access the kernel's
  Landlock knows. From Landlock ABI 4 it also denies TCP connections and binds, and from
  ABI 6 signals and abstract Unix sockets that leave the sandbox;
- applies a seccomp filter. It allows reads from standard input, writes to standard
  output and error, private anonymous mappings (no shared/grows-down or executable
  maps), non-executable memory protection changes, signals to itself, private futex
  wait/wake/timed wait-bitset, clock IDs 0–7, random bytes and exit. Negative clock IDs
  that name other processes are refused. Unlisted calls presented to the filter fail
  with EPERM: opening files, sockets, new processes and programs, tracing, io_uring
  and BPF among them. A different architecture or x32 ABI kills the worker when the
  kernel passes the call through this filter. Newer kernels exempt their `uretprobe`
  and `uprobe` trampoline syscalls from seccomp by design; that is not a general
  syscall-filter bypass available to the extension.

The worker then checks that opening `/` and creating a socket are refused, with the
errors that show Landlock and seccomp refused them.

Lua fails closed: without Landlock (Linux before 5.13, or switched off), on another
architecture, or where any step fails. There is no unrestricted fallback. The sandbox
tests compile the same lock-down code into a probe that tries to read, write, connect,
fork, map executable/shared/grows-down memory, reserve 256 MiB, use forbidden futexes
and clocks, and signal its parent. They check the installed limits, compare every
syscall number 0–1023 to an independent policy, and require a fully started worker
to stop within one second of abrupt parent exit. Runtime tests also run the actual
Cargo-built Lua worker, including Unicode, instruction and allocation limits.

As on Windows, this is defense in depth around an unsigned native worker. It relies on
the kernel's Landlock and seccomp and cannot rule out kernel flaws. Rights that later
Landlock versions add stay unhandled until Rocket names them; seccomp independently
restricts the available calls and arguments, subject to the kernel exceptions above.
A system that forbids executable memory files (`vm.memfd_noexec=2`) can't
start the worker.

## Linux app updates (unreleased, 2026-11-07)

On Linux x86_64 the updater follows the Windows flow: public GitHub releases only, no
credentials, and an install only after you approve it. What differs:

**The release file.** Linux looks only for `Rocket-<version>-linux-x64.tar.gz`, never the
Windows ZIP. As on Windows, its size and the SHA-256 that GitHub publishes for it are
checked before anything is unpacked.

**System programs.** Rocket runs only `/usr/bin/curl` (8.4.0 or later, which enforces
size limits on streamed downloads) and `/usr/bin/gzip`. Each must be the file itself, not
a link, owned by root in a folder root owns, and writable by no one else, as Windows runs
only its System32 curl. A copy on `PATH` is never used.

**Unpacking.** gzip only decompresses. Rocket reads the archive itself and writes every
file new into a fresh folder, so it never relies on how a tar program treats a hostile
archive. It accepts POSIX ustar entries that are plain files or folders, all inside one
top folder named `Rocket-<x.y.z>-linux-x64`. It refuses:
- links, devices and other special entries, and extended headers;
- absolute paths, `..`, empty or `.` path parts, control characters and backslashes;
- a second top folder, or a name that appears twice, ignoring case;
- bad checksums, non-zero padding, data after the end, and a truncated archive;
- more than 20000 entries, a file over 256 MiB, or 1 GiB unpacked.

The unpacked release then passes the same manifest and hash checks as on Windows.

**Install.** The helper is a copy of the installed `rocket` in the update's folder,
started in its own process group, so neither Ctrl+C nor closing the terminal Rocket was
started from stops it mid-swap. It waits for Rocket through a process descriptor
(`pidfd_open`), opened while Rocket is still waiting for it, as Windows opens a process
handle. It swaps the folders by rename and keeps the previous one, as on Windows. Copied
files get their permissions from Rocket: 0755 for its own programs, 0644 for everything
else, whatever the archive said. Linux can't lock the installation folder against
renames, as Windows does, so the swap relies on its checks of both folders' contents just
before it.

## Linux hosting and extension requests (unreleased, 2026-11-07)

On Linux, GitHub sign-in and reads, the other hosting services and the Lua extensions' web
requests run only the system's `/usr/bin/curl`. It must pass the check app updates use,
which they now share: the file itself, not a link, owned by root in a folder root owns,
and changeable by no one else. `SystemRoot`, `PATH` and a curl beside Git can't select
another copy, as on Windows, where only the System32 curl runs. Without such a curl these
features report curl unavailable; nothing falls back to another copy.

The requests are the same on both systems:
- the token and any body reach curl on standard input, never in its arguments;
- `-q` keeps curl from reading a `.curlrc`, and `SSLKEYLOGFILE` is removed;
- redirects are never followed, and only HTTPS is allowed, except plain HTTP to a
  loopback test server;
- curl verifies certificates as it is set up to; Rocket never turns that off.

As on Windows, this relies on the system's protection of the program's folder; it does not
authenticate the program itself.

## Local crash reports (0.92.0)

Desktop, operational CLI commands and the MCP host install a process-local Rust
panic hook and a best-effort Windows unhandled-exception filter after the privilege
check. They do not catch or resume a faulting process. Caught panics can produce a
report without terminating the app; the viewer says so explicitly.

Reports contain only version, recorded OS/architecture, fixed component/kind,
time, numeric process/thread IDs, a validated source basename/line and bounded
code offsets in a fixed set of known modules. Unknown module addresses are
omitted. Panic payloads, thread names, full paths, command arguments, repository
contents, environment, credentials, registers and memory dumps are excluded.
There is no upload service, telemetry call or automatic issue submission.

The writer uses a pre-opened file, a stack buffer and a nonblocking reentrancy
guard independently of operation-log locks. Each process attempts at most eight
records of 2 KiB. Storage retains at most 20 session files (320 KiB of records),
preserves active files, rejects redirected directory ancestry, and inspects at
most 256 entries. Reads validate/project the private record format; incomplete or
damaged records do not become raw text in the UI or exports. Export publishes a
new JSON file atomically without overwriting an existing file. Cancellation before
publication creates no destination; completion remains success after a late Stop.

Default storage is `%LOCALAPPDATA%\Rocket\crashes`; on Linux it is `crashes` under
`$XDG_STATE_HOME/rocket` (by default `~/.local/state/rocket`), whose folders Rocket creates
for the current user only (0700). An explicit CLI `--log-dir`
puts reports in its `crashes` subdirectory; MCP uses its explicit profile or test
root. Initialization failure is visible in the viewer/CLI warning. Crash-time disk
failure, simultaneous/reentrant failures, exhausted report limits, hard process
termination, fail-fast, a debugger, stack corruption or a replaced exception hook
can prevent capture. Git, terminal and sandboxed Lua helper failures remain in
their existing operation diagnostics; sandbox filesystem access is not expanded.

API contracts: [Rust panic hooks](https://doc.rust-lang.org/std/panic/fn.set_hook.html),
[Windows exception filters](https://learn.microsoft.com/en-us/windows/win32/api/errhandlingapi/nf-errhandlingapi-setunhandledexceptionfilter).

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
Structured counters use a third narrow form: a uniformly cased alphabetic
namespace of 2–24 letters, a camel-case label with at least three 3–24-letter
words, and exactly one full ascending or descending decimal cycle (ten digits,
with wraparound). The three components use the same hyphen or underscore.
The words retain the 3.5-bit bound and combined case-folded letters the 4.0-bit
bound. Decimal permutations, mixed-case words, partial cycles and extra payload
do not match. This recognizes structure, not provider validity or words such as
"fake"; known provider-key checks still take precedence, even for readable labels.
All three marker forms require an explicit
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
128 MiB per object, a configurable total raw-object budget (default **256 MiB**),
and the configured operation timeout. Preferences → Security → Total scan budget
accepts positive whole MiB values up to the platform's representable size. This
application-owned setting is saved locally; repository config cannot change it.
Existing profiles use the new default. Each scan snapshots its starting budget;
changes apply to future scans. CLI clients also default to 256 MiB but do not read
desktop preferences. The 8 MiB
batch target groups smaller objects; larger individual objects are read separately
within the separate 128 MiB per-object cap and the total budget. Increasing the
total does not increase a single read or remove cancellation/time limits. The
total includes unchanged staged files for a
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

Apple Silicon Mac integration uses the separately locked fixed-guest Pulley
helper, described in [Mac Lua isolation](docs/MacOS-lua-wasm.md). Its executable
is signed before its SHA-256 is embedded in the host. Only that helper's three
sealed files are copied into fresh private run storage, then signature/resources
and the copied executable hash are checked before launch. There is no script-
selected executable, guest module, native import or environment. A pipe lifeline,
bounded host collector and trusted worker monitor handle cancellation/parent exit.
Windows/Linux launchers remain unchanged. Mac private copies do not provide the
same immutable-file guarantee as Linux memfd against a malicious native process
of the same user; replacement of the trusted host is outside this boundary too.
The native 192 MiB threshold is sampled, not a kernel allocation limit. App Sandbox
alone is insufficient; isolation depends on the fixed import-free Wasm guest.

The following LPAC and atomic job-attachment details apply to Windows:

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
