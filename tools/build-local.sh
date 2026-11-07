#!/bin/sh
# Builds a matching unsigned Linux x86_64 bundle, as tools/Build-Local.ps1 does on Windows:
# the Lua worker first, then the hosts that embed its SHA-256. Requires the Rust toolchain
# and a C compiler.
#
#   tools/build-local.sh [release|debug] [--offline]
set -eu
profile=release
offline=
for argument in "$@"; do
    case "$argument" in
        release | debug) profile=$argument ;;
        --offline) offline=--offline ;;
        *)
            echo "usage: tools/build-local.sh [release|debug] [--offline]" >&2
            exit 2
            ;;
    esac
done
root=$(cd "$(dirname "$0")/.." && pwd)
target="$root/target"
cd "$root"
# Fix the target directory; a Cargo target override would change its layout.
if [ -n "${CARGO_BUILD_TARGET:-}" ]; then
    echo "Unset CARGO_BUILD_TARGET for this native build." >&2
    exit 1
fi
if [ "$(uname -s)" != Linux ] || [ "$(uname -m)" != x86_64 ]; then
    echo "The Lua sandbox needs Linux x86_64." >&2
    exit 1
fi
set -- build --locked --target-dir "$target"
if [ -n "$offline" ]; then
    set -- "$@" "$offline"
fi
cargo_profile=dev
if [ "$profile" = release ]; then
    set -- "$@" --release
    cargo_profile=release
fi
# The worker locks itself down after the dynamic loader has opened its libraries, so it
# links them as usual; Windows links its runtime statically instead. Rocket copies and
# hashes the whole worker for every run, so it is built without debug info, for an
# explicit target that keeps this variant's cache apart from the ordinary host builds.
worker_target=x86_64-unknown-linux-gnu
cargo "$@" --target "$worker_target" --config "profile.$cargo_profile.strip=\"debuginfo\"" \
    -p rocket-integrations --features runtime --bin rocket-lua-host
worker="$target/$profile/rocket-lua-host"
mkdir -p "$target/$profile"
cp -f "$target/$worker_target/$profile/rocket-lua-host" "$worker"
expected=$(sha256sum "$worker" | cut -d ' ' -f 1)
# Exclude the worker's package from binary targets so the file just hashed cannot be
# relinked while building the hosts that embed its digest.
ROCKET_LUA_WORKER_SHA256=$expected cargo "$@" --workspace --exclude rocket-integrations \
    --bins --features rocket-integrations/runtime
if [ "$(sha256sum "$worker" | cut -d ' ' -f 1)" != "$expected" ]; then
    echo "Worker changed during the host build. Rebuild the complete bundle." >&2
    exit 1
fi
echo "Built unsigned $profile bundle in $target/$profile"
echo "Embedded Lua worker SHA-256: $expected"
