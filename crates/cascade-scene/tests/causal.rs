//! The causal flow view: encoding, focus, filters, badges, diff, stability.

mod common;

use cascade_core::diff::{DiffStatus, ModelDiff};
use cascade_core::{CausalEdgeKind, Direction, ElementRef, Finding, FindingDetail, Severity};
use cascade_layout::Point;
use cascade_scene::{
    Arrow, Border, ConeFocus, Dash, EdgeKind, Emphasis, HitTarget, OutsideFocus, Overlay, Rgba, SceneBuilder, Shape,
    ViewState,
};
use common::*;

const OKABE_BLUE: Rgba = Rgba::hex(0x0072B2);
const OKABE_GREEN: Rgba = Rgba::hex(0x009E73);

#[test]
fn every_element_kind_has_its_encoding() {
    let fx = Fixture::new(SPEC_EXAMPLE);
    let scene = fx.scene(&ViewState::default());
    let theme = &fx.theme;

    let pill = node(&scene, "transition:Order:pending->paid@capture_ok");
    assert_eq!(pill.shape, Shape::Pill);
    assert_eq!(pill.fill, Some(OKABE_BLUE), "full machine hue");
    assert_eq!(pill.stroke.dash, Dash::Solid);
    assert_eq!(pill.border, Border::Single);
    assert_eq!(pill.labels[0].text, "Order: pending → paid");
    assert_eq!(pill.labels[1].text, "capture_ok");
    assert!(pill.labels[1].font_size < pill.labels[0].font_size, "trigger underneath, smaller");
    assert!(pill.labels[1].origin.y > pill.labels[0].origin.y);

    let event = node(&scene, "event:OrderPaid");
    assert_eq!(event.shape, Shape::Tag);
    assert_eq!(event.stroke.color, theme.neutral, "neutral gray");
    assert_eq!(event.labels[0].text, "OrderPaid");

    let controller = node(&scene, "handler:Fulfillment/OrderPaid");
    assert_eq!(controller.shape, Shape::Hexagon);
    assert_eq!(controller.fill, None, "no fill");
    assert_eq!(controller.stroke.color, theme.controller, "dark neutral outline");
    assert_eq!(controller.labels[0].text, "Fulfillment");

    let source = node(&scene, "external:Customer");
    assert_eq!(source.shape, Shape::Rect);
    assert_eq!(source.fill, None);
    assert_eq!(source.stroke.color, theme.external);

    // States are hidden in the causal view.
    assert!(
        scene
            .nodes
            .iter()
            .all(|n| !matches!(&n.target, HitTarget::Element(k) if k.kind() == cascade_core::ElementKind::State))
    );
    // 3 sources, 5 transitions, 3 events, 1 handler.
    assert_eq!(scene.nodes.len(), 12);
    for n in &scene.nodes {
        assert_eq!(n.opacity, 1.0);
        assert_eq!(n.emphasis, Emphasis::Normal);
        assert_eq!(n.stroke.width, theme.stroke_width);
        assert!(n.badge.is_none());
        assert!(n.diff.is_none());
        for label in &n.labels {
            assert!(n.rect.contains(label.origin), "{} outside {:?}", label.text, n.rect);
        }
    }
    assert!(scene.notes.is_empty());
}

#[test]
fn links_are_encoded_by_line_style_and_hue() {
    let fx = Fixture::new(SPEC_EXAMPLE);
    let scene = fx.scene(&ViewState::default());
    let theme = &fx.theme;
    assert_eq!(scene.edges.len(), fx.graph.edge_count());

    let emits = edges_of(&scene, EdgeKind::Emit);
    assert_eq!(emits.len(), 3);
    for e in &emits {
        assert!(matches!(e.stroke.dash, Dash::Dashed { .. }), "emits are dashed");
        assert_eq!(e.stroke.color, theme.neutral, "emits are gray");
    }
    let fires = edges_of(&scene, EdgeKind::Fire);
    assert_eq!(fires.len(), 1);
    assert!(matches!(fires[0].stroke.dash, Dash::Dashed { .. }));
    assert_eq!(fires[0].stroke.color, OKABE_GREEN, "fires take the target machine's hue");
    assert_eq!(fires[0].target, target("rule:Fulfillment/OrderPaid#0"));
    let triggers = edges_of(&scene, EdgeKind::Trigger);
    assert_eq!(triggers.len(), 3);
    assert!(triggers.iter().all(|e| e.stroke.dash == Dash::Solid && e.stroke.color == theme.external));
    assert_eq!(edges_of(&scene, EdgeKind::Subscribe).len(), 1);
    for e in &scene.edges {
        assert_eq!(e.arrow, Arrow::End);
        assert!(e.points.len() >= 2);
        assert!(!e.back_edge, "the spec example has no cycle");
    }
}

