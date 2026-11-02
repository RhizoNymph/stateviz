//! The structure view with subscriptions, timers, choices and `sets`:
//! pills and arrows say what triggers them, guards read as canonical text
//! with choices in italics, `sets` is shown compactly, and subscription
//! links join the emitting and the subscribing transitions.

mod build_play;
mod semantics;

use build_play::Bench;
use cascade_scene::{
    Arrow, Dash, EdgeKind, FontStyle, HitTarget, MonoMeasure, SceneBuilder, SceneMode, TextMeasure, ViewKind,
    ViewState, machine_styles,
};
use semantics::*;

fn structure() -> ViewState {
    ViewState { view: ViewKind::Structure, ..ViewState::default() }
}

fn arrows() -> ViewState {
    ViewState { transition_pills: false, ..structure() }
}

fn scene(state: &ViewState, mode: SceneMode) -> cascade_scene::Scene {
    Bench::new(SAGA).build_with(&mut SceneBuilder::new(), state, mode, None)
}

fn hue(bench: &Bench, machine: &str) -> cascade_scene::Rgba {
    let m = bench.model.machine_by_name(machine).expect("machine");
    machine_styles(&bench.model, &bench.theme)[m.index()].hue
}

#[test]
fn pills_say_what_triggers_them() {
    let scene = scene(&structure(), SceneMode::View);
    assert_eq!(label_texts(node(&scene, CHECKOUT)), ["cart → placed", "checkout"], "a command by name");
    assert_eq!(
        label_texts(node(&scene, CONFIRMED)),
        ["placed → confirmed", "on PaymentCaptured", "where orderId == event.orderId"],
        "a subscription by its event, the filter underneath"
    );
    assert_eq!(label_texts(node(&scene, CHARGING)), ["idle → charging", "on OrderPlaced"], "no filter, no line");
    assert_eq!(label_texts(node(&scene, EXPIRED)), ["placed → cancelled", "◷ 30m"], "a timer by its delay");
    assert_eq!(label_texts(node(&scene, RETRY)), ["charging → charging", "◷ 10s"]);
    // Every line fits inside its pill, in the same small size as the
    // trigger line.
    let measure = MonoMeasure::default();
    for key in [CHECKOUT, CONFIRMED, CHARGING, EXPIRED, RETRY] {
        let pill = node(&scene, key);
        for label in &pill.labels {
            let w = measure.width(&label.text, label.font_size);
            assert!(label.origin.x >= pill.rect.left() - 0.01, "{key}: {label:?}");
            assert!(label.origin.x + w <= pill.rect.right() + 0.01, "{key}: {label:?}");
            assert!(label.origin.y >= pill.rect.top() - 0.01, "{key}: {label:?}");
            assert!(
                label.origin.y + measure.line_height(label.font_size) <= pill.rect.bottom() + 0.01,
                "{key}: {label:?}"
            );
        }
        assert!(pill.labels[1..].iter().all(|l| l.font_size < pill.labels[0].font_size), "{key}");
    }
}

#[test]
fn guards_are_canonical_and_choices_are_italic() {
    let scene = scene(&structure(), SceneMode::View);
    let guard = |text: &str| {
        let found = labelled(&scene, text);
        assert_eq!(found.len(), 1, "one edge labelled {text}");
        found[0]
    };
    let accepted = guard("[?card accepted]");
    assert_eq!(accepted.target, target(CAPTURED));
    assert_eq!(label_of(accepted).style, FontStyle::Italic, "a choice is a decision, not data");
    assert_eq!(label_of(guard("[else]")).style, FontStyle::Italic, "else is the fallback choice");
    let expr = guard("[self.method == card]");
    assert_eq!(expr.target, target(RETRY));
    assert_eq!(label_of(expr).style, FontStyle::Normal, "an expression reads plainly");
    // Pills and lane titles are never italic.
    assert!(scene.nodes.iter().flat_map(|n| &n.labels).all(|l| l.style == FontStyle::Normal));
}

#[test]
fn sets_label_the_way_out_of_the_pill() {
    let scene = scene(&structure(), SceneMode::View);
    let sets = labelled(&scene, "sets paidBy");
    assert_eq!(sets.len(), 1);
    assert_eq!(sets[0].target, target(CONFIRMED));
    assert_eq!(sets[0].kind, EdgeKind::Transition);
    assert_eq!(sets[0].arrow, Arrow::End, "on the half that enters the target state");
    assert_eq!(labelled(&scene, "sets orderId").len(), 1);
    assert!(labelled(&scene, "sets orderId")[0].target == target(CHARGING));
    // Transitions without `sets` keep an unlabelled way out.
    let checkout_out: Vec<_> =
        scene.edges.iter().filter(|e| e.target == target(CHECKOUT) && e.arrow == Arrow::End).collect();
    assert_eq!(checkout_out.len(), 1);
    assert!(checkout_out[0].label.is_none());
}

