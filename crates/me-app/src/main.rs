#[cfg(not(any(target_os = "macos", target_os = "linux")))]
compile_error!("ME supports macOS and Linux.");

use gpui::{
    App, Application, Bounds, Context, KeyBinding, Menu, MenuItem, TitlebarOptions, Window,
    WindowBounds, WindowOptions, actions, div, prelude::*, px, rgb, size,
};
use me_core::{APP_NAME, TAGLINE};

actions!(me, [Quit]);

struct MeApp;

impl Render for MeApp {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(0xf8f8f5))
            .text_color(rgb(0x20211e))
            .p_8()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(div().text_xl().child(APP_NAME))
                    .child(div().text_sm().text_color(rgb(0x747770)).child("Basic")),
            )
            .child(
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .justify_center()
                    .items_center()
                    .gap_4()
                    .child(div().text_size(px(44.)).child(TAGLINE))
                    .child(
                        div()
                            .text_base()
                            .text_color(rgb(0x747770))
                            .child("Ein Platz für deine persönlichen Daten."),
                    ),
            )
    }
}

fn main() {
    Application::new().run(|cx: &mut App| {
        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.bind_keys([KeyBinding::new(
            if cfg!(target_os = "macos") {
                "cmd-q"
            } else {
                "ctrl-q"
            },
            Quit,
            None,
        )]);
        cx.set_menus(vec![Menu {
            name: APP_NAME.into(),
            items: vec![MenuItem::action("ME beenden", Quit)],
        }]);

        // Until the menu-bar lifecycle is implemented, closing the last window
        // exits instead of leaving an unreachable background process running.
        cx.on_window_closed(|cx| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();

        let bounds = Bounds::centered(None, size(px(760.), px(520.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                window_min_size: Some(size(px(520.), px(360.))),
                titlebar: Some(TitlebarOptions {
                    title: Some(APP_NAME.into()),
                    ..Default::default()
                }),
                app_id: Some("me-desktop".into()),
                ..Default::default()
            },
            |_, cx| cx.new(|_| MeApp),
        )
        .expect("ME could not create its main window");

        cx.activate(true);
    });
}
