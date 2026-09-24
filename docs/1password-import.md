# Import from 1Password

Use **Logins → Import** or **Settings → 1Password** to select a `.1pux` export
from 1Password 8. Dropping a `.1pux` file also reaches the import review. The
import confirmation requires a 1PUX export to be selected on its own.

1. Export from 1Password with **File → Export → 1PUX**.
2. Select the export in ME. Parsing and validation run locally in a worker.
3. Review the counts of new, unchanged, changed and archived items, and vaults.
4. Import. Unchanged items are skipped. Changed items are additional versions;
   existing credentials are never overwritten. Favorites become pinned items.
5. Open **Logins** for the list and inline detail/editor. Other categories remain
   available through Search and Browser. Reveal or copy individual fields.
   Values are hidden again after 30 seconds. Copied values expire after 30 seconds
   if the clipboard has not been replaced. Lock clears the view and owned clipboard.
6. Check the imported data, then delete the plaintext export yourself.

The import supports 1PUX version 3. CSV and legacy 1PIF imports are not implemented.
Passkeys are not included in 1Password's desktop exports. OTP secrets/URIs are
preserved as fields; generating codes and browser autofill are separate work.

## Preserved data

All item categories are stored, including unknown categories. Login fields,
standalone passwords, URLs, notes, section/custom fields, password history and tags
are available in item details. Complex field values retain their JSON structure.
The dedicated Login detail shows all fields. The legacy modal for other entry
categories displays the first 200 fields and explains when more are present.
Account and vault identifiers, original timestamps and all unknown fields remain
in the original data. Archived items remain visible and labeled as archived.

The complete original archive is stored in SQLCipher, including document items,
attachments, custom icons and fields that ME does not interpret. Document/attachment
references resolve by either `documentId__filename` (current exports) or the
published `documentId___filename` prefix, or an explicit `files/` path. The original
filename metadata is used for display, including filenames beginning with underscores.
Missing or ambiguous attachments reject the import before any writes.
Attachments can be exported individually. **Gesamten Originalexport speichern**
exports the entire original import, including its other items. The UI explains
that these exports create plaintext files. Existing destinations are never replaced.

## Storage and access

Schema 6 extends collection items with a credential kind. Credential records and
original archives live in the encrypted database and are included in the existing
encrypted backup/restore flow. Their source sensitivity is always `credential`.
They never create assertions, FTS segments, extraction jobs or AI evaluation tasks.
The main Search page finds imported entries by title and vault name alongside
notes, documents and confirmed personal data. Opening a credential result uses
the existing masked detail view. Only titles and vault names are searched locally. Contents are
excluded from the Codex read scope and document extraction APIs.

Deduplication uses account UUID + vault UUID + item UUID + a fingerprint of the
canonical item JSON and referenced attachment bytes. This is deliberately scoped
by vault: 1Password item UUIDs are not globally unique. A changed version is kept
separately. A later repeat of either version is skipped. A complete batch, original
archive and journal entries commit in one database transaction. A failed import
leaves no partial items. Retry by selecting the same file again.

No original export is deleted automatically. No archives are unpacked to disk.
Archive and field buffers are zeroized on drop; renderer/clipboard/JSON-parser and
allocator internals do not guarantee erasure of all transient copies. This remains
an initial implementation, not a security-audited password manager.

Limits: 256 MiB input, 512 MiB declared expanded size, 128 MiB per attachment,
32 MiB export.data, 50,000 ZIP entries and 20,000 items. Invalid JSON, unsupported
versions, duplicate item identities, traversal paths, symlinks, encrypted entries,
missing data and unreadable attachments fail with content-free errors.

## Validation

Synthetic tests cover exact password whitespace/Unicode, custom fields and OTP
secrets, password history, archived/favorite state, attachments and exact original
round-trip, same UUIDs across accounts/vaults, repeat imports, changed versions,
transaction rollback, schema migration, encrypted persistence and backup/restore,
AI/search exclusion, unsupported files and malformed/oversized archives.

