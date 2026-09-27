//! Scene ↔ screen coordinates for the canvas.
//!
//! A [`Viewport`] names the scene point at the centre of the canvas and the
//! zoom (screen pixels per scene unit). Screen coordinates are window
//! coordinates in logical pixels, so the canvas' own position in the window
//! is part of every transform.

use cascade_layout::{Point, Rect};
use cascade_scene::Viewport;

/// Smallest and largest zoom the canvas allows.
pub const MIN_ZOOM: f32 = 0.05;
pub const MAX_ZOOM: f32 = 8.0;
/// Fitting never magnifies past this, so a tiny diagram is not blown up.
pub const FIT_MAX_ZOOM: f32 = 1.5;
/// Screen pixels kept free around the scene when fitting.
pub const FIT_MARGIN: f32 = 32.0;
/// Zoom factor of one `+`/`-` key press.
pub const KEY_ZOOM_STEP: f32 = 1.25;
/// Screen pixels one arrow key press pans.
pub const KEY_PAN_STEP: f32 = 80.0;

/// A point in window coordinates (logical pixels).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ScreenPoint {
    pub x: f32,
    pub y: f32,
}

impl ScreenPoint {
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }

    pub fn distance(self, other: ScreenPoint) -> f32 {
        ((self.x - other.x).powi(2) + (self.y - other.y).powi(2)).sqrt()
    }
}

/// A rectangle in window coordinates (logical pixels).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ScreenRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl ScreenRect {
    pub const fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self { x, y, width, height }
    }

    pub fn center(&self) -> ScreenPoint {
        ScreenPoint::new(self.x + self.width / 2.0, self.y + self.height / 2.0)
    }

    pub fn right(&self) -> f32 {
        self.x + self.width
    }

    pub fn bottom(&self) -> f32 {
        self.y + self.height
    }

    pub fn contains(&self, p: ScreenPoint) -> bool {
        p.x >= self.x && p.x <= self.right() && p.y >= self.y && p.y <= self.bottom()
    }

    pub fn intersects(&self, other: &ScreenRect) -> bool {
        self.x < other.right() && other.x < self.right() && self.y < other.bottom() && other.y < self.bottom()
    }

    /// Shrink by `d` on every side (grow when negative); never below zero size.
    pub fn inset(&self, d: f32) -> ScreenRect {
        ScreenRect::new(self.x + d, self.y + d, (self.width - 2.0 * d).max(0.0), (self.height - 2.0 * d).max(0.0))
    }
}

/// Clamp a zoom into the allowed range; non-finite values reset to 1.
pub fn clamp_zoom(zoom: f32) -> f32 {
    if zoom.is_finite() && zoom > 0.0 { zoom.clamp(MIN_ZOOM, MAX_ZOOM) } else { 1.0 }
}

pub fn to_screen(vp: Viewport, canvas: ScreenRect, p: Point) -> ScreenPoint {
    let c = canvas.center();
    ScreenPoint::new(c.x + (p.x - vp.center.x) * vp.zoom, c.y + (p.y - vp.center.y) * vp.zoom)
}

pub fn to_scene(vp: Viewport, canvas: ScreenRect, s: ScreenPoint) -> Point {
    let c = canvas.center();
    Point::new(vp.center.x + (s.x - c.x) / vp.zoom, vp.center.y + (s.y - c.y) / vp.zoom)
}

pub fn rect_to_screen(vp: Viewport, canvas: ScreenRect, r: Rect) -> ScreenRect {
    let origin = to_screen(vp, canvas, r.origin);
    ScreenRect::new(origin.x, origin.y, r.size.width * vp.zoom, r.size.height * vp.zoom)
}

