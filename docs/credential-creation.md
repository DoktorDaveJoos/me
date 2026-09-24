# Credential creation and context capture

New item (Cmd/Ctrl+Shift+N) opens a modal over the Logins workspace. Its first
section offers Login, Password/secret, API credential, SSH key, Wi-Fi and
Server/database. Choosing a type closes the modal and opens its editable detail
page. Cancel/Escape dismisses the chooser without changing the selected item.
The existing ME. tokens, outline icons, honeycomb and reduced-motion setting apply.

## Registration from a window

The modal shows actual thumbnails of up to 12 other visible macOS windows, with
window titles and application names. It refreshes on opening or explicit Refresh;
it does not monitor windows in the background. Screen Recording permission and
macOS 14 or later are needed for thumbnails. Images are held in memory, never
sent to a model or saved to capture files, and dropped when the modal closes.

Selecting a thumbnail reads that exact window, matched by process, window ID and
current bounds. Accessibility supplies labels, visible text, field structure and
the selected browser page's URL. If little text is available, Apple Vision runs
OCR locally on that selected window. OCR cannot identify autofill targets.
Permission denial and unsupported windows leave manual type selection available.

A recognizable empty email/password registration form prepares a Login draft:

- The stored website is its origin (scheme, host, optional port), without the
  registration path, query or fragment. `https://forge.laravel.com/register`
  therefore becomes `https://forge.laravel.com`.
- An email already typed in the form takes precedence. Otherwise the remembered
  ME. account email is suggested, with an editable field for another identity.
- A 20-character password is generated locally using the OS CSPRNG. Sign-in pages,
  password updates and forms that already contain a password do not get an
  automatically generated replacement.
- A name already present in a labelled form field is retained. A Name field is
  available for the user to complete; ME. does not invent a name or infer one from
  an email. The factual tag Web is suggested. Optional notes are left empty.
- Clear forms are classified locally. TypeSafe is consulted automatically for
  ambiguous cases using only fixed detected label concepts. Screenshots, raw
  text, URLs, names, emails and credential values never enter that request.

The user reviews the detail page and chooses Save, or Save and fill where the
browser exposes supported writable Accessibility fields. Save and fill commits
an encrypted credential first, then revalidates the selected window, exact page
URL and form structure before writing email, password/confirmation and an entered
name. Changing the draft's website to another origin disables that operation.
Existing differing form values are not overwritten. Required names exposed by
Accessibility must be completed in the draft first. Filling never clicks Submit,
checks terms, or completes registration on the user's behalf.

If the page navigates, closes, changes its form, denies access or only partially
accepts writes, the login remains saved and the UI describes the failure. It does
not retry automatically or create a second credential. Locking cancels pending
fill work. A save failure never fills the page. HTTP is allowed only for loopback test forms. HTTP remote sites, unsupported
browsers and cross-origin embedded forms do not get a fill target. Native
Accessibility support varies by browser/page; this is not a browser extension and
must not be described as universal autofill support.

Paste copied details remains a secondary option (including Cmd/Ctrl+V in the
chooser). It recognizes labelled fields, OpenSSH key blocks and recovery-code
lists. Recovery/password updates retain an explicit account-selection and review
step. Website matches are advisory, not proof of account identity.

## macOS permission identity and recovery

The bundle helper signs the main app and nested executables with the same available
Developer ID (or Apple Development) certificate. It preserves a previous signer,
requires an explicit choice when certificates are ambiguous, and refuses to replace
a running bundle or silently downgrade it to ad-hoc signing. Set
`ME_CODESIGN_IDENTITY` to a certificate name or SHA-1 to select it. Explicit `-`
is limited to disposable ad-hoc builds; those do not retain privacy grants across
changed binaries. No custom weaker designated requirement, TCC database editing,
or automatic permission reset is used. These are local development signatures;
this change does not add release notarization or timestamping.

Capture and autofill check Accessibility silently. A denied capture retains the
selected window and shows Open Settings and Try again at the top of the chooser.
Only Open Settings requests the system prompt. Try again starts a fresh helper,
checks the current grant and revalidates the selected window before reading it.
The system prompt is asynchronous and is never interpreted as an approval.

