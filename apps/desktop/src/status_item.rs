//! Menu bar status item on macOS.
//!
//! The pinned GPUI has no status item API, so the item is built with AppKit
//! directly. A left click toggles the quick panel; a right or control click shows
//! the menu. Clicks only enqueue a command; GPUI handles it on its own foreground
//! task. The Linux tray (StatusNotifierItem) is a separate prototype.
//!
//! This is the only module allowed to use `unsafe`: AppKit's class init and menu
//! item target/action setters are unsafe in objc2. Keep every block minimal.
#![allow(unsafe_code)]

use std::cell::OnceCell;
use std::io::Cursor;

use futures_channel::mpsc::{UnboundedSender, unbounded};
use futures_util::StreamExt;
use gpui::App;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject};
use objc2::{
    AnyThread, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel,
};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSEventMask, NSEventModifierFlags, NSEventType,
    NSImage, NSMenu, NSMenuItem, NSStatusBar, NSStatusItem, NSVariableStatusItemLength, NSView,
};
use objc2_foundation::{NSData, NSNumber, NSSize, NSString};
use resvg::{tiny_skia, usvg};

use crate::assets::Icon;

/// AppKit's menu bar icon size in points. Platform geometry, not a UI token.
const ICON_POINTS: f64 = 18.;
/// Pixels per point for the rasterized template image (Retina).
const ICON_SCALE: u32 = 2;

/// Where the quick panel hangs: the item's horizontal center and the bottom of
/// the menu bar, relative to the top-left of the display showing the item.
#[derive(Clone, Copy)]
pub struct Anchor {
    pub display: u32,
    pub center_x: f64,
    pub bottom_y: f64,
}

#[derive(Clone, Copy)]
enum Command {
    TogglePanel(Option<Anchor>),
    Open,
    Lock,
    Quit,
}

struct Ivars {
    commands: UnboundedSender<Command>,
    item: OnceCell<Retained<NSStatusItem>>,
    menu: OnceCell<Retained<NSMenu>>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "MEStatusItemTarget"]
    #[ivars = Ivars]
    struct Target;

    impl Target {
        #[unsafe(method(click:))]
        fn click(&self, sender: Option<&AnyObject>) {
            let mtm = self.mtm();
            let event = NSApplication::sharedApplication(mtm).currentEvent();
            let wants_menu = event.is_some_and(|event| {
                event.r#type() == NSEventType::RightMouseUp
                    || event.modifierFlags().contains(NSEventModifierFlags::Control)
            });
            if wants_menu {
                self.show_menu(mtm);
            } else {
                let anchor = sender
                    .and_then(|sender| sender.downcast_ref::<NSView>())
                    .and_then(anchor);
                self.send(Command::TogglePanel(anchor));
            }
        }

        #[unsafe(method(open:))]
        fn open(&self, _sender: Option<&AnyObject>) {
            self.send(Command::Open);
        }

        #[unsafe(method(lock:))]
        fn lock(&self, _sender: Option<&AnyObject>) {
            self.send(Command::Lock);
        }

        #[unsafe(method(quit:))]
        fn quit(&self, _sender: Option<&AnyObject>) {
            self.send(Command::Quit);
        }
    }
);

impl Target {
    fn new(mtm: MainThreadMarker, commands: UnboundedSender<Command>) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(Ivars {
            commands,
            item: OnceCell::new(),
            menu: OnceCell::new(),
        });
        unsafe { msg_send![super(this), init] }
    }

    fn send(&self, command: Command) {
        let _ = self.ivars().commands.unbounded_send(command);
    }

    /// Attaches the menu only for this click, so a left click stays free for
    /// the panel.
    fn show_menu(&self, mtm: MainThreadMarker) {
        let (Some(item), Some(menu)) = (self.ivars().item.get(), self.ivars().menu.get()) else {
            return;
        };
        item.setMenu(Some(menu));
        if let Some(button) = item.button(mtm) {
            unsafe { button.performClick(None) };
        }
        item.setMenu(None);
    }
}

fn anchor(view: &NSView) -> Option<Anchor> {
    let window = view.window()?;
    let screen = window.screen()?;
    let item = window.frame();
    let display = screen.frame();
    let number = screen
        .deviceDescription()
        .objectForKey(&NSString::from_str("NSScreenNumber"))?
        .downcast::<NSNumber>()
        .ok()?;
    Some(Anchor {
        display: number.unsignedIntValue(),
        center_x: item.origin.x - display.origin.x + item.size.width / 2.,
        bottom_y: display.origin.y + display.size.height - item.origin.y,
    })
}

