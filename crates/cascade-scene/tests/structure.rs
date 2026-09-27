//! The structure view: lanes, states, pills, nesting, collapse, links.

mod common;

use cascade_core::{Direction, ElementRef, Finding, FindingDetail, Severity};
use cascade_scene::{
    Arrow, Border, ConeFocus, Dash, EdgeKind, Emphasis, FontWeight, HitTarget, OutsideFocus, Overlay, Rgba, Scene,
    Shape, ViewKind, ViewState,
};
use common::*;

const OKABE_BLUE: Rgba = Rgba::hex(0x0072B2);
const OKABE_GREEN: Rgba = Rgba::hex(0x009E73);

const NESTED: &str = r#"
machines:
  Job:
    color: orange
    initial: idle
    states:
      - idle
      - running:
          initial: fetching
          states: [fetching, parsing]
      - history: { kind: history }
      - deep: { kind: deep-history }
      - done: { kind: final }
    transitions:
      - { from: idle, to: running, on: go, guard: "queue not empty", emits: [Started] }
      - { from: running.fetching, to: running.parsing, on: fetched }
      - { from: running, to: done, on: finish }
      - { from: idle, to: running.parsing, on: resume }
  Audit:
    states: [waiting, logged]
    transitions:
      - { from: waiting, to: logged, on: log }
controllers:
  Auditor:
    on:
      Started: [{ fire: Audit.log }]
"#;

fn structure() -> ViewState {
    ViewState { view: ViewKind::Structure, ..ViewState::default() }
}

fn lane<'a>(scene: &'a Scene, key: &str) -> &'a cascade_scene::Lane {
    let t = target(key);
    scene.lanes.iter().find(|l| l.target == t).unwrap_or_else(|| panic!("no lane {key}"))
}

fn inside(outer: cascade_layout::Rect, inner: cascade_layout::Rect) -> bool {
    inner.left() >= outer.left() - 0.01
        && inner.top() >= outer.top() - 0.01
        && inner.right() <= outer.right() + 0.01
        && inner.bottom() <= outer.bottom() + 0.01
}

#[test]
fn machines_are_lanes_stacked_in_definition_order() {
    let fx = Fixture::new(SPEC_EXAMPLE);
    let scene = fx.scene(&structure());
    assert_eq!(scene.view, ViewKind::Structure);
    let order = lane(&scene, "machine:Order");
    let shipment = lane(&scene, "machine:Shipment");
    assert_eq!(scene.lanes.len(), 2);
    assert!(order.rect.bottom() <= shipment.rect.top(), "stacked top to bottom");
    assert_eq!(order.title.text, "Order");
    assert_eq!(order.title.color, OKABE_BLUE, "header in the machine's hue");
    assert_eq!(order.title.weight, FontWeight::Bold);
    assert!(!order.collapsed);
    assert_eq!(shipment.title.color, OKABE_GREEN);
    for state in ["state:Order:draft", "state:Order:pending", "state:Order:paid", "state:Order:cancelled"] {
        assert!(inside(order.rect, node(&scene, state).rect), "{state} in its lane");
    }
    assert!(inside(shipment.rect, node(&scene, "state:Shipment:picking").rect));
}

#[test]
fn states_are_pale_rounded_rects_with_initial_final_and_history_marks() {
    let fx = Fixture::new(NESTED);
    let scene = fx.scene(&structure());
    let idle = node(&scene, "state:Job:idle");
    assert!(matches!(idle.shape, Shape::RoundedRect { .. }));
    assert!(matches!(idle.border, Border::ThickLeft(w) if w > 0.0), "initial state");
    let style = cascade_scene::machine_styles(&fx.model, &fx.theme)[0];
    assert_eq!(idle.fill, Some(style.pale), "pale machine fill");
    assert_eq!(idle.stroke.color, style.hue);
    assert_eq!(node(&scene, "state:Job:done").border, Border::Double, "final state");
    assert_eq!(node(&scene, "state:Job:running.parsing").border, Border::Single);
    assert!(matches!(node(&scene, "state:Job:running.fetching").border, Border::ThickLeft(_)), "initial child");

    let history = node(&scene, "state:Job:history");
    assert!(history.labels.iter().any(|l| l.text == "H"));
    let deep = node(&scene, "state:Job:deep");
    assert!(deep.labels.iter().any(|l| l.text == "H*"));
    let rings = scene
        .overlays
        .iter()
        .filter(|o| matches!(o, Overlay::Rect { rect, radius, .. } if *radius > 0.0 && (inside(history.rect, *rect) || inside(deep.rect, *rect))))
        .count();
    assert_eq!(rings, 2, "a circle around each history marker");
}

