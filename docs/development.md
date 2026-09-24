# Development builds

The canonical source is `/Users/david/Workspace/me`. Commit and push product code
from that checkout. Temporary work/staging directories are not application roots.
The developer's everyday test app is **ME Dev**, an actual Cargo debug build with
`development-tools`. Never hand out a per-feature app as its replacement.

| Channel | Command | Installed app | Bundle identifier | Data |
| --- | --- | --- | --- | --- |
| Development | `./scripts/dev-macos` | `~/Applications/ME Dev.app` | `local.me.desktop.dev` | `~/Library/Application Support/ME Dev/` |
| Isolated preview | `./scripts/bundle-macos preview` | `target/preview/ME Preview.app` | `local.me.desktop.preview` | `target/preview/data/` |
| Release candidate | `./scripts/bundle-macos release` | `target/release/ME.app` | `local.me.desktop` | Normal ME location or explicit configuration |

`./scripts/bundle-macos` defaults to development; `debug` remains an alias for
`dev`. Previews use optimized code with development controls and a separate vault.
Use the existing synthetic gallery examples for layout tests. Release candidates
omit development controls; local signing does not constitute a notarized public
release. The local account API remains a separate service; see [accounts.md](accounts.md).
No command publishes a binary or resets OS permissions.

## First setup and subsequent updates

Install a valid Developer ID or Apple Development certificate in the local
keychain. Set `ME_CODESIGN_IDENTITY` to its full name or SHA-1 if multiple
certificates are available. Set `ME_ACCOUNT_API_URL` to the development API's
HTTPS origin (loopback HTTP is supported), then run `./scripts/dev-macos`.
The API origin is retained in the installed app across subsequent builds.

One-time migration from the old registration test app:

```sh
# Quit the old app first. Supply its actual path on this Mac.
./scripts/dev-macos --migrate-from '/path/to/ME Registration Test.app'
open "$HOME/Applications/ME Dev.app"
```

Migration copies all vaults/account slots, account selection, device interface
preferences and dedicated provider sign-in state to Application Support. Generated
provider caches, skills and temporary executable links are omitted and regenerate. Files are hashed
before and after; changing source data, symlinks and a nonempty destination are
rejected. The original data stays untouched as a fallback. No decrypted vault
content or provider credentials are printed, bundled or committed.

For every later update, quit ME Dev and run `./scripts/dev-macos`. Open that same
installed path through Finder/LaunchServices. Do not run `target/debug/me` as the
user's everyday build: its process identity differs from the installed app.
The installer signs a staged bundle, checks it, refuses to replace a running app,
and swaps it into the fixed location. It leaves all account data alone.

## Stable macOS privacy identity

The first ME Dev install pins its certificate and the main/helper designated code
requirements in `~/Library/Application Support/ME Development/channel.json`.
Every update must match all three requirements. Bundle ID, path, certificate and
helper identifiers stay fixed even as build version and source revision change.
A different signer, identifier or ad-hoc fallback fails installation instead of
quietly invalidating privacy grants. The previous app remains installed if signing
or verification fails. Do not delete this pin file to work around a failure.

ME Dev needs an initial Screen Recording and Accessibility grant because it has
its own identity, separate from old test apps and previews. Open **ME Dev → Privacy
permissions…**, also available from Settings while unlocked. The panel checks the
actual current grants using a fresh helper, without capturing windows. Allow access
requests only that permission; Open Settings goes to its macOS pane. Check again
reads the current state. Follow macOS's quit/reopen instruction if requested.

Once granted, ordinary builds satisfying the pinned identity retain permissions.
Users can still revoke access, and macOS or device policy may require renewed
consent independently of app updates. This workflow never edits TCC databases,
adds private entitlements or weakens code requirements. See Apple's
[code requirements note](https://developer.apple.com/documentation/technotes/tn3127-inside-code-signing-requirements)
and [ScreenCaptureKit signing explanation](https://developer.apple.com/forums/thread/819406).

## Checks before handing out a build

Run `./scripts/check-design-system`, the Python design/packaging regression tests,
`./scripts/cargo fmt --all -- --check`, workspace check, workspace tests, and strict
Clippy. The account API's ignored PostgreSQL test needs its own disposable DB.
Visually inspect changed views at 1120×820 and 800×600 with synthetic data.
Check the installed bundle's signature and permission panel. A successful test
launched from a terminal/agent does not prove the Finder-launched app has access.

Push source, migrations, documentation, lockfiles and synthetic fixtures. Keep
vaults, exports, account/provider state, logs, local configuration, signing keys,
compiled apps and the local toolchain out of Git. The repository is public but
source-available under its existing restrictive license, not open source.
