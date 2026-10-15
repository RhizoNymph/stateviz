//! The causal view and the matrix with subscriptions and timers: `Notify`
//! edges (event → subscribed transition) and `Delay` edges (transition →
//! timer transition) are drawn distinctly from subscribe, fire and trigger
//! links, focus follows them, causal lanes keep every arrow pointing right,
//! and the matrix counts subscriptions as links.

mod build_play;
mod semantics;

use build_play::Bench;
use cascade_core::Direction;
use cascade_scene::metrics::{leftward_edges, measure};
use cascade_scene::{
    Arrow, ConeFocus, Dash, EdgeKind, Emphasis, HitTarget, Scene, SceneBuilder, SceneMode, ViewKind, ViewState,
    machine_styles,
};
use semantics::*;

fn causal() -> ViewState {
    ViewState { view: ViewKind::Causal, ..ViewState::default() }
}

fn lanes() -> ViewState {
    ViewState { group_by_machine: true, ..causal() }
}

fn build(bench: &Bench, state: &ViewState) -> Scene {
    bench.build_with(&mut SceneBuilder::new(), state, SceneMode::View, None)
}

fn with_selection(state: ViewState, keys: &[&str]) -> ViewState {
    ViewState { selection: keys.iter().map(|k| k.parse().expect("key")).collect(), ..state }
}

fn hue(bench: &Bench, machine: &str) -> cascade_scene::Rgba {
    let m = bench.model.machine_by_name(machine).expect("machine");
    machine_styles(&bench.model, &bench.theme)[m.index()].hue
}

#[test]
fn notify_edges_run_from_the_event_into_the_subscribed_pill() {
    let bench = Bench::new(SAGA);
    let scene = build(&bench, &causal());
    let notify = edges_of(&scene, EdgeKind::Notify);
    assert_eq!(notify.len(), 3);
    let placed: Vec<_> = notify.iter().filter(|e| e.target == target("trigger:Payment.OrderPlaced")).collect();
    assert_eq!(placed.len(), 1);
    let edge = placed[0];
    let (event, pill) = (node(&scene, "event:OrderPlaced").rect, node(&scene, CHARGING).rect);
    let near =
        |r: cascade_layout::Rect, p: cascade_layout::Point| r.outset(cascade_layout::Insets::uniform(0.5)).contains(p);
    assert!(near(event, edge.points[0]));
    let end = *edge.points.last().expect("points");
    assert!(near(pill, end) && (end.x - pill.left()).abs() < 0.5, "into the pill's input (West) side");
    assert_eq!(edge.stroke.color, hue(&bench, "Payment"));
    assert_eq!(edge.stroke.dash, Dash::Solid);
    assert_eq!(edge.arrow, Arrow::End);
    // Distinct from subscribe (solid gray) and fire (dashed hue).
    let subscribe = edges_of(&scene, EdgeKind::Subscribe);
    assert_eq!(subscribe.len(), 1, "OrderConfirmed → Mailer");
    assert_ne!(subscribe[0].stroke.color, edge.stroke.color);
    let fire = edges_of(&scene, EdgeKind::Fire);
    assert_eq!(fire.len(), 1);
    assert_ne!(fire[0].stroke.dash, edge.stroke.dash);
    // The pill of a subscription says so.
    assert_eq!(label_texts(node(&scene, CONFIRMED))[1..], ["on PaymentCaptured", "where orderId == event.orderId"]);
}

#[test]
fn delay_edges_are_dashed_and_carry_the_clock() {
    let bench = Bench::new(SAGA);
    let scene = build(&bench, &causal());
    let delay = edges_of(&scene, EdgeKind::Delay);
    // checkout arms the 30m timer; entering charging (by subscription or by
    // the retry itself) arms the 10s one.
    assert_eq!(delay.len(), 3, "{delay:#?}");
    for edge in &delay {
        assert!(matches!(edge.stroke.dash, Dash::Dashed { .. }), "{edge:?}");
        let text = &label_of(edge).text;
        assert!(text.starts_with("◷ "), "{text}");
    }
    let expiry: Vec<_> = delay.iter().filter(|e| e.target == target("trigger:Order.after-30m")).collect();
    assert_eq!(expiry.len(), 1);
    assert_eq!(label_of(expiry[0]).text, "◷ 30m");
    assert_eq!(expiry[0].stroke.color, hue(&bench, "Order"), "in the machine's hue");
    let (from, to) = (node(&scene, CHECKOUT).rect, node(&scene, EXPIRED).rect);
    let (first, last) = (expiry[0].points[0], *expiry[0].points.last().expect("points"));
    assert!((first.x - from.right()).abs() < 0.5, "out of the arming pill's East side");
    assert!((last.x - to.left()).abs() < 0.5, "into the timer pill's West side");
    let retries: Vec<_> = delay.iter().filter(|e| e.target == target("trigger:Payment.after-10s")).collect();
    assert_eq!(retries.len(), 2);
    assert_eq!(label_of(retries[0]).text, "◷ 10s [self.method == card]", "with the timer's guard");
    // A timer that re-arms itself is a loop: drawn red as a cycle.
    let self_loop: Vec<_> = retries.iter().filter(|e| e.points.len() > 2 && e.back_edge).collect();
    assert_eq!(self_loop.len(), 1);
    assert_eq!(self_loop[0].stroke.color, bench.theme.finding);
    // Timer pills read their delay.
    assert_eq!(label_texts(node(&scene, EXPIRED))[1], "◷ 30m");
}

