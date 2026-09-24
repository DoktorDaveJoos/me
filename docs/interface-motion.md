# Native interface motion

The honeycomb is a recurring visual motif, drawn with GPUI paths. The startup
field grows inward from the window perimeter with deterministic pen directions,
indigo drawing tips and rounded cells. Account screens fade every stroke inward,
including animated tips, and add small center dots and isometric cube edges.
The large account fingerprint follows a finite orbital entrance and settles half
behind the form card, without a seal or constellation. The form remains usable
throughout. Legacy local-vault/restore views keep the small fingerprint seal.
Sidebar and workspace fields fade smoothly inward from their outer corners,
including drawing fronts and busy highlights. They settle into quiet linework. Navigation, page changes,
focused text fields and dialogs have short finite transitions. No new dependency
or data schema is required.

Knowledge connections reveal from the selected endpoint along the existing
border routes. Selection draws an inset; new cells fade into their persisted
positions. Refreshing identical data does not restart cell arrivals. Selecting
the current node does not restart its connections. Pan and zoom remain immediate.
Only active Knowledge transitions request new frames, and shared GPUI animations
run once; decorative effects do not run a perpetual idle loop. During account submission or workspace loading, a clockwise highlight follows
the existing honeycomb lattice. Account actions also show a small spinner. Both stop as soon as work finishes; reduced motion
omits the traveling highlight and shows a static spinner arc.

Appearance preference `reduce_motion` lives in `interface.json` beside the vault.
It contains no vault content. Reading and atomic replacement run off the UI
thread. The UI waits for this preference before playing the startup entrance.
Malformed preferences conservatively disable motion and display a recoverable
error. A failed save restores the previous setting. Reduce motion is available
in Settings in every build; its floating pre-unlock shortcut is debug-only and
absent from release builds. It is explicit rather than imported from the OS.
Account activity honors this preference without changing operation timing.

## Verification

Use the `motion_gallery` example for synthetic-only visual checks. It refuses
vault paths outside `/tmp`, requires a fresh path and never uses the personal
vault or launches the Codex worker. Modes: `locked`, `search`, `settings`, `dialog`,
`knowledge`, plus `drawing` to inspect exact intermediate honeycomb strokes (click
to advance the phase); append `small` for 800×600 or `reduced` for a persisted static state.
Normal size is 1120×820. Example:

```sh
ME_VAULT_DIR=/tmp/me-motion-unique/vault ./scripts/cargo run --offline --release -p me-app --example motion_gallery -- locked small
```

Check unlock, input focus, reduced-motion persistence, page changes, dialogs and
Knowledge selection at both sizes. Shared animation identities must survive
ordinary rerenders such as typing; geometry must remain unchanged after the
animation. Automated checks cover path reveal, settled/reduced phases and
preference persistence/malformed input; the design guard, formatting, Rust
compilation and Clippy are also required. Linux rendering requires a separate
platform check.

## macOS validation — 2026-09-20

The native release preview was inspected with synthetic data at 1120×820 and
800×600. Checks included the startup perimeter and fingerprint seal, Search,
Settings, Knowledge selection, add/detail dialogs, text entry, and the saved
Reduce motion control. The production drawing painter was also inspected paused
at 30% and 50%, confirming the four-sided build and moving indigo tips. Normal
workspace and minimum-size startup viewport dimensions were confirmed by the
harness. No real vault data was used.

All eight desktop tests, workspace/all-target/all-feature Clippy with warnings
denied, formatting, the design-system guard and its five regression checks passed.
The macOS development release bundle was rebuilt. Linux visual verification and
exhaustive checks of every modal state remain unperformed.

Account onboarding, sign-in and unlock share a 1800 ms orbital fingerprint,
Space Grotesk Bold wordmark and a tagline aligned by measured font baselines. The card covers the right half
of the resting fingerprint. The wide artwork is omitted in compact layouts.
Reduced motion renders its final pose immediately. The `drawing` mode now pauses
both the production perimeter painter and orbital artwork for visual checks.

## Branding and account motion — 2026-09-22

The Sora Bold wordmark is bundled from the official sora-xor/sora-font repository,
with the unchanged OFL retained in the app's licenses. The font blob was verified
against upstream Git SHA-1 `e4aa33f99882ecb63bbc7ef2f061d615b99318c8`.
The tagline is “All about you.”; the primary account headline is “Your life.
Brought together.” Both reuse shared components throughout the app.

Release previews were checked on macOS at 1120×820 and 800×600 for login,
registration, remembered-account unlock, ChatGPT setup and Settings, plus the wide recovery-code
step. Checks included baseline alignment, Register navigation, password focus,
reduced-motion rendering, the fingerprint's final half-occluded position and
absence of the floating debug motion control. The production drawing was also
inspected paused at 30%, with inward fading applied to its animated tips.

Formatting, workspace/all-target/all-feature compilation, strict Clippy, the
design guard with five regression checks, 12 desktop tests and the release build
passed. Linux rendering and screen-reader behavior remain unverified.

## Account activity validation — 22 September 2026

The release macOS preview was checked at 1120×820 and 800×600 for sign-in,
registration preparation, recovery-code submission and reduced-motion unlock.
The masked values remain visible, the action spinner fits in the existing button,
and the clockwise highlight fades toward the center. Compact scrolling, frozen
field input and blocked navigation during submission were checked natively.
Switching from busy to idle preserves the sign-in geometry and stops the activity.

The native account lifecycle harness passed with real local HTTP/PostgreSQL,
synthetic credentials and a fake provider, including in-flight field retention,
failed-unlock retention and successful-transition cleanup. Four core account tests
and 12 desktop tests passed, along with design checks, formatting, all-target /
all-feature compilation, strict Clippy and release builds. Linux rendering,
screen-reader behavior and public-cloud latency remain unverified.

