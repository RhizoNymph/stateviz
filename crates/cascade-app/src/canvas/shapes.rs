//! Pure geometry for painting scene shapes in screen space.

use cascade_scene::Dash;

use crate::viewport::{ScreenPoint, ScreenRect};

/// Scene units a hexagon's side points stick out, as a fraction of height:
/// `tan(30°) / 2` gives a regular flat-topped hexagon.
const HEX_SLOPE: f32 = 0.288_675;
/// A stub's corner radius in scene units.
pub const STUB_RADIUS: f32 = 8.0;
/// Gap between the two lines of a double border, in scene units.
pub const DOUBLE_BORDER_GAP: f32 = 3.0;
/// Smallest on-screen label size worth shaping; smaller text is skipped.
pub const MIN_LABEL_PX: f32 = 3.5;
/// Arrowhead length in scene units (scaled by stroke width).
const ARROW_LENGTH: f32 = 9.0;

/// Corners of a flat-topped hexagon filling `r`.
pub fn hexagon(r: ScreenRect) -> [ScreenPoint; 6] {
    let inset = (r.height * HEX_SLOPE).min(r.width / 4.0);
    let mid = r.y + r.height / 2.0;
    [
        ScreenPoint::new(r.x + inset, r.y),
        ScreenPoint::new(r.right() - inset, r.y),
        ScreenPoint::new(r.right(), mid),
        ScreenPoint::new(r.right() - inset, r.bottom()),
        ScreenPoint::new(r.x + inset, r.bottom()),
        ScreenPoint::new(r.x, mid),
    ]
}

/// Corners of a tag: a rectangle with a pointed right end.
pub fn tag(r: ScreenRect) -> [ScreenPoint; 5] {
    let point = (r.height / 2.0).min(r.width / 3.0);
    let mid = r.y + r.height / 2.0;
    [
        ScreenPoint::new(r.x, r.y),
        ScreenPoint::new(r.right() - point, r.y),
        ScreenPoint::new(r.right(), mid),
        ScreenPoint::new(r.right() - point, r.bottom()),
        ScreenPoint::new(r.x, r.bottom()),
    ]
}

/// A filled triangle whose tip is at `tip`, pointing along `from → tip`.
/// `size` is its length in screen pixels. `None` for a zero-length segment.
pub fn arrowhead(from: ScreenPoint, tip: ScreenPoint, size: f32) -> Option<[ScreenPoint; 3]> {
    let (dx, dy) = (tip.x - from.x, tip.y - from.y);
    let len = (dx * dx + dy * dy).sqrt();
    if len < 1e-3 || size <= 0.0 {
        return None;
    }
    let (ux, uy) = (dx / len, dy / len);
    let half = size * 0.45;
    let base = ScreenPoint::new(tip.x - ux * size, tip.y - uy * size);
    Some([
        tip,
        ScreenPoint::new(base.x - uy * half, base.y + ux * half),
        ScreenPoint::new(base.x + uy * half, base.y - ux * half),
    ])
}

/// Arrowhead length on screen for a stroke of `width` scene units.
pub fn arrow_size(width: f32, zoom: f32) -> f32 {
    (ARROW_LENGTH + width * 2.0) * zoom
}

/// Where the last segment of a polyline starts, skipping repeated end
/// points, so the arrow direction is defined.
pub fn last_segment(points: &[ScreenPoint]) -> Option<(ScreenPoint, ScreenPoint)> {
    let tip = *points.last()?;
    let from = points.iter().rev().skip(1).find(|p| p.distance(tip) > 1e-3)?;
    Some((*from, tip))
}

/// Dash lengths on screen (`[on, off]`), or `None` for a solid line.
/// Dash lengths are in scene units and scale with zoom; dots are as long as
/// the line is wide.
pub fn dash_pattern(dash: Dash, width_px: f32, zoom: f32) -> Option<[f32; 2]> {
    match dash {
        Dash::Solid => None,
        Dash::Dashed { on, off } => {
            let (on, off) = ((on * zoom).max(1.0), (off * zoom).max(1.0));
            Some([on, off])
        }
        Dash::Dotted => {
            let dot = width_px.max(1.0);
            Some([dot, dot * 2.0])
        }
    }
}

/// On-screen stroke width: scaled, but never thinner than a hairline.
pub fn stroke_px(width: f32, zoom: f32) -> f32 {
    (width * zoom).max(0.75)
}

/// Font sizes are rounded to this step on screen, so a zoom gesture reuses
/// shaped lines from GPUI's line cache instead of reshaping every label on
/// every frame.
pub const FONT_STEP_PX: f32 = 0.25;

/// `font_px` rounded to [`FONT_STEP_PX`].
pub fn quantize_font(font_px: f32) -> f32 {
    ((font_px / FONT_STEP_PX).round() * FONT_STEP_PX).max(FONT_STEP_PX)
}

