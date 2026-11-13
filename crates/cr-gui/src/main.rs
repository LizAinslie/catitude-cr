//! Read-only desktop inspector for crlib.
//!
//! `crgui` lists PC/SC readers, waits for a card, and renders the fields the
//! standard registry returns. PC/SC runs on a worker thread. The window is
//! gpui-ce (`gpui_ce_kit` 0.3): select, button, list row, and a status tag.

mod format;
mod model;
mod theme;
mod view;
mod worker;

use gpui_kit::prelude::*;
use gpui_kit::{point, px, size, Bounds, QuitMode, TitlebarOptions, WindowBounds, WindowOptions};

fn main() {
    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .with_quit_mode(QuitMode::LastWindowClosed)
        .run(|cx| {
            gpui_kit::init(cx);
            theme::apply_mocha(cx);

            gpui_kit::open_window(window_options(), cx, |window, cx| {
                cx.new(|cx| view::CrApp::new(window, cx))
            })
            .expect("open crgui window");
        });
}

fn window_options() -> WindowOptions {
    WindowOptions::default()
        .window_bounds(WindowBounds::Windowed(Bounds {
            origin: point(px(80.), px(60.)),
            size: size(px(960.), px(720.)),
        }))
        .titlebar(Some(TitlebarOptions {
            title: Some("crgui".into()),
            ..Default::default()
        }))
}
