//! Semantic colors for the comparison UI, independent of legacy transfer themes.
use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tuirealm::ratatui::style::{Color, Style};

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub(super) enum Theme {
    #[default]
    CatppuccinMocha,
    TokyoNight,
    Terminal,
}

impl Theme {
    pub const ALL: [Self; 3] = [Self::CatppuccinMocha, Self::TokyoNight, Self::Terminal];

    pub fn name(self) -> &'static str {
        match self {
            Self::CatppuccinMocha => "Catppuccin Mocha",
            Self::TokyoNight => "Tokyo Night",
            Self::Terminal => "Terminal default",
        }
    }

    pub fn palette(self) -> Palette {
        match self {
            // https://catppuccin.com/palette/ (Mocha)
            Self::CatppuccinMocha => Palette {
                background: Color::Rgb(30, 30, 46),     // Base
                text: Color::Rgb(205, 214, 244),        // Text
                muted: Color::Rgb(147, 153, 178),       // Overlay 2
                border: Color::Rgb(108, 112, 134),      // Overlay 0
                accent: Color::Rgb(137, 180, 250),      // Blue
                source: Color::Rgb(148, 226, 213),      // Teal
                destination: Color::Rgb(203, 166, 247), // Mauve
                warning: Color::Rgb(249, 226, 175),     // Yellow
                error: Color::Rgb(243, 139, 168),       // Red
                selection: Color::Rgb(69, 71, 90),      // Surface 1
                changed: Color::Rgb(49, 50, 68),        // Surface 0
            },
            // Default variant: https://terminalcolors.com/themes/tokyo-night/
            // Exact values: /downloads/ghostty/tokyo-night-default on the same site.
            Self::TokyoNight => Palette {
                background: Color::Rgb(26, 27, 38),     // #1a1b26 background
                text: Color::Rgb(192, 202, 245),        // #c0caf5 foreground
                muted: Color::Rgb(169, 177, 214),       // #a9b1d6 ANSI white
                border: Color::Rgb(65, 72, 104),        // #414868 ANSI bright black
                accent: Color::Rgb(122, 162, 247),      // #7aa2f7 blue
                source: Color::Rgb(125, 207, 255),      // #7dcfff cyan
                destination: Color::Rgb(187, 154, 247), // #bb9af7 magenta
                warning: Color::Rgb(224, 175, 104),     // #e0af68 yellow
                error: Color::Rgb(247, 118, 142),       // #f7768e red
                selection: Color::Rgb(40, 52, 87),      // #283457 selection-background
                changed: Color::Rgb(21, 22, 30),        // #15161e ANSI black
            },
            Self::Terminal => Palette {
                background: Color::Reset,
                text: Color::Reset,
                muted: Color::DarkGray,
                border: Color::Reset,
                accent: Color::Cyan,
                source: Color::Cyan,
                destination: Color::Magenta,
                warning: Color::Yellow,
                error: Color::Red,
                selection: Color::DarkGray,
                changed: Color::Rgb(48, 36, 30),
            },
        }
    }

    /// Recolor the cached base16-ocean syntax tokens without parsing on redraw.
    pub fn syntax_color(self, color: [u8; 3]) -> Color {
        if self == Self::Terminal {
            return Color::Rgb(color[0], color[1], color[2]);
        }
        if self == Self::TokyoNight {
            // Keep syntax within the terminal palette, not Neovim's extra colors.
            let rgb = match color {
                [101, 115, 126] | [128, 128, 128] => [169, 177, 214], // Comments/markers
                [167, 173, 186] => [169, 177, 214],                   // Punctuation
                [171, 121, 103] => [187, 154, 247],                   // Escapes
                [180, 142, 173] => [187, 154, 247],                   // Keywords
                [163, 190, 140] => [158, 206, 106],                   // Strings
                [208, 135, 112] => [224, 175, 104],                   // Numbers/constants
                [235, 203, 139] => [125, 207, 255],                   // Types
                [143, 161, 179] => [122, 162, 247],                   // Functions
                [150, 181, 180] => [125, 207, 255],                   // Operators
                [191, 97, 106] => [247, 118, 142],                    // Variables/tags
                _ => [192, 202, 245],
            };
            return Color::Rgb(rgb[0], rgb[1], rgb[2]);
        }
        let rgb = match color {
            [101, 115, 126] | [128, 128, 128] => [147, 153, 178], // Comments/markers
            [167, 173, 186] => [186, 194, 222],                   // Punctuation
            [171, 121, 103] => [245, 194, 231],                   // Escapes
            [180, 142, 173] => [203, 166, 247],                   // Keywords
            [163, 190, 140] => [166, 227, 161],                   // Strings
            [208, 135, 112] => [250, 179, 135],                   // Numbers/constants
            [235, 203, 139] => [249, 226, 175],                   // Types
            [143, 161, 179] => [137, 180, 250],                   // Functions
            [150, 181, 180] => [148, 226, 213],                   // Operators
            [191, 97, 106] => [243, 139, 168],                    // Variables/tags
            _ => [205, 214, 244],
        };
        Color::Rgb(rgb[0], rgb[1], rgb[2])
    }
}

#[derive(Clone, Copy)]
pub(super) struct Palette {
    pub background: Color,
    pub text: Color,
    pub muted: Color,
    pub border: Color,
    pub accent: Color,
    pub source: Color,
    pub destination: Color,
    pub warning: Color,
    pub error: Color,
    pub selection: Color,
    pub changed: Color,
}

