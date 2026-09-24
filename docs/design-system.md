# ME. design system

This is the required design contract for every current and future ME. screen.
It was derived from the existing desktop UI, preserving the cool neutral palette,
Geist typography, indigo accents, compact sidebar and quiet outline iconography.
Product behavior and copy conventions remain in [design.md](design.md).

The executable source of truth is
[`design_system.rs`](../apps/desktop/src/design_system.rs).
[`theme.rs`](../apps/desktop/src/theme.rs) provides shared components;
[`assets.rs`](../apps/desktop/src/assets.rs) owns the typed icon registry.
All measurements below are logical pixels, scaled by GPUI for the display.

## Shape, spacing and layout

**Use an 8 px radius for every rectangle:** buttons, input frames, cards, notices,
attachment chips, navigation rows and dialogs. Use `radius::STANDARD`, never an
inline number. Only circular status dots and pill toggles use `rounded_full()`.
Do not use different radii to distinguish primary and secondary actions.

Use the same scale for padding, margins, gaps and list indentation:

| Token (`space::`) | px | Intended use |
| --- | ---: | --- |
| `MICRO` | 2 | Optical alignment, such as the review selection box |
| `XS` | 4 | Separation between navigation rows |
| `SM` | 8 | Related labels, icon/text gaps, compact groups |
| `MD` | 12 | Compact card/control inset |
| `LG` | 16 | Standard card inset, horizontal action padding |
| `XL` | 20 | Result-row horizontal inset |
| `XXL` | 24 | Dialog inset and vertical rhythm, tree depth step |
| `XXXL` | 28 | Settings section separation |
| `SECTION` | 32 | Major section separation |
| `SECTION_LOOSE` | 36 | Space below the sidebar identity |
| `PAGE` | 40 | Main page side/bottom gutters |
| `TITLEBAR` | 56 | Sidebar top clearance |
| `PAGE_TOP` | 72 | Main page top clearance |

The base grid is 4 px; 2 px is reserved for optical alignment. Auto-centering,
zero insets and proportional sizes are allowed. Do not pick intermediate values.
Use `layout::SIDEBAR_WIDTH` (192), `SEARCH_WIDTH` (760), `CONTENT_WIDTH` (640),
and `TREE_INDENT` (24) for recurring geometry. Controls use the named compact
(32), default (36) or large (44) heights. Dialog widths and maximum scroll heights
may vary with content: these are geometry, not spacing. Keep those choices
explicit, constrain to the available width, and check the 800×600 minimum window.

## Typography

Bundle and use **Geist** (`font::SANS`) for UI and **Geist Mono** (`font::MONO`)
for stored values and technical metadata. The **ME. wordmark alone** uses bundled
**Space Grotesk Bold** (`font::BRAND`, `font::BRAND_WEIGHT`) through `wordmark()`;
`brand_signature()` aligns “All about you.” using the fonts’ measured baseline
metrics, with the same method at both wordmark sizes (not element-box alignment).
Do not rely on a machine's installed
fonts. Set the base family at the screen root; font inheritance within a screen
is intentional. Monospace uses the bundled regular face only.

Apply size, line height and default weight together with `type_style(Type::…)`.

| Role | Size / line height | Default weight | Use |
| --- | --- | --- | --- |
| `Caption` | 11 / 16 | Regular | Eyebrows, metadata, source labels, helper text |
| `Small` | 12 / 20 | Regular | Compact actions, descriptions, form labels |
| `Body` | 13 / 22 | Regular | Navigation, fields, standard content |
| `Label` | 14 / 22 | Regular | Card labels, section introductions |
| `Value` | 16 / 24 | Regular | Prominent stored values, with Geist Mono |
| `Section` | 18 / 24 | Semibold | Subsection titles |
| `Title` | 24 / 32 | Medium | Page and dialog titles, via `heading()` |
| `BrandSmall` | 28 / 36 | Bold / Space Grotesk | Sidebar and compact wordmark |
| `Brand` | 40 / 48 | Bold / Space Grotesk | Wide setup/unlock wordmark |
| `Hero` | 40 / 48 | Medium | Search-page headline only |

Use `font::MEDIUM` or `font::EMPHASIS` sparingly to emphasize a row or label.
Do not create a separate font size for each new feature. Text must be allowed to
wrap or truncate intentionally, with scrollable content where needed.

