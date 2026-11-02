# Hosting accounts and pull requests

Open a repository, then **Repository → Remotes & Hosting → Hosting**; accounts are in **Edit → Preferences → Accounts** (Hosting links there). This is separate from Git transport, Git Credential Manager and system SSH. Connecting a hosting account does not rewrite remotes or local Git configuration.

## Providers

GitHub's PR and issue views additionally support server-side literal title/body
search, author and assignee usernames. Search stays within the selected repository
and state, with 50-result pages and GitHub's 1,000-result cap. Partial responses
are explicit. Up to twelve recent searches persist as local settings and can be
cleared. Cancellation closes the request while retaining previously shown results.
These search controls support GitHub.com and GitHub Enterprise Server; other provider lists are unchanged.

| Provider | Server | Repository field | Features |
| --- | --- | --- | --- |
| GitHub | https://github.com | owner/repository | PRs, issues, reviewed same-repository/cross-fork creation; PR details, inline reviews and reviewed merge/squash/rebase |
| GitHub Enterprise Server | your HTTPS server origin, optional port | owner/repository | Token accounts; PRs, issues, search, reviewed creation, PR details/reviews and reviewed merge/squash/rebase |
| GitLab | https://gitlab.com or your HTTPS GitLab server | namespace/project; nested namespaces allowed | MRs, issues, reviewed creation; details, comments, approval and merge/squash |
| Bitbucket Cloud | https://bitbucket.org | workspace/repository | PR creation/details, comments, approval/change requests and merge/squash; no native issue adapter |
| Bitbucket Server / Data Center | your HTTPS server, including context path | PROJECT/repository or ~user/repository | Token accounts, PR creation/details, comments, review votes and version-checked merge/squash; no Jira adapter |
| Azure DevOps Services | https://dev.azure.com/organization | project/repository | PRs, work items, reviewed PR creation/details, comments, votes and completion |
| Azure DevOps Server | your HTTPS collection URL, including virtual directory | project/repository | PAT accounts, explicit API 6.0/7.0/7.1, PRs/work items, PR creation/details, comments, votes and completion |
| Gitea | your HTTPS Gitea server | owner/repository | PRs, issues, reviewed creation/details, reviews and merge/squash/rebase |

GitLab, Gitea and Bitbucket Server/Data Center support an explicitly configured HTTPS server and optional base path. Azure DevOps Server requires its collection path. Redirects are refused; enter the final server address.

## PR collaboration and saved drafts (unreleased)

Open a PR's **Details and reviews** to read commits, changed files, discussion,
reviews and available checks. Each collection is bounded and reports missing
permissions or partial data. Counts unavailable from the provider are not shown
as a complete zero. Open the provider for the full discussion and policy details.

**Write review** opens a local draft. Review the preview, then explicitly submit.
GitLab supports comments and approval; its request-changes vote is unavailable in
this adapter. Bitbucket Cloud/Server and Azure support comments and review votes.
Their vote endpoints do not accept accompanying text: publish a separate comment
if needed. Gitea supports comments, approvals and change requests with a message.
Only GitHub.com/GHES currently publish inline comments and manage review threads.

For GitHub, choose a changed line in **Files** and optionally set a first/last
line on the same diff side and hunk. In **Discussion**, **Load / refresh threads**
allows replying, resolving or reopening a thread if the server permits it. Every
write gets a fresh, single-use preview. Changed commits or thread state require
another preview. The viewer shows up to 50 threads and 20 comments per thread.

Rocket saves review text, line locations, reply drafts and uncertain review,
thread and merge outcomes in `hosting-drafts.dpapi`, protected for the current
Windows user. **Saved drafts** restores them only with the same provider, server,
API contract, account and repository. Up to 32 drafts are retained. Tokens and
executable previews are never stored there. A restored draft needs fresh server
data; line comments tied to an older revision retain their text for review.
Drafts are written off the UI thread. A recovery record must reach disk before
a remote write starts. Storage errors or external changes prevent submission and
preserve existing data. An unreadable store does not prevent exiting when there
is no unsaved local draft.

