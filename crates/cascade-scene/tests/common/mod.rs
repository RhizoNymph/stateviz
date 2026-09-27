//! Shared fixtures for the scene integration tests.

#![allow(dead_code)]

use cascade_core::diff::ModelDiff;
use cascade_core::{CausalGraph, ElementKey, Finding, Model, load_str};
use cascade_scene::{
    EdgeKind, HitTarget, LayoutSidecar, MonoMeasure, Scene, SceneBuilder, SceneEdge, SceneInput, SceneNode, Theme,
    ViewState,
};
use cascade_sim::Trace;

pub const SPEC_EXAMPLE: &str = include_str!("../../../../examples/order-fulfillment/cascade.yaml");

/// The causal-graph test chain:
///
/// ```text
/// User ─▶ A:a0→a1 ─(Go)─▶ C1 ─▶ B:b0→b1 ─(Done)─▶ C2 ─▶ A:a1→a0 ─(Back)─▶ C3 ─▶ B:b1→b0
///                                                     └──────────────▶ D:d0→d1
/// ```
pub const CHAIN: &str = r#"
machines:
  A:
    states: [a0, a1]
    transitions:
      - { from: a0, to: a1, on: go, emits: [Go] }
      - { from: a1, to: a0, on: reset, emits: [Back] }
  B:
    states: [b0, b1]
    transitions:
      - { from: b0, to: b1, on: start, emits: [Done] }
      - { from: b1, to: b0, on: rewind }
  D:
    states: [d0, d1]
    transitions:
      - { from: d0, to: d1, on: note }
controllers:
  C1:
    on:
      Go: [{ fire: B.start }]
  C2:
    on:
      Done: [{ fire: A.reset }, { fire: D.note }]
  C3:
    on:
      Back: [{ fire: B.rewind }]
external:
  User: [A.go]
"#;

/// A retry loop: `R: s → s` emits `Failed`, which `Retrier` answers by
/// firing `retry` again.
pub const RETRY: &str = r#"
machines:
  R:
    states: [s, t]
    transitions:
      - { from: s, to: s, on: retry, emits: [Failed] }
      - { from: s, to: t, on: go, emits: [Went] }
controllers:
  Retrier:
    on:
      Failed: [{ fire: R.retry }]
external:
  User: [R.go, R.retry]
"#;

pub struct Fixture {
    pub model: Model,
    pub graph: CausalGraph,
    pub findings: Vec<Finding>,
    pub sidecar: LayoutSidecar,
    pub theme: Theme,
    pub diff: Option<ModelDiff>,
    pub traces: Vec<Trace>,
}

impl Fixture {
    pub fn new(yaml: &str) -> Self {
        let model = match load_str(yaml) {
            Ok(m) => m,
            Err(err) => panic!("{err}"),
        };
        let graph = CausalGraph::build(&model);
        Self {
            model,
            graph,
            findings: Vec::new(),
            sidecar: LayoutSidecar::default(),
            theme: Theme::light(),
            diff: None,
            traces: Vec::new(),
        }
    }

    pub fn build(&self, builder: &mut SceneBuilder, view: &ViewState) -> Scene {
        let input = SceneInput {
            model: &self.model,
            graph: &self.graph,
            findings: &self.findings,
            view,
            theme: &self.theme,
            measure: &MonoMeasure::default(),
            sidecar: &self.sidecar,
            traces: &self.traces,
            diff: self.diff.as_ref(),
        };
        match builder.build(&input) {
            Ok(scene) => scene,
            Err(err) => panic!("build failed: {err}"),
        }
    }

    /// Build once with a fresh builder.
    pub fn scene(&self, view: &ViewState) -> Scene {
        self.build(&mut SceneBuilder::new(), view)
    }

    pub fn element(&self, key: &str) -> cascade_core::ElementRef {
        self.model.resolve_key(&k(key)).unwrap_or_else(|| panic!("no element {key}"))
    }
}

pub fn k(s: &str) -> ElementKey {
    s.parse().unwrap_or_else(|_| panic!("bad key {s}"))
}

pub fn target(key: &str) -> HitTarget {
    HitTarget::Element(k(key))
}

pub fn find_node<'a>(scene: &'a Scene, key: &str) -> Option<&'a SceneNode> {
    let t = target(key);
    scene.nodes.iter().find(|n| n.target == t)
}

pub fn node<'a>(scene: &'a Scene, key: &str) -> &'a SceneNode {
    find_node(scene, key).unwrap_or_else(|| panic!("no node {key} in {:#?}", node_targets(scene)))
}

pub fn node_targets(scene: &Scene) -> Vec<String> {
    scene.nodes.iter().map(|n| describe(&n.target)).collect()
}

pub fn describe(t: &HitTarget) -> String {
    match t {
        HitTarget::Element(k) => k.to_string(),
        other => format!("{other:?}"),
    }
}

pub fn edges_of(scene: &Scene, kind: EdgeKind) -> Vec<&SceneEdge> {
    scene.edges.iter().filter(|e| e.kind == kind).collect()
}

pub fn edge_to<'a>(scene: &'a Scene, key: &str) -> Vec<&'a SceneEdge> {
    let t = target(key);
    scene.edges.iter().filter(|e| e.target == t).collect()
}

pub fn view(selection: &[&str]) -> ViewState {
    ViewState { selection: selection.iter().map(|s| k(s)).collect(), ..ViewState::default() }
}

pub fn assert_close(a: f32, b: f32) {
    assert!((a - b).abs() < 0.5, "{a} != {b}");
}

/// Whether `p` lies in `r` or on its outline, allowing for rounding in
/// where a layout engine attaches an edge.
pub fn touches(r: cascade_layout::Rect, p: cascade_layout::Point) -> bool {
    r.outset(cascade_layout::Insets::uniform(0.5)).contains(p)
}
