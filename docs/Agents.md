# Local agent sessions

Open a repository, then choose **Agents** in
the sidebar or **View → Go to → Agents**. Install and sign in to your preferred
CLI separately; Rocket does not install it or store an AI API key.

1. Choose **New session**, enter a task name, base revision, unused branch name and
   a new worktree folder outside existing repositories.
2. Choose Claude Code, Codex, GitHub Copilot or Custom command. Check the absolute executable
   path. Arguments are literal, one per line; do not add shell quoting. A shell
   runs only if you deliberately select one as the executable. When a preset's
   program is not found, the field for its path opens: type the path or choose
   **Browse**. A program chosen with Browse is where Rocket finds that preset
   from then on. Claude Code's and Codex's are the Program setting under
   **Preferences → Commit messages → AI drafts**; GitHub Copilot's is saved with
   Rocket's settings. A typed path applies to that session only.
3. Optionally select a setup executable and arguments. Rocket never discovers or
   runs a repository's setup scripts automatically. A failing setup prevents the
   agent from starting and keeps the worktree for inspection.
4. Review the exact base commit, folder, program SHA-256 and arguments, then start.
   Rocket saves the session record before creating the worktree. It rechecks the
   executable and Git state before applying the reviewed operation. A changed
   executable, including an update performed by setup, requires a fresh review.
5. Enter your task in the agent's terminal. Existing provider permission prompts
   stay in that terminal. Process state and conversation activity are separate.
   Neither a finished turn nor exit code zero proves the work is correct.

Each managed session has its own worktree and terminal. The source checkout's
staged, unstaged and untracked files stay in place; Rocket does not copy them,
ignored files, credentials or environment files into the new worktree. External
programs inherit the user's environment and may use existing credentials. They
run with the user's access: worktrees are not a security sandbox. Rocket does
not add options that bypass agent permissions, nor auto-commit, merge or push.

The **GitHub Copilot** preset is pending release. It finds the native `copilot`
executable on your absolute PATH entries, in `~/.local/bin`, or Windows' WinGet
links folder. Installed links resolve to their native target for review. If it
is not found, select the native executable yourself; a Windows `.cmd`/`.bat`
wrapper is not accepted as an executable. The preset adds `--no-auto-update`
so the reviewed version runs instead of updating itself or selecting a newer
downloaded version. Update Copilot separately with its sessions stopped, then
review the executable again. Sign in using Copilot's own flow; Rocket neither
copies nor replaces your login.

Copilot CLI can also connect to a trusted VS Code workspace opened at the same
worktree, automatically or through `/ide`. That integration shares editor context
and supports VS Code diff review; Rocket hosts the CLI terminal, not VS Code's
Copilot interface. See GitHub's [VS Code connection guide](https://docs.github.com/en/copilot/how-tos/copilot-cli/use-copilot-cli/connecting-vs-code).

On Linux and macOS, Rocket copies each selected executable into a private temporary
directory and hashes those copied bytes for review. It launches that copy, so
replacing or overwriting the original after the last check cannot substitute a
different executable. The copy remains available until both the review and its
terminal worker release it. Windows instead holds a file handle that denies
writes and deletion through the same lifetime. Cancelled preparation removes
partial copies; normal completion, Stop and closing the review release owned copies.
A forced application/OS crash can leave a private temporary copy behind.

Before copying and again before launch, Rocket checks the opened source's execution
policy. Linux uses the kernel's effective-permission check, including ACLs and
`noexec` mounts (Linux 5.8 or newer); an unavailable check refuses launch. macOS
refuses files or enclosing folders/bundles carrying `com.apple.quarantine`, including
empty or previously approved-looking values. This includes code bundles without
an `.app` suffix; a clean leaf executable cannot discard its enclosing bundle's
protection through Rocket's temporary copy. Choose a trusted installed executable without quarantine
metadata. Rocket does not clear quarantine, grant a Gatekeeper exception or treat
a copied signature as proof of approval. This conservative restriction also applies
to quarantined executables macOS may otherwise have approved. Permissions and
metadata on the original file remain unchanged.

Arguments and the worktree working directory are unchanged. Native Unix programs
retain the original `argv[0]`, but programs locating resources through their actual
executable location, and scripts using their own filename, see the private copy.
Choose a standalone native executable when possible. For an interpreter, specify
the script and resource paths explicitly; the executable review does **not** bind
scripts, shared libraries, configuration or other files it loads. Rocket never
falls back to launching an unverified original if preparation fails.

**Stop** immediately shows Stopping and requests termination of the owned process
tree. Rocket retains the worktree and any changes. Select **Open worktree** to
open the worktree's ordinary Changes view; a running agent can still change those
files. **Review and restart** verifies the worktree's registered identity, branch, commit and selected
executable again. Setup is off by default on restart and can be selected explicitly.