## Semantic colors

Use the role, not a visually similar hex value. These RGB values are centralized
in `design_system::color`, re-exported by `theme`.

| Token | Hex | Use |
| --- | --- | --- |
| `BG` | `#F8F9FB` | App background, inset content |
| `SURFACE` | `#FFFFFF` | Cards, dialogs, inverse text on primary actions |
| `SIDEBAR` | `#F0F2F5` | Navigation background |
| `INK` | `#20242B` | Main text and primary action fill |
| `MUTED` | `#758091` | Secondary labels |
| `FAINT` | `#929CAB` | Tertiary metadata/placeholders |
| `LINE` | `#E2E6ED` | 1 px borders and separators |
| `HOVER` | `#F0F3F8` | Hover fills and icon backplates |
| `ACCENT` | `#4D66DE` | Links, selection and active controls |
| `FOCUS` | `#B2C3DD` | Focused input border |
| `PRIMARY_HOVER` | `#3C485C` | Hovered primary action |
| `DECORATIVE` | `#CFD6E0` | Decorative fingerprint and occupied map borders |
| `KNOWLEDGE_SOURCE` | `#3C485C` | Source cells in the Knowledge map; shares the established graphite |
| `SUCCESS` | `#5A947C` | Positive status indicator |
| `WARNING` | `#765B2E` | Warnings and conflicting values |
| `WARNING_SURFACE` | `#FFF4DD` | Warning notice fill |
| `DANGER` | `#995252` | Errors and destructive status |
| `DANGER_SURFACE` | `#FFEDED` | Error notice fill |

`SCRIM` (`#18253B25`) and `SELECTION` (`#729BDC40`) use **RGBA**, via `rgba()`.
All other roles use `rgb()`. Status must always include explanatory text, rather
than relying on color alone. This extraction preserves the existing palette;
it is not a contrast/accessibility certification.

## Components and states

- `progress_bar(fraction, color, activity)`: shared 8 px track (`layout::PROGRESS_HEIGHT`), standard rectangle radius and semantic status color. Fill uses completed work units only; unknown totals use a moving marker and a visible Working label. Elapsed time never sets a percentage.
- `heading(text)`: all page/dialog headings use the same title style.
- `eyebrow(text)`: mono caption style with the shared secondary color.
- `primary_action()`: 36 px height, 16 px horizontal inset, 8 px radius, 12/20
  type, graphite background and white text. Assign an ID before adding a hover
  state; use `PRIMARY_HOVER`. Setup/unlock use the 44 px height. Keep busy/disabled
  behavior and click handlers explicit in the feature.
- `secondary_action()`: the primary action’s geometry and type, with `SURFACE`,
  `INK`, and a `LINE` border. Use `HOVER` for its hover fill.
- `action_indicator(busy, animated)`: a 16 px spinner replaces the arrow within the
  existing action footprint. Busy actions retain their full-contrast fill and describe
  the operation. Reduced motion shows a static arc.
- `checkbox(checked, label)`: a 24 px (`layout::CHECKBOX`) square with the standard
  radius, a 2 px ink outline when unchecked and accent-filled with an ME Outline check when
  checked. Its label shares a minimum 44 px hit row. Callers supply state, focus,
  keyboard activation and busy behavior; labels stay stable when toggled.
- `modal_panel(width, animated)`: shared white surface, 1 px border, 8 px radius, 24 px inset
  and 24 px content gap, constrained to the available width. The feature owns
  scrolling and maximum height. The common overlay owns the scrim.
- Input frames: 44 px height, 12 px horizontal inset, 8 px radius, 1 px `LINE`
  border; focused inputs use `FOCUS`. Text inputs use the `Body` style.
- Result cards: white surface, 1 px `LINE` border, 8 px radius, 20 px horizontal
  and 16 px vertical inset. Stored values use `Value` with Geist Mono.
- Navigation: the same radius, type, height and insets for every destination,
  including Settings. Selected destination uses white fill and an indigo icon.
- Warning/error notices: the matching semantic surface/text pair, with a caption
  or small/body message according to its role. Do not add another amber or red.

Example:

