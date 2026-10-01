//! Emphasis, finding badges and diff decorations, applied to a built scene.
//!
//! Runs after layout on every build, so none of it can move a node:
//!
//! - **Emphasis** ([`Interaction::emphasis`]): selected items get the
//!   theme's selected outline width, focused ones a middle width, dimmed ones
//!   the theme's dim opacity; search matches get a subtle dotted halo. No
//!   hue changes.
//! - **Findings:** every element in a finding's `subjects()` gets a badge
//!   (count of findings, highest severity) and a red outline.
//! - **Diff:** added elements get a green outline, removed ones become red
//!   dashed ghosts at reduced opacity; `diff` is set on every item.

use std::collections::{BTreeSet, HashMap};

use cascade_core::diff::{DiffStatus, ModelDiff};
use cascade_core::{ElementRef, Finding, Model, Severity};
use cascade_layout::{Insets, Point, Rect};

use crate::color::Theme;
use crate::emphasis::Interaction;
use crate::play::SceneMode;
use crate::scene::{Badge, Dash, Emphasis, Label, Layer, Overlay, Scene, Shape, Stroke};
use crate::text::TextMeasure;
use crate::views::draft::{EdgeInfo, Meta};

/// Opacity of a removed element's ghost in diff mode.
pub(crate) const GHOST_OPACITY: f32 = 0.45;
/// Radius of a finding badge.
pub(crate) const BADGE_RADIUS: f32 = 8.0;
/// Gap between a search match and its halo.
const HALO_GAP: f32 = 4.0;

/// Findings per element, by finding index.
pub(crate) struct FindingIndex<'a> {
    findings: &'a [Finding],
    by_element: HashMap<ElementRef, Vec<usize>>,
}

impl<'a> FindingIndex<'a> {
    pub fn new(findings: &'a [Finding]) -> Self {
        Self::for_mode(findings, SceneMode::View)
    }

    /// Findings as badged in `mode`. Build mode badges only errors: the
    /// warnings and notes of a half-built system (unreachable states,
    /// unhandled events, state-dependent fires) would cover the canvas.
    pub fn for_mode(findings: &'a [Finding], mode: SceneMode) -> Self {
        let mut by_element: HashMap<ElementRef, Vec<usize>> = HashMap::new();
        for (i, f) in findings.iter().enumerate() {
            if mode == SceneMode::Edit && f.severity != Severity::Error {
                continue;
            }
            for subject in f.detail.subjects() {
                let list = by_element.entry(subject).or_default();
                if list.last() != Some(&i) {
                    list.push(i);
                }
            }
        }
        Self { findings, by_element }
    }

    /// Distinct findings about any of `elements`: their count and highest
    /// severity.
    pub fn summary<'e>(&self, elements: impl IntoIterator<Item = &'e ElementRef>) -> Option<(u32, Severity)> {
        let ids: BTreeSet<usize> =
            elements.into_iter().filter_map(|e| self.by_element.get(e)).flatten().copied().collect();
        let severity = ids.iter().filter_map(|&i| self.findings.get(i)).map(|f| f.severity).max()?;
        Some((u32::try_from(ids.len()).unwrap_or(u32::MAX), severity))
    }
}

/// Everything the decoration pass reads.
pub(crate) struct Decor<'a> {
    pub model: &'a Model,
    pub theme: &'a Theme,
    pub interaction: &'a Interaction,
    pub findings: FindingIndex<'a>,
    pub diff: Option<&'a ModelDiff>,
}

