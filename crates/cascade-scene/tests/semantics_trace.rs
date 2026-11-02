//! The trace view draws the steps of subscriptions, the clock, choices and
//! `sets`: a delivery arrow for `Notify`, a full-width divider for `Wait`,
//! a clock self-message for `TimerExpired`, a decision marker for `Choice`
//! and a note for `SetFields`.

mod build_play;
mod semantics;

use build_play::Bench;
use cascade_scene::{
    Arrow, Dash, EdgeKind, Emphasis, FontStyle, HitTarget, MonoMeasure, Scene, SceneBuilder, SceneEdge, SceneInput,
    SceneMode, SceneNode, TextMeasure, ViewKind, ViewState, machine_styles,
};
use cascade_sim::Trace;
use semantics::*;

fn build(bench: &Bench, traces: &[Trace], selection: &[&str]) -> Scene {
    let state = ViewState {
        view: ViewKind::Trace,
        selection: selection.iter().map(|k| k.parse().expect("key")).collect(),
        ..ViewState::default()
    };
    let input = SceneInput {
        model: &bench.model,
        graph: &bench.graph,
        findings: &[],
        view: &state,
        theme: &bench.theme,
        measure: &MonoMeasure::default(),
        sidecar: &bench.sidecar,
        traces,
        diff: None,
        mode: SceneMode::View,
        play: None,
    };
    SceneBuilder::new().build(&input).expect("scene builds")
}

fn saga() -> (Bench, Scene) {
    let bench = Bench::new(SAGA);
    let trace = saga_trace(&bench.model);
    let scene = build(&bench, &[trace], &[]);
    (bench, scene)
}

fn step_target(step: u32) -> HitTarget {
    HitTarget::TraceStep { ordering: 0, step }
}

fn step_edge(scene: &Scene, step: u32) -> &SceneEdge {
    let t = step_target(step);
    scene.edges.iter().find(|e| e.target == t).unwrap_or_else(|| panic!("no edge for step {step}"))
}

fn step_node(scene: &Scene, step: u32) -> &SceneNode {
    let t = step_target(step);
    scene.nodes.iter().find(|n| n.target == t).unwrap_or_else(|| panic!("no node for step {step}"))
}

fn header(scene: &Scene, lifeline: u32) -> &SceneNode {
    let t = HitTarget::Lifeline { ordering: 0, lifeline };
    scene.nodes.iter().find(|n| n.target == t).expect("header")
}

fn x(scene: &Scene, lifeline: cascade_sim::LifelineIx) -> f32 {
    header(scene, lifeline.0).rect.center().x
}

#[test]
fn every_step_draws_something() {
    let (bench, scene) = saga();
    let steps = saga_trace(&bench.model).steps.len();
    for i in 0..steps {
        let t = step_target(u32::try_from(i).expect("small"));
        let drawn = scene.nodes.iter().any(|n| n.target == t) || scene.edges.iter().any(|e| e.target == t);
        assert!(drawn, "step {i} is drawn");
    }
}

#[test]
fn notify_is_a_delivery_from_the_emitter_to_the_subscriber() {
    let (bench, scene) = saga();
    let edge = step_edge(&scene, NOTIFY_PLACED);
    assert_eq!(edge.kind, EdgeKind::Message);
    assert!((edge.points[0].x - x(&scene, O1)).abs() < 0.5, "from o1, which emitted OrderPlaced");
    assert!((edge.points.last().expect("end").x - x(&scene, P1)).abs() < 0.5, "to p1");
    assert_eq!(label_of(edge).text, "OrderPlaced");
    let payment = machine_styles(&bench.model, &bench.theme)[1].hue;
    assert_eq!(edge.stroke.color, payment, "in the subscriber's hue");
    assert_eq!(edge.stroke.dash, Dash::Solid);
    assert_eq!(edge.arrow, Arrow::End);
    let back = step_edge(&scene, NOTIFY_CAPTURED);
    assert!((back.points[0].x - x(&scene, P1)).abs() < 0.5 && (back.points[1].x - x(&scene, O1)).abs() < 0.5);
}

