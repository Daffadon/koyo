use std::sync::{Arc, LazyLock};

use gpui_kit::component::{ActiveTheme, Theme, ThemeMode, ThemeToken};
use gpui_kit::*;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum ThemeChoice {
    #[default]
    Dark,
    Light,
    Kimi,
}

impl ThemeChoice {
    pub(crate) const ALL: [Self; 3] = [Self::Dark, Self::Light, Self::Kimi];

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Dark => "dark",
            Self::Light => "light",
            Self::Kimi => "kimi",
        }
    }

    pub(crate) fn from_persisted(value: &str) -> Self {
        match value {
            "light" => Self::Light,
            "kimi" => Self::Kimi,
            _ => Self::Dark,
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Dark => "Dark",
            Self::Light => "Light",
            Self::Kimi => "Kimi no Nawa",
        }
    }

    pub(crate) fn description(self) -> &'static str {
        match self {
            Self::Dark => "Black surfaces with the comet action gradient.",
            Self::Light => "Pale twilight surfaces with a twilight blue action color.",
            Self::Kimi => "Deep twilight gradient with comet blue, lavender, and soft pink light.",
        }
    }

    pub(crate) fn preview_colors(self) -> [gpui_kit::Hsla; 3] {
        let colors = match self {
            Self::Dark => [0x050505, 0x080808, 0x5FE7FF],
            Self::Light => [0xF4F2FC, 0xECE9F7, 0x2E55C7],
            Self::Kimi => [0x05060F, 0x1B2A6B, 0x7FD1F2],
        };
        colors.map(|hex| gpui_kit::rgb(hex).into())
    }

    pub(crate) fn preview_gradient(self) -> [gpui_kit::Hsla; 2] {
        let colors = match self {
            Self::Dark => [0x5FE7FF, 0xA78BFA],
            Self::Light => [0x2E55C7, 0x6B5CE7],
            Self::Kimi => [0x7FD1F2, 0xB48AD3],
        };
        colors.map(|hex| gpui_kit::rgb(hex).into())
    }
}

/// Generated 1920x1080 backdrop: blurred, dark twilight with a comet band.
static KIMI_AURORA: LazyLock<Arc<gpui_kit::Image>> = LazyLock::new(|| {
    Arc::new(gpui_kit::Image::from_bytes(
        gpui_kit::ImageFormat::Webp,
        include_bytes!("../../assets/kimi-aurora.webp").to_vec(),
    ))
});

pub(crate) fn kimi_aurora() -> gpui_kit::ImageSource {
    gpui_kit::ImageSource::Image(KIMI_AURORA.clone())
}

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
    pub canvas: gpui_kit::Background,
    pub sidebar_gradient: gpui_kit::Background,
    pub selection: gpui_kit::Background,
    pub editor: gpui_kit::Background,
}

pub(crate) fn palette(choice: ThemeChoice, cx: &App) -> Palette {
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
        canvas: match choice {
            ThemeChoice::Dark => gradient(180.0, theme.background, tone(0x0A0A0A)),
            ThemeChoice::Light => gradient(160.0, theme.background, theme.accent),
            ThemeChoice::Kimi => gradient(
                180.0,
                tone(0x000000).opacity(0.0),
                tone(0x000000).opacity(0.0),
            ),
        },
        sidebar_gradient: match choice {
            ThemeChoice::Kimi => gradient(
                180.0,
                tone(0x05060F).opacity(0.86),
                tone(0x05060F).opacity(0.66),
            ),
            _ => gradient(180.0, theme.sidebar, theme.background),
        },
        selection: match choice {
            ThemeChoice::Kimi => gradient(
                135.0,
                tone(0x3E4C8A).opacity(0.88),
                tone(0x05060F).opacity(0.32),
            ),
            _ => gradient(135.0, theme.accent, theme.background),
        },
        editor: match choice {
            ThemeChoice::Dark => gradient(180.0, theme.background, tone(0x0A0A0A)),
            ThemeChoice::Light => gradient(160.0, theme.secondary, theme.accent),
            ThemeChoice::Kimi => gradient(
                180.0,
                tone(0x05060F).opacity(0.24),
                tone(0x05060F).opacity(0.24),
            ),
        },
    }
}

