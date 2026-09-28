//! The trace view: a sequence diagram built from hand-made traces.

mod common;

use std::collections::BTreeMap;

use cascade_core::{ElementRef, Model};
use cascade_layout::Rect;
use cascade_scene::{Dash, EdgeKind, Emphasis, HitTarget, Overlay, Rgba, Scene, SceneNode, Shape, ViewKind, ViewState};
use cascade_sim::{Lifeline, LifelineIx, StepIx, Trace, TraceStep, TraceStepKind};
use common::*;

const OKABE_BLUE: Rgba = Rgba::hex(0x0072B2);
const OKABE_GREEN: Rgba = Rgba::hex(0x009E73);

fn trace_view() -> ViewState {
    ViewState { view: ViewKind::Trace, ..ViewState::default() }
}

fn id<T>(fx: &Fixture, key: &str, pick: impl Fn(ElementRef) -> Option<T>) -> T {
    pick(fx.element(key)).unwrap_or_else(|| panic!("wrong kind for {key}"))
}

fn step(cause: Option<u32>, kind: TraceStepKind) -> TraceStep {
    TraceStep { cause: cause.map(StepIx), kind }
}

/// The spec example, run: submit, capture, and Fulfillment starts the
/// shipment; then a late timeout is dropped and two selector problems.
fn spec_trace(fx: &Fixture, ordering: Option<&str>) -> Trace {
    let m: &Model = &fx.model;
    let t = |k: &str| id(fx, k, |e| if let ElementRef::Transition(t) = e { Some(t) } else { None });
    let s = |k: &str| id(fx, k, |e| if let ElementRef::State(s) = e { Some(s) } else { None });
    let trig = |k: &str| id(fx, k, |e| if let ElementRef::Trigger(t) = e { Some(t) } else { None });
    let rule = id(fx, "rule:Fulfillment/OrderPaid#0", |e| if let ElementRef::Rule(r) = e { Some(r) } else { None });
    let handler =
        id(fx, "handler:Fulfillment/OrderPaid", |e| if let ElementRef::Handler(h) = e { Some(h) } else { None });
    let paid_event = id(fx, "event:OrderPaid", |e| if let ElementRef::Event(ev) = e { Some(ev) } else { None });
    let (customer, gateway) = (LifelineIx(0), LifelineIx(1));
    let (o1, s1, fulfillment) = (LifelineIx(2), LifelineIx(3), LifelineIx(4));
    let submit = t("transition:Order:draft->pending@submit");
    let paid = t("transition:Order:pending->paid@capture_ok");
    let start = t("transition:Shipment:idle->picking@start");
    Trace {
        scenario: "happy path".into(),
        ordering: ordering.map(str::to_owned),
        lifelines: vec![
            Lifeline::External { source: m.external_by_name("Customer").expect("c") },
            Lifeline::External { source: m.external_by_name("PaymentGateway").expect("g") },
            Lifeline::Instance { machine: m.machine_by_name("Order").expect("o"), name: "o1".into() },
            Lifeline::Instance { machine: m.machine_by_name("Shipment").expect("s"), name: "s1".into() },
            Lifeline::Controller { controller: m.controller_by_name("Fulfillment").expect("f") },
        ],
        steps: vec![
            step(
                None,
                TraceStepKind::ExternalFire { source: customer, target: o1, trigger: trig("trigger:Order.submit") },
            ),
            step(
                Some(0),
                TraceStepKind::Transition {
                    instance: o1,
                    transition: submit,
                    from: s("state:Order:draft"),
                    to: s("state:Order:pending"),
                },
            ),
            step(
                None,
                TraceStepKind::ExternalFire { source: gateway, target: o1, trigger: trig("trigger:Order.capture_ok") },
            ),
            step(
                Some(2),
                TraceStepKind::Transition {
                    instance: o1,
                    transition: paid,
                    from: s("state:Order:pending"),
                    to: s("state:Order:paid"),
                },
            ),
            step(Some(3), TraceStepKind::Emit { instance: o1, event: paid_event }),
            step(Some(4), TraceStepKind::Deliver { controller: fulfillment, event: paid_event, handler }),
            step(Some(5), TraceStepKind::Fire { controller: fulfillment, target: s1, rule }),
            step(
                Some(6),
                TraceStepKind::Transition {
                    instance: s1,
                    transition: start,
                    from: s("state:Shipment:idle"),
                    to: s("state:Shipment:picking"),
                },
            ),
            step(
                None,
                TraceStepKind::Dropped {
                    instance: o1,
                    trigger: trig("trigger:Order.timeout"),
                    state: s("state:Order:paid"),
                },
            ),
            step(Some(5), TraceStepKind::NoTarget { controller: fulfillment, rule }),
            step(Some(5), TraceStepKind::Ambiguous { controller: fulfillment, rule, candidates: vec![s1, s1] }),
        ],
        final_states: BTreeMap::from([(o1, s("state:Order:paid")), (s1, s("state:Shipment:picking"))]),
    }
}

