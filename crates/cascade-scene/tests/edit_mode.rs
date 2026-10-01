//! `SceneMode::Edit` in the structure view: the build canvas. Machine lanes
//! as in view mode, plus wiring in gutters between them (event tags,
//! controller hexagons, source boxes) with real emit, subscribe, fire and
//! trigger edges, and a connect handle on every connectable element.

mod build_play;
mod common;

use build_play::*;
use cascade_core::ElementKey;
use cascade_scene::{
    Dash, EdgeKind, HitTarget, Overlay, Rgba, Scene, SceneBuilder, SceneEdge, SceneMode, Shape, ViewKind, ViewState,
    to_svg,
};
use common::{CHAIN, SPEC_EXAMPLE};

const OKABE_GREEN: Rgba = Rgba::hex(0x009E73);

fn node_shape(scene: &Scene, key: &str) -> Shape {
    let t = target(key);
    scene.nodes.iter().find(|n| n.target == t).map(|n| n.shape).unwrap_or_else(|| panic!("no node {key}"))
}

/// The one edge of `kind` whose ends touch the two nodes.
fn edge_between<'a>(scene: &'a Scene, kind: EdgeKind, from: &str, to: &str) -> &'a SceneEdge {
    let (a, b) = (node_rect(scene, from), node_rect(scene, to));
    let found: Vec<_> = scene
        .edges
        .iter()
        .filter(|e| e.kind == kind)
        .filter(|e| touches(a, e.points[0]) && e.points.last().is_some_and(|p| touches(b, *p)))
        .collect();
    assert_eq!(found.len(), 1, "{kind:?} edges {from} → {to}");
    found[0]
}

#[test]
fn edit_mode_puts_the_wiring_in_gutters_between_the_lanes() {
    let bench = Bench::new(SPEC_EXAMPLE);
    let scene = bench.edit();
    assert_eq!(scene.view, ViewKind::Structure);

    for event in ["event:OrderPaid", "event:OrderCancelled", "event:Shipped"] {
        assert_eq!(node_shape(&scene, event), Shape::Tag, "{event}");
    }
    assert_eq!(node_shape(&scene, "controller:Fulfillment"), Shape::Hexagon);
    for source in ["external:Customer", "external:PaymentGateway", "external:Clock"] {
        assert_eq!(node_shape(&scene, source), Shape::Rect, "{source}");
    }
    // One hexagon per controller (not per handler), listing its handlers.
    let hexagons: Vec<_> = scene.nodes.iter().filter(|n| n.shape == Shape::Hexagon).collect();
    assert_eq!(hexagons.len(), 1);
    let texts: Vec<&str> = hexagons[0].labels.iter().map(|l| l.text.as_str()).collect();
    assert_eq!(texts, ["Fulfillment", "on OrderPaid"]);

    // Every wiring node sits in a neutral gutter lane, outside every
    // machine lane.
    let gutters: Vec<_> = scene.lanes.iter().filter(|l| l.target == HitTarget::None).collect();
    let machines: Vec<_> = scene
        .lanes
        .iter()
        .filter(|l| matches!(&l.target, HitTarget::Element(ElementKey::Machine { .. })))
        .map(|l| l.rect)
        .collect();
    for n in scene.nodes.iter().filter(|n| matches!(n.shape, Shape::Tag | Shape::Hexagon | Shape::Rect)) {
        assert!(gutters.iter().any(|g| inside(g.rect, n.rect)), "{:?} in a gutter", n.target);
        assert!(machines.iter().all(|m| !m.intersects(&n.rect)), "{:?} outside the lanes", n.target);
    }
    for gutter in &gutters {
        assert_eq!(gutter.title.color, bench.theme.text_muted, "gutters are neutral");
        assert!(machines.iter().all(|m| !m.intersects(&gutter.rect)), "gutters lie between lanes");
    }
    // Next to what they wire: OrderPaid and Fulfillment between the Order
    // lane (which emits it) and the Shipment lane (which it fires into),
    // the sources above the Order lane they trigger, Shipped below the
    // Shipment lane.
    let order = lane_rect_of(&scene, "machine:Order");
    let shipment = lane_rect_of(&scene, "machine:Shipment");
    for key in ["event:OrderPaid", "controller:Fulfillment"] {
        let r = node_rect(&scene, key);
        assert!(r.top() >= order.bottom() && r.bottom() <= shipment.top(), "{key} between Order and Shipment");
    }
    for key in ["external:Customer", "external:PaymentGateway", "external:Clock"] {
        assert!(node_rect(&scene, key).bottom() <= order.top(), "{key} above Order");
    }
    assert!(node_rect(&scene, "event:Shipped").top() >= shipment.bottom(), "Shipped below Shipment");
}

