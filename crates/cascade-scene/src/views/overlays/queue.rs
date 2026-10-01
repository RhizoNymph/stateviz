//! The last step and the queue.
//!
//! - **Active** (what the last action went through): the theme's selected
//!   outline weight, plus a translucent glow ring in the item's own outline
//!   color behind nodes. Edges get the weight only. An edge is active when
//!   it stands for an active element, or joins two active nodes (an emit
//!   from a taken transition to the event it emitted).
//! - **Pending** (queued events and fires, in queue order): a dotted
//!   outline and a chip with the entry's queue position, `1` being the head
//!   (filled; later positions hollow). An item queued more than once lists
//!   every position ("1, 3"). Node chips sit on the top-left corner, edge
//!   chips just before the arrowhead.
//!
//! Matching: an item stands for an element when its meta lists it. When
//! nothing drawn does (the structure view's view mode has no event nodes),
//! items related through a rule stand in: links whose rules handle that
//! event, run in that handler, belong to that controller or fire that
//! trigger, and nodes badged with the element.

use cascade_core::{ElementRef, Model};
use cascade_layout::Point;

use crate::scene::{Dash, FontWeight, HitTarget, Layer, Overlay, Scene, Stroke};
use crate::views::decorate::halo_radius;
use crate::views::draft::{EdgeInfo, Meta};
use crate::views::overlays::PlayDecor;
use crate::views::overlays::chip::{self, ChipStyle};

/// Gap between a node and its glow ring.
const GLOW_GAP: f32 = 4.0;
/// Stroke width of the glow ring.
const GLOW_WIDTH: f32 = 6.0;
/// Opacity of the glow ring's color.
const GLOW_ALPHA: f32 = 0.35;
/// How far before the arrowhead an edge's queue chip sits.
const EDGE_CHIP_BACK: f32 = 18.0;

/// Scene items standing for one element, by index.
#[derive(Default)]
struct Items {
    nodes: Vec<usize>,
    edges: Vec<usize>,
}

impl Items {
    fn is_empty(&self) -> bool {
        self.nodes.is_empty() && self.edges.is_empty()
    }
}

/// Whether `other` is a rule linked to `element` (see the module docs).
fn related(model: &Model, element: ElementRef, other: ElementRef) -> bool {
    let ElementRef::Rule(r) = other else { return false };
    let rule = model.rule(r);
    element == ElementRef::Event(rule.event)
        || element == ElementRef::Handler(rule.handler)
        || element == ElementRef::Controller(rule.controller)
        || element == ElementRef::Trigger(rule.trigger)
}

fn items_for(model: &Model, element: ElementRef, nodes: &[Meta], edges: &[EdgeInfo]) -> Items {
    let direct = Items {
        nodes: (0..nodes.len()).filter(|&i| nodes[i].elements.contains(&element)).collect(),
        edges: (0..edges.len()).filter(|&i| edges[i].meta.elements.contains(&element)).collect(),
    };
    if !direct.is_empty() {
        return direct;
    }
    let linked = |meta: &Meta| meta.elements.iter().any(|&e| related(model, element, e));
    Items {
        nodes: (0..nodes.len()).filter(|&i| nodes[i].badge_elements.contains(&element) || linked(&nodes[i])).collect(),
        edges: (0..edges.len()).filter(|&i| linked(&edges[i].meta)).collect(),
    }
}

pub(super) fn active(
    decor: &PlayDecor<'_>,
    scene: &mut Scene,
    nodes: &[Meta],
    edges: &[EdgeInfo],
    keys: &[cascade_core::ElementKey],
) {
    if keys.is_empty() {
        return;
    }
    let model = decor.model;
    let theme = decor.painter.theme;
    let mut node_on = vec![false; scene.nodes.len()];
    let mut edge_on = vec![false; scene.edges.len()];
    for element in keys.iter().filter_map(|k| model.resolve_key(k)) {
        let items = items_for(model, element, nodes, edges);
        for i in items.nodes {
            if let Some(on) = node_on.get_mut(i) {
                *on = true;
            }
        }
        for i in items.edges {
            if let Some(on) = edge_on.get_mut(i) {
                *on = true;
            }
        }
    }
    for (i, info) in edges.iter().enumerate().take(edge_on.len()) {
        if let Some((a, b)) = info.ends
            && node_on.get(a).copied().unwrap_or(false)
            && node_on.get(b).copied().unwrap_or(false)
        {
            edge_on[i] = true;
        }
    }
    let mut glows = Vec::new();
    for (node, _) in scene.nodes.iter_mut().zip(&node_on).filter(|(_, on)| **on) {
        node.stroke.width = node.stroke.width.max(theme.selected_stroke_width);
        glows.push(Overlay::Rect {
            rect: node.rect.outset(cascade_layout::Insets::uniform(GLOW_GAP)),
            fill: None,
            stroke: Some(Stroke::solid(node.stroke.color.with_alpha(GLOW_ALPHA), GLOW_WIDTH)),
            radius: halo_radius(node.shape, node.rect),
            opacity: node.opacity,
            layer: Layer::Under,
            target: HitTarget::None,
        });
    }
    scene.overlays.extend(glows);
    for (edge, _) in scene.edges.iter_mut().zip(&edge_on).filter(|(_, on)| **on) {
        edge.stroke.width = edge.stroke.width.max(theme.selected_stroke_width);
    }
}

