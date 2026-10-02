//! Arrow mode (`ViewState::transition_pills` off): the structure view draws
//! each transition as one labelled state → state arrow instead of a pill,
//! in view and edit mode alike, and wires attach to a point on the arrow.

mod build_play;
mod common;

use build_play::*;
use cascade_core::ElementKey;
use cascade_layout::Point;
use cascade_scene::metrics::measure;
use cascade_scene::scene::polyline_distance;
use cascade_scene::{
    Arrow, EdgeKind, HitTarget, Overlay, PlayOverlay, Scene, SceneBuilder, SceneEdge, SceneMode, Shape, ViewKind,
    ViewLinkError, ViewState, to_svg,
};
use common::{RETRY, SPEC_EXAMPLE};

fn arrows() -> ViewState {
    ViewState { view: ViewKind::Structure, transition_pills: false, ..ViewState::default() }
}

fn arrow_scene(yaml: &str, mode: SceneMode) -> Scene {
    Bench::new(yaml).build_with(&mut SceneBuilder::new(), &arrows(), mode, None)
}

/// The transition arrows: transition edges targeting a transition.
fn transition_arrows(scene: &Scene) -> Vec<&SceneEdge> {
    scene
        .edges
        .iter()
        .filter(|e| {
            e.kind == EdgeKind::Transition && matches!(&e.target, HitTarget::Element(ElementKey::Transition { .. }))
        })
        .collect()
}

fn arrow_for<'a>(scene: &'a Scene, key: &str) -> &'a SceneEdge {
    let found: Vec<_> = transition_arrows(scene).into_iter().filter(|e| e.target == target(key)).collect();
    assert_eq!(found.len(), 1, "one arrow for {key}");
    found[0]
}

fn dots(scene: &Scene) -> Vec<Point> {
    scene
        .overlays
        .iter()
        .filter_map(|o| match o {
            Overlay::Rect { rect, target: HitTarget::None, radius, fill: Some(_), stroke: None, .. }
                if (*radius - rect.size.width / 2.0).abs() < 0.01 && rect.size.width < 10.0 =>
            {
                Some(rect.center())
            }
            _ => None,
        })
        .collect()
}

// --- Links ------------------------------------------------------------------

#[test]
fn pills_are_on_by_default_and_left_out_of_links() {
    assert!(ViewState::default().transition_pills);
    assert_eq!(ViewState { view: ViewKind::Structure, ..ViewState::default() }.to_link(), "cascade://structure");
    assert!(ViewState::from_link("cascade://structure").expect("parses").transition_pills);
}

#[test]
fn pills_off_round_trips_as_pills_0() {
    let state = arrows();
    assert_eq!(state.to_link(), "cascade://structure?pills=0");
    assert_eq!(ViewState::from_link("cascade://structure?pills=0"), Ok(state.clone()));
    let with_more = ViewState { selection: vec![k("state:Order:paid")], group_by_machine: true, ..state };
    assert_eq!(ViewState::from_link(&with_more.to_link()), Ok(with_more));
    assert!(ViewState::from_link("cascade://structure?pills=1").expect("parses").transition_pills);
}

#[test]
fn other_pill_values_are_rejected() {
    for bad in ["cascade://structure?pills=no", "cascade://structure?pills=", "cascade://structure?pills=2"] {
        assert!(matches!(ViewState::from_link(bad), Err(ViewLinkError::InvalidValue { param: "pills", .. })), "{bad}");
    }
}

// --- Drawing ------------------------------------------------------------------