fn lane_rect_of(scene: &Scene, key: &str) -> cascade_layout::Rect {
    let t = target(key);
    scene.lanes.iter().find(|l| l.target == t).map(|l| l.rect).unwrap_or_else(|| panic!("no lane {key}"))
}

#[test]
fn wiring_edges_replace_the_direct_links() {
    let bench = Bench::new(SPEC_EXAMPLE);
    let scene = bench.edit();
    let theme = &bench.theme;

    let emit = edge_between(&scene, EdgeKind::Emit, "transition:Order:pending->paid@capture_ok", "event:OrderPaid");
    assert!(matches!(emit.stroke.dash, Dash::Dashed { .. }));
    assert_eq!(emit.stroke.color, theme.neutral, "emit: dashed gray");
    assert_eq!(emit.target, target("transition:Order:pending->paid@capture_ok"));

    let subscribe = edge_between(&scene, EdgeKind::Subscribe, "event:OrderPaid", "controller:Fulfillment");
    assert_eq!(subscribe.stroke.dash, Dash::Solid);
    assert_eq!(subscribe.stroke.color, theme.neutral);
    assert_eq!(subscribe.target, target("handler:Fulfillment/OrderPaid"));

    let fire =
        edge_between(&scene, EdgeKind::Fire, "controller:Fulfillment", "transition:Shipment:idle->picking@start");
    assert!(matches!(fire.stroke.dash, Dash::Dashed { .. }));
    assert_eq!(fire.stroke.color, OKABE_GREEN, "fire: dashed in the target hue");
    assert_eq!(fire.label.as_ref().map(|l| l.text.as_str()), Some("by orderId"));
    assert_eq!(fire.target, target("rule:Fulfillment/OrderPaid#0"));

    let trigger =
        edge_between(&scene, EdgeKind::Trigger, "external:Customer", "transition:Order:draft->pending@submit");
    assert_eq!(trigger.stroke.dash, Dash::Solid);
    assert_eq!(trigger.stroke.color, theme.external);
    assert_eq!(trigger.target, target("trigger:Order.submit"));

    // No more pill-to-pill links, and every wiring edge counts once.
    assert_eq!(scene.edges.iter().filter(|e| e.kind == EdgeKind::Fire).count(), 1);
    assert_eq!(scene.edges.iter().filter(|e| e.kind == EdgeKind::Emit).count(), 3);
    assert_eq!(scene.edges.iter().filter(|e| e.kind == EdgeKind::Subscribe).count(), 1);
    assert_eq!(scene.edges.iter().filter(|e| e.kind == EdgeKind::Trigger).count(), 3);
    assert_eq!(scene.edges.iter().filter(|e| e.kind == EdgeKind::Transition).count(), 10);
    for e in &scene.edges {
        assert!(e.points.len() >= 2);
    }
}

#[test]
fn fire_labels_show_selectors_and_conditions_only_when_present() {
    let chain = Bench::new(CHAIN).edit();
    for fire in chain.edges.iter().filter(|e| e.kind == EdgeKind::Fire) {
        assert_eq!(fire.label, None, "no selector, no condition: no label");
    }
    let shop = Bench::new(SHOP).edit();
    let labels: Vec<String> = shop
        .edges
        .iter()
        .filter(|e| e.kind == EdgeKind::Fire)
        .filter_map(|e| e.label.as_ref().map(|l| l.text.clone()))
        .collect();
    assert!(labels.contains(&"by orderId [risk score above threshold]".to_owned()), "{labels:?}");
    assert!(labels.contains(&"new with orderId".to_owned()), "{labels:?}");
}

