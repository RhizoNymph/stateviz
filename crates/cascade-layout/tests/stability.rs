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

/// A machine for [`shop_lanes`]: its states and `(from, to)` transitions.
struct Machine {
    name: &'static str,
    states: Vec<&'static str>,
    transitions: Vec<(&'static str, &'static str)>,
}

/// The structure view's shape: one lane per machine, states and transition
/// pills (with West/East/North/South ports) inside, and fire edges from a
/// pill's South port to another lane's pill North port, between adjacent
/// and non-adjacent lanes.
fn shop_lanes(extra_shipment_transition: bool) -> cascade_layout::LayoutGraph {
    use cascade_layout::{EdgeEnd, Insets, LayoutEdge, LayoutGroup, LayoutNode, Port, PortSide, Size};
    let lanes: Vec<Machine> = vec![
        Machine {
            name: "Order",
            states: vec!["cart", "placed", "paid", "shipped", "delivered", "cancelled"],
            transitions: vec![
                ("cart", "placed"),
                ("placed", "paid"),
                ("paid", "shipped"),
                ("shipped", "delivered"),
                ("placed", "cancelled"),
                ("paid", "cancelled"),
            ],
        },
        Machine {
            name: "Payment",
            states: vec!["created", "authorizing", "authorized", "captured", "voided", "failed", "refunded"],
            transitions: vec![
                ("created", "authorizing"),
                ("authorizing", "authorized"),
                ("authorizing", "failed"),
                ("authorized", "captured"),
                ("authorized", "failed"),
                ("authorized", "voided"),
                ("captured", "refunded"),
                ("failed", "created"),
            ],
        },
        Machine {
            name: "Inventory",
            states: vec!["requested", "reserved", "backordered", "committed"],
            transitions: vec![
                ("requested", "reserved"),
                ("requested", "backordered"),
                ("backordered", "reserved"),
                ("reserved", "committed"),
            ],
        },
        Machine {
            name: "Shipment",
            states: vec!["idle", "packing", "in_transit", "delivered", "lost"],
            transitions: vec![
                ("idle", "packing"),
                ("packing", "in_transit"),
                ("in_transit", "in_transit"),
                ("in_transit", "delivered"),
                ("in_transit", "lost"),
            ],
        },
        Machine {
            name: "Notification",
            states: vec!["queued", "sending", "delivered", "bounced"],
            transitions: vec![("queued", "sending"), ("sending", "delivered"), ("sending", "bounced")],
        },
    ];
    let mut g = cascade_layout::LayoutGraph::new();
    for Machine { name: lane, states, transitions } in &lanes {
        let group = g.add_group(LayoutGroup { key: (*lane).to_string(), padding: Insets::uniform(10.0), header: 24.0 });
        for s in states {
            let key = format!("{lane}:{s}");
            g.add_node(LayoutNode::new(key.clone(), Size::new(40.0 + 8.0 * s.len() as f32, 29.0)).in_group(group))
                .expect("state");
        }
        let mut transitions = transitions.clone();
        if extra_shipment_transition && *lane == "Shipment" {
            transitions.insert(0, ("packing", "lost"));
        }
        for (from, to) in transitions {
            let key = format!("{lane}:{from}->{to}");
            let sides = [PortSide::West, PortSide::East, PortSide::North, PortSide::South];
            let pill = g
                .add_node(
                    LayoutNode::new(key.clone(), Size::new(60.0 + 8.0 * (from.len() + to.len()) as f32, 43.0))
                        .in_group(group)
                        .with_ports(sides.iter().map(|&side| Port { side }).collect()),
                )
                .expect("pill");
            let s = g.node_by_key(&format!("{lane}:{from}")).expect("from");
            let t = g.node_by_key(&format!("{lane}:{to}")).expect("to");
            g.add_edge(LayoutEdge::new(EdgeEnd::node(s), EdgeEnd::port(pill, 0))).expect("in");
            g.add_edge(LayoutEdge::new(EdgeEnd::port(pill, 1), EdgeEnd::node(t))).expect("out");
        }
    }
    for (a, b) in [
        ("Order:cart->placed", "Payment:created->authorizing"),
        ("Payment:authorizing->authorized", "Order:placed->paid"),
        ("Order:placed->paid", "Inventory:requested->reserved"),
        ("Inventory:reserved->committed", "Shipment:idle->packing"),
        ("Shipment:packing->in_transit", "Notification:queued->sending"),
        ("Shipment:in_transit->delivered", "Order:shipped->delivered"),
        ("Payment:captured->refunded", "Notification:queued->sending"),
        ("Shipment:in_transit->lost", "Payment:captured->refunded"),
        ("Order:paid->cancelled", "Payment:authorized->voided"),
        ("Order:shipped->delivered", "Notification:sending->delivered"),
    ] {
        let a = g.node_by_key(a).expect("fire source");
        let b = g.node_by_key(b).expect("fire target");
        g.add_edge(LayoutEdge::new(EdgeEnd::port(a, 3), EdgeEnd::port(b, 2))).expect("fire");
    }
    g
}

