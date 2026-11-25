//! The Windows 11 (Fluent) look: the theme palette, following the Windows
//! light/dark app theme and accent color.

mod registry;

/// An sRGB color with straight (not premultiplied) alpha.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const fn rgb(r: u8, g: u8, b: u8) -> Color {
        Color { r, g, b, a: 0xFF }
    }

    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Color {
        Color { r, g, b, a }
    }
}

const PERSONALIZE_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize";
const ACCENT_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\Accent";
/// Forces a theme: `light` or `dark`.
const THEME_VARIABLE: &str = "SIMPLE_SCREEN_GRABBER_THEME";

/// Default accent colors of Windows 11 (the blue accent).
const DEFAULT_ACCENT_LIGHT: Color = Color::rgb(0x00, 0x5F, 0xB8); // SystemAccentColorDark1
const DEFAULT_ACCENT_DARK: Color = Color::rgb(0x60, 0xCD, 0xFF); // SystemAccentColorLight2

/// The colors of one theme, from the Windows 11 light and dark color sets.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Palette {
    pub dark: bool,
    /// Dims the screen outside the selection.
    pub scrim: Color,
    pub accent: Color,
    /// Text and icons on `accent`.
    pub on_accent: Color,
    /// Outer disc of the selection handles.
    pub handle: Color,
    pub handle_border: Color,
    /// Background of flyout-like surfaces (toolbars, labels).
    pub flyout: Color,
    pub flyout_border: Color,
    pub text: Color,
    pub text_secondary: Color,
}

impl Palette {
    /// The palette of the current user's theme and accent color.
    pub fn load() -> Palette {
        Palette::new(
            system_dark(),
            registry::binary(ACCENT_KEY, "AccentPalette").as_deref(),
        )
    }

    /// The palette of a theme. `accent_palette` is the `AccentPalette`
    /// registry value: 8 colors of 4 bytes (R, G, B, A), from the lightest
    /// shade (index 0, Light3) to the darkest (index 6, Dark3).
    pub fn new(dark: bool, accent_palette: Option<&[u8]>) -> Palette {
        // Accent fills use Dark1 in the light theme and Light2 in the dark.
        let (index, default) = if dark {
            (1, DEFAULT_ACCENT_DARK)
        } else {
            (4, DEFAULT_ACCENT_LIGHT)
        };
        let accent = match accent_palette {
            Some(p) if p.len() >= 32 => {
                Color::rgb(p[index * 4], p[index * 4 + 1], p[index * 4 + 2])
            }
            _ => default,
        };
        let scrim = Color::rgba(0, 0, 0, 0x66);
        if dark {
            Palette {
                dark,
                scrim,
                accent,
                on_accent: Color::rgb(0x00, 0x00, 0x00),
                handle: Color::rgb(0x45, 0x45, 0x45),
                handle_border: Color::rgba(0xFF, 0xFF, 0xFF, 0x24),
                flyout: Color::rgb(0x2C, 0x2C, 0x2C),
                flyout_border: Color::rgba(0xFF, 0xFF, 0xFF, 0x17),
                text: Color::rgb(0xFF, 0xFF, 0xFF),
                text_secondary: Color::rgb(0xC5, 0xC5, 0xC5),
            }
        } else {
            Palette {
                dark,
                scrim,
                accent,
                on_accent: Color::rgb(0xFF, 0xFF, 0xFF),
                handle: Color::rgb(0xFF, 0xFF, 0xFF),
                handle_border: Color::rgba(0x00, 0x00, 0x00, 0x33),
                flyout: Color::rgb(0xF9, 0xF9, 0xF9),
                flyout_border: Color::rgba(0x00, 0x00, 0x00, 0x1A),
                text: Color::rgb(0x1B, 0x1B, 0x1B),
                text_secondary: Color::rgb(0x5F, 0x5F, 0x5F),
            }
        }
    }
}

/// Whether the dark app theme is in use (or forced by the environment).
fn system_dark() -> bool {
    match std::env::var(THEME_VARIABLE).as_deref() {
        Ok("dark") => return true,
        Ok("light") => return false,
        _ => {}
    }
    registry::dword(PERSONALIZE_KEY, "AppsUseLightTheme") == Some(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accent_comes_from_the_palette_shade_of_the_theme() {
        let mut palette = [0u8; 32];
        palette[4..8].copy_from_slice(&[1, 2, 3, 0xFF]); // Light2
        palette[16..20].copy_from_slice(&[4, 5, 6, 0xFF]); // Dark1
        assert_eq!(
            Palette::new(true, Some(&palette)).accent,
            Color::rgb(1, 2, 3)
        );
        assert_eq!(
            Palette::new(false, Some(&palette)).accent,
            Color::rgb(4, 5, 6)
        );
    }

    #[test]
    fn missing_accent_uses_the_windows_default() {
        assert_eq!(Palette::new(false, None).accent, DEFAULT_ACCENT_LIGHT);
        assert_eq!(
            Palette::new(true, Some(&[0; 8])).accent,
            DEFAULT_ACCENT_DARK
        );
    }
}
