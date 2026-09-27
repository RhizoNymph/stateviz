//! Causal graph derivation, depths, cones and path queries.

use cascade_core::{CausalEdgeKind, CausalGraph, CausalNode, Direction, Model, NodeIx, load_str};

const SPEC_EXAMPLE: &str = include_str!("../../../examples/order-fulfillment/cascade.yaml");

/// A chain with a cycle and a fan-out:
///
/// ```text
/// User ─▶ A:a0→a1 ─(Go)─▶ C1 ─▶ B:b0→b1 ─(Done)─▶ C2 ─▶ A:a1→a0 ─(Back)─▶ C3 ─▶ B:b1→b0
///                                                     └──────────────▶ D:d0→d1
/// ```
const CHAIN: &str = r#"
machines:
  A:
    states: [a0, a1]
    transitions:
      - { from: a0, to: a1, on: go, emits: [Go] }
      - { from: a1, to: a0, on: reset, emits: [Back] }
  B:
    states: [b0, b1]
    transitions:
      - { from: b0, to: b1, on: start, emits: [Done] }
      - { from: b1, to: b0, on: rewind }
  D:
    states: [d0, d1]
    transitions:
      - { from: d0, to: d1, on: note }
controllers:
  C1:
    on:
      Go: [{ fire: B.start }]
  C2:
    on:
      Done: [{ fire: A.reset }, { fire: D.note }]
  C3:
    on:
      Back: [{ fire: B.rewind }]
external:
  User: [A.go]
"#;

fn load(text: &str) -> Model {
    match load_str(text) {
        Ok(m) => m,
        Err(err) => panic!("{err}"),
    }
}

fn transition_node(model: &Model, graph: &CausalGraph, label: &str) -> NodeIx {
    let t = model
        .transition_ids()
        .find(|&t| model.transition_label(t) == label)
        .unwrap_or_else(|| panic!("no transition {label}"));
    graph.ix_of(CausalNode::Transition(t)).expect("transition node")
}

fn labels(model: &Model, graph: &CausalGraph, nodes: impl Iterator<Item = NodeIx>) -> Vec<String> {
    let mut out: Vec<String> = nodes.map(|n| model.label_of(graph.node(n).element())).collect();
    out.sort();
    out
}

#[test]
fn spec_example_edges() {
    let model = load(SPEC_EXAMPLE);
    let graph = CausalGraph::build(&model);
    // 3 externals + 5 transitions + 3 events + 1 handler.
    assert_eq!(graph.node_count(), 12);

    let kinds: Vec<&str> = graph
        .edges()
        .map(|(_, e)| match e.kind {
            CausalEdgeKind::Trigger { .. } => "trigger",
            CausalEdgeKind::Emit => "emit",
            CausalEdgeKind::Subscribe => "subscribe",
            CausalEdgeKind::Fire { .. } => "fire",
        })
        .collect();
    assert_eq!(kinds.iter().filter(|k| **k == "trigger").count(), 3);
    assert_eq!(kinds.iter().filter(|k| **k == "emit").count(), 3);
    assert_eq!(kinds.iter().filter(|k| **k == "subscribe").count(), 1);
    assert_eq!(kinds.iter().filter(|k| **k == "fire").count(), 1);

    // The spec's worked example: pending → paid causes idle → picking.
    let paid = model.transition_ids().nth(1).expect("paid");
    let successors = graph.transition_successors(paid);
    assert_eq!(successors.len(), 1);
    assert_eq!(model.transition_label(successors[0].0), "Shipment: idle → picking");
}

#[test]
fn depths_from_external_sources() {
    let model = load(SPEC_EXAMPLE);
    let graph = CausalGraph::build(&model);
    let depths = graph.depths();
    let depth_of = |label: &str| depths[transition_node(&model, &graph, label).index()];
    assert_eq!(depth_of("Order: pending → paid"), Some(1));
    assert_eq!(depth_of("Shipment: idle → picking"), Some(4));
    // Nothing external or causal ever fires `handoff`.
    assert_eq!(depth_of("Shipment: picking → shipped"), None);
}