#[test]
fn wait_is_a_full_width_divider_with_the_new_time() {
    let (_, scene) = saga();
    let divider = step_edge(&scene, WAIT);
    assert_eq!(divider.arrow, Arrow::None);
    assert_eq!(divider.points.len(), 2);
    assert!((divider.points[0].y - divider.points[1].y).abs() < 0.01, "horizontal");
    let first = header(&scene, 0).rect;
    let last = header(&scene, 5).rect;
    assert!(divider.points[0].x <= first.left() + 0.01, "from the block's left edge");
    assert!(divider.points[1].x >= last.right() - 0.01, "to its right edge");
    assert!(matches!(divider.stroke.dash, Dash::Dashed { .. }));
    assert_eq!(label_of(divider).text, "◷ +10s · now 10s");
}

#[test]
fn an_expired_timer_is_a_clock_self_message() {
    let (bench, scene) = saga();
    let edge = step_edge(&scene, TIMER);
    let px = x(&scene, P1);
    assert!((edge.points[0].x - px).abs() < 0.5 && (edge.points.last().expect("end").x - px).abs() < 0.5);
    assert!(edge.points.len() > 2, "a loop on p1's lifeline");
    assert!(edge.points.iter().all(|p| p.x >= px - 0.01), "to the right of it");
    let label = label_of(edge);
    assert_eq!(label.text, "◷ 10s");
    assert!(label.origin.x > px, "beside the loop");
    // Clear of the next lifeline.
    let w = MonoMeasure::default().width(&label.text, label.font_size);
    assert!(label.origin.x + w < x(&scene, R1), "{label:?}");
    let payment = machine_styles(&bench.model, &bench.theme)[1].hue;
    assert_eq!(edge.stroke.color, payment);
    assert!(matches!(edge.stroke.dash, Dash::Dashed { .. }));
}

#[test]
fn a_choice_marks_the_option_taken() {
    let (_, scene) = saga();
    let marker = step_node(&scene, CHOICE);
    assert!((marker.rect.center().x - x(&scene, P1)).abs() < 0.5);
    assert_eq!(marker.labels[0].text, "◇ card accepted (default)");
    assert_eq!(marker.labels[0].style, FontStyle::Italic);
    assert_eq!(marker.labels[1].text, "of card accepted | else");
    // Chosen from a pending decision: no `default`.
    let bench = Bench::new(SAGA);
    let mut trace = saga_trace(&bench.model);
    if let cascade_sim::TraceStepKind::Choice { defaulted, chosen, .. } = &mut trace.steps[CHOICE as usize].kind {
        *defaulted = false;
        *chosen = "else".into();
    }
    let scene = build(&bench, &[trace], &[]);
    assert_eq!(step_node(&scene, CHOICE).labels[0].text, "◇ else");
}

#[test]
fn set_fields_is_a_note_on_the_instance() {
    let (_, scene) = saga();
    let note = step_node(&scene, SET_ORDER_ID);
    assert!((note.rect.center().x - x(&scene, P1)).abs() < 0.5);
    assert_eq!(note.labels[0].text, "set orderId = o-1");
}

#[test]
fn rows_keep_their_order() {
    let (_, scene) = saga();
    let y = |step: u32| {
        let t = step_target(step);
        scene
            .nodes
            .iter()
            .find(|n| n.target == t)
            .map(|n| n.rect.center().y)
            .or_else(|| scene.edges.iter().find(|e| e.target == t).map(|e| e.points[0].y))
            .expect("drawn")
    };
    let ys: Vec<f32> = (0..15).map(y).collect();
    assert!(ys.windows(2).all(|w| w[1] > w[0]), "{ys:?}");
}

#[test]
fn selection_reaches_the_new_steps() {
    let bench = Bench::new(SAGA);
    let trace = saga_trace(&bench.model);
    let scene = build(&bench, std::slice::from_ref(&trace), &["trigger:Payment.OrderPlaced"]);
    assert_eq!(step_edge(&scene, NOTIFY_PLACED).emphasis, Emphasis::Selected);
    assert_eq!(step_edge(&scene, NOTIFY_CAPTURED).emphasis, Emphasis::Normal);
    let scene = build(&bench, std::slice::from_ref(&trace), &["trigger:Payment.after-10s"]);
    assert_eq!(step_edge(&scene, TIMER).emphasis, Emphasis::Selected);
}