#[test]
fn every_transition_is_one_labelled_arrow_between_its_states() {
    for mode in [SceneMode::View, SceneMode::Edit] {
        let scene = arrow_scene(SPEC_EXAMPLE, mode);
        assert!(scene.nodes.iter().all(|n| n.shape != Shape::Pill), "no pills ({mode:?})");
        assert!(
            scene.nodes.iter().all(|n| !matches!(&n.target, HitTarget::Element(ElementKey::Transition { .. }))),
            "no node stands for a transition ({mode:?})"
        );
        for (key, from, to, label) in [
            ("transition:Order:draft->pending@submit", "state:Order:draft", "state:Order:pending", "submit"),
            ("transition:Order:pending->paid@capture_ok", "state:Order:pending", "state:Order:paid", "capture_ok"),
            ("transition:Order:pending->cancelled@timeout", "state:Order:pending", "state:Order:cancelled", "timeout"),
            ("transition:Shipment:idle->picking@start", "state:Shipment:idle", "state:Shipment:picking", "start"),
            (
                "transition:Shipment:picking->shipped@handoff",
                "state:Shipment:picking",
                "state:Shipment:shipped",
                "handoff",
            ),
        ] {
            let arrow = arrow_for(&scene, key);
            assert_eq!(arrow.arrow, Arrow::End, "{key}");
            assert!(touches(node_rect(&scene, from), arrow.points[0]), "{key} leaves {from}");
            let last = *arrow.points.last().expect("points");
            assert!(touches(node_rect(&scene, to), last), "{key} enters {to}");
            assert_eq!(arrow.label.as_ref().map(|l| l.text.as_str()), Some(label), "{key}");
        }
        assert_eq!(transition_arrows(&scene).len(), 5, "{mode:?}");
    }
}

#[test]
fn guards_follow_the_trigger_in_brackets() {
    let scene = arrow_scene(SHOP, SceneMode::View);
    let labels: Vec<&str> =
        transition_arrows(&scene).iter().filter_map(|e| e.label.as_ref()).map(|l| l.text.as_str()).collect();
    assert!(labels.contains(&"capture [amount <= authorized_amount]"), "{labels:?}");
    assert!(labels.contains(&"capture [else]"), "{labels:?}");
    assert!(labels.contains(&"reserve [stock < quantity]"), "{labels:?}");
}

#[test]
fn arrows_take_the_machine_hue_and_selection_is_weight_only() {
    let bench = Bench::new(SPEC_EXAMPLE);
    let key = "transition:Order:pending->paid@capture_ok";
    let plain = bench.build_with(&mut SceneBuilder::new(), &arrows(), SceneMode::View, None);
    let selected_state = ViewState { selection: vec![k(key)], ..arrows() };
    let selected = bench.build_with(&mut SceneBuilder::new(), &selected_state, SceneMode::View, None);
    let (a, b) = (arrow_for(&plain, key), arrow_for(&selected, key));
    let order = plain.lanes.iter().find(|l| l.target == target("machine:Order")).expect("Order lane");
    assert_eq!(a.stroke.color, order.title.color, "the machine's hue");
    assert_eq!(b.stroke.color, a.stroke.color, "selection never changes the hue");
    assert!(b.stroke.width > a.stroke.width, "selection thickens the arrow");
    assert_eq!(b.stroke.width, bench.theme.selected_stroke_width);
}

#[test]
fn self_loops_still_read_as_loops() {
    let scene = arrow_scene(RETRY, SceneMode::View);
    let arrow = arrow_for(&scene, "transition:R:s->s@retry");
    let s = node_rect(&scene, "state:R:s");
    assert!(touches(s, arrow.points[0]) && touches(s, *arrow.points.last().expect("points")));
    assert!(arrow.points.iter().any(|p| !s.contains(*p)), "the loop leaves the state");
    assert!(arrow.points.len() >= 4, "a loop has corners: {:?}", arrow.points);
}

#[test]
fn view_mode_links_attach_to_the_arrows_with_a_dot() {
    let scene = arrow_scene(SPEC_EXAMPLE, SceneMode::View);
    let paid = arrow_for(&scene, "transition:Order:pending->paid@capture_ok");
    let start = arrow_for(&scene, "transition:Shipment:idle->picking@start");
    let links: Vec<_> = scene.edges.iter().filter(|e| e.kind == EdgeKind::Fire).collect();
    assert_eq!(links.len(), 1);
    let link = links[0];
    let (first, last) = (link.points[0], *link.points.last().expect("points"));
    assert!(polyline_distance(&paid.points, first) < 0.5, "starts on the causing arrow");
    assert!(polyline_distance(&start.points, last) < 0.5, "ends on the caused arrow");
    let dots = dots(&scene);
    for end in [first, last] {
        assert!(dots.iter().any(|d| d.distance(end) < 0.5), "a dot at {end:?}: {dots:?}");
    }
}