#[test]
fn pills_have_an_input_port_west_and_an_output_port_east() {
    let fx = Fixture::new(SPEC_EXAMPLE);
    let scene = fx.scene(&ViewState::default());
    let pill = node(&scene, "transition:Shipment:idle->picking@start");
    let fire = edges_of(&scene, EdgeKind::Fire)[0];
    let end = *fire.points.last().expect("end");
    assert_close(end.x, pill.rect.left());
    let paid = node(&scene, "transition:Order:pending->paid@capture_ok");
    let emit = edges_of(&scene, EdgeKind::Emit)
        .into_iter()
        .find(|e| touches(paid.rect, e.points[0]))
        .expect("emit leaves the paid pill");
    assert_close(emit.points[0].x, paid.rect.right());
}

#[test]
fn external_sources_sit_in_the_first_layer() {
    // Engines align nodes within a layer column differently (flush left,
    // centred), so this checks the column rather than exact x positions.
    let fx = Fixture::new(SPEC_EXAMPLE);
    let scene = fx.scene(&ViewState::default());
    let sources: Vec<_> = ["external:Customer", "external:PaymentGateway", "external:Clock"]
        .iter()
        .map(|key| node(&scene, key).rect)
        .collect();
    // One column: every source overlaps every other horizontally.
    for a in &sources {
        for b in &sources {
            assert!(a.left() < b.right() && b.left() < a.right(), "{a:?} and {b:?} are in different columns");
        }
    }
    // The leftmost column: nothing lies entirely left of a source.
    for n in &scene.nodes {
        for s in &sources {
            assert!(n.rect.right() > s.left(), "{} lies left of a source", describe(&n.target));
        }
    }
    // Everything triggered or fired lies entirely right of every source.
    let caused: std::collections::BTreeSet<String> = fx
        .graph
        .edges()
        .filter(|(_, e)| matches!(e.kind, CausalEdgeKind::Trigger { .. } | CausalEdgeKind::Fire { .. }))
        .map(|(_, e)| fx.model.key_of(fx.graph.node(e.to).element()).to_string())
        .collect();
    assert_eq!(caused.len(), 4, "three triggered transitions and one fired one");
    for key in &caused {
        let pill = node(&scene, key).rect;
        for s in &sources {
            assert!(pill.left() > s.right(), "{key} is not right of the sources");
        }
    }
}

#[test]
fn guards_and_conditions_label_the_arrows_into_a_transition() {
    let fx = Fixture::new(
        r#"
machines:
  Order:
    states: [pending, paid]
    transitions:
      - { from: pending, to: paid, on: pay, guard: "amount > 0", emits: [Paid] }
  Ship:
    states: [idle, busy]
    transitions:
      - { from: idle, to: busy, on: start, guard: "stock" }
controllers:
  F:
    on:
      Paid: [{ fire: Ship.start, when: "not gift" }]
external:
  Customer: [Order.pay]
"#,
    );
    let scene = fx.scene(&ViewState::default());
    let trigger = edges_of(&scene, EdgeKind::Trigger)[0];
    assert_eq!(trigger.label.as_ref().map(|l| l.text.as_str()), Some("[amount > 0]"));
    assert_eq!(trigger.label.as_ref().map(|l| l.color), Some(fx.theme.text), "default text color");
    let fire = edges_of(&scene, EdgeKind::Fire)[0];
    assert_eq!(fire.label.as_ref().map(|l| l.text.as_str()), Some("[not gift] [stock]"));
    assert!(edges_of(&scene, EdgeKind::Emit)[0].label.is_none());
}

#[test]
fn cascade_cycle_back_edges_are_red() {
    let fx = Fixture::new(RETRY);
    let scene = fx.scene(&ViewState::default());
    let back: Vec<_> = scene.edges.iter().filter(|e| e.back_edge).collect();
    assert_eq!(back.len(), 1, "one edge reversed to break the retry loop");
    assert_eq!(back[0].stroke.color, fx.theme.finding);
    for e in scene.edges.iter().filter(|e| !e.back_edge) {
        assert_ne!(e.stroke.color, fx.theme.finding);
    }
}

