//! Causal lanes (`ViewState::group_by_machine`): one lane per machine on
//! shared causal columns, placement rules, no leftward forward arrows, and
//! everything the causal view does (emphasis, cones, hide mode, stubs,
//! badges, back edges, play overlay) still working.

mod common;

use cascade_core::{Direction, analyze};
use cascade_layout::Rect;
use cascade_scene::metrics::{leftward_edges, measure};
use cascade_scene::{
    ConeFocus, Emphasis, HitTarget, OutsideFocus, PlayOverlay, Scene, SceneBuilder, SceneInput, SceneMode, ViewState,
};
use common::*;

const SHOP: &str = include_str!("../../../examples/shop/cascade.yaml");

fn lanes() -> ViewState {
    ViewState { group_by_machine: true, ..ViewState::default() }
}

fn with_findings(yaml: &str) -> Fixture {
    let mut fx = Fixture::new(yaml);
    fx.findings = analyze(&fx.model, &fx.graph);
    fx
}

fn lane_titles(scene: &Scene) -> Vec<String> {
    scene.lanes.iter().map(|l| l.title.text.clone()).collect()
}

fn lane_rect(scene: &Scene, title: &str) -> Rect {
    scene.lanes.iter().find(|l| l.title.text == title).map(|l| l.rect).unwrap_or_else(|| panic!("no lane {title}"))
}

fn in_lane(scene: &Scene, key: &str, title: &str) -> bool {
    let lane = lane_rect(scene, title);
    let r = node(scene, key).rect;
    r.top() >= lane.top() && r.bottom() <= lane.bottom() && r.left() >= lane.left() && r.right() <= lane.right()
}

#[test]
fn every_machine_gets_a_lane_in_definition_order() {
    let fx = Fixture::new(SPEC_EXAMPLE);
    let scene = fx.scene(&lanes());
    assert_eq!(lane_titles(&scene), ["Order", "Shipment"]);
    let order = &scene.lanes[0];
    assert_eq!(order.title.color, node(&scene, "transition:Order:pending->paid@capture_ok").fill.expect("fill"));
    assert_eq!(order.target, target("machine:Order"));
    // Lanes stack top to bottom and share one width.
    assert!(scene.lanes[0].rect.bottom() < scene.lanes[1].rect.top());
    assert_eq!(scene.lanes[0].rect.left(), scene.lanes[1].rect.left());
    assert_eq!(scene.lanes[0].rect.right(), scene.lanes[1].rect.right());
    // The flat causal view has no lanes.
    assert!(fx.scene(&ViewState::default()).lanes.is_empty());
}

#[test]
fn nodes_follow_the_placement_rules() {
    let fx = Fixture::new(SPEC_EXAMPLE);
    let scene = fx.scene(&lanes());
    // Transitions in their machine's lane.
    assert!(in_lane(&scene, "transition:Order:pending->paid@capture_ok", "Order"));
    assert!(in_lane(&scene, "transition:Shipment:idle->picking@start", "Shipment"));
    // Events with their first emitter.
    assert!(in_lane(&scene, "event:OrderPaid", "Order"));
    assert!(in_lane(&scene, "event:Shipped", "Shipment"));
    // A handler in the lane of the machine it fires into.
    assert!(in_lane(&scene, "handler:Fulfillment/OrderPaid", "Shipment"));
    // Sources in the lane of the first machine they trigger.
    assert!(in_lane(&scene, "external:Customer", "Order"));
}

#[test]
fn nodes_with_no_machine_go_to_the_unattached_lane_last() {
    let fx = Fixture::new(SHOP);
    let scene = fx.scene(&lanes());
    let titles = lane_titles(&scene);
    assert_eq!(titles.last().map(String::as_str), Some("Unattached"), "{titles:?}");
    // ReturnRequested is handled but emitted by no transition.
    assert!(in_lane(&scene, "event:ReturnRequested", "Unattached"));
    let unattached = scene.lanes.last().expect("lane");
    assert_eq!(unattached.target, HitTarget::None);
}

#[test]
fn pills_name_their_states_and_trigger_the_lane_names_the_machine() {
    let fx = Fixture::new(SPEC_EXAMPLE);
    let scene = fx.scene(&lanes());
    let pill = node(&scene, "transition:Order:pending->paid@capture_ok");
    assert_eq!(pill.labels[0].text, "pending → paid");
    assert_eq!(pill.labels[1].text, "capture_ok");
    let flat = fx.scene(&ViewState::default());
    assert_eq!(node(&flat, "transition:Order:pending->paid@capture_ok").labels[0].text, "Order: pending → paid");
}

