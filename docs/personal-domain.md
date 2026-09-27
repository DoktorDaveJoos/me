# Personal knowledge, authority and replication foundation

Schema 14 evolves the existing SQLCipher vault. It preserves old IDs, sources,
decisions, imported archives, credential contents and backups. Legacy document
fields retain their meaning; migration never guesses a new person or contract.
Older application builds cannot open a schema-14 vault. Retain a pre-upgrade backup.

## Implemented behavior

`me-core` owns typed entity and assertion APIs, exact decimal/money values,
validity intervals and as-of resolution. Entity-valued assertions are graph edges
with the same evidence and review semantics as other facts. Scoped identifiers
can return several candidates; equal names never automatically merge entities.
Explicit same-kind merges preserve original assertions and can be reversed.
Soft-deleted entities retain their history and have an explicit restore API.

Intervals use an inclusive start and exclusive end. Recorded time and effective
time are separate. The resolver distinguishes missing, resolved, multiple,
conflicting and uncertain values. Unknown validity remains uncertain in the new
resolver; legacy profile APIs retain their previous unknown-date behavior.
The Knowledge map retains dated history and only marks incompatible overlapping
single-value assertions as conflicts. Search/profile readers exclude facts that
are not effective on the current UTC date. Arbitrary custom properties remain
available. Existing property type/cardinality cannot be silently replaced.

Every vault installs a shared property vocabulary (`PERSONAL_VOCABULARY`) on
creation, unlock and development wipe. It adds missing keys only and never
replaces a user's existing definition. It covers residence, employer, income,
ownership, address, vehicle, contract and insurance terms, so people, extraction
and agents describe the same relationship with the same key. Definitions can
restrict subject kinds and linked entity kinds; writes that link the wrong kinds
are rejected. Redefining a key with different kinds is refused like any other
change of meaning. Money can carry a recurrence period (day to year); an amount
without a period stays a one-off or total, and older stored amounts are unchanged.

Entity-valued facts resolve to the linked entity's current merged identity.
Links to a deleted entity are omitted rather than returned as dangling IDs.
Resolving an undefined property is an error, not a missing value.
`entity_context_at(root, as_of, depth)` returns the accepted graph around an
entity on one date: reached entities with their link distance and the resolved
status of each of their properties. It follows links in both directions, so a
person reaches the contract that insures their car. Depth is limited to 4 and
the result to 256 entities, with an explicit truncation flag. This is a trusted
core API; it is not an MCP tool and no grant covers it yet.

Every verified or unverified extraction candidate now creates an immutable
observation with its run, original wording, subject/context and source locator.
Separate runs remain distinct even when proposal deduplication suppresses another
review card. `resolve_observation` explicitly records a user's choice of entity,
property, typed value and validity. The resulting user-authored evidence points
back to the observation as context; a corrected value never inherits a fabricated
verbatim document quotation. Repeating the same mapping is idempotent.

Native and imported credential writes automatically append protected payload
versions in the same transaction. Credential identities can be linked to entity
and service identities. A version preserves the entire protected payload,
including unknown imported fields; the original archive and existing secret
history remain intact. Versions are a coarse conflict unit. Per-field merging,
passkeys and OAuth operations are separate future work.

Core capability APIs bind a grant to principal, task, exact resource, operation,
expiry and issuing device. Field reads, source reads, draft creation and the
broker-only credential API recheck authority and record an audit event. A
credential-use grant is bound to a normalized HTTPS origin and does not grant a
secret-reveal operation. Entity merges do not widen an existing field grant.
Derived assertions are withheld by field-grant reads until input-level authority
is implemented. Permission and audit records never automatically include secret
values. Trusted application code issues grants and approvals; these APIs are not
exposed as model tools.

Action intents, immutable draft revisions, exact-revision approvals and execution
attempts are separate records. Editing invalidates previous approvals. Attachments
are rechecked before reserving an attempt. A revision can be attempted only once
locally; interrupted attempts become indeterminate on unlock and cannot be blindly
retried. No external mail/API executor was added. Cross-device execution will also
require an online coordinator or equivalent execution authority.

## What the application uses now

Existing note, credential, extraction and deletion workflows automatically gain
domain history; imports also gain observations and credentials gain protected
versions. Typed canonical facts are displayed in the existing collection and
Knowledge map. The legacy free-text editor refuses to rewrite typed, dated or
other-entity assertions as self-profile text.

Entity editing, observation-to-entity mapping, permission requests and action
approval have tested core APIs. Their complete desktop workflows and automatic
entity-resolution suggestions are not yet present. The current MCP bridge keeps
its explicit read-only source-snapshot contract; it does not gain credential or
execution tools. This migration does not silently expand agent access.