fn cone(selection: &str, direction: Direction, depth: Option<u32>) -> ViewState {
    ViewState { cone: Some(ConeFocus { direction, depth }), ..view(&[selection]) }
}

fn dimmed(scene: &cascade_scene::Scene) -> Vec<String> {
    let mut out: Vec<String> =
        scene.nodes.iter().filter(|n| n.emphasis == Emphasis::Dimmed).map(|n| describe(&n.target)).collect();
    out.sort();
    out
}

#[test]
fn forward_cone_dims_everything_outside_to_fifteen_percent() {
    let fx = Fixture::new(CHAIN);
    let scene = fx.scene(&cone("transition:A:a0->a1@go", Direction::Forward, Some(1)));
    let theme = &fx.theme;
    assert_eq!(
        dimmed(&scene),
        [
            "external:User",
            "event:Back",
            "handler:C3/Back",
            "transition:A:a1->a0@reset",
            "transition:B:b1->b0@rewind",
            "transition:D:d0->d1@note",
        ]
        .into_iter()
        .map(String::from)
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>()
    );
    for n in &scene.nodes {
        match n.emphasis {
            Emphasis::Dimmed => assert_eq!(n.opacity, theme.dim_opacity),
            Emphasis::Selected => {
                assert_eq!(n.target, target("transition:A:a0->a1@go"));
                assert_eq!(n.stroke.width, theme.selected_stroke_width);
                assert_eq!(n.opacity, 1.0);
            }
            Emphasis::Focused => {
                assert_eq!(n.opacity, 1.0);
                assert!(n.stroke.width > theme.stroke_width && n.stroke.width < theme.selected_stroke_width);
            }
            other => panic!("unexpected {other:?} on {:?}", n.target),
        }
    }
    // The selection keeps its hue: emphasis never changes color.
    let selected = node(&scene, "transition:A:a0->a1@go");
    let plain = node(&fx.scene(&ViewState::default()), "transition:A:a0->a1@go").clone();
    assert_eq!(selected.fill, plain.fill);
    assert_eq!(selected.stroke.color, plain.stroke.color);
    // Edges inside the cone are focused, the rest dimmed.
    let focused_edges = scene.edges.iter().filter(|e| e.emphasis == Emphasis::Focused).count();
    assert_eq!(focused_edges, 5, "emit Go, subscribe C1, fire B, emit Done, subscribe C2");
    assert!(scene.edges.iter().filter(|e| e.emphasis == Emphasis::Dimmed).all(|e| e.opacity == theme.dim_opacity));
}

#[test]
fn backward_cone_with_depth() {
    let fx = Fixture::new(CHAIN);
    let unlimited = fx.scene(&cone("transition:D:d0->d1@note", Direction::Backward, None));
    assert_eq!(
        dimmed(&unlimited),
        ["event:Back", "handler:C3/Back", "transition:A:a1->a0@reset", "transition:B:b1->b0@rewind"]
    );
    let one = fx.scene(&cone("transition:D:d0->d1@note", Direction::Backward, Some(1)));
    assert_eq!(
        dimmed(&one),
        [
            "event:Back",
            "external:User",
            "handler:C3/Back",
            "transition:A:a0->a1@go",
            "transition:A:a1->a0@reset",
            "transition:B:b1->b0@rewind",
        ]
    );
}

#[test]
fn path_query_shows_every_path_and_nothing_else() {
    let fx = Fixture::new(CHAIN);
    let scene = fx.scene(&view(&["transition:A:a0->a1@go", "transition:D:d0->d1@note"]));
    assert_eq!(
        dimmed(&scene),
        ["event:Back", "external:User", "handler:C3/Back", "transition:A:a1->a0@reset", "transition:B:b1->b0@rewind",]
    );
    let selected: Vec<_> = scene.nodes.iter().filter(|n| n.emphasis == Emphasis::Selected).collect();
    assert_eq!(selected.len(), 2);
    assert_eq!(scene.edges.iter().filter(|e| e.emphasis != Emphasis::Dimmed).count(), 6);

    let none = fx.scene(&view(&["transition:D:d0->d1@note", "transition:B:b1->b0@rewind"]));
    assert_eq!(none.notes.len(), 1, "an empty path query says so");
    assert_eq!(none.nodes.iter().filter(|n| n.emphasis == Emphasis::Dimmed).count(), none.nodes.len() - 2);
}

