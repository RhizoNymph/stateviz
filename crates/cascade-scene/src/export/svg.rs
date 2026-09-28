//! Scene → standalone SVG.
//!
//! Draws exactly what the scene holds, in paint order: background, lanes,
//! under-overlays, edges, nodes, over-overlays. Each item is a `<g>` with a
//! class (`lane`, `overlay`, `edge`, `node`, `badge`) carrying its opacity.
//! Arrowheads are markers, one per stroke color; initial-state bars are
//! clipped to their shape. Labels use a monospace font; a label's origin is
//! the top-left of a line box 1.3 em tall, so its baseline sits 0.95 em
//! below it.

use std::collections::BTreeSet;
use std::fmt::Write;

use cascade_core::Severity;
use cascade_layout::{Insets, Point, Rect};

use crate::color::{Rgba, Theme};
use crate::export::ExportError;
use crate::scene::{
    Arrow, Badge, Border, Dash, FontWeight, HitTarget, Label, Layer, Overlay, Scene, SceneEdge, SceneNode, Shape,
    Stroke,
};

/// Space around the scene's bounds.
const MARGIN: f32 = 16.0;
/// Baseline below a label's origin, in em.
const BASELINE: f32 = 0.95;
/// Gap between the outlines of a double border.
const DOUBLE_GAP: f32 = 3.0;
/// Corner radius of stubs and lanes.
const STUB_RADIUS: f32 = 6.0;
const FONT_FAMILY: &str = "DejaVu Sans Mono, Menlo, Consolas, monospace";

pub(crate) fn to_svg(scene: &Scene) -> Result<String, ExportError> {
    check_finite(scene)?;
    let mut w = SvgWriter { scene, defs: String::new(), body: String::new(), markers: BTreeSet::new(), clips: 0 };
    w.write_body()?;
    w.finish()
}

struct SvgWriter<'a> {
    scene: &'a Scene,
    defs: String,
    body: String,
    /// Arrowhead colors, as `rrggbb`.
    markers: BTreeSet<String>,
    clips: usize,
}