#[test]
fn transition_pills_sit_on_the_edges() {
    let fx = Fixture::new(SPEC_EXAMPLE);
    let scene = fx.scene(&structure());
    let pill = node(&scene, "transition:Order:pending->paid@capture_ok");
    assert_eq!(pill.shape, Shape::Pill);
    assert_eq!(pill.fill, Some(OKABE_BLUE));
    assert_eq!(pill.labels[0].text, "pending → paid");
    assert_eq!(pill.labels[1].text, "capture_ok");
    assert!(inside(lane(&scene, "machine:Order").rect, pill.rect));

    let arrows = edge_to(&scene, "transition:Order:pending->paid@capture_ok");
    assert_eq!(arrows.len(), 2, "state → pill → state");
    for e in &arrows {
        assert_eq!(e.kind, EdgeKind::Transition);
        assert_eq!(e.stroke.dash, Dash::Solid);
        assert_eq!(e.stroke.color, OKABE_BLUE, "solid, machine hue");
    }
    let pending = node(&scene, "state:Order:pending");
    let paid = node(&scene, "state:Order:paid");
    let into = arrows.iter().find(|e| touches(pending.rect, e.points[0])).expect("pending → pill");
    assert_eq!(into.arrow, Arrow::None);
    assert!(touches(pill.rect, *into.points.last().expect("end")));
    let out = arrows.iter().find(|e| touches(paid.rect, *e.points.last().expect("end"))).expect("pill → paid");
    assert_eq!(out.arrow, Arrow::End);
    // 5 transitions, 2 arrows each, plus one cross-lane link.
    assert_eq!(edges_of(&scene, EdgeKind::Transition).len(), 10);
    assert_eq!(scene.nodes.iter().filter(|n| n.shape == Shape::Pill).count(), 5);
}

#[test]
fn guards_label_the_arrow_into_the_pill() {
    let fx = Fixture::new(NESTED);
    let scene = fx.scene(&structure());
    let labels: Vec<_> = edge_to(&scene, "transition:Job:idle->running@go")
        .iter()
        .filter_map(|e| e.label.as_ref())
        .map(|l| l.text.clone())
        .collect();
    assert_eq!(labels, ["[queue not empty]"]);
}

#[test]
fn cross_lane_links_join_pills_with_event_and_controller_names() {
    let fx = Fixture::new(SPEC_EXAMPLE);
    let scene = fx.scene(&structure());
    let fires = edges_of(&scene, EdgeKind::Fire);
    assert_eq!(fires.len(), 1);
    let link = fires[0];
    assert!(matches!(link.stroke.dash, Dash::Dashed { .. }));
    assert_eq!(link.stroke.color, OKABE_GREEN, "dashed in the target machine's hue");
    assert_eq!(link.label.as_ref().map(|l| l.text.as_str()), Some("OrderPaid › Fulfillment"));
    assert_eq!(link.target, target("rule:Fulfillment/OrderPaid#0"));
    let from = node(&scene, "transition:Order:pending->paid@capture_ok");
    let to = node(&scene, "transition:Shipment:idle->picking@start");
    assert!(touches(from.rect, link.points[0]));
    assert!(touches(to.rect, *link.points.last().expect("end")));
    // Events, controllers and sources are not drawn as nodes here.
    assert!(find_node(&scene, "event:OrderPaid").is_none());
    assert!(find_node(&scene, "handler:Fulfillment/OrderPaid").is_none());
    assert!(find_node(&scene, "external:Customer").is_none());
}