impl Palette {
    pub fn base(self) -> Style {
        Style::default().fg(self.text).bg(self.background)
    }
}

#[derive(Default, Deserialize, Serialize)]
#[serde(default)]
struct Preferences {
    theme: Theme,
}

pub(super) struct ThemeSettings {
    pub theme: Theme,
    path: PathBuf,
}

impl ThemeSettings {
    /// Invalid or future settings must never prevent opening the application.
    pub fn load(path: PathBuf) -> Self {
        let preferences: Preferences = fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        Self {
            theme: preferences.theme,
            path,
        }
    }

    /// Commit to memory only after the preference has been safely persisted.
    pub fn apply(&mut self, theme: Theme) -> io::Result<()> {
        let parent = self
            .path
            .parent()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "No settings directory"))?;
        let mut file = tempfile::NamedTempFile::new_in(parent)?;
        let bytes = serde_json::to_vec_pretty(&Preferences { theme })?;
        file.write_all(&bytes)?;
        file.as_file().sync_all()?;
        file.persist(&self.path).map_err(|error| error.error)?;
        self.theme = theme;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_and_invalid_settings_are_safe() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("comparison-ui.json");
        assert_eq!(
            ThemeSettings::load(path.clone()).theme,
            Theme::CatppuccinMocha
        );
        for content in ["{", "{}", r#"{"theme":"future-theme"}"#] {
            fs::write(&path, content).unwrap();
            assert_eq!(
                ThemeSettings::load(path.clone()).theme,
                Theme::CatppuccinMocha
            );
        }
    }

    #[test]
    fn persists_selection_and_preserves_it_when_a_write_fails() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("comparison-ui.json");
        let mut settings = ThemeSettings::load(path.clone());
        settings.apply(Theme::Terminal).unwrap();
        assert_eq!(ThemeSettings::load(path.clone()).theme, Theme::Terminal);
        settings.path = directory.path().join("missing").join("settings.json");
        assert!(settings.apply(Theme::CatppuccinMocha).is_err());
        assert_eq!(settings.theme, Theme::Terminal);
        assert_eq!(ThemeSettings::load(path).theme, Theme::Terminal);
    }

    #[test]
    fn all_themes_persist_without_changing_the_default() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("comparison-ui.json");
        let mut settings = ThemeSettings::load(path.clone());
        assert_eq!(settings.theme, Theme::CatppuccinMocha);
        for theme in Theme::ALL {
            settings.apply(theme).unwrap();
            assert_eq!(ThemeSettings::load(path.clone()).theme, theme);
        }
        assert_eq!(
            serde_json::to_string(&Theme::TokyoNight).unwrap(),
            r#""tokyo-night""#
        );
    }

    #[test]
    fn tokyo_night_matches_terminalcolors_default_palette() {
        let theme = Theme::TokyoNight;
        let palette = theme.palette();
        assert_eq!(theme.name(), "Tokyo Night");
        // Values from the reference site's Ghostty and Alacritty downloads.
        for (actual, expected) in [
            (palette.background, Color::Rgb(0x1a, 0x1b, 0x26)),
            (palette.text, Color::Rgb(0xc0, 0xca, 0xf5)),
            (palette.muted, Color::Rgb(0xa9, 0xb1, 0xd6)),
            (palette.border, Color::Rgb(0x41, 0x48, 0x68)),
            (palette.accent, Color::Rgb(0x7a, 0xa2, 0xf7)),
            (palette.source, Color::Rgb(0x7d, 0xcf, 0xff)),
            (palette.destination, Color::Rgb(0xbb, 0x9a, 0xf7)),
            (palette.warning, Color::Rgb(0xe0, 0xaf, 0x68)),
            (palette.error, Color::Rgb(0xf7, 0x76, 0x8e)),
            (palette.selection, Color::Rgb(0x28, 0x34, 0x57)),
            (palette.changed, Color::Rgb(0x15, 0x16, 0x1e)),
        ] {
            assert_eq!(actual, expected);
        }
        for (cached, expected) in [
            ([180, 142, 173], palette.destination),          // Keywords
            ([171, 121, 103], palette.destination),          // Escapes
            ([163, 190, 140], Color::Rgb(0x9e, 0xce, 0x6a)), // Strings (ANSI green)
            ([208, 135, 112], palette.warning),              // Numbers/constants
            ([235, 203, 139], palette.source),               // Types
            ([143, 161, 179], palette.accent),               // Functions
            ([150, 181, 180], palette.source),               // Operators
            ([191, 97, 106], palette.error),                 // Variables/tags
            ([101, 115, 126], palette.muted),                // Comments
            ([128, 128, 128], palette.muted),                // Markers
            ([167, 173, 186], palette.muted),                // Punctuation
            ([192, 197, 206], palette.text),                 // Plain text
        ] {
            assert_eq!(theme.syntax_color(cached), expected);
        }
    }

    #[test]
    fn mocha_uses_official_palette_and_recolors_syntax() {
        let theme = Theme::CatppuccinMocha;
        assert_eq!(theme.palette().background, Color::Rgb(30, 30, 46));
        assert_eq!(
            theme.syntax_color([180, 142, 173]),
            Color::Rgb(203, 166, 247)
        );
        assert_eq!(Theme::Terminal.syntax_color([1, 2, 3]), Color::Rgb(1, 2, 3));
    }
}