/// The viewport that shows all of `bounds` inside `canvas` with a margin.
pub fn fit(bounds: Rect, canvas: ScreenRect) -> Viewport {
    let avail_w = (canvas.width - 2.0 * FIT_MARGIN).max(1.0);
    let avail_h = (canvas.height - 2.0 * FIT_MARGIN).max(1.0);
    let zoom = if bounds.size.width <= 0.0 && bounds.size.height <= 0.0 {
        1.0
    } else {
        (avail_w / bounds.size.width.max(1.0)).min(avail_h / bounds.size.height.max(1.0)).min(FIT_MAX_ZOOM)
    };
    Viewport { center: bounds.center(), zoom: clamp_zoom(zoom) }
}

/// Zoom by `factor`, keeping the scene point under `anchor` where it is.
pub fn zoom_about(vp: Viewport, canvas: ScreenRect, anchor: ScreenPoint, factor: f32) -> Viewport {
    let before = to_scene(vp, canvas, anchor);
    let zoom = clamp_zoom(vp.zoom * factor);
    let c = canvas.center();
    Viewport { center: Point::new(before.x - (anchor.x - c.x) / zoom, before.y - (anchor.y - c.y) / zoom), zoom }
}

/// Move the content by a screen-space delta (the content follows the
/// pointer).
pub fn pan_by(vp: Viewport, dx: f32, dy: f32) -> Viewport {
    Viewport { center: Point::new(vp.center.x - dx / vp.zoom, vp.center.y - dy / vp.zoom), zoom: vp.zoom }
}

/// Centre `rect`, keeping the zoom unless the rect would not fit, in which
/// case zoom out just enough.
pub fn center_on(vp: Viewport, canvas: ScreenRect, rect: Rect) -> Viewport {
    let avail_w = (canvas.width - 2.0 * FIT_MARGIN).max(1.0);
    let avail_h = (canvas.height - 2.0 * FIT_MARGIN).max(1.0);
    let needed = (avail_w / rect.size.width.max(1.0)).min(avail_h / rect.size.height.max(1.0));
    Viewport { center: rect.center(), zoom: clamp_zoom(vp.zoom.min(needed)) }
}

/// Zoom factor for a scroll of `dy` pixels with the zoom modifier held.
/// Scrolling up (positive `dy`) zooms in.
pub fn wheel_zoom_factor(dy: f32) -> f32 {
    (dy * 0.0025).clamp(-1.0, 1.0).exp()
}

