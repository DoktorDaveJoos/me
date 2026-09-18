# ME. — personal data filter

The product is **ME.**, always uppercase with a period. The English interface
preserves source filenames and data in their original language, including German.

## Home

A persistent, narrow left sidebar contains Search, Review, Browser, and Settings.
The start page reads **It's about you. Your data. Your fingerprints.** It contains
one large search field and copyable data rows. Empty search shows recently found
or copied details, stored in the encrypted vault. A new vault shows available
confirmed details until the first search.

Typing immediately replaces recent rows with local matches. After a short pause,
AI interprets the intent and chooses keys from the existing field catalog. It does
not generate answers or values. English and German aliases work locally; semantic
matching handles broader intent. Results always resolve to current confirmed
vault values. Conflicting values remain visible and labeled. Copy buttons copy
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
