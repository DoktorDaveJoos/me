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
for stored values and technical metadata. Do not rely on a machine's installed
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
| `BrandSmall` | 28 / 36 | Semibold | Sidebar wordmark only |
| `Brand` | 32 / 40 | Semibold | Setup/unlock wordmark only |
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
- `modal_panel(width)`: shared white surface, 1 px border, 8 px radius, 24 px inset
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
modal_panel(486.)
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

## Identity motion

`fingerprint_intro()` uses the bundled Hero fingerprint, `DECORATIVE` tint and
`space::SECTION` travel. It slides in and makes a foreshortened turn once over
`motion::IDENTITY_ENTER_MS` (1100 ms), then rests. The password form stays still
and interactive throughout; this motion never delays unlocking or repeats while
typing. No new icon, color or typography role is introduced.

## Icons

**ME Outline** is the app's existing bundled icon family. Use it everywhere via
`assets::icon(Icon::…, IconSize::…, semantic_color)`; do not mix in platform glyphs,
emoji, a second icon package or runtime icon downloads. Keyboard shortcut symbols
and punctuation are text, not icons.

All 30 SVG assets use a 24×24 viewBox, no fill, 1.5 px strokes, round caps and round
joins. Black is the SVG mask source; GPUI applies the semantic tint at runtime.
Sizes are `Small` 12, `Medium` 16, `Large` 20, `Brand` 24 and `Hero` 72. The last two
are for the fingerprint identity, not ordinary action icons. Icon-only controls
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