#[test]
fn hide_mode_removes_outside_items_and_leaves_stubs_for_cut_links() {
    let fx = Fixture::new(CHAIN);
    let mut state = cone("transition:B:b0->b1@start", Direction::Forward, Some(0));
    state.outside = OutsideFocus::Hide;
    let scene = fx.scene(&state);
    let mut shown = node_targets(&scene);
    shown.sort();
    assert_eq!(shown, ["event:Done", "handler:C2/Done", "transition:B:b0->b1@start"]);

    let stubs: Vec<_> = scene.edges.iter().filter(|e| e.stroke.dash == Dash::Dotted).collect();
    assert_eq!(stubs.len(), 3, "C1 fires into B; C2 fires A and D");
    let mut stub_targets: Vec<String> = stubs.iter().map(|e| describe(&e.target)).collect();
    stub_targets.sort();
    assert_eq!(stub_targets, ["handler:C1/Go", "transition:A:a1->a0@reset", "transition:D:d0->d1@note"]);
    let b = node(&scene, "transition:B:b0->b1@start");
    let incoming = stubs.iter().find(|e| e.target == target("handler:C1/Go")).expect("stub");
    assert_close(incoming.points.last().expect("end").x, b.rect.left());
    // Real edges: emit Done and subscribe C2.
    assert_eq!(scene.edges.len() - stubs.len(), 2);
    assert!(scene.nodes.iter().all(|n| n.emphasis != Emphasis::Dimmed));
}

#[test]
fn hidden_machines_collapse_to_a_stub_that_keeps_its_links() {
    let fx = Fixture::new(SPEC_EXAMPLE);
    let state = ViewState { hidden_machines: ["Shipment".to_owned()].into_iter().collect(), ..ViewState::default() };
    let scene = fx.scene(&state);
    assert!(find_node(&scene, "transition:Shipment:idle->picking@start").is_none());
    let stub = scene
        .nodes
        .iter()
        .find(|n| matches!(&n.target, HitTarget::MachineStub { machine, .. } if machine == "Shipment"))
        .expect("stub");
    assert_eq!(stub.shape, Shape::Stub);
    assert_eq!(stub.target, HitTarget::MachineStub { machine: "Shipment".into(), links: 2 });
    assert_eq!(stub.labels[0].text, "Shipment, 2 links");
    assert!(matches!(stub.stroke.dash, Dash::Dashed { .. }), "stubs are dashed");
    let links = edges_of(&scene, EdgeKind::StubLink);
    assert_eq!(links.len(), 2, "Fulfillment fires into it; it emits Shipped");
    assert!(links.iter().all(|e| e.stroke.dash == Dash::Dotted));
    let into = links.iter().find(|e| touches(stub.rect, *e.points.last().expect("end"))).expect("fire into stub");
    assert_eq!(into.stroke.color, OKABE_GREEN, "a rerouted fire keeps the target hue");
    assert_eq!(edges_of(&scene, EdgeKind::Fire).len(), 0);
}

#[test]
fn stub_links_aggregate_parallel_links() {
    let fx = Fixture::new(
        r#"
machines:
  Order:
    states: [pending, paid]
    transitions:
      - { from: pending, to: paid, on: pay, emits: [Paid] }
  Shipment:
    states: [idle, held, picking]
    transitions:
      - { from: idle, to: picking, on: start }
      - { from: held, to: picking, on: start }
controllers:
  Fulfillment:
    on:
      Paid: [{ fire: Shipment.start }]
external:
  Customer: [Order.pay]
"#,
    );
    let state = ViewState {
        hidden_machines: ["Shipment".to_owned(), "Nope".to_owned()].into_iter().collect(),
        ..ViewState::default()
    };
    let scene = fx.scene(&state);
    let links = edges_of(&scene, EdgeKind::StubLink);
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].label.as_ref().map(|l| l.text.as_str()), Some("×2"));
    let stub = scene.nodes.iter().find(|n| n.shape == Shape::Stub).expect("stub");
    assert_eq!(stub.labels[0].text, "Shipment, 1 link");
    assert_eq!(scene.nodes.iter().filter(|n| n.shape == Shape::Stub).count(), 1, "unknown names are ignored");
}