Cancellation cannot undo an accepted server request. After a lost response or
restart, refresh the same PR, inspect its current server state, then acknowledge
the uncertainty before creating a new preview. Rocket never automatically replays
saved requests. Recovery records with unknown outcomes cannot be deleted here.

Merge previews expose supported methods and the provider's condition. GitHub,
GitLab and Gitea receive the reviewed head; Bitbucket Server receives its PR
version; Azure receives `lastMergeSourceCommit`. Bitbucket Cloud's merge endpoint
has no documented expected-head condition. Both tips are rechecked, but that does
not remove a race after the check. GitLab project policy determines its merge
strategy; selecting merge does not override that policy. Only a confirmed merged
response counts as success; asynchronous or unclear results require inspection.
Bitbucket Cloud may queue a merge when its target has an active merge queue;
Rocket reports that unconfirmed outcome for inspection instead of retrying it.
Rocket does not bypass policies, delete source branches or alter local files.

The adapters use official API contracts and isolated mock tests. This is not
live acceptance of each host, enterprise version, token scope or server policy.
No Jira/custom-script runtime is included. A future scripted connector should
use permissioned, brokered requests with credentials retained by Rocket, explicit
write previews, cancellation and a separate bounded worker process.

## Azure DevOps Server (unreleased)

The PR collaboration extension described below adds details, comments, votes and
reviewed completion to this adapter. These features use the selected REST contract.

1. Open **Edit → Preferences → Accounts** and choose **Azure DevOps Server** under Self-hosted. Give the profile
   a name, then enter the HTTPS collection URL, for example
   `https://devops.example.com/tfs/DefaultCollection`. Include the virtual
   directory and collection; do not append a project, `_git` or `_apis`.
2. Select **2020 (6.0)**, **2022 (7.0)** or **2022.1+ (7.1)** for the server's
   supported REST contract. The default is 7.0. This is an explicit selection,
   not automatic server-version detection. Earlier servers are not supported.
3. Enter a PAT issued by that server and choose **Connect and verify**. Rocket
   checks the collection's versionless `/_apis/connectionData` endpoint for an
   active authenticated user. A successful anonymous response is insufficient.
   Optional **Remember** protects the PAT and selected version for your Windows
   user, separately from Azure DevOps Services profiles.
4. Enter `project/repository`. Browse pull requests or project-wide work items;
   work-item states are exact, project-defined names. To create a PR, use two
   already-pushed branches, review their tips, then confirm. No Git push occurs.

PAT scopes are **Code (Read)** for PR/branch reads, **Work Items (Read)** for work
items and **Code (Read & write)** for PR creation. Server policies and account
permissions still apply. Windows integrated authentication, browser SSO/Entra,
work-item editing, inline reviews and cross-fork creation are outside
this Server adapter. Git transport credentials remain separate.

Changing the collection or API version disconnects the session and clears old
results/previews. Pages are bound to the same collection, repository, filter,
version and session. HTTPS remotes can prefill the repository only on an exact
configured collection match. SSH remotes need that same host and collection
path; aliases or different virtual-directory mappings require manual selection.

Cancellation retains existing read results. If PR creation was submitted before
cancellation, its outcome may be unknown: refresh the same repository's PR list,
check it on the server and acknowledge the result before another attempt. Rocket
does not automatically retry or change API versions. Branch tips are rechecked
before submission, but Azure's create API has no atomic compare-and-create guard;
the server can still change immediately after that check.

Token rejection, forbidden access, unavailable resources, rate limits, TLS,
redirects, service failures and rejected API requests have separate diagnostic
codes. A 400/410 means the API request was rejected; it does not by itself prove
an unsupported version. Check collection, contract and filters. Certificate
verification stays enabled. Ask the administrator about private CA trust used
by the installed curl helper or PAT/IIS policy; do not enable IIS Basic auth as
a workaround. Private addresses, PATs and response bodies are excluded from logs.

