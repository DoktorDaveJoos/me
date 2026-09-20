# ME. — personal data filter

The product is **ME.**, always uppercase with a period. The English interface
preserves source filenames and data in their original language, including German.

## Home

A persistent, narrow left sidebar contains Search, Review, Browser, and Settings.
The start page reads **It's about you. Your data. Your fingerprints.** It contains
one large search field and copyable data rows. Empty search shows recently found
or copied details, stored in the encrypted vault. A new vault shows available
confirmed details until the first search.

Typing replaces recent rows with local matches across notes, documents (filenames
and enabled content), and imported credentials (titles and vault names). Item rows
open the existing note, document, or masked credential view. Confirmed details keep
their copy action and source; a matching note appears only once. Local search works
without AI and refreshes after imports and edits. After a short pause,
AI interprets the intent and chooses keys from the existing field catalog. It does
not generate answers or values. English and German aliases work locally; semantic
matching adds broader intent matches without removing local results. Copyable
details always resolve to current confirmed vault values. Conflicting values remain visible and labeled. Copy buttons copy
only the stored value, with the existing clipboard expiry behavior. Source links
open the underlying detail. Enter retries the semantic filter; Cmd/Ctrl+K focuses
search. Lock clears in-memory data and cancels outstanding work.

## Dropped forms

Drop, paste, or pick a document in Search. ME. stores the original, extracts text
locally, then asks the connected model which blank fields need filling. This is
an explicit request to inspect the attached document, including when automatic
background analysis is disabled. Source text and field labels go to the existing
ChatGPT-backed connection. Stored values stay local during field matching.

Detected fields must cite an exact source quote. Each one shows matching confirmed
values or **Not found**. PDF text/OCR is supported; macOS also reads widget names
and whether fields are empty. Form inspection is limited to eight attachments,
120,000 bytes of combined extracted text, and 100 requested fields. Large or
unreadable forms show a recoverable error. There is no silent truncation.

**Review handoff** previews the exact unambiguous values to export. After the user
chooses a destination, ME. creates a private working folder with original form
copies, `matched-data.json`, and `HANDOFF.md`. It excludes unrelated data,
credentials, and conflicting values. These working files are unencrypted, which
the review explains. The original vault files are unchanged.

On macOS, the supported `codex app PATH` command opens that folder. ME. copies a
task prompt; the user pastes it in Codex to start. This does not automatically
create or run a task. Codex can use PDF tools or computer use where available; it
must ask about missing data and return a new filled copy for review. Signing,
submission, and transmission are not authorized by the handoff. If launch fails,
the saved folder and prompt remain usable. Linux exports the same handoff for
manual opening in Codex.

## Other views

Review retains individual and atomic batch approval. Browser retains the
AI-organized, expandable virtual directory tree. Settings retains connection,
analysis preferences, backups, locking, and on-demand 1Password import. Imported
credentials remain in the local Passwords branch and are excluded from AI.

## Validation and limits

Schema 8 adds encrypted recent-detail references. Tests cover multilingual
matching, persistent recents, stale/retracted data, unknown AI keys, grounded form
fields, conflict handling, minimal exports, and PDF widget extraction. Native UI
checks use an isolated synthetic vault and fake model/launcher; they do not
establish live model quality, computer-use availability, or Linux UI behavior.

References: [Codex workspace launch](https://learn.chatgpt.com/docs/developer-commands?surface=cli),
[PDFKit field names](https://developer.apple.com/documentation/pdfkit/pdfannotation/fieldname),
[PDFKit widget values](https://developer.apple.com/documentation/pdfkit/pdfannotation/widgetstringvalue).

### All-entity search validation — 2026-09-20

The main Search view queries the local collection on a background worker, with
a short debounce and request/session guards to discard stale results. This finds
existing imports without migration or re-import. Filename matching also covers
documents classified as credentials; their contents remain excluded. Credential
payloads remain outside FTS, AI field catalogs, and document processing.

All 73 core tests pass, including mixed collection kinds, five imported credential
categories, archived entries, prefix/case matching, persistence after unlocking,
deleted-item exclusion, secret-content exclusion, and additive semantic matching.
Formatting, workspace compilation and Clippy (all targets/features), the design
guard and its five tests pass. The macOS release bundle was rebuilt and signed.

The synthetic search gallery was inspected at 1120×820 and 800×600 with AI
disconnected. Instagram returns a login, document and one copyable note; their
open actions reach the masked credential panel, document panel and note editor.
Long filenames truncate, all rows remain reachable by scrolling, clearing restores
personal details, and an unmatched query shows the empty state. Loading/error
and rapid-query cancellation paths were inspected in code; Linux and live AI
filtering were not visually exercised. No personal vault was opened.