#[test]
fn an_edit_inside_one_lane_moves_no_node_in_another_lane() {
    let options = LayoutOptions { layer_spacing: 48.0, ..LayoutOptions::default() };
    let before = shop_lanes(false);
    let r1 = run_with(&before, &options, &LayoutHints::default());
    let after = shop_lanes(true);
    let hints = LayoutHints { previous: Some(r1.to_previous(&before)), ..LayoutHints::default() };
    let r2 = run_with(&after, &options, &hints);
    if let Err(msg) = check(&after, &options, &hints, &r2, Checks { labels: false, ..Checks::ALL }) {
        panic!("relayout invariant violated: {msg}");
    }
    let mut moved = Vec::new();
    for (id, n) in after.nodes() {
        if n.key.starts_with("Shipment:") {
            continue;
        }
        let old = before.node_by_key(&n.key).expect("existing node");
        if r1.node(old).rect != r2.node(id).rect {
            moved.push(format!("{}: {:?} -> {:?}", n.key, r1.node(old).rect, r2.node(id).rect));
        }
    }
    assert!(moved.is_empty(), "nodes outside the edited lane moved:\n{}", moved.join("\n"));
    // Removing the transition again restores every other lane too.
    let hints = LayoutHints { previous: Some(r2.to_previous(&after)), ..LayoutHints::default() };
    let r3 = run_with(&before, &options, &hints);
    for (id, n) in before.nodes() {
        if !n.key.starts_with("Shipment:") {
            assert_eq!(r1.node(id).rect, r3.node(id).rect, "{} moved after the removal", n.key);
        }
    }
}

#[test]
fn relaying_out_unchanged_lanes_keeps_them_exactly() {
    // No edit at all: every node, every route and every group rect comes
    // back identical.
    let options = LayoutOptions { layer_spacing: 48.0, ..LayoutOptions::default() };
    let g = shop_lanes(false);
    let r1 = run_with(&g, &options, &LayoutHints::default());
    let hints = LayoutHints { previous: Some(r1.to_previous(&g)), ..LayoutHints::default() };
    let r2 = run_with(&g, &options, &hints);
    for (id, n) in g.nodes() {
        assert_eq!(r1.node(id).rect, r2.node(id).rect, "{} moved", n.key);
    }
    for (gid, group) in g.groups() {
        assert_eq!(r1.group(gid), r2.group(gid), "group {} changed", group.key);
    }
}

#[test]
fn new_cross_lane_edges_move_only_their_ends() {
    use cascade_layout::{EdgeEnd, LayoutEdge};
    let options = LayoutOptions { layer_spacing: 48.0, ..LayoutOptions::default() };
    let before = shop_lanes(false);
    let r1 = run_with(&before, &options, &LayoutHints::default());
    // Three new fire edges: two into the Order/Payment gap, one skipping to
    // Inventory through the corridor, so gaps gain tracks.
    let added = [
        ("Order:cart->placed", "Payment:authorized->captured"),
        ("Order:placed->cancelled", "Payment:authorizing->failed"),
        ("Order:paid->shipped", "Inventory:requested->backordered"),
    ];
    let mut after = shop_lanes(false);
    for (a, b) in added {
        let a = after.node_by_key(a).expect("source");
        let b = after.node_by_key(b).expect("target");
        after.add_edge(LayoutEdge::new(EdgeEnd::port(a, 3), EdgeEnd::port(b, 2))).expect("fire");
    }
    let hints = LayoutHints { previous: Some(r1.to_previous(&before)), ..LayoutHints::default() };
    let r2 = run_with(&after, &options, &hints);
    if let Err(msg) = check(&after, &options, &hints, &r2, Checks { labels: false, ..Checks::ALL }) {
        panic!("relayout invariant violated: {msg}");
    }
    let touched: BTreeSet<&str> = added.iter().flat_map(|(a, b)| [*a, *b]).collect();
    for (id, n) in after.nodes() {
        if !touched.contains(n.key.as_str()) {
            assert_eq!(r1.node(id).rect, r2.node(id).rect, "{} moved", n.key);
        }
    }
}

