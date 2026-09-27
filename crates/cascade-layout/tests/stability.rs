//! Stability: feeding the previous layout back means an edit that adds or
//! removes one node or edge moves no unrelated node.

mod common;

use std::collections::BTreeSet;

use cascade_layout::{LayoutGraph, LayoutHints, LayoutOptions, LayoutResult};
use common::*;

fn base() -> Spec {
    Spec::new()
        .node("a", 60.0, 24.0)
        .node("b", 60.0, 24.0)
        .node("c", 80.0, 30.0)
        .node("d", 60.0, 24.0)
        .node("e", 50.0, 24.0)
        .node("f", 60.0, 40.0)
        .node("g", 60.0, 24.0)
        .node("h", 60.0, 24.0)
        .node("i", 70.0, 24.0)
        .node("j", 60.0, 24.0)
        .edge("a", "b")
        .edge("b", "c")
        .edge("c", "d")
        .edge("a", "e")
        .edge("e", "f")
        .edge("f", "d")
        .edge("g", "h")
        .edge("h", "i")
        .edge("g", "i")
        .edge("d", "j")
        .edge("j", "a")
}

fn relayout(before: &Spec, after: &Spec) -> (LayoutGraph, LayoutResult, LayoutGraph, LayoutResult) {
    let g1 = before.build();
    let r1 = run(&g1);
    let g2 = after.build();
    let hints = LayoutHints { previous: Some(r1.to_previous(&g1)), ..LayoutHints::default() };
    let r2 = run_with(&g2, &LayoutOptions::default(), &hints);
    let checks = Checks { labels: false, ..Checks::ALL };
    if let Err(msg) = check(&g2, &LayoutOptions::default(), &hints, &r2, checks) {
        panic!("relayout invariant violated: {msg}");
    }
    (g1, r1, g2, r2)
}

/// Keys touched by the edit: changed nodes plus the endpoints of changed
/// edges.
fn touched(before: &Spec, after: &Spec) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let nodes_a: BTreeSet<_> = before.nodes.iter().map(|n| n.0.clone()).collect();
    let nodes_b: BTreeSet<_> = after.nodes.iter().map(|n| n.0.clone()).collect();
    out.extend(nodes_a.symmetric_difference(&nodes_b).cloned());
    let edges_a: BTreeSet<_> = before.edges.iter().cloned().collect();
    let edges_b: BTreeSet<_> = after.edges.iter().cloned().collect();
    for (x, y) in edges_a.symmetric_difference(&edges_b) {
        out.insert(x.clone());
        out.insert(y.clone());
    }
    out
}

fn assert_unrelated_unmoved(before: &Spec, after: &Spec) {
    let (g1, r1, g2, r2) = relayout(before, after);
    let touched = touched(before, after);
    for (id, n) in g2.nodes() {
        if touched.contains(&n.key) {
            continue;
        }
        let Some(old) = g1.node_by_key(&n.key) else { continue };
        assert_eq!(r1.node(old).rect, r2.node(id).rect, "unrelated node {} moved", n.key);
        assert_eq!(r1.node(old).layer, r2.node(id).layer, "unrelated node {} changed layer", n.key);
    }
}

#[test]
fn unchanged_graph_relayouts_identically() {
    let spec = base();
    let (g1, r1, g2, r2) = relayout(&spec, &spec);
    for (id, n) in g2.nodes() {
        let old = g1.node_by_key(&n.key).expect("same keys");
        assert_eq!(r1.node(old), r2.node(id));
    }
}

#[test]
fn adding_a_node_and_edge_moves_nothing_else() {
    let before = base();
    assert_unrelated_unmoved(&before, &base().node("new", 60.0, 24.0).edge("c", "new"));
    assert_unrelated_unmoved(&before, &base().node("new", 60.0, 24.0).edge("new", "h"));
    assert_unrelated_unmoved(&before, &base().node("wide", 120.0, 50.0).edge("b", "wide").edge("wide", "d"));
}

#[test]
fn adding_an_edge_moves_nothing_else() {
    let before = base();
    assert_unrelated_unmoved(&before, &base().edge("b", "i"));
    assert_unrelated_unmoved(&before, &base().edge("i", "a"));
    assert_unrelated_unmoved(&before, &base().edge("a", "d"));
}

#[test]
fn removing_a_node_or_edge_moves_nothing_else() {
    let before = base();
    assert_unrelated_unmoved(&before, &base().without_node("e"));
    assert_unrelated_unmoved(&before, &base().without_node("h"));
    assert_unrelated_unmoved(&before, &base().without_edge("c", "d"));
    assert_unrelated_unmoved(&before, &base().without_edge("j", "a"));
}

