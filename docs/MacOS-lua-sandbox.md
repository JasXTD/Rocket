# macOS Lua sandbox investigation

Status: native characterization completed; WebAssembly route investigated,
2026-11-09. **No production
macOS sandbox is implemented or verified. Lua remains unavailable on macOS.** There is no environment variable,
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

## First native characterization

`tools/test-macos-sandbox.sh` runs a first-party Objective-C probe compiled against
the installed Apple SDK, signed ad hoc with hardened runtime and exactly the App
Sandbox entitlement. It needs no Developer ID, notarization, Lua input, third-party
package or system configuration change. Run it only in a disposable ordinary OS
user: macOS creates an app container with a fresh random bundle identity, and the
runner deliberately never deletes entries under a normal user's Library/Containers.

The `sandbox` choice in the existing manual Apple Silicon workflow executes only
this harness, its lifecycle tests, and the real host/worker refusal checks. It does
not build the UI, overwrite a package, contact a credential service, or enable Lua.
Its job retains the same 15-minute cap; account/budget approval is still required.

Measurements include:

- Exact signed entitlements, ordinary-user identity and actual container location.
- A readable/writable synthetic private file and reachable loopback listener as
  positive controls before asserting that the sandbox denies access.
- Persistence of a synthetic container marker between two fresh processes.
- One immediately exiting/reaped child, with and without a hard `RLIMIT_NPROC` of 0.
- Separate `RLIMIT_AS` and `RLIMIT_DATA` setting/read-back, followed by bounded
  private/shared `mmap` and Mach VM reservations above 128 MiB. Reservations are
  released without touching their pages; this tests address-space enforcement,
  **not** resident-memory pressure. Setter success alone is never a passing limit.
- Explicit Stop, timed termination, bounded streaming output and preservation of
  pre-existing output files. Every native probe also has its own five-second alarm.

`result.json` distinguishes completed characterization from production readiness.
It always keeps `production_ready: false`; remaining IPC, identity, inheritance,
parent-death, real-worker and package gates cannot be skipped because the initial
probes pass. The probe is not an XPC implementation, and its Stop measurement is
the test supervisor's measurement, not proof of Rocket's eventual worker lifecycle.

The public macOS 15-era [XNU resource-limit implementation](https://github.com/apple-oss-distributions/xnu/blob/xnu-11417.140.69/bsd/kern/kern_resource.c)
routes `RLIMIT_AS` and `RLIMIT_DATA` to VM limits. That is stronger evidence than
old RSS documentation, but the pre-mapped shared cache and native allocation paths
still require measurement. Do not infer a 128 MiB hard limit from source alone.
Apple explicitly documents unrestricted access to an app's own container in
[Protecting user data with App Sandbox](https://developer.apple.com/documentation/security/protecting-user-data-with-app-sandbox).
The persistence probe checks the concrete impact on Rocket's per-invocation contract.

### Measured result

[Native run 42499156369](https://github.com/JasXTD/Rocket/actions/runs/42499156369)
passed on `b1a4cbfefcb9f1a9f11bf2b434e3b52724d02823`, macOS 15.8.2,
Apple Silicon, SDK 15.5. It ran 10 controller regressions, six native checks and
four actual host/worker refusal tests, with no failures or skipped tests. The same
10 controller regressions passed on Windows and WSL; WSL also verified the script
refuses a non-Mac host before creating output.

| Observation | Measured result and limit of the evidence |
| --- | --- |
| Exact entitlement and ordinary-user launch | Verified; only App Sandbox was granted. |
| Synthetic outside-container file access and loopback connection | Denied, with successful unsandboxed positive controls. This does not cover every filesystem or IPC route. |
| Container state across fresh processes | The second process read the first process's marker. Fresh processes alone do not remove persistent capabilities. |
| Child creation | App Sandbox allowed one child, which immediately exited and was reaped. A separately tested hard `RLIMIT_NPROC=0` prevented that fork. |
| 128 MiB `RLIMIT_AS` / `RLIMIT_DATA` | Both setters returned `EINVAL` in this Foundation helper. All six allocation probes correctly reported `attempted: false`; no memory cap was established. This is not a measurement of every possible native helper design. |
| Stop | The controller observed process close 0.339 ms after sending its kill signal in this run. This is test-supervisor timing, not Rocket UI cancellation or parent-death proof. |

The downloaded evidence ZIP was checked against GitHub's SHA-256:
`87f52ef7365d8e2ad303b585584da46be2bd33e63cdd71b5d20e86f7bd80b33e`.
Resource-limit read-backs use decimal strings so `RLIM_INFINITY` cannot be rounded
into misleading evidence by JavaScript. The earlier run exposed two harness
representation bugs (Objective-C numeric booleans and large-integer precision);
both were corrected before the final result above.

**App Sandbox alone does not meet the current invocation contract.** The user
approved investigating an additional WebAssembly boundary; see the
[proposed Lua-in-WebAssembly prototype](MacOS-lua-wasm.md). This is a change to
the candidate architecture, not permission to weaken the existing isolation or
enable production Lua. Broker identity, Mach/credential access, inherited handles,
parent/supervisor death and final package binding remain unproved.

No production launcher, VM security rule, dependency, Windows/Linux sandbox or
release package changed in this characterization pass. The final workflow took
38 seconds. The existing task ledger totals 53 rounded job-minutes, approximately
$3.286 of the user-approved $6 included-compute cap; this is a conservative estimate,
not a GitHub invoice. No further native run is needed for this research report.