fn with_traces(traces: Vec<Trace>) -> (Fixture, Scene) {
    let mut fx = Fixture::new(SPEC_EXAMPLE);
    fx.traces = traces;
    let scene = fx.scene(&trace_view());
    (fx, scene)
}

fn at<'a>(scene: &'a Scene, t: &HitTarget) -> Vec<&'a SceneNode> {
    scene.nodes.iter().filter(|n| &n.target == t).collect()
}

fn header(scene: &Scene, ordering: u8, lifeline: u32) -> SceneNode {
    at(scene, &HitTarget::Lifeline { ordering, lifeline })
        .into_iter()
        .min_by(|a, b| a.rect.top().total_cmp(&b.rect.top()))
        .cloned()
        .unwrap_or_else(|| panic!("no header {ordering}/{lifeline}"))
}

fn step_edge(scene: &Scene, ordering: u8, step: u32) -> &cascade_scene::SceneEdge {
    let t = HitTarget::TraceStep { ordering, step };
    scene.edges.iter().find(|e| e.target == t).unwrap_or_else(|| panic!("no edge for step {step}"))
}

fn step_node(scene: &Scene, ordering: u8, step: u32) -> &SceneNode {
    let t = HitTarget::TraceStep { ordering, step };
    scene.nodes.iter().find(|n| n.target == t).unwrap_or_else(|| panic!("no node for step {step}"))
}

fn step_y(scene: &Scene, step: u32) -> f32 {
    let t = HitTarget::TraceStep { ordering: 0, step };
    if let Some(e) = scene.edges.iter().find(|e| e.target == t) {
        return e.points[0].y;
    }
    step_node(scene, 0, step).rect.center().y
}

#[test]
fn no_traces_gives_an_empty_scene_with_a_hint() {
    let fx = Fixture::new(SPEC_EXAMPLE);
    let scene = fx.scene(&trace_view());
    assert_eq!(scene.view, ViewKind::Trace);
    assert!(scene.nodes.is_empty() && scene.edges.is_empty() && scene.lanes.is_empty());
    assert_eq!(scene.notes, ["Pick a scenario to trace."]);
}

#[test]
fn lifelines_have_headers_in_the_entity_hue_and_run_down() {
    let fx = Fixture::new(SPEC_EXAMPLE);
    let (_, scene) = with_traces(vec![spec_trace(&fx, None)]);
    let heads: Vec<SceneNode> = (0..5).map(|i| header(&scene, 0, i)).collect();
    for pair in heads.windows(2) {
        assert!(pair[0].rect.right() <= pair[1].rect.left(), "definition order, left to right");
    }
    let order = &heads[2];
    assert_eq!(order.shape, Shape::Rect);
    assert_eq!(order.fill, Some(OKABE_BLUE), "instances take their machine's hue");
    assert_eq!(order.labels[0].text, "o1: Order");
    assert_eq!(heads[3].fill, Some(OKABE_GREEN));
    let source = &heads[0];
    assert_eq!(source.fill, None, "sources are neutral");
    assert_eq!(source.stroke.color, fx.theme.external);
    assert_eq!(source.labels[0].text, "Customer");
    let controller = &heads[4];
    assert_eq!(controller.shape, Shape::Hexagon);
    assert_eq!(controller.fill, None);
    assert_eq!(controller.stroke.color, fx.theme.controller);

    let lines: Vec<_> = scene
        .overlays
        .iter()
        .filter_map(|o| match o {
            Overlay::Line { from, to, .. } => Some((*from, *to)),
            _ => None,
        })
        .collect();
    assert_eq!(lines.len(), 5, "one lifeline each");
    for (head, (from, to)) in heads.iter().zip(&lines) {
        assert_eq!(from.x, to.x, "vertical");
        assert!((from.x - head.rect.center().x).abs() < 0.5);
        assert!(from.y >= head.rect.bottom() - 0.01 && to.y > from.y);
    }
    assert_eq!(scene.notes, Vec::<String>::new());
}