#[test]
fn a_sequence_of_edits_stays_stable() {
    let mut spec = base();
    let mut g = spec.build();
    let mut r = run(&g);
    let edits: Vec<Box<dyn Fn(Spec) -> Spec>> = vec![
        Box::new(|s| s.node("n1", 60.0, 24.0).edge("d", "n1")),
        Box::new(|s| s.node("n2", 40.0, 24.0).edge("n1", "n2")),
        Box::new(|s| s.without_node("f")),
        Box::new(|s| s.edge("n2", "g")),
        Box::new(|s| s.without_edge("g", "i")),
        Box::new(|s| s.node("n3", 90.0, 60.0).edge("a", "n3").edge("n3", "b")),
    ];
    for edit in edits {
        let next = edit(Spec { groups: spec.groups.clone(), nodes: spec.nodes.clone(), edges: spec.edges.clone() });
        let touched = touched(&spec, &next);
        let g2 = next.build();
        let hints = LayoutHints { previous: Some(r.to_previous(&g)), ..LayoutHints::default() };
        let r2 = run_with(&g2, &LayoutOptions::default(), &hints);
        let checks = Checks { labels: false, ..Checks::ALL };
        if let Err(msg) = check(&g2, &LayoutOptions::default(), &hints, &r2, checks) {
            panic!("{msg}");
        }
        for (id, n) in g2.nodes() {
            if touched.contains(&n.key) {
                continue;
            }
            if let Some(old) = g.node_by_key(&n.key) {
                assert_eq!(r.node(old).rect, r2.node(id).rect, "node {} moved", n.key);
            }
        }
        spec = next;
        g = g2;
        r = r2;
    }
}

fn lanes() -> Spec {
    Spec::new()
        .group("A")
        .group("B")
        .group("C")
        .node_in("a1", 60.0, 24.0, 0)
        .node_in("a2", 60.0, 24.0, 0)
        .node_in("a3", 60.0, 24.0, 0)
        .node_in("b1", 60.0, 24.0, 1)
        .node_in("b2", 60.0, 24.0, 1)
        .node_in("c1", 60.0, 24.0, 2)
        .node_in("c2", 60.0, 24.0, 2)
        .node_in("c3", 60.0, 24.0, 2)
        .edge("a1", "a2")
        .edge("a2", "a3")
        .edge("b1", "b2")
        .edge("c1", "c2")
        .edge("c2", "c3")
        .edge("a2", "b1")
        .edge("b2", "c2")
        .edge("c3", "a1")
}

#[test]
fn grouped_edits_move_nothing_else() {
    let before = lanes();
    assert_unrelated_unmoved(&before, &lanes().node_in("b3", 60.0, 24.0, 1).edge("b2", "b3"));
    assert_unrelated_unmoved(&before, &lanes().edge("a3", "c1"));
    assert_unrelated_unmoved(&before, &lanes().without_edge("a2", "b1"));
    assert_unrelated_unmoved(&before, &lanes().without_node("c3"));
}

#[test]
fn previous_order_is_kept_for_existing_nodes() {
    let before = base();
    let after = base().node("x", 60.0, 24.0).edge("a", "x").edge("x", "d");
    let (g1, r1, g2, r2) = relayout(&before, &after);
    for (a, na) in g2.nodes() {
        for (b, nb) in g2.nodes() {
            let (Some(oa), Some(ob)) = (g1.node_by_key(&na.key), g1.node_by_key(&nb.key)) else { continue };
            if r1.node(oa).layer == r1.node(ob).layer && r2.node(a).layer == r2.node(b).layer {
                assert_eq!(
                    r1.node(oa).order < r1.node(ob).order,
                    r2.node(a).order < r2.node(b).order,
                    "relative order of {} and {} changed",
                    na.key,
                    nb.key
                );
            }
        }
    }
}

#[test]
fn adding_a_transition_with_its_event_and_handler_moves_nothing_else() {
    // A causal-view style edit: one new transition brings a new event and
    // handler with it, wired into existing nodes.
    let before = base();
    let after = base()
        .node("t_new", 140.0, 36.0)
        .node("e_new", 70.0, 24.0)
        .node("h_new", 90.0, 24.0)
        .edge("b", "t_new")
        .edge("t_new", "e_new")
        .edge("e_new", "h_new")
        .edge("h_new", "i");
    assert_unrelated_unmoved(&before, &after);
    // And taking it out again.
    assert_unrelated_unmoved(&after, &before);
}

fn big() -> Spec {
    let mut rng = Lcg::new(99);
    let mut s = Spec::new();
    let n = 300;
    for i in 0..n {
        s = s.node(&format!("v{i}"), 50.0 + rng.below(80) as f32, 20.0 + rng.below(20) as f32);
    }
    let mut added = 0;
    while added < 450 {
        let a = rng.below(n) as usize;
        let b = if rng.chance(5) {
            a.saturating_sub(1 + rng.below(20) as usize)
        } else {
            (a + 1 + rng.below(12) as usize).min(n as usize - 1)
        };
        if a == b {
            continue;
        }
        s = s.edge(&format!("v{a}"), &format!("v{b}"));
        added += 1;
    }
    s
}

fn big_clone() -> Spec {
    let b = big();
    Spec { groups: b.groups.clone(), nodes: b.nodes.clone(), edges: b.edges.clone() }
}

#[test]
fn edits_to_a_large_graph_move_nothing_else() {
    let before = big();
    assert_unrelated_unmoved(
        &before,
        &big_clone().node("extra", 90.0, 30.0).edge("v120", "extra").edge("extra", "v140"),
    );
    assert_unrelated_unmoved(&before, &big_clone().without_node("v77"));
    assert_unrelated_unmoved(&before, &big_clone().edge("v10", "v250"));
}