```rust
modal_panel(486., self.motion_enabled())
    .child(heading("Add a detail"))
    .child(
        primary_action()
            .id("save-detail")
            .hover(|s| s.bg(rgb(PRIMARY_HOVER)))
            .on_click(/* feature handler */)
            .child("Save")
            .child(icon(Icon::Arrow, IconSize::Medium, SURFACE)),
    )
```

## Knowledge map

The native honeycomb uses the shared palette and type roles. `knowledge::`
centralizes its geometry: 52 px outer radius, a 76×68 px text/hit region, 1 px
borders, 2 px connection strokes and a 5 px halo at 12% opacity. Hexagonal corners
use `radius::STANDARD`; rectangular controls retain exactly the same 8 px radius.
The grid uses `LINE` at 72% opacity and nonmatching search results dim to 45%.
Document and credential cells use `KNOWLEDGE_SOURCE` with inverse text. Data
labels use Caption, values Small/Geist Mono, and the selected full value uses Value.
At less than 80% zoom the text becomes a typed outline symbol; selection still
reveals the full data below the map. Zoom changes in 20% steps, from 40% to 200%.
Confirmed connections follow cell borders in `ACCENT`; unconfirmed/context
connections are dashed `WARNING` paths. The detail panel spells out status and
relationship, including conflicts and unverified sources, without relying on color.
Only selected connections are emphasized. Background grid, data fill, inset
selection and faint halos keep the reference’s layered appearance restrained.

## Honeycomb and interface motion

Use `theme::motion` for native, finite transitions. Pass the device's motion
preference to shared helpers; no decorative animation may delay an operation,
intercept input or change stored Knowledge positions. The recurring honeycomb
uses existing `LINE`, `DECORATIVE` and `ACCENT` colors with rounded 8 px corners.

`onboarding_fingerprint(progress)` uses the ME Outline fingerprint at 224 px,
without an enclosing seal or constellation. In the wide account layout its center
sits at the card’s left edge, so the opaque card covers its right half. The 1800 ms
entrance follows a shrinking orbit with a foreshortened turn, depth scaling and
fade-in. Its clock starts once when the window’s motion preference loads, and is
retained across account/recovery/provider steps, login switches and relocking.
It settles completely and never moves the form or intercepts input. Page and form
transitions never restart the fingerprint; no frames are requested after it settles.
Compact layouts omit this art. `fingerprint_intro` remains the smaller 72 px
identity for the legacy local-vault/restore view.

| `motion::` timing | ms | Behavior |
| --- | ---: | --- |
| `IDENTITY_ENTER_MS` | 1100 | Existing fingerprint entrance |
| `FIELD_ENTER_MS` | 1900 | Perimeter lattice and identity seal |
| `PAGE_ENTER_MS` | 260 | Page fade with `space::SM` travel |
| `OVERLAY_ENTER_MS` | 180 | Dialog fade, without moving controls |
| `FRAME_TRACE_MS` | 760 | Panel/navigation border trace, fading to rest |
| `FOCUS_TRACE_MS` | 420 | Focused field border trace |
| `NODE_ENTER_MS` | 440 | Newly added Knowledge cells fade in place |
| `CONNECTION_TRACE_MS` | 640 | Selected connections draw along cell borders |
| `SELECTION_TRACE_MS` | 280 | Selected cell inset draws in place |

Motion geometry is centralized alongside timing: 32 px field cells, 20 px
sidebar cells, 1 px structure and 1.5 px drawing fronts.
The identity field fades from every edge over a 320 px band with a smoothstep
curve. Its static lines and animated indigo fronts use the same opacity mask;
there is no hard central cutout. Selected cells have three inner cube edges and
a 1.5 px filled center dot. Paths are split into at most 8 px segments and grouped
in 32 alpha batches. Sidebar decoration fades through the upper-right 160×224 px
area; the workspace corner uses 320×160 px. Both use the same smoothstep opacity
curve as the identity field, applied to normalized elliptical distance from the
outer corner. All three fields include quiet cube edges and dots. Static lines,
entrance drawing fronts and busy highlights share the exact same fade mask.
Paint these decorative layers before page and sidebar content, including during
loading. Buttons, cards, text and dialogs always draw above them. Page roots use
the shell background so the decoration stays visible in empty space; opaque
control and card surfaces cover it. Decoration has no hit targets or input handlers.
Page navigation replays the short field entrance; sidebar decoration stays settled.
Actual loading work adds the same clockwise activity highlight, which disappears
when that work ends. Reduced motion suppresses it. Field opacity is 80%, sidebar 70%, header 55%;
animated fronts use 65%, navigation 40%, cube edges 70% of field opacity.
The drawing tail spans 16% of a cell outline; fronts use 48% of the timeline for
inward spread, 10% deterministic stagger and 38% per-cell drawing. Page content
starts at 35% opacity and new nodes at 40%. Frame traces are inset 1 px.