#[test]
fn fires_from_one_controller_into_one_pill_merge() {
    // Tracking fires `poll_tracking` from two handlers: one edge, both rules.
    let scene = Bench::new(SHOP).edit();
    let fires: Vec<_> = scene
        .edges
        .iter()
        .filter(|e| e.kind == EdgeKind::Fire)
        .filter(|e| {
            e.points.last().is_some_and(|p| {
                touches(node_rect(&scene, "transition:Shipment:in_transit->in_transit@poll_tracking"), *p)
            })
        })
        .collect();
    assert_eq!(fires.len(), 1);
    assert_eq!(fires[0].target, target("rule:Tracking/ShipmentDispatched#0"));
    let label = fires[0].label.as_ref().map(|l| l.text.clone()).unwrap_or_default();
    assert_eq!(label, "by orderId [parcel not yet delivered]", "each selector and condition once");
    // Every event appears once, including ones nobody emits or handles.
    for event in ["event:ShipmentLost", "event:ReturnRequested", "event:TrackingPolled"] {
        assert_eq!(node_shape(&scene, event), Shape::Tag);
    }
    let tags = scene.nodes.iter().filter(|n| n.shape == Shape::Tag).count();
    assert_eq!(tags, 13);
    assert_eq!(scene.nodes.iter().filter(|n| n.shape == Shape::Hexagon).count(), 10);
}

#[test]
fn every_connectable_element_gets_one_handle_on_its_east_edge() {
    let bench = Bench::new(SPEC_EXAMPLE);
    let scene = bench.edit();
    let handles = handles(&scene);
    let mut expected = Vec::new();
    for n in &scene.nodes {
        if let HitTarget::Element(key) = &n.target {
            let connectable = matches!(
                key,
                ElementKey::State { .. }
                    | ElementKey::Transition { .. }
                    | ElementKey::Controller { .. }
                    | ElementKey::External { .. }
            );
            let handle = handles.get(&key.to_string());
            assert_eq!(handle.is_some(), connectable, "{key}");
            if let Some(h) = handle {
                expected.push(key.to_string());
                let c = h.center();
                assert!((c.x - n.rect.right()).abs() < 0.01, "{key}: on the east edge");
                assert!(c.y > n.rect.top() && c.y < n.rect.bottom(), "{key}");
                assert!(h.size.width <= 12.0 && h.size.width >= 6.0, "{key}: small");
                assert_eq!(scene.hit_test(c, 1.0), Some(&HitTarget::ConnectHandle { element: key.clone() }));
            }
        }
    }
    // 4 + 3 states, 5 pills, 1 controller, 3 sources.
    assert_eq!(expected.len(), 16);
    assert_eq!(handles.len(), 16);
    for o in &scene.overlays {
        if let Overlay::Rect { target: HitTarget::ConnectHandle { .. }, radius, rect, fill, .. } = o {
            assert!((radius * 2.0 - rect.size.width).abs() < 0.01, "drawn as a circle");
            assert_eq!(*fill, Some(bench.theme.background));
        }
    }
    // View mode has no handles and no band.
    let view = bench.scene(ViewKind::Structure, SceneMode::View, None);
    assert!(handles_of(&view).is_empty());
    assert!(view.nodes.iter().all(|n| n.shape != Shape::Tag));
}

fn handles_of(scene: &Scene) -> Vec<&Overlay> {
    scene
        .overlays
        .iter()
        .filter(|o| matches!(o, Overlay::Rect { target: HitTarget::ConnectHandle { .. }, .. }))
        .collect()
}

#[test]
fn other_views_ignore_edit_mode() {
    let bench = Bench::new(SPEC_EXAMPLE);
    for view in [ViewKind::Causal, ViewKind::Matrix] {
        assert_eq!(bench.scene(view, SceneMode::Edit, None), bench.scene(view, SceneMode::View, None), "{view}");
    }
}

#[test]
fn an_empty_definition_says_how_to_start() {
    let bench = Bench::new("machines: {}\n");
    let scene = bench.edit();
    assert!(scene.nodes.is_empty());
    assert!(scene.notes.iter().any(|n| n.contains("add a machine")), "{:?}", scene.notes);
    let texts = texts(&scene);
    assert!(texts.iter().any(|(t, _)| t.contains("add a machine")), "{texts:?}");
    assert!(scene.bounds.size.width > 0.0 && scene.bounds.size.height > 0.0, "the note is in bounds");
    to_svg(&scene).unwrap_or_else(|err| panic!("{err}"));
    // View mode keeps its old behaviour.
    let view = bench.scene(ViewKind::Structure, SceneMode::View, None);
    assert!(view.overlays.is_empty());
}