## Replication contract

Authoritative writes are captured by database triggers in the same transaction
as their domain mutation. Existing write paths therefore participate without
depending on a future network worker. Each revision has an opaque globally unique
ID, stable record identity, parent revision IDs, operation, full record payload,
format version, logical key scope, actor, originating device and recorded time.
Deletes are retained as delete revisions; soft deletes are full revisions with
their deletion state. A migration bootstraps current records once. Historical
assertions and decisions remain individually addressable.

Device-local `collection_item.local_id`, decision sequence numbers, head pointers,
search indexes, layout/view state, recent-use state and worker progress are not
the shared domain protocol. Foreign item references are exported as stable IDs.
Decision payloads preserve the previous decision's ID. Legacy revision integers
are compatibility metadata, not a cross-device ordering rule. Personal, credential
and authority scope tags are routing intent within encrypted payloads; they are
not separate cryptographic key domains yet.

`encrypted_revisions(after, limit)` exports up to 100 immutable revisions, aiming
for at most 8 MiB of plaintext per batch; one larger record travels alone rather
than becoming permanently unexportable. It returns a local database cursor for
resumption. The cursor must never be used to compare different devices. Each
payload uses the existing XChaCha20-Poly1305 implementation with vault/revision
identity authenticated as associated data. Ciphertext is cached so retries of
the same revision are byte-identical. All routing-sensitive payload contents are
encrypted. This is a whole-vault export using the current object key, not a
reviewed selective-client key hierarchy or a sender-signature protocol.

Original files and imported credential archives are separate encrypted assets via
`encrypted_replication_asset`. Records refer to their stable object/archive IDs;
raw archives are not copied into every record revision. Content indexes can be
rebuilt locally. Source-relative evidence and observation quotations persist even
when derived search chunks are rebuilt. Remote projection/application rules still
need to address compatibility proposal segment references.

The vault-local `device-id` file is excluded from backups. Restore retains record
identities and ancestors but creates a new device identity; normal reopen retains
the same identity. These replica IDs are bookkeeping, not cryptographic device
authentication. Existing device-bound capabilities are ineffective on a restored
device. Synthetic fork tests confirm that independent edits preserve a common
parent, different revision IDs and different device IDs.

**This foundation does not perform live sync.** Server upload/download endpoints,
device enrollment, key distribution/rotation, authenticated incoming revisions,
causal conflict application, receipt/acknowledgement state and cross-device action
coordination still require the sync design. Incoming data is not automatically
applied, and there is no last-writer-wins merge hidden behind the local views.
Revision history currently has no compaction policy. Explicit development wipe
clears local domain history along with content; it must not be treated as a
production synchronized delete operation.

## Verification

The domain suite covers exact values/time, entity matching and reversible merges,
observation provenance and retry behavior, schema-13 migration, existing credential
history, atomic rollback, stable encrypted retries, restored-device ancestry,
expired/revoked/destination-scoped access, restricted evidence, merge-scope limits,
draft/execute separation, stale approval and interrupted execution.

A synthetic car-insurance household verifies that one context read from the
person returns residence, employer, monthly income, vehicle make/model/mileage,
the insuring contract, its monthly premium and its provider; that sold vehicles
and future contracts follow the requested date; and that merges and deletions
never leave stale links.

Run `./scripts/cargo test -p me-core domain_tests --locked`. For the synthetic
native view, run the `knowledge_gallery` example with `domain` (and optional
`small`), a fresh temporary `ME_VAULT_DIR`, and no live provider credentials.
All behavioral fixtures use synthetic data. Schema and API validation is not an
independent security review or proof of cross-device synchronization.

## Verification completed on 27 September 2026

The full workspace test run passed; eight tests requiring live services or an
external PostgreSQL instance stayed ignored. The final core suite and all 16 new
domain tests passed. Workspace/all-target/all-feature compilation and strict
Clippy passed, as did formatting, the design-system guard, its five regression
tests and 17 packaging/signing regression tests.

The synthetic domain gallery was inspected at 1120×820 and 800×600, including
entity relationship labels, selection, exact monetary values, historical and
future validity text, and absence of false temporal conflicts. No personal vault
was used for these checks. Linux rendering and accessibility remain unverified.

ME Dev was updated with `./scripts/dev-macos`. The installed bundle passed its
pinned identity and signature checks, launched successfully, and its own privacy
panel reported both Window previews and Read and fill selected windows as Allowed.
The personal vault was left locked; its migration runs on the next successful unlock.
