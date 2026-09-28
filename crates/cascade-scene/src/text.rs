//! Text measurement for sizing nodes.
//!
//! Views use a monospace font so that sizes computed here match what every
//! backend draws (GPUI, SVG, PNG) without a shared font system.

pub trait TextMeasure {
    /// Advance width of `text` at `font_size` logical pixels.
    fn width(&self, text: &str, font_size: f32) -> f32;

    /// Line height at `font_size`.
    fn line_height(&self, font_size: f32) -> f32 {
        font_size * 1.3
    }
}

/// Fixed advance per character, as a fraction of the font size.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MonoMeasure {
    pub advance_em: f32,
}

impl Default for MonoMeasure {
    fn default() -> Self {
        Self { advance_em: 0.6 }
    }
}

impl TextMeasure for MonoMeasure {
    fn width(&self, text: &str, font_size: f32) -> f32 {
        let chars = u16::try_from(text.chars().count()).unwrap_or(u16::MAX);
        f32::from(chars) * font_size * self.advance_em
    }
}
