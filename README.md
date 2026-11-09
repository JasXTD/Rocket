<p align="center">
  <img src="docs/images/rocket-logo.png" alt="Rocket logo" width="160" height="160">
</p>

<h1 align="center">Rocket</h1>

<p align="center">
  <strong>A native Git client built in Rust.</strong><br>
  An open-source alternative to Fork and GitKraken for Windows, Linux and macOS.
</p>

<p align="center">
  <a href="#features">Features</a> ·
  <a href="#platforms">Platforms</a> ·
  <a href="#build-and-run">Build and run</a> ·
  <a href="#comparison">Comparison</a> ·
  <a href="docs/wiki/Home.md">Wiki</a> ·
  <a href="crates/rocket-ui/src/guide.md">User guide</a> ·
  <a href="CHANGELOG.md">Changelog</a>
</p>

<p align="center">
  <picture>
    <source media="(prefers-color-scheme: light)" srcset="docs/media/hero-light.webp">
    <img src="docs/media/hero-dark.webp" width="960" alt="The Changes page in Clay, Rocket's default theme: staged and unstaged files, a commit message, and a diff with a checkbox on each changed line.">
  </picture>
</p>

Rocket puts everyday Git work and the advanced history tools in one native desktop app.
Open repositories in tabs, stage changes down to single lines, manage branches and remotes,
and resolve conflicts without leaving the window. The desktop app and the [CLI](CLI.md)
share one Rust backend that runs Git itself, so your repositories, credentials and SSH
setup stay as they are.

**New to Rocket?** Follow the [first-commit walkthrough](docs/wiki/Getting-started.md) or
open **Help → Setup Guide** in the app. **Ctrl+K** opens the command palette and **F1** the
guide.

## Features

**Changes and review**

- Stage whole files, hunks or single lines, and drag files between Staged and Unstaged.
- Unified or side-by-side diffs with syntax and word highlights; image compare with overlay,
  swipe and pixel differences.
- Commit details, file history and blame in their own windows, plus external editor, diff
  and merge tool profiles.

**History and branches**

- Commit graph across branches, file renames, and the repository tree at any commit.
- Find code with `git log -S` / `-G` and **Search in files** (`git grep`) at any commit.
- Merge, rebase and interactive rebase (reorder, reword, edit, squash, fixup, drop) with a
  preview first.
- Cherry-pick, revert, amend, bisect, reflog recovery and **Undo Last Commit**.
- A [Repository map](docs/wiki/Repository-map.md) of packages, folders, branches and who
  works where.

<p align="center">
  <picture>
    <source media="(prefers-color-scheme: light)" srcset="docs/media/history-light.webp">
    <img src="docs/media/history-dark.webp" width="760" alt="The Commits page with a branch graph across five branches, merges, and branch and remote labels.">
  </picture>
</p>

**Conflicts and recovery**

- A three-column conflict editor (Ours, Result, Theirs) with linked scrolling.
- Stashes with partial apply, patches and commit series, and Continue, Skip or Abort for
  paused operations.
- Discards keep recovery copies, and history rewrites keep the original commits reachable.

<p align="center">
  <picture>
    <source media="(prefers-color-scheme: light)" srcset="docs/media/conflict-light.webp">
    <img src="docs/media/conflict-dark.webp" width="880" alt="The three-column conflict editor with Ours, Result and Theirs, and two conflicts numbered and linked across the columns.">
  </picture>
</p>

**Remotes and hosting**

- Clone, fetch, pull (fast-forward, merge or rebase) and reviewed push, including
  force-with-lease and atomic multi-branch push.
- Pull requests, issues, reviews and CI marks for GitHub, GitHub Enterprise Server, GitLab,
  Bitbucket, Azure DevOps and Gitea ([what each supports](HOSTING.md)).
- An **Actions** page, and **Re-run failed jobs** on GitHub, GitLab and Gitea.
- An SSH key manager with per-repository keys; your existing keys and SSH config stay put.

**Many repositories**

- A Repository manager with folders, nested workspaces and tabs; **Scan folder** finds
  repositories and worktrees.
- Every saved repository's status at a glance, with **Fetch all** and **Pull all**
  (fast-forward only).
- **Explore** other repositories (search GitHub or paste an address) in a read-only preview
  before you clone them.
- Worktrees, submodules, tags, Git LFS, ignore rules and commit signing.

**AI, only when you ask**

- Draft a commit message, review a branch diff or get a proposed conflict resolution from
  Claude Code or Codex on your computer, or from a connected MCP agent.
