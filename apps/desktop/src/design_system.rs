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
    pub const REGULAR: FontWeight = FontWeight::NORMAL;
    pub const MEDIUM: FontWeight = FontWeight::MEDIUM;
    pub const EMPHASIS: FontWeight = FontWeight::SEMIBOLD;
}

pub mod layout {
    pub const SIDEBAR_WIDTH: f32 = 192.;
    pub const SEARCH_WIDTH: f32 = 760.;
    pub const CONTENT_WIDTH: f32 = 640.;
    pub const TREE_INDENT: f32 = 24.;
    pub const PROGRESS_HEIGHT: f32 = 8.;
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
    fn metrics(self) -> (f32, f32, FontWeight) {
        match self {
            Self::Caption => (11., 16., font::REGULAR),
            Self::Small => (12., 20., font::REGULAR),
            Self::Body => (13., 22., font::REGULAR),
            Self::Label => (14., 22., font::REGULAR),
            Self::Value => (16., 24., font::REGULAR),
            Self::Section => (18., 24., font::EMPHASIS),
            Self::Title => (24., 32., font::MEDIUM),
            Self::BrandSmall => (28., 36., font::EMPHASIS),
            Self::Brand => (32., 40., font::EMPHASIS),
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
