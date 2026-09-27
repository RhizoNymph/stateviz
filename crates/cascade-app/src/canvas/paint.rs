//! Paints a [`Scene`] with GPUI primitives.
//!
//! Order: background, lanes, `Layer::Under` overlays, edges, nodes,
//! `Layer::Over` overlays, then the node being dragged. Everything is
//! transformed from scene to window coordinates through the viewport and
//! culled against the canvas. Every color is multiplied by its item's
//! opacity. Labels use a monospace font with a forced advance, so text
//! occupies exactly the width `MonoMeasure` gave the scene builder.

use cascade_core::ElementKey;
use cascade_layout::{Point, Rect};
use cascade_scene::{
    Arrow, Badge, Border, Dash, FontWeight, HitTarget, Label, Lane, Layer, MonoMeasure, Overlay, Scene, SceneEdge,
    SceneNode, Shape, Stroke, TextMeasure, Theme, Viewport,
};
use gpui::{
    App, BorderStyle, Bounds, ContentMask, Corners, Hsla, PathBuilder, Pixels, SharedString, TextRun, Window, fill,
    font, point, px, quad, size, transparent_black,
};

use super::shapes::{
    DOUBLE_BORDER_GAP, MIN_LABEL_PX, STUB_RADIUS, arrow_size, arrowhead, bounds, dash_pattern, hexagon, last_segment,
    quantize_font, stroke_px, tag, visible,
};
use crate::theme::hsla;
use crate::viewport::{ScreenPoint, ScreenRect, rect_to_screen, to_screen};

/// Extra outline weight of the hovered item, in screen pixels. Hover shows
/// by weight only, never by color.
const HOVER_EXTRA: f32 = 1.5;
/// Culling slack for strokes, arrowheads and labels that overhang.
const CULL_SLACK: f32 = 24.0;

/// Everything a frame of the canvas needs.
pub struct PaintInput<'a> {
    pub scene: &'a Scene,
    pub viewport: Viewport,
    pub canvas: ScreenRect,
    pub theme: &'a Theme,
    pub mono: SharedString,
    pub hover: Option<&'a HitTarget>,
    /// The node being dragged and its current top-left corner.
    pub drag: Option<(&'a ElementKey, Point)>,
}

pub fn to_bounds(r: ScreenRect) -> Bounds<Pixels> {
    Bounds::new(point(px(r.x), px(r.y)), size(px(r.width.max(0.0)), px(r.height.max(0.0))))
}

fn gpoint(p: ScreenPoint) -> gpui::Point<Pixels> {
    point(px(p.x), px(p.y))
}

pub fn paint_scene(input: &PaintInput<'_>, window: &mut Window, cx: &mut App) {
    let canvas_bounds = to_bounds(input.canvas);
    window.paint_quad(fill(canvas_bounds, hsla(input.scene.background, 1.0)));
    let painter = Painter {
        vp: input.viewport,
        canvas: input.canvas,
        theme: input.theme,
        mono: input.mono.clone(),
        measure: MonoMeasure::default(),
    };
    window.with_content_mask(Some(ContentMask { bounds: canvas_bounds }), |window| {
        let scene = input.scene;
        let is_hovered = |t: &HitTarget| !matches!(t, HitTarget::None) && input.hover == Some(t);
        for lane in &scene.lanes {
            painter.lane(lane, window, cx);
        }
        for overlay in scene.overlays.iter().filter(|o| overlay_layer(o) == Layer::Under) {
            painter.overlay(overlay, window, cx);
        }
        for edge in &scene.edges {
            painter.edge(edge, is_hovered(&edge.target), window, cx);
        }
        let dragged =
            |n: &SceneNode| matches!((&n.target, input.drag), (HitTarget::Element(k), Some((d, _))) if k == d);
        for node in scene.nodes.iter().filter(|n| !dragged(n)) {
            painter.node(node, (0.0, 0.0), is_hovered(&node.target), window, cx);
        }
        for overlay in scene.overlays.iter().filter(|o| overlay_layer(o) == Layer::Over) {
            painter.overlay(overlay, window, cx);
        }
        if let Some((_, top_left)) = input.drag
            && let Some(node) = scene.nodes.iter().find(|n| dragged(n))
        {
            let offset = (top_left.x - node.rect.left(), top_left.y - node.rect.top());
            painter.node(node, offset, true, window, cx);
        }
    });
}