/// Bounding box of screen points.
pub fn bounds(points: &[ScreenPoint]) -> Option<ScreenRect> {
    let first = points.first()?;
    let (mut l, mut t, mut r, mut b) = (first.x, first.y, first.x, first.y);
    for p in points {
        l = l.min(p.x);
        t = t.min(p.y);
        r = r.max(p.x);
        b = b.max(p.y);
    }
    Some(ScreenRect::new(l, t, r - l, b - t))
}

/// Whether anything within `slack` pixels of `r` is on the canvas.
pub fn visible(r: ScreenRect, canvas: ScreenRect, slack: f32) -> bool {
    r.inset(-slack).intersects(&canvas)
        || (r.width == 0.0 && r.height == 0.0 && canvas.inset(-slack).contains(ScreenPoint::new(r.x, r.y)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: ScreenPoint, b: ScreenPoint) -> bool {
        (a.x - b.x).abs() < 1e-3 && (a.y - b.y).abs() < 1e-3
    }

    #[test]
    fn hexagon_fits_its_rect() {
        let r = ScreenRect::new(0.0, 0.0, 100.0, 40.0);
        let h = hexagon(r);
        let b = bounds(&h).expect("points");
        assert_eq!(b, r);
        assert!(close(h[2], ScreenPoint::new(100.0, 20.0)));
        assert!(close(h[5], ScreenPoint::new(0.0, 20.0)));
        let narrow = hexagon(ScreenRect::new(0.0, 0.0, 8.0, 40.0));
        assert!(narrow[0].x <= narrow[1].x);
    }

    #[test]
    fn tag_points_right() {
        let t = tag(ScreenRect::new(10.0, 10.0, 60.0, 20.0));
        assert!(close(t[2], ScreenPoint::new(70.0, 20.0)));
        assert!(close(t[1], ScreenPoint::new(60.0, 10.0)));
        assert_eq!(bounds(&t), Some(ScreenRect::new(10.0, 10.0, 60.0, 20.0)));
    }

    #[test]
    fn arrowhead_points_along_the_segment() {
        let a = arrowhead(ScreenPoint::new(0.0, 0.0), ScreenPoint::new(10.0, 0.0), 4.0).expect("defined");
        assert!(close(a[0], ScreenPoint::new(10.0, 0.0)));
        assert!((a[1].x - 6.0).abs() < 1e-3 && (a[2].x - 6.0).abs() < 1e-3);
        assert!((a[1].y + a[2].y).abs() < 1e-3);
        assert!(arrowhead(ScreenPoint::new(1.0, 1.0), ScreenPoint::new(1.0, 1.0), 4.0).is_none());
    }

    #[test]
    fn last_segment_skips_duplicate_points() {
        let pts = [ScreenPoint::new(0.0, 0.0), ScreenPoint::new(5.0, 0.0), ScreenPoint::new(5.0, 0.0)];
        assert_eq!(last_segment(&pts), Some((ScreenPoint::new(0.0, 0.0), ScreenPoint::new(5.0, 0.0))));
        assert_eq!(last_segment(&pts[..1]), None);
    }

    #[test]
    fn dash_patterns() {
        assert_eq!(dash_pattern(Dash::Solid, 1.0, 1.0), None);
        assert_eq!(dash_pattern(Dash::Dashed { on: 6.0, off: 4.0 }, 1.0, 2.0), Some([12.0, 8.0]));
        assert_eq!(dash_pattern(Dash::Dashed { on: 6.0, off: 4.0 }, 1.0, 0.01), Some([1.0, 1.0]));
        assert_eq!(dash_pattern(Dash::Dotted, 2.0, 1.0), Some([2.0, 4.0]));
    }

    #[test]
    fn strokes_never_vanish() {
        assert_eq!(stroke_px(1.5, 2.0), 3.0);
        assert_eq!(stroke_px(1.5, 0.1), 0.75);
        assert!(arrow_size(1.5, 1.0) > arrow_size(1.5, 0.5));
    }

    #[test]
    fn font_sizes_snap_to_quarter_pixels() {
        assert_eq!(quantize_font(13.0), 13.0);
        assert_eq!(quantize_font(13.1), 13.0);
        assert_eq!(quantize_font(13.2), 13.25);
        assert_eq!(quantize_font(0.01), FONT_STEP_PX);
    }

    #[test]
    fn culling() {
        let canvas = ScreenRect::new(0.0, 0.0, 100.0, 100.0);
        assert!(visible(ScreenRect::new(90.0, 90.0, 20.0, 20.0), canvas, 0.0));
        assert!(!visible(ScreenRect::new(120.0, 0.0, 20.0, 20.0), canvas, 0.0));
        assert!(visible(ScreenRect::new(105.0, 0.0, 20.0, 20.0), canvas, 10.0));
        assert!(visible(ScreenRect::new(50.0, 0.0, 0.0, 30.0), canvas, 1.0));
    }
}