#[test]
fn a_new_node_without_neighbours_takes_the_top_of_its_empty_column() {
    use cascade_layout::{Insets, LayerConstraint, LayoutGroup, LayoutNode, Size};
    let build = |with_new: bool| {
        let mut g = LayoutGraph::new();
        let lanes: Vec<_> = ["A", "B", "C"]
            .iter()
            .map(|k| g.add_group(LayoutGroup { key: (*k).to_string(), padding: Insets::uniform(8.0), header: 20.0 }))
            .collect();
        for (i, lane) in lanes.iter().enumerate() {
            let ids: Vec<_> = (0..3)
                .map(|j| {
                    g.add_node(LayoutNode::new(format!("n{i}_{j}"), Size::new(60.0, 24.0)).in_group(*lane))
                        .expect("node")
                })
                .collect();
            for w in ids.windows(2) {
                edge(&mut g, w[0], w[1]);
            }
        }
        if with_new {
            let x =
                LayoutNode::new("x", Size::new(60.0, 24.0)).in_group(lanes[1]).with_layer(LayerConstraint::Exact(3));
            g.add_node(x).expect("x");
        }
        g
    };
    let before = build(false);
    let r1 = run(&before);
    let after = build(true);
    let hints = LayoutHints { previous: Some(r1.to_previous(&before)), ..LayoutHints::default() };
    let r2 = run_with(&after, &LayoutOptions::default(), &hints);
    assert_ok(&after, &LayoutOptions::default(), &hints, &r2);
    for (id, n) in before.nodes() {
        let now = after.node_by_key(&n.key).expect("kept");
        assert_eq!(r1.node(id).rect, r2.node(now).rect, "{} moved", n.key);
    }
    // The new node sits level with its lane's row, not below it.
    let x = rect_of(&after, &r2, "x");
    let row = rect_of(&after, &r2, "n1_2");
    assert!((x.top() - row.top()).abs() < 1.0, "{x:?} vs {row:?}");
}

fn aligned() -> LayoutOptions {
    LayoutOptions { layer_spacing: 48.0, align_across_groups: true, ..LayoutOptions::default() }
}

#[test]
fn aligned_lanes_come_back_exactly() {
    let g = shop_lanes(false);
    let r1 = run_with(&g, &aligned(), &LayoutHints::default());
    assert_ok(&g, &aligned(), &LayoutHints::default(), &r1);
    let hints = LayoutHints { previous: Some(r1.to_previous(&g)), ..LayoutHints::default() };
    let r2 = run_with(&g, &aligned(), &hints);
    for (id, n) in g.nodes() {
        assert_eq!(r1.node(id).rect, r2.node(id).rect, "{} moved", n.key);
    }
    for (gid, group) in g.groups() {
        assert_eq!(r1.group(gid), r2.group(gid), "group {} changed", group.key);
    }
    for (e, _) in g.edges() {
        assert_eq!(r1.edge(e).points, r2.edge(e).points, "route of edge {} changed", e.index());
    }
}

#[test]
fn an_edit_inside_one_aligned_lane_moves_no_node_in_another_lane() {
    let before = shop_lanes(false);
    let r1 = run_with(&before, &aligned(), &LayoutHints::default());
    let after = shop_lanes(true);
    let hints = LayoutHints { previous: Some(r1.to_previous(&before)), ..LayoutHints::default() };
    let r2 = run_with(&after, &aligned(), &hints);
    if let Err(msg) = check(&after, &aligned(), &hints, &r2, Checks { labels: false, ..Checks::ALL }) {
        panic!("relayout invariant violated: {msg}");
    }
    for (id, n) in after.nodes() {
        if n.key.starts_with("Shipment:") {
            continue;
        }
        let old = before.node_by_key(&n.key).expect("existing node");
        assert_eq!(r1.node(old).rect, r2.node(id).rect, "{} moved", n.key);
    }
}