pub(crate) fn apply_theme(choice: ThemeChoice, cx: &mut App) {
    let mode = match choice {
        ThemeChoice::Dark | ThemeChoice::Kimi => ThemeMode::Dark,
        ThemeChoice::Light => ThemeMode::Light,
    };
    Theme::change(mode, None, cx);
    Theme::update(cx, |theme| match choice {
        ThemeChoice::Dark => apply_dark(theme),
        ThemeChoice::Light => apply_light(theme),
        ThemeChoice::Kimi => apply_kimi(theme),
    });
}

fn tone(hex: u32) -> gpui_kit::Hsla {
    gpui_kit::rgb(hex).into()
}

fn gradient(angle: f32, from: gpui_kit::Hsla, to: gpui_kit::Hsla) -> gpui_kit::Background {
    linear_gradient(
        angle,
        linear_color_stop(from, 0.0),
        linear_color_stop(to, 1.0),
    )
}

fn apply_dark(theme: &mut Theme) {
    let colors = &mut theme.colors;
    colors.background = tone(0x050505);
    colors.foreground = tone(0xF2F2F2);
    colors.muted = tone(0x141414);
    colors.muted_foreground = tone(0xA8A8A8);
    colors.border = tone(0x2A2A2A);

    colors.sidebar = tone(0x080808);
    colors.sidebar_border = tone(0x242424);
    colors.sidebar_foreground = tone(0xF2F2F2);
    colors.sidebar_accent = tone(0x1E1830);
    colors.sidebar_accent_foreground = tone(0x8FF0FF);

    colors.secondary = tone(0x101010);
    colors.secondary_foreground = tone(0xF2F2F2);
    colors.secondary_hover = tone(0x181818);
    colors.secondary_active = tone(0x202020);

    colors.primary = tone(0x5FE7FF);
    colors.primary_hover = tone(0x8AF0FF);
    colors.primary_active = tone(0x35CBE8);
    colors.primary_foreground = tone(0x04122E);
    colors.accent = tone(0x1E1830);
    colors.accent_foreground = tone(0xF3D9FF);
    colors.input = tone(0x707070);
    colors.ring = tone(0xA78BFA);
    colors.caret = tone(0x5FE7FF);

    colors.button = tone(0x141414);
    colors.button_hover = tone(0x1C1C1C);
    colors.button_active = tone(0x242424);
    colors.button_foreground = tone(0xF2F2F2);
    colors.button_primary = tone(0x5FE7FF);
    colors.button_primary_hover = tone(0x8AF0FF);
    colors.button_primary_active = tone(0x35CBE8);
    colors.button_primary_foreground = tone(0x04122E);
    colors.button_secondary = tone(0x181818);
    colors.button_secondary_hover = tone(0x202020);
    colors.button_secondary_active = tone(0x282828);
    colors.button_secondary_foreground = tone(0xE6E2FF);

    colors.popover = tone(0x0A0A0A);
    colors.popover_foreground = tone(0xF2F2F2);
    colors.list = tone(0x0A0A0A);
    colors.list_hover = tone(0x181818);
    colors.list_active = tone(0x242424);
    colors.list_even = tone(0x0C0C0C);
    colors.list_head = tone(0x111111);

    colors.success = tone(0x6EE7B7);
    colors.success_foreground = tone(0x06251A);
    colors.warning = tone(0xFFD166);
    colors.warning_foreground = tone(0x2A1C00);
    colors.danger = tone(0xFF6B8A);
    colors.danger_foreground = tone(0x2A0710);
    colors.link = tone(0x8CEBFF);
    colors.link_hover = tone(0xB5F4FF);
    colors.link_active = tone(0x5FE7FF);

    colors.scrollbar = tone(0x050505);
    colors.scrollbar_thumb = tone(0x2A2A2A);
    colors.scrollbar_thumb_hover = tone(0x3A3A3A);

    colors.title_bar = tone(0x080808);
    colors.title_bar_border = tone(0x242424);
    colors.status_bar = tone(0x080808);
    colors.status_bar_border = tone(0x242424);
    colors.tab_bar = tone(0x080808);
    colors.tab = tone(0x101010);
    colors.tab_active = tone(0x242424);
    colors.tab_active_foreground = tone(0xF2F2F2);
    colors.tab_foreground = tone(0xA8A8A8);

    colors.table = tone(0x0A0A0A);
    colors.table_hover = tone(0x181818);
    colors.table_active = tone(0x242424);
    colors.table_even = tone(0x0C0C0C);
    colors.table_head = tone(0x111111);
    colors.table_head_foreground = tone(0xDDDDDD);
    colors.table_foot = tone(0x080808);
    colors.table_foot_foreground = tone(0xA8A8A8);
    colors.table_row_border = tone(0x242424);

    colors.switch = tone(0x2A2A2A);
    colors.switch_thumb = tone(0xF2F2F2);
    colors.selection = tone(0x3A2A5A);

    let primary = colors.primary;
    let primary_hover = colors.primary_hover;
    let primary_active = colors.primary_active;
    let tab_bar = colors.tab_bar;
    let tab_active = colors.tab_active;
    theme.tokens.button_primary =
        ThemeToken::new(primary, gradient(135.0, primary, tone(0xA78BFA)));
    theme.tokens.button_primary_hover = ThemeToken::new(
        primary_hover,
        gradient(135.0, primary_hover, tone(0xC4A6FF)),
    );
    theme.tokens.button_primary_active = ThemeToken::new(
        primary_active,
        gradient(135.0, primary_active, tone(0x9A77F0)),
    );
    theme.tokens.tab_bar = ThemeToken::new(tab_bar, gradient(180.0, tab_bar, tone(0x050505)));
    theme.tokens.tab_active =
        ThemeToken::new(tab_active, gradient(180.0, tab_active, tone(0x050505)));

    let highlight = Arc::make_mut(&mut theme.highlight_theme);
    highlight.style.editor_background = Some(tone(0x000000).opacity(0.85));
    highlight.style.editor_foreground = Some(tone(0xF2F2F2));
    highlight.style.editor_active_line = Some(tone(0x1A1A1A).opacity(0.60));
    highlight.style.editor_line_number = Some(tone(0x8A8A8A));
    highlight.style.editor_active_line_number = Some(tone(0xF2F2F2));
    highlight.style.editor_invisible = Some(tone(0x707070));
    highlight.style.editor_gutter_background = Some(tone(0x000000).opacity(0.50));
}