Closing Rocket stops its owned sessions. Reopening restores metadata only; it
does not launch any executable. Session definitions live with Rocket's protected
settings. Arguments are saved, so never place passwords or tokens in them.
Terminal input/output is not persisted as session metadata or diagnostic events.

**Remove worktree** previews and rechecks removal. Staged, unstaged, untracked and
ignored files, locks or active Git operations block it. The branch and recovery
reference remain. **Forget session** only removes Rocket's record;
use Worktrees to manage the retained checkout. Failed or cancelled creation can
leave a registered worktree or branch: inspect it before trying another creation.

### Conversation activity and resume (pending release)

**Track conversation activity** adds session-scoped lifecycle hooks for the
Claude Code, Codex and Copilot presets. It does not edit provider configuration
files or change logins. Codex may ask you to review these hooks in `/hooks`;
Rocket never bypasses that trust review. Old saved sessions retain their previous
behavior until you enable tracking in **Review and restart**.

The activity badge distinguishes Ready, Working, Needs your attention, Turn
finished and Turn interrupted. Permission requests and question tools raise an
attention badge and request the operating system's taskbar/Dock attention signal.
The sidebar count stays, and the session is listed under **Needs you**, until you acknowledge
the event or the provider starts working again. Repeated identical events do not
notify again; unseen badges survive reopening Rocket. OS notification behavior
still depends on focus and system settings.

These are provider reports, not a claim that a project task or its tests passed.
A Stop hook can run before another provider hook continues the turn. New work
updates the badge again. Unsupported/disabled hooks display **Activity not
connected**; Rocket does not guess from terminal text, silence or a zero exit.
Copilot CLI 1.0.109 creates its conversation lazily: that state can remain until
the first prompt, even though its terminal is already usable.
The session input honors the CLI's requested bracketed-paste mode and then sends
Enter separately in the same input block; it does not retry or duplicate a prompt.
Custom commands retain process status unless they explicitly report lifecycle
events through Rocket's child-scoped callback protocol.

After a provider reports a conversation ID, **Resume conversation** reopens that
exact conversation in one click. Rocket first verifies the registered worktree,
branch, executable bytes and reviewed arguments. Changed launch settings require
a fresh review. Resume never runs setup, chooses the most recent conversation,
or falls back to a new conversation. A provider may still refuse a deleted or
unavailable conversation; its error stays in the terminal. **Review and restart**
in Details deliberately starts a fresh conversation instead. One-click resume is
available for interactive presets with recognized launch options (including model
selection). A saved initial prompt, exec/print mode or an unrecognized option
requires reviewed restart; Rocket will not silently replay that original prompt.

Only the conversation ID, activity badge, acknowledgement and review fingerprint
are saved. Hook payloads are reduced before transmission to those lifecycle
fields: prompts, tool arguments, transcripts, replies and credentials are neither
stored nor logged by Rocket. Each run has a bounded capability-protected loopback
receiver; stopping or exiting closes it. Provider processes retain their normal
access and own history. These events never grant permission or trigger a merge.

### Integrate (pending release)

1. In **Integrate**, choose **Open worktree** to see its Changes view. Review and commit
   the changes you want to bring back; Rocket does not commit them automatically.
2. Stop the agent and choose **Check integration**. Both the agent worktree and
   original checkout must be clean, and the original checkout must have a named
   branch. The panel names that destination branch and shows incoming commits.
3. **Compare committed changes** opens the pinned comparison. Return to Agents
   and check again after changes or a new session selection.
4. **Review merge** opens Rocket's existing merge preview. Applying it is a
   separate explicit action. Both branch tips and checkout state are rechecked;
   conflicts use the conflict editor and normal Continue/Abort recovery. Stop
   cancels a check; once a Git write starts, inspect its reported result.

The agent branch and worktree remain after integration. Nothing is pushed,
deleted, stashed, or discarded automatically. A dirty checkout shows a refusal
with its files preserved. There is no remote agent service or cost dashboard.

Provider contracts: [Codex hooks](https://learn.chatgpt.com/docs/hooks),
[Claude hooks](https://code.claude.com/docs/en/hooks),
[Claude CLI](https://code.claude.com/docs/en/cli-reference), and
[Copilot hooks](https://docs.github.com/en/copilot/reference/hooks-reference).
Native synthetic tests exercise Rocket's event transport, PTY lifecycle and Git
safeguards; they are not live acceptance of every installed provider version.

Worktree creation supports Windows, Linux and macOS. On macOS it uses exclusive
rename, refusing filesystems that cannot guarantee creation without replacement.
The next native Mac acceptance run is prepared in
`tools/macos/test-worktree-agents.sh`; cross-compilation alone does not establish
Apple Silicon runtime behavior. See VALIDATION.md for the performed checks.