#[test]
fn lanes_keep_every_arrow_pointing_right() {
    let bench = Bench::new(SAGA);
    let scene = build(&bench, &lanes());
    assert_eq!(leftward_edges(&scene), 0, "{:#?}", scene.edges.iter().filter(|e| !e.back_edge).collect::<Vec<_>>());
    assert_eq!(edges_of(&scene, EdgeKind::Delay).len(), 3);
    assert_eq!(edges_of(&scene, EdgeKind::Notify).len(), 3);
    let m = measure(&scene);
    assert_eq!(m.label_overlaps, 0, "{m}");
    assert_eq!(m.edges_through_nodes, 0, "{m}");
    // Hiding a machine keeps the guarantee; its timers stay hidden inside
    // its stub (no stub-to-itself link).
    let hidden = ViewState { hidden_machines: ["Payment".to_owned()].into_iter().collect(), ..lanes() };
    let scene = build(&bench, &hidden);
    assert_eq!(leftward_edges(&scene), 0);
    // Bank's settle in, OrderPlaced in, PaymentCaptured and PaymentFailed out.
    assert_eq!(edges_of(&scene, EdgeKind::StubLink).len(), 4);
    assert!(scene.nodes.iter().any(|n| n.target == HitTarget::MachineStub { machine: "Payment".into(), links: 4 }));
}

#[test]
fn cones_follow_subscriptions_and_timers() {
    let bench = Bench::new(SAGA);
    let cone = Some(ConeFocus { direction: Direction::Forward, depth: None });
    let state = ViewState { cone, ..with_selection(causal(), &[CHECKOUT]) };
    let scene = build(&bench, &state);
    assert_eq!(node(&scene, CHECKOUT).emphasis, Emphasis::Selected);
    for key in ["event:OrderPlaced", CHARGING, EXPIRED, RETRY] {
        assert_eq!(node(&scene, key).emphasis, Emphasis::Focused, "{key} follows checkout");
    }
    for key in [CAPTURED, SEND] {
        assert_eq!(node(&scene, key).emphasis, Emphasis::Dimmed, "{key} needs the bank");
    }
    let notify = edges_of(&scene, EdgeKind::Notify);
    let into_payment = notify.iter().find(|e| e.target == target("trigger:Payment.OrderPlaced")).expect("notify");
    assert_ne!(into_payment.emphasis, Emphasis::Dimmed);
}

#[test]
fn path_queries_run_through_subscriptions() {
    let bench = Bench::new(SAGA);
    let scene = build(&bench, &with_selection(causal(), &[CAPTURED, SEND]));
    assert!(scene.notes.is_empty(), "{:?}", scene.notes);
    for key in ["event:PaymentCaptured", CONFIRMED, "event:OrderConfirmed", "handler:Mailer/OrderConfirmed"] {
        assert_eq!(node(&scene, key).emphasis, Emphasis::Focused, "{key} is on the path");
    }
    assert_eq!(node(&scene, CHECKOUT).emphasis, Emphasis::Dimmed);
    // Within one machine, a timer is a path.
    let scene = build(&bench, &with_selection(causal(), &[CHECKOUT, EXPIRED]));
    assert!(scene.notes.is_empty(), "{:?}", scene.notes);
    // Without one, the note says how a machine's transitions can cause each
    // other.
    let scene = build(&bench, &with_selection(causal(), &[ORDER_FAILED, CONFIRMED]));
    assert_eq!(scene.notes.len(), 1);
    assert!(scene.notes[0].contains("timer"), "{}", scene.notes[0]);
    assert!(scene.notes[0].contains("subscribes"), "{}", scene.notes[0]);
}

#[test]
fn the_matrix_counts_subscriptions_as_links() {
    let bench = Bench::new(SAGA);
    let scene = build(&bench, &ViewState { view: ViewKind::Matrix, ..ViewState::default() });
    let cell = |row: &str, column: &str| {
        scene
            .nodes
            .iter()
            .find_map(|n| match &n.target {
                HitTarget::MatrixCell { row: r, column: c, count } if r == row && c == column => Some(*count),
                _ => None,
            })
            .unwrap_or_else(|| panic!("no cell {row} × {column}"))
    };
    assert_eq!(cell("Order", "Payment"), 1, "OrderPlaced, by subscription");
    assert_eq!(cell("Payment", "Order"), 2, "PaymentCaptured and PaymentFailed");
    assert_eq!(cell("Order", "Receipt"), 1, "OrderConfirmed through Mailer");
    assert_eq!(cell("Payment", "Payment"), 0, "timers are not links");
    assert_eq!(cell("Order", "Order"), 0);
}
