# macOS Lua: WebAssembly isolation proposal

Status: research only, 2026-11-09. The user approved investigation after the
[native App Sandbox probes](MacOS-lua-sandbox.md#measured-result). No WebAssembly
package or compiler has been installed, no guest has been built, and Mac Lua
remains unavailable. Windows and Linux retain their existing implementations.

## Recommendation

Prototype the existing Lua runtime inside **Wasmtime 50.0.1 using its Pulley
interpreter**, in a separate Mac worker with App Sandbox as an outer boundary.
Treat this version as a research candidate, not an approved dependency or a safety
certification. Do the compiler/runtime compatibility proof before UI integration.

The objective is unchanged Lua scripts and Rocket's existing reviewed request/UI
protocol. A Lua/native-library fault should still be confined to WebAssembly
memory, without an API for accessing the native worker's container, credentials,
files, sockets or processes. The native VM and its embedding remain trusted code.
A defect there could expose the weaker outer sandbox; WebAssembly does not make
those residual risks disappear.

| Candidate | Finding | Decision for the first prototype |
| --- | --- | --- |
| App Sandbox alone | Native testing found persistent container access and no established 128 MiB native cap. | Insufficient by itself; retain as an additional boundary. |
| Wasmi 2.0.0 | Its versioned feature table lists exception handling as planned. Rocket's current Lua WASI build uses it for error unwinding. | Do not assume compatibility or remove error recovery to make it fit. |
| Wasmtime 50.0.1 / Pulley | The versioned documentation describes interpreter parity with Cranelift, including the required Wasm features. Exceptions require Wasmtime's `gc` build feature. | Preferred compatibility experiment; prove the exact configuration and resource bounds. |
| Native-code Wasmtime execution | Another possible route, but generated executable memory adds Mac signing/entitlement considerations. | Defer while the interpreter route is assessed. |

Sources: [Wasmi 2.0.0 feature table](https://github.com/wasmi-labs/wasmi/blob/v2.0.0/README.md),
[Wasmtime 50.0.1 Pulley design](https://github.com/bytecodealliance/wasmtime/blob/v50.0.1/docs/examples-pulley.md),
[versioned exception configuration](https://github.com/bytecodealliance/wasmtime/blob/v50.0.1/crates/wasmtime/src/config.rs).
These establish source-level feasibility, not tested Rocket compatibility.

## Trust and data flow

```mermaid
flowchart LR
    H["Rocket host: permissions and reviewed actions"]
    S["Owned native worker: App Sandbox and watchdog"]
    W["Pulley: bounded WebAssembly instance"]
    L["Fixed Lua runtime plus untrusted Lua source"]
    H -->|"bounded input JSON"| S
    S -->|"checked memory copy"| W
    W --> L
    L -->|"bounded result or request"| W
    W --> S
    S -->|"validated response"| H
```

This is a proposed flow, not an implemented architecture. All HTTP/Git actions and
credential use stay in the existing host-side authorization flow. The guest only
returns data. Never pass native pointers, file descriptors, repository handles,
environment contents, tokens or an unrestricted host-call interface to it.

Ship one trusted guest, built from Rocket's reviewed Lua/runtime sources. Extension
authors provide Lua source, not Wasm/Pulley modules. Prefer embedding the compiled
guest bytes in the helper so their identity follows the helper's code signature;
otherwise verify a bound digest over the exact bytes subsequently loaded, with no
path re-open race. Keep runtime version, target, feature settings and guest digest
bound together, and reject mismatches before processing script input.

Pulley supports precompilation during the build. That could remove the compiler
from the shipped worker and avoid native JIT execution. However, Wasmtime's
precompiled format is trusted executable input, not a validated interchange
format. Never deserialize an extension-supplied or unverified cache file. The
minimum runtime feature set and absence of JIT entitlements still need native
proof. [Wasmtime precompilation and safety requirements](https://docs.wasmtime.dev/examples-pre-compiling-wasm.html).

## Compatibility work before implementation

Rocket currently pins `mlua 0.12.2`, `mlua-sys 0.13.0` and `lua-src 551.1.0`
(Lua 5.5.1). Local source inspection found that `lua-src`'s WASI path emits standard
Wasm exceptions and links `setjmp`; the `mlua` README documents Emscripten support,
which does not prove the proposed WASI target works. WASI SDK's own documentation
describes its setjmp/longjmp support as evolving. Check real Lua `pcall`, `xpcall`,
syntax errors, nested callbacks, allocator failure, finalizers and Rust/Lua unwind
boundaries before accepting the toolchain.
[WASI SDK 34 error-unwinding implementation](https://github.com/WebAssembly/wasi-sdk/blob/wasi-sdk-34/SetjmpLongjmp.md).

The reusable code is `crates/rocket-integrations/src/runtime.rs`: it accepts and
returns JSON and already exposes declarative UI/request functions. It currently
depends on the crate's request model, while the crate itself depends on
`rocket-git`. A small shared runtime/protocol layer should separate those pieces
from catalog/storage/native launch code; do not copy the SDK or compile the whole
Git backend into the guest. Keep existing host validation authoritative. Extract
only after compatibility is proved, with existing tests protecting Windows/Linux.

The current native `host.rs` calls `enter_isolation` before reading stdin. Do not
make that entry point succeed on Mac as a shortcut. The proposed Mac launcher and
guest need their own verified path; refusal remains the default during experiments.

Inspect the guest's actual imports and exports. Target zero ambient capabilities:
no filesystem, socket, process, environment or clock imports. If compiler/runtime
support requires a small shim, enumerate it explicitly and prove its bounded
in-memory behavior. Do not silently link a general WASI implementation or inherit
stdio, directories or environment. Unexpected imports are a build/test failure,
not a reason to grant more host access. The exact minimal import set is unknown
until the first real build.

## Resource and lifecycle contract

These are acceptance criteria for the prototype, not current guarantees:

- One fresh native worker and fresh guest store/instance per invocation; destroy
  them after one response. Do not expose or reuse persistent app-container state.
- Exactly one non-shared 32-bit linear memory, capped at 128 MiB in both the guest
  module and the runtime limiter. Reject shared memory, threads, extra memories,
  dynamic modules and unused proposals. Cap table entries and instance counts too.
- Account separately for the Wasm execution stack, exception/GC storage, host JSON
  buffers and VM bookkeeping. A linear-memory cap is **not** a 128 MiB process RSS
  cap. The exception implementation's additional allocations are an explicit test
  gate; do not disable needed exception support to hide that accounting problem.
- Preserve the current 8 MiB input, 1 MiB output and 4 KiB diagnostic limits.
  Check pointer/length arithmetic, bounds and allocation limits before copying;
  cap parsing depth and aggregate response size. Never trust a guest response to
  authorize a host action. Do not log script payloads or credentials.
- Fuel limits plus the existing five-second wall deadline. Test loops inside Lua
  library code and repeated error handling as well as ordinary Lua loops. Keep a
  host-side termination path independent of guest cooperation; Stop must wait for
  actual exit and reject late/stale results.
- Verify host and supervisor crash/exit handling, early cancellation, startup
  races, inherited handles and absence of surviving children. App Sandbox and an
  interpreter alone do not solve process lifecycle ownership.

Wasmtime's [resource-limiter contract](https://github.com/bytecodealliance/wasmtime/blob/v50.0.1/crates/wasmtime/src/runtime/limits.rs)
explicitly excludes some native/store allocations and does not apply its normal
growth callback to shared memory. This is why resource totals and shared-memory
rejection must be independent checks rather than a single memory-size setting.

## Supply-chain research and approval boundary

Read-only metadata checks on 2026-11-09 confirmed the official crates.io records
for `wasmtime 50.0.1` and `pulley-interpreter 50.0.1` point to Bytecode Alliance's
repository. Both were published 2026-10-29, were not yanked, and declare Rust 1.97
(the project uses 1.99.1). OSV's version queries returned no listed advisories for
these two exact versions or the Wasmi comparator. **This is not a full dependency
audit or evidence that the packages contain no exploitable defect.**

The [50.0.1 release notes](https://github.com/bytecodealliance/wasmtime/releases/tag/v50.0.1)
include recent security fixes. An older
[Cranelift sandbox-escape advisory](https://github.com/bytecodealliance/wasmtime/security/advisories/GHSA-897f-g5gm-fmxm)
also illustrates why the compiler and transitive dependencies belong in the audit,
even if compilation occurs only during Rocket's build. A small host API does not
remove VM/compiler supply-chain risk.

The next dependency proposal must list exact packages/features and the resolved
transitive graph, licenses, official archives/checksums, relevant advisories and
source/build-script findings. Obtain the user's installation/download approval
and required antivirus scan before executing anything new. Candidate build tools
are WASI SDK 34 and the existing Rust 1.99.1 toolchain's WASI target; neither is
installed here. WSL has no detected Clang/Wasm linker or Wasmtime. No packages,
Cargo manifests, lockfiles, PATH settings or system software changed in this pass.

Research used publisher documentation, versioned source and public registry/advisory
JSON as evidence only. Embedded instructions were not executed; no personal
information or credentials were supplied. Web-hosted `.txt` content was excluded.

## Staged acceptance

1. **Dependency review and isolated compatibility prototype:** approve the exact
   build/runtime graph, build only the fixed Lua guest and a test runner, enumerate
   imports, and prove JSON/UI-request parity and error unwinding. Measure startup,
   package size, CPU and memory with representative integrations; interpreter
   performance is not yet measured in Rocket.
2. **Adversarial VM tests:** attempt out-of-bounds access, forged result pointers,
   oversized input/output, memory/table/exception growth, infinite loops, forbidden
   imports and cross-invocation state. Use separately constructed test modules as
   negative fixtures without accepting them through the production loading path.
3. **Native Mac containment and lifecycle:** test the signed helper, entitlements,
   module binding/replacement, files/network/IPC, parent death, cancellation and
   resource pressure on the capped disposable CI runner. Do not infer these results
   from Linux execution or the earlier Objective-C characterization.
4. **Integration and packaging:** only after the gates pass, connect the existing
   UI and broker with clear errors, then perform affected Windows/Linux regressions.
   Preserve the refusal path for unsupported or invalid packages. Developer ID,
   notarization and bare-metal acceptance remain later release gates.

The investigation is complete; dependency adoption, the compatibility prototype
and production sandbox are not. No remaining native-run budget is spent merely
to revalidate this documentation.