/// Zoom factor for a pinch gesture delta (0.1 = 10% larger).
pub fn pinch_zoom_factor(delta: f32) -> f32 {
    (1.0 + delta).clamp(0.5, 2.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CANVAS: ScreenRect = ScreenRect::new(100.0, 50.0, 800.0, 600.0);

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    fn vp(x: f32, y: f32, zoom: f32) -> Viewport {
        Viewport { center: Point::new(x, y), zoom }
    }

    #[test]
    fn centre_maps_to_canvas_centre() {
        let v = vp(10.0, 20.0, 2.0);
        assert_eq!(to_screen(v, CANVAS, Point::new(10.0, 20.0)), ScreenPoint::new(500.0, 350.0));
        assert_eq!(to_screen(v, CANVAS, Point::new(11.0, 20.0)), ScreenPoint::new(502.0, 350.0));
    }

    #[test]
    fn screen_and_scene_round_trip() {
        let v = vp(-40.0, 13.5, 0.75);
        for p in [Point::new(0.0, 0.0), Point::new(123.0, -45.0), Point::new(-7.5, 900.0)] {
            let back = to_scene(v, CANVAS, to_screen(v, CANVAS, p));
            assert!(close(back.x, p.x) && close(back.y, p.y), "{p:?} -> {back:?}");
        }
    }

    #[test]
    fn rect_scales_with_zoom() {
        let r = rect_to_screen(vp(0.0, 0.0, 2.0), CANVAS, Rect::new(0.0, 0.0, 10.0, 5.0));
        assert_eq!(r, ScreenRect::new(500.0, 350.0, 20.0, 10.0));
    }

    #[test]
    fn fit_contains_bounds_with_margin() {
        let bounds = Rect::new(0.0, 0.0, 1600.0, 400.0);
        let v = fit(bounds, CANVAS);
        assert_eq!(v.center, Point::new(800.0, 200.0));
        let r = rect_to_screen(v, CANVAS, bounds);
        assert!(r.x >= CANVAS.x + FIT_MARGIN - 0.01 && r.right() <= CANVAS.right() - FIT_MARGIN + 0.01);
        assert!(r.y >= CANVAS.y && r.bottom() <= CANVAS.bottom());
        assert!(close(v.zoom, (800.0 - 64.0) / 1600.0));
    }

    #[test]
    fn fit_does_not_magnify_small_scenes_and_handles_empty() {
        assert_eq!(fit(Rect::new(0.0, 0.0, 10.0, 10.0), CANVAS).zoom, FIT_MAX_ZOOM);
        let empty = fit(Rect::default(), CANVAS);
        assert_eq!(empty.zoom, 1.0);
    }

    #[test]
    fn zoom_keeps_anchor_fixed() {
        let v = vp(30.0, -10.0, 1.0);
        let anchor = ScreenPoint::new(250.0, 120.0);
        let before = to_scene(v, CANVAS, anchor);
        let z = zoom_about(v, CANVAS, anchor, 2.5);
        assert!(close(z.zoom, 2.5));
        let after = to_scene(z, CANVAS, anchor);
        assert!(close(before.x, after.x) && close(before.y, after.y));
    }

    #[test]
    fn zoom_is_clamped() {
        let v = vp(0.0, 0.0, 1.0);
        assert_eq!(zoom_about(v, CANVAS, CANVAS.center(), 1000.0).zoom, MAX_ZOOM);
        assert_eq!(zoom_about(v, CANVAS, CANVAS.center(), 0.0001).zoom, MIN_ZOOM);
        assert_eq!(clamp_zoom(f32::NAN), 1.0);
        assert_eq!(clamp_zoom(-3.0), 1.0);
    }

    #[test]
    fn pan_moves_content_with_the_pointer() {
        let v = vp(0.0, 0.0, 2.0);
        let p = Point::new(5.0, 5.0);
        let before = to_screen(v, CANVAS, p);
        let after = to_screen(pan_by(v, 30.0, -10.0), CANVAS, p);
        assert!(close(after.x - before.x, 30.0) && close(after.y - before.y, -10.0));
    }

    #[test]
    fn center_on_keeps_zoom_when_it_fits_and_zooms_out_otherwise() {
        let v = vp(0.0, 0.0, 1.0);
        let small = center_on(v, CANVAS, Rect::new(100.0, 100.0, 50.0, 20.0));
        assert_eq!(small, vp(125.0, 110.0, 1.0));
        let big = center_on(v, CANVAS, Rect::new(0.0, 0.0, 4000.0, 100.0));
        assert!(big.zoom < 1.0);
        assert_eq!(big.center, Point::new(2000.0, 50.0));
    }

    #[test]
    fn wheel_and_pinch_factors_have_the_right_sign() {
        assert!(wheel_zoom_factor(100.0) > 1.0);
        assert!(wheel_zoom_factor(-100.0) < 1.0);
        assert_eq!(wheel_zoom_factor(0.0), 1.0);
        assert!(close(pinch_zoom_factor(0.1), 1.1));
        assert_eq!(pinch_zoom_factor(-5.0), 0.5);
    }

    #[test]
    fn screen_rect_helpers() {
        let r = ScreenRect::new(0.0, 0.0, 10.0, 10.0);
        assert!(r.contains(ScreenPoint::new(10.0, 0.0)));
        assert!(!r.contains(ScreenPoint::new(10.1, 0.0)));
        assert!(r.intersects(&ScreenRect::new(9.0, 9.0, 5.0, 5.0)));
        assert!(!r.intersects(&ScreenRect::new(10.0, 0.0, 5.0, 5.0)));
        assert_eq!(r.inset(2.0), ScreenRect::new(2.0, 2.0, 6.0, 6.0));
        assert_eq!(r.inset(8.0).width, 0.0);
    }
}