#[test]
fn build_canvas_wires_attach_to_the_arrows_and_transitions_have_no_handles() {
    let scene = arrow_scene(SPEC_EXAMPLE, SceneMode::Edit);
    let handles = handles(&scene);
    assert!(handles.keys().all(|k| !k.starts_with("transition:")), "{handles:?}");
    for key in ["state:Order:draft", "controller:Fulfillment", "external:Customer"] {
        assert!(handles.contains_key(key), "{key} keeps its handle");
    }
    let arrows = transition_arrows(&scene);
    let dots = dots(&scene);
    let mut ends = 0;
    for wire in scene.edges.iter().filter(|e| matches!(e.kind, EdgeKind::Emit | EdgeKind::Fire | EdgeKind::Trigger)) {
        let (first, last) = (wire.points[0], *wire.points.last().expect("points"));
        let end = match wire.kind {
            EdgeKind::Emit => first,
            _ => last,
        };
        assert!(
            arrows.iter().any(|a| polyline_distance(&a.points, end) < 0.5),
            "{:?} {:?} ends on an arrow at {end:?}",
            wire.kind,
            wire.target
        );
        assert!(dots.iter().any(|d| d.distance(end) < 0.5), "a dot where {:?} meets its arrow", wire.target);
        ends += 1;
    }
    // Emits OrderPaid, OrderCancelled, Shipped; one fire; three triggers.
    assert_eq!(ends, 7);
}

#[test]
fn the_causal_view_ignores_the_setting() {
    let bench = Bench::new(SPEC_EXAMPLE);
    let pills = bench.build_with(&mut SceneBuilder::new(), &causal(), SceneMode::View, None);
    let off = ViewState { transition_pills: false, ..causal() };
    let arrows = bench.build_with(&mut SceneBuilder::new(), &off, SceneMode::View, None);
    assert_eq!(pills, arrows);
    assert!(pills.nodes.iter().any(|n| n.shape == Shape::Pill));
}

#[test]
fn emphasis_never_relayouts_in_arrow_mode() {
    let bench = Bench::new(SPEC_EXAMPLE);
    let mut builder = SceneBuilder::new();
    let first = bench.build_with(&mut builder, &arrows(), SceneMode::Edit, None);
    let runs = builder.layouts_run();
    let selected = ViewState { selection: vec![k("transition:Order:pending->paid@capture_ok")], ..arrows() };
    let second = bench.build_with(&mut builder, &selected, SceneMode::Edit, None);
    assert_eq!(builder.layouts_run(), runs);
    assert_eq!(rects(&first), rects(&second));
}

#[test]
fn toggling_keeps_states_in_their_lanes_and_returns_exactly() {
    let bench = Bench::new(SHOP);
    let mut builder = SceneBuilder::new();
    let pills = bench.build_with(&mut builder, &structure(), SceneMode::Edit, None);
    let arrows_scene = bench.build_with(&mut builder, &arrows(), SceneMode::Edit, None);
    let back = bench.build_with(&mut builder, &structure(), SceneMode::Edit, None);
    assert_eq!(pills, back, "switching back hits the cache");
    for lane in pills.lanes.iter().filter(|l| matches!(&l.target, HitTarget::Element(ElementKey::Machine { .. }))) {
        let other = arrows_scene.lanes.iter().find(|l| l.target == lane.target).expect("the lane is drawn");
        assert_eq!(lane.title.text, other.title.text);
    }
}

#[test]
fn findings_on_a_transition_badge_its_arrow() {
    let bench = Bench::new(SHOP);
    let findings = cascade_core::analyze(&bench.model, &bench.graph);
    let errors: Vec<_> = findings.iter().filter(|f| f.severity == cascade_core::Severity::Error).collect();
    let input = cascade_scene::SceneInput {
        model: &bench.model,
        graph: &bench.graph,
        findings: &findings,
        view: &arrows(),
        theme: &bench.theme,
        measure: &cascade_scene::MonoMeasure::default(),
        sidecar: &bench.sidecar,
        traces: &[],
        diff: None,
        mode: SceneMode::Edit,
        play: None,
    };
    let scene = SceneBuilder::new().build(&input).expect("builds");
    let badged: Vec<_> =
        transition_arrows(&scene).into_iter().filter(|e| e.stroke.color == bench.theme.finding).collect();
    let transition_errors = errors
        .iter()
        .flat_map(|f| f.detail.subjects())
        .filter(|e| matches!(e, cascade_core::ElementRef::Transition(_)))
        .count();
    if transition_errors > 0 {
        assert!(!badged.is_empty(), "error findings on transitions turn their arrows red");
        // Each badged arrow has a badge circle aimed at its transition.
        for arrow in badged {
            assert!(
                scene.overlays.iter().any(|o| matches!(o, Overlay::Rect { target, .. } if *target == arrow.target))
            );
        }
    }
}

