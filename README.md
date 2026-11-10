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
  <a href="#command-line-interface">CLI</a> ·
  <a href="#build-and-run">Build and run</a> ·
  <a href="#comparison">Comparison</a> ·
  <a href="#contributing">Contributing</a> ·
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

> [!NOTE]
> [Download Rocket 0.92.2](https://github.com/JasXTD/Rocket/releases/tag/v0.92.2):
> Windows x64, Linux x64 (glibc 2.43+) and the Apple Silicon macOS 15+ preview.
> The Mac preview is not Developer-ID-signed or notarized.
> [Build from source](#build-and-run) is also available.

**New in 0.92.2:** choose a preview language in Preferences; your saved theme
appears from the first frame. Hosting adds inline comments for GitLab, Gitea,
Bitbucket and Azure DevOps, including supported private-server variants. The CLI
now includes hosting commands, reviewed Commit & Push and Save, switch & restore.
See the [release notes](docs/releases/0.92.2.md) for scope and limitations.

## Features

**Changes and review**

- Stage files, hunks or single lines; drag between Staged and Unstaged.
- Unified or side-by-side diffs with syntax and word highlights, search and an overview strip.
- Image compare with overlay, swipe and pixel differences.
- **Compare** branches, tags, commits, the staging area or the working tree.
- **Commit & Push**, commit-message spell check, and a **Summary** page with the next step.
- Restore or discard files and selected lines, with recovery copies.
- Commit details, **File history** and blame in their own windows.
- External editor, diff and merge tool profiles.

**History and branches**

- Commit graph across branches, file renames, and **Files** at any commit.
- Find code with `git log -S` / `-G` and **Search in files** (`git grep`).
- Create, switch, rename and clean up branches; **Save, switch & restore** keeps uncommitted work.
- Merge, rebase and interactive rebase (reorder, reword, edit, squash, fixup, drop), previewed first.
- Cherry-pick, revert, amend and **conflict prediction**.
- Reflog recovery, **Undo Last Commit**, and manual or automatic **bisect**.
- **Git Flow** feature, release and hotfix branches.
- Tags, commit signing and signature checks, and **issue links** to Jira, Redmine or any tracker.
- [**Repository map**](docs/wiki/Repository-map.md) and **Statistics**.

<p align="center">
  <picture>
    <source media="(prefers-color-scheme: light)" srcset="docs/media/history-light.webp">
    <img src="docs/media/history-dark.webp" width="760" alt="The Commits page with a branch graph across five branches, merges, and branch and remote labels.">
  </picture>
</p>

**Conflicts and recovery**

- A three-column conflict editor (Ours, Result, Theirs) with linked scrolling.
- Stashes with partial apply; import and export patches and commit series.
- Continue, Skip or Abort for paused operations, and recovery data before anything is replaced.

<p align="center">
  <picture>
    <source media="(prefers-color-scheme: light)" srcset="docs/media/conflict-light.webp">
    <img src="docs/media/conflict-dark.webp" width="880" alt="The three-column conflict editor with Ours, Result and Theirs, and two conflicts numbered and linked across the columns.">
  </picture>
</p>

**Remotes and hosting**

- Clone (HTTPS, SSH or local), manage remotes, and fetch in the background.
- Pull (fast-forward, merge or rebase) and reviewed push, force-with-lease and atomic
  multi-branch push.
- Remote tags, remote branch deletion and prune.
- **Publish to GitHub** from a local repository.
- Pull requests, issues and reviews on GitHub, GitHub Enterprise Server, GitLab, Bitbucket,
  Azure DevOps and Gitea ([what each supports](HOSTING.md)).
- Inline review drafts with revision checks before submission; GitLab server-side
  search for merge requests and issues. Non-GitHub adapters are tested with mock APIs.
- CI marks on commits and branches, an **Actions** page and **Re-run failed jobs**.
- **SSH keys**: find, create, unlock and assign per repository, plus Direct key mode without
  ssh-agent.

**Many repositories**

- Tabs, automatic refresh, and a **Repository manager** with folders and nested workspaces.
- **Scan folder**, every repository's status at a glance, **Fetch all** and **Pull all**.
- **Explore**: search GitHub or paste an address, preview read-only, then Keep or Discard.
- Worktrees (move, repair, prune), submodules, Git LFS (objects, tracking rules, locks) and
  ignore rules.

**AI, only when you ask**

- **Agents** (pending release): run parallel Claude Code, Codex or custom executable
  sessions in separate worktrees, with reviewed setup, interactive terminals,
  Stop/restart and safe cleanup. Uses your installed tools and existing logins;
  worktrees separate files, not permissions. [Session guide](docs/Agents.md).
- **AI drafts** for commit messages, **AI review** of a branch diff and **AI resolve** for
  conflicts.
- Runs Claude Code or Codex on your computer, or a connected MCP agent. No AI API key is
  stored, and you see what's sent first.

**Extensions**

- Sandboxed Lua packages with native panels, forms, tables, and commands in menus and the
  palette.
- Reviewed install; an extension can't run Git or shell commands, read your files or see your
  tokens.
- An offline **Lua playground**, developer tools and the [extension API](INTEGRATIONS.md).
- **Jira** and **PlayGround** ship with Rocket, switched off.

<p align="center">
  <picture>
    <source media="(prefers-color-scheme: light)" srcset="docs/media/lua-light.webp">
    <img src="docs/media/lua-dark.webp" width="880" alt="The Lua playground: a short script on the left, and on the right the release checklist panel it draws, with a table, a progress bar and a button.">
  </picture>
</p>

**Safety**

- **Secret protection** and **Guard AI API keys** block recognized secrets in commits and
  pushes.
- **Dependency advisories**: offline OSV scans, with an optional check before push.
- Reviews before writes, Stop on running operations, and recovery data for destructive steps.

**Everything else**

- Themes: Clay (the default), Ocean, Dark, Light, System and Custom.
- **Multilingual interface**: English and ten preview translations, with automatic
  system-language detection and a saved language choice in Preferences.
- **Command palette**, task-grouped menus, custom commands, and shortcuts and mouse gestures
  with two bindings each.
- An integrated terminal with your choice of shell.
- **Setup Guide** and a built-in guide (F1).
- Activity history, a diagnostics viewer, **Repository benchmark**, local crash reports and
  storage locations.
- Choice of Git program (bundled, installed or custom), and commit identity globally or per
  repository.
- In-app updates from [GitHub Releases](https://github.com/JasXTD/Rocket/releases), with the
  previous version kept.
- A JSON [CLI](CLI.md) for Git, hosting and reviewed composed workflows, plus a
  localhost [MCP service](tools/README.md#localhost-mcp) for UI automation.

### Languages

Rocket supports **English, German, Swiss German, French, Swedish, Russian,
Ukrainian, Arabic, Simplified Chinese, Japanese and Korean**. At first launch it
detects a supported system language; change it in **Preferences → General →
Languages** at any time. Your choice is saved for the next launch.

The ten non-English translations are labeled **Preview**. Missing translations
fall back to English. Repository names, paths, commit messages and other user
content keep their original text; changing the interface language does not
translate or modify your repository.

## Platforms

| Platform | Status |
| --- | --- |
| Windows x64 | Main platform, tested mostly on Windows 10. A portable ZIP with Git for Windows 2.56.0.windows.4, Git Credential Manager and Git LFS bundled; no installer or administrator access. |
| Linux x86_64 | Supported under X11 or Wayland; the portable package requires glibc 2.43+. Uses your system Git. Everyday Git work, the terminal, SSH, hosting, Lua extensions and app updates work. |
| macOS 15+ on Apple Silicon | Preview, not yet notarized. Native menus, Keychain, system spelling, automatic refresh, a zsh terminal and Lua extensions. Uses your installed Git. |

See [Current limits](#current-limits) for what still differs between platforms.

## Command-line interface

The portable packages include **`rocket-cli`** (`rocket-cli.exe` on Windows). It runs
without the desktop app and shares Rocket's Rust backend, Git helper, protection checks
and repository write coordination. Use `--json` for structured results, errors and
recovery details in scripts.

Commands cover repository inspection, staging and commits, branches, fetch/pull/push,
hosting PRs and reviews, and selected recovery workflows. The CLI covers a subset of
the desktop; see the [command reference and examples](CLI.md) for supported operations,
preview requirements, exit codes and cancellation.

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
| Windows | ✅ x64 | ✅ | ✅ | ✅ |
| macOS | Preview, Apple Silicon | ✅ | ✅ | ✅ |
| Linux | ✅ x86_64 | — | ✅ | ✅ |
| Interface | Native Rust desktop + CLI | Desktop | Desktop + companion CLI | Terminal UI |
| **Everyday Git** | | | | |
| Stage hunks and single lines | ✅ | ✅ | ✅ | ✅ |
| Interactive rebase | ✅ visual | ✅ visual | ✅ visual | ✅ keyboard |
| Built-in conflict editor | ✅ three columns | ✅ | ✅ | ✅ hunk picker |
| Linked worktrees | ✅ | ✅ | ✅ | ✅ |
| Many repositories | ✅ folder scan, workspaces, Fetch all, Pull all | ✅ workspaces | ✅ workspaces | Recent-repo switcher |
| **AI** | | | | |
| Commit messages | ✅ Claude Code, Codex | ✅ external AI agents | 💲 GitKraken AI | — |
| Code review | ✅ branch reviews | ✅ external AI agents | GitKraken.dev (browser) | — |
| Conflict resolution | ✅ reviewed suggestions | — | 💲 preview | — |
| **Hosting** | | | | |
| Pull requests | ✅ list, create, inline review, merge | ✅ list, create | ✅ list/create; GitHub review/merge | GitHub PR indicators; create/open in browser |
| CI status | ✅ commits, branches and PR heads; rerun where supported | — | ✅ on pull requests | — |
| **Extras** | | | | |
| Command-line interface | ✅ [`rocket-cli`](CLI.md), JSON output | [Open desktop from CLI](https://git-fork.com/blog/posts/fork-1.0.67/) | ✅ companion [`gk`, JSON output](https://help.gitkraken.com/cli/gk-cli-json-output/) | Interactive terminal UI |
| Custom commands | ✅ contextual commands, input forms, JSON import/export | ✅ contextual commands and input forms | Agent launch/setup commands | ✅ YAML configuration and prompts |
| Sandboxed UI extensions | ✅ Lua with native panels | — | — | — |
| Price | Free | $59.99 once, free evaluation | Free for local and public repos, paid plans | Free |
| License | [MIT](LICENSE-MIT) / [Apache-2.0](LICENSE-APACHE) | Commercial | Commercial | MIT |

✅ supported as described · 💲 paid plan · — not documented in the sources checked

Updated for **Rocket 0.92.2 on 2026-11-10**. This compares documented capabilities,
not speed, maturity or identical coverage. Features vary by platform, host and plan;
AI providers may require a separate subscription. See [Platforms](#platforms) for
Rocket's package requirements and preview status.

Rocket's [hosting matrix](HOSTING.md) describes merge methods and review limits by
provider. PR checks use the PR's head commit; Bitbucket Server/Data Center CI is not
supported. Non-GitHub adapters have mock-API coverage, not live-account acceptance.
[Custom commands](crates/rocket-ui/src/guide.md#custom-commands) are separate from Lua
extensions. Lazygit's GitHub PR indicators require the separate `gh` tool and sign-in.

Official comparison sources:

- **Fork:** [features and price](https://git-fork.com/),
  [Windows release notes](https://git-fork.com/releasenoteswin) and
  [macOS release notes](https://git-fork.com/releasenotes).
- **GitKraken:** [PRs and CI status](https://help.gitkraken.com/gitkraken-desktop/pull-requests/),
  [AI](https://help.gitkraken.com/gitkraken-desktop/gkd-gitkraken-ai/),
  [custom agent commands](https://help.gitkraken.com/gitkraken-desktop/preferences/) and
  [pricing](https://www.gitkraken.com/pricing).
- **Lazygit:** [features](https://github.com/jesseduffield/lazygit) and
  [custom commands](https://github.com/jesseduffield/lazygit/blob/master/docs/Custom_Command_Keybindings.md).

## Current limits

- **macOS:** the downloadable preview is ad-hoc signed, without a Developer ID or
  notarization; in-app updates refuse to install it. Tested on GitHub's Apple Silicon
  runners, not yet on a physical Mac. Intel Macs aren't supported.
- **Linux:** Lua needs x86_64 with Landlock. Steps that must never overwrite a file need a
  local filesystem such as ext4, XFS or Btrfs (NFS, some FUSE mounts and WSL's `/mnt` are
  refused). Most MCP acceptance runners and the non-GitHub hosting tests still run on Windows.
- **Windows:** HTTPS remotes need Git for Windows 2.56.0.windows.4 or newer. An older Git
  you pick yourself is limited to local and SSH remotes
  ([why](SECURITY.md#windows-git-http-runtime-gate-2026-10-29)).
- **Hosting:** only GitHub's account sign-in has been checked against the real service.
  GitHub Enterprise Server, GitLab, Bitbucket, Azure DevOps and Gitea are tested against
  local fixtures. Inline capabilities and limits differ by host; see [Hosting](HOSTING.md).
- **Languages:** all ten translations remain previews while advanced-screen coverage
  and wording are reviewed. Untranslated interface text falls back to English;
  Git/provider diagnostics, extension-authored content and long-form guides can
  retain their original language.
- **Integration acceptance:** Jira is checked against its API documentation and mock
  services, without a live tenant. AI conflict proposals have also been checked with
  a real Astra response through localhost MCP using synthetic code; the separate
  Claude Code/Codex CLI provider paths still use stand-ins in acceptance tests.
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
| `tools/` | Build scripts and focused tests ([guide](tools/README.md)). |

## Documentation

- [Wiki](docs/wiki/Home.md): first commit, everyday work, remotes and recovery.
- [User guide](crates/rocket-ui/src/guide.md): every desktop workflow in detail.
- [CLI](CLI.md), [Hosting](HOSTING.md), [Extensions](docs/wiki/Integrations.md) and the
  [extension API](INTEGRATIONS.md).
- [Security](SECURITY.md), [Dependencies](DEPENDENCIES.md) and the [Changelog](CHANGELOG.md).

## Contributing

Bug reports, ideas and pull requests are welcome. Read [CONTRIBUTING.md](CONTRIBUTING.md)
first, and report security problems privately as [SECURITY.md](SECURITY.md) describes.

## Development credits

Created and maintained by [JasXTD](https://github.com/JasXTD), with development
assistance from OpenAI Codex and Anthropic Claude Code.

## License

Licensed under either [Apache-2.0](LICENSE-APACHE) or [MIT](LICENSE-MIT), at your option.
See [third-party notices](THIRD-PARTY-NOTICES.txt) for bundled components.