Orbital fingerprint parameters are centralized: 224 px horizontal travel, 72 px
orbit height, 64% initial depth scale, 18% minimum foreshortened width, 0.24 rad
initial tilt and 85% final opacity. All are drawing geometry, not spacing roles.

Reduce motion remains available in Settings. The floating control on login,
registration and unlock is **debug-only** (`cfg!(debug_assertions)`), including
when a release build enables development tools. Release account screens omit it.
The saved device preference still renders settled decoration before unlock;
system accessibility preferences are not automatically imported.
See [interface-motion.md](interface-motion.md) for lifecycle and verification.

## Icons

**ME Outline** is the app's existing bundled icon family. Use it everywhere via
`assets::icon(Icon::…, IconSize::…, semantic_color)`; do not mix in platform glyphs,
emoji or a second icon package. Website logos in Logins are content images,
not action icons; they use the bounded website-artwork loader described below.
Keyboard shortcut symbols
and punctuation are text, not icons.

All 31 SVG assets use a 24×24 viewBox, no fill, 1.5 px strokes, round caps and round
joins. Black is the SVG mask source; GPUI applies the semantic tint at runtime.
Sizes are `Small` 12, `Medium` 16, `Large` 20, `Brand` 24, `Hero` 72 and
`Identity` 224. The last three are for the fingerprint identity, not ordinary
action icons. Icon-only controls
still need a surrounding hit area; a 12 px glyph is not a 12 px button.

Add an icon only when no existing symbol fits. Match the SVG contract and add its
typed enum variant, path and embedded bytes in `assets.rs` together. Unknown names
and arbitrary sizes cannot be passed to the rendering helper.

## Enforcement and future changes

`AGENTS.md` requires this file before every UI feature. The Cargo helper runs
`./scripts/check-design-system` before compiling/running, and the CI workflow
runs that guard plus its regression checks on pushes and pull requests.

The guard recursively checks current and newly added Rust views for local colors,
font sizes/leading, font families, weights, spacing and corner-radius overrides,
then validates every SVG and its registry. It ignores comments and text content.
It is a source guard, not a full Rust semantic analysis or visual regression suite;
review composed layout and dynamic/interactive states as well.

When changing the design system:

1. Reuse an existing role first. If a new role is needed, define it centrally and
   explain its purpose here; do not copy a literal into a screen.
2. Update all affected components and states together. Keep business logic out of
   style helpers and keep user data out of visual fixtures/screenshots.
3. Run the guard and its regression checks, `cargo fmt`, workspace compilation
   and Clippy using the pinned GPUI API.
4. Inspect the changed views at normal and minimum size, including long labels,
   empty/loading/error states, focus, selected navigation and relevant dialogs.
   Record which states were actually verified.

## Consistency audit — 2026-09-17

The existing source had nine different rectangular radii (4–14 px), 14 text sizes,
three near-duplicate warning colors, two error colors, and three SVG stroke
weights (1.35, 1.4 and 1.5). Spacing included 5/7/9/11/13/17/21/23 px variants.

The migration applies the contract across Search/forms, Review, Browser, Settings,
setup/unlock, note/add/access dialogs, 1Password/credential details, document
analysis/question cards and the custom text input. It also aligns page gutters,
button heights, title roles, mono credential values and tree indentation.

The fixed palette and existing icon shapes are retained. Ordinary rectangles now
share one radius; status colors, type metrics, layout scale and icon stroke are
centralized. The settings icon's opaque circle fills were replaced with line gaps
so it behaves as the same tintable outline family on any surface.

Validation: the source/asset guard and its five regression checks passed, as did
workspace compilation and Clippy with warnings denied. An isolated macOS build
using synthetic fixtures was visually checked for Search, Review, Browser,
Settings, the 1Password panel and the note dialog. Search and the note dialog were
also checked at the 800×600 minimum. No real vault data was used. The remaining
workflow states were audited in source; a complete interaction/accessibility
regression and Linux visual pass were not performed.