fn overlay_layer(overlay: &Overlay) -> Layer {
    match overlay {
        Overlay::Line { layer, .. } | Overlay::Rect { layer, .. } | Overlay::Text { layer, .. } => *layer,
    }
}

struct Painter<'a> {
    vp: Viewport,
    canvas: ScreenRect,
    theme: &'a Theme,
    mono: SharedString,
    measure: MonoMeasure,
}

impl Painter<'_> {
    fn pt(&self, p: Point) -> ScreenPoint {
        to_screen(self.vp, self.canvas, p)
    }

    fn rect(&self, r: Rect) -> ScreenRect {
        rect_to_screen(self.vp, self.canvas, r)
    }

    fn on_canvas(&self, r: ScreenRect) -> bool {
        visible(r, self.canvas, CULL_SLACK)
    }

    fn border_style(dash: Dash) -> BorderStyle {
        match dash {
            Dash::Solid => BorderStyle::Solid,
            Dash::Dashed { .. } | Dash::Dotted => BorderStyle::Dashed,
        }
    }

    // --- Lanes and overlays ------------------------------------------------

    fn lane(&self, lane: &Lane, window: &mut Window, cx: &mut App) {
        let r = self.rect(lane.rect);
        if !self.on_canvas(r) {
            return;
        }
        let sw = stroke_px(lane.stroke.width, self.vp.zoom);
        window.paint_quad(quad(
            to_bounds(r),
            px(0.0),
            hsla(lane.fill, lane.opacity),
            px(sw),
            hsla(lane.stroke.color, lane.opacity),
            Self::border_style(lane.stroke.dash),
        ));
        self.label(&lane.title, (0.0, 0.0), lane.opacity, window, cx);
    }

    fn overlay(&self, overlay: &Overlay, window: &mut Window, cx: &mut App) {
        match overlay {
            Overlay::Line { from, to, stroke, opacity, .. } => {
                let points = [self.pt(*from), self.pt(*to)];
                if bounds(&points).is_some_and(|b| self.on_canvas(b)) {
                    self.polyline(&points, stroke, *opacity, 0.0, window);
                }
            }
            Overlay::Rect { rect, fill: bg, stroke, radius, opacity, .. } => {
                let r = self.rect(*rect);
                if !self.on_canvas(r) {
                    return;
                }
                let (sw, color, style) = stroke.map_or((0.0, transparent_black(), BorderStyle::Solid), |s| {
                    (stroke_px(s.width, self.vp.zoom), hsla(s.color, *opacity), Self::border_style(s.dash))
                });
                let background = bg.map_or(transparent_black(), |c| hsla(c, *opacity));
                window.paint_quad(quad(to_bounds(r), px(radius * self.vp.zoom), background, px(sw), color, style));
            }
            Overlay::Text { label, opacity, .. } => self.label(label, (0.0, 0.0), *opacity, window, cx),
        }
    }

    // --- Edges -------------------------------------------------------------

    fn edge(&self, edge: &SceneEdge, hovered: bool, window: &mut Window, cx: &mut App) {
        if edge.points.len() < 2 {
            return;
        }
        let points: Vec<ScreenPoint> = edge.points.iter().map(|p| self.pt(*p)).collect();
        if !bounds(&points).is_some_and(|b| self.on_canvas(b)) {
            return;
        }
        let extra = if hovered { HOVER_EXTRA } else { 0.0 };
        self.polyline(&points, &edge.stroke, edge.opacity, extra, window);
        if edge.arrow == Arrow::End
            && let Some((from, tip)) = last_segment(&points)
            && let Some(head) = arrowhead(from, tip, arrow_size(edge.stroke.width, self.vp.zoom))
        {
            self.fill_polygon(&head, hsla(edge.stroke.color, edge.opacity), window);
        }
        if let Some(label) = &edge.label {
            self.label(label, (0.0, 0.0), edge.opacity, window, cx);
        }
    }

    fn polyline(&self, points: &[ScreenPoint], stroke: &Stroke, opacity: f32, extra: f32, window: &mut Window) {
        let width = stroke_px(stroke.width, self.vp.zoom) + extra;
        let mut builder = PathBuilder::stroke(px(width));
        if let Some([on, off]) = dash_pattern(stroke.dash, width, self.vp.zoom) {
            builder = builder.dash_array(&[px(on), px(off)]);
        }
        let mut iter = points.iter();
        let Some(first) = iter.next() else {
            return;
        };
        builder.move_to(gpoint(*first));
        for p in iter {
            builder.line_to(gpoint(*p));
        }
        match builder.build() {
            Ok(path) => window.paint_path(path, hsla(stroke.color, opacity)),
            Err(error) => tracing::debug!(%error, "cannot build edge path"),
        }
    }

    fn fill_polygon(&self, points: &[ScreenPoint], color: Hsla, window: &mut Window) {
        let mut builder = PathBuilder::fill();
        let points: Vec<_> = points.iter().map(|p| gpoint(*p)).collect();
        builder.add_polygon(&points, true);
        match builder.build() {
            Ok(path) => window.paint_path(path, color),
            Err(error) => tracing::debug!(%error, "cannot build fill path"),
        }
    }

    fn stroke_polygon(&self, points: &[ScreenPoint], width: f32, dash: Dash, color: Hsla, window: &mut Window) {
        let mut builder = PathBuilder::stroke(px(width));
        if let Some([on, off]) = dash_pattern(dash, width, self.vp.zoom) {
            builder = builder.dash_array(&[px(on), px(off)]);
        }
        let points: Vec<_> = points.iter().map(|p| gpoint(*p)).collect();
        builder.add_polygon(&points, true);
        match builder.build() {
            Ok(path) => window.paint_path(path, color),
            Err(error) => tracing::debug!(%error, "cannot build outline path"),
        }
    }

    // --- Nodes -------------------------------------------------------------

    fn node(&self, node: &SceneNode, offset: (f32, f32), hovered: bool, window: &mut Window, cx: &mut App) {
        let r = self.rect(node.rect.translate(offset.0, offset.1));
        if !self.on_canvas(r) {
            return;
        }
        let zoom = self.vp.zoom;
        let op = node.opacity;
        let background = node.fill.map_or(transparent_black(), |c| hsla(c, op));
        let stroke = hsla(node.stroke.color, op);
        let sw = stroke_px(node.stroke.width, zoom);
        let extra = if hovered { HOVER_EXTRA } else { 0.0 };
        let radius = match node.shape {
            Shape::Pill => Some(r.height.min(r.width) / 2.0),
            Shape::Rect => Some(0.0),
            Shape::RoundedRect { radius } => Some((radius * zoom).min(r.height.min(r.width) / 2.0)),
            Shape::Stub => Some((STUB_RADIUS * zoom).min(r.height.min(r.width) / 2.0)),
            Shape::Hexagon | Shape::Tag => None,
        };
        let gap = DOUBLE_BORDER_GAP * zoom + sw;
        match radius {
            Some(radius) => {
                let style = Self::border_style(node.stroke.dash);
                window.paint_quad(quad(to_bounds(r), px(radius), background, px(sw + extra), stroke, style));
                if node.border == Border::Double {
                    let inner = r.inset(gap);
                    window.paint_quad(quad(
                        to_bounds(inner),
                        px((radius - gap).max(0.0)),
                        transparent_black(),
                        px(sw),
                        stroke,
                        style,
                    ));
                }
                if let Border::ThickLeft(width) = node.border {
                    let bar = (width * zoom).min(r.width);
                    let corner = px(radius.min(bar));
                    window.paint_quad(fill(to_bounds(ScreenRect::new(r.x, r.y, bar, r.height)), stroke).corner_radii(
                        Corners { top_left: corner, bottom_left: corner, top_right: px(0.0), bottom_right: px(0.0) },
                    ));
                }
            }
            None => {
                let outline = |rect: ScreenRect| -> Vec<ScreenPoint> {
                    match node.shape {
                        Shape::Hexagon => hexagon(rect).to_vec(),
                        _ => tag(rect).to_vec(),
                    }
                };
                let points = outline(r);
                if node.fill.is_some() {
                    self.fill_polygon(&points, background, window);
                }
                self.stroke_polygon(&points, sw + extra, node.stroke.dash, stroke, window);
                if node.border == Border::Double {
                    self.stroke_polygon(&outline(r.inset(gap)), sw, node.stroke.dash, stroke, window);
                }
                if let Border::ThickLeft(width) = node.border {
                    let bar = (width * zoom).min(r.width);
                    window.paint_quad(fill(to_bounds(ScreenRect::new(r.x, r.y, bar, r.height)), stroke));
                }
            }
        }
        for label in &node.labels {
            self.label(label, offset, op, window, cx);
        }
        if let Some(badge) = &node.badge {
            self.badge(badge, offset, op, window, cx);
        }
    }

    fn badge(&self, badge: &Badge, offset: (f32, f32), opacity: f32, window: &mut Window, cx: &mut App) {
        let c = self.pt(badge.center.offset(offset.0, offset.1));
        let r = (badge.radius * self.vp.zoom).max(3.0);
        let color = hsla(self.theme.finding, opacity);
        let circle = ScreenRect::new(c.x - r, c.y - r, 2.0 * r, 2.0 * r);
        window.paint_quad(quad(
            to_bounds(circle),
            px(r),
            hsla(self.theme.background, opacity),
            px(stroke_px(self.theme.stroke_width, self.vp.zoom)),
            color,
            BorderStyle::Solid,
        ));
        let font_px = r * 1.1;
        if font_px < MIN_LABEL_PX {
            return;
        }
        let text = badge.count.to_string();
        let width = self.measure.width(&text, font_px);
        let line = self.measure.line_height(font_px);
        let origin = ScreenPoint::new(c.x - width / 2.0, c.y - line / 2.0);
        self.text(&text, origin, font_px, line, color, FontWeight::Bold, window, cx);
    }

    // --- Text --------------------------------------------------------------

    fn label(&self, label: &Label, offset: (f32, f32), opacity: f32, window: &mut Window, cx: &mut App) {
        let font_px = label.font_size * self.vp.zoom;
        if font_px < MIN_LABEL_PX || label.text.is_empty() {
            return;
        }
        let origin = self.pt(label.origin.offset(offset.0, offset.1));
        let line = self.measure.line_height(label.font_size) * self.vp.zoom;
        let width = self.measure.width(&label.text, font_px);
        if !self.on_canvas(ScreenRect::new(origin.x, origin.y, width, line)) {
            return;
        }
        self.text(&label.text, origin, font_px, line, hsla(label.color, opacity), label.weight, window, cx);
    }

    #[allow(clippy::too_many_arguments)]
    fn text(
        &self,
        text: &str,
        origin: ScreenPoint,
        font_px: f32,
        line_height: f32,
        color: Hsla,
        weight: FontWeight,
        window: &mut Window,
        cx: &mut App,
    ) {
        let mut run_font = font(self.mono.clone());
        if weight == FontWeight::Bold {
            run_font = run_font.bold();
        }
        let run = TextRun {
            len: text.len(),
            font: run_font,
            color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let font_px = quantize_font(font_px);
        let advance = px(font_px * self.measure.advance_em);
        let shaped =
            window.text_system().shape_line(SharedString::from(text.to_owned()), px(font_px), &[run], Some(advance));
        if let Err(error) = shaped.paint(gpoint(origin), px(line_height), gpui::TextAlign::Left, None, window, cx) {
            tracing::debug!(%error, "cannot paint label");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounds_conversion_never_negative() {
        let b = to_bounds(ScreenRect::new(1.0, 2.0, -3.0, 4.0));
        assert_eq!(b.size.width, px(0.0));
        assert_eq!(b.origin.x, px(1.0));
    }
}