#[test]
fn messages_run_between_lifelines_in_time_order() {
    let fx = Fixture::new(SPEC_EXAMPLE);
    let (fx2, scene) = with_traces(vec![spec_trace(&fx, None)]);
    let theme = &fx2.theme;
    let x = |i: u32| header(&scene, 0, i).rect.center().x;

    let submit = step_edge(&scene, 0, 0);
    assert_eq!(submit.kind, EdgeKind::Message);
    assert!((submit.points[0].x - x(0)).abs() < 0.5 && (submit.points.last().expect("end").x - x(2)).abs() < 0.5);
    assert_eq!(submit.label.as_ref().map(|l| l.text.as_str()), Some("submit"));
    assert_eq!(submit.stroke.color, theme.external);

    let deliver = step_edge(&scene, 0, 5);
    assert!((deliver.points[0].x - x(2)).abs() < 0.5, "from the emitting instance");
    assert!((deliver.points.last().expect("end").x - x(4)).abs() < 0.5, "to the controller");
    assert_eq!(deliver.label.as_ref().map(|l| l.text.as_str()), Some("OrderPaid"));
    assert!(matches!(deliver.stroke.dash, Dash::Dashed { .. }));
    assert_eq!(deliver.stroke.color, theme.neutral);

    let fire = step_edge(&scene, 0, 6);
    assert!((fire.points[0].x - x(4)).abs() < 0.5 && (fire.points.last().expect("end").x - x(3)).abs() < 0.5);
    assert_eq!(fire.label.as_ref().map(|l| l.text.as_str()), Some("start"));
    assert_eq!(fire.stroke.color, OKABE_GREEN, "fires take the target's hue");
    assert!(matches!(fire.stroke.dash, Dash::Dashed { .. }));

    let ys: Vec<f32> = (0..11).map(|i| step_y(&scene, i)).collect();
    for pair in ys.windows(2) {
        assert!(pair[1] > pair[0], "time runs down: {ys:?}");
    }
    let first_header_bottom = header(&scene, 0, 0).rect.bottom();
    assert!(ys[0] > first_header_bottom);
}

#[test]
fn transitions_emits_drops_and_notes_sit_on_their_lifeline() {
    let fx = Fixture::new(SPEC_EXAMPLE);
    let (fx2, scene) = with_traces(vec![spec_trace(&fx, None)]);
    let x = |i: u32| header(&scene, 0, i).rect.center().x;
    let style = cascade_scene::machine_styles(&fx2.model, &fx2.theme)[0];

    let paid = step_node(&scene, 0, 3);
    assert!(matches!(paid.shape, Shape::RoundedRect { .. }));
    assert_eq!(paid.labels[0].text, "pending → paid");
    assert_eq!(paid.fill, Some(style.pale));
    assert!((paid.rect.center().x - x(2)).abs() < 0.5);

    let emit = step_node(&scene, 0, 4);
    assert_eq!(emit.shape, Shape::Tag);
    assert_eq!(emit.labels[0].text, "OrderPaid");
    assert!(emit.rect.contains(cascade_layout::Point::new(x(2), emit.rect.center().y)));

    let dropped = step_node(&scene, 0, 8);
    assert_eq!(dropped.stroke.color, fx2.theme.finding, "red outline");
    assert!(dropped.labels[0].text.starts_with('✕'));
    assert!(dropped.labels[0].text.contains("timeout"));

    let none = step_node(&scene, 0, 9);
    assert!(none.labels[0].text.contains("no Shipment"), "{}", none.labels[0].text);
    assert!((none.rect.center().x - x(4)).abs() < 0.5, "notes sit on the controller");
    let ambiguous = step_node(&scene, 0, 10);
    assert!(ambiguous.labels[0].text.contains("ambiguous"));

    // Final states at the bottom of each instance lifeline.
    let finals: Vec<_> = at(&scene, &HitTarget::Lifeline { ordering: 0, lifeline: 2 });
    assert!(finals.iter().any(|n| n.labels.iter().any(|l| l.text == "paid")));
}

