//! M3 "done when": an edit to one transition moves no unrelated node.
//!
//! Runs the whole pipeline (parse → causal graph → analysis → scene builder
//! → layered layout) twice with one `SceneBuilder`, the way the app does on a
//! file reload, and compares node rectangles by element key.

use std::collections::BTreeMap;

use cascade_core::{CausalGraph, analyze, load_str};
use cascade_layout::Rect;
use cascade_scene::{
    HitTarget, LayoutSidecar, MonoMeasure, Scene, SceneBuilder, SceneInput, Theme, ViewKind, ViewState,
};

const BEFORE: &str = include_str!("../../../examples/shop/cascade.yaml");

fn build(builder: &mut SceneBuilder, yaml: &str, view: ViewKind) -> Scene {
    let model = load_str(yaml).expect("definition loads");
    let graph = CausalGraph::build(&model);
    let findings = analyze(&model, &graph);
    let state = ViewState { view, ..ViewState::default() };
    let theme = Theme::light();
    let sidecar = LayoutSidecar::default();
    let input = SceneInput {
        model: &model,
        graph: &graph,
        findings: &findings,
        view: &state,
        theme: &theme,
        measure: &MonoMeasure::default(),
        sidecar: &sidecar,
        traces: &[],
        mode: cascade_scene::SceneMode::View,
        play: None,
        diff: None,
    };
    builder.build(&input).expect("scene builds")
}

fn rects(scene: &Scene) -> BTreeMap<String, Rect> {
    scene
        .nodes
        .iter()
        .filter_map(|n| match &n.target {
            HitTarget::Element(key) => Some((key.to_string(), n.rect)),
            _ => None,
        })
        .collect()
}

/// Add one transition to the Shipment machine: `packing → lost` on `abort`.
/// Only nodes of the Shipment machine may move.
fn edited() -> String {
    let anchor = "  Shipment:\n";
    assert!(BEFORE.contains(anchor), "the shop example still has a Shipment machine");
    let marker = "    transitions:\n";
    let start = BEFORE.find(anchor).expect("anchor");
    let offset = BEFORE[start..].find(marker).expect("Shipment has transitions") + start + marker.len();
    let mut text = BEFORE.to_owned();
    text.insert_str(offset, "      - { from: packing, to: lost, on: abort }\n");
    text
}

#[test]
fn adding_a_transition_moves_no_unrelated_node() {
    let after_text = edited();
    for view in [ViewKind::Causal, ViewKind::Structure] {
        let mut builder = SceneBuilder::new();
        let before = rects(&build(&mut builder, BEFORE, view));
        let after = rects(&build(&mut builder, &after_text, view));

        let mut moved = Vec::new();
        for (key, rect) in &before {
            // Everything in the Shipment machine may shift to make room; any
            // other node must keep its exact rectangle.
            if key.contains(":Shipment:") || key.contains(":Shipment.") {
                continue;
            }
            match after.get(key) {
                Some(new) if new == rect => {}
                Some(new) => moved.push(format!("{key}: {rect:?} -> {new:?}")),
                None => moved.push(format!("{key}: disappeared")),
            }
        }
        assert!(moved.is_empty(), "{view} view moved unrelated nodes:\n{}", moved.join("\n"));
    }
}
