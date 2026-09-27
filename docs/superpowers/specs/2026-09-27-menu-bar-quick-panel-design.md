# Menu bar quick panel, background lifecycle and single instance

Date: 2026-09-27. Status: approved design, not implemented.

## Goal

ME. behaves like a menu bar app on macOS: it keeps running after its main
window closes, runs once per app identity, and its honeycomb status item opens a
quick panel for search and fast intake of documents and notes.

## Decisions

- Closing the main window does not quit. Quit only via "Quit ME." (status item
  menu or app menu) or Cmd-Q.
- Dock icon only while the main window is open (accessory policy otherwise).
- One instance per app identity (ME., ME Dev, ME Preview are separate).
- Locked vault: the panel unlocks with the master password in place.
- Panel intake: dropped files and typed text. Dragged text from other apps is
  out of scope.
- Architecture: approach A. The existing `MeApp` entity outlives its window and
  is shared by the main window and the panel. No session extraction refactor.

## Components

### 1. App lifecycle (`apps/desktop/src/main.rs`, new `lifecycle.rs`)

- A global holds the single `Entity<MeApp>`, created once at startup.
- `open_main_window(cx)` reuses an open main window (activate) or opens a new
  window whose root view is the kept entity. Used at startup, by "Open ME.",
  by the panel's "open in ME." actions and by the single-instance "show" request.
- The `on_window_closed` quit rule is removed. When the last main window closes
  the app switches to the accessory activation policy; opening one switches back
  to regular. The policy switch is macOS-only and lives in `status_item.rs`
  (the only module allowed `unsafe`); Linux keeps default behavior.
- Linux has no tray yet: on Linux, closing the last window still quits, so the
  process never runs invisibly without a way back.

### 2. Single instance (`apps/desktop/src/single_instance.rs`)

- At startup, before opening any window, bind a Unix domain socket at
  `<app data dir>/me.sock` (the directory that already holds the vault, which is
  separate per app identity).
- Bind succeeds: this is the primary. A background thread accepts connections
  and forwards each valid `show` line to GPUI through a channel; the foreground
  task calls `open_main_window`.
- Bind fails with address in use: connect and send `show\n`. If the connect
  succeeds, exit with status 0 without opening a window. If the connect fails,
  the socket is stale: remove it, bind again and continue as primary.
- The socket file gets mode 0600. Only the literal `show` command is accepted;
  anything else is ignored. No other data crosses the socket.
- The existing vault file lock remains the data-integrity backstop.

### 3. Status item (`apps/desktop/src/status_item.rs`)

- Left click toggles the quick panel. Right click (or control-click) shows the
  existing menu: Open ME., Lock ME., Quit ME.
- The item reports its button's screen frame so the panel can be placed directly
  below it, clamped to the screen's visible frame.

### 4. Quick panel (`apps/desktop/src/quick_panel.rs`)

- A GPUI pop-up window (no title bar, not in the Dock or window list) with a
  fixed layout width defined as layout geometry, not a spacing token.
- Its view holds a handle to the kept `MeApp` entity and calls narrow
  `pub(crate)` methods on it: unlock, search query/results, `accept_documents`,
  pending-import confirmation and `save_note`.
- Closes on Esc, and when the panel window loses activation.
- States:
  - Locked: master-password field and unlock action; errors reuse the existing
    unlock messages. Busy state while key derivation runs off the UI thread.
  - Unlocked: search field focused on open, top results below (click copies
    the value; a secondary action opens the item in the main window), and a
    drop zone that also accepts typed text.
  - Files dropped: shown as pending with one "Add" action; nothing is imported
    or released to AI before that confirmation, same as the main window.
  - Text entered: saved as a note via `save_note` on Enter/"Save"; the first
    line becomes the title.
  - Result feedback: a short confirmation line after save/import; failures stay
    visible until dismissed.
- Uses only design-system roles, `space::*`, `radius::STANDARD`,
  `type_style(Type::…)`, theme components and bundled icons.

## Error handling

- Status item or panel creation failure: log without personal data; the main
  window remains fully usable.
- Socket errors other than address in use: log and continue as primary without
  single-instance handoff.
- Vault `InUse` stays a visible error if another process holds the vault.

## Security limits

- The vault stays unlocked while the main window is closed, until "Lock ME." or
  quit. Auto-lock (sleep, screen lock, idle) is not part of this change and
  should follow soon.
- Search results in the panel are vault data; the panel is closed and its view
  state cleared on lock.
- This does not change the unaudited status of the vault.

## Testing

- Unit: socket handoff (primary receives `show`, second exits), stale socket
  recovery, rejection of unknown commands.
- Behavioral: panel note saving creates a note; dropped paths become pending,
  not imported, until confirmed; lock clears panel state.
- Existing honeycomb raster test stays.
- Manual on `ME Preview.app` at normal and minimum sizes: close window keeps
  process and icon; Dock icon hides and returns; second launch shows the first
  instance; panel locked, unlocked, search, drop and note flows.
- Required checks: design-system guard and tests, fmt, check, Clippy.