#[test]
fn forward_cone_unlimited_and_limited() {
    let model = load(CHAIN);
    let graph = CausalGraph::build(&model);
    let a_go = transition_node(&model, &graph, "A: a0 → a1");

    let all = graph.cone(&[a_go], Direction::Forward, None);
    assert_eq!(
        labels(&model, &graph, all.nodes().map(|(n, _)| n)),
        [
            "A: a0 → a1",
            "A: a1 → a0",
            "B: b0 → b1",
            "B: b1 → b0",
            "Back",
            "C1 on Go",
            "C2 on Done",
            "C3 on Back",
            "D: d0 → d1",
            "Done",
            "Go",
        ]
    );
    assert_eq!(all.hops(a_go), Some(0));
    assert_eq!(all.hops(transition_node(&model, &graph, "B: b0 → b1")), Some(1));
    assert_eq!(all.hops(transition_node(&model, &graph, "D: d0 → d1")), Some(2));
    assert_eq!(all.hops(transition_node(&model, &graph, "B: b1 → b0")), Some(3));

    let one = graph.cone(&[a_go], Direction::Forward, Some(1));
    assert_eq!(
        labels(&model, &graph, one.nodes().map(|(n, _)| n)),
        ["A: a0 → a1", "B: b0 → b1", "C1 on Go", "C2 on Done", "Done", "Go"]
    );
    // Edges leaving the cone are excluded; edges inside are included.
    for (e, edge) in graph.edges() {
        let inside = one.contains(edge.from) && one.contains(edge.to);
        assert_eq!(one.contains_edge(e), inside, "edge {e:?}");
    }

    let zero = graph.cone(&[a_go], Direction::Forward, Some(0));
    assert_eq!(labels(&model, &graph, zero.nodes().map(|(n, _)| n)), ["A: a0 → a1", "C1 on Go", "Go"]);
}

#[test]
fn backward_cone_reaches_external_sources() {
    let model = load(CHAIN);
    let graph = CausalGraph::build(&model);
    let d_note = transition_node(&model, &graph, "D: d0 → d1");
    let cone = graph.cone(&[d_note], Direction::Backward, None);
    assert_eq!(
        labels(&model, &graph, cone.nodes().map(|(n, _)| n)),
        ["A: a0 → a1", "B: b0 → b1", "C1 on Go", "C2 on Done", "D: d0 → d1", "Done", "Go", "User"]
    );
    let user = graph.nodes().find(|(_, n)| matches!(n, CausalNode::External(_))).map(|(ix, _)| ix).expect("User");
    assert_eq!(cone.hops(user), Some(3));
}

#[test]
fn cycles_terminate_and_include_the_back_edge() {
    let model = load(CHAIN);
    let graph = CausalGraph::build(&model);
    // A:a1→a0 emits Back → C3 fires B:b1→b0, which emits nothing; but
    // B:b0→b1 → Done → C2 → A:a1→a0 → Back → C3 → B:b1→b0 is not a cycle.
    // Add one by asking for the cone of b0→b1 in both directions.
    let b_start = transition_node(&model, &graph, "B: b0 → b1");
    let forward = graph.cone(&[b_start], Direction::Forward, None);
    let backward = graph.cone(&[b_start], Direction::Backward, None);
    assert!(forward.contains(b_start) && backward.contains(b_start));
    assert!(!forward.contains(transition_node(&model, &graph, "A: a0 → a1")));
}

#[test]
fn self_triggering_cycle() {
    let model = load(
        r#"
machines:
  R:
    states: [s]
    transitions: [{ from: s, to: s, on: retry, emits: [Failed] }]
controllers:
  Retrier:
    on:
      Failed: [{ fire: R.retry }]
"#,
    );
    let graph = CausalGraph::build(&model);
    let t = transition_node(&model, &graph, "R: s → s");
    let cone = graph.cone(&[t], Direction::Forward, None);
    assert_eq!(cone.hops(t), Some(0));
    assert_eq!(cone.edges().count(), 3, "emit, subscribe and the fire back to itself");
    assert_eq!(graph.transition_successors(model.transition_ids().next().expect("t")).len(), 1);
}

#[test]
fn paths_between_two_transitions() {
    let model = load(CHAIN);
    let graph = CausalGraph::build(&model);
    let a_go = transition_node(&model, &graph, "A: a0 → a1");
    let d_note = transition_node(&model, &graph, "D: d0 → d1");
    let expected = ["A: a0 → a1", "B: b0 → b1", "C1 on Go", "C2 on Done", "D: d0 → d1", "Done", "Go"];

    let paths = graph.paths_between(a_go, d_note);
    assert_eq!(labels(&model, &graph, paths.nodes().map(|(n, _)| n)), expected);
    assert_eq!(paths.edges().count(), 6);

    // Order of selection does not matter.
    let reversed = graph.paths_between(d_note, a_go);
    assert_eq!(labels(&model, &graph, reversed.nodes().map(|(n, _)| n)), expected);

    // Unrelated transitions have no paths.
    let b_rewind = transition_node(&model, &graph, "B: b1 → b0");
    assert!(graph.paths_between(d_note, b_rewind).is_empty());
}