impl SvgWriter<'_> {
    fn write_body(&mut self) -> Result<(), ExportError> {
        let scene = self.scene;
        let view = viewport(scene);
        writeln!(
            self.body,
            r#"<rect class="background" x="{}" y="{}" width="{}" height="{}" fill="{}"/>"#,
            num(view.left()),
            num(view.top()),
            num(view.size.width),
            num(view.size.height),
            scene.background.to_hex()
        )?;
        for lane in &scene.lanes {
            writeln!(self.body, r#"<g class="lane"{}>"#, opacity(lane.opacity))?;
            writeln!(
                self.body,
                "{}",
                rect_element(
                    lane.rect,
                    STUB_RADIUS,
                    &format!("{} {}", paint("fill", Some(lane.fill)), stroke(&lane.stroke))
                )
            )?;
            writeln!(self.body, "{}", text(&lane.title, ""))?;
            writeln!(self.body, "</g>")?;
        }
        self.overlays(Layer::Under)?;
        for edge in &scene.edges {
            self.edge(edge)?;
        }
        for node in &scene.nodes {
            self.node(node)?;
        }
        self.overlays(Layer::Over)
    }

    fn overlays(&mut self, layer: Layer) -> Result<(), ExportError> {
        for overlay in &self.scene.overlays {
            match overlay {
                Overlay::Line { from, to, stroke: s, opacity: o, layer: l } if *l == layer => {
                    writeln!(
                        self.body,
                        r#"<g class="overlay"{}><line x1="{}" y1="{}" x2="{}" y2="{}" {}/></g>"#,
                        opacity(*o),
                        num(from.x),
                        num(from.y),
                        num(to.x),
                        num(to.y),
                        stroke(s)
                    )?;
                }
                // Connect handles are an editing affordance, not part of
                // the picture: exports leave them out.
                Overlay::Rect { target: HitTarget::ConnectHandle { .. }, .. } => {}
                Overlay::Rect { rect, fill, stroke: s, radius, opacity: o, layer: l, .. } if *l == layer => {
                    let attrs = format!(
                        "{} {}",
                        paint("fill", *fill),
                        s.as_ref().map_or_else(|| r#"stroke="none""#.to_owned(), stroke)
                    );
                    writeln!(
                        self.body,
                        r#"<g class="overlay"{}>{}</g>"#,
                        opacity(*o),
                        rect_element(*rect, *radius, &attrs)
                    )?;
                }
                Overlay::Text { label, opacity: o, layer: l } if *l == layer => {
                    writeln!(self.body, r#"<g class="overlay"{}>{}</g>"#, opacity(*o), text(label, ""))?;
                }
                Overlay::Line { .. } | Overlay::Rect { .. } | Overlay::Text { .. } => {}
            }
        }
        Ok(())
    }

    fn edge(&mut self, edge: &SceneEdge) -> Result<(), ExportError> {
        let points: Vec<String> = edge.points.iter().map(|p| format!("{},{}", num(p.x), num(p.y))).collect();
        let marker = match edge.arrow {
            Arrow::End => {
                let id = edge.stroke.color.to_hex().trim_start_matches('#').to_owned();
                let attr = format!(r#" marker-end="url(#arrow-{id})""#);
                self.markers.insert(id);
                attr
            }
            Arrow::None => String::new(),
        };
        writeln!(self.body, r#"<g class="edge"{}>"#, opacity(edge.opacity))?;
        writeln!(
            self.body,
            r#"<polyline points="{}" fill="none" {} stroke-linejoin="round"{marker}/>"#,
            points.join(" "),
            stroke(&edge.stroke)
        )?;
        if let Some(label) = &edge.label {
            let halo = format!(
                r#" stroke="{}" stroke-width="3" stroke-linejoin="round" paint-order="stroke""#,
                self.scene.background.to_hex()
            );
            writeln!(self.body, "{}", text(label, &halo))?;
        }
        writeln!(self.body, "</g>")?;
        Ok(())
    }

    fn node(&mut self, node: &SceneNode) -> Result<(), ExportError> {
        let r = node.rect;
        writeln!(self.body, r#"<g class="node"{}>"#, opacity(node.opacity))?;
        let attrs = format!("{} {}", paint("fill", node.fill), stroke(&node.stroke));
        writeln!(self.body, "{}", shape_element(node.shape, r, &attrs))?;
        match node.border {
            Border::Single => {}
            Border::Double => {
                let inner = inset(r, DOUBLE_GAP);
                let inner_shape = match node.shape {
                    Shape::RoundedRect { radius } => Shape::RoundedRect { radius: (radius - DOUBLE_GAP).max(0.0) },
                    other => other,
                };
                let inner_attrs =
                    format!(r#"fill="none" {}"#, stroke(&Stroke { width: node.stroke.width.min(1.5), ..node.stroke }));
                writeln!(self.body, "{}", shape_element(inner_shape, inner, &inner_attrs))?;
            }
            Border::ThickLeft(width) => {
                let id = format!("clip-{}", self.clips);
                self.clips += 1;
                writeln!(self.defs, r#"<clipPath id="{id}">{}</clipPath>"#, shape_element(node.shape, r, ""))?;
                writeln!(
                    self.body,
                    r#"<rect class="initial-bar" x="{}" y="{}" width="{}" height="{}" {} clip-path="url(#{id})"/>"#,
                    num(r.left()),
                    num(r.top()),
                    num(width),
                    num(r.size.height),
                    paint("fill", Some(node.stroke.color))
                )?;
            }
        }
        for label in &node.labels {
            writeln!(self.body, "{}", text(label, ""))?;
        }
        if let Some(badge) = &node.badge {
            writeln!(self.body, "{}", self.badge(badge))?;
        }
        writeln!(self.body, "</g>")?;
        Ok(())
    }

    /// A finding badge: a red-outlined circle with the count, filled for
    /// errors.
    fn badge(&self, badge: &Badge) -> String {
        let background = self.scene.background;
        let red = finding_color(background);
        let (fill, ink) = match badge.severity {
            Severity::Error => (red, background),
            Severity::Warning | Severity::Info => (background, red),
        };
        let dash = if badge.severity == Severity::Info { Dash::Dashed { on: 2.0, off: 2.0 } } else { Dash::Solid };
        let size = badge.radius * 1.2;
        let label = Label {
            text: badge.count.to_string(),
            origin: Point::new(badge.center.x, badge.center.y - size * 1.3 / 2.0),
            font_size: size,
            color: ink,
            weight: FontWeight::Bold,
        };
        format!(
            r#"<g class="badge"><circle cx="{}" cy="{}" r="{}" {} {}/>{}</g>"#,
            num(badge.center.x),
            num(badge.center.y),
            num(badge.radius),
            paint("fill", Some(fill)),
            stroke(&Stroke { color: red, width: 1.5, dash }),
            text(&label, r#" text-anchor="middle""#)
        )
    }

    fn finish(self) -> Result<String, ExportError> {
        let view = viewport(self.scene);
        let mut out = String::with_capacity(self.body.len() + self.defs.len() + 1024);
        writeln!(out, r#"<?xml version="1.0" encoding="UTF-8"?>"#)?;
        writeln!(
            out,
            r#"<svg xmlns="http://www.w3.org/2000/svg" version="1.1" width="{}" height="{}" viewBox="{} {} {} {}" font-family="{FONT_FAMILY}">"#,
            num(view.size.width.ceil()),
            num(view.size.height.ceil()),
            num(view.left()),
            num(view.top()),
            num(view.size.width.ceil()),
            num(view.size.height.ceil())
        )?;
        writeln!(out, "<defs>")?;
        for id in &self.markers {
            writeln!(
                out,
                r##"<marker id="arrow-{id}" viewBox="0 0 10 8" refX="9" refY="4" markerWidth="10" markerHeight="8" markerUnits="userSpaceOnUse" orient="auto"><path d="M0,0 L10,4 L0,8 z" fill="#{id}"/></marker>"##
            )?;
        }
        out.push_str(&self.defs);
        writeln!(out, "</defs>")?;
        out.push_str(&self.body);
        writeln!(out, "</svg>")?;
        Ok(out)
    }
}

/// The area the SVG shows: the scene's bounds plus a margin, never empty.
fn viewport(scene: &Scene) -> Rect {
    let r = scene.bounds.outset(Insets::uniform(MARGIN));
    Rect::new(r.left(), r.top(), r.size.width.max(1.0), r.size.height.max(1.0))
}

/// The finding red of the theme whose background this is.
fn finding_color(background: Rgba) -> Rgba {
    let luminance =
        0.2126 * f32::from(background.r) + 0.7152 * f32::from(background.g) + 0.0722 * f32::from(background.b);
    if luminance < 128.0 { Theme::dark().finding } else { Theme::light().finding }
}

fn inset(r: Rect, by: f32) -> Rect {
    let by = by.min(r.size.width / 2.0).min(r.size.height / 2.0);
    Rect::new(r.left() + by, r.top() + by, r.size.width - 2.0 * by, r.size.height - 2.0 * by)
}

fn shape_element(shape: Shape, r: Rect, attrs: &str) -> String {
    let half = (r.size.height / 2.0).min(r.size.width / 2.0);
    match shape {
        Shape::Rect => rect_element(r, 0.0, attrs),
        Shape::RoundedRect { radius } => rect_element(r, radius.min(half), attrs),
        Shape::Pill => rect_element(r, half, attrs),
        Shape::Stub => rect_element(r, STUB_RADIUS.min(half), attrs),
        Shape::Tag => {
            let point = (r.size.height / 2.0).min(r.size.width / 3.0);
            let d = format!(
                "M{},{} H{} L{},{} L{},{} H{} Z",
                num(r.left()),
                num(r.top()),
                num(r.right() - point),
                num(r.right()),
                num(r.center().y),
                num(r.right() - point),
                num(r.bottom()),
                num(r.left())
            );
            format!(r#"<path d="{d}" {attrs}/>"#)
        }
        Shape::Hexagon => {
            let point = (r.size.height * 0.4).min(r.size.width / 4.0);
            let d = format!(
                "M{},{} H{} L{},{} L{},{} H{} L{},{} Z",
                num(r.left() + point),
                num(r.top()),
                num(r.right() - point),
                num(r.right()),
                num(r.center().y),
                num(r.right() - point),
                num(r.bottom()),
                num(r.left() + point),
                num(r.left()),
                num(r.center().y)
            );
            format!(r#"<path d="{d}" {attrs}/>"#)
        }
    }
}

fn rect_element(r: Rect, radius: f32, attrs: &str) -> String {
    let rounded = if radius > 0.0 { format!(r#" rx="{0}" ry="{0}""#, num(radius)) } else { String::new() };
    format!(
        r#"<rect x="{}" y="{}" width="{}" height="{}"{rounded} {attrs}/>"#,
        num(r.left()),
        num(r.top()),
        num(r.size.width.max(0.0)),
        num(r.size.height.max(0.0))
    )
}

fn text(label: &Label, extra: &str) -> String {
    let weight = match label.weight {
        FontWeight::Bold => r#" font-weight="bold""#,
        FontWeight::Normal => "",
    };
    format!(
        r#"<text x="{}" y="{}" font-size="{}" {}{weight}{extra} xml:space="preserve">{}</text>"#,
        num(label.origin.x),
        num(label.origin.y + label.font_size * BASELINE),
        num(label.font_size),
        paint("fill", Some(label.color)),
        escape(&label.text)
    )
}

/// `stroke`, width, opacity and dash attributes.
fn stroke(s: &Stroke) -> String {
    let mut out = format!(r#"{} stroke-width="{}""#, paint("stroke", Some(s.color)), num(s.width));
    match s.dash {
        Dash::Solid => {}
        Dash::Dashed { on, off } => out.push_str(&format!(r#" stroke-dasharray="{} {}""#, num(on), num(off))),
        Dash::Dotted => {
            let dot = s.width.max(1.0);
            out.push_str(&format!(r#" stroke-dasharray="{} {}""#, num(dot), num(dot * 2.0)));
        }
    }
    out
}

/// `fill="#rrggbb"` (plus `fill-opacity` for translucent colors) or
/// `fill="none"`.
fn paint(attr: &str, color: Option<Rgba>) -> String {
    match color {
        None => format!(r#"{attr}="none""#),
        Some(c) if c.a < 255 => format!(r#"{attr}="{}" {attr}-opacity="{}""#, c.to_hex(), num(c.alpha_f32())),
        Some(c) => format!(r#"{attr}="{}""#, c.to_hex()),
    }
}

fn opacity(value: f32) -> String {
    if value < 1.0 { format!(r#" opacity="{}""#, num(value.max(0.0))) } else { String::new() }
}

/// A compact number: integers without a fraction, otherwise at most two
/// decimals.
fn num(v: f32) -> String {
    if !v.is_finite() {
        return "0".to_owned();
    }
    let rounded = (v * 100.0).round() / 100.0;
    if rounded == 0.0 {
        return "0".to_owned();
    }
    let s = format!("{rounded:.2}");
    s.trim_end_matches('0').trim_end_matches('.').to_owned()
}

fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c if (c as u32) < 0x20 && !matches!(c, '\t' | '\n' | '\r') => {}
            c => out.push(c),
        }
    }
    out
}

/// Every coordinate is finite, so the SVG never contains `NaN`.
fn check_finite(scene: &Scene) -> Result<(), ExportError> {
    let rect_ok = |r: &Rect| [r.left(), r.top(), r.size.width, r.size.height].iter().all(|v| v.is_finite());
    let point_ok = |p: &Point| p.x.is_finite() && p.y.is_finite();
    let label_ok = |l: &Label| point_ok(&l.origin) && l.font_size.is_finite();
    let ok = rect_ok(&scene.bounds)
        && scene.lanes.iter().all(|l| rect_ok(&l.rect) && label_ok(&l.title) && l.stroke.width.is_finite())
        && scene.nodes.iter().all(|n| {
            rect_ok(&n.rect)
                && n.stroke.width.is_finite()
                && n.labels.iter().all(label_ok)
                && n.badge.as_ref().is_none_or(|b| point_ok(&b.center) && b.radius.is_finite())
        })
        && scene.edges.iter().all(|e| {
            e.points.iter().all(point_ok) && e.stroke.width.is_finite() && e.label.as_ref().is_none_or(label_ok)
        })
        && scene.overlays.iter().all(|o| match o {
            Overlay::Line { from, to, .. } => point_ok(from) && point_ok(to),
            Overlay::Rect { rect, radius, .. } => rect_ok(rect) && radius.is_finite(),
            Overlay::Text { label, .. } => label_ok(label),
        });
    if ok { Ok(()) } else { Err(ExportError::NonFinite) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_are_compact() {
        assert_eq!(num(20.0), "20");
        assert_eq!(num(0.15), "0.15");
        assert_eq!(num(1.001), "1");
        assert_eq!(num(-3.5), "-3.5");
        assert_eq!(num(-0.001), "0");
        assert_eq!(num(f32::NAN), "0");
    }

    #[test]
    fn text_is_escaped() {
        assert_eq!(escape(r#"a<b & "c" 'd'>"#), "a&lt;b &amp; &quot;c&quot; &apos;d&apos;&gt;");
        assert_eq!(escape("bell\u{7}"), "bell");
    }

    #[test]
    fn translucent_paint_adds_opacity() {
        assert_eq!(paint("fill", None), r#"fill="none""#);
        assert_eq!(paint("fill", Some(Rgba::hex(0x112233))), "fill=\"#112233\"");
        assert!(paint("stroke", Some(Rgba::hex(0x112233).with_alpha(0.5))).contains(r#"stroke-opacity="0.5""#));
    }

    #[test]
    fn finding_red_follows_the_background() {
        assert_eq!(finding_color(Theme::light().background), Theme::light().finding);
        assert_eq!(finding_color(Theme::dark().background), Theme::dark().finding);
    }
}
