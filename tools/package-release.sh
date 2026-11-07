#!/bin/sh
# Builds and verifies a new local unsigned Linux x86_64 release and its update archive, as
# tools/Package-Release.ps1 and tools/New-UpdateArchive.ps1 do for Windows. It never
# replaces an existing version and never uploads anything.
#
#   tools/package-release.sh [--offline]
#
# The release is tools/releases/Rocket-<version>-linux-x64, beside its update archive
# Rocket-<version>-linux-x64.tar.gz: POSIX ustar with that one folder, compressed with
# gzip. The folder holds rocket-update.json, the manifest Rocket's updater checks, and
# SHA256SUMS.txt, which `sha256sum -c SHA256SUMS.txt` checks by hand.
set -eu
offline=
for argument in "$@"; do
    case "$argument" in
        --offline) offline=--offline ;;
        *)
            echo "usage: tools/package-release.sh [--offline]" >&2
            exit 2
            ;;
    esac
done
fail() {
    echo "$*" >&2
    exit 1
}
root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"
version=$(sed -n '/^\[workspace\.package\]/,/^\[/s/^version *= *"\([0-9.]*\)" *$/\1/p' Cargo.toml)
echo "$version" | grep -Eqx '(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)' ||
    fail "Expected an explicit three-part workspace release version."
name="Rocket-$version-linux-x64"
releases="$root/tools/releases"
bundle="$releases/$name"
archive="$releases/$name.tar.gz"
for existing in "$bundle" "$archive"; do
    [ ! -e "$existing" ] ||
        fail "Release $version for Linux already exists; choose a new version instead of replacing it."
done
manifest=$(sha256sum Cargo.toml)

tools/build-local.sh release $offline
source="$root/target/release"
# The update checks need every one of these (BINARIES in crates/rocket-ui/src/app_updates).
binaries="rocket rocket-mcp rocket-cli rocket-rebase-editor rocket-lua-host rocket-ssh"
documents="CHANGELOG.md CLI.md DEPENDENCIES.md HOSTING.md INTEGRATIONS.md LICENSE-APACHE
LICENSE-MIT README.md SECURITY.md THIRD-PARTY-NOTICES.txt docs/images/rocket-logo.png
crates/rocket-ui/src/guide.md tools/README.md"
# User-facing wiki pages ship with the app, as on Windows.
documents="$documents $(cd "$root" && ls docs/wiki/*.md)"
for file in $binaries; do
    [ -f "$source/$file" ] || fail "Missing build output: $file"