#[test]
fn a_machine_without_transitions_still_gets_a_lane_and_a_hint() {
    let bench = Bench::new(
        "machines:\n  Door:\n    states: [closed]\n  Lamp:\n    states: [off, on]\n    transitions:\n      - { from: off, to: on, on: flip }\n",
    );
    let scene = bench.edit();
    let door = scene.lanes.iter().find(|l| l.target == target("machine:Door")).expect("Door lane");
    assert_eq!(door.title.text, "Door");
    assert!(inside(door.rect, node_rect(&scene, "state:Door:closed")));
    assert!(handles(&scene).contains_key("state:Door:closed"));
    let hints: Vec<_> = texts(&scene).into_iter().filter(|(t, _)| t.contains("no transitions")).collect();
    assert_eq!(hints.len(), 1, "only the empty machine gets the hint");
    let (_, at) = &hints[0];
    assert!(at.y >= door.rect.top() && at.y <= door.rect.top() + 30.0, "in the lane's header row");
    assert!(at.x > door.title.origin.x, "after the title");
    // No band without events, controllers or sources.
    assert!(scene.lanes.iter().all(|l| l.target != HitTarget::None));
}

#[test]
fn hidden_machines_keep_their_wiring_on_a_stub() {
    let bench = Bench::new(SPEC_EXAMPLE);
    let state = ViewState { hidden_machines: ["Order".to_owned()].into_iter().collect(), ..structure() };
    let scene = bench.build_with(&mut SceneBuilder::new(), &state, SceneMode::Edit, None);
    let stub = scene
        .nodes
        .iter()
        .find(|n| matches!(&n.target, HitTarget::MachineStub { machine, .. } if machine == "Order"))
        .expect("stub");
    // Emits to OrderPaid and OrderCancelled, triggers from three sources.
    assert_eq!(stub.target, HitTarget::MachineStub { machine: "Order".into(), links: 5 });
    let stub_links: Vec<_> = scene.edges.iter().filter(|e| e.kind == EdgeKind::StubLink).collect();
    assert_eq!(stub_links.len(), 5);
    for e in &stub_links {
        assert_eq!(e.stroke.dash, Dash::Dotted);
    }
    assert!(!handles(&scene).keys().any(|k| k.contains(":Order:")), "no handles on hidden states");
}

#[test]
fn collapsed_states_take_the_wiring_of_their_transitions() {
    let bench = Bench::new(SHOP);
    let state = ViewState { collapsed: [k("state:Order:placed")].into_iter().collect(), ..structure() };
    let scene = bench.build_with(&mut SceneBuilder::new(), &state, SceneMode::Edit, None);
    // `placed.awaiting_payment → placed.paid` is inside the collapsed state:
    // its emit of OrderPaid and the fire into it attach to `placed`.
    edge_between(&scene, EdgeKind::Emit, "state:Order:placed", "event:OrderPaid");
    edge_between(&scene, EdgeKind::Fire, "controller:Orders", "state:Order:placed");
    assert!(handles(&scene).contains_key("state:Order:placed"));
}

#[test]
fn edit_mode_exports_without_handles() {
    let scene = Bench::new(SPEC_EXAMPLE).edit();
    let svg = to_svg(&scene).unwrap_or_else(|err| panic!("{err}"));
    let doc = roxmltree::Document::parse(&svg).unwrap_or_else(|err| panic!("{err}"));
    let overlays = doc.descendants().filter(|n| n.attribute("class") == Some("overlay")).count();
    let handle_count = handles_of(&scene).len();
    assert!(handle_count > 0);
    assert_eq!(overlays, scene.overlays.len() - handle_count, "handles are left out of exports");
    for text in ["OrderPaid", "Customer", "on OrderPaid", "by orderId"] {
        assert!(svg.contains(text), "{text}");
    }
}

#[test]
fn edit_mode_builds_are_deterministic() {
    let bench = Bench::new(SHOP);
    assert_eq!(bench.edit(), bench.edit());
}