## Account onboarding

The three-step account introduction uses the shared `modal_panel`, fields,
actions, semantic colors and typed icons. Its layout roles are
`ONBOARDING_WIDTH` (960 px; also the two-column breakpoint),
`ONBOARDING_PANEL_WIDTH` (464 px), and `ONBOARDING_ART_SIZE` (256 px).
Below the breakpoint a compact ME. identity replaces the side illustration;
the entire form scrolls, with titlebar clearance and bottom breathing room.
The recovery-code field contains its own 32 px Copy action and transient Copied
feedback, keeping the stored value in Geist Mono. Copying never confirms safekeeping.
Safekeeping uses the shared checkbox with the stable label “I’ve saved my recovery
code.” Both the square and label toggle it; Continue is enabled only while checked.
The recovery step supports Tab navigation and Space/Enter activation of its controls.

The checkbox was visually checked in the release macOS preview at 1120 × 820 and
800 × 600, including checked/unchecked states, label clicks, keyboard toggling and
Continue gating. Formatting, the design guard and its regression checks, workspace
compilation, strict Clippy and the release build passed. Linux rendering and
screen-reader behavior remain unverified.

The wordmark and tagline share a baseline in both layouts. The headline
“Your life. Brought together.” introduces notes, documents and everyday details.
The wide layout positions the orbital fingerprint behind the card; the small
layout retains the complete form without decorative art. Forms reuse the 260 ms
page arrival and panel trace. Reduced motion renders the settled drawing
immediately. Network progress uses explicit text, never simulated percentages.
During submission, fields retain their text (secrets stay masked) as inert snapshots;
secondary actions stay in place and cannot navigate away. Successful transitions
clear password fields; failed attempts retain them for correction.
The button pairs an operation label with a spinner. A clockwise light wave travels
through the existing perimeter honeycombs only while work is active and fades inward
with the static lattice. `ACCOUNT_ACTIVITY_MS` is 2800 ms with a 28% angular tail;
`SPINNER_MS` is 1000 ms, size 16 px with 2 px inset, 1.5 px stroke, a 20% track and
70% bright arc. Reduced motion omits the lattice wave and stops the spinner.
Neither indicator represents a percentage or delays completion.

The remembered-account lock screen reuses this same shell, panel, responsive
breakpoint and motion. It shows a single Unlock rail and a pre-filled, read-only
email field with the shared 44 px input geometry. Keyboard focus starts in the
master-password field; unlocking remains a local operation. Recovery, online
sign-in, Register and Log out retain their existing actions below the primary
Unlock button. At compact sizes the form and secondary actions scroll together.


## Logins workspace

Logins uses the existing 192 px sidebar, a virtualized login list, and an inline
scrolling detail/editor pane. The list uses `LOGIN_LIST_WIDTH` (288 px), switching
to `LOGIN_LIST_COMPACT_WIDTH` (224 px) below `LOGIN_COMPACT_BREAKPOINT` (1000 px).
`LOGIN_ROW_HEIGHT` is 72 px; rows fill the list width and long labels truncate.
The detail uses the existing 640 px maximum content width and 24 px gutters.
All rectangles use the shared 8 px radius; fields, actions, color and typography
retain existing roles. The new typed Key icon follows the ME Outline contract.

Login editing reuses TextInput, with hard-line-break support for notes and tags.
Multiline frames scroll after `LOGIN_TEXTAREA_HEIGHT` (176 px). Input focus uses
FOCUS; field values use Geist Mono in the detail. Concealed values remain masked
until explicitly revealed, including while editing. The list stays visible during
editing; switching destinations requires Save or Cancel. Keyboard navigation uses
Up/Down for the list, Tab/Shift-Tab for fields, Cmd/Ctrl+E to edit, and Cmd/Ctrl+S
to save. Escape cancels editing. At 800×600 all three columns remain available.


The Logins page leaves the shared workspace honeycomb visible behind its detail
pane. List rows have rounded hexagonal key backplates; the rectangular hit target
keeps the standard 8 px radius. The detail header contains an 80 px honeycomb
ribbon (`LOGIN_IDENTITY_HEIGHT`) with a steady graphite key cell, indigo cube
edges and fading outer cells. The drawing uses 24 px cells (`LOGIN_CELL_RADIUS`).
The honeycomb geometry does not depend on credential values. When available,
a website logo replaces the central key while retaining the surrounding motion.

