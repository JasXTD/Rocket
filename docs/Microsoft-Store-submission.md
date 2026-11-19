# GitRocket Microsoft Store submission

These notes describe the current Windows MSIX candidate. They are preparation
for Partner Center, not evidence of Microsoft approval or publication. The Inno
Setup installer and portable ZIP remain supported separately.

The security update is Rocket **0.92.6**, MSIX **1.92.6.0**. The previous
certification report covers 1.92.5.0. Verify the new package and run WACK
against its exact bytes before submitting the update.

## Product identity

| Partner Center field | Value |
| --- | --- |
| Display name | GitRocket |
| Package/Identity/Name | `TwinCatz.GitRocket` |
| Package/Identity/Publisher | `CN=3C1B7D92-578D-491F-BAF8-AED91B184402` |
| Package/Properties/PublisherDisplayName | `TwinCatz` |

Use the exact assigned identity. The Store signs MSIX packages during publishing;
the local candidate does not include a private signing key. Installation outside
the Store requires a separately trusted signature. See Microsoft's
[package requirements](https://learn.microsoft.com/en-us/windows/apps/publish/publish-your-app/msix/app-package-requirements).

## Privacy policy

The policy source is [PRIVACY.md](../PRIVACY.md); its website page is
[privacy.html](privacy.html). The intended Partner Center URL is
`https://jasxtd.github.io/Rocket/privacy.html`. Confirm that the page is deployed
and publicly accessible before using that URL in a submission; local preparation
does not publish it.

Declare that GitRocket accesses personal information: Git author/committer
identities, connected account information and user-provided content can contain
personal data. Local processing does not mean the app never accesses such data.
The policy distinguishes local storage, remote Git/hosting operations, optional
AI/extensions/MCP, update services, diagnostics and retained data after uninstall.
General privacy questions use GitHub Issues without publishing private details;
the policy contains no personal email address.

## runFullTrust explanation

Paste into the corresponding restricted-capability field:

> GitRocket is a native Windows desktop Git client. It needs standard-user desktop
> access to read and modify repositories the user selects, run its bundled Git and
> SSH helpers, and support user-initiated terminal and external editor/diff/merge
> workflows. These operations use Win32 processes and files outside an
> AppContainer's private storage. The application runs without administrator
> privileges and refuses elevated startup. Its embedded Lua extensions use a
> separate restricted worker; this capability does not remove that worker's
> isolation. MSIX installations use Windows-managed application updates.

`runFullTrust` is the declaration for a desktop process running at normal user
integrity; it is not an elevation request. GitRocket does not declare
`allowElevation`. Microsoft's
[capability documentation](https://learn.microsoft.com/en-us/windows/apps/package-and-deploy/app-capability-declarations)
describes this distinction.

## unvirtualizedResources explanation

Paste into the corresponding restricted-capability field if requesting approval
for the current package:

> GitRocket shares per-user settings and files with its portable/Inno versions
> and with Git, terminals and external tools launched by the user. It requests
> unvirtualizedResources to disable AppData and HKCU write redirection so these
> workflows see the same data. %LOCALAPPDATA%\Rocket holds preferences, saved
> repository lists, drafts, diagnostics and recovery data. Package-private copies
> would separate those settings and hide changes from tools outside the package.
> Normal user permissions still apply. Uninstall removes the application but
> deliberately retains settings and recovery data; repositories, credentials and
> SSH keys are not deleted.

**Approval is uncertain.** Microsoft's current capability table describes
`unvirtualizedResources` as intended for limited scenarios, including certain
desktop games and packages with external location, and cautions against other
uses because of uninstall behavior. GitRocket is a Git client, not one of those
examples. This explanation is an exception request based on the app's actual
behavior, not a claim that Microsoft has approved that use.

The [virtualization documentation](https://learn.microsoft.com/en-us/windows/msix/desktop/flexible-virtualization)
explains sharing and retention. The
[submission options](https://learn.microsoft.com/en-us/windows/apps/publish/publish-your-app/msix/manage-submission-options#restricted-capabilities)
explain capability review. Do not claim this capability is optional for the
current implementation or remove it only to pass upload validation: package
storage, Git helpers, settings continuity and uninstall must be retested if the
storage model changes. Inno Setup and the ZIP remain distribution options if
Microsoft declines this request.

## Notes for certification

Suggested reviewer notes:

> GitRocket is a desktop Git client. Local repository browsing, history and change
> inspection can be tested without a hosting account. Use a disposable local Git
> repository for tests that change files or history. Remote hosting and agent
> integrations are optional and use accounts/tools supplied by the user. No
> developer credentials or SSH keys are included in the package.
>
> Git is bundled for offline local repository operations. The Store build's Help
> update command opens Windows-managed update guidance; it does not replace its
> own package using GitHub downloads. Saved settings, drafts and recovery data
> deliberately remain after uninstall as described in our capability request.

## Validation and submission boundary

- Check the package's SHA-256 against its verification record before upload.
- Run the installed Windows App Certification Kit against that exact package,
  inspect its report and address required failures. Processing an individual
  task successfully or returning exit code zero is not the same as passing its
  tests. Record optional warnings separately rather than hiding them.
- Review bundled dependency security updates separately: WACK does not establish
  that vendor components are free of known vulnerabilities. A changed runtime or
  rebuilt payload needs a new package verification and certification run.
- Complete the Store listing, privacy information and both capability fields.
- Local tests do not grant Store certification or restricted-capability approval.
  Upload and publication remain separate user actions.

Detailed local WACK evidence is recorded outside the distributable package under
`tools/maintenance/wack-20261118`. Never upload private profiles, raw user-state
inventories, test keys or local diagnostic logs as listing assets.