impl Decor<'_> {
    /// Outline width for an emphasis, never thinner than `base`.
    pub fn width(&self, emphasis: Emphasis, base: f32) -> f32 {
        match emphasis {
            Emphasis::Selected => base.max(self.theme.selected_stroke_width),
            Emphasis::Focused => base.max(focus_width(self.theme)),
            Emphasis::Normal | Emphasis::Dimmed | Emphasis::SearchMatch => base,
        }
    }

    pub fn opacity(&self, emphasis: Emphasis) -> f32 {
        if emphasis == Emphasis::Dimmed { self.theme.dim_opacity } else { 1.0 }
    }

    fn status(&self, element: Option<&ElementRef>) -> Option<DiffStatus> {
        let diff = self.diff?;
        Some(element.map_or(DiffStatus::Unchanged, |e| diff.status(&self.model.key_of(*e))))
    }

    /// Decorate nodes and edges in place. `node_metas` and `edge_infos` are
    /// aligned with `scene.nodes` and `scene.edges`; overlays owned by a
    /// node follow its opacity.
    pub fn apply(
        &self,
        scene: &mut Scene,
        node_metas: &[Meta],
        edge_infos: &[EdgeInfo],
        overlay_owner: &[Option<usize>],
    ) {
        let mut halos = Vec::new();
        for (node, meta) in scene.nodes.iter_mut().zip(node_metas) {
            let emphasis = self.interaction.emphasis(&meta.elements, &meta.anchor);
            node.emphasis = emphasis;
            node.stroke.width = self.width(emphasis, node.stroke.width);
            node.opacity = self.opacity(emphasis);

            if let Some((count, severity)) = self.findings.summary(meta.elements.iter().chain(&meta.badge_elements)) {
                node.stroke.color = self.theme.finding;
                node.badge = Some(Badge {
                    count,
                    severity,
                    center: Point::new(node.rect.right() - 2.0, node.rect.top() + 2.0),
                    radius: BADGE_RADIUS,
                });
            }

            node.diff = self.status(meta.elements.first());
            match node.diff {
                Some(DiffStatus::Added) => {
                    node.stroke.color = self.theme.added;
                    node.stroke.width = node.stroke.width.max(focus_width(self.theme));
                }
                Some(DiffStatus::Removed) => {
                    node.stroke = Stroke::dashed(self.theme.removed, node.stroke.width.max(focus_width(self.theme)));
                    node.fill = node.fill.map(|f| f.mix(self.theme.background, 0.6));
                    node.opacity *= GHOST_OPACITY;
                }
                Some(DiffStatus::Changed | DiffStatus::Unchanged) | None => {}
            }

            if emphasis == Emphasis::SearchMatch {
                halos.push(Overlay::Rect {
                    rect: node.rect.outset(Insets::uniform(HALO_GAP)),
                    fill: None,
                    stroke: Some(Stroke { color: self.theme.text_muted, width: 1.5, dash: Dash::Dotted }),
                    radius: halo_radius(node.shape, node.rect),
                    opacity: node.opacity,
                    layer: Layer::Under,
                    target: crate::scene::HitTarget::None,
                });
            }
        }

        for (overlay, owner) in scene.overlays.iter_mut().zip(overlay_owner) {
            if let Some(node) = owner.and_then(|i| scene.nodes.get(i)) {
                set_overlay_opacity(overlay, node.opacity);
            }
        }
        scene.overlays.extend(halos);

        let node_diff: Vec<Option<DiffStatus>> = scene.nodes.iter().map(|n| n.diff).collect();
        for (edge, info) in scene.edges.iter_mut().zip(edge_infos) {
            let meta = &info.meta;
            let emphasis = self.interaction.emphasis(&meta.elements, &meta.anchor);
            edge.emphasis = emphasis;
            edge.stroke.width = self.width(emphasis, edge.stroke.width);
            edge.opacity = self.opacity(emphasis);
            if self.diff.is_none() {
                continue;
            }
            let own = self.status(meta.elements.first());
            let ends =
                info.ends.map(|(a, b)| [node_diff.get(a).copied().flatten(), node_diff.get(b).copied().flatten()]);
            edge.diff = inherit_diff(own, ends);
            match edge.diff {
                Some(DiffStatus::Added) => edge.stroke.color = self.theme.added,
                Some(DiffStatus::Removed) => {
                    edge.stroke = Stroke::dashed(self.theme.removed, edge.stroke.width);
                    edge.opacity *= GHOST_OPACITY;
                }
                Some(DiffStatus::Changed | DiffStatus::Unchanged) | None => {}
            }
        }
    }
}