#[test]
fn compound_states_get_a_band_inside_their_machine_lane() {
    let fx = Fixture::new(NESTED);
    let scene = fx.scene(&structure());
    let job = lane(&scene, "machine:Job");
    let band = lane(&scene, "state:Job:running");
    assert!(inside(job.rect, band.rect));
    assert_eq!(band.title.text, "running");
    for child in ["state:Job:running.fetching", "state:Job:running.parsing"] {
        assert!(inside(band.rect, node(&scene, child).rect), "{child} in its band");
    }
    let running = node(&scene, "state:Job:running");
    assert!(inside(job.rect, running.rect));
    assert!(!inside(band.rect, running.rect));
    assert!(running.labels.iter().any(|l| l.text == "▾ 2 states"));
    // Machine lanes come before the bands they contain.
    let job_at = scene.lanes.iter().position(|l| l.target == target("machine:Job")).expect("job");
    let band_at = scene.lanes.iter().position(|l| l.target == target("state:Job:running")).expect("band");
    assert!(job_at < band_at);
}

#[test]
fn collapsing_a_compound_state_hides_its_children_and_reroutes() {
    let fx = Fixture::new(NESTED);
    let state = ViewState { collapsed: [k("state:Job:running")].into_iter().collect(), ..structure() };
    let scene = fx.scene(&state);
    assert!(find_node(&scene, "state:Job:running.fetching").is_none());
    assert!(find_node(&scene, "state:Job:running.parsing").is_none());
    assert!(scene.lanes.iter().all(|l| l.target != target("state:Job:running")), "no band");
    let running = node(&scene, "state:Job:running");
    assert!(running.labels.iter().any(|l| l.text == "▸ 2 states"));
    // The internal transition disappears; the one into a child reroutes.
    assert!(find_node(&scene, "transition:Job:running.fetching->running.parsing@fetched").is_none());
    let resume = edge_to(&scene, "transition:Job:idle->running.parsing@resume");
    assert_eq!(resume.len(), 2);
    let into_running = resume.iter().find(|e| e.arrow == Arrow::End).expect("into running");
    assert!(touches(running.rect, *into_running.points.last().expect("end")));
    let pill = node(&scene, "transition:Job:idle->running.parsing@resume");
    assert_eq!(pill.labels[0].text, "idle → running.parsing", "pills keep their real endpoints");
}

#[test]
fn collapsing_a_machine_makes_it_one_node() {
    let fx = Fixture::new(SPEC_EXAMPLE);
    let state = ViewState { collapsed: [k("machine:Order")].into_iter().collect(), ..structure() };
    let scene = fx.scene(&state);
    let order = lane(&scene, "machine:Order");
    assert!(order.collapsed);
    assert!(!lane(&scene, "machine:Shipment").collapsed);
    let machine = node(&scene, "machine:Order");
    assert!(inside(order.rect, machine.rect));
    assert_eq!(scene.nodes.iter().filter(|n| order.rect.contains(n.rect.center())).count(), 1);
    assert!(find_node(&scene, "state:Order:pending").is_none());
    let fires = edges_of(&scene, EdgeKind::Fire);
    assert_eq!(fires.len(), 1, "the cross link now leaves the machine node");
    assert!(touches(machine.rect, fires[0].points[0]));
}

#[test]
fn hidden_machines_become_stubs_in_the_structure_view() {
    let fx = Fixture::new(SPEC_EXAMPLE);
    let state = ViewState { hidden_machines: ["Shipment".to_owned()].into_iter().collect(), ..structure() };
    let scene = fx.scene(&state);
    assert!(scene.lanes.iter().all(|l| l.target != target("machine:Shipment")));
    let stub = scene.nodes.iter().find(|n| n.shape == Shape::Stub).expect("stub");
    assert_eq!(stub.target, HitTarget::MachineStub { machine: "Shipment".into(), links: 1 });
    assert_eq!(stub.labels[0].text, "Shipment, 1 link");
    let links = edges_of(&scene, EdgeKind::StubLink);
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].stroke.dash, Dash::Dotted);
    assert!(find_node(&scene, "state:Shipment:idle").is_none());
}