#[test]
fn arrows_carry_trigger_guard_filter_and_sets_in_one_label() {
    let scene = scene(&arrows(), SceneMode::View);
    let arrow = |key: &str| {
        let found: Vec<_> =
            scene.edges.iter().filter(|e| e.kind == EdgeKind::Transition && e.target == target(key)).collect();
        assert_eq!(found.len(), 1, "one arrow for {key}");
        found[0]
    };
    let text = |key: &str| label_of(arrow(key)).text.clone();
    assert_eq!(text(CHECKOUT), "checkout");
    assert_eq!(text(CONFIRMED), "on PaymentCaptured where orderId == event.orderId / sets paidBy");
    assert_eq!(text(CHARGING), "on OrderPlaced / sets orderId");
    assert_eq!(text(EXPIRED), "◷ 30m");
    assert_eq!(text(RETRY), "◷ 10s [self.method == card]");
    assert_eq!(text(CAPTURED), "settle [?card accepted]");
    assert_eq!(label_of(arrow(CAPTURED)).style, FontStyle::Italic, "the label carries a choice");
    assert_eq!(label_of(arrow(DECLINED)).style, FontStyle::Italic);
    assert_eq!(label_of(arrow(RETRY)).style, FontStyle::Normal);
    assert_eq!(label_of(arrow(CONFIRMED)).style, FontStyle::Normal);
}

#[test]
fn subscription_links_join_emitter_and_subscriber_across_lanes() {
    let bench = Bench::new(SAGA);
    for state in [structure(), arrows()] {
        let scene = bench.build_with(&mut SceneBuilder::new(), &state, SceneMode::View, None);
        let notify = edges_of(&scene, EdgeKind::Notify);
        // OrderPlaced → Payment, PaymentCaptured and PaymentFailed → Order.
        assert_eq!(notify.len(), 3, "{notify:#?}");
        let placed = labelled(&scene, "OrderPlaced");
        assert_eq!(placed.len(), 1);
        let placed = placed[0];
        assert_eq!(placed.kind, EdgeKind::Notify);
        assert_eq!(placed.target, target("trigger:Payment.OrderPlaced"), "clicking selects the subscription");
        assert_eq!(placed.stroke.color, hue(&bench, "Payment"), "in the subscriber's hue");
        assert_eq!(placed.stroke.dash, Dash::Solid, "solid: no controller in between");
        assert_eq!(placed.arrow, Arrow::End);
        // The controller link stays a fire.
        let fires = edges_of(&scene, EdgeKind::Fire);
        assert_eq!(fires.len(), 1);
        assert_eq!(label_of(fires[0]).text, "OrderConfirmed › Mailer");
        assert!(matches!(fires[0].stroke.dash, Dash::Dashed { .. }));
        // Timers need no link.
        assert!(edges_of(&scene, EdgeKind::Delay).is_empty());
    }
    // With pills, the links run from pill to pill.
    let scene = bench.build_with(&mut SceneBuilder::new(), &structure(), SceneMode::View, None);
    let placed = labelled(&scene, "OrderPlaced")[0];
    let (from, to) = (node(&scene, CHECKOUT).rect, node(&scene, CHARGING).rect);
    let (first, last) = (placed.points[0], *placed.points.last().expect("points"));
    assert!(from.outset(cascade_layout::Insets::uniform(0.5)).contains(first), "{first:?} on {from:?}");
    assert!(to.outset(cascade_layout::Insets::uniform(0.5)).contains(last), "{last:?} on {to:?}");
}

#[test]
fn hidden_subscribers_count_their_links_on_the_stub() {
    let state = ViewState { hidden_machines: ["Payment".to_owned()].into_iter().collect(), ..structure() };
    let scene = scene(&state, SceneMode::View);
    let stub = scene
        .nodes
        .iter()
        .find_map(|n| match &n.target {
            HitTarget::MachineStub { machine, links } if machine == "Payment" => Some(*links),
            _ => None,
        })
        .expect("a Payment stub");
    assert_eq!(stub, 3, "the three subscription links, merged per pill");
    assert!(edges_of(&scene, EdgeKind::Notify).is_empty(), "links to a stub are stub links");
    assert_eq!(edges_of(&scene, EdgeKind::StubLink).len(), 3);
}
