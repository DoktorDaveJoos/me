//! ME.'s visual source of truth, derived from the existing desktop UI.
//! Read docs/design-system.md before changing a token or adding a component.
use gpui::{FontWeight, Styled, px};

pub mod color {
    pub const BG: u32 = 0xf8f9fb;
    pub const SURFACE: u32 = 0xffffff;
    pub const SIDEBAR: u32 = 0xf0f2f5;
    pub const INK: u32 = 0x20242b;
    pub const MUTED: u32 = 0x758091;
    pub const FAINT: u32 = 0x929cab;
    pub const LINE: u32 = 0xe2e6ed;
    pub const HOVER: u32 = 0xf0f3f8;
    pub const ACCENT: u32 = 0x4d66de;
    pub const FOCUS: u32 = 0xb2c3dd;
    pub const PRIMARY_HOVER: u32 = 0x3c485c;
    pub const DECORATIVE: u32 = 0xcfd6e0;
    /// Dark source cells in the Knowledge map, using the established graphite.
    pub const KNOWLEDGE_SOURCE: u32 = PRIMARY_HOVER;
    pub const SUCCESS: u32 = 0x5a947c;
    pub const WARNING: u32 = 0x765b2e;
    pub const WARNING_SURFACE: u32 = 0xfff4dd;
    pub const DANGER: u32 = 0x995252;
    pub const DANGER_SURFACE: u32 = 0xffeded;
    // RGBA, unlike the RGB colors above.
    pub const SCRIM: u32 = 0x18253b25;
    pub const SELECTION: u32 = 0x729bdc40;
}

/// Logical pixels. Use the same scale for padding, margin and gaps.
pub mod space {
    pub const MICRO: f32 = 2.;
    pub const XS: f32 = 4.;
    pub const SM: f32 = 8.;
    pub const MD: f32 = 12.;
    pub const LG: f32 = 16.;
    pub const XL: f32 = 20.;
    pub const XXL: f32 = 24.;
    pub const XXXL: f32 = 28.;
    pub const SECTION: f32 = 32.;
    pub const SECTION_LOOSE: f32 = 36.;
    pub const PAGE: f32 = 40.;
    pub const TITLEBAR: f32 = 56.;
    pub const PAGE_TOP: f32 = 72.;
}

pub mod motion {
    /// One-shot fingerprint entrance on the vault screen; never delays input.
    pub const IDENTITY_ENTER_MS: u64 = 1100;
    pub const FIELD_ENTER_MS: u64 = 1900;
    /// Login selection: one finite clock for artwork, row trace and field arrival.
    pub const LOGIN_ENTER_MS: u64 = 900;
    pub const LOGIN_CONTENT_MS: u64 = 240;
    pub const LOGIN_STAGGER_MS: u64 = 32;
    pub const LOGIN_STAGGER_LIMIT: usize = 4;
    pub const LOGIN_CONTENT_OPACITY: f32 = 0.72;
    pub const LOGIN_CELL_RADIUS: f32 = 24.;
    pub const LOGIN_BLOOM_SCALE: f32 = 0.68;
    pub const LOGIN_BLOOM_OVERSHOOT: f32 = 0.8;
    pub const LOGIN_CELL_STAGGER: f32 = 0.07;
    pub const LOGIN_CELL_DRAW: f32 = 0.62;
    pub const LOGIN_FIELD_SPREAD: f32 = 3.5;
    pub const LOGIN_FIELD_OPACITY: f32 = 0.8;
    pub const LOGIN_BADGE_TINT: f32 = 0.08;
    pub const ACCOUNT_ACTIVITY_MS: u64 = 2800;
    pub const ACCOUNT_ACTIVITY_TAIL: f32 = 0.28;
    pub const SPINNER_MS: u64 = 1000;
    pub const SPINNER_SIZE: f32 = 16.;
    pub const SPINNER_INSET: f32 = 2.;
    pub const SPINNER_TRACK_OPACITY: f32 = 0.2;
    pub const SPINNER_ARC: f32 = 0.7;
    pub const PAGE_ENTER_MS: u64 = 260;
    pub const OVERLAY_ENTER_MS: u64 = 180;
    pub const FRAME_TRACE_MS: u64 = 760;
    pub const FOCUS_TRACE_MS: u64 = 420;
    pub const NODE_ENTER_MS: u64 = 440;
    pub const CONNECTION_TRACE_MS: u64 = 640;
    pub const SELECTION_TRACE_MS: u64 = 280;
    pub const CONTENT_START_OPACITY: f32 = 0.35;
    pub const NODE_START_OPACITY: f32 = 0.4;
    pub const FIELD_CELL_RADIUS: f32 = 32.;
    pub const SIDEBAR_CELL_RADIUS: f32 = 20.;
    pub const FIELD_STROKE: f32 = 1.;
    pub const TRACE_STROKE: f32 = 1.5;
    pub const FIELD_BAND: f32 = 320.;
    pub const FIELD_FADE_BUCKETS: usize = 32;
    pub const FIELD_SEGMENT_LENGTH: f32 = 8.;
    pub const FIELD_DOT_RADIUS: f32 = 1.5;
    pub const FIELD_CUBE_OPACITY: f32 = 0.7;
    pub const FINGERPRINT_ENTER_MS: u64 = 1800;
    pub const FINGERPRINT_TRAVEL: f32 = 224.;
    pub const FINGERPRINT_ORBIT_HEIGHT: f32 = 72.;
    pub const FINGERPRINT_MIN_SCALE: f32 = 0.64;
    pub const FINGERPRINT_MIN_WIDTH: f32 = 0.18;
    pub const FINGERPRINT_TILT: f32 = 0.24;
    pub const FINGERPRINT_OPACITY: f32 = 0.85;
    pub const SIDEBAR_FIELD_WIDTH: f32 = 160.;
    pub const SIDEBAR_FIELD_HEIGHT: f32 = 224.;
    pub const HEADER_FIELD_WIDTH: f32 = 320.;
    pub const HEADER_FIELD_HEIGHT: f32 = 160.;
    pub const FIELD_OPACITY: f32 = 0.8;
    pub const SIDEBAR_FIELD_OPACITY: f32 = 0.7;
    pub const HEADER_FIELD_OPACITY: f32 = 0.55;
    pub const TRACE_OPACITY: f32 = 0.65;
    pub const NAV_TRACE_OPACITY: f32 = 0.4;
    pub const SEAL_OPACITY: f32 = 0.8;
    pub const SEAL_RADIUS: f32 = 48.;
    pub const FRAME_INSET: f32 = 1.;
    pub const TRACE_TAIL: f32 = 0.16;
    pub const FIELD_STAGGER: f32 = 0.1;
    pub const FIELD_SPREAD: f32 = 0.48;
    pub const CELL_DRAW_FRACTION: f32 = 0.38;
}