fn apply_light(theme: &mut Theme) {
    let colors = &mut theme.colors;
    colors.background = tone(0xF4F2FC);
    colors.foreground = tone(0x1A1A45);
    colors.muted = tone(0xE8E5F4);
    colors.muted_foreground = tone(0x5D5D80);
    colors.border = tone(0xD2CDE6);

    colors.sidebar = tone(0xECE9F7);
    colors.sidebar_border = tone(0xD2CDE6);
    colors.sidebar_foreground = tone(0x242447);
    colors.sidebar_accent = tone(0xDEE4FA);
    colors.sidebar_accent_foreground = tone(0x243D8F);

    colors.secondary = tone(0xFAF9FF);
    colors.secondary_foreground = tone(0x1A1A45);
    colors.secondary_hover = tone(0xF0EDFA);
    colors.secondary_active = tone(0xE6E1F3);

    colors.primary = tone(0x2E55C7);
    colors.primary_hover = tone(0x3F68DD);
    colors.primary_active = tone(0x2346AD);
    colors.primary_foreground = tone(0xFFFFFF);
    colors.accent = tone(0xDCE6FF);
    colors.accent_foreground = tone(0x183B86);
    colors.input = tone(0x77739A);
    colors.ring = tone(0x6B5CE7);
    colors.caret = tone(0x2E55C7);

    colors.button = tone(0xECE9F7);
    colors.button_hover = tone(0xE2DEF2);
    colors.button_active = tone(0xD8D3EB);
    colors.button_foreground = tone(0x1A1A45);
    colors.button_primary = tone(0x2E55C7);
    colors.button_primary_hover = tone(0x3F68DD);
    colors.button_primary_active = tone(0x2346AD);
    colors.button_primary_foreground = tone(0xFFFFFF);
    colors.button_secondary = tone(0xE8E5F4);
    colors.button_secondary_hover = tone(0xDFDAF0);
    colors.button_secondary_active = tone(0xD5CFE9);
    colors.button_secondary_foreground = tone(0x1A1A45);

    colors.popover = tone(0xFFFFFF);
    colors.popover_foreground = tone(0x1A1A45);
    colors.list = tone(0xFAF9FF);
    colors.list_hover = tone(0xF0EDFA);
    colors.list_active = tone(0xE2E9FF);
    colors.list_even = tone(0xF7F5FE);
    colors.list_head = tone(0xEFECF8);

    colors.success = tone(0x2C765D);
    colors.success_foreground = tone(0xF7FFFB);
    colors.warning = tone(0x7A5222);
    colors.warning_foreground = tone(0xFFF9ED);
    colors.danger = tone(0xA23F5E);
    colors.danger_foreground = tone(0xFFF7F9);
    colors.link = tone(0x2E55C7);
    colors.link_hover = tone(0x3F68DD);
    colors.link_active = tone(0x2346AD);

    colors.scrollbar = tone(0xF4F2FC);
    colors.scrollbar_thumb = tone(0xC9C3DE);
    colors.scrollbar_thumb_hover = tone(0xB4ADCE);

    colors.title_bar = tone(0xECE9F7);
    colors.title_bar_border = tone(0xD2CDE6);
    colors.status_bar = tone(0xECE9F7);
    colors.status_bar_border = tone(0xD2CDE6);
    colors.tab_bar = tone(0xECE9F7);
    colors.tab = tone(0xFAF9FF);
    colors.tab_active = tone(0xE2E9FF);
    colors.tab_active_foreground = tone(0x1A1A45);
    colors.tab_foreground = tone(0x5D5D80);

    colors.table = tone(0xFAF9FF);
    colors.table_hover = tone(0xF0EDFA);
    colors.table_active = tone(0xE2E9FF);
    colors.table_even = tone(0xF7F5FE);
    colors.table_head = tone(0xEFECF8);
    colors.table_head_foreground = tone(0x3D3D63);
    colors.table_foot = tone(0xECE9F7);
    colors.table_foot_foreground = tone(0x5D5D80);
    colors.table_row_border = tone(0xE4E0F1);

    colors.switch = tone(0xC9C3DE);
    colors.switch_thumb = tone(0xFFFFFF);
    colors.selection = tone(0xC9D7FF);

    let primary = colors.primary;
    let primary_hover = colors.primary_hover;
    let primary_active = colors.primary_active;
    let tab_bar = colors.tab_bar;
    let tab_active = colors.tab_active;
    theme.tokens.button_primary =
        ThemeToken::new(primary, gradient(135.0, primary, tone(0x6B5CE7)));
    theme.tokens.button_primary_hover = ThemeToken::new(
        primary_hover,
        gradient(135.0, primary_hover, tone(0x6B5CE7)),
    );
    theme.tokens.button_primary_active = ThemeToken::new(
        primary_active,
        gradient(135.0, primary_active, tone(0x5A4ACB)),
    );
    theme.tokens.tab_bar = ThemeToken::new(tab_bar, gradient(180.0, tab_bar, tone(0xF4F2FC)));
    theme.tokens.tab_active =
        ThemeToken::new(tab_active, gradient(180.0, tab_active, tone(0xF4F2FC)));

    let highlight = Arc::make_mut(&mut theme.highlight_theme);
    highlight.style.editor_background = Some(tone(0xFAF9FF).opacity(0.70));
    highlight.style.editor_foreground = Some(tone(0x1A1A45));
    highlight.style.editor_active_line = Some(tone(0xF0EDFA).opacity(0.60));
    highlight.style.editor_line_number = Some(tone(0x5D5D80));
    highlight.style.editor_active_line_number = Some(tone(0x1A1A45));
    highlight.style.editor_invisible = Some(tone(0x77739A));
    highlight.style.editor_gutter_background = Some(tone(0xFAF9FF).opacity(0.50));
}