Existing protected profiles remain readable. Older builds that do not recognize
`azure-server` refuse that profile file without replacing it; manage it with
this build. Local mock acceptance does not establish real Server versions,
enterprise proxies/CAs, PAT scopes, SSO or organization-policy compatibility.

Contracts: [Microsoft's REST version matrix](https://learn.microsoft.com/en-us/rest/api/azure/devops/?view=azure-devops-rest-7.2),
[on-premises authentication guidance](https://learn.microsoft.com/en-us/azure/devops/integrate/get-started/authentication/authentication-guidance?view=azure-devops-2022),
[connection identity](https://learn.microsoft.com/en-us/javascript/api/azure-devops-extension-api/connectiondata)
and [Microsoft's versionless connection implementation](https://github.com/microsoft/azure-devops-node-api/blob/master/api/WebApi.ts).

## GitHub Enterprise Server (unreleased)

1. Open **Edit → Preferences → Accounts**, or **Accounts** on the Hosting page.
2. Choose **GitHub Enterprise Server**, name the profile and enter the server
   origin, such as `https://git.example.com` or `https://git.example.com:8443`.
   Do not append `/api/v3`, a repository path, credentials or a query.
3. Enter a personal access token issued by that server and **Connect and verify**.
   Rocket verifies `/api/v3/user` and shows the authenticated account. A valid
   reported enterprise version is displayed; an absent version is not guessed.
4. Choose `owner/repository`. A matching HTTPS/SSH remote can prefill it. Browse
   PRs/issues, search, or use the same reviewed PR workflows described below.

The adapter uses [GitHub's Enterprise REST API](https://docs.github.com/en/enterprise-server@3.20/rest/using-the-rest-api/getting-started-with-the-rest-api?tool=curl)
with the explicit `2022-11-28` API contract. This is a REST contract, not the
appliance version. See [GitHub's API version policy](https://docs.github.com/en/enterprise-server@3.20/rest/about-the-rest-api/api-versions).
Missing permissions/endpoints stay visible; unsupported API responses never
silently enable features or retry with another version.

Each saved account retains its provider, origin and protected token. Changing
the server invalidates the active session. **Remember** uses the existing Windows
user-protected account file; leaving it off keeps the token in memory. It does
not read or change Git Credential Manager, system SSH keys or repository config.
Older Rocket builds do not recognize Enterprise profiles and will refuse to load
that account file without changing it; use this build to manage those profiles.
GHES browser OAuth, public-GitHub notifications and new-repository publication
are outside this adapter. Browser sign-in and Publish repository remain scoped
to their existing public providers.

TLS verification stays enabled. For a private CA, have the administrator configure
the certificate trust used by the installed curl helper; Rocket does not provide
an insecure bypass. Redirects are refused, including redirects to a browser login.
Pagination and browser links are bound to the configured server and repository.
Token rejection, forbidden access, rate limits, certificate failures, redirects,
retired APIs and service errors have fixed diagnostic codes and actionable messages.
Response bodies, tokens, server addresses and repository metadata stay out of logs.
Cancellation shows Stopping immediately; a possibly submitted write retains the
existing uncertain-outcome recovery workflow instead of retrying automatically.

Local mock APIs and native MCP tests validate these contracts, including a
loopback untrusted-certificate rejection. No real GHES deployment, organization
policy, SSO configuration or token scope has been accepted end to end yet.

## Connect accounts

For GitHub, **Sign in with browser** uses the installed Git Credential Manager
(GCM). Review GitHub's consent screen before granting access; GCM chooses the
OAuth scopes. Rocket invokes `git credential fill` in a neutral directory with a
fresh helper list, browser-only GitHub authentication, credential storage disabled
and tracing disabled for this child. It does not approve/reject credentials in
your normal Git store. The returned token must pass account verification before
Rocket connects or optionally saves it with current-user DPAPI. If GCM is missing,
use the existing token flow. Bitbucket Cloud also offers browser sign-in, with the
temporary-session limits below. Other providers use access tokens.

Stopping terminates the owned helper and closes the pending request. It cannot
revoke consent already granted in the external browser. GCM uses the same GitHub
OAuth application as ordinary Git authentication: revoking that application's
grant can also break your existing Git login. Disconnect in Rocket to discard
only its in-memory session. Forgetting a Rocket profile does not revoke access.
GitHub also limits tokens per user/application/scope; repeated fresh sign-ins can
revoke an older token under its [OAuth token limits](https://docs.github.com/en/apps/oauth-apps/using-oauth-apps/authorizing-oauth-apps).
The localhost MCP hosting fixture replaces the OAuth exchange with a synthetic
loopback endpoint and never launches GCM or accesses personal credentials. That
fixture verifies UI state, cancellation, persistence and log privacy; it does not
prove real GitHub consent, organization policy or live token refresh behavior.
GCM settings follow its [official configuration documentation](https://github.com/git-ecosystem/git-credential-manager/blob/main/docs/configuration.md).

Choose the provider, name the profile, enter its server and an access token, then **Connect and verify**. For Bitbucket, leave the email blank for token-only Bearer authentication. Existing profiles with an Atlassian email retain Basic authentication with the API token. Both methods are documented by [Atlassian](https://support.atlassian.com/bitbucket-cloud/docs/using-api-tokens/). App passwords are not supported.

Account verification also needs access to the current-user endpoint. For Bitbucket API tokens, include `read:user:bitbucket` as documented in [Get current user](https://developer.atlassian.com/cloud/bitbucket/rest/api-group-users/#api-user-get), in addition to repository and pull-request permissions.

Tokens stay in memory by default. **Remember this account** stores the profile and token in hosting-accounts.dpapi inside the Rocket profile, encrypted with Windows current-user DPAPI. It is not a portable backup or a shared credential store. Rocket never writes the token to Git configuration, command arguments, diagnostics or MCP state. Forget removes only the selected saved Rocket profile; it does not revoke a provider token or delete Git/SSH credentials. Disconnect ends the in-memory session.

Token permissions vary by provider and organization. Begin with repository metadata, PR and issue read permissions; PR creation needs the corresponding write permission. GitLab read_api allows browsing; api is needed for MR creation. Azure uses Code and Work Items scopes; organizations may disable PAT use. Review exact provider requirements:
- [GitHub pull request API](https://docs.github.com/en/rest/pulls/pulls)
- [GitLab authentication](https://docs.gitlab.com/api/rest/authentication/)
- [Bitbucket API tokens](https://support.atlassian.com/bitbucket-cloud/docs/using-api-tokens/)
- [Azure DevOps PAT guidance](https://learn.microsoft.com/en-us/azure/devops/organizations/accounts/use-personal-access-tokens-to-authenticate?view=azure-devops)
- [Gitea API authentication](https://docs.gitea.com/development/api-usage/)

Existing GitHub repository/notification browsing through Git Credential Manager remains in its separate menu entry.

## Browse and create

### Bitbucket accounts (0.79)

Cloud's **Sign in with browser** uses installed GCM in OAuth-only mode with its
credential store disabled, separate from existing Git logins. Rocket receives an
access token, not GCM's refresh token. This first version keeps browser sessions
in memory and asks for sign-in again on expiry; **Remember** applies only to API
tokens for Cloud. Atlassian documents a one-hour OAuth lifetime. GCM's grant and
the account policy determine PR permissions; if PR access is denied, connect with
a suitably scoped API token. Automatic OAuth renewal is not implemented.

Server/Data Center uses a personal HTTP access token, sent as Bearer, and the
HTTPS base URL such as `https://git.example.com:8443/bitbucket`. The application
properties response must contain one non-anonymous authenticated `X-AUSERNAME`;
an optional username must match it. Missing/stripped identity headers are refused
with `hosting_identity_unverified`, rather than treating a public response as a
login. Project/repository tokens without a user identity, server browser SSO and
Jira issues are not supported in this pass. Server deployments remain unverified.

Server PR lists follow `nextPageStart` and `isLastPage`; Closed filters the
provider's ALL results to merged/declined PRs and may produce short/empty pages
with more results. Branch preview uses exact ref IDs after the API's fuzzy filter.
HTTP(S) clone URLs with `/scm/`, SSH port URLs and personal projects are supported
for inference; SSH aliases require explicit account/repository selection.

References: [GCM Bitbucket browser authentication](https://github.com/git-ecosystem/git-credential-manager/blob/v2.9.1/docs/bitbucket-authentication.md),
[Atlassian OAuth](https://developer.atlassian.com/cloud/bitbucket/rest/intro/),
[HTTP tokens](https://confluence.atlassian.com/bitbucketserver/http-access-tokens-939515499.html),
[REST 1.0](https://docs.atlassian.com/bitbucket-server/rest/7.7.0/bitbucket-rest.html),
[Atlassian identity-header guidance](https://community.developer.atlassian.com/t/obtain-authorised-users-username-from-api/24422).

### Lists and previews

A matching HTTPS/SSH remote pre-fills the repository after connecting; otherwise enter it explicitly. Open, Closed/merged and All select the provider state. Load next page continues a bounded list (up to 100 pages). Text filtering is explicitly over loaded results. Azure work items belong to the whole project; their optional exact-state field supports project-specific state names. [Bitbucket retired its issue tracker in August 2026](https://community.atlassian.com/forums/discussion/3193882/announcing-sunset-of-bitbucket-issues-and-wikis).

New pull request accepts two already-pushed branches in the **same remote repository**, title and description. Review reads both remote branch IDs and shows the account, server, repository and direction. Create rechecks those IDs and refuses a stale/expired/already-submitted review. The review expires after two minutes. Branch changes are detected before submission; provider APIs do not offer an atomic compare-and-create guarantee. Rocket does not push, merge or modify the local checkout.

Cancelling a read retains earlier results. Cancelling after a create request may have reached the server reports **Outcome unknown**, never a guaranteed rollback. Refresh Pull requests, inspect the provider, and explicitly acknowledge that you checked the same server/repository before another review. There is no automatic retry. Server validation failures and authentication, permission, missing-resource, rate-limit and transport failures have separate, payload-free diagnostic codes.

Pagination follows Bitbucket's returned continuation and GitHub/GitLab/Gitea
Link headers, including short server-limited pages. Links must match the selected
API origin, base path and exact collection; redirects and other destinations are
refused. A cursor is tied to the account session, repository and filter that
created it. `hosting_invalid_pagination` means refresh the list; earlier results
remain visible. Cancelling Load next page retains its cursor for an explicit retry.

## GitHub PR details and reviews (0.87)

In **Pull requests**, choose **Details and reviews** below a GitHub PR. Overview,
Commits, Files, Discussion, Reviews and Checks show provider data without writing
the local repository. Renames retain the previous path. Files display bounded
patch text; binary/missing/large patches have an explanation. Remote HTML, images
and API-supplied URLs are not executed or followed. Hidden control characters
are escaped in provider text while ordinary Unicode is retained.

Each collection loads at most four pages / 200 entries. A collection failure or
remaining server results is explicit; open GitHub for full history. Patch previews
are limited to 64 KiB per file and 1 MiB per snapshot. Description previews are
64 KiB; individual discussion/review bodies are 8 KiB. GitHub may further truncate
patches. The PR head/base and state are read again after loading to reject a mixed
revision snapshot. Conversation and CI are observations, not a transactional view
of everything on GitHub.

Check runs and commit statuses are queried for the exact loaded head in the base
repository. Missing/permission-denied data never becomes a passing result. Checks
running in a fork may not appear here. Required branch rules, required approvals
and merge eligibility are not evaluated by this view.

**Write a review** offers Comment, Approve and Request changes. Comment and
Request changes require a message; approval may have an empty message. Rocket
refuses approval/request-changes on your own PR. Closed/merged PRs are read-only.
**Review before sending** shows the destination, account, action, body and exact
head. Preview expires after two minutes; submitting rechecks the head/base/state
and sends an explicit `commit_id`. GitHub does not offer a conditional review
POST: a push racing after revalidation leaves the review on the reviewed commit,
not the new head. Submission may notify participants. See the official
[GitHub review API](https://docs.github.com/en/rest/pulls/reviews).

Tokens need Pull requests read/write permissions as appropriate; checks, statuses
and issue comments have separate access requirements. Browser consent, account
permissions and organization policy remain authoritative. Existing Git/GCM/SSH
credentials are untouched. Rocket's current verified account is reused.

Read cancellation retains the previous details and draft. A failed review keeps
the draft. After a possible submitted request, cancellation/invalid response means
**Outcome unknown**, not rollback. Refresh that same PR's reviews, inspect GitHub,
then explicitly acknowledge before another preview. Incomplete/unavailable review
history cannot clear that guard. Confirmed successful responses remain successful
despite late cancellation. There is no automatic retry. Drafts and uncertainty
state are protected locally in the PR collaboration extension, including across
restarts. Another PR cannot silently take over an existing draft.

### Inline reviews (unreleased)

In **Files**, click a left or right line number, write a comment, then **Save to
review draft**. Left refers to the old file and right to the new file; renames use
the current filename. Edit/remove queued comments in the review composer. Save
or cancel an unfinished line draft before **Review before sending**, then submit
the comments together with Comment, Approve or Request changes. The existing
overall-message requirement still applies to Comment and Request changes.

Up to 32 line/range comments are allowed, with 8 KiB per comment and 128 KiB of
total review text. Only complete, parsed hunks in the bounded patch can supply
locations (up to 2,000 displayed rows per file). Binary, missing, oversized or
malformed/truncated hunks cannot supply invented locations. Ordinary Unicode
paths are retained; paths whose display requires escaping are refused for comments.
Comments retain their original account, PR, head/base and line. After a revision
change, remove and re-select stale comments; refresh never moves them silently.
These are protected local drafts, not GitHub server-side pending reviews.
The PR collaboration extension adds replies, resolution and multiline ranges.

### Merge on GitHub (unreleased)

Submit or discard review drafts, choose **Merge commit**, **Squash and merge** or
**Rebase and merge**, then **Review merge**. The preview shows repository, target,
method and both commit IDs. **Merge reviewed PR** rechecks the open/non-draft PR,
head/base, current account write permission, enabled method and GitHub's clean
mergeability state. Unknown, conflicting, blocked or unstable state is refused.
GitHub's actual rules and permissions decide the result; the Checks tab is not a
complete branch-rule evaluator. Merging needs suitable Contents write permission.

The request supplies GitHub's `sha` head condition. GitHub offers no equivalent
base-SHA condition on this endpoint: the base is checked just before sending but
can race afterward. A successful result needs `merged: true` and a valid returned
commit ID. No branch deletion, local checkout write, bypass flag, automatic retry,
merge-queue enrollment or async auto-merge is requested. The preview is single-use
and expires after two minutes. See [GitHub's merge API](https://docs.github.com/en/rest/pulls/pulls#merge-a-pull-request).

Cancellation before sending makes no merge request. Interruption or an invalid
response after sending means **Outcome unknown**. Refresh that same PR; a merged
response resolves its state but cannot identify which attempt merged it. Otherwise
inspect GitHub and explicitly acknowledge before making another preview. Another
PR cannot clear the guard. This state persists across restarts in the PR
collaboration extension. Confirmed success survives late cancellation.

### Cross-fork creation (unreleased)

In **Create pull request**, the **Repository** is the destination. Enable **Use a
different source repository (fork)** and enter its `owner/repository`. Both
repositories must be in the same GitHub fork network and both branches must
already exist remotely. Rocket creates neither forks nor local/remote branches.
The source and target may share a branch name when they are in different repos.

Review both repositories and branch IDs. Before sending, Rocket rechecks the fork
network and both tips. It sends an owner-qualified head and explicit `head_repo`
(including same-organization forks), with maintainer edits disabled. GitHub has
no atomic compare-and-create operation; changed or unexpected returned endpoints
become an uncertain result to inspect, never an automatic retry. See
[GitHub's create-PR API](https://docs.github.com/en/rest/pulls/pulls#create-a-pull-request).

These new workflows are GitHub desktop features. Other providers keep their
existing list/create adapters; CLI review commands are not included. Local
synthetic API/native MCP checks do not establish real GitHub review/merge scopes,
organization policy or cross-fork acceptance. Existing live Git transport and
sign-in evidence is separate.

## Publish a local repository to GitHub

With a local repository open, connect a GitHub account in **Preferences → Accounts**,
then choose **Publish repository** (or **Publish** at compact widths). Enter your
login or organization as owner, a new repository name, the local branch and an
unused remote name. Private is the default. Public publishing requires an
acknowledgement that the branch's entire committed history will be readable.

**Review publish** checks the branch, configuration, destination and outgoing
history. **Create repository and push** rechecks that review, creates an empty
GitHub repository, appends the local remote and pushes the reviewed commit. It
does not stage or commit changes, initialize a README/license/ignore template,
publish other branches/tags, or overwrite an existing remote. With secret
protection enabled, suspected secrets in historical objects block creation too.

GitHub creation uses the connected hosting account; the push uses existing Git
HTTPS credentials or system SSH. The API token never enters Git arguments or
config, and Rocket does not replace/revoke the normal Git credential store. The
two accounts can differ: creation may succeed while the push is denied. In that
case keep the created repository and configured remote, correct Git access and
review a normal Push. No automatic retry or remote deletion is performed.

GitHub's [create-repository API](https://docs.github.com/en/rest/repos/repos#create-a-repository-for-the-authenticated-user)
requires suitable creation permissions; classic/OAuth tokens need `repo` for
private repositories (`public_repo` or `repo` for public ones). Organization
membership and policy also apply. A read-only account can browse successfully
while repository creation is forbidden.

Results separately show repository creation, remote attachment and branch push.
Cancelling a submitted creation request may leave an unknown result: inspect the
reviewed GitHub URL before acknowledging a new review. A successful creation is
retained if a local edit, config lock, cancellation or push rejection stops the
remaining steps. A confirmed push can still have an upstream/refresh warning.

Reviews expire after two minutes and are bound to the account, Git directory,
branch commit and effective configuration. The shared repository write permit
excludes other Rocket writes throughout submission. External Git can still run;
revalidation detects earlier edits and the first push uses an absent-branch lease.
Staged/working files are preserved. Existing URL rewrites are refused for this
first-publish workflow; manage those remotes explicitly. Existing upstreams are
preserved by turning off **Set upstream**. No custom SSH alias selector or CLI
publish command is added in this pass.

The localhost fixture requires the existing explicit hosting API marker plus
pre-created local bare repositories in `publish-remotes/<name>.git`. API response
clone URLs are never used as transport destinations. The native policy confines
publication to the active marked branch. These local fixture tests use no
personal credentials and create no real GitHub repositories. The separate live
acceptance below covers private personal creation and an SSH first push.

## Live publishing acceptance

The optional operator-created `rocket-github-publish-check` marker adds one
strict live acceptance mode. Its exact five LF-terminated lines are
`ROCKET-GITHUB-PUBLISH-CHECK-1`, `owner/rocket-publish-test-<32 lowercase hex>`,
`rocket-ui-test/publish-<first 12 hex>`, `private`, and `ssh`. Only that private
repository, marked local branch and exact GitHub SSH destination are allowed.
It cannot be combined with a loopback fixture or account-only mode. MCP has no
tool to create the marker. Existing modes keep their previous restrictions.

The hosting session permits only account verification, inspection of that exact
repository and private creation through `POST /user/repos`. Public creation,
other repository APIs, deletion, browser sign-in and credential persistence are
disabled in this acceptance mode. The token stays in memory with Remember off;
the first push uses the existing explicitly selected Windows SSH agent profile.
Neither the key nor the credential store is changed. A test runner verifies
the original login and user configuration afterward. If the current credential
lacks GitHub's `delete_repo` scope, the user removes the temporary repository in
GitHub settings; the runner never expands OAuth permissions to perform cleanup.

Version 0.75.1 was tested through localhost MCP against one real private GitHub
repository containing synthetic history. Private creation, SSH publication,
upstream refresh, invalid authentication, existing names/remotes, stale review,
preview cancellation, push cancellation and its reviewed retry passed. The
existing GCM credential, personal Git/SSH files and loaded agent keys were
verified unchanged. Live public/organization publishing, first-push HTTPS and a
lost creation response were not exercised; the last case remains covered by
the controlled local fixture. See VALIDATION.md for evidence and cleanup status.

## Validation and development

Validation uses local provider-shaped HTTP fixtures and native localhost MCP, with temporary repositories and disposable tokens. Completed results are recorded in VALIDATION.md, including the 0.69.1 contract review. That does not prove real credentials/scopes, SSO, self-hosted deployment versions or provider policy compatibility. Live GitHub Enterprise Server, GitLab, Bitbucket, Azure and Gitea account acceptance remains unverified. GitHub's separately completed browser account verification does not establish those providers' behavior or live PR creation.

The MCP host disables these adapters by default. Only a bounded regular rocket-hosting-api marker containing an exact http://127.0.0.1:port endpoint enables its synthetic API override. Every request then stays on that loopback API. The older GitHub read-only live marker does not authorize these writes. Hosting account state in tests stays in the fixture's Rocket profile.

A separate operator-created `rocket-github-browser-check` marker permits one
manually completed live sign-in test. Its exact contents are
`ROCKET-GITHUB-BROWSER-CHECK-1`, `https://github.com`, and
`JasXTD/rocket-auth-test`, each followed by a newline. This mode disables Remember,
token entry, repository browsing and hosting writes. The resulting backend
session permits only `GET /user`; disconnect discards it. The MCP API cannot
create the opt-in marker. Test runners verify the pre-existing get-only GCM
credential still works and do not revoke the shared GitHub OAuth application.

Focused checks:
- cargo test --offline --locked -p rocket-git --test forge
- cargo test --offline --locked -p rocket-git --lib forge::
- cargo test --offline --locked -p rocket-ui --lib hosting_accounts::
- node tools/tests/test-mcp-hosting.cjs (requires the coordinated release/MCP slot)

Implementation study and official API references: work/hosting-study/README.md. Early mockup: work/hosting-preview/preview.html. No new third-party dependency or installed tool is required. Windows OpenSSH key management is in Edit → Preferences → SSH keys (Repository → SSH Keys and Accounts before the Preferences revamp); see [SSH profiles](crates/rocket-ui/src/guide.md#ssh-keys). API tokens and SSH profiles remain independent.

## Windows Git security prerequisite

Use Git for Windows 2.56.0.windows.4 or newer for HTTP(S) clone, fetch, pull, push
and remote checks. Rocket restricts older or unrecognized Windows Git builds to
local/SSH transport, including after URL rewrites. Preferences → Git shows the
full build suffix and any restriction. Restart Rocket after updating Git.
Hosting API requests use Windows' System32 curl, resolved by the operating system;
a missing helper is reported without falling back to curl beside Git.