#[test]
fn cones_focus_pills_and_the_states_they_touch() {
    let fx = Fixture::new(SPEC_EXAMPLE);
    let state = ViewState {
        cone: Some(ConeFocus { direction: Direction::Forward, depth: None }),
        ..ViewState { selection: vec![k("transition:Order:pending->paid@capture_ok")], ..structure() }
    };
    let scene = fx.scene(&state);
    assert_eq!(node(&scene, "transition:Order:pending->paid@capture_ok").emphasis, Emphasis::Selected);
    assert_eq!(node(&scene, "transition:Shipment:idle->picking@start").emphasis, Emphasis::Focused);
    assert_eq!(node(&scene, "state:Shipment:picking").emphasis, Emphasis::Focused);
    assert_eq!(node(&scene, "transition:Shipment:picking->shipped@handoff").emphasis, Emphasis::Dimmed);
    assert_eq!(node(&scene, "transition:Order:draft->pending@submit").emphasis, Emphasis::Dimmed);
    assert_eq!(edges_of(&scene, EdgeKind::Fire)[0].emphasis, Emphasis::Focused);
    assert!(scene.lanes.iter().all(|l| l.opacity == 1.0), "lanes stay as context");

    let mut hide = state.clone();
    hide.outside = OutsideFocus::Hide;
    let hidden = fx.scene(&hide);
    assert!(find_node(&hidden, "transition:Order:draft->pending@submit").is_none());
    assert!(find_node(&hidden, "transition:Shipment:idle->picking@start").is_some());
    assert!(hidden.nodes.iter().all(|n| n.emphasis != Emphasis::Dimmed));
}

#[test]
fn hide_mode_drops_lanes_with_nothing_left() {
    let fx = Fixture::new(CHAIN);
    let state = ViewState {
        selection: vec![k("transition:A:a0->a1@go")],
        cone: Some(ConeFocus { direction: Direction::Forward, depth: Some(1) }),
        outside: OutsideFocus::Hide,
        ..structure()
    };
    let scene = fx.scene(&state);
    assert!(scene.lanes.iter().any(|l| l.target == target("machine:B")));
    assert!(scene.lanes.iter().all(|l| l.target != target("machine:D")), "D is outside the cone");
}

#[test]
fn findings_badge_states_and_pills() {
    let mut fx = Fixture::new(SPEC_EXAMPLE);
    let ElementRef::State(state) = fx.element("state:Shipment:shipped") else { panic!("state") };
    let ElementRef::Transition(t) = fx.element("transition:Shipment:picking->shipped@handoff") else { panic!("t") };
    fx.findings = vec![
        Finding { severity: Severity::Warning, detail: FindingDetail::UnreachableState { state }, message: "u".into() },
        Finding {
            severity: Severity::Error,
            detail: FindingDetail::Nondeterminism {
                state,
                trigger: fx.model.transition(t).trigger,
                transitions: vec![t, t],
            },
            message: "n".into(),
        },
    ];
    let scene = fx.scene(&structure());
    let shipped = node(&scene, "state:Shipment:shipped");
    let badge = shipped.badge.as_ref().expect("badge");
    assert_eq!((badge.count, badge.severity), (2, Severity::Error));
    assert_eq!(shipped.stroke.color, fx.theme.finding);
    let pill = node(&scene, "transition:Shipment:picking->shipped@handoff");
    assert_eq!(pill.badge.as_ref().map(|b| b.count), Some(1));
}

#[test]
fn self_loops_and_same_machine_links_are_drawn() {
    let fx = Fixture::new(RETRY);
    let scene = fx.scene(&structure());
    let pill = node(&scene, "transition:R:s->s@retry");
    let fires = edges_of(&scene, EdgeKind::Fire);
    assert_eq!(fires.len(), 1, "the retry loop re-fires its own transition");
    assert!(fires[0].points.len() >= 3);
    assert!(pill.rect.contains(fires[0].points[0]) || fires[0].points[0].y <= pill.rect.top() + 0.01);
    assert!(scene.edges.iter().all(|e| !e.back_edge), "only the causal view marks back edges");
}

#[test]
fn selected_machines_and_compound_states_outline_their_lanes() {
    let fx = Fixture::new(NESTED);
    let state = ViewState { selection: vec![k("machine:Audit"), k("state:Job:running")], ..structure() };
    let scene = fx.scene(&state);
    assert_eq!(lane(&scene, "machine:Audit").stroke.width, fx.theme.selected_stroke_width);
    assert_eq!(lane(&scene, "state:Job:running").stroke.width, fx.theme.selected_stroke_width);
    assert!(lane(&scene, "machine:Job").stroke.width < fx.theme.selected_stroke_width);
    assert_eq!(lane(&scene, "machine:Audit").stroke.color, lane(&fx.scene(&structure()), "machine:Audit").stroke.color);
}