#[test]
fn play_overlays_thicken_the_active_arrow() {
    let bench = Bench::new(SPEC_EXAMPLE);
    let key = "transition:Order:pending->paid@capture_ok";
    let play = PlayOverlay {
        markers: vec![marker("o1", "state:Order:paid")],
        active: vec![k(key)],
        pending: vec![k("event:OrderPaid")],
    };
    let scene = bench.build_with(&mut SceneBuilder::new(), &arrows(), SceneMode::View, Some(&play));
    assert_eq!(arrow_for(&scene, key).stroke.width, bench.theme.selected_stroke_width);
}

#[test]
fn hide_mode_collapse_and_hidden_machines_build() {
    let bench = Bench::new(SHOP);
    let selected = ViewState {
        selection: vec![k("transition:Order:cart->placed@checkout")],
        cone: Some(cascade_scene::ConeFocus { direction: cascade_core::Direction::Forward, depth: None }),
        outside: cascade_scene::OutsideFocus::Hide,
        ..arrows()
    };
    let collapsed = ViewState { collapsed: [k("machine:Payment")].into_iter().collect(), ..arrows() };
    let hidden = ViewState { hidden_machines: ["Payment".to_owned()].into_iter().collect(), ..arrows() };
    for state in [selected, collapsed, hidden] {
        for mode in [SceneMode::View, SceneMode::Edit] {
            let scene = bench.build_with(&mut SceneBuilder::new(), &state, mode, None);
            assert!(scene.nodes.iter().all(|n| n.shape != Shape::Pill));
            assert!(scene.edges.iter().all(|e| e.points.len() >= 2));
            to_svg(&scene).expect("exports");
        }
    }
}

#[test]
fn builds_are_deterministic() {
    assert_eq!(arrow_scene(SHOP, SceneMode::Edit), arrow_scene(SHOP, SceneMode::Edit));
}

#[test]
fn arrow_hits_select_their_transition() {
    let scene = arrow_scene(SPEC_EXAMPLE, SceneMode::Edit);
    let key = "transition:Shipment:picking->shipped@handoff";
    let arrow = arrow_for(&scene, key);
    let mid = arrow.points[0]
        .offset((arrow.points[1].x - arrow.points[0].x) / 2.0, (arrow.points[1].y - arrow.points[0].y) / 2.0);
    assert_eq!(scene.hit_test(mid, 3.0), Some(&target(key)));
    assert_eq!(scene.arrow_at(mid.offset(0.0, 6.0), 8.0), Some(&k(key)));
    assert_eq!(
        scene.hit_test_arrows(mid.offset(0.0, 6.0), 3.0, 8.0),
        Some(HitTarget::ConnectHandle { element: k(key) })
    );
}

// --- Readability ------------------------------------------------------------

/// Arrow mode reads at least as well as pill mode on crossings, length,
/// label overlaps and corridor edges, for both examples, in both modes.
#[test]
fn arrows_read_at_least_as_well_as_pills() {
    for yaml in [SPEC_EXAMPLE, SHOP] {
        for mode in [SceneMode::View, SceneMode::Edit] {
            let bench = Bench::new(yaml);
            let pills = measure(&bench.build_with(&mut SceneBuilder::new(), &structure(), mode, None));
            let arrows = measure(&bench.build_with(&mut SceneBuilder::new(), &arrows(), mode, None));
            assert!(arrows.edge_crossings <= pills.edge_crossings, "{mode:?} crossings: {arrows} vs {pills}");
            assert!(arrows.total_edge_length <= pills.total_edge_length, "{mode:?} length: {arrows} vs {pills}");
            assert!(arrows.label_overlaps <= pills.label_overlaps, "{mode:?} labels: {arrows} vs {pills}");
            assert!(arrows.corridor_edges <= pills.corridor_edges, "{mode:?} corridor: {arrows} vs {pills}");
            assert_eq!(arrows.edges_through_nodes, 0, "{mode:?}: {arrows}");
        }
    }
}
