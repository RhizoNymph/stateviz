//! The build canvas with subscriptions and timers: `Notify` wires run from
//! the event tag to the subscribing transition, events sit between their
//! emitters and subscribers, and every event tag carries a subscribe
//! handle the app drags from to subscribe a transition to the event.

mod build_play;
mod semantics;

use build_play::{Bench, handles, inside, touches};
use cascade_core::ElementKey;
use cascade_layout::Rect;
use cascade_scene::scene::polyline_distance;
use cascade_scene::{
    Dash, EdgeKind, HitTarget, Overlay, Scene, SceneBuilder, SceneEdge, SceneMode, Shape, ViewKind, ViewState,
    machine_styles, to_svg,
};
use semantics::*;

fn structure() -> ViewState {
    ViewState { view: ViewKind::Structure, ..ViewState::default() }
}

fn arrows() -> ViewState {
    ViewState { transition_pills: false, ..structure() }
}

fn edit(state: &ViewState) -> Scene {
    Bench::new(SAGA).build_with(&mut SceneBuilder::new(), state, SceneMode::Edit, None)
}

fn lane(scene: &Scene, machine: &str) -> Rect {
    let t = target(&format!("machine:{machine}"));
    scene.lanes.iter().find(|l| l.target == t).map(|l| l.rect).unwrap_or_else(|| panic!("no lane {machine}"))
}

fn notify_into<'a>(scene: &'a Scene, trigger: &str) -> Vec<&'a SceneEdge> {
    edges_of(scene, EdgeKind::Notify).into_iter().filter(|e| e.target == target(trigger)).collect()
}

/// Subscribe handles: event name → handle rect.
fn subscribe_handles(scene: &Scene) -> Vec<(String, Rect)> {
    scene
        .overlays
        .iter()
        .filter_map(|o| match o {
            Overlay::Rect { rect, target: HitTarget::SubscribeHandle { event }, .. } => Some((event.clone(), *rect)),
            _ => None,
        })
        .collect()
}

#[test]
fn notify_wires_run_from_the_event_to_the_subscribing_pill() {
    let bench = Bench::new(SAGA);
    let scene = edit(&structure());
    let payment_hue = machine_styles(&bench.model, &bench.theme)[1].hue;
    let wires = notify_into(&scene, "trigger:Payment.OrderPlaced");
    assert_eq!(wires.len(), 1);
    let wire = wires[0];
    let (tag, pill) = (node(&scene, "event:OrderPlaced").rect, node(&scene, CHARGING).rect);
    assert!(touches(tag, wire.points[0]), "from the tag");
    let end = *wire.points.last().expect("points");
    assert!(touches(pill, end), "to the pill");
    assert!((end.y - pill.top()).abs() < 0.5 || (end.y - pill.bottom()).abs() < 0.5, "the pill's end faces the gutter");
    assert_eq!(wire.stroke.color, payment_hue);
    assert_eq!(wire.stroke.dash, Dash::Solid);
    assert!(wire.label.is_none(), "the pill already says `on OrderPlaced`");
    assert_eq!(notify_into(&scene, "trigger:Order.PaymentCaptured").len(), 1);
    assert_eq!(notify_into(&scene, "trigger:Order.PaymentFailed").len(), 1);
    assert_eq!(edges_of(&scene, EdgeKind::Notify).len(), 3);
    // Controller wiring is unchanged next to it.
    assert_eq!(edges_of(&scene, EdgeKind::Subscribe).len(), 1);
    assert_eq!(edges_of(&scene, EdgeKind::Fire).len(), 1);
    // Timers need no wire.
    assert!(edges_of(&scene, EdgeKind::Delay).is_empty());
}

