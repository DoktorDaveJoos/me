# Knowledge map

Knowledge in the sidebar opens a native GPUI honeycomb on a continuous two-axis
canvas. It is a local view of the vault’s canonical records, not a second store of
personal values. Search, select, copy, edit, source opening and Review reuse the
existing app workflows. Adding or updating data refreshes the visible map.

## Coverage and attribution

- Live manual details keep their collection identity and current value.
- Imported documents are source cells. Confirmed extracted facts are deduplicated
  by assertion identity, even when the collection also exposes them as notes.
  Every live supporting document retains a connection, quotation and source locator.
- Verified proposals awaiting ownership confirmation and unverified suggestions
  remain visible with different explicit states. A manually answered question has
  a context-only link to its document; an unverified model quote is never evidence.
- Accepted entity relationships, source attachments and source revisions appear
  when their endpoints are live. Relationships use existing recorded evidence;
  spatial proximity alone never asserts a relationship.
- Credential entries show only title, category and vault grouping. Protected fields
  remain behind the existing credential detail view and are excluded from the graph
  projection and map search. Restricted documents show metadata only.
- Conflicting accepted single-value properties are marked. Scoped document fields
  and properties that allow many values are not treated as false conflicts.

The map shows current data, rather than every historical assertion or audit event.
Source contents remain available through Open. Rejected, retracted, deleted and
purged records are omitted. The map is not included in agent tools or cloud requests.

## Stable placement

Schema 11 adds `knowledge_position` and `knowledge_view` inside the encrypted
SQLCipher vault. Positions store stable node keys, cluster identity and integer
axial coordinates `(q,r)`, with a unique cell constraint. The viewport stores
center, zoom and selected node. Layout caches contain no copies of data values.

Placement is deterministic and incremental. Existing positions are fixed constraints.
New cells minimize distance to linked cells and their group while penalizing foreign
group crowding. Documents are placed first; new related facts inherit their saved
source neighborhood. Existing folder groups and credential vaults provide clusters;
manual details without recorded relationships share Your details. Groups have
breathing room. No force simulation or random packing runs on reload.

Confirmation transfers a suggestion’s saved cell to its canonical fact when that
fact does not already exist. If the fact already exists, it retains its position
and gains the additional source connection. Deleted cells are freed without moving
survivors. Folder changes do not move previously placed cells. Normal refreshes
reconcile metadata while preserving positions. There is no automatic global repack.

Migrations run on unlock. Backup and restore include the saved map, and the development
wipe clears both tables. Locking discards the in-memory graph; asynchronous callbacks
cannot reinstall a graph from an old vault session. View changes save on a debounced
vault worker, with additional saves when locking or quitting.

## Navigation and rendering

- Drag the background or scroll in either axis to pan; Shift-wheel pans horizontally.
- Use +/− controls or Control/Command-wheel to zoom. At low zoom, compact symbols
  replace per-cell text. The selection panel retains the full value.
- Search matches labels, displayed values, context and groups locally; Next match
  centers each result. Group controls return to a neighborhood.
- With map focus, arrow keys select nearby cells, Enter opens the item and +/− zoom.
- Select a cell to highlight its related border paths. Expand connections for
  relationship text, source location, verified quotation and source opening.

Only visible cells are laid out and tessellated. Border paths run through the
honeycomb’s degree-three vertex lattice, with bounded A* on a background worker.
Very distant connections that exceed its work bound remain available in the
connection panel. Background vault projection, reconciliation and camera saving
keep database work off the UI thread.

## Reproducible verification

`cargo run -p me-app --release --example knowledge_gallery` requires a fresh
`ME_VAULT_DIR` under `/tmp`. It creates only synthetic data, with Work/Home/Personal
source groups and manual details. Pass `small` for 800×600; the default is 1120×820.
Use `empty`, `loading` or `error` as the first argument for those states, optionally
followed by `small`. Never point a gallery at a real vault.

Core tests cover stable refresh/edit/add/unlock/backup behavior, confirmation alias
transfer, multiple sources, uncertainty, conflicts, credential metadata isolation,
migration, deletion, camera validation and wipe. Native geometry tests check border
routing and axial conversion.

Validation on macOS: the complete offline workspace suite passed (148 tests;
6 existing live-service tests intentionally ignored), as did workspace/all-target
Clippy with warnings denied, formatting, the design-system guard and its five
regression checks. The normal desktop release executable was built successfully.
Synthetic native screens were inspected at 1120×820 and 800×600, including long
value truncation, selected navigation, empty/loading/error states, direct selection,
local search with and without matches, source expansion, Add, and compact zoom.
The underlying data tests verify edits and insertions without moving existing cells.
Linux visual checks and a full keyboard/screen-reader regression were not performed.