`LOGIN_ENTER_MS` (900 ms) owns a single finite selection clock: the surrounding
cells unfold from 68% spread, lightly overshoot (0.8 cubic coefficient) and settle;
indigo fronts trace their outlines. Cell draws take 62% of that interval with a
7% delay per ring. The field fades across 3.5 cells at 80% maximum opacity. The
selected list badge uses an 8% indigo tint and a border trace. Field cards fade
from 72% to full opacity over `LOGIN_CONTENT_MS` (240 ms), staggered by
`LOGIN_STAGGER_MS` (32 ms), capped at four steps. Values and controls never move.
Typing, reveal/copy, saving and reselecting the same login do not replay motion.
Reduced motion renders the final state immediately; idle frames stop after the
selection clock completes. Empty/loading states retain the same honeycomb ribbon.


### Login detail hierarchy and website artwork

Website logos appear inside the existing honeycomb backplates in the list and
detail header. `LOGIN_LOGO_LIST` (20 px) and `LOGIN_LOGO_DETAIL` (28 px) bound
these content images; contain-fit preserves their proportions. Missing, blocked,
invalid or disabled images use the bundled ME Outline key with identical geometry.
Logos keep their native colors; navigation and action icons remain ME Outline.

Read-only tags and account labels use shared `tag_chip()`: Small type, HOVER fill,
ACCENT text, 8/4 px horizontal/vertical inset and the standard 8 px radius. Chips
wrap and have no copy action. Numeric metadata uses a plain label/value layout,
without a card, copy button or concealment unless explicitly guarded. Editing
retains the existing text/tag/number storage types. Credentials, URLs, free text,
notes and password history keep their useful Copy actions.

Password history is sorted newest first and remains masked. Each old password
shows “Used until” with its exported retirement date/time in UTC and an elapsed
age. Missing/invalid dates are stated explicitly; they are never inferred from
an import date. Old passwords remain read-only. The 30-second reveal timer and
clipboard expiration still apply.

## Credential creation and context suggestions

The Logins list places **New item** and Import on a separate action row so both
fit at the minimum width. New item opens a scrollable type chooser in the detail
column, retaining the shared honeycomb identity and existing rectangular controls.
A window/paste suggestion shows its source, detected type and captured field count;
secret values appear only in the masked draft editor. Type cards use standard
card insets, Label/Small roles and the shared hover fill. Source and account
choices wrap inside the detail column. No new colors, spacing or radius tokens.

Manual creation and context review use the same editor. Save commits one native
credential; Cancel discards the draft. Password generation is local and replaces
only the draft field. Native records show Created metadata and omit the original
1Password-export action. Captured recovery codes are appended for review when a
native login already has codes, and added in a separate section for imported
logins, retaining their original import unchanged.

## Credential chooser modal

New item uses the shared `modal_panel` and scrim over the unchanged Logins detail
view. The 760 px panel is constrained to the viewport with `space::LG` outer
clearance; its body scrolls while the title and Cancel remain visible. Six typed
credential choices form two columns. Open-window thumbnails follow in two
columns, using real image content with application/title captions, standard
rectangular radius and semantic focus/hover borders. Permission, empty, loading
and unavailable-preview states use existing text and notice roles. No placeholder
image is presented as a captured window. Choosing a type or a recognized window
closes the modal and opens the corresponding editable detail view.


An Accessibility-denied window selection keeps the chooser open and places a
recovery notice first in its scrolling body. It uses the shared warning surface,
Section/Small type, standard radius/insets, and the existing Open Settings / Try
again action components. The source is retained for retry; permission errors do
not repeatedly trigger system pop-ups on thumbnail clicks.


## Privacy permission panel

A shared 640 px modal is available from the application menu before unlock and
from Settings afterward. Its scrollable body uses existing card geometry,
Label/Small type and semantic status text. Screen Recording and Accessibility
have separate grant states, request buttons and Settings links. Done and Check
again retain the shared action geometry. Opening/checking the panel never captures
a window. The panel fits both 1120×820 and 800×600 without new visual tokens.
