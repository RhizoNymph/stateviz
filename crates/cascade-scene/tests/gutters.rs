//! The build canvas's gutters: wiring sits between the lanes it connects,
//! rows run in pill order, pill ends face their gutter, fire labels are
//! short and said once, and gutter columns survive edits.

mod build_play;
mod common;

use build_play::*;
use cascade_layout::Rect;
use cascade_scene::{EdgeKind, HitTarget, Scene, SceneBuilder, SceneEdge, SceneMode};
use common::SPEC_EXAMPLE;

fn lane(scene: &Scene, key: &str) -> Rect {
    let t = target(key);
    scene.lanes.iter().find(|l| l.target == t).map(|l| l.rect).unwrap_or_else(|| panic!("no lane {key}"))
}

fn gutters(scene: &Scene) -> Vec<Rect> {
    let mut out: Vec<Rect> = scene.lanes.iter().filter(|l| l.target == HitTarget::None).map(|l| l.rect).collect();
    out.sort_by(|a, b| a.top().total_cmp(&b.top()));
    out
}

fn gutter_of(scene: &Scene, key: &str) -> usize {
    let r = node_rect(scene, key);
    gutters(scene).iter().position(|g| inside(*g, r)).unwrap_or_else(|| panic!("{key} is in no gutter"))
}

/// Edges of `kind` whose first point touches `from`'s node.
fn edges_from<'a>(scene: &'a Scene, kind: EdgeKind, from: &str) -> Vec<&'a SceneEdge> {
    let r = node_rect(scene, from);
    scene.edges.iter().filter(|e| e.kind == kind && touches(r, e.points[0])).collect()
}

fn edit_build(builder: &mut SceneBuilder, yaml: &str) -> Scene {
    Bench::new(yaml).build_with(builder, &structure(), SceneMode::Edit, None)
}

#[test]
fn gutters_sit_between_lanes_and_only_where_needed() {
    let scene = Bench::new(SPEC_EXAMPLE).edit();
    let order = lane(&scene, "machine:Order");
    let shipment = lane(&scene, "machine:Shipment");
    let g = gutters(&scene);
    // Above Order (the sources), between the lanes, below Shipment.
    assert_eq!(g.len(), 3, "{g:?}");
    assert!(g[0].bottom() <= order.top());
    assert!(g[1].top() >= order.bottom() && g[1].bottom() <= shipment.top());
    assert!(g[2].top() >= shipment.bottom());
    for r in &g {
        assert!(r.size.height < order.size.height, "gutters are thin: {r:?}");
    }
}

#[test]
fn shop_wiring_sits_next_to_what_it_wires() {
    let scene = Bench::new(SHOP).edit();
    let payment = lane(&scene, "machine:Payment");
    // PaymentAuthorized is emitted in Payment and handled by Billing and
    // FraudCheck, which fire back into Payment: all three share a gutter
    // bordering the Payment lane.
    let g = gutter_of(&scene, "event:PaymentAuthorized");
    assert_eq!(gutter_of(&scene, "controller:Billing"), g);
    assert_eq!(gutter_of(&scene, "controller:FraudCheck"), g);
    let r = gutters(&scene)[g];
    assert!(r.bottom() <= payment.top() || r.top() >= payment.bottom());
    // Sources sit right above the lane they trigger.
    for (source, machine) in [
        ("external:Customer", "machine:Order"),
        ("external:PaymentGateway", "machine:Payment"),
        ("external:Warehouse", "machine:Inventory"),
        ("external:Carrier", "machine:Shipment"),
        ("external:MailProvider", "machine:Notification"),
    ] {
        let s = node_rect(&scene, source);
        let l = lane(&scene, machine);
        assert!(s.bottom() <= l.top(), "{source} above {machine}");
        let between = scene
            .lanes
            .iter()
            .filter(|x| x.target != HitTarget::None && x.rect.top() >= s.bottom() && x.rect.bottom() <= l.top())
            .count();
        assert_eq!(between, 0, "{source}: no lane between it and {machine}");
    }
}

#[test]
fn rows_follow_the_pills_they_wire() {
    // Customer triggers `submit` (first pill), PaymentGateway and Clock
    // triggers further right.
    let scene = Bench::new(SPEC_EXAMPLE).edit();
    let x = |k: &str| node_rect(&scene, k).left();
    assert!(x("external:Customer") < x("external:PaymentGateway"));
    assert!(x("external:PaymentGateway") < x("external:Clock"), "ties keep definition order");
    // A controller sits right after the event it handles.
    assert!(x("event:OrderPaid") < x("controller:Fulfillment"));
    assert!(x("controller:Fulfillment") < x("event:OrderCancelled"), "next to its event, not after others");
}

#[test]
fn subscriptions_inside_a_gutter_point_right() {
    let scene = Bench::new(SHOP).edit();
    for e in scene.edges.iter().filter(|e| e.kind == EdgeKind::Subscribe) {
        let (a, b) = (e.points[0], e.points[e.points.len() - 1]);
        if (a.y - b.y).abs() < 40.0 {
            assert!(a.x < b.x, "{:?} runs left", e.target);
        }
    }
}

