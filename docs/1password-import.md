# Import from 1Password

In **Einstellungen → 1Password**, select a `.1pux` export from 1Password 8.
The add dialog also offers **Aus 1Password importieren**, and dropping a single
`.1pux` file opens the same review flow. Mixed drops are rejected before intake.

1. Export from 1Password with **File → Export → 1PUX**.
2. Select the export in ME. Parsing and validation run locally in a worker.
3. Review the counts of new, unchanged, changed and archived items, and vaults.
4. Import. Unchanged items are skipped. Changed items are additional versions;
   existing credentials are never overwritten. Favorites become pinned items.
5. Open an imported item from the collection. Reveal or copy individual fields.
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
The detail view displays the first 200 fields and explains when more are present.
Account and vault identifiers, original timestamps and all unknown fields remain
in the original data. Archived items remain visible and labeled as archived.

The complete original archive is stored in SQLCipher, including document items,
attachments, custom icons and fields that ME does not interpret. Document/attachment
references resolve by the documented `documentId___filename` prefix or an explicit
`files/` path. Missing or ambiguous attachments reject the import before any writes.
Attachments can be exported individually. **Gesamten Originalexport speichern**
exports the entire original import, including its other items. The UI explains
that these exports create plaintext files. Existing destinations are never replaced.

## Storage and access

Schema 6 extends collection items with a credential kind. Credential records and
original archives live in the encrypted database and are included in the existing
encrypted backup/restore flow. Their source sensitivity is always `credential`.
They never create assertions, FTS segments, extraction jobs or AI evaluation tasks.
Only titles and vault names are searched in the local collection. Contents are
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
