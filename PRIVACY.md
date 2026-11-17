# Privacy policy

Last updated: 18 November 2026

Rocket is an open-source desktop Git client. Its Microsoft Store package is named
**GitRocket**, with **TwinCatz** as the publisher. This policy covers the desktop
app, command-line and local MCP tools, and the Rocket website. It describes the
data used by these features; services and external programs you connect have
their own privacy practices.

Rocket works with repositories on your computer. There is no Rocket account,
advertising service, usage-tracking service or automatic crash-report upload.
Network features still communicate with the services described below.

## Repositories and local data

Rocket reads repository files, paths, diffs, branches, remotes and Git history to
show and manage your work. History can include author and committer names, email
addresses, commit messages and timestamps. Code, History and People maps and
repository activity metrics use this data; PR metrics also use the selected
hosting service. These views do not send repository data to a Rocket analytics
service.

Rocket saves preferences, repository locations and groups, drafts, recent commit
messages, tool configurations, account profiles and agent-session metadata
locally. Some workflows also keep cached extension forms/results, repository
previews, temporary file copies, recovery backups, update downloads or exported
reports. These files can contain private code, paths or text you entered.

The normal application-data locations are:

- Windows: `%LOCALAPPDATA%\Rocket`.
- Linux: `$XDG_STATE_HOME/rocket`, or `~/.local/state/rocket` when no valid absolute
  XDG state directory is set.
- macOS: `~/Library/Application Support/Rocket`.

Custom profiles can use a different folder. Repositories, worktrees, Git's own
configuration and recovery data, SSH files and user-selected exports can live
outside the application-data folder. Rocket does not automatically encrypt all
of these files.

## Git and hosting connections

Clone, fetch, pull, push, Git LFS and other remote operations communicate with the
servers configured for those repositories. A push can transmit file contents,
history, commit identities and messages. Creating or publishing a repository can
make that information available to other people according to the destination's
visibility and access settings.

Hosting features contact the provider you select, such as GitHub, GitLab,
Bitbucket, Azure DevOps, Gitea or a configured private server. Requests can include
account credentials, repository identifiers, search terms and the PR, review,
issue or CI information needed for the action. Submitted descriptions, comments
and reviews go to that service. Enabled views can refresh hosting information;
optional automatic fetch contacts Git remotes at the interval you choose and is
off by default. Local file watching alone does not upload changes.

The receiving service can see your IP address and request metadata and applies
its own account, access, retention and privacy rules. Rocket's maintainers do not
receive a separate copy through a Rocket collection service.

## Credentials and SSH keys

Hosting tokens stay in memory unless you choose to remember a supported account.
Saved hosting and extension credentials use current-user protection: Windows
DPAPI, or encrypted files with a key held by Linux Secret Service or macOS
Keychain. Ordinary preferences, repositories and diagnostic files do not receive
that same credential encryption. This protection does not isolate your data from
all programs running as you or from an administrator.

Git authentication can use your configured credential helper, system SSH agent
or a selected SSH key. The SSH manager can discover local keys and perform the
key and configuration actions you choose. Rocket does not upload your private
SSH key to its maintainers; SSH uses it locally for authentication. A public key
you copy or register is shared with the destination you choose.

Disconnecting or forgetting a Rocket account does not revoke its token, remove
the provider's OAuth grant, or delete credentials stored separately by Git or
SSH. Use the provider's settings and your credential/SSH tools for those actions.

## AI, external tools, extensions and MCP

AI commit drafts, branch reviews and conflict proposals can send selected diffs,
file paths, messages or conflict context to your chosen CLI or connected agent.
That program may transmit the information to its AI provider using your existing
login. Review the shared content and the provider's privacy and retention settings.
Rocket does not require an AI API key stored in Rocket for these workflows.

Managed Agents, terminals, custom commands and external editors/diff/merge tools
run with your normal user permissions. They can access files, use existing
credentials and make their own network requests. A separate worktree is not a
sandbox. Those programs may keep their own command history, conversations, logs
and telemetry. Rocket's saved agent metadata includes task/session information,
arguments, conversation IDs and activity state, not the terminal transcript.
Do not put passwords or tokens in saved command arguments or task text.

