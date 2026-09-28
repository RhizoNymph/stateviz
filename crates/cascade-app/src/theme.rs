//! Light/dark theme: the scene theme and matching app chrome.
//!
//! The scene [`Theme`] comes from `cascade-scene`; the chrome (panels,
//! toolbar, status bar) derives its neutrals from it, so both always
//! match. Chrome never uses hue for anything but severity, keeping hue
//! meaning "machine" on the canvas.

use cascade_scene::{Rgba, Theme, ThemeMode};
use gpui::{App, Global, Hsla};

/// What the user picked; `System` follows the window appearance.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ThemeChoice {
    #[default]
    System,
    Light,
    Dark,
}

impl ThemeChoice {
    /// System → the opposite of what the system shows → back to system.
    pub fn next(self, system: ThemeMode) -> ThemeChoice {
        match self {
            ThemeChoice::System => match system {
                ThemeMode::Light => ThemeChoice::Dark,
                ThemeMode::Dark => ThemeChoice::Light,
            },
            ThemeChoice::Light | ThemeChoice::Dark => ThemeChoice::System,
        }
    }

    pub fn resolve(self, system: ThemeMode) -> ThemeMode {
        match self {
            ThemeChoice::System => system,
            ThemeChoice::Light => ThemeMode::Light,
            ThemeChoice::Dark => ThemeMode::Dark,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            ThemeChoice::System => "Theme: system",
            ThemeChoice::Light => "Theme: light",
            ThemeChoice::Dark => "Theme: dark",
        }
    }
}

pub fn scene_theme(mode: ThemeMode) -> Theme {
    match mode {
        ThemeMode::Light => Theme::light(),
        ThemeMode::Dark => Theme::dark(),
    }
}

/// Convert a scene color, multiplying its alpha by `opacity`.
pub fn hsla(color: Rgba, opacity: f32) -> Hsla {
    let alpha = color.alpha_f32() * opacity.clamp(0.0, 1.0);
    gpui::Rgba { r: f32::from(color.r) / 255.0, g: f32::from(color.g) / 255.0, b: f32::from(color.b) / 255.0, a: alpha }
        .into()
}

/// Colors of the app chrome.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Chrome {
    pub mode: ThemeMode,
    pub background: Hsla,
    pub panel: Hsla,
    pub raised: Hsla,
    pub hover: Hsla,
    pub active: Hsla,
    pub border: Hsla,
    pub text: Hsla,
    pub muted: Hsla,
    pub error: Hsla,
    pub warning: Hsla,
    pub info: Hsla,
    pub added: Hsla,
}

impl Chrome {
    pub fn from_theme(theme: &Theme) -> Chrome {
        let bg = theme.background;
        let fg = theme.text;
        let warning = match theme.mode {
            ThemeMode::Light => Rgba::hex(0x9A6700),
            ThemeMode::Dark => Rgba::hex(0xD29922),
        };
        Chrome {
            mode: theme.mode,
            background: hsla(bg, 1.0),
            panel: hsla(bg.mix(fg, 0.035), 1.0),
            raised: hsla(bg.mix(fg, 0.07), 1.0),
            hover: hsla(bg.mix(fg, 0.12), 1.0),
            active: hsla(bg.mix(fg, 0.2), 1.0),
            border: hsla(theme.rule, 1.0),
            text: hsla(fg, 1.0),
            muted: hsla(theme.text_muted, 1.0),
            error: hsla(theme.finding, 1.0),
            warning: hsla(warning, 1.0),
            info: hsla(theme.text_muted, 1.0),
            added: hsla(theme.added, 1.0),
        }
    }
}

/// The chrome of the (single) window, readable by every view.
pub struct ActiveChrome(pub Chrome);

impl Global for ActiveChrome {}

pub fn chrome(cx: &App) -> Chrome {
    cx.try_global::<ActiveChrome>().map_or_else(|| Chrome::from_theme(&Theme::light()), |g| g.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn choice_cycles_through_the_opposite_of_the_system() {
        assert_eq!(ThemeChoice::System.next(ThemeMode::Light), ThemeChoice::Dark);
        assert_eq!(ThemeChoice::System.next(ThemeMode::Dark), ThemeChoice::Light);
        assert_eq!(ThemeChoice::Dark.next(ThemeMode::Light), ThemeChoice::System);
        assert_eq!(ThemeChoice::System.resolve(ThemeMode::Dark), ThemeMode::Dark);
        assert_eq!(ThemeChoice::Light.resolve(ThemeMode::Dark), ThemeMode::Light);
    }

    #[test]
    fn color_conversion_applies_opacity() {
        let c: gpui::Rgba = hsla(Rgba::hex(0xFF0000), 0.5).into();
        assert!((c.r - 1.0).abs() < 1e-3 && c.g.abs() < 1e-3 && (c.a - 0.5).abs() < 1e-3);
        let transparent: gpui::Rgba = hsla(Rgba::hex(0x00FF00).with_alpha(0.5), 0.5).into();
        assert!((transparent.a - 0.25).abs() < 1e-2);
        let clamped: gpui::Rgba = hsla(Rgba::hex(0x0000FF), 3.0).into();
        assert!((clamped.a - 1.0).abs() < 1e-3);
    }

    #[test]
    fn chrome_follows_the_theme() {
        let light = Chrome::from_theme(&Theme::light());
        let dark = Chrome::from_theme(&Theme::dark());
        assert_eq!(light.mode, ThemeMode::Light);
        assert!(light.background.l > dark.background.l);
        assert!(light.text.l < dark.text.l);
        assert_eq!(scene_theme(ThemeMode::Dark), Theme::dark());
    }
}
