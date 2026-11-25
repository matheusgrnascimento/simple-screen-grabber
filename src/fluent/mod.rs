//! The Windows 11 (Fluent) look: the theme palette, following the Windows
//! light/dark app theme, accent color, high contrast, and the transparency
//! and animation settings.

pub mod menus;
mod registry;

use windows::Win32::Graphics::Gdi::{
    GetSysColor, COLOR_GRAYTEXT, COLOR_HIGHLIGHT, COLOR_HIGHLIGHTTEXT, COLOR_WINDOW,
    COLOR_WINDOWTEXT, SYS_COLOR_INDEX,
};
use windows::Win32::UI::Accessibility::{HCF_HIGHCONTRASTON, HIGHCONTRASTW};
use windows::Win32::UI::WindowsAndMessaging::{
    SystemParametersInfoW, SPI_GETCLIENTAREAANIMATION, SPI_GETHIGHCONTRAST,
    SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS,
};

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

    /// This color blended toward `other` by `t` (0 = self, 1 = other).
    pub fn mix(self, other: Color, t: f32) -> Color {
        let f = |a: u8, b: u8| (f32::from(a) * (1.0 - t) + f32::from(b) * t).round() as u8;
        Color::rgb(f(self.r, other.r), f(self.g, other.g), f(self.b, other.b))
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
    pub accent_hover: Color,
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
    pub text_disabled: Color,
    /// Translucent hover and pressed fills of toolbar buttons.
    pub subtle_hover: Color,
    pub subtle_pressed: Color,
    /// Separators inside toolbars.
    pub divider: Color,
    /// Ring around color swatches, so white and black stay visible.
    pub swatch_ring: Color,
    /// The Acrylic tint over the blurred backdrop (its alpha is the
    /// material's luminosity opacity).
    pub acrylic_tint: Color,
    /// The elevation shadow under toolbars and flyouts.
    pub shadow: Color,
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
            let flyout = Color::rgb(0x2C, 0x2C, 0x2C);
            Palette {
                dark,
                scrim,
                accent,
                accent_hover: accent.mix(flyout, 0.1),
                on_accent: Color::rgb(0x00, 0x00, 0x00),
                handle: Color::rgb(0x45, 0x45, 0x45),
                handle_border: Color::rgba(0xFF, 0xFF, 0xFF, 0x24),
                flyout,
                flyout_border: Color::rgba(0xFF, 0xFF, 0xFF, 0x17),
                text: Color::rgb(0xFF, 0xFF, 0xFF),
                text_secondary: Color::rgb(0xC5, 0xC5, 0xC5),
                text_disabled: Color::rgb(0x78, 0x78, 0x78),
                subtle_hover: Color::rgba(0xFF, 0xFF, 0xFF, 0x0F),
                subtle_pressed: Color::rgba(0xFF, 0xFF, 0xFF, 0x0A),
                divider: Color::rgb(0x3D, 0x3D, 0x3D),
                swatch_ring: Color::rgba(0xFF, 0xFF, 0xFF, 0x4D),
                acrylic_tint: Color::rgba(0x2C, 0x2C, 0x2C, 0xD9),
                shadow: Color::rgba(0x00, 0x00, 0x00, 0x66),
            }
        } else {
            let flyout = Color::rgb(0xF9, 0xF9, 0xF9);
            Palette {
                dark,
                scrim,
                accent,
                accent_hover: accent.mix(flyout, 0.1),
                on_accent: Color::rgb(0xFF, 0xFF, 0xFF),
                handle: Color::rgb(0xFF, 0xFF, 0xFF),
                handle_border: Color::rgba(0x00, 0x00, 0x00, 0x33),
                flyout,
                flyout_border: Color::rgba(0x00, 0x00, 0x00, 0x1A),
                text: Color::rgb(0x1B, 0x1B, 0x1B),
                text_secondary: Color::rgb(0x5F, 0x5F, 0x5F),
                text_disabled: Color::rgb(0xA0, 0xA0, 0xA0),
                subtle_hover: Color::rgba(0x00, 0x00, 0x00, 0x0A),
                subtle_pressed: Color::rgba(0x00, 0x00, 0x00, 0x06),
                divider: Color::rgb(0xE5, 0xE5, 0xE5),
                swatch_ring: Color::rgba(0x00, 0x00, 0x00, 0x40),
                acrylic_tint: Color::rgba(0xFC, 0xFC, 0xFC, 0xCC),
                shadow: Color::rgba(0x00, 0x00, 0x00, 0x30),
            }
        }
    }
}

impl Palette {
    /// The palette of a high contrast theme: the user's system colors,
    /// solid, without translucency.
    pub fn high_contrast() -> Palette {
        let window = system_color(COLOR_WINDOW);
        let text = system_color(COLOR_WINDOWTEXT);
        let highlight = system_color(COLOR_HIGHLIGHT);
        let on_highlight = system_color(COLOR_HIGHLIGHTTEXT);
        let disabled = system_color(COLOR_GRAYTEXT);
        Palette {
            dark: (u32::from(window.r) + u32::from(window.g) + u32::from(window.b)) < 3 * 128,
            scrim: Color::rgba(0, 0, 0, 0x66),
            accent: highlight,
            accent_hover: highlight,
            on_accent: on_highlight,
            handle: window,
            handle_border: text,
            flyout: window,
            flyout_border: text,
            text,
            text_secondary: text,
            text_disabled: disabled,
            subtle_hover: highlight,
            subtle_pressed: highlight,
            divider: text,
            swatch_ring: text,
            acrylic_tint: window,
            shadow: Color::rgba(0, 0, 0, 0),
        }
    }
}

/// The look to draw with: the palette and the user's visual settings.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Theme {
    pub palette: Palette,
    /// Acrylic and shadows; off when "Transparency effects" is off or in
    /// high contrast.
    pub materials: bool,
    /// "Animation effects".
    pub animations: bool,
    pub high_contrast: bool,
}

impl Theme {
    /// The current user's theme and settings.
    pub fn load() -> Theme {
        let high_contrast = high_contrast_on();
        let palette = if high_contrast {
            Palette::high_contrast()
        } else {
            Palette::load()
        };
        let transparency = registry::dword(PERSONALIZE_KEY, "EnableTransparency") != Some(0);
        Theme {
            palette,
            materials: transparency && !high_contrast,
            animations: animations_on(),
            high_contrast,
        }
    }
}

/// A system color (`GetSysColor`).
fn system_color(index: SYS_COLOR_INDEX) -> Color {
    // SAFETY: plain query.
    let c = unsafe { GetSysColor(index) };
    Color::rgb(c as u8, (c >> 8) as u8, (c >> 16) as u8)
}

fn high_contrast_on() -> bool {
    let mut info = HIGHCONTRASTW {
        cbSize: size_of::<HIGHCONTRASTW>() as u32,
        ..Default::default()
    };
    // SAFETY: the structure is initialized with its size.
    let ok = unsafe {
        SystemParametersInfoW(
            SPI_GETHIGHCONTRAST,
            info.cbSize,
            Some((&mut info as *mut HIGHCONTRASTW).cast()),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    };
    ok.is_ok() && info.dwFlags.contains(HCF_HIGHCONTRASTON)
}

fn animations_on() -> bool {
    let mut on = windows::core::BOOL(1);
    // SAFETY: the setting is written to a BOOL.
    let ok = unsafe {
        SystemParametersInfoW(
            SPI_GETCLIENTAREAANIMATION,
            0,
            Some((&mut on as *mut windows::core::BOOL).cast()),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    };
    ok.is_err() || on.as_bool()
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
