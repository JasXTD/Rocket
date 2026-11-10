# Local agent sessions

The Agents view is pending release. Open a repository, then choose **Agents** in
the sidebar or **View → Go to → Agents**. Install and sign in to your preferred
CLI separately; Rocket does not install it or store an AI API key.

1. Choose **New session**, enter a task name, base revision, unused branch name and
   a new worktree folder outside existing repositories.
2. Choose Claude Code, Codex or Custom command. Check the absolute executable
   path. Arguments are literal, one per line; do not add shell quoting. A shell
   runs only if you deliberately select one as the executable.
3. Optionally select a setup executable and arguments. Rocket never discovers or
   runs a repository's setup scripts automatically. A failing setup prevents the
   agent from starting and keeps the worktree for inspection.
4. Review the exact base commit, folder, program SHA-256 and arguments, then start.
   Rocket saves the session record before creating the worktree. It rechecks the
   executable and Git state before applying the reviewed operation.
5. Enter your task in the agent's terminal. Existing provider permission prompts
   stay in that terminal. Session status reports process lifecycle, not guessed
   intent or task completion. An exit code of zero does not prove the work is correct.

Each managed session has its own worktree and terminal. The source checkout's
staged, unstaged and untracked files stay in place; Rocket does not copy them,
ignored files, credentials or environment files into the new worktree. External
programs inherit the user's environment and may use existing credentials. They
run with the user's access: worktrees are not a security sandbox. Rocket does
not add options that bypass agent permissions, nor auto-commit, merge or push.

**Stop** immediately shows Stopping and requests termination of the owned process
tree. Rocket retains the worktree and any changes. Select **Review changes** to
open the worktree's ordinary Changes view; a running agent can still change those
files. **Review and restart** verifies the worktree's registered identity, branch, commit and selected
executable again. Setup is off by default on restart and can be selected explicitly.

Closing Rocket stops its owned sessions. Reopening restores metadata only; it
does not launch any executable. Session definitions live with Rocket's protected
settings. Arguments are saved, so never place passwords or tokens in them.
Terminal input/output is not persisted as session metadata or diagnostic events.

**Remove worktree** previews and rechecks removal. Staged, unstaged, untracked and
ignored files, locks or active Git operations block it. The branch and recovery
reference remain. **Forget session, keep files** only removes Rocket's record;
use Worktrees to manage the retained checkout. Failed or cancelled creation can
leave a registered worktree or branch: inspect it before trying another creation.

The first version has no provider-specific waiting-for-input detection, task
completion hooks, automatic integration, remote agent service or cost dashboard.
Worktree creation supports Windows, Linux and macOS. On macOS it uses exclusive
rename, refusing filesystems that cannot guarantee creation without replacement.
The next native Mac acceptance run is prepared in
`tools/macos/test-worktree-agents.sh`; cross-compilation alone does not establish
Apple Silicon runtime behavior. See VALIDATION.md for the performed checks.