#[test]
fn events_sit_between_their_emitters_and_their_subscribers() {
    let scene = edit(&structure());
    let (order, payment) = (lane(&scene, "Order"), lane(&scene, "Payment"));
    for event in ["event:OrderPlaced", "event:PaymentCaptured", "event:PaymentFailed"] {
        let r = node(&scene, event).rect;
        assert!(r.top() >= order.bottom() - 0.01 && r.bottom() <= payment.top() + 0.01, "{event} between the lanes");
    }
}

#[test]
fn every_event_tag_gets_a_subscribe_handle() {
    let scene = edit(&structure());
    let subscribe = subscribe_handles(&scene);
    let tags: Vec<_> = scene.nodes.iter().filter(|n| n.shape == Shape::Tag).collect();
    assert_eq!(subscribe.len(), tags.len());
    assert_eq!(tags.len(), 4);
    for tag in tags {
        let HitTarget::Element(ElementKey::Event { event }) = &tag.target else { panic!("{:?}", tag.target) };
        let rect = subscribe.iter().find(|(e, _)| e == event).map(|(_, r)| *r).expect("a handle");
        let c = rect.center();
        assert!((c.x - tag.rect.right()).abs() < 0.01, "{event}: on the tag's point");
        assert!((c.y - tag.rect.center().y).abs() < 0.01, "{event}");
        assert!(rect.size.width <= 12.0 && rect.size.width >= 6.0, "small");
        let handle = HitTarget::SubscribeHandle { event: event.clone() };
        assert_eq!(scene.hit_test(c, 1.0), Some(&handle), "wins over the tag");
        assert_eq!(scene.hit_test_arrows(c, 1.0, 8.0), Some(handle));
        // Inside the tag, away from the handle, the tag itself.
        let inner = cascade_layout::Point::new(tag.rect.left() + 4.0, tag.rect.center().y);
        assert_eq!(scene.hit_test(inner, 1.0), Some(&tag.target));
    }
    // Connect handles are as before: none on events.
    assert!(handles(&scene).keys().all(|k| !k.starts_with("event:")));
    // View mode has none.
    let view = Bench::new(SAGA).build_with(&mut SceneBuilder::new(), &structure(), SceneMode::View, None);
    assert!(subscribe_handles(&view).is_empty());
}

#[test]
fn subscribe_handles_stay_in_exports() {
    // Connect handles are left out of exports; subscribe handles stay, a
    // small circle on each tag's point marking where subscriptions leave.
    let scene = edit(&structure());
    let mut without = scene.clone();
    without.overlays.retain(|o| !matches!(o, Overlay::Rect { target: HitTarget::SubscribeHandle { .. }, .. }));
    assert_eq!(scene.overlays.len() - without.overlays.len(), 4);
    let (with_svg, without_svg) = (to_svg(&scene).expect("svg"), to_svg(&without).expect("svg"));
    assert_eq!(with_svg.matches(r#"class="overlay""#).count(), without_svg.matches(r#"class="overlay""#).count() + 4);
}

#[test]
fn notify_wires_end_on_the_arrow_in_arrow_mode() {
    let scene = edit(&arrows());
    assert!(subscribe_handles(&scene).len() == 4, "handles in arrow mode too");
    let arrow: Vec<_> =
        scene.edges.iter().filter(|e| e.kind == EdgeKind::Transition && e.target == target(CHARGING)).collect();
    assert_eq!(arrow.len(), 1);
    let wire = notify_into(&scene, "trigger:Payment.OrderPlaced");
    assert_eq!(wire.len(), 1);
    let end = *wire[0].points.last().expect("points");
    assert!(polyline_distance(&arrow[0].points, end) < 0.5, "snapped onto the arrow");
    let tag = node(&scene, "event:OrderPlaced").rect;
    assert!(touches(tag, wire[0].points[0]));
    // Wiring nodes still sit in gutters.
    let gutters: Vec<Rect> = scene.lanes.iter().filter(|l| l.target == HitTarget::None).map(|l| l.rect).collect();
    assert!(gutters.iter().any(|g| inside(*g, tag)));
}
