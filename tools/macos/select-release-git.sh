#!/usr/bin/env bash
# Pick an installed Git for native release tests using the same gate as Rocket.
set -euo pipefail
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
echo '::error::None of the installed Homebrew or Xcode Git executables passed Rocket’s security version check.' >&2
exit 1