#[test]
fn causes_sit_left_of_what_they_cause_in_every_lane() {
    for yaml in [SPEC_EXAMPLE, SHOP, CHAIN] {
        let fx = Fixture::new(yaml);
        let scene = fx.scene(&lanes());
        assert_eq!(leftward_edges(&scene), 0);
        // Every forward edge ends right of where it starts.
        for e in scene.edges.iter().filter(|e| !e.back_edge) {
            let (a, b) = (e.points[0], e.points[e.points.len() - 1]);
            assert!(b.x >= a.x - 0.01, "{:?}", e.points);
        }
    }
}

#[test]
fn a_cause_and_its_effect_in_different_lanes_share_the_column_order() {
    // Fulfillment (Shipment lane) is caused by OrderPaid (Order lane) and
    // causes idle → picking (Shipment lane).
    let fx = Fixture::new(SPEC_EXAMPLE);
    let scene = fx.scene(&lanes());
    let paid = node(&scene, "event:OrderPaid").rect;
    let handler = node(&scene, "handler:Fulfillment/OrderPaid").rect;
    let start = node(&scene, "transition:Shipment:idle->picking@start").rect;
    assert!(paid.right() < handler.left() && handler.right() < start.left());
    // Columns are shared: the first column lines up across both lanes.
    let customer = node(&scene, "external:Customer").rect;
    let handoff = node(&scene, "transition:Shipment:picking->shipped@handoff").rect;
    assert!((customer.center().x - handoff.center().x).abs() < 0.01, "{customer:?} {handoff:?}");
}

#[test]
fn cycle_back_edges_stay_red() {
    let fx = Fixture::new(SHOP);
    let flat = fx.scene(&ViewState::default());
    let scene = fx.scene(&lanes());
    let back = scene.edges.iter().filter(|e| e.back_edge).count();
    assert!(back > 0, "the shop's cascade cycles show back edges");
    assert!(flat.edges.iter().any(|e| e.back_edge));
    for e in scene.edges.iter().filter(|e| e.back_edge) {
        assert_eq!(e.stroke.color, fx.theme.finding);
    }
}

#[test]
fn metrics_are_reasonable() {
    for (yaml, max_crossings, max_length) in [(SPEC_EXAMPLE, 0, 1_200.0), (SHOP, 40, 21_000.0)] {
        let fx = Fixture::new(yaml);
        let m = measure(&fx.scene(&lanes()));
        assert!(m.edge_crossings <= max_crossings, "{m}");
        assert!(m.total_edge_length <= max_length, "{m}");
        assert_eq!(m.label_overlaps, 0, "{m}");
        assert_eq!(m.corridor_edges, 0, "{m}");
        assert_eq!(m.edges_through_nodes, 0, "{m}");
    }
}

#[test]
fn selection_and_cones_never_relayout() {
    let fx = Fixture::new(SHOP);
    let mut builder = SceneBuilder::new();
    let plain = fx.build(&mut builder, &lanes());
    let key = "transition:Order:cart->placed@checkout";
    let selected = ViewState { selection: vec![k(key)], ..lanes() };
    let coned = ViewState { cone: Some(ConeFocus { direction: Direction::Forward, depth: None }), ..selected.clone() };
    let searched = ViewState { search: Some("Order".to_owned()), ..lanes() };
    for state in [&selected, &coned, &searched] {
        let scene = fx.build(&mut builder, state);
        let rects: Vec<Rect> = scene.nodes.iter().map(|n| n.rect).collect();
        assert_eq!(rects, plain.nodes.iter().map(|n| n.rect).collect::<Vec<_>>());
        assert_eq!(scene.lanes.len(), plain.lanes.len());
    }
    assert_eq!(builder.layouts_run(), 1);
    let scene = fx.build(&mut builder, &coned);
    assert_eq!(node(&scene, key).emphasis, Emphasis::Selected);
    assert!(scene.nodes.iter().any(|n| n.opacity < 1.0), "outside the cone dims");
}

#[test]
fn a_selected_machine_outlines_its_lane() {
    let fx = Fixture::new(SPEC_EXAMPLE);
    let scene = fx.scene(&ViewState { selection: vec![k("machine:Shipment")], ..lanes() });
    let shipment = &scene.lanes[1];
    assert_eq!(shipment.stroke.width, fx.theme.selected_stroke_width);
    assert!(scene.lanes[0].stroke.width < fx.theme.selected_stroke_width);
}

