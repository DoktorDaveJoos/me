# Accounts and the first-run setup

The first account implementation uses the selected Rust + Axum + Tokio backend,
with PostgreSQL through SQLx. It creates accounts and validates credentials;
**vault content sync, email delivery and public cloud deployment are not included.**
One account has one vault. This is an implementation preview, not a reviewed
identity or encryption protocol ready for public registration.

## User experience

- First run asks for an email address and a master password (at least 10
  characters, up to 4096 bytes, no composition rules), plus password confirmation.
- The same master password signs in and unlocks the local vault. There is no
  separate app-unlock password. Local unlock works offline after account setup.
- Show a generated recovery code before submitting registration. The user copies
  or writes it down outside ME and explicitly confirms it is saved. The backend
  creates the account when the user continues from the recovery-code step. Never show the code again after
  completing the flow. Copies owned by ME are cleared after 30 seconds, on exit,
  or when leaving setup, if the clipboard still contains the copied value.
- The third step connects an existing OpenAI ChatGPT subscription through the
  existing Codex browser sign-in. Only ChatGPT is offered in this version; the
  account email may differ from the ME. email. Account type, model access and
  usage availability are checked without sending documents or creating model turns.
  The user enters ME. after a successful check. Cancellation, failed checks, API-key
  authentication, missing model access and exhausted quota cannot complete setup.
- First-run completion is saved in the encrypted vault (schema 12), independently
  of live provider readiness. Closing or locking before completion resumes the
  provider step on the next unlock. Previously completed vaults still open offline;
  AI availability continues to require a live connection. Existing account-linked
  vaults from the two-step preview receive the third step on their next unlock;
  unlinked legacy vaults retain their existing startup behavior. Backup restoration
  preserves this introduction preference, while provider authentication remains local.
- An existing local vault is attached using its current password and keys. No
  files are reimported, replaced or uploaded. Backup restoration is retained.
- An existing account can sign in. On the original device this also refreshes its
  password envelope after a password change. On a new device the completion view
  explains that vault download is unavailable until sync is implemented; it
  does not invent a second empty vault.
- Recovery takes email, the saved recovery code and a new master password.
  Before committing, show and require saving a new recovery code. Recovery
  rotates the old code, preserves the vault keys, and updates a matching local
  vault's password envelope. The backend does not recover the original password.

Email syntax is validated and normalized to ASCII lowercase; aliases and dots
are not rewritten. `email_verified` is false: syntax validation and successful
password authentication do not prove email ownership. Mailbox verification,
anti-abuse controls and associated lifecycle policies are release gates.

## Boundaries and storage

`me-protocol` defines bounded API payloads and validation without encryption
capabilities. `me-core::account` implements client-side key wrapping and derives
credentials. `services/api` has no dependency on `me-core`, SQLCipher or client
vault decryption. The desktop uses libcurl with certificate verification,
redirects disabled, bounded responses and connection/request timeouts. Work runs
on its background executor; server password hashing uses bounded blocking workers.

The master password never leaves the client. Account authentication uses Argon2id
v19, 64 MiB, 3 iterations, parallelism 1, with a 16-byte domain-separated SHA-256
salt derived from the normalized email (`me-account-auth-v1:`). The existing vault
password envelope independently uses Argon2id with a random 16-byte salt. The
server stores a salted Argon2id verifier of the derived authentication secret,
not the secret itself. The derived authentication secret is nevertheless a
replayable credential and must be protected like a password; this is not a PAKE.

A recovery code contains 32 OS-random bytes encoded in eight groups of eight hex
characters (256 bits). Domain-separated SHA-256 derives the recovery authentication
secret (`me-recovery-auth-v1:`) and recovery wrapping key (`me-recovery-wrap-v1:`).
Only the former is sent to the server, where it is salted/Argon2id-hashed.
The wrapping key encrypts the existing 64-byte random database/object key bundle
with XChaCha20-Poly1305, bound to `me-recovery-v1:<vault-id>`. Password wrapping
retains `me-keys-v1:<vault-id>`. The server stores only the two encrypted envelopes
and account metadata, never the code or unwrapped vault keys.

Account-linked vaults use header format version 2. The unencrypted header includes account ID, email, backend
origin and account revision. These are account metadata, not vault contents; email
is consequently visible on the locked screen and in encrypted-backup headers.
Existing version-1 headers without account metadata remain readable. Old builds whose
strict header parser predates this field cannot open account-linked headers;
retain a pre-upgrade backup. All fresh backup headers carry the account binding.