pub(super) fn pending(
    decor: &PlayDecor<'_>,
    scene: &mut Scene,
    nodes: &[Meta],
    edges: &[EdgeInfo],
    keys: &[cascade_core::ElementKey],
) {
    if keys.is_empty() {
        return;
    }
    let model = decor.model;
    // Queue positions (1-based) per item.
    let mut node_at: Vec<Vec<usize>> = vec![Vec::new(); scene.nodes.len()];
    let mut edge_at: Vec<Vec<usize>> = vec![Vec::new(); scene.edges.len()];
    for (pos, key) in keys.iter().enumerate() {
        let Some(element) = model.resolve_key(key) else { continue };
        let items = items_for(model, element, nodes, edges);
        for i in items.nodes {
            if let Some(list) = node_at.get_mut(i) {
                list.push(pos + 1);
            }
        }
        for i in items.edges {
            if let Some(list) = edge_at.get_mut(i) {
                list.push(pos + 1);
            }
        }
    }

    let painter = decor.painter;
    let mut chips: Vec<(Point, String, bool, HitTarget, f32)> = Vec::new();
    for (node, at) in scene.nodes.iter_mut().zip(&node_at).filter(|(_, at)| !at.is_empty()) {
        node.stroke.dash = Dash::Dotted;
        let text = positions(at);
        let size = chip::size(painter, &text);
        let corner = Point::new(node.rect.left() - size.width / 2.0 + 2.0, node.rect.top() - size.height / 2.0 + 2.0);
        chips.push((corner, text, at.contains(&1), node.target.clone(), node.opacity));
    }
    for (edge, at) in scene.edges.iter_mut().zip(&edge_at).filter(|(_, at)| !at.is_empty()) {
        edge.stroke.dash = Dash::Dotted;
        let text = positions(at);
        let size = chip::size(painter, &text);
        let c = before_end(&edge.points, EDGE_CHIP_BACK);
        let corner = Point::new(c.x - size.width / 2.0, c.y - size.height / 2.0);
        chips.push((corner, text, at.contains(&1), edge.target.clone(), edge.opacity));
    }
    let theme = painter.theme;
    let head = ChipStyle {
        fill: theme.text,
        stroke: Stroke::solid(theme.text, 1.0),
        text: theme.background,
        weight: FontWeight::Bold,
    };
    let rest = ChipStyle {
        fill: theme.background,
        stroke: Stroke::solid(theme.text_muted, 1.0),
        text: theme.text,
        weight: FontWeight::Normal,
    };
    for (corner, text, is_head, target, opacity) in chips {
        let style = if is_head { &head } else { &rest };
        chip::push(scene, painter, corner, &text, style, target, opacity);
    }
}

/// "1", "1, 3".
fn positions(at: &[usize]) -> String {
    at.iter().map(usize::to_string).collect::<Vec<_>>().join(", ")
}

/// The point `back` units before the end of a polyline (its middle when it
/// is shorter than twice that).
fn before_end(points: &[Point], back: f32) -> Point {
    let total: f32 = points.windows(2).map(|w| w[0].distance(w[1])).sum();
    let mut remaining = back.min(total / 2.0);
    for w in points.windows(2).rev() {
        let len = w[0].distance(w[1]);
        if len > 0.0 && remaining <= len {
            let t = remaining / len;
            return Point::new(w[1].x + (w[0].x - w[1].x) * t, w[1].y + (w[0].y - w[1].y) * t);
        }
        remaining -= len;
    }
    points.last().copied().unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn positions_list_every_queue_slot() {
        assert_eq!(positions(&[1]), "1");
        assert_eq!(positions(&[1, 3]), "1, 3");
    }

    #[test]
    fn points_before_the_end_of_a_polyline() {
        let pts = [Point::new(0.0, 0.0), Point::new(100.0, 0.0), Point::new(100.0, 10.0)];
        assert_eq!(before_end(&pts, 18.0), Point::new(92.0, 0.0));
        assert_eq!(before_end(&pts, 5.0), Point::new(100.0, 5.0));
        let short = [Point::new(0.0, 0.0), Point::new(10.0, 0.0)];
        assert_eq!(before_end(&short, 18.0), Point::new(5.0, 0.0));
        assert_eq!(before_end(&[Point::new(1.0, 2.0)], 18.0), Point::new(1.0, 2.0));
    }
}
