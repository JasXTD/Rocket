# Advanced tools

[Wiki home](Home.md) · [Glossary](Glossary.md) · [Full reference](../../crates/rocket-ui/src/guide.md)

Use **Ctrl+K** to find a command by name. Disabled results explain what is missing,
such as an open repository or a suitable branch. Press **F1** for the detailed
rules while you are in a dialog.

## Choose a tool by the problem

| What you need | Tool and reference | What to keep in mind |
| --- | --- | --- |
| See what changed between branches or commits | [Compare](../../crates/rocket-ui/src/guide.md#compare-local-contents) | Read-only snapshots; rerun after local edits |
| Follow a file or a particular line through history | [File history and blame](../../crates/rocket-ui/src/guide.md#file-history) | Rename tracking is supported; loaded history has limits |
| Clean up a local series of commits | [Interactive rebase](../../crates/rocket-ui/src/guide.md#interactive-rebase) | Rewrites commit IDs; review drops, squash and fixup carefully |
| Replay or undo an existing commit | [Cherry-pick and revert](../../crates/rocket-ui/src/guide.md#commits) | A conflict can pause the sequence for recovery |
| Find the commit that introduced a bug | [Bisect](../../crates/rocket-ui/src/guide.md#bisect) | Manual or automated; test commands execute locally |
| Find a previous branch tip or lost commit | [Reflog and recovery](../../crates/rocket-ui/src/guide.md#reflog-and-recovery) | Recover onto a new branch before changing current history |
| Keep two branches checked out at once | [Worktrees](../../crates/rocket-ui/src/guide.md#worktrees) | Separate working folders share repository history |
| Save unfinished work or apply only part of it | [Stashes](../../crates/rocket-ui/src/guide.md#stashes) | Apply keeps the stash; dropping it is a separate decision |
| Mark a release or manage remote tags | [Tags](../../crates/rocket-ui/src/guide.md#tags) | Local and remote deletion are separate operations |
| Handle repositories nested inside a parent | [Submodules](../../crates/rocket-ui/src/guide.md#submodules) | The parent stores a child commit, not the child's uncommitted edits |
| Work with large-file pointers and storage | [Git LFS](../../crates/rocket-ui/src/guide.md#git-lfs) | Requires Git LFS; tracking rules alone do not rewrite old history |
| Exchange changes as files | [Patches and commit series](../../crates/rocket-ui/src/guide.md#patches) | Inspect the target and apply preview; series may need Continue/Skip/Abort |
| Exclude generated files from future additions | [Ignore rules](../../crates/rocket-ui/src/guide.md#ignore-rules) | Ignoring a path does not remove already committed history |
| Discard edits or restore an older file | [Restoring files](../../crates/rocket-ui/src/guide.md#restoring-files) | Replaces working content; review backup and external-edit checks |
| Prove who signed a commit or tag | [Signing](../../crates/rocket-ui/src/guide.md#signing) | A valid signature and trust in the signer are different questions |
| Push several branches as one update | [Atomic push](../../crates/rocket-ui/src/guide.md#push-branches-together) | Requires server support; no fallback to separate pushes |
| Replace a remote branch after rewriting it | [Force push with lease](../../crates/rocket-ui/src/guide.md#force-push-with-lease) | Replaces history only at the reviewed remote tip |
| Use your preferred editor or merge tool | [External tools](../../crates/rocket-ui/src/guide.md#external-tools) | Editor saves affect real files; merge results are reviewed before import |
| Run a repeatable project action | [Custom commands](../../crates/rocket-ui/src/guide.md#custom-commands) | Review the command and substitutions; it runs with your permissions |

## Preferences you may want early

- **Git:** select an installed executable and set global commit identity defaults.
  Use repository-local settings when one project needs a different identity.
- **Shortcuts:** find a command by name or by pressing its keys, click its
  Shortcut or Alternate cell, then press a key chord or mouse gesture; it is
  saved at once (Esc cancels, Delete clears the cell). Side buttons work alone;
  other clicks and wheel directions need Ctrl, Alt or Shift. If another command
  uses the input, choose Use for this command (the other one loses it) or Keep.
  The wiki uses defaults; your customized binding can differ.
- **External tools:** save editor, diff and merge profiles. These have their own
  save controls; use a real executable path and the documented arguments.
- **Security:** leave secret protection enabled. A refusal should lead to removing
  or rotating the credential, not disguising a real secret as a test value.

## CLI and local automation

The desktop app and **rocket-cli.exe** call the same backend, which invokes the
selected bundled or installed Git. The CLI is a scripting interface, not a second repository engine.
See the [CLI reference](../../CLI.md) for supported commands and JSON output;
not every desktop workflow is exposed there.

**Edit → Preferences → MCP server** configures the optional local UI control
service. It controls marked test repositories in the configured test folder and
needs its access token. It is not necessary for everyday Git work. Follow the
[localhost MCP guide](../../tools/README.md#localhost-mcp) when writing automated
tests, and keep the token private.

## Optional AI assistance

AI commit-message drafts and branch reviews are opt-in workflows involving a
separately configured provider/tool. Review what will be sent: repository content
can leave your computer. Drafting a message does not commit it for you. See
[AI drafts](../../crates/rocket-ui/src/guide.md#ai-drafts) and the relevant dialog
before using these features on private source.