For a copy upgraded from the old ad-hoc build, macOS may retain an enabled entry
whose code requirement no longer matches. One-time recovery: quit ME., remove
that old entry from Privacy & Security → Accessibility, add the updated app at
its actual installation path, enable it, and reopen ME. Screen Recording and access to a vault stored in Documents may also
need their grants renewed after this initial signing change. Future builds must keep
the bundle ID and signing identity stable. Granting access remains a user action.
Apple documents the [asynchronous Accessibility prompt](https://developer.apple.com/documentation/applicationservices/1459186-axisprocesstrustedwithoptions)
and [certificate-based code requirements](https://developer.apple.com/documentation/technotes/tn3127-inside-code-signing-requirements).

## Storage and platform limits

Native records use `me-v1` data in the encrypted credential table, retain
credential sensitivity and metadata-only global search, and remain excluded from
document processing/agent scope. Schema 13 preserves imported archives. Creation,
field edits and revisions commit atomically. Password, token, private-key,
passphrase and recovery-code replacements retain dated history. Secrets stay
masked until an explicit reveal. SSH storage does not generate/register keys;
API tokens still come from their provider. Passkeys/live TOTP are not implemented.

The helper bounds node/depth counts, elapsed time, input and output sizes and
image dimensions. It does not read secure-field contents into capture responses,
use clipboard access, execute page scripts, or log captured values. Rust secret
buffers use zeroizing wrappers where possible. Swift, parser, GPU and GPUI
allocations do not provide a comprehensive memory-wipe guarantee.

Linux retains manual creation and pasting. Native window previews/filling are not
implemented on Linux. macOS 13 retains manual creation; previews require 14+.
Screen-reader and Linux runtime verification remain separate.

## Implementation references

[ScreenCaptureKit window enumeration](https://developer.apple.com/documentation/screencapturekit/scshareablecontent)
and [single-frame capture](https://developer.apple.com/documentation/screencapturekit/scscreenshotmanager)
provide thumbnails. [Vision text recognition](https://developer.apple.com/documentation/vision/vnrecognizetextrequest)
provides the local OCR fallback. [Accessibility attribute writes](https://developer.apple.com/documentation/applicationservices/1460434-axuielementsetattributevalue)
provide explicitly reviewed native filling; unsupported attributes are errors.

## Verification — 24 September 2026

The desktop suite (29 tests), four native-credential storage tests, workspace
all-target/all-feature compilation, strict Clippy, formatting, design guard and
its five regression tests passed. The Swift helper self-test covers field
semantics, form fingerprints, registration versus sign-in actions, origin
normalization and rejection of remote HTTP/credential-bearing URLs.

Native macOS review covered the chooser at 1120×820 and 800×600, loading and
error states, real window thumbnails, scrolling, Escape, and manual API selection
opening the corresponding editor. A local synthetic Chrome form verified the
complete preview → registration draft → encrypted save → fill flow: typed email
and name were retained, a password was generated, both password fields filled,
and the browser came forward without submission. Navigating after draft creation
was rejected; the item remained saved and neither password field was filled.
A partial-write failure also retained the saved item and displayed the failure.

No real account was registered or real website password changed. The actual
Laravel Forge registration flow, other browsers, permission-grant screens, OCR
fallback and Linux runtime remain unverified. Screenshot permission was already
available on the test machine. This verification does not establish universal
browser compatibility or a security audit.


### Accessibility repair — 24 September 2026

The affected installed app reproduced a TCC “Failed to match existing code
requirement” error while its Accessibility toggle was enabled; the saved and
current ad-hoc cdhash requirements differed. The replacement app and both helper
executables now use the existing Developer ID certificate, with the installation
path, bundle identifier, account API setting and vault directory preserved.

Workspace compilation, strict Clippy, 30 desktop tests, six signing-policy
regressions, four bundle-configuration regressions, formatting, design guard and
five design regressions passed. The Swift semantic self-test also passed. Two
separately compiled and signed synthetic app revisions had different code hashes
but identical certificate requirements; the second satisfied the first revision's
requirement, and both passed strict signature verification. The recovery screen
was visually checked at 1120×820 and 800×600; a retry of a disappeared synthetic
window reported unavailability without another permission prompt. Renewal of the
old installed app's OS permission entry requires user approval and remains the
final live verification step. The installed app was reopened through LaunchServices;
macOS then requested Documents-folder access for the existing vault location.
That system dialog is protected from computer-use control and must be handled
by the user before the installed app can finish startup. No vault data changed.

## Permanent development channel

Use [ME Dev](development.md) for the user-facing development installation. Its
installer pins and verifies the app and helper requirements across updates.
Screen Recording and Accessibility now share the Privacy permissions panel,
available from the application menu even before unlocking. A quiet status check
uses a fresh helper and captures no content. The Screen Recording request path
honors an immediate successful grant instead of always returning denial; Refresh
can retry after returning from System Settings. Old registration-test grants do
not carry to the deliberately separate ME Dev identity.
