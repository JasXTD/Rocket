#!/bin/sh
# Test the Linux packager with tiny synthetic build outputs and the real tar/gzip.
# No release build, network access or installation is performed.
set -eu
root=$(cd "$(dirname "$0")/../.." && pwd)
fixtures=$(mktemp -d /tmp/rocket-package-test.XXXXXXXX)
failed=0
for mode in normal create-failure list-failure; do
    repo="$fixtures/$mode"
    mkdir -p "$repo/tools" "$repo/target/release" "$repo/bin"
    cp "$root/tools/package-release.sh" "$repo/tools/package-release.sh"
    printf '[workspace.package]\nversion = "9.0.0"\n' > "$repo/Cargo.toml"
    printf '#!/bin/sh\nexit 0\n' > "$repo/tools/build-local.sh"
    chmod 0755 "$repo/tools/build-local.sh"
    for program in rocket rocket-mcp rocket-cli rocket-rebase-editor rocket-lua-host rocket-ssh; do
        printf '#!/bin/sh\nprintf '\''{"ok":true,"version":"9.0.0"}\\n'\''\n' > "$repo/target/release/$program"
        chmod 0755 "$repo/target/release/$program"
    done
    for file in CHANGELOG.md CLI.md DEPENDENCIES.md HOSTING.md INTEGRATIONS.md LICENSE-APACHE LICENSE-MIT README.md SECURITY.md THIRD-PARTY-NOTICES.txt docs/images/rocket-logo.png crates/rocket-ui/src/guide.md tools/README.md docs/wiki/Quick-start.md; do
        mkdir -p "$repo/$(dirname "$file")"
        printf 'synthetic release fixture\n' > "$repo/$file"
    done
    # These failures follow valid tar output, exposing shell pipelines that accidentally
    # treat a successful gzip/grep as proof that tar succeeded too.
    cat > "$repo/bin/tar" <<'SH'
#!/bin/sh
case "$ROCKET_TEST_TAR_FAILURE:$*" in
    create-failure:*--format=ustar*) /usr/bin/tar "$@"; exit 31 ;;
    list-failure:*-tzf*) /usr/bin/tar "$@"; exit 32 ;;
    *) exec /usr/bin/tar "$@" ;;
esac
SH
    chmod 0755 "$repo/bin/tar"
    if PATH="$repo/bin:/usr/bin:/bin" SOURCE_DATE_EPOCH=1 ROCKET_TEST_TAR_FAILURE="$mode" \
        /bin/sh "$repo/tools/package-release.sh" --offline > "$repo/result.log" 2>&1; then
        code=0
    else
        code=$?
    fi
    archive="$repo/tools/releases/Rocket-9.0.0-linux-x64.tar.gz"
    if [ "$mode" = normal ]; then
        test "$code" = 0
        test -f "$archive"
        mkdir "$repo/unpacked"
        /usr/bin/tar -xzf "$archive" -C "$repo/unpacked"
        (cd "$repo/unpacked/Rocket-9.0.0-linux-x64" && sha256sum --check --quiet --strict SHA256SUMS.txt)
        printf 'PASS normal archive and extracted checksums\n'
    elif [ "$code" != 0 ] && [ ! -e "$archive" ]; then
        printf 'PASS %s refused; no final archive\n' "$mode"
    else
        printf 'FAIL %s returned %s\n' "$mode" "$code"
        failed=1
    fi
done
if [ "$failed" != 0 ]; then
    printf 'Failed fixtures retained: %s\n' "$fixtures" >&2
    exit 1
fi
# Only the mktemp directory owned by this run, never a supplied path.
case "$fixtures" in
    /tmp/rocket-package-test.????????) test -d "$fixtures" && test ! -L "$fixtures" ;;
    *) exit 1 ;;
esac
rm -rf -- "$fixtures"
