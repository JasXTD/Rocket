# Lua / Pulley compatibility experiment

This separately locked workspace supplies the qualified Mac fixed-guest helper
as well as its tests and offline compiler. `tools/macos/build-lua-helper.sh` signs
and packages only the worker; Rocket's host embeds that final signed hash. The
standalone probes are not shipped. Windows/Linux keep their native workers and
root dependency graph. See `docs/MacOS-lua-wasm.md` for the app binding and limits.

The guest compiles the application's actual `runtime.rs` and schema `model.rs`
through a small compatibility facade, avoiding a second implementation of the
Lua SDK or a dependency on the native Git/credential backend. Input/output remain
the existing bounded JSON protocol. Lua source is untrusted; the compiled guest
and the native VM are trusted, hash-reviewed build inputs.

Dependencies were approved on 2026-11-09. All 127 archives matched their official
SHA-256 hashes and the user reported a clean Malwarebytes scan before extraction.
The standalone lockfile and `approved-crates.json` bind all 124 registry versions.
No system toolchain, root Cargo dependency or production package is changed.

The first build with stable Rust's prebuilt WASI std failed four Rust callback
error paths. Rebuilding std with approved nightly-2026-11-08 and SDK 34's existing
`eh/libunwind.a` fixes those paths. `approved-unwind-crates.json` pins the 25
additional standard-library registry versions; `approved-tools.json` pins the
four Linux nightly components and SDK archive. These additional 29 archives were
also SHA-256 verified and reported clean by the user's Malwarebytes scan before
execution. Native Wasmtime hosts continue to use Rust 1.99.1.

The final local run passed 18 runtime/schema tests, 9 VM tests, 7 controller tests,
7 archive tests and 14 complete JSON comparisons against the native runtime,
plus oversized-input refusal and cancellation of an executing worker. All 149
registry archives were re-extracted by the CI stager and matched the reviewed
source checksums. The same affected checks subsequently passed on native Darwin/
arm64 in [CI run 42507855345](https://github.com/JasXTD/Rocket/actions/runs/42507855345),
including exact response comparisons and a 0.419 ms Stop-to-close sample. The
downloaded artifacts and same-run guest hashes were verified. This establishes
prototype compatibility, not production sandbox qualification. The original
failing baseline is retained in the investigation.

### Reproduce in an isolated directory

After approval and scanning, `build-ci-guest.sh NEW_DEPS NEW_RESULTS` on Linux
downloads only the pinned official archives, verifies each hash, stages an
offline Cargo source directory and builds the fixed guest. It installs nothing
globally. The script refuses existing staging/results directories.

On a native host with the approved Rust 1.99.1, run `prepare-deps.cjs NEW_DEPS host`
with Node, set `CARGO_HOME` to `NEW_DEPS/cargo-home` and `CARGO_TARGET_DIR` to a
separate cache, then run `run-host-checks.sh ABSOLUTE_GUEST_WASM NEW_RESULTS` with
Bash. The fixed Wasm must come from the reviewed source build, never an extension.
Mac containment tests additionally require `ROCKET_MACOS_SANDBOX_DISPOSABLE_USER=1`
and an ordinary disposable OS user. They create fresh ad-hoc signed app bundles
and leave their containers to runner teardown. Never use a personal Mac account
for these probes. The actual Mac worker refuses unsigned or extra-entitlement
launches and requires a dedicated parent lifeline on descriptor 3.
`build-guest.sh` and `run-checks.sh` retain the original failing stable-std baseline;
the unwind-enabled guest uses `build-guest-unwind.sh`.

The manual `macos-wasm-prototype.yml` workflow rebuilds the guest in a disposable
Linux job (5-minute timeout) and verifies its same-run SHA-256 before testing a
Pulley host on Apple Silicon (15-minute timeout). Its integration scope builds
and verifies the bound development app; prototype scope reruns isolated checks.
It publishes no release. Results have one-day retention; build output stays out of Git.

The 128 MiB limit applies to guest linear memory, not total native RSS. The new
qualification runner covers bounded exception pressure, signed App Sandbox
file/network controls, inherited descriptors, parent/lifeline loss and private/
shared native pressure. Its 192 MiB native watchdog is a sampled threshold with
possible overshoot, not a hard allocation cap. Local lifecycle/VM checks and all
18 containment checks pass on native Apple Silicon in
[run 42514579040](https://github.com/JasXTD/Rocket/actions/runs/42514579040), including
the actual Lua worker's explicit lifeline-loss exit code. The native run also
passes 12 VM, 18 runtime/schema, 14 Node checks, all 14 exact Lua comparisons and
input/Stop cases; artifact digests and the zero-import guest were verified.
App helper identity, broker integration and packaging subsequently passed in
[run 42517429117](https://github.com/JasXTD/Rocket/actions/runs/42517429117).
Bare-metal and Developer ID distribution testing remain separate gates.
The current test worker also contains compiler code and is not a release-size
measurement. Windows AppContainer and Linux Landlock/seccomp stay untouched.

The test plan covers declarative UI/Unicode, callback errors and Lua unwinding,
resource limits, exact imports, malformed output, cancellation and fresh state.
Use only synthetic fixtures. Mac applications without a matching bound helper
remain fail-closed; the integration workflow builds and tests the bound app.

See [the architecture and remaining qualification gates](../../../docs/MacOS-lua-wasm.md).