Sources: [1Password export instructions](https://support.1password.com/export/),
[1PUX format specification](https://support.1password.com/1pux-format/).

Validation on macOS (2026-09-15): all 48 core tests, workspace Clippy and
compilation passed. Native synthetic smoke checks covered preview, import,
masked/revealed values, automatic re-hiding and locking. The full workspace run
reported one unrelated failure in `payroll_columns_keep_labels_and_values_together`;
it also fails on the live project without these importer changes.

Fix validated on macOS (2026-09-19): an export using the two-underscore attachment
separator previously failed as “missing or ambiguous”. Regression coverage now
checks both delimiters, original filenames/bytes, round-trip recovery, duplicate
imports, ambiguous attachments, and document ID prefix collisions. Import diagnostics
record picker/read/preview/import stages, counts and static failure reasons only;
filenames, vault names, identifiers and credential contents are never logged.

Validation completed on 2026-09-20: all 71 core tests and 2 diagnostics tests pass,
as do the design-system guard/tests, formatting, workspace Clippy with all targets
and features, and the signed release bundle build. The originally failing export
passes local parsing and preview. Native synthetic checks cover preview, commit,
masked details, original attachment names, and a missing-attachment error; the
preview and error screens were also checked at 800 × 600. Sanitized success and
failure events were verified in the diagnostic log.


## Login list and editing — September 23, 2026

The Logins page has three columns: ME. navigation, a virtualized list, and the
selected login. It includes every nondeleted category `001` entry, without the
200-result search cap. All includes archives; Favorites and Archive are filters.
Local search matches login titles and vault names. Vault, archive state, favorite
state, and import-version number distinguish rows. Other 1Password categories are
preserved and available in Search/Browser, without being mislabeled as logins.

Edit supports title, favorite, username, password, existing websites, notes, tags,
and custom fields. Missing username/password/primary website can be filled in.
Save and Cancel are explicit; navigation and file drops keep a draft in place.
Cmd/Ctrl+E edits; Cmd/Ctrl+S saves; Tab and Shift-Tab move through editor fields.
Notes and tags accept hard line breaks. Structured custom values use a JSON editor
that validates JSON and retains the original outer type; unknown fields and field
metadata are preserved. Password history and attachment contents are read-only.
This increment does not add login creation, field creation/deletion, archive-state
editing, OTP generation, browser filling, or passkey support.

Edits update the current encrypted credential record in one transaction, increment
its revision, update the title used by global search, and journal only an empty
payload. A stale revision is rejected without overwriting newer data. Changing
the primary password preserves its exact previous value in password history.
Passwords and notes are never trimmed. Login contents stay outside FTS, document
extraction, and AI scope. Clipboard ownership/expiry and vault-lock clearing remain
in force. Revealed fields hide after 30 seconds, including in the editor.

Original 1PUX archive bytes and import fingerprints remain unchanged by editing.
Re-importing the same export is still a duplicate and keeps the local edit. A
changed upstream item is retained as an additional imported version. “Save
original export” in the detail view exports the original import, not
local changes; encrypted vault backups include current edits.

### Synthetic testing

Run `python3 scripts/generate-login-fixture.py` to regenerate
`crates/me-core/tests/fixtures/synthetic-logins.1pux`. It contains only invented
data: 10 logins, 2 other entries, 3 vaults, archives/favorites, multiple URLs,
Unicode/whitespace passwords, blank fields, notes, tags, OTP/custom fields,
password history, unknown metadata, and one attachment. No real account is used.

For an isolated native preview:

```sh
ME_VAULT_DIR=/tmp/me-logins-preview/vault ./scripts/cargo run --release -p me-app --example logins_gallery -- list
```

The gallery accepts `list`, `edit`, `empty`, `preview`, `error`, or `no-match`, plus
`small` for 800×600. Use a fresh `/tmp` vault for empty/preview states. The gallery
never connects to a provider. The same test vault can be reopened to check saved
edits. Normal user testing can import the fixture through Logins → Import.

### Verification

The full workspace suite passed (172 tests; 7 existing environment-dependent tests
ignored). Strict Clippy for all targets/features, workspace compilation, formatting,
the design guard and its five regression tests passed. The release app and synthetic
native gallery built successfully on macOS.

Native checks at 1120×820 and 800×600 covered list/detail layout, import review and
commit (12 entries yielding 10 Login rows), selecting entries, title/username/password
editing, Save/Cancel, unsaved-navigation protection, field focus and keyboard traversal,
Unicode multiline notes, read-mode reveal and automatic re-hiding, archive/favorite
filters, validation errors and no matching results. Storage tests cover reopening,
encrypted backup/restore, unchanged re-import after local edits, changed upstream
versions, exact secrets, missing/cleared fields, typed custom fields, attachments,
original archive preservation, revision conflicts, rejected writes and a 235-login
list. The personal 1Password export was not accessed. Linux rendering, screen readers,
live passkeys/filling, and transient loading/file-error UI states remain unverified.


## Login detail presentation

The 1PUX `passwordHistory[].time` records when a password became non-current,
as documented in the [1Password export format](https://support.1password.com/1pux-format/).
ME. displays this as “Used until” with UTC date/time and an elapsed age, newest
first. Missing or invalid timestamps stay unknown. Editing the current password
records the old password's retirement timestamp in the encrypted item.

Tags and account labels are compact chips; counts are ordinary visible metadata
unless explicitly guarded. These fields have no redundant Copy action. Editable
values retain their original export types and original archives remain unchanged.

Website logos load directly from the saved site's public HTTPS origin while the
vault is unlocked. Only the origin is derived from the login: saved URL paths,
queries, fragments and embedded credentials are discarded. The loader reads the
public homepage for same-origin icon links, with conventional icon fallbacks;
it does not follow redirects, use cookies or authentication, contact third-party
icon lookup services, or fetch local/private destinations. DNS results are checked
and pinned to public addresses. Network and raster decoding run on background
workers (four at most), with request time/size and image-dimension limits. SVG and
animated artwork are not rendered; decoded images are downscaled to one frame.

A 256-site memory cache includes failed lookups and is cleared on lock or when
Website icons is disabled in Settings. It leaves no domain-named files on disk.
The website still receives the user's IP address and an icon request. Restricted,
redirect-only, unsupported and offline sites retain the honeycomb key fallback.
The setting is device-local and survives changes to Reduce motion. No icon data
is placed in the vault search index or shared with AI.

Use `logins_gallery list live-icons` for an optional public-network check using
synthetic GitHub and Google entries. Other synthetic export domains remain
reserved and are never requested. The default fixture stays fully offline.


Validation (23 September 2026): the macOS release preview was inspected at
1120×820 and 800×600 with synthetic accounts. Public GitHub/Google logos loaded
in both list and detail, and disabling the setting restored the honeycomb key.
Checks covered long wrapping tags, keyboard editing, saving a numeric count and
a tag, masked history with retirement dates/age, and compact Settings wrapping.
Toggling Reduce motion retained the disabled icon preference in the saved file.
All 20 desktop tests, 39 core tests, five design-guard regressions, formatting,
workspace/all-target/all-feature compilation and strict Clippy passed. Targeted
login regressions also passed after the numeric validation wording was refined.
The macOS bundle was rebuilt and signed. Linux rendering, screen readers, and
exhaustive favicon compatibility across other websites remain unverified.

Native credential creation and local context-assisted drafts are documented in
[credential-creation.md](credential-creation.md). Original import archives remain
unchanged when recovery codes or edited passwords are saved in ME.