## API v1

All account endpoints accept JSON POST requests and return `Cache-Control:
no-store`. Requests are capped at 8 KiB. Responses never contain verifiers,
passwords or raw recovery codes.

| Endpoint | Behavior |
| --- | --- |
| `/v1/accounts/register` | Atomic, unique email and vault ID; returns account and encrypted key envelopes. Exact authenticated retry returns the original account. |
| `/v1/accounts/sign-in` | Verifies derived password credential and returns account/envelopes. |
| `/v1/accounts/recovery/prepare` | Verifies derived recovery credential and returns the encrypted envelopes. No mutation. |
| `/v1/accounts/recovery/complete` | Atomically replaces both verifiers/envelopes using the old recovery credential and expected revision. Exactly one concurrent recovery wins. |
| `/health` | Empty 204 readiness response for the running process. |

No long-lived session, access token, entitlement, device authorization or sync
endpoint is issued in this slice. These authenticated responses are not sessions.
New vault storage is staged locally before the registration network commit. A lost
response can be retried with the exact request; after a restart, sign-in finishes
attaching the staged vault. A lost recovery response is reconciled by authenticating
with the new credential and comparing the exact expected envelope/revision.
Atomic header replacement happens only after decrypting and checking the local
SQLCipher database and its matching vault ID. A different account cannot overwrite
a local vault. New-device sign-in never creates local vault data.

The service limits each socket-peer IP to 20 account attempts per minute, bounds
its in-memory rate map and permits four concurrent hashing workflows. It does not
trust forwarded IP headers. Deploying behind a proxy needs trusted ingress rate
limits and a deliberate peer-identity policy. State is per process, so distributed
limits and abuse protection must precede a public multi-instance deployment.

## Local development

Use a disposable database for tests; keep production credentials out of shell
history. The following password is a synthetic local-development value only.

```sh
docker run --detach --rm --name me-account-dev \
  --publish 127.0.0.1:55432:5432 \
  --env POSTGRES_PASSWORD=synthetic-local-test \
  --env POSTGRES_DB=me_accounts_test postgres:17-alpine

ME_DATABASE_URL=postgres://postgres:synthetic-local-test@127.0.0.1:55432/me_accounts_test \
  ./scripts/cargo run -p me-api

# In another terminal; set ME_VAULT_DIR to a fresh synthetic absolute path.
ME_ACCOUNT_API_URL=http://127.0.0.1:8787 \
  ME_VAULT_DIR=/tmp/me-account-demo/vault ./scripts/cargo run -p me-app
```

The API binds to `127.0.0.1:8787` by default (`ME_API_BIND` overrides it).
`ME_DATABASE_URL` is required. The desktop requires `ME_ACCOUNT_API_URL` during
initial setup; remote origins must use HTTPS. HTTP is accepted only for loopback
development. The chosen origin is bound to the local account and must match the configured
origin on subsequent online sign-in/recovery. A modified/restored header cannot
silently redirect credentials to an arbitrary origin. Offline unlock needs no
service configuration.

Finder-launched macOS bundles need these settings in their launch configuration,
because they do not inherit a terminal's environment. Package a configured build
with `ME_ACCOUNT_API_URL`; ME Dev uses its fixed Application Support vault directory:

```sh
ME_ACCOUNT_API_URL=http://127.0.0.1:8787 \
  ./scripts/dev-macos
```

The bundler records the account origin, vault location and non-secret channel name
in `LSEnvironment`. ME Dev fixes its vault path and retains the account origin
across rebuilds. An explicit origin overrides it; development setup rejects a
missing origin. Database
credentials, provider keys and other shell variables are never bundled. The
service must already be running. There is no implicit cloud endpoint. Never distribute
a development bundle containing a local vault path as a production build.

The first migration is idempotent and creates the account table at startup.
Use PostgreSQL backups and versioned migrations for subsequent schema changes.
The narrower `sqlx-core`/`sqlx-postgres` crates avoid pulling the optional SQLite
backend, whose native link dependency conflicts with the desktop's pinned SQLCipher.

```sh
./scripts/check-design-system
python3 scripts/test-design-system.py
./scripts/cargo fmt --all --check
./scripts/cargo check --workspace --all-targets --all-features
./scripts/cargo clippy --workspace --all-targets --all-features -- -D warnings
./scripts/cargo test --workspace
ME_TEST_DATABASE_URL=postgres://postgres:synthetic-local-test@127.0.0.1:55432/me_accounts_test \
  ./scripts/cargo test -p me-api -- --ignored
```

