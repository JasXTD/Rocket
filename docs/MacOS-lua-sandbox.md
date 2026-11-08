# macOS Lua sandbox investigation

Status: design investigation, 2026-11-08. **No macOS sandbox is implemented or
verified. Lua remains unavailable on macOS.** There is no environment variable,
missing-hash workaround, in-process execution or unrestricted worker fallback.
The development CI artifact excludes the Lua worker.

## Current contract

`IntegrationWorker::run` checks cancellation and process privileges before worker
access. Supported Windows/Linux hosts require a bound SHA-256 and their existing
launcher isolation. macOS now reports `integration_worker_sandbox` before hash
validation, so an unavailable sandbox cannot be mistaken for a repairable bundle.
The actual worker entry point calls `enter_isolation` before reading input.

The host supplies bounded JSON, not a repository handle or ambient credentials.
The existing transport allows at most 8 MiB of input, 1 MiB of output, 4 KiB of
stderr and a five-second invocation. Lua has its own restricted API, instruction
and allocation limits, but a runtime exploit would bypass language-level limits.
The OS process boundary must still protect the user's files, secrets and network.

## Candidate: an independently sandboxed XPC helper

Apple documents separate sandbox entitlements for each target, including XPC
services. Its XPC services are managed by launchd and may be reused/restarted;
they are not automatically fresh per invocation. This is a useful isolation
mechanism to prototype, not evidence of equivalence to Rocket's other sandboxes.
See [Apple's XPC service design](https://developer.apple.com/library/archive/documentation/MacOSX/Conceptual/BPSystemStartup/Chapters/CreatingXPCServices.html)
and [App Sandbox entitlement definitions](https://developer.apple.com/library/archive/documentation/Miscellaneous/Reference/EntitlementKeyReference/Chapters/EnablingAppSandbox.html).

Proposed boundaries, subject to the tests below:

- Keep the Git/UI host outside the worker's sandbox. Give the helper its own
  identity and only `com.apple.security.app-sandbox`; do not inherit the host's
  rights or grant network, user-selected files, app groups, automation or broad
  filesystem exceptions. This is an initial entitlement proposal, not a verified
  final allowlist.
- Send bounded values over a narrowly defined IPC protocol, never arbitrary
  paths, file descriptors, Keychain references, security-scoped bookmarks or
  commands. Validate every response before the existing host-side permissions
  and review flows act on it. An XPC connection is not authorization to run Git.
- Verify both peers' code identity, protocol version and expected installed
  helper before accepting input. Bind identity to the signed bundle; test
  replacement races. Signing changes executable bytes: any embedded worker hash
  must be calculated after the helper's final signing, before signing the host.
- Require one invocation per worker process with a verified exit before another
  invocation. Define invalidation, cancellation, reconnect and crash handling so
  launchd cannot silently retry a partially completed request or reuse Lua state.
- Establish OS resource limits before Lua input, with an independent host
  watchdog and an explicit parent-exit mechanism. Linux's `prctl`, seccomp,
  Landlock, sealed memfd and resource-limit constants cannot be ported directly.
  Prove which Darwin limits bound total resident/virtual memory; an allocation
  limit inside Lua is not a replacement. Do not promise a Windows-equivalent
  memory cap until pressure tests establish it.

App Sandbox permits access to some system resources and an app's own container;
it must not be described as “only three pipes and no filesystem.” A compromised
worker may be able to write persistent container data. Determine whether a
disposable container/identity or a different isolation design is necessary to
prevent cross-run persistence; deleting files after execution alone is not proof.
Similarly, sandboxed child processes and Mach services require explicit attack
tests; absence of a network entitlement does not prove every IPC route is denied.
Do not ship a deprecated/custom `sandbox-exec` profile as an assumed substitute
for this investigation. No App Store distribution decision is made here.

For the prototype, investigate a small trusted supervisor using current SDK
definitions for `kqueue`/`EVFILT_PROC`/`NOTE_EXIT` to observe host death and stop
only its owned worker. The event itself does not kill a process; startup races,
PID reuse and supervisor death need tests. Apple's archived
[kqueue manual](https://developer.apple.com/library/archive/documentation/System/Conceptual/ManPages_iPhoneOS/man2/kqueue.2.html)
establishes the mechanism, not a tested lifecycle design. Its archived
[resource-limit manual](https://developer.apple.com/library/archive/documentation/System/Conceptual/ManPages_iPhoneOS/man2/setrlimit.2.html)
also describes RSS as a pressure preference, so do not infer a hard total-memory
cap from the name `RLIMIT_RSS` or copy Linux's numeric limits. Verify the current
SDK/kernel behavior and measure it on each supported macOS version.

## Required native proof before enabling Lua

| Probe | Required result |
| --- | --- |
| Missing/incorrect sandbox or signature | Refuse before input; do not start an unrestricted replacement. |
| Files, links, inherited descriptors | Cannot read synthetic private files, repositories, SSH/credential fixtures or write outside the deliberately permitted sandbox. |
| Network and IPC | Cannot connect/listen on loopback or external sockets, obtain host capabilities or use an unreviewed IPC route. |
| Native-code escape probes | Exercise raw OS operations outside Lua's language restrictions, so the test actually measures the OS boundary. |
| CPU / memory / threads / processes | Enforce measured limits without exhausting the machine or leaving descendants. Test shared mappings, stacks and executable memory too. |
| Host exit, crash, Stop and timeout | Worker and descendants stop within a measured bound; an invalidated connection alone is insufficient proof. |
| Malformed/oversized IPC and stale results | Reject before large allocation or host actions; preserve the previous UI result and ignore superseded replies. |
| Two successive invocations | No globals, handles, queued messages, credentials or container state escape into the next invocation. |
| Packaged launch | The final signed helper has the reviewed entitlements, same restrictions and identity checks outside the development checkout. |

Use a disposable ordinary-user account, synthetic files and loopback services on
macOS CI and then EC2 Mac. Protect the host from hostile resource probes with a
separate test supervisor and termination budget. If any requirement cannot be
enforced with the candidate design, keep Lua disabled and review the architecture
before adding a weaker fallback. New dependencies/bindings require their own
provenance and vulnerability review before adoption.

The stage-1 tests prove only **refusal**: the host refuses bound/unbound workers,
pre-cancellation remains effective, and the Cargo-built helper must return the
sandbox error without waiting for stdin to close or executing supplied Lua.
They do not count as sandbox escape tests or permission to enable extensions.
