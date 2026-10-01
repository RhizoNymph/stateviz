//! Fixtures for the build (edit mode) and play (overlay) tests.

#![allow(dead_code)]

use std::collections::BTreeMap;

use cascade_core::{CausalGraph, ElementKey, Model, load_str};
use cascade_layout::{Point, Rect};
use cascade_scene::{
    HitTarget, LayoutSidecar, MonoMeasure, Overlay, PlayMarker, PlayOverlay, Scene, SceneBuilder, SceneInput,
    SceneMode, Theme, ViewKind, ViewState,
};

pub const SHOP: &str = include_str!("../../../../examples/shop/cascade.yaml");

pub struct Bench {
    pub model: Model,
    pub graph: CausalGraph,
    pub theme: Theme,
    pub sidecar: LayoutSidecar,
}

impl Bench {
    pub fn new(yaml: &str) -> Self {
        let model = load_str(yaml).unwrap_or_else(|err| panic!("{err}"));
        let graph = CausalGraph::build(&model);
        Self { model, graph, theme: Theme::light(), sidecar: LayoutSidecar::default() }
    }

    pub fn build_with(
        &self,
        builder: &mut SceneBuilder,
        state: &ViewState,
        mode: SceneMode,
        play: Option<&PlayOverlay>,
    ) -> Scene {
        let input = SceneInput {
            model: &self.model,
            graph: &self.graph,
            findings: &[],
            view: state,
            theme: &self.theme,
            measure: &MonoMeasure::default(),
            sidecar: &self.sidecar,
            traces: &[],
            diff: None,
            mode,
            play,
        };
        builder.build(&input).unwrap_or_else(|err| panic!("build failed: {err}"))
    }

    /// The structure view in edit mode, fresh builder.
    pub fn edit(&self) -> Scene {
        self.build_with(&mut SceneBuilder::new(), &structure(), SceneMode::Edit, None)
    }

    pub fn scene(&self, view: ViewKind, mode: SceneMode, play: Option<&PlayOverlay>) -> Scene {
        let state = ViewState { view, ..ViewState::default() };
        self.build_with(&mut SceneBuilder::new(), &state, mode, play)
    }
}

pub fn structure() -> ViewState {
    ViewState { view: ViewKind::Structure, ..ViewState::default() }
}

pub fn causal() -> ViewState {
    ViewState { view: ViewKind::Causal, ..ViewState::default() }
}

pub fn k(s: &str) -> ElementKey {
    s.parse().unwrap_or_else(|_| panic!("bad key {s}"))
}

pub fn marker(instance: &str, state: &str) -> PlayMarker {
    let key = k(state);
    let machine = match &key {
        ElementKey::State { machine, .. } => machine.clone(),
        other => panic!("not a state key: {other}"),
    };
    PlayMarker { instance: instance.to_owned(), machine, state: key }
}

pub fn target(key: &str) -> HitTarget {
    HitTarget::Element(k(key))
}

pub fn node_rect(scene: &Scene, key: &str) -> Rect {
    let t = target(key);
    scene.nodes.iter().find(|n| n.target == t).map(|n| n.rect).unwrap_or_else(|| panic!("no node {key}"))
}

pub fn rects(scene: &Scene) -> BTreeMap<String, Rect> {
    scene
        .nodes
        .iter()
        .filter_map(|n| match &n.target {
            HitTarget::Element(key) => Some((key.to_string(), n.rect)),
            _ => None,
        })
        .collect()
}

/// Connect handles: element key string → handle rect.
pub fn handles(scene: &Scene) -> BTreeMap<String, Rect> {
    let mut out = BTreeMap::new();
    for o in &scene.overlays {
        if let Overlay::Rect { rect, target: HitTarget::ConnectHandle { element }, .. } = o {
            assert!(out.insert(element.to_string(), *rect).is_none(), "two handles for {element}");
        }
    }
    out
}

/// Overlay texts with their positions.
pub fn texts(scene: &Scene) -> Vec<(String, Point)> {
    scene
        .overlays
        .iter()
        .filter_map(|o| match o {
            Overlay::Text { label, .. } => Some((label.text.clone(), label.origin)),
            _ => None,
        })
        .collect()
}

pub fn touches(r: Rect, p: Point) -> bool {
    r.outset(cascade_layout::Insets::uniform(0.5)).contains(p)
}

pub fn inside(outer: Rect, inner: Rect) -> bool {
    inner.left() >= outer.left() - 0.01
        && inner.top() >= outer.top() - 0.01
        && inner.right() <= outer.right() + 0.01
        && inner.bottom() <= outer.bottom() + 0.01
}

pub fn overlaps(a: Rect, b: Rect) -> bool {
    a.left() < b.right() && b.left() < a.right() && a.top() < b.bottom() && b.top() < a.bottom()
}
