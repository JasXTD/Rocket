#!/bin/bash
# Source this first-party helper after locating the repository root. No installs.
rocket_macos_architecture() {
    if [ "$(uname -s)" != Darwin ]; then
        echo 'This build requires native macOS, not Windows or Linux.' >&2
        return 2
    fi
    case "$(uname -m)" in
        arm64) ROCKET_MAC_ARCH=arm64; ROCKET_MAC_TARGET=aarch64-apple-darwin; ROCKET_MAC_CLANG_ARCH=arm64 ;;
        x86_64)
            if [ "$(/usr/sbin/sysctl -in sysctl.proc_translated 2>/dev/null || true)" = 1 ]; then
                echo 'Run a native Mac toolchain; Rosetta builds are not supported.' >&2
                return 2
            fi
            ROCKET_MAC_ARCH=x64; ROCKET_MAC_TARGET=x86_64-apple-darwin; ROCKET_MAC_CLANG_ARCH=x86_64 ;;
        *) echo 'Supported Mac architectures are Apple Silicon arm64 and Intel x86_64.' >&2; return 2 ;;
    esac
}
