//! Chips: small rounded labels drawn over the scene (instance markers and
//! queue positions), as an `Overlay::Rect` plus an `Overlay::Text`.

use cascade_layout::{Point, Rect, Size};

use crate::color::Rgba;
use crate::scene::{FontWeight, HitTarget, Label, Layer, Overlay, Scene, Stroke};
use crate::views::style::Painter;

/// Horizontal text padding inside a chip.
const CHIP_PAD_X: f32 = 5.0;
/// Vertical text padding inside a chip.
const CHIP_PAD_Y: f32 = 1.0;

/// How a chip looks.
pub(super) struct ChipStyle {
    pub fill: Rgba,
    pub stroke: Stroke,
    pub text: Rgba,
    pub weight: FontWeight,
}

/// Size of a chip showing `text`: fully rounded, never narrower than tall.
pub(super) fn size(painter: &Painter<'_>, text: &str) -> Size {
    let font = painter.theme.small_font_size;
    let height = painter.line_height(font) + 2.0 * CHIP_PAD_Y;
    Size::new((painter.text_width(text, font) + 2.0 * CHIP_PAD_X).max(height), height)
}

/// Push a chip with its top-left at `at`.
pub(super) fn push(
    scene: &mut Scene,
    painter: &Painter<'_>,
    at: Point,
    text: &str,
    style: &ChipStyle,
    target: HitTarget,
    opacity: f32,
) {
    let size = size(painter, text);
    let rect = Rect::from_origin_size(at, size);
    let font = painter.theme.small_font_size;
    let width = painter.text_width(text, font);
    scene.overlays.push(Overlay::Rect {
        rect,
        fill: Some(style.fill),
        stroke: Some(style.stroke),
        radius: size.height / 2.0,
        opacity,
        layer: Layer::Over,
        target,
    });
    scene.overlays.push(Overlay::Text {
        label: Label {
            text: text.to_owned(),
            origin: Point::new(rect.center().x - width / 2.0, at.y + CHIP_PAD_Y),
            font_size: font,
            color: style.text,
            weight: style.weight,
        },
        opacity,
        layer: Layer::Over,
    });
}