#[test]
fn machine_pair_restricts_to_the_two_machines_and_their_links() {
    let fx = Fixture::new(CHAIN);
    let state = ViewState { machine_pair: Some(("A".into(), "B".into())), ..ViewState::default() };
    let scene = fx.scene(&state);
    let mut shown = node_targets(&scene);
    shown.sort();
    assert_eq!(
        shown,
        [
            "event:Back",
            "event:Done",
            "event:Go",
            "external:User",
            "handler:C1/Go",
            "handler:C2/Done",
            "handler:C3/Back",
            "transition:A:a0->a1@go",
            "transition:A:a1->a0@reset",
            "transition:B:b0->b1@start",
            "transition:B:b1->b0@rewind",
        ]
    );
    // C2's fire into D is gone with D.
    assert_eq!(edges_of(&scene, EdgeKind::Fire).len(), 3);
}

#[test]
fn findings_badge_their_subjects_with_count_and_highest_severity() {
    let mut fx = Fixture::new(SPEC_EXAMPLE);
    let shipped = match fx.element("event:Shipped") {
        ElementRef::Event(e) => e,
        other => panic!("{other:?}"),
    };
    let rule = match fx.element("rule:Fulfillment/OrderPaid#0") {
        ElementRef::Rule(r) => r,
        other => panic!("{other:?}"),
    };
    let trigger = fx.model.rule(rule).trigger;
    fx.findings = vec![
        Finding {
            severity: Severity::Error,
            detail: FindingDetail::InvalidFire { rule, trigger },
            message: "x".into(),
        },
        Finding {
            severity: Severity::Warning,
            detail: FindingDetail::UnhandledEvent { event: shipped },
            message: "y".into(),
        },
        Finding {
            severity: Severity::Info,
            detail: FindingDetail::StateDependentFire { rule, trigger, dropped_in: Vec::new() },
            message: "z".into(),
        },
    ];
    let scene = fx.scene(&ViewState::default());
    let event = node(&scene, "event:Shipped");
    let badge = event.badge.as_ref().expect("badge");
    assert_eq!((badge.count, badge.severity), (1, Severity::Warning));
    assert_eq!(event.stroke.color, fx.theme.finding, "red outline");
    let handler = node(&scene, "handler:Fulfillment/OrderPaid");
    let badge = handler.badge.as_ref().expect("badge");
    assert_eq!((badge.count, badge.severity), (2, Severity::Error));
    assert_eq!(handler.stroke.color, fx.theme.finding);
    assert!(node(&scene, "event:OrderPaid").badge.is_none());
    assert_eq!(node(&scene, "event:OrderPaid").stroke.color, fx.theme.neutral);
}

#[test]
fn diff_mode_outlines_added_and_ghosts_removed() {
    let mut fx = Fixture::new(SPEC_EXAMPLE);
    fx.diff = Some(ModelDiff::from_statuses([
        (k("transition:Order:pending->paid@capture_ok"), DiffStatus::Added),
        (k("event:OrderCancelled"), DiffStatus::Removed),
        (k("event:OrderPaid"), DiffStatus::Changed),
    ]));
    let scene = fx.scene(&ViewState::default());
    let added = node(&scene, "transition:Order:pending->paid@capture_ok");
    assert_eq!(added.diff, Some(DiffStatus::Added));
    assert_eq!(added.stroke.color, fx.theme.added);
    assert_eq!(added.fill, Some(OKABE_BLUE), "the hue is kept");
    let removed = node(&scene, "event:OrderCancelled");
    assert_eq!(removed.diff, Some(DiffStatus::Removed));
    assert_eq!(removed.stroke.color, fx.theme.removed);
    assert!(matches!(removed.stroke.dash, Dash::Dashed { .. }));
    assert!(removed.opacity < 1.0);
    assert_eq!(node(&scene, "event:OrderPaid").diff, Some(DiffStatus::Changed));
    assert_eq!(node(&scene, "event:Shipped").diff, Some(DiffStatus::Unchanged));
    // The emit into the removed event is a ghost too.
    let ghost_edges: Vec<_> = scene.edges.iter().filter(|e| e.diff == Some(DiffStatus::Removed)).collect();
    assert_eq!(ghost_edges.len(), 1);
    assert!(ghost_edges[0].opacity < 1.0);
    assert!(scene.edges.iter().all(|e| e.diff.is_some()));
}