#[test]
fn hide_mode_drops_empty_lanes_and_keeps_arrows_forward() {
    let fx = Fixture::new(SHOP);
    let state = ViewState {
        selection: vec![k("transition:Order:cart->placed@checkout")],
        cone: Some(ConeFocus { direction: Direction::Forward, depth: Some(2) }),
        outside: OutsideFocus::Hide,
        ..lanes()
    };
    let scene = fx.scene(&state);
    assert!(scene.lanes.len() < fx.scene(&lanes()).lanes.len(), "{:?}", lane_titles(&scene));
    assert_eq!(leftward_edges(&scene), 0);
}

#[test]
fn hidden_machines_become_stubs_in_their_lane() {
    let fx = Fixture::new(SPEC_EXAMPLE);
    let state = ViewState { hidden_machines: ["Shipment".to_owned()].into_iter().collect(), ..lanes() };
    let scene = fx.scene(&state);
    let stub = scene
        .nodes
        .iter()
        .find(|n| matches!(&n.target, HitTarget::MachineStub { machine, .. } if machine == "Shipment"))
        .expect("stub");
    let lane = scene.lanes.iter().find(|l| l.title.text == "Shipment").expect("lane");
    assert!(lane.collapsed);
    assert!(lane.rect.contains(stub.rect.center()));
    assert_eq!(leftward_edges(&scene), 0);
}

#[test]
fn badges_match_the_flat_view() {
    let fx = with_findings(SHOP);
    let badged = |s: &Scene| {
        let mut v: Vec<String> = s.nodes.iter().filter(|n| n.badge.is_some()).map(|n| describe(&n.target)).collect();
        v.sort();
        v
    };
    assert_eq!(badged(&fx.scene(&lanes())), badged(&fx.scene(&ViewState::default())));
}

#[test]
fn toggling_lanes_off_returns_the_flat_layout() {
    let fx = Fixture::new(SHOP);
    let fresh_flat = fx.scene(&ViewState::default());
    let fresh_lanes = fx.scene(&lanes());
    let mut builder = SceneBuilder::new();
    let flat = fx.build(&mut builder, &ViewState::default());
    // The lanes are laid out afresh, not seeded by the flat layout.
    let grouped = fx.build(&mut builder, &lanes());
    assert_eq!(grouped, fresh_lanes);
    let back = fx.build(&mut builder, &ViewState::default());
    assert_eq!(back, flat);
    assert_eq!(flat, fresh_flat);
    assert_eq!(builder.layouts_run(), 2);
}

#[test]
fn an_edit_in_one_machine_moves_no_node_in_the_other_lanes() {
    let before = Fixture::new(SPEC_EXAMPLE);
    let edited = SPEC_EXAMPLE.replace(
        "      - { from: picking, to: shipped, on: handoff, emits: [Shipped] }",
        "      - { from: picking, to: shipped, on: handoff, emits: [Shipped] }\n      - { from: shipped, to: idle, on: reset }",
    );
    assert_ne!(edited, SPEC_EXAMPLE);
    let after = Fixture::new(&edited);
    let mut builder = SceneBuilder::new();
    let a = before.build(&mut builder, &lanes());
    let b = after.build(&mut builder, &lanes());
    for key in [
        "external:Customer",
        "external:PaymentGateway",
        "external:Clock",
        "transition:Order:draft->pending@submit",
        "transition:Order:pending->paid@capture_ok",
        "transition:Order:pending->cancelled@timeout",
        "event:OrderPaid",
        "event:OrderCancelled",
    ] {
        assert_eq!(node(&a, key).rect, node(&b, key).rect, "{key} moved");
    }
    assert_eq!(leftward_edges(&b), 0);
}

#[test]
fn the_play_overlay_draws_over_the_lanes() {
    let fx = Fixture::new(SPEC_EXAMPLE);
    let findings = Vec::new();
    let key = "transition:Order:pending->paid@capture_ok";
    let play = PlayOverlay { markers: Vec::new(), active: vec![k(key)], pending: Vec::new() };
    let view = lanes();
    let input = SceneInput {
        model: &fx.model,
        graph: &fx.graph,
        findings: &findings,
        view: &view,
        theme: &fx.theme,
        measure: &cascade_scene::MonoMeasure::default(),
        sidecar: &fx.sidecar,
        traces: &[],
        mode: SceneMode::View,
        play: Some(&play),
        diff: None,
    };
    let scene = SceneBuilder::new().build(&input).expect("builds");
    assert_eq!(node(&scene, key).stroke.width, fx.theme.selected_stroke_width);
    let plain = fx.scene(&lanes());
    assert_eq!(node(&scene, key).rect, node(&plain, key).rect, "the overlay never relayouts");
}