Enabled Lua extensions can send form values and requests to their configured
service when network permission is granted. If you also grant repository-metadata
sharing, the extension receives branch and commit IDs and change counts. Cached
forms, results and drafts can remain locally. Rocket's host attaches saved
extension credentials to permitted requests instead of including them in the
script context.
The Lua playground's offline host does not make HTTP requests.

When enabled, Rocket's HTTP MCP server listens on loopback and requires a local
access token. Authorized clients can read the data and perform the actions
exposed by their configured scope. A connected AI client may send information it
reads to an external provider. Localhost transport does not make that client's
use of the data local-only. Treat exported snapshots and access tokens as private.

## Updates and security checks

Standalone desktop builds check GitHub Releases once at launch and when you
request a check. Requests identify Rocket's version and expose ordinary connection
metadata, including your IP address, to GitHub. Downloading an update contacts
GitHub's release-asset hosts. Installing and restarting requires your choice.
MSIX installations use Windows-managed updates instead of Rocket's GitHub
self-updater; Microsoft handles Store update requests under its own policies.

Secret protection and API-key detection scan locally. Optional dependency checks
run the configured scanner offline against local advisory databases. An explicit
advisory-data update downloads public ecosystem databases using a synthetic
inventory, without sending your repository paths or dependency names/versions.
Opening advisory links sends a normal browser request to the linked service.

## Diagnostics, crash reports and support

Rocket records local operation names, timings, outcomes and fixed error codes for
activity history and diagnostics. Diagnostic records are designed to omit raw
command output, credentials and repository contents. Terminal output and exports
are separate and may contain sensitive information; inspect them before sharing.

Local crash reports contain bounded technical information such as Rocket version,
OS/architecture, time, component, numeric process/thread IDs and limited code
locations. They exclude memory dumps, credentials, repository contents, full file
paths and panic messages. Reports are not uploaded automatically. Current crash
storage keeps at most 20 session files with up to eight 2 KiB records per file;
operation logs rotate across five files of up to 1 MiB each.

If you choose to share an issue, screenshot, export or report, its recipient
receives that information. Material posted in public GitHub issues is public and
handled by GitHub as well as the project's maintainers. Remove private code,
credentials and personal information before posting.

## Retention and your choices

Saved preferences, drafts, accounts, extension data and recovery files can remain
until you remove or replace them. Closing Rocket is not a request to erase saved
data. Uninstalling the Windows Inno or MSIX app deliberately retains settings and
recovery data; deleting a portable app folder also does not clear a separate
profile. Repositories and SSH keys are not removed by uninstalling Rocket.

Use the account and extension controls to disconnect, forget or remove entries.
To clear the whole local profile, close Rocket and its helpers, review and back
up any drafts/recovery files you still need, then remove only the Rocket profile
folder you intend to clear. This does not erase external repositories, exported
files, provider accounts, credentials in OS stores or other tools' histories.
Removing an agent-session record also does not delete its worktree.

You can leave optional network integrations off, disable automatic fetch, stop
MCP, and choose which tools or extensions to trust. Manage remotely published
content and revoke credentials with the receiving provider. Copies already
shared with collaborators, retained in Git history or stored in backups may
remain; local deletion does not recall them.

## Website and third-party services

The Rocket website serves its own scripts, images and media without advertising
or third-party analytics scripts. It uses browser local storage for the
`rocket-theme` light/dark preference; the site does not send that value to Rocket's
maintainers. Clear the site's browser data to remove it. The website does not set
its own tracking cookies.

The website, downloads, source and issue tracker are hosted by GitHub. GitHub can
process connection and usage information under its
[privacy statement](https://docs.github.com/en/site-policy/privacy-policies/github-general-privacy-statement).
Microsoft Store installation and update services are covered by
[Microsoft's privacy statement](https://privacy.microsoft.com/en-us/privacystatement).
Other Git hosts, AI providers and extension services apply their own policies;
the destination you choose determines who receives that data and where it is
processed.

## Questions and policy changes

For general privacy questions, use
[Rocket's GitHub issue tracker](https://github.com/JasXTD/Rocket/issues).
Do not post private information there. Report security vulnerabilities through
GitHub's private vulnerability-reporting flow, as described in
[SECURITY.md](https://github.com/JasXTD/Rocket/blob/main/SECURITY.md).
Requests about data held by GitHub, Microsoft or another connected service must
also be directed to that service.

Changes to this policy will be published here with an updated date. The source
history records revisions. Review the policy again when enabling a new connected
feature or updating Rocket.