#[test]
fn search_matches_get_a_subtle_halo_without_changing_hue() {
    let fx = Fixture::new(SPEC_EXAMPLE);
    let state = ViewState { search: Some("fulfil".into()), ..ViewState::default() };
    let scene = fx.scene(&state);
    let handler = node(&scene, "handler:Fulfillment/OrderPaid");
    assert_eq!(handler.emphasis, Emphasis::SearchMatch);
    assert_eq!(handler.stroke.color, fx.theme.controller);
    assert_eq!(handler.opacity, 1.0);
    let halos: Vec<_> = scene
        .overlays
        .iter()
        .filter(|o| matches!(o, Overlay::Rect { stroke: Some(s), .. } if s.dash == Dash::Dotted))
        .collect();
    assert_eq!(halos.len(), scene.nodes.iter().filter(|n| n.emphasis == Emphasis::SearchMatch).count());
    assert!(!halos.is_empty());
}

#[test]
fn emphasis_changes_never_relayout() {
    let mut fx = Fixture::new(CHAIN);
    let mut builder = SceneBuilder::new();
    let rects = |s: &cascade_scene::Scene| s.nodes.iter().map(|n| (describe(&n.target), n.rect)).collect::<Vec<_>>();
    let base = fx.build(&mut builder, &ViewState::default());
    let selected = fx.build(&mut builder, &view(&["transition:B:b0->b1@start"]));
    let coned = fx.build(&mut builder, &cone("transition:B:b0->b1@start", Direction::Backward, Some(2)));
    let path = fx.build(&mut builder, &view(&["transition:A:a0->a1@go", "transition:D:d0->d1@note"]));
    let searched = fx.build(&mut builder, &ViewState { search: Some("C2".into()), ..ViewState::default() });
    fx.diff = Some(ModelDiff::from_statuses([(k("event:Go"), DiffStatus::Added)]));
    let diffed = fx.build(&mut builder, &ViewState::default());
    assert_eq!(builder.layouts_run(), 1);
    for scene in [&selected, &coned, &path, &searched, &diffed] {
        assert_eq!(rects(scene), rects(&base));
    }

    // Hiding relayouts; going back to dimming restores the same picture.
    let mut hide = cone("transition:B:b0->b1@start", Direction::Forward, Some(0));
    hide.outside = OutsideFocus::Hide;
    let hidden = fx.build(&mut builder, &hide);
    assert!(hidden.nodes.len() < base.nodes.len());
    assert_eq!(builder.layouts_run(), 2);
    let again = fx.build(&mut builder, &ViewState::default());
    assert_eq!(builder.layouts_run(), 2);
    assert_eq!(rects(&again), rects(&base));

    builder.reset();
    let _ = fx.build(&mut builder, &ViewState::default());
    assert_eq!(builder.layouts_run(), 3);
}

#[test]
fn pins_place_nodes_exactly() {
    let mut fx = Fixture::new(SPEC_EXAMPLE);
    fx.sidecar.pin(cascade_scene::ViewKind::Causal, k("event:OrderPaid"), Point::new(900.0, 700.0));
    fx.sidecar.pin(cascade_scene::ViewKind::Structure, k("event:Shipped"), Point::new(5.0, 5.0));
    let scene = fx.scene(&ViewState::default());
    assert_eq!(node(&scene, "event:OrderPaid").rect.origin, Point::new(900.0, 700.0));
    assert_ne!(node(&scene, "event:Shipped").rect.origin, Point::new(5.0, 5.0), "pins are per view");
    assert!(scene.bounds.right() >= node(&scene, "event:OrderPaid").rect.right());
}

#[test]
fn bounds_cover_every_item() {
    let fx = Fixture::new(SPEC_EXAMPLE);
    let scene = fx.scene(&ViewState::default());
    for n in &scene.nodes {
        assert!(
            scene.bounds.contains(n.rect.origin) && scene.bounds.contains(Point::new(n.rect.right(), n.rect.bottom()))
        );
    }
    for e in &scene.edges {
        assert!(e.points.iter().all(|p| scene.bounds.contains(*p)));
    }
}

#[test]
fn dark_theme_keeps_the_encoding() {
    let mut fx = Fixture::new(SPEC_EXAMPLE);
    fx.theme = cascade_scene::Theme::dark();
    let scene = fx.scene(&ViewState::default());
    assert_eq!(scene.background, fx.theme.background);
    assert_eq!(node(&scene, "handler:Fulfillment/OrderPaid").stroke.color, fx.theme.controller);
}