fn apply_kimi(theme: &mut Theme) {
    let colors = &mut theme.colors;
    colors.background = tone(0x05060F);
    colors.foreground = tone(0xEEF0FF);
    colors.muted = tone(0x0C1020);
    colors.muted_foreground = tone(0xA9A8C4);
    colors.border = tone(0x262B45);

    colors.sidebar = tone(0x070A16);
    colors.sidebar_border = tone(0x252B45);
    colors.sidebar_foreground = tone(0xEBEDFF);
    colors.sidebar_accent = tone(0x1E2450);
    colors.sidebar_accent_foreground = tone(0x9FDCF2);

    colors.secondary = tone(0x0A0E1E);
    colors.secondary_foreground = tone(0xF2F2FF);
    colors.secondary_hover = tone(0x12182C);
    colors.secondary_active = tone(0x1A2140);

    colors.primary = tone(0x7FD1F2);
    colors.primary_hover = tone(0xA6E4FF);
    colors.primary_active = tone(0x57B6DD);
    colors.primary_foreground = tone(0x071426);
    colors.accent = tone(0x2B235C);
    colors.accent_foreground = tone(0xF1E9FF);
    colors.input = tone(0x6F78A8);
    colors.ring = tone(0xB48AD3);
    colors.caret = tone(0x7FD1F2);

    colors.button = tone(0x0B1020);
    colors.button_hover = tone(0x151B32);
    colors.button_active = tone(0x1E2542);
    colors.button_foreground = tone(0xF2F2FF);
    colors.button_primary = tone(0x7FD1F2);
    colors.button_primary_hover = tone(0xA6E4FF);
    colors.button_primary_active = tone(0x57B6DD);
    colors.button_primary_foreground = tone(0x071426);
    colors.button_secondary = tone(0x141A30);
    colors.button_secondary_hover = tone(0x1C2440);
    colors.button_secondary_active = tone(0x252E50);
    colors.button_secondary_foreground = tone(0xE6E9FF);

    colors.popover = tone(0x070A14);
    colors.popover_foreground = tone(0xF2F2FF);
    colors.list = tone(0x080C18);
    colors.list_hover = tone(0x151B32);
    colors.list_active = tone(0x1E2542);
    colors.list_even = tone(0x0A0F1E);
    colors.list_head = tone(0x0E1428);

    colors.success = tone(0x7CC6B5);
    colors.success_foreground = tone(0x04201B);
    colors.warning = tone(0xF7B267);
    colors.warning_foreground = tone(0x2A1A05);
    colors.danger = tone(0xF28AB2);
    colors.danger_foreground = tone(0x2A0714);
    colors.link = tone(0x9FDCF2);
    colors.link_hover = tone(0xC8EEFF);
    colors.link_active = tone(0x7FD1F2);

    colors.scrollbar = tone(0x05060F);
    colors.scrollbar_thumb = tone(0x262B45);
    colors.scrollbar_thumb_hover = tone(0x3E4C8A);

    colors.title_bar = tone(0x070A16);
    colors.title_bar_border = tone(0x252B45);
    colors.status_bar = tone(0x070A16);
    colors.status_bar_border = tone(0x252B45);
    colors.tab_bar = tone(0x070A16);
    colors.tab = tone(0x0A0E1E);
    colors.tab_active = tone(0x1A2140);
    colors.tab_active_foreground = tone(0xF2F2FF);
    colors.tab_foreground = tone(0xA9A8C4);

    colors.table = tone(0x080C18);
    colors.table_hover = tone(0x151B32);
    colors.table_active = tone(0x1E2542);
    colors.table_even = tone(0x0A0F1E);
    colors.table_head = tone(0x0E1428);
    colors.table_head_foreground = tone(0xD9DDFF);
    colors.table_foot = tone(0x070A16);
    colors.table_foot_foreground = tone(0xA9A8C4);
    colors.table_row_border = tone(0x232A45);

    colors.switch = tone(0x262B45);
    colors.switch_thumb = tone(0xF2F4FF);
    colors.selection = tone(0x3E4C8A);

    let primary = colors.primary;
    let primary_hover = colors.primary_hover;
    let primary_active = colors.primary_active;
    let tab_bar = colors.tab_bar;
    let tab_active = colors.tab_active;
    theme.tokens.button_primary =
        ThemeToken::new(primary, gradient(135.0, primary, tone(0xB48AD3)));
    theme.tokens.button_primary_hover = ThemeToken::new(
        primary_hover,
        gradient(135.0, primary_hover, tone(0xCBB2F2)),
    );
    theme.tokens.button_primary_active = ThemeToken::new(
        primary_active,
        gradient(135.0, primary_active, tone(0x9B7FC0)),
    );
    theme.tokens.tab_bar = ThemeToken::new(tab_bar, gradient(180.0, tab_bar, tone(0x05060F)));
    theme.tokens.tab_active =
        ThemeToken::new(tab_active, gradient(180.0, tab_active, tone(0x05060F)));

    let highlight = Arc::make_mut(&mut theme.highlight_theme);
    highlight.style.editor_background = Some(tone(0x05060F).opacity(0.24));
    highlight.style.editor_foreground = Some(tone(0xF2EEFF));
    highlight.style.editor_active_line = Some(tone(0x1A2140).opacity(0.55));
    highlight.style.editor_line_number = Some(tone(0xA9A8C4));
    highlight.style.editor_active_line_number = Some(tone(0xF2F2FF));
    highlight.style.editor_invisible = Some(tone(0x6F78A8));
    highlight.style.editor_gutter_background = Some(tone(0x05060F).opacity(0.16));
}