#[test]
fn pill_ends_face_their_gutter() {
    let scene = Bench::new(SPEC_EXAMPLE).edit();
    // Clock (above Order) fires `timeout`: the trigger enters the pill's
    // top edge. OrderPaid (below Order) takes the emit from the bottom.
    let pill = node_rect(&scene, "transition:Order:pending->cancelled@timeout");
    let trigger = &edges_from(&scene, EdgeKind::Trigger, "external:Clock")[0];
    let end = trigger.points[trigger.points.len() - 1];
    assert!((end.y - pill.top()).abs() < 0.5, "enters from above");
    let pill = node_rect(&scene, "transition:Order:pending->paid@capture_ok");
    let emit = &edges_from(&scene, EdgeKind::Emit, "transition:Order:pending->paid@capture_ok")[0];
    assert!((emit.points[0].y - pill.bottom()).abs() < 0.5, "leaves downwards");
    // Fulfillment (above Shipment) fires down into `start`.
    let pill = node_rect(&scene, "transition:Shipment:idle->picking@start");
    let fire = &edges_from(&scene, EdgeKind::Fire, "controller:Fulfillment")[0];
    let end = fire.points[fire.points.len() - 1];
    assert!((end.y - pill.top()).abs() < 0.5, "enters from above");
}

#[test]
fn fire_labels_are_short_and_said_once() {
    let scene = Bench::new(SHOP).edit();
    let labels = |controller: &str| -> Vec<Option<String>> {
        edges_from(&scene, EdgeKind::Fire, controller)
            .iter()
            .map(|e| e.label.as_ref().map(|l| l.text.clone()))
            .collect()
    };
    // Orders fires into four Order pills, all selecting by orderId: one
    // label covers the parallel wires.
    let orders = labels("controller:Orders");
    assert_eq!(orders.len(), 4);
    assert_eq!(orders.iter().flatten().collect::<Vec<_>>(), ["by orderId"]);
    assert_eq!(labels("controller:Checkout"), [Some("new with orderId".to_owned())]);
    assert_eq!(labels("controller:FraudCheck"), [Some("by orderId [risk score above threshold]".to_owned())]);
    // Billing fires `capture`, which two Payment transitions accept.
    let billing = labels("controller:Billing");
    assert_eq!(billing.len(), 2);
    assert_eq!(billing.iter().flatten().count(), 1, "{billing:?}");
    // No fire label repeats the long selector anywhere.
    for e in scene.edges.iter().filter_map(|e| e.label.as_ref()) {
        assert!(!e.text.contains("event."), "{}", e.text);
    }
}

#[test]
fn undoing_an_edit_restores_the_picture_from_the_cache() {
    // Shipping also reacts to ShipmentLost, then the edit is undone.
    let edited = SHOP.replacen(
        "  Shipping:\n    on:\n",
        "  Shipping:\n    on:\n      ShipmentLost:\n        - fire: Notification.send\n          target: new Notification with orderId = event.orderId\n",
        1,
    );
    assert_ne!(edited, SHOP);
    let mut builder = SceneBuilder::new();
    let first = edit_build(&mut builder, SHOP);
    let _ = edit_build(&mut builder, &edited);
    let runs = builder.layouts_run();
    assert_eq!(edit_build(&mut builder, SHOP), first, "same columns, same picture");
    assert_eq!(builder.layouts_run(), runs, "served from the cache");
}

#[test]
fn a_node_joining_a_gutter_moves_nothing_already_there() {
    // A new controller handling PaymentAuthorized and firing into Payment
    // joins that event's gutter at the end of the row.
    let edited = SHOP.replacen(
        "\nexternal:\n",
        "  Audit:\n    on:\n      PaymentAuthorized:\n        - fire: Payment.void\n          target: Payment where orderId == event.orderId\n\nexternal:\n",
        1,
    );
    let mut builder = SceneBuilder::new();
    let before = edit_build(&mut builder, SHOP);
    let after = edit_build(&mut builder, &edited);
    let g = gutter_of(&after, "controller:Audit");
    assert_eq!(g, gutter_of(&after, "event:PaymentAuthorized"));
    let row = gutters(&before)[g];
    let (b, a) = (rects(&before), rects(&after));
    for (key, r) in &b {
        if inside(row, *r) {
            assert_eq!(a.get(key), Some(r), "{key} moved");
        }
    }
    let audit = node_rect(&after, "controller:Audit");
    assert!(b.values().filter(|r| inside(row, **r)).all(|r| r.left() < audit.left()), "joins the end");
}

#[test]
fn reset_returns_to_the_tidy_order() {
    let edited = SHOP.replacen(
        "\nexternal:\n",
        "  Audit:\n    on:\n      PaymentAuthorized:\n        - fire: Payment.void\n          target: Payment where orderId == event.orderId\n\nexternal:\n",
        1,
    );
    let mut builder = SceneBuilder::new();
    let _ = edit_build(&mut builder, SHOP);
    let _ = edit_build(&mut builder, &edited);
    builder.reset();
    let fresh = edit_build(&mut SceneBuilder::new(), &edited);
    assert_eq!(edit_build(&mut builder, &edited), fresh);
}