done
cli=$("$source/rocket-cli" --json --version) || fail "Built CLI version check failed."
echo "$cli" | grep -q '"ok":true' || fail "Built CLI version check failed."
[ "$(echo "$cli" | grep -o '"version":"[^"]*"' | sort -u)" = "\"version\":\"$version\"" ] ||
    fail "Built CLI version does not match the workspace release version."

# Only a verified candidate receives the release name.
mkdir -p "$releases"
stage=$(mktemp -d "$releases/.$name.XXXXXXXX.partial")
for file in $binaries; do
    cp "$source/$file" "$stage/$file"
    chmod 0755 "$stage/$file"
done
for file in $documents; do
    mkdir -p "$stage/$(dirname "$file")"
    cp "$root/$file" "$stage/$file"
    chmod 0644 "$stage/$file"
done
# Paths go into JSON unescaped, so only plain names are allowed.
files=$(cd "$stage" && find . -type f | sed 's|^\./||' | LC_ALL=C sort)
count=$(echo "$files" | wc -l)
[ "$count" -lt 20000 ] || fail "Release exceeds the updater's file limit."
records=
for file in $files; do
    echo "$file" | grep -Eqx '[A-Za-z0-9._/-]+' || fail "Unexpected file name in the release: $file"
    size=$(stat -c %s "$stage/$file")
    [ "$size" -le 268435456 ] || fail "Release file exceeds the updater's size limit: $file"
    sha=$(sha256sum "$stage/$file" | cut -d ' ' -f 1)
    records="$records${records:+,}{\"path\":\"$file\",\"sha256\":\"$sha\",\"size\":$size}"
done
printf '{"schema":1,"version":"%s","platform":"linux-x64","entrypoint":"rocket","files":[%s]}' \
    "$version" "$records" > "$stage/rocket-update.json"
[ "$(stat -c %s "$stage/rocket-update.json")" -le 4194304 ] || fail "Update metadata is too large."
chmod 0644 "$stage/rocket-update.json"
# SHA256SUMS.txt lists every file but itself, rocket-update.json included. It is written
# outside the folder first, so the listing never includes it.
sums=$(mktemp "$releases/.$name.sums.XXXXXXXX")
(cd "$stage" && find . -type f | sed 's|^\./||' | LC_ALL=C sort |
    while read -r file; do sha256sum "$file"; done) > "$sums"
mv "$sums" "$stage/SHA256SUMS.txt"
chmod 0644 "$stage/SHA256SUMS.txt"
# mktemp made the folder private; the release is an ordinary folder.
chmod 0755 "$stage"
total=$(cd "$stage" && find . -type f -printf '%s\n' | awk '{ s += $1 } END { print s }')
[ "$total" -le 1073741824 ] || fail "Release exceeds the updater's unpacked size limit."
(cd "$stage" && sha256sum --check --quiet --strict SHA256SUMS.txt) || fail "Release verification failed."
[ "$(cd "$stage" && find . -type f | wc -l)" -eq "$(($(wc -l < "$stage/SHA256SUMS.txt") + 1))" ] ||
    fail "Release verification found an unlisted file."
for file in $binaries; do
    [ "$(sha256sum < "$stage/$file")" = "$(sha256sum < "$source/$file")" ] ||
        fail "Build output changed while packaging: $file"
done
[ "$(sha256sum Cargo.toml)" = "$manifest" ] || fail "Workspace manifest changed during packaging."
[ ! -e "$bundle" ] || fail "Another process created this release version; candidate preserved: $stage"
mv "$stage" "$bundle"
echo "Verified unsigned release: $bundle"

# The archive: POSIX ustar, entries in name order with fixed owners and times, so the
# same release always packs to the same bytes.
epoch=${SOURCE_DATE_EPOCH:-$(git log -1 --format=%ct)}
partial=$(mktemp "$releases/.$name.tar.gz.XXXXXXXX.partial")
(cd "$releases" && tar --format=ustar --sort=name --owner=0 --group=0 --numeric-owner \
    --mtime="@$epoch" -I 'gzip -n -9' -cf "$partial" "$name") ||
    fail "Could not create update archive; candidate retained for inspection: $partial"
[ "$(stat -c %s "$partial")" -le 536870912 ] ||
    fail "Compressed update exceeds 512 MiB; candidate retained for inspection: $partial"
# POSIX sh reports only the last command's pipeline status. Check tar separately so
# a failed producer or damaged archive cannot be hidden by gzip/grep succeeding.
listing=$(tar -tzf "$partial") ||
    fail "Could not verify update archive; candidate retained for inspection: $partial"
outside=$(printf '%s\n' "$listing" | grep -cv "^$name\(/\|$\)" || true)
[ "$outside" -eq 0 ] || fail "Update archive has entries outside its folder; candidate retained."
(cd "$bundle" && sha256sum --check --quiet --strict SHA256SUMS.txt) ||
    fail "Release changed while archiving; candidate retained."
[ ! -e "$archive" ] || fail "This update archive already exists; never overwrite a published version."
mv "$partial" "$archive"
echo "Update archive: $archive"
echo "SHA-256: $(sha256sum "$archive" | cut -d ' ' -f 1)"
echo "Size: $(stat -c %s "$archive") bytes"
