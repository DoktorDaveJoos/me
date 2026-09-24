# ME accounts, independent clients, encrypted sync, and login matching

Date: 2026-09-20.
Status: confirmed product direction and agreed architectural direction; not an
implementation or security certification. Rust + Axum + Tokio was subsequently
selected for the backend; supporting providers and detailed protocols remain open.

This records the product discussion following the 1Password import work. It
supersedes earlier assumptions that the Chrome extension might depend on a local
desktop bridge, and that accounts and cross-device access can be postponed while
building a complete login experience. The September 15 sync research remains
useful background where it does not conflict with this decision.

## Confirmed product requirements

1. Every ME user must have a ME account, including users of the planned free,
   desktop-only tier. Initial account setup requires connectivity. This product
   direction does not itself change the repository license or release terms.
2. Desktop on macOS/Linux, a standalone Chrome extension, and an iPhone app are
   part of the intended product family. Each connects independently to ME Cloud.
3. The Chrome extension must not communicate with the local desktop app for its
   vault access or filling workflow. Desktop installation and availability are
   not prerequisites. The iPhone also works with the desktop switched off.
4. Cloud synchronization connects the clients. Previously downloaded information
   remains locally usable after setup and unlocking, including while offline or
   when cloud sessions expire. Sync resumes after connectivity/authentication is
   restored; the UI must distinguish local changes from uploaded changes.
5. Existing users attach their current vault to an account through a migration
   that preserves their data. Another 1Password import must not be required.

## Information types and protections

Keep one collection with meaningful types and simple filters: All, Personal
information, and Logins & secrets. Tasks, events, and reminders have their own
action/time semantics when introduced.

A login contains the service, account identity, and authentication methods. A
password is a field of that login; standalone secrets can also exist. Preserve
1Password item categories, including cards, identities, documents, and secure
notes, instead of labeling every imported entry as a password.

Sensitivity and permitted use are separate from item type. Moving an imported
item into Personal information must not silently authorize AI access, broad
search, or form filling. Keep existing protections until an explicit policy
allows a use. Passkey support requires its own implementation; it does not appear
automatically through 1PUX import, sync, or password filling.

## Accounts, device enrollment, and encryption

Account authentication governs identity, sessions, device registration, and plan
access. Vault unlocking governs access to encrypted personal information. These
are separate operations even when the interface makes them feel continuous.

Vault contents are encrypted on an authorized client before upload and decrypted
on an authorized client after unlocking. This includes login titles, usernames,
URLs, notes, attachments, and saved website/app associations. The sync service
receives ciphertext and the minimum account/routing/version metadata it needs.
Server storage encryption alone is insufficient for this requirement.

New devices need account authorization plus a secure way to obtain the permitted
vault keys, such as approval by an enrolled device or a deliberately configured
recovery mechanism. Email-based account reset must not automatically decrypt the
vault. Key wrapping, recovery after losing every device, device revocation,
rotation, and migration of the existing vault need a reviewed specification.
Use established cryptographic constructions and libraries.

Clients may receive scoped collections; an extension does not automatically need
every document. Cryptographic scope requires corresponding key separation, not
just hidden interface rows. Revoking a device prevents future authorized access
and may require key rotation; it cannot erase data the device already copied.

## Sync behavior

- Synchronize versioned records and immutable encrypted file objects. Do not use
  copying live SQLCipher database files as the multi-device update protocol.
- Preserve stable item/operation IDs, schema and key versions, parent revisions,
  authenticated content, and resumable receipt positions.
- Commit local edits and pending upload records together. Commit received edits
  and receipt positions together. Repeated requests must not duplicate changes.
- Preserve conflicting edits. Concurrent changes to the same password retain
  both versions and require resolution; timestamps alone do not choose a winner.
- Synchronize deletion markers so an old offline client cannot resurrect items.
- Synchronize approved website/app associations with their login.
- Keep recoverable history/backups separately: sync also propagates mistakes.
- Large attachments can download on demand or be explicitly kept offline.
- Mobile scheduling and suspended browser workers prevent a promise of constant,
  instantaneous background synchronization. Clients catch up when available.

## Login matching and filling

Use this order: established website/app match, user-approved association, local
name/metadata candidates, TypeSafe-assisted suggestions for unresolved cases,
then manual search or an explicit no-match result.

Example: an imported item is named Instagram but has no website. On the genuine
Instagram origin, ME can suggest that item by name. The user confirms the first
association; ME stores it with the login and syncs it. Other clients then use the
approved association without repeating AI inference.

An AI suggestion never independently authorizes a new destination to receive a
secret. Validate the actual browser origin or app identity, show the intended
destination, and bind filling to it. Branding, app names, and page text alone are
insufficient. Recheck the target immediately before filling. Filling and form
submission are separate actions.

TypeSafe receives only the limited metadata authorized for the matching request,
with bounded candidate identifiers and a no-match option. Passwords, OTP seeds,
recovery codes, and private keys stay outside model prompts. Even titles and URLs
can be sensitive. End-to-end sync encryption does not extend to metadata disclosed
for a cloud AI request. The sync server cannot search or infer over plaintext
vault contents that it cannot decrypt.

Browser filling belongs in the standalone browser extension. iPhone filling uses
a native ME app with Apple's AutoFill Credential Provider extension. macOS can
use the system credential-provider integration and an Accessibility-based
fallback; Linux app filling needs an explicit desktop support matrix. Screen or
computer-use assistance may help locate unusual controls, with local secret
injection and destination checks. Universal app support is not yet established.

## Technical direction and implementation sequence

Current local SQLCipher storage can remain. The proposed cloud persistence is
PostgreSQL for accounts, devices, and encrypted versions, plus object storage for
encrypted files. The browser uses its own encrypted cache. Share portable protocol
definitions and test vectors across clients, without giving the backend vault
decryption capabilities. Rust + Axum + Tokio is the selected backend stack.
Specific identity, sync, deployment, and storage providers remain unselected.
See the [proposed repository and release model](REPOSITORY-AND-RELEASES.md).

Suggested sequence:

1. Account and device/key/recovery design, including existing-vault migration.
2. Account service and encrypted sync with offline/conflict/recovery tests.
3. Standalone Chrome client and deterministic login matching/filling.
4. iPhone client, AutoFill, and compatible offline storage.
5. TypeSafe-assisted unresolved matching and expanded desktop app filling.

This note records direction. The subsequent [account setup implementation](../../accounts.md)
adds registration, sign-in and recovery with one master password and one vault.
Sync, browser and iPhone clients remain unimplemented.

## Open product and engineering decisions

- Whether credential sync is free or paid; document-storage and AI allowances.
  Independent browser + desktop access requires sync even on the same computer.
- V1 uses email plus one master password and a recovery code; no separate app-unlock
  password. Email verification, account deletion and production identity review remain open.
- Exact key hierarchy, enrollment/recovery protocol, threat model, and review.
- Supporting backend components and hosting region/provider; see [backend selection](BACKEND-OPTIONS.md).
- Retention, conflict resolution UX, backups, quotas, and support boundaries.
- Permission and cost controls for TypeSafe metadata requests.

## References

- [Earlier sync investigation](../2026-09-15/SYNC-ENTSCHEIDUNG.md).
- [Existing 1Password import behavior](../../1password-import.md).
- [Apple credential provider extensions](https://support.apple.com/guide/security/credential-provider-extensions-sec6319ac7b9/web).
- [Chrome extension storage and lifecycle considerations](https://developer.chrome.com/docs/extensions/reference/api/storage).
