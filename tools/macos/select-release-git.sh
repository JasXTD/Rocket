#!/usr/bin/env bash
# Prefer installed Git, otherwise build the Defender-scanned source in runner temp.
set -euo pipefail
umask 077
cd "$(dirname "$0")/../.."
probe="${RUNNER_TEMP:?Expected a disposable runner}/rocket-check-git-version"
rustc --edition=2024 -o "$probe" - <<'RUST'
#[allow(dead_code)]
#[path = "crates/rocket-git/src/runner/http_safety.rs"]
mod http_safety;
fn main() {
    let version = std::env::args().nth(1).unwrap_or_default();
    std::process::exit(i32::from(http_safety::check_version(&version, false).is_err()));
}
RUST

shopt -s nullglob
for git in /opt/homebrew/bin/git /usr/local/bin/git \
    /Library/Developer/CommandLineTools/usr/bin/git \
    /Applications/Xcode.app/Contents/Developer/usr/bin/git \
    /Applications/Xcode_*.app/Contents/Developer/usr/bin/git \
    /opt/homebrew/Cellar/git/*/bin/git /usr/local/Cellar/git/*/bin/git; do
  [[ -x "$git" && ! "$git" -ef /usr/bin/git ]] || continue
  version=$("$git" --version) || continue
  if "$probe" "$version"; then
    bin="$RUNNER_TEMP/rocket-release-git-bin"
    mkdir "$bin"
    ln -s "$git" "$bin/git"
    printf '%s\n' "$bin" >> "${GITHUB_PATH:?Expected the job path file}"
    echo 'Selected an installed Git accepted by Rocket.'
    exit 0
  fi
done

[[ ${GITHUB_ACTIONS:-} == true && ${RUNNER_OS:-} == macOS && ${ROCKET_MACOS_SANDBOX_DISPOSABLE_USER:-} == 1 ]]
read -r expected archive < tools/macos/release-git.sha256
if [[ ! $expected =~ ^[0-9a-f]{64}$ || $archive != git-2.56.1.tar.gz || ${ROCKET_DEFENDER_SCANNED_GIT_SHA:-} != "$expected" ]]; then
  echo '::error::Building Git requires a successful Defender scan of the pinned source in this run.' >&2
  exit 1
fi
work=$(mktemp -d "$RUNNER_TEMP/rocket-release-git.XXXXXXXX")
prefix="$work/prefix"
cd "$work"
curl --proto '=https' --fail --silent --show-error --max-time 120 --output "$archive" \
  "https://www.kernel.org/pub/software/scm/git/$archive"
printf '%s  %s\n' "$expected" "$archive" | shasum -a 256 --check --status
tar -xzf "$archive"
make -C git-2.56.1 -j3 prefix="$prefix" NO_GETTEXT=YesPlease NO_TCLTK=YesPlease install
version=$("$prefix/bin/git" --version)
[[ "$version" == 'git version 2.56.1' ]]
"$probe" "$version"
printf '%s\n' "$prefix/bin" >> "${GITHUB_PATH:?Expected the job path file}"
echo 'Built and verified Git 2.56.1 for this disposable runner.'