The PostgreSQL integration test is explicitly ignored unless invoked as above.
It checks concurrent/repeated registration, duplicate rejection, persistence
across reconnects, authentication failures, concurrent recovery, old-code/password
invalidation, malformed input and rate limits. Core tests check key continuity,
data preservation, backup compatibility, wrong-account/corrupted-envelope rejection,
retry staging and absence of raw passwords/codes from registration payloads.

## Remaining release work

Choose hosting/region and deploy behind TLS; add email ownership verification,
account deletion, session/device lifecycle, policy and privacy review, stronger
abuse controls, operational backups/monitoring and an independent security review.
Email reset alone must never release vault keys. A stolen recovery code can recover
the account. Losing both password and recovery code has no backdoor recovery.
Previously copied backups and offline devices remain accessible with their older
passwords; changing a password cannot revoke already-copied keys or data. An old
backup still uses the password it was created with; recovery-aware restore UI is
not included. Linux runtime/accessibility verification remains separate.

Additional local HTTP smoke test (with the disposable API running):
`ME_ACCOUNT_API_URL=http://127.0.0.1:8787 ./scripts/cargo run -p me-app --example account_roundtrip`.
The native `account_gallery` example renders each account state with synthetic
fixtures at normal or `small` minimum size. It requires a fresh temporary
`ME_VAULT_DIR` and `ME_CODEX_BIN=/usr/bin/false`.

