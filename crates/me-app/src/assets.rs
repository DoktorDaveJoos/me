use gpui::{App, AssetSource, SharedString, Styled, Svg, px, rgb, svg};
use std::borrow::Cow;

/// ME Outline: the bundled, monochrome 24 × 24, 1.5 px stroke icon family.
#[derive(Clone, Copy)]
pub enum Icon {
    Arrow,
    Attach,
    Calendar,
    Card,
    Check,
    Chevron,
    Close,
    Contact,
    Copy,
    Document,
    Down,
    Fingerprint,
    Folder,
    Grid,
    Hash,
    Info,
    Location,
    Mail,
    Menu,
    Phone,
    Plus,
    Search,
    Send,
    Settings,
    Spark,
    Star,
    Text,
    Upload,
    User,
}
impl Icon {
    pub const ALL: &[Self] = &[
        Self::Arrow,
        Self::Attach,
        Self::Calendar,
        Self::Card,
        Self::Check,
        Self::Chevron,
        Self::Close,
        Self::Contact,
        Self::Copy,
        Self::Document,
        Self::Down,
        Self::Fingerprint,
        Self::Folder,
        Self::Grid,
        Self::Hash,
        Self::Info,
        Self::Location,
        Self::Mail,
        Self::Menu,
        Self::Phone,
        Self::Plus,
        Self::Search,
        Self::Send,
        Self::Settings,
        Self::Spark,
        Self::Star,
        Self::Text,
        Self::Upload,
        Self::User,
    ];
    pub fn path(self) -> &'static str {
        match self {
            Self::Arrow => "icons/arrow.svg",
            Self::Attach => "icons/attach.svg",
            Self::Calendar => "icons/calendar.svg",
            Self::Card => "icons/card.svg",
            Self::Check => "icons/check.svg",
            Self::Chevron => "icons/chevron.svg",
            Self::Close => "icons/close.svg",
            Self::Contact => "icons/contact.svg",
            Self::Copy => "icons/copy.svg",
            Self::Document => "icons/document.svg",
            Self::Down => "icons/down.svg",
            Self::Fingerprint => "icons/fingerprint.svg",
            Self::Folder => "icons/folder.svg",
            Self::Grid => "icons/grid.svg",
            Self::Hash => "icons/hash.svg",
            Self::Info => "icons/info.svg",
            Self::Location => "icons/location.svg",
            Self::Mail => "icons/mail.svg",
            Self::Menu => "icons/menu.svg",
            Self::Phone => "icons/phone.svg",
            Self::Plus => "icons/plus.svg",
            Self::Search => "icons/search.svg",
            Self::Send => "icons/send.svg",
            Self::Settings => "icons/settings.svg",
            Self::Spark => "icons/spark.svg",
            Self::Star => "icons/star.svg",
            Self::Text => "icons/text.svg",
            Self::Upload => "icons/upload.svg",
            Self::User => "icons/user.svg",
        }
    }
    fn bytes(self) -> &'static [u8] {
        match self {
            Self::Arrow => include_bytes!("../assets/icons/arrow.svg"),
            Self::Attach => include_bytes!("../assets/icons/attach.svg"),
            Self::Calendar => include_bytes!("../assets/icons/calendar.svg"),
            Self::Card => include_bytes!("../assets/icons/card.svg"),
            Self::Check => include_bytes!("../assets/icons/check.svg"),
            Self::Chevron => include_bytes!("../assets/icons/chevron.svg"),
            Self::Close => include_bytes!("../assets/icons/close.svg"),
            Self::Contact => include_bytes!("../assets/icons/contact.svg"),
            Self::Copy => include_bytes!("../assets/icons/copy.svg"),
            Self::Document => include_bytes!("../assets/icons/document.svg"),
            Self::Down => include_bytes!("../assets/icons/down.svg"),
            Self::Fingerprint => include_bytes!("../assets/icons/fingerprint.svg"),
            Self::Folder => include_bytes!("../assets/icons/folder.svg"),
            Self::Grid => include_bytes!("../assets/icons/grid.svg"),
            Self::Hash => include_bytes!("../assets/icons/hash.svg"),
            Self::Info => include_bytes!("../assets/icons/info.svg"),
            Self::Location => include_bytes!("../assets/icons/location.svg"),
            Self::Mail => include_bytes!("../assets/icons/mail.svg"),
            Self::Menu => include_bytes!("../assets/icons/menu.svg"),
            Self::Phone => include_bytes!("../assets/icons/phone.svg"),
            Self::Plus => include_bytes!("../assets/icons/plus.svg"),
            Self::Search => include_bytes!("../assets/icons/search.svg"),
            Self::Send => include_bytes!("../assets/icons/send.svg"),
            Self::Settings => include_bytes!("../assets/icons/settings.svg"),
            Self::Spark => include_bytes!("../assets/icons/spark.svg"),
            Self::Star => include_bytes!("../assets/icons/star.svg"),
            Self::Text => include_bytes!("../assets/icons/text.svg"),
            Self::Upload => include_bytes!("../assets/icons/upload.svg"),
            Self::User => include_bytes!("../assets/icons/user.svg"),
        }
    }
}

#[derive(Clone, Copy)]
pub enum IconSize {
    Small,
    Medium,
    Large,
    Brand,
    Hero,
}
impl IconSize {
    fn pixels(self) -> f32 {
        match self {
            Self::Small => 12.,
            Self::Medium => 16.,
            Self::Large => 20.,
            Self::Brand => 24.,
            Self::Hero => 72.,
        }
    }
}

pub struct Assets;
impl AssetSource for Assets {
    fn load(&self, path: &str) -> gpui::Result<Option<Cow<'static, [u8]>>> {
        Ok(Icon::ALL
            .iter()
            .find(|icon| icon.path() == path)
            .map(|icon| Cow::Borrowed(icon.bytes())))
    }
    fn list(&self, path: &str) -> gpui::Result<Vec<SharedString>> {
        Ok(Icon::ALL
            .iter()
            .filter(|icon| icon.path().starts_with(path))
            .map(|icon| icon.path().into())
            .collect())
    }
}

pub fn icon(name: Icon, size: IconSize, color: u32) -> Svg {
    svg()
        .path(name.path())
        .size(px(size.pixels()))
        .text_color(rgb(color))
        .flex_shrink_0()
}

pub fn load_fonts(cx: &App) -> gpui::Result<()> {
    cx.text_system().add_fonts(vec![
        Cow::Borrowed(include_bytes!("../assets/fonts/Geist-Regular.ttf")),
        Cow::Borrowed(include_bytes!("../assets/fonts/Geist-Light.ttf")),
        Cow::Borrowed(include_bytes!("../assets/fonts/Geist-Medium.ttf")),
        Cow::Borrowed(include_bytes!("../assets/fonts/Geist-SemiBold.ttf")),
        Cow::Borrowed(include_bytes!("../assets/fonts/GeistMono-Regular.ttf")),
    ])
}
