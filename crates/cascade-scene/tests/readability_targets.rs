//! The readability targets of docs/features/readability.md as assertions,
//! for the shop and order-fulfillment examples. The report itself is
//! `tests/readability.rs`.
//!
//! Targets that placement alone cannot reach are `#[ignore]`d with the
//! reason; they depend on the layout engine's routing work
//! (`feat/readable-routing`) and are switched on at integration.

use cascade_core::{CausalGraph, analyze, load_str};
use cascade_scene::metrics::{SceneMetrics, measure};
use cascade_scene::{LayoutSidecar, MonoMeasure, SceneBuilder, SceneInput, SceneMode, Theme, ViewKind, ViewState};

const SHOP: &str = include_str!("../../../examples/shop/cascade.yaml");
const ORDER_FULFILLMENT: &str = include_str!("../../../examples/order-fulfillment/cascade.yaml");

fn metrics(yaml: &str, view: ViewKind, mode: SceneMode) -> SceneMetrics {
    let model = load_str(yaml).unwrap_or_else(|err| panic!("{err}"));
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
        mode,
        play: None,
        diff: None,
    };
    measure(&SceneBuilder::new().build(&input).unwrap_or_else(|err| panic!("{err}")))
}

fn shop_build() -> SceneMetrics {
    metrics(SHOP, ViewKind::Structure, SceneMode::Edit)
}

fn order_fulfillment_build() -> SceneMetrics {
    metrics(ORDER_FULFILLMENT, ViewKind::Structure, SceneMode::Edit)
}

/// Baselines from before the readability work (readability.md), which view
/// mode and the causal view must not exceed.
struct Baseline {
    crossings: usize,
    length: f32,
    bends: usize,
    label_overlaps: usize,
    corridor: usize,
}

fn no_worse(m: &SceneMetrics, b: &Baseline) {
    assert!(m.edge_crossings <= b.crossings, "crossings {} > {}", m.edge_crossings, b.crossings);
    assert!(m.total_edge_length <= b.length + 0.5, "length {} > {}", m.total_edge_length, b.length);
    assert!(m.bends <= b.bends, "bends {} > {}", m.bends, b.bends);
    assert!(m.label_overlaps <= b.label_overlaps, "label overlaps {} > {}", m.label_overlaps, b.label_overlaps);
    assert!(m.corridor_edges <= b.corridor, "corridor edges {} > {}", m.corridor_edges, b.corridor);
    assert_eq!(m.edges_through_nodes, 0, "edges through nodes");
}

#[test]
fn shop_build_canvas_crossings_length_and_labels() {
    let m = shop_build();
    assert!(m.edge_crossings <= 150, "crossings {m}");
    assert!(m.total_edge_length <= 70_000.0, "length {m}");
    assert_eq!(m.label_overlaps, 0, "{m}");
    assert_eq!(m.edges_through_nodes, 0, "{m}");
}

/// Wires between groups that are not neighbours in the stack detour
/// through a side corridor. With gutters the placement gets this from 38
/// to 11; a wider search over gutter assignments (coordinate descent from
/// 200 starts) finds no better than 10 under the current routing, so
/// reaching 5 needs routing that can run wires between the nodes of a lane.
#[test]
#[ignore = "needs routing across lanes (feat/readable-routing); placement alone reaches 11"]
fn shop_build_canvas_corridors() {
    let m = shop_build();
    assert!(m.corridor_edges <= 5, "{m}");
}

#[test]
fn shop_build_canvas_uses_few_corridors_already() {
    let m = shop_build();
    assert!(m.corridor_edges <= 11, "{m}");
}

#[test]
fn order_fulfillment_build_canvas_meets_every_target() {
    let m = order_fulfillment_build();
    assert!(m.edge_crossings <= 5, "crossings {m}");
    assert!(m.total_edge_length <= 5_000.0, "length {m}");
    assert_eq!(m.label_overlaps, 0, "{m}");
    assert_eq!(m.corridor_edges, 0, "{m}");
    assert_eq!(m.edges_through_nodes, 0, "{m}");
}

#[test]
fn shop_view_mode_does_not_regress() {
    let m = metrics(SHOP, ViewKind::Structure, SceneMode::View);
    no_worse(&m, &Baseline { crossings: 38, length: 31_165.0, bends: 151, label_overlaps: 5, corridor: 9 });
    assert!(m.label_overlaps <= 3, "the self-link's label sits clear of the lane's nodes: {m}");
}

/// The remaining overlaps are labels of different links that the engine
/// puts in the same spot of a shared gap.
#[test]
#[ignore = "needs collision-free label placement in the engine (feat/readable-routing); placement reaches 3"]
fn shop_view_mode_labels_do_not_overlap() {
    let m = metrics(SHOP, ViewKind::Structure, SceneMode::View);
    assert_eq!(m.label_overlaps, 0, "{m}");
}

#[test]
fn order_fulfillment_view_mode_does_not_regress() {
    let m = metrics(ORDER_FULFILLMENT, ViewKind::Structure, SceneMode::View);
    no_worse(&m, &Baseline { crossings: 1, length: 1541.0, bends: 10, label_overlaps: 0, corridor: 0 });
}

#[test]
fn causal_views_do_not_regress() {
    let shop = metrics(SHOP, ViewKind::Causal, SceneMode::View);
    no_worse(&shop, &Baseline { crossings: 17, length: 16_518.0, bends: 72, label_overlaps: 0, corridor: 0 });
    let of = metrics(ORDER_FULFILLMENT, ViewKind::Causal, SceneMode::View);
    no_worse(&of, &Baseline { crossings: 0, length: 896.0, bends: 0, label_overlaps: 0, corridor: 0 });
}