Cryptographic implementation references:
[Argon2](https://docs.rs/argon2/0.5.3/argon2/),
[XChaCha20-Poly1305](https://docs.rs/chacha20poly1305/0.10.1/chacha20poly1305/),
[OWASP password storage](https://cheatsheetseries.owasp.org/cheatsheets/Password_Storage_Cheat_Sheet.html).

## Verification on 20 September 2026

The workspace tests, real PostgreSQL integration test, real HTTP desktop-client
round trip, format check, all-target/all-feature compilation, strict Clippy and
design-system guard/regression checks passed on macOS. The native production
registration, sign-in, recovery form, recovery-code handoff, error and completion
views were inspected at 1120×820 and 800×600. Minimum-size scrolling was checked
for the longer forms and error messages; the loading view was checked at 800×600.
Only synthetic credentials and disposable local storage were used. The account-linked
offline-unlock and restored-backup screens were checked in source, but not visually
rechecked. Linux runtime, screen-reader behavior and public deployment have not
been verified.

The ChatGPT browser flow follows the [OpenAI authentication documentation](https://learn.chatgpt.com/docs/auth#openai-authentication).
The recovery-code field now has a typed Copy icon, in-field Copy action and brief
Copied feedback. A separate safekeeping acknowledgement remains required.

## Three-step onboarding verification

The extended registration flow was exercised through the native `onboarding_flow`
harness against the disposable local HTTP API and PostgreSQL, using only synthetic
credentials and a fake ChatGPT provider. It verified recovery-code acknowledgement,
transition to provider setup, interruption/reopening before completion, cancellation,
offline and quota failures, explicit completion after a successful provider check,
and subsequent offline unlock. Provider setup made no model turns.

The complete workspace test suite, schema 11-to-12 migration and existing migration
fixtures, development reset retention, all-target/all-feature check, strict Clippy,
formatting and design-system checks passed on macOS. Registration, recovery-code
handoff and ChatGPT required/success/error states were inspected at 1120×820 and
800×600, including compact scrolling, Copy/Copied feedback and cancellation.
The loading state, native drawing transition and reduced-motion rendering were
also inspected. Linux and screen-reader verification remain outstanding; a real
user ChatGPT login was not completed during testing.

Run the isolated native behavioral harness with a fresh temporary directory:

```sh
ME_VAULT_DIR=/tmp/me-onboarding-check/vault \
  ME_CODEX_BIN=/tmp/me-onboarding-check/fake-codex \
  ./scripts/cargo run -p me-app --example onboarding_flow
```

Set `ME_ACCOUNT_API_URL` to a disposable loopback API to include actual registration
through the desktop HTTP client. Without it, the harness creates a synthetic
account-linked local vault directly. Never point this harness at a real vault.
The `account_gallery` supports `provider`, `provider-working`, `provider-ready`,
`provider-error` and `provider-quota` in addition to its existing account states;
F6 toggles window size and F7 cycles states. Optional `reduced` disables motion.

## Remembered account, logout and registration

ME. remembers one selected account. After setup or online sign-in on this device,
launching or locking the app displays its email and asks only for the master
password. Lock retains selection and closes the encrypted session; Log out closes
the session, cancels account work, clears visible account data and persists a
signed-out state. Logout is available in Settings, on the unlock screen and during
ChatGPT setup. There is no account switcher.

Register remains available from sign-in and unlock, and Settings offers Register
a new account. From a selected account, this first logs out and assigns a fresh
local vault location. Previous accounts’ encrypted vaults are preserved. Signing
in to an earlier account locates its original vault using the authenticated
account/vault identity and backend origin, then selects it again. Recovery after
logout locates and updates the same vault. Remote accounts without a local vault
still show the existing no-sync completion message; no empty replacement is made.

The device-local `device-account.json` stores a version and a validated relative
slot: `primary` for the original vault, or a generated UUID. Signed-out state stores
an empty active selection and a draft UUID so interrupted registration can resume.
It contains no credentials and is saved atomically with mode 0600. Additional
vaults live under `accounts/<uuid>/vault` beside the original vault; each has its
own sibling `codex-inbox` provider home. Motion preferences remain device-wide.
Logout does not delete vaults or sign the browser out of ChatGPT. This release
continues to unlock with a master password; biometric unlocking is not implemented.

Validation includes two real local API registrations through the native harness:
logout, creating the second account, startup restoring its selection, rejecting the
first account’s password for the second vault, and later sign-in/recovery returning
to the original vault with its data intact. Unit tests cover durable selection,
logout, staged registration lookup, separate data, backend-origin matching,
restricted file permissions, path validation and symlink rejection.

The remembered-account unlock screen, signed-out sign-in/Register flow, Settings
account controls and ChatGPT setup logout were visually checked on macOS at
1120×820 and 800×600. Native clicks verified Register navigation and Settings
logout returning to blank sign-in. Linux and screen-reader checks remain unverified.

The lock screen now shares the login/registration card and animated identity
layout. Its remembered email is read-only, and the master-password field receives
initial focus. The native macOS preview was checked at 1120×820 and 800×600 for
normal and error states, with compact scrolling and recovery navigation. Long
email, reduced-motion and busy states were also checked at minimum size. Linux
visual and screen-reader checks remain outstanding.

## Account loading and local timing — 22 September 2026

Account submission and offline unlock retain masked password fields while work
runs. The fields are inert until completion, secondary actions keep their space,
and buttons describe the operation alongside a spinner. Successful transitions
clear the hidden fields; failure preserves them for correction. The native
`onboarding_flow` harness checks this lifecycle with synthetic credentials.

Local account opening now derives the password wrapping key once. The desktop
only performs separate envelope validation for remote-only accounts; local vault
opening authenticates that same envelope inside `unlock_with_header` before
checking the database and replacing its header. No KDF parameters, authentication
checks or encryption settings were weakened. Existing wrong-account, corruption,
recovery, retry and data-preservation checks pass, including a wrong-password
regression against an existing account vault.

The registration test launcher now runs the API in release mode. Measurements on
this Mac used a release desktop harness, temporary vaults and synthetic accounts
against the local PostgreSQL container (not a production latency guarantee):

| Operation | Previous path / debug API | Updated path / release API |
| --- | ---: | ---: |
| Prepare registration/recovery-code step | 179 ms | 179 ms |
| Stage local vault | 98 ms | 102 ms |
| Register HTTP request | 489 ms | 40 ms |
| Sign-in client derivation + HTTP | 294 ms | 88 ms |
| Local account open | 150 ms | 87 ms |
| Extra desktop envelope validation before local open | 68 ms | Removed as duplicate |
| Offline unlock | 71 ms | 76 ms |

A separate three-request authentication probe measured 225–269 ms with the debug
API versus 15–23 ms with the release API. The desktop timing harness is
`account_timing` (run with `./scripts/cargo run --release -p me-app --example account_timing`
and `ME_ACCOUNT_API_URL` pointing to a disposable loopback service); omitting the API variable measures only local
work. It prints durations, never credentials or vault contents. Password hashing
remains intentional work, and real cloud/network and ChatGPT connection timings
will differ.

The permanent local development app and account-data migration are documented in
[Development builds](development.md). Use ME Dev for everyday testing; its fixed
Application Support path avoids keeping the working vault in a source/test folder.
