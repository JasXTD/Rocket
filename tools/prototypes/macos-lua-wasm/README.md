# Lua / Pulley compatibility experiment

This standalone workspace does not enable Lua on macOS or change Rocket's
Windows/Linux runtime. It is not a production sandbox or release component.

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
source checksums. This is Linux-hosted compatibility evidence, not Mac sandbox
qualification. The original failing baseline is retained in the investigation.

### Reproduce in an isolated directory

After approval and scanning, `build-ci-guest.sh NEW_DEPS NEW_RESULTS` on Linux
downloads only the pinned official archives, verifies each hash, stages an
offline Cargo source directory and builds the fixed guest. It installs nothing
globally. The script refuses existing staging/results directories.

On a native host with the approved Rust 1.99.1, run `prepare-deps.cjs NEW_DEPS host`
with Node, set `CARGO_HOME` to `NEW_DEPS/cargo-home` and `CARGO_TARGET_DIR` to a
separate cache, then run `run-host-checks.sh ABSOLUTE_GUEST_WASM NEW_RESULTS` with
Bash. The fixed Wasm must come from the reviewed source build, never an extension.
`build-guest.sh` and `run-checks.sh` retain the original failing stable-std baseline;
the unwind-enabled guest uses `build-guest-unwind.sh`.

The manual `macos-wasm-prototype.yml` workflow rebuilds the guest in a disposable
Linux job (10-minute timeout) and verifies its same-run SHA-256 before testing a
Pulley host on Apple Silicon (15-minute timeout). It does not change a release or
enable production Lua. Results have one-day retention; no build output enters Git.

The 128 MiB limit applies to guest linear memory, not total native RSS. Native
memory-pressure/exception-heap testing, signed helper binding, OS containment,
parent death, inherited handles and production packaging remain separate gates.
The current test worker also contains compiler code and is not a release-size
measurement. Windows AppContainer and Linux Landlock/seccomp stay untouched.

The test plan covers declarative UI/Unicode, callback errors and Lua unwinding,
resource limits, exact imports, malformed output, cancellation and fresh state.
Use only synthetic fixtures. The ordinary application remains fail-closed on Mac.

See [the architecture and remaining qualification gates](../../../docs/MacOS-lua-wasm.md).
