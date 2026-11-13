//! Catppuccin Mocha, applied on top of gpui-ce's dark theme.
//!
//! Only the colors the inspector actually shows are overridden. The component
//! kit still owns radius, type, and focus rings.

use gpui_kit::component::{Theme, ThemeMode};
use gpui_kit::{rgb, rgb_to_hsla, App, Hsla};

pub fn apply_mocha(cx: &mut App) {
    Theme::change(ThemeMode::Dark, None, cx);
    Theme::update(cx, |theme| {
        let crust = paint(0x11111b);
        let mantle = paint(0x181825);
        let base = paint(0x1e1e2e);
        let surface0 = paint(0x313244);
        let surface1 = paint(0x45475a);
        let surface2 = paint(0x585b70);
        let overlay0 = paint(0x6c7086);
        let subtext = paint(0xa6adc8);
        let text = paint(0xcdd6f4);
        let blue = paint(0x89b4fa);
        let sapphire = paint(0x74c7ec);
        let mauve = paint(0xcba6f7);
        let green = paint(0xa6e3a1);
        let yellow = paint(0xf9e2af);
        let red = paint(0xf38ba8);

        theme.background = base;
        theme.foreground = text;
        theme.popover = mantle;
        theme.popover_foreground = text;
        theme.sidebar = mantle;
        theme.sidebar_foreground = text;
        theme.sidebar_accent = surface0;
        theme.sidebar_accent_foreground = text;
        theme.sidebar_border = surface1;
        theme.sidebar_primary = blue;
        theme.sidebar_primary_foreground = crust;
        theme.title_bar = mantle;
        theme.title_bar_border = surface1;
        theme.border = surface1;
        theme.input = surface0;
        theme.ring = blue;
        theme.muted = surface0;
        theme.muted_foreground = subtext;
        theme.accent = surface1;
        theme.accent_foreground = text;
        theme.secondary = surface0;
        theme.secondary_foreground = text;
        theme.secondary_hover = surface1;
        theme.secondary_active = surface2;
        theme.primary = blue;
        theme.primary_foreground = crust;
        theme.primary_hover = sapphire;
        theme.primary_active = mauve;
        theme.danger = red;
        theme.danger_foreground = crust;
        theme.danger_hover = red;
        theme.danger_active = red;
        theme.success = green;
        theme.success_foreground = crust;
        theme.warning = yellow;
        theme.warning_foreground = crust;
        theme.info = sapphire;
        theme.info_foreground = crust;
        // `Theme::list` is list layout settings. The color lives on `colors`.
        theme.colors.list = mantle;
        theme.list_hover = surface0;
        theme.list_active = surface1;
        theme.list_active_border = blue;
        theme.list_even = base;
        theme.list_head = crust;
        theme.selection = surface1;
        theme.caret = text;
        theme.link = blue;
        theme.link_hover = sapphire;
        theme.link_active = mauve;
        theme.scrollbar = crust;
        theme.scrollbar_thumb = surface2;
        theme.scrollbar_thumb_hover = overlay0;
        theme.button = surface0;
        theme.button_foreground = text;
        theme.button_hover = surface1;
        theme.button_active = surface2;
        theme.button_primary = blue;
        theme.button_primary_foreground = crust;
        theme.button_primary_hover = sapphire;
        theme.button_primary_active = mauve;
        theme.button_secondary = surface0;
        theme.button_secondary_foreground = text;
        theme.button_secondary_hover = surface1;
        theme.button_secondary_active = surface2;
        theme.button_danger = red;
        theme.button_danger_foreground = crust;
        theme.group_box = mantle;
        theme.group_box_foreground = text;
    });
}

fn paint(hex: u32) -> Hsla {
    rgb_to_hsla(rgb(hex))
}