## Branding and shared fades — 23 September 2026

Space Grotesk Bold replaces Sora for the wordmark. The static font is bundled from
[Florian Karsten’s official repository](https://github.com/floriankarsten/space-grotesk),
with its unchanged SIL OFL notice. Git blob SHA-1:
`f8eb245d1ec7fd02d77f5a9b55ef6afc4c9ed1c1`.
The tagline baseline uses the resolved fonts’ ascent/descent and line-height metrics.

The fingerprint’s entrance clock belongs to the window instead of the current
account step. Recreating the form, moving to recovery or ChatGPT, and relocking
cannot replay it. Compact layouts can omit the drawing without resetting that clock.
The recovery acknowledgment is a clear 24 px checkbox with a 2 px ink outline;
its square and label both toggle the check, and Continue requires acknowledgment.

Sidebar and workspace decoration reuse the identity field’s smoothstep fade.
Their elliptical corner regions extend 160×224 px and 320×160 px respectively;
cells intersecting the fade are drawn even if their centers fall outside it.
Busy highlights and entrance fronts use the same per-segment opacity mask. The
workspace entrance runs on navigation; busy animation follows real account/search,
import, save and knowledge-loading state, stopping at completion and respecting
reduced motion. No Knowledge data cells or stored positions are faded or moved.

The final release previews were inspected on macOS at 1120×820 and 800×600.
The shaped wordmark and tagline share a visible baseline at both sizes. Moving
from the initial account screen to recovery leaves the fingerprint in its settled
pose. Recovery acknowledgment was checked with keyboard focus and Space, including
unchecked/checked appearance and Continue gating. Pointer toggling was not
reverified in this pass; the existing label-wide click handler is retained.
Sidebar and page fades were inspected both idle and during loading at both sizes,
and the highlight stops when activity ends. A compact reduced-motion workspace
was also inspected; toggling its synthetic loading state was inconclusive through
native automation, so that specific interaction remains unverified.

The native account lifecycle harness passed against the local HTTP/PostgreSQL
backend with synthetic credentials and a fake provider, including assertions that
the window entrance clock survives setup transitions. The 12 desktop tests,
design-system guard and five guard regression checks, formatting, workspace
all-target/all-feature compilation, strict Clippy, and release builds passed.
Linux rendering and screen-reader behavior remain unverified.


## Background stacking correction — 23 September 2026

Workspace honeycomb fields and loading highlights now precede page content in
GPUI's child paint order. They previously followed the page, which drew the lines
over header controls. Sidebar decoration already uses the background order.
Settings uses the shell background instead of covering the decorative layer with
an extra opaque page surface. Buttons, cards and dialogs retain their own fills,
and the decorative canvases still have no input handlers.

The release macOS preview was checked with synthetic Imports and Settings at
1120×820 and 800×600. Static fields and active loading highlights remained behind
the Add files button; sidebar navigation, selected rows and Settings controls
remained in front. Navigation by pointer was verified. The design guard, its five
regression checks, formatting, workspace/all-target/all-feature compilation,
strict Clippy and release builds passed. Linux rendering, screen readers and an
exhaustive pass through every dialog were not reverified for this correction.


## Logins honeycomb motion — 23 September 2026

Logins uses a finite 900 ms selection clock instead of mount-based animation IDs.
It drives a rounded honeycomb identity ribbon, the selected list border and a
short staggered fade on field cards. The graphite key and every control stay in
place while adjacent cells unfold and draw with indigo fronts. The outer cells
settle into the existing decorative palette. Values never seed this drawing.
Only an actual selection change starts the clock; ordinary rerenders, scrolling,
editing, revealing a password and saving keep the current phase. Re-selecting the
current login avoids an unnecessary reload. No idle animation loop is added.
Reduced motion bypasses the clock and uses the complete drawing immediately.

The `logins_gallery` harness uses the checked-in synthetic export and supports
`list`, `empty`, `no-match`, `edit`, `preview`, `error`, `loading` and
`detail-error` modes; append `small`
for 800×600 or `reduced` for the persisted reduced-motion preference.


Verification: the macOS release preview was inspected with synthetic logins at
1120×820 and 800×600. Selection was observed during its trace/unfolding phase and
after settling. Checks covered repeated selection, keyboard navigation, long
titles, editing, saving a synthetic title, no search matches, loading, and recovery
from a synthetic detail error. Reduce motion was enabled through Settings at the
minimum size; selection rendered the complete static design and the persisted
preference was verified. The original import/data behavior was not changed.

All 14 desktop tests, five design-guard regression tests, the design-system guard,
formatting, workspace/all-target/all-feature compilation and strict Clippy passed.
The release macOS app bundle was rebuilt. Linux rendering and screen-reader
behavior remain unverified.


Website logos can replace the central key in the Logins ribbon and list badge.
Their loading does not restart the selection clock or move the surrounding layout;
the key remains visible until a validated, bounded raster is ready. Disabling
Website icons leaves the honeycomb and reduced-motion preference intact.


## Login selection perimeter correction — 23 September 2026

The shared phase-driven frame canvas now uses explicit zero insets. Its previous
static position inherited the login row's 12 px content padding, shifting the
animated outline right. The overlay is anchored to the row bounds while keeping
the existing stroke inset, timing, layout and reduced-motion behavior.

The active trace was visually checked in the native macOS release preview at
1120×820 and 800×600 with synthetic data. The three shared motion tests, design
guard and its five regression tests, formatting and workspace/all-target/all-feature
strict Clippy passed. The macOS bundle was rebuilt; Linux rendering was not checked.