/// Adds the honeycomb status item. It lives as long as the app.
pub fn install(cx: &mut App) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let (commands, mut received) = unbounded();
    let target = Target::new(mtm, commands);

    let item = NSStatusBar::systemStatusBar().statusItemWithLength(NSVariableStatusItemLength);
    if let Some(button) = item.button(mtm) {
        match template_image() {
            Some(image) => button.setImage(Some(&image)),
            None => button.setTitle(&NSString::from_str(me_core::APP_NAME)),
        }
        button.setToolTip(Some(&NSString::from_str(me_core::APP_NAME)));
        unsafe {
            button.setTarget(Some(&target));
            button.setAction(Some(sel!(click:)));
        }
        button.sendActionOn(NSEventMask::LeftMouseUp | NSEventMask::RightMouseUp);
    }

    let menu = NSMenu::new(mtm);
    for entry in [
        Some(("Open ME.", sel!(open:))),
        Some(("Lock ME.", sel!(lock:))),
        None,
        Some(("Quit ME.", sel!(quit:))),
    ] {
        let Some((title, action)) = entry else {
            menu.addItem(&NSMenuItem::separatorItem(mtm));
            continue;
        };
        let entry = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                NSMenuItem::alloc(mtm),
                &NSString::from_str(title),
                Some(action),
                &NSString::new(),
            )
        };
        unsafe { entry.setTarget(Some(&target)) };
        menu.addItem(&entry);
    }
    let _ = target.ivars().menu.set(menu);
    let _ = target.ivars().item.set(item);

    cx.spawn(async move |cx| {
        // Targets are weak references; this task keeps the item and its target.
        let _keep = target;
        while let Some(command) = received.next().await {
            if cx.update(|cx| handle(command, cx)).is_err() {
                break;
            }
        }
    })
    .detach();
}

/// Shows ME. in the Dock and app switcher only while its main window is open.
pub fn set_dock_visible(visible: bool) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let policy = if visible {
        NSApplicationActivationPolicy::Regular
    } else {
        NSApplicationActivationPolicy::Accessory
    };
    NSApplication::sharedApplication(mtm).setActivationPolicy(policy);
}

fn handle(command: Command, cx: &mut App) {
    match command {
        Command::TogglePanel(anchor) => crate::quick_panel::toggle(anchor, cx),
        Command::Open => crate::lifecycle::open_main_window(cx),
        Command::Lock => {
            crate::lifecycle::app(cx).update(cx, |app, cx| app.lock(cx));
        }
        Command::Quit => cx.quit(),
    }
}

/// Rasterizes the bundled honeycomb outline into an AppKit template image, so
/// the menu bar tints it for light, dark and highlighted states.
fn template_image() -> Option<Retained<NSImage>> {
    let png = honeycomb_png()?;
    let image = NSImage::initWithData(NSImage::alloc(), &NSData::with_bytes(&png))?;
    image.setSize(NSSize::new(ICON_POINTS, ICON_POINTS));
    image.setTemplate(true);
    Some(image)
}

fn honeycomb_png() -> Option<Vec<u8>> {
    let tree = usvg::Tree::from_data(Icon::Honeycomb.bytes(), &usvg::Options::default()).ok()?;
    let pixels = ICON_POINTS as u32 * ICON_SCALE;
    let mut pixmap = tiny_skia::Pixmap::new(pixels, pixels)?;
    let scale = pixels as f32 / tree.size().width();
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    let rgba = image::RgbaImage::from_raw(pixels, pixels, pixmap.take())?;
    let mut png = Cursor::new(Vec::new());
    rgba.write_to(&mut png, image::ImageFormat::Png).ok()?;
    Some(png.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn honeycomb_rasterizes_to_a_retina_template() {
        let png = honeycomb_png().expect("honeycomb rasterizes");
        let image = image::load_from_memory(&png).expect("valid PNG").to_rgba8();
        let side = ICON_POINTS as u32 * ICON_SCALE;
        assert_eq!(image.dimensions(), (side, side));
        let drawn = image.pixels().filter(|pixel| pixel[3] > 0).count();
        assert!(drawn > 0, "outline must be visible");
        assert!(
            drawn < (side * side) as usize / 2,
            "outline, not a filled block"
        );
        // Template images carry shape in alpha only; AppKit supplies the tint.
        assert!(
            image
                .pixels()
                .all(|pixel| pixel[0] == 0 && pixel[1] == 0 && pixel[2] == 0)
        );
    }
}
