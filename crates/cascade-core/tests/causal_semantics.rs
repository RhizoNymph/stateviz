//! Subscription (`Notify`) and timer (`Delay`) edges in the causal graph,
//! and how they count as transition successors and in cycles.

use cascade_core::analysis::{Check, FindingDetail};
use cascade_core::{CausalEdgeKind, CausalGraph, CausalNode, Model, TransitionId, analyze, load_str};

fn load(text: &str) -> Model {
    match load_str(text) {
        Ok(model) => model,
        Err(err) => panic!("{err}"),
    }
}

fn transition(model: &Model, label: &str) -> TransitionId {
    match model.transition_ids().find(|&t| model.transition_label(t) == label) {
        Some(t) => t,
        None => panic!("no transition {label}"),
    }
}

/// `(from label, to label, kind)` of every edge of the given kind family.
fn edges(model: &Model, graph: &CausalGraph, want: fn(CausalEdgeKind) -> bool) -> Vec<(String, String)> {
    let label = |node: CausalNode| match node {
        CausalNode::Transition(t) => model.transition_label(t),
        CausalNode::Event(e) => model.event(e).name.clone(),
        CausalNode::External(x) => model.external(x).name.clone(),
        CausalNode::Handler(h) => model.label_of(cascade_core::ElementRef::Handler(h)),
    };
    let mut out: Vec<(String, String)> = graph
        .edges()
        .filter(|(_, e)| want(e.kind))
        .map(|(_, e)| (label(graph.node(e.from)), label(graph.node(e.to))))
        .collect();
    out.sort();
    out
}

const SAGA: &str = r#"
machines:
  Payment:
    states: [open, captured]
    transitions:
      - { from: open, to: captured, on: capture, emits: [PaymentCaptured] }
  Checkout:
    fields: [orderId]
    states:
      - waiting
      - paid
      - review:
          states: [first, second]
      - expired
    transitions:
      - from: waiting
        to: paid
        on: { event: PaymentCaptured, where: "orderId == event.orderId" }
      - { from: paid, to: review, on: inspect }
      - { from: review.first, to: review.second, on: next }
      - { from: [waiting, paid], to: expired, after: 30m }
      - { from: review, to: expired, after: 1h }
      - { from: review.second, to: expired, after: 5s }
external:
  Gateway: [Payment.capture]
  Clerk: [Checkout.inspect, Checkout.next]
"#;

#[test]
fn notify_edges_go_from_the_event_to_each_subscribing_transition() {
    let model = load(SAGA);
    let graph = CausalGraph::build(&model);
    let notify = edges(&model, &graph, |k| matches!(k, CausalEdgeKind::Notify { .. }));
    assert_eq!(notify, [("PaymentCaptured".to_owned(), "Checkout: waiting → paid".to_owned())]);
    let capture = transition(&model, "Payment: open → captured");
    let successors = graph.transition_successors(capture);
    assert_eq!(successors, [(transition(&model, "Checkout: waiting → paid"), None)]);
}

#[test]
fn delay_edges_go_from_transitions_that_enter_the_timer_source() {
    let model = load(SAGA);
    let graph = CausalGraph::build(&model);
    let delay = edges(&model, &graph, |k| matches!(k, CausalEdgeKind::Delay { .. }));
    assert_eq!(
        delay,
        [
            // Entering `paid` arms its 30m timer.
            ("Checkout: waiting → paid".to_owned(), "Checkout: paid → expired".to_owned()),
            // Entering `review` arms the 1h timer, and enters `review.first` by default.
            ("Checkout: paid → review".to_owned(), "Checkout: review → expired".to_owned()),
            // Moving inside `review` enters `review.second`.
            ("Checkout: review.first → review.second".to_owned(), "Checkout: review.second → expired".to_owned()),
            // A transition into `review.second` is inside `review`: the contract's rule.
            ("Checkout: review.first → review.second".to_owned(), "Checkout: review → expired".to_owned()),
        ]
        .into_iter()
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>()
    );
    // The initial state's timer has no incoming `Delay` edge.
    let waiting_timer = transition(&model, "Checkout: waiting → expired");
    let ix = graph.ix_of(CausalNode::Transition(waiting_timer)).expect("in graph");
    assert!(graph.incoming(ix).is_empty());
    // Delay links are not successors.
    let into_paid = transition(&model, "Checkout: waiting → paid");
    assert!(graph.transition_successors(into_paid).is_empty());
}

#[test]
fn history_and_default_entry_arm_inner_timers() {
    let model = load(
        r#"
machines:
  M:
    states:
      - idle
      - busy:
          states: [a, b, { h: { kind: history } }]
    transitions:
      - { from: idle, to: busy, on: start }
      - { from: idle, to: busy.h, on: resume }
      - { from: busy.b, to: idle, after: 1s }
      - { from: busy.a, to: idle, after: 2s }
external:
  U: [M.start, M.resume]
"#,
    );
    let graph = CausalGraph::build(&model);
    let delay = edges(&model, &graph, |k| matches!(k, CausalEdgeKind::Delay { .. }));
    assert_eq!(
        delay,
        [
            ("M: idle → busy".to_owned(), "M: busy.a → idle".to_owned()),
            ("M: idle → busy.h".to_owned(), "M: busy.a → idle".to_owned()),
            ("M: idle → busy.h".to_owned(), "M: busy.b → idle".to_owned()),
        ]
    );
}

#[test]
fn subscription_cycles_have_no_rule() {
    let model = load(
        r#"
machines:
  A:
    states: [x]
    transitions:
      - { from: x, to: x, on: { event: Pong }, emits: [Ping] }
  B:
    states: [y]
    transitions:
      - { from: y, to: y, on: { event: Ping }, emits: [Pong] }
"#,
    );
    let graph = CausalGraph::build(&model);
    let findings = analyze(&model, &graph);
    let cycle = findings.iter().find(|f| f.check() == Check::CascadeCycle).expect("a cycle");
    let FindingDetail::CascadeCycle { first, rest } = &cycle.detail else { panic!("not a cycle") };
    assert!(first.rule.is_none());
    assert!(rest.iter().all(|s| s.rule.is_none()));
    assert!(cycle.message.contains("subscription"), "{}", cycle.message);
}