#[test]
fn spawns_create_the_lifeline_where_they_happen() {
    let fx = Fixture::new(SPEC_EXAMPLE);
    let mut trace = spec_trace(&fx, None);
    let rule = match trace.steps[6].kind {
        TraceStepKind::Fire { rule, .. } => rule,
        _ => panic!("fire"),
    };
    trace
        .lifelines
        .push(Lifeline::Instance { machine: fx.model.machine_by_name("Shipment").expect("s"), name: "s2".into() });
    trace.steps.push(step(Some(5), TraceStepKind::Spawn { controller: LifelineIx(4), instance: LifelineIx(5), rule }));
    let (_, scene) = with_traces(vec![trace]);
    let spawn = step_edge(&scene, 0, 11);
    let created = header(&scene, 0, 5);
    assert!(created.rect.top() > header(&scene, 0, 0).rect.bottom(), "created mid-trace");
    assert!(
        created.rect.contains(*spawn.points.last().expect("end"))
            || (spawn.points.last().expect("end").x - created.rect.left()).abs() < 0.5
    );
    assert!(spawn.label.as_ref().is_some_and(|l| l.text.contains("s2")));
}

#[test]
fn race_orderings_render_side_by_side_with_titles() {
    let fx = Fixture::new(SPEC_EXAMPLE);
    let (_, scene) =
        with_traces(vec![spec_trace(&fx, Some("Fulfillment first")), spec_trace(&fx, Some("Billing first"))]);
    let titles: Vec<_> = scene
        .overlays
        .iter()
        .filter_map(|o| match o {
            Overlay::Text { label, .. } => Some(label.text.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(titles, ["Fulfillment first", "Billing first"]);
    let left: Rect = (0..5).map(|i| header(&scene, 0, i).rect).reduce(|a, b| a.union(&b)).expect("left");
    let right: Rect = (0..5).map(|i| header(&scene, 1, i).rect).reduce(|a, b| a.union(&b)).expect("right");
    assert!(left.right() < right.left());
    assert!(scene.edges.iter().any(|e| e.target == HitTarget::TraceStep { ordering: 1, step: 6 }));
    // Nothing of the first ordering (wide notes included) reaches into the second.
    let side = |n: &SceneNode| match n.target {
        HitTarget::TraceStep { ordering, .. } | HitTarget::Lifeline { ordering, .. } => ordering,
        _ => panic!("unexpected target {:?}", n.target),
    };
    let first_right = scene.nodes.iter().filter(|n| side(n) == 0).map(|n| n.rect.right()).fold(f32::MIN, f32::max);
    let second_left = scene.nodes.iter().filter(|n| side(n) == 1).map(|n| n.rect.left()).fold(f32::MAX, f32::min);
    assert!(first_right < second_left, "{first_right} vs {second_left}");
}

#[test]
fn selection_and_cones_emphasise_steps() {
    let fx = Fixture::new(SPEC_EXAMPLE);
    let mut fx2 = Fixture::new(SPEC_EXAMPLE);
    fx2.traces = vec![spec_trace(&fx, None)];
    let state = ViewState { selection: vec![k("transition:Order:pending->paid@capture_ok")], ..trace_view() };
    let scene = fx2.scene(&state);
    assert_eq!(step_node(&scene, 0, 3).emphasis, Emphasis::Selected);
    assert_eq!(step_node(&scene, 0, 1).emphasis, Emphasis::Normal);

    let coned = ViewState {
        cone: Some(cascade_scene::ConeFocus { direction: cascade_core::Direction::Forward, depth: None }),
        outside: cascade_scene::OutsideFocus::Hide,
        ..state
    };
    let scene = fx2.scene(&coned);
    assert_eq!(step_node(&scene, 0, 7).emphasis, Emphasis::Focused, "idle → picking follows");
    assert_eq!(step_node(&scene, 0, 1).emphasis, Emphasis::Dimmed, "hide acts as dim in a trace");
    assert_eq!(step_node(&scene, 0, 1).opacity, fx2.theme.dim_opacity);
    assert_eq!(header(&scene, 0, 0).emphasis, Emphasis::Normal, "headers stay");
}
