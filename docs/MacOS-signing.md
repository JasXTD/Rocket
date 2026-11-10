# Preparing a signed macOS release

Status: preparation only. Rocket's development bundle uses ad-hoc integrity
signatures. The owner has no Developer ID certificate yet; no certificate was
created, account enrolled, secret uploaded or notarization request submitted.
The existing build workflow does not publish a macOS release.

## Prerequisites and boundaries

A future distributor needs Apple Developer Program access, a **Developer ID
Application** certificate with its private key in a controlled Mac Keychain,
and notarization credentials. Keep these outside source, logs and ordinary CI
artifacts. The account owner should configure them locally; this guide uses
only a Keychain profile name, never an account address or password.

Use an isolated signing workspace and an already verified, clean source commit.
Build and test the five executables for the same native Mac architecture (arm64
or Intel x86_64) and the icon before signing. The current
`tools/macos/bundle.cjs stage` can create a fresh **unfinalized** package directory.
Do not run its `finalize` command on a distribution candidate: that command
deliberately applies ad-hoc development signatures.

The release packager still needs an explicit distribution mode and independent
Developer ID/Team ID checks. The following commands are a reviewed runbook for
that future implementation, not an automated release path.

## Sign the helpers, then the application

When including Lua, first sign `RocketLua.app` with the final distribution
identity, hardened runtime and **only** `com.apple.security.app-sandbox=true`.
Use identifier `io.github.jasxtd.rocket.lua`. Then rebuild UI/MCP hosts with the
SHA-256 of that final executable (`ROCKET_LUA_WORKER_SHA256`) and stage the exact
helper at `Contents/Helpers/RocketLua.app`. Changing its signature afterward
invalidates the host binding. The development helper builder uses ad-hoc signing;
the future distribution mode must implement this same order with Developer ID.
The normal outer-app signing loop below deliberately excludes the bound Lua app.

With `ROCKET_APP` pointing to the new workspace's `Rocket.app`, and
`ROCKET_SIGNING_IDENTITY` containing the reviewed certificate fingerprint, sign
inside out. A stable helper identifier matters for Keychain access after an
upgrade. Do not use `--deep` to sign everything indiscriminately.

```sh
for helper in rocket-mcp rocket-cli rocket-rebase-editor rocket-ssh; do
  /usr/bin/codesign --force --sign "$ROCKET_SIGNING_IDENTITY" \
    --options runtime --timestamp \
    --identifier "io.github.jasxtd.rocket.$helper" \
    "$ROCKET_APP/Contents/MacOS/$helper"
done
/usr/bin/codesign --force --sign "$ROCKET_SIGNING_IDENTITY" \
  --options runtime --timestamp --identifier io.github.jasxtd.rocket "$ROCKET_APP"
/usr/bin/codesign --verify --deep --strict --all-architectures "$ROCKET_APP"
/usr/bin/codesign --display --verbose=4 "$ROCKET_APP"
```

Confirm the intended Developer ID authority, Team ID, secure timestamp and
hardened-runtime flag for every executable. Start with no extra entitlements;
add an exception only for a demonstrated need and test it. The separate Lua helper
uses a fixed import-free Wasm guest and App Sandbox; hardened runtime alone is not
the extension sandbox. Apple's
[signing guidance](https://developer.apple.com/library/archive/technotes/tn2206/)
and [notarization requirements](https://developer.apple.com/documentation/security/resolving-common-notarization-issues)
describe these checks.

## Notarize and staple

The account owner first creates the `rocket-notary` Keychain profile using
Apple's local credential setup. Do not put credential values in shell history,
repository settings or workflow output. Use `notarytool`, not the retired
`altool` submission path. See Apple's
[credential-storage instructions](https://developer.apple.com/documentation/technotes/tn3147-migrating-to-the-latest-notarization-tool).

The submission ZIP must be new and contain the signed app. These commands upload
the app to Apple; execute only when release signing is authorized:

```sh
/usr/bin/ditto -c -k --keepParent "$ROCKET_APP" "$ROCKET_NOTARY_ZIP"
xcrun notarytool submit "$ROCKET_NOTARY_ZIP" --keychain-profile rocket-notary --wait
```

Require **Accepted**, retaining the submission ID and any diagnostic JSON.
An upload receipt or timeout is not acceptance. After acceptance:

```sh
xcrun stapler staple "$ROCKET_APP"
xcrun stapler validate "$ROCKET_APP"
/usr/bin/codesign --verify --deep --strict --all-architectures "$ROCKET_APP"
/usr/sbin/spctl --assess --type execute --verbose=4 "$ROCKET_APP"
```

Staple the application, not its ZIP, then create a **new** final ZIP. See Apple's
[notarization workflow](https://developer.apple.com/documentation/security/customizing-the-notarization-workflow).
Signing and stapling change bytes: generate the final manifest and SHA-256 sums
afterward, preserve the original development artifact, and verify an extracted
copy of the final archive. Do not reuse its earlier hashes.

For Rocket's in-app updater, also create the separate signed tar asset with
`node tools/macos/update-package.cjs "$ROCKET_APP" /path/to/new-output TEAM_ID`.
This validates Developer ID, the same publisher identity and notarization again,
then writes `Rocket-VERSION-macos-ARCH.tar.gz` (`ARCH` is `arm64` or `x64`) and its SHA-256 sidecar. Its
inventory stays outside the sealed app. See the [update contract](MacOS-desktop-integrations.md).
The development bundle ZIP is not an in-app update asset.

## Release acceptance still required

On a separate ordinary-user Mac, download the final archive through the intended
distribution route and test Finder launch with Gatekeeper enabled. Check helpers,
Keychain prompts and access after an upgrade, Git discovery, SSH, PTY behavior,
repository notifications, Unicode, cancellation and compact/Retina layouts.
Repeat signature, ticket and hash checks on the downloaded copy. Do not remove
quarantine or disable Gatekeeper to turn a rejection into a pass.

Do not ship until the distribution packager and these checks pass. The future
Developer-ID-bound Lua helper and signed app-update/rollback paths require their
own distribution validation; ad-hoc CI checks cannot establish that acceptance.