/// An edge's diff status: its own when added or removed, otherwise removed
/// (or added) when an endpoint is, otherwise its own.
fn inherit_diff(own: Option<DiffStatus>, ends: Option<[Option<DiffStatus>; 2]>) -> Option<DiffStatus> {
    match own {
        Some(DiffStatus::Added | DiffStatus::Removed) | None => own,
        Some(DiffStatus::Changed | DiffStatus::Unchanged) => {
            let ends = ends.unwrap_or([None, None]);
            if ends.contains(&Some(DiffStatus::Removed)) {
                Some(DiffStatus::Removed)
            } else if ends.contains(&Some(DiffStatus::Added)) {
                Some(DiffStatus::Added)
            } else {
                own
            }
        }
    }
}

/// Between the normal and the selected outline width.
pub(crate) fn focus_width(theme: &Theme) -> f32 {
    (theme.stroke_width + theme.selected_stroke_width) / 2.0
}

pub(crate) fn halo_radius(shape: Shape, rect: Rect) -> f32 {
    match shape {
        Shape::Pill => rect.size.height / 2.0 + HALO_GAP,
        Shape::RoundedRect { radius } => radius + HALO_GAP,
        Shape::Stub => crate::views::style::STUB_RADIUS + HALO_GAP,
        Shape::Tag | Shape::Hexagon | Shape::Rect => HALO_GAP,
    }
}

fn set_overlay_opacity(overlay: &mut Overlay, value: f32) {
    match overlay {
        Overlay::Line { opacity, .. } | Overlay::Rect { opacity, .. } | Overlay::Text { opacity, .. } => {
            *opacity = value;
        }
    }
}

/// The bounding box of everything drawn: nodes (with badges), edges, lanes,
/// overlays and every label.
pub(crate) fn scene_bounds(scene: &Scene, measure: &dyn TextMeasure) -> Rect {
    let mut bounds: Option<Rect> = None;
    let mut grow = |r: Rect| bounds = Some(bounds.map_or(r, |b| b.union(&r)));
    let label_rect = |l: &Label| {
        Rect::new(l.origin.x, l.origin.y, measure.width(&l.text, l.font_size), measure.line_height(l.font_size))
    };
    for lane in &scene.lanes {
        grow(lane.rect);
        grow(label_rect(&lane.title));
    }
    for node in &scene.nodes {
        grow(node.rect);
        node.labels.iter().for_each(|l| grow(label_rect(l)));
        if let Some(b) = &node.badge {
            grow(Rect::new(b.center.x - b.radius, b.center.y - b.radius, 2.0 * b.radius, 2.0 * b.radius));
        }
    }
    for edge in &scene.edges {
        for p in &edge.points {
            grow(Rect::from_origin_size(*p, Default::default()));
        }
        if let Some(l) = &edge.label {
            grow(label_rect(l));
        }
    }
    for overlay in &scene.overlays {
        match overlay {
            Overlay::Line { from, to, .. } => {
                grow(Rect::from_origin_size(*from, Default::default()));
                grow(Rect::from_origin_size(*to, Default::default()));
            }
            Overlay::Rect { rect, .. } => grow(*rect),
            Overlay::Text { label, .. } => grow(label_rect(label)),
        }
    }
    bounds.unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edges_inherit_removal_and_addition_from_endpoints() {
        use DiffStatus::*;
        assert_eq!(inherit_diff(Some(Added), Some([Some(Removed), None])), Some(Added));
        assert_eq!(inherit_diff(Some(Unchanged), Some([Some(Removed), Some(Added)])), Some(Removed));
        assert_eq!(inherit_diff(Some(Unchanged), Some([Some(Unchanged), Some(Added)])), Some(Added));
        assert_eq!(inherit_diff(Some(Changed), Some([Some(Unchanged), Some(Unchanged)])), Some(Changed));
        assert_eq!(inherit_diff(Some(Unchanged), None), Some(Unchanged));
        assert_eq!(inherit_diff(None, Some([Some(Removed), None])), None);
    }
}