pub mod radius {
    /// Every rectangular card, button, input, navigation item and dialog.
    /// Only pill toggles and circular status dots use rounded_full().
    pub const STANDARD: f32 = 8.;
}

pub mod font {
    use super::FontWeight;
    pub const SANS: &str = "Geist";
    pub const MONO: &str = "Geist Mono";
    pub const BRAND: &str = "Space Grotesk";
    pub const BRAND_WEIGHT: FontWeight = FontWeight::BOLD;
    pub const REGULAR: FontWeight = FontWeight::NORMAL;
    pub const MEDIUM: FontWeight = FontWeight::MEDIUM;
    pub const EMPHASIS: FontWeight = FontWeight::SEMIBOLD;
}

pub mod layout {
    pub const ONBOARDING_WIDTH: f32 = 960.;
    pub const ONBOARDING_PANEL_WIDTH: f32 = 464.;
    pub const ONBOARDING_ART_SIZE: f32 = 256.;
    pub const FINGERPRINT_ART_SIZE: f32 = 224.;
    pub const LOGIN_LIST_WIDTH: f32 = 288.;
    pub const LOGIN_LIST_COMPACT_WIDTH: f32 = 224.;
    pub const LOGIN_COMPACT_BREAKPOINT: f32 = 1000.;
    pub const LOGIN_ROW_HEIGHT: f32 = 72.;
    pub const LOGIN_IDENTITY_HEIGHT: f32 = 80.;
    pub const LOGIN_LOGO_LIST: f32 = 20.;
    pub const LOGIN_LOGO_DETAIL: f32 = 28.;
    pub const LOGIN_TEXTAREA_HEIGHT: f32 = 176.;
    pub const SIDEBAR_WIDTH: f32 = 192.;
    pub const IDENTITY_ICON: f32 = 72.;
    pub const SEARCH_WIDTH: f32 = 760.;
    pub const CONTENT_WIDTH: f32 = 640.;
    pub const TREE_INDENT: f32 = 24.;
    pub const PROGRESS_HEIGHT: f32 = 8.;
    pub const CHECKBOX: f32 = 24.;
    pub const CONTROL_COMPACT: f32 = 32.;
    pub const CONTROL: f32 = 36.;
    pub const CONTROL_LARGE: f32 = 44.;
}

#[derive(Clone, Copy)]
pub enum Type {
    Caption,
    Small,
    Body,
    Label,
    Value,
    Section,
    Title,
    BrandSmall,
    Brand,
    Hero,
}

impl Type {
    pub(crate) fn metrics(self) -> (f32, f32, FontWeight) {
        match self {
            Self::Caption => (11., 16., font::REGULAR),
            Self::Small => (12., 20., font::REGULAR),
            Self::Body => (13., 22., font::REGULAR),
            Self::Label => (14., 22., font::REGULAR),
            Self::Value => (16., 24., font::REGULAR),
            Self::Section => (18., 24., font::EMPHASIS),
            Self::Title => (24., 32., font::MEDIUM),
            Self::BrandSmall => (28., 36., font::BRAND_WEIGHT),
            Self::Brand => (40., 48., font::BRAND_WEIGHT),
            Self::Hero => (40., 48., font::MEDIUM),
        }
    }
}

/// Shared typography is applied as a unit, including leading and weight.
/// The inherited family is Geist; explicitly choose font::MONO for stored values.
pub trait DesignStyle: Styled {
    fn type_style(self, role: Type) -> Self {
        let (size, leading, weight) = role.metrics();
        self.text_size(px(size))
            .line_height(px(leading))
            .font_weight(weight)
    }
}
impl<T: Styled> DesignStyle for T {}

/// Honeycomb geometry and state emphasis. Positions in the vault remain axial
/// coordinates; these values control only their native presentation.
pub mod knowledge {
    pub const CELL_RADIUS: f32 = 52.;
    pub const TEXT_WIDTH: f32 = 76.;
    pub const TEXT_HEIGHT: f32 = 68.;
    pub const BORDER: f32 = 1.;
    pub const CONNECTION_WIDTH: f32 = 2.;
    pub const HALO_WIDTH: f32 = 5.;
    pub const HALO_OPACITY: f32 = 0.12;
    pub const GRID_OPACITY: f32 = 0.72;
    pub const DIM_OPACITY: f32 = 0.45;
    pub const DETAIL_ZOOM: f64 = 0.8;
    pub const ZOOM_STEP: f64 = 0.2;
}
