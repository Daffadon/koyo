use gpui_kit::component::{ActiveTheme, Theme};
use gpui_kit::*;

#[derive(Clone, Copy)]
pub(crate) struct Palette {
    pub background: gpui_kit::Hsla,
    pub foreground: gpui_kit::Hsla,
    pub muted: gpui_kit::Hsla,
    pub border: gpui_kit::Hsla,
    pub sidebar: gpui_kit::Hsla,
    pub sidebar_border: gpui_kit::Hsla,
    pub panel: gpui_kit::Hsla,
    pub panel_alt: gpui_kit::Hsla,
    pub accent: gpui_kit::Hsla,
    pub accent_soft: gpui_kit::Hsla,
    pub ring: gpui_kit::Hsla,
    pub success: gpui_kit::Hsla,
    pub warning: gpui_kit::Hsla,
    pub danger: gpui_kit::Hsla,
}

pub(crate) fn palette(cx: &App) -> Palette {
    let theme = cx.theme();
    Palette {
        background: theme.background,
        foreground: theme.foreground,
        muted: theme.muted_foreground,
        border: theme.border,
        sidebar: theme.sidebar,
        sidebar_border: theme.sidebar_border,
        panel: theme.secondary,
        panel_alt: theme.muted,
        accent: theme.primary,
        accent_soft: theme.accent,
        ring: theme.ring,
        success: theme.success,
        warning: theme.warning,
        danger: theme.danger,
    }
}

/// A warm, low-contrast light palette. The stock light theme is pure white
/// with saturated accents, which is harsh next to the dark theme; this keeps
/// paper-like surfaces, muted sage accents, and dark gray-green type.
pub(crate) fn apply_calm_light_theme(cx: &mut App) {
    fn tone(hex: u32) -> gpui_kit::Hsla {
        gpui_kit::rgb(hex).into()
    }

    Theme::update(cx, |theme| {
        let colors = &mut theme.colors;
        colors.background = tone(0xF2F0E9);
        colors.foreground = tone(0x30342F);
        colors.muted = tone(0xE7E3D8);
        colors.muted_foreground = tone(0x5F665D);
        colors.border = tone(0xD5D0C3);

        colors.sidebar = tone(0xEAE7DD);
        colors.sidebar_border = tone(0xD5D0C3);
        colors.sidebar_foreground = tone(0x3B403A);
        colors.sidebar_accent = tone(0xE0E6DE);
        colors.sidebar_accent_foreground = tone(0x355344);

        colors.secondary = tone(0xF8F6F0);
        colors.secondary_foreground = tone(0x30342F);
        colors.secondary_hover = tone(0xEFECE3);
        colors.secondary_active = tone(0xE6E1D5);

        colors.primary = tone(0x557A68);
        colors.primary_hover = tone(0x648B78);
        colors.primary_active = tone(0x4A6D5C);
        colors.primary_foreground = tone(0xFBF9F4);
        colors.accent = tone(0xDCE5DE);
        colors.accent_foreground = tone(0x3B5A4B);
        colors.input = tone(0x7A8177);
        colors.ring = tone(0x4C7463);
        colors.caret = tone(0x557A68);

        colors.button = tone(0xEAE7DD);
        colors.button_hover = tone(0xE1DDD2);
        colors.button_active = tone(0xD7D2C6);
        colors.button_foreground = tone(0x30342F);
        colors.button_primary = tone(0x557A68);
        colors.button_primary_hover = tone(0x648B78);
        colors.button_primary_active = tone(0x4A6D5C);
        colors.button_primary_foreground = tone(0xFBF9F4);
        colors.button_secondary = tone(0xE7E3D8);
        colors.button_secondary_hover = tone(0xDFDACE);
        colors.button_secondary_active = tone(0xD4CFC2);
        colors.button_secondary_foreground = tone(0x3D423C);

        colors.popover = tone(0xFBF9F4);
        colors.popover_foreground = tone(0x30342F);
        colors.list = tone(0xF8F6F0);
        colors.list_hover = tone(0xEFECE3);
        colors.list_active = tone(0xE4EBE5);
        colors.list_even = tone(0xF4F2EB);
        colors.list_head = tone(0xEEEAE0);

        colors.success = tone(0x4F7F63);
        colors.success_foreground = tone(0xF7FBF8);
        colors.warning = tone(0x7A5222);
        colors.warning_foreground = tone(0xFDF9F1);
        colors.danger = tone(0x7F4A43);
        colors.danger_foreground = tone(0xFFF8F7);
        colors.link = tone(0x4C7463);
        colors.link_hover = tone(0x3F6353);
        colors.link_active = tone(0x345547);

        colors.scrollbar = tone(0xE5E1D6);
        colors.scrollbar_thumb = tone(0xC4BEB0);
        colors.scrollbar_thumb_hover = tone(0xAFA898);
    });
}