- Rocket stores no AI API key and shows what will be sent first. Nothing is committed or
  applied without you.

**Extensions**

- Sandboxed Lua packages with native panels, forms and tables, and commands in menus and the
  palette. An extension can't run Git or shell commands, read your files or see your tokens.
- An offline Lua playground and the [extension API](INTEGRATIONS.md). Jira and PlayGround
  ship with Rocket, switched off.

<p align="center">
  <picture>
    <source media="(prefers-color-scheme: light)" srcset="docs/media/lua-light.webp">
    <img src="docs/media/lua-dark.webp" width="880" alt="The Lua playground: a short script on the left, and on the right the release checklist panel it draws, with a table, a progress bar and a button.">
  </picture>
</p>

**Everything else**

- Themes: Clay (the default), Ocean, Dark, Light, System, or Custom with your own accent
  colour and background picture.
- An integrated terminal, custom commands, configurable shortcuts, Statistics, activity
  history and local crash reports.
- Secret guards that stop recognized credentials and AI API keys from being committed or
  pushed.
- In-app updates from [GitHub Releases](https://github.com/JasXTD/Rocket/releases),
  installed only when you choose, with the previous version kept.
- A JSON [CLI](CLI.md) and an optional localhost [MCP service](tools/README.md#localhost-mcp)
  for automation and UI testing.

## Platforms

| Platform | Status |
| --- | --- |
| Windows x64 | Main platform, tested mostly on Windows 10. A portable ZIP with Git for Windows 2.56.0.windows.4, Git Credential Manager and Git LFS bundled; no installer or administrator access. |
| Linux x86_64 | Supported under X11 or Wayland. Uses your system Git. Everyday Git work, the terminal, SSH, hosting, Lua extensions and app updates work. |
| macOS 15+ on Apple Silicon | Supported, not yet notarized. Native menus, Keychain, system spelling, automatic refresh, a zsh terminal and Lua extensions. Uses your installed Git. |

See [Current limits](#current-limits) for what still differs between platforms.

## Build and run

The build scripts bind Rocket to its Lua worker by hash, so extensions work. Plain
`cargo build` also compiles, but turns Lua off.

**Windows** needs the Rust MSVC toolchain and the Microsoft C++ build tools:

```powershell
.\tools\Build-Local.ps1
.\target\release\rocket.exe
```

Add `-Offline` when the locked dependencies are cached, or `-Profile debug` for a debug
build. When moving a build, keep everything beside `rocket.exe` together: the CLI, the
terminal, rebase and Lua helpers, `conpty/` and the bundled `git/`.

**Linux** needs Rust 1.95+, a C compiler and Git 2.46+:

```bash
tools/build-local.sh
./target/release/rocket
```

`tools/package-release.sh` turns that build into a portable folder and its update archive.

<details>
<summary>Optional Linux system pieces</summary>

- Remembered tokens and passphrases: libsecret and an unlocked keyring such as GNOME Keyring.
- Hosting and extension web requests: the system's root-owned `/usr/bin/curl`.
- Spell checking: a Hunspell dictionary such as `hunspell-en-us`.
- Git LFS: version 3.8.1 or later beside Git.
- Dependency checks: OSV-Scanner 2.7.0.
- Lua extensions: Landlock (Linux 5.13+, switched on as in Ubuntu).

Settings and logs live in `$XDG_STATE_HOME/rocket` (`~/.local/state/rocket`), which must be a
private folder you own. Rocket refuses to run as root.

</details>

**macOS** needs Rust 1.99.1, the Xcode command-line tools, Node.js and Git:

```bash
bash tools/build-macos.sh
```

This makes `Rocket.app` and a ZIP under `target/macos-stage1`. For Lua extensions, build the
helper with `tools/macos/build-lua-helper.sh` first and pass it as `ROCKET_MACOS_LUA_APP`
(see [Mac Lua](docs/MacOS-lua-wasm.md)). The [macOS port notes](docs/MacOS-port.md) cover
the rest.

## Comparison

| | **Rocket** | [**Fork**](https://git-fork.com/) | [**GitKraken Desktop**](https://www.gitkraken.com/git-client) | [**Lazygit**](https://github.com/jesseduffield/lazygit) |
| :-- | :-: | :-: | :-: | :-: |
| **Platforms** | | | | |
| Windows | ✅ | ✅ | ✅ | ✅ |
| macOS | ✅ Apple Silicon | ✅ | ✅ | ✅ |
| Linux | ✅ | — | ✅ | ✅ |
| Interface | Desktop, native Rust | Desktop | Desktop | Terminal |
| **Everyday Git** | | | | |
| Stage hunks and single lines | ✅ | ✅ | ✅ | ✅ |
| Interactive rebase | ✅ visual | ✅ visual | ✅ visual | ✅ keyboard |
| Built-in conflict editor | ✅ three columns | ✅ | ✅ | ✅ hunk picker |
| Linked worktrees | ✅ | ✅ | ✅ | ✅ |
| Many repositories | ✅ workspaces, Fetch all, Pull all | ✅ workspaces | ✅ workspaces | Recent-repo switcher |
| **AI** | | | | |
| Commit messages | ✅ Claude Code, Codex | ✅ Claude Code, Codex, Cursor | 💲 GitKraken AI | — |
| Code review | ✅ | ✅ | In the browser | — |
| Conflict resolution | ✅ | — | 💲 preview | — |
| **Hosting** | | | | |
| Pull requests | ✅ list, create, review | ✅ list, create | ✅ list, create, merge | ✅ create, open |
| CI status | ✅ commits and branches, re-run failed jobs | — | ✅ on pull requests | — |
| **Extras** | | | | |
| Extensions | ✅ sandboxed Lua with native panels | Custom commands | — | Custom commands |
| Price | Free | $59.99 once, free evaluation | Free for local and public repos, paid plans | Free |
| License | [MIT](LICENSE-MIT) / [Apache-2.0](LICENSE-APACHE) | Commercial | Commercial | MIT |

✅ built in · 💲 paid plan · — not in the app's documentation

Checked on 2026-11-09 against Fork 2.24 (Windows) and 2.71 (macOS), GitKraken Desktop 12.6.1
and Lazygit 0.66.3, using their official documentation, release notes and pricing pages. This
compares documented features, not speed or maturity, and features change.

## Current limits

- **macOS:** not yet signed with a Developer ID or notarized, so it isn't ready to
  distribute and in-app updates won't install a release. Tested on GitHub's Apple Silicon
  runners, not yet on a physical Mac. Intel Macs aren't supported.
- **Linux:** Lua needs x86_64 with Landlock. Steps that must never overwrite a file need a
  local filesystem such as ext4, XFS or Btrfs (NFS, some FUSE mounts and WSL's `/mnt` are
  refused). Most MCP acceptance runners and the non-GitHub hosting tests still run on Windows.
- **Windows:** HTTPS remotes need Git for Windows 2.56.0.windows.4 or newer. An older Git
  you pick yourself is limited to local and SSH remotes
  ([why](SECURITY.md#windows-git-http-runtime-gate-2026-10-29)).
- **Hosting:** only GitHub's account sign-in has been checked against the real service.
  GitHub Enterprise Server, GitLab, Bitbucket, Azure DevOps and Gitea are tested against
  local fixtures. See [Hosting](HOSTING.md).
- **Stand-ins:** Jira was tested against mock services, and AI resolve against a stand-in
  program, not real Claude Code or Codex replies.
- **Partial actions** (single lines, hunks) cover specific text cases; other files need a
  whole-file action. The CLI and MCP service cover a subset of the desktop.

## Project structure

| Folder | What it holds |
| --- | --- |
| `crates/rocket-ui` | The desktop app (egui/eframe). |
| `crates/rocket-git` | Git execution, previews, write coordination and recovery. |
| `crates/rocket-integrations` | Extension packages and the Lua worker. |
| `crates/rocket-cli` | The command-line interface. |
| `crates/rocket-mcp`, `crates/rocket-mcp-control` | The localhost UI automation service. |
| `crates/rocket-terminal`, `crates/rocket-spell` | The terminal and commit-message spelling. |
| `runtime/` | Reviewed bundled runtimes and their provenance. |
| `tools/` | Build scripts, mockups and focused tests ([guide](tools/README.md)). |

## Documentation

- [Wiki](docs/wiki/Home.md): first commit, everyday work, remotes and recovery.
- [User guide](crates/rocket-ui/src/guide.md): every desktop workflow in detail.
- [CLI](CLI.md), [Hosting](HOSTING.md), [Extensions](docs/wiki/Integrations.md) and the
  [extension API](INTEGRATIONS.md).
- [Security](SECURITY.md), [Dependencies](DEPENDENCIES.md) and the [Changelog](CHANGELOG.md).

## License

Licensed under either [Apache-2.0](LICENSE-APACHE) or [MIT](LICENSE-MIT), at your option.
See [third-party notices](THIRD-PARTY-NOTICES.txt) for bundled components.