#[cfg(test)]
mod tests {
    use super::ThemeChoice;

    #[test]
    fn persisted_names_round_trip() {
        for choice in ThemeChoice::ALL {
            assert_eq!(ThemeChoice::from_persisted(choice.as_str()), choice);
        }
    }

    #[test]
    fn unknown_persisted_name_loads_dark() {
        assert_eq!(ThemeChoice::from_persisted(""), ThemeChoice::Dark);
        assert_eq!(ThemeChoice::from_persisted("paper"), ThemeChoice::Dark);
        assert_eq!(ThemeChoice::from_persisted("Dark"), ThemeChoice::Dark);
    }

    #[test]
    fn labels_match_the_contract() {
        assert_eq!(ThemeChoice::Dark.label(), "Dark");
        assert_eq!(ThemeChoice::Light.label(), "Light");
        assert_eq!(ThemeChoice::Kimi.label(), "Kimi no Nawa");
    }

    #[test]
    fn descriptions_match_the_contract() {
        assert_eq!(
            ThemeChoice::Dark.description(),
            "Black surfaces with the comet action gradient."
        );
        assert_eq!(
            ThemeChoice::Light.description(),
            "Pale twilight surfaces with a twilight blue action color."
        );
        assert_eq!(
            ThemeChoice::Kimi.description(),
            "Deep twilight gradient with comet blue, lavender, and soft pink light."
        );
    }
}
