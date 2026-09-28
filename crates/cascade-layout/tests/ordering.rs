//! Crossing minimisation on small graphs with known optima.

mod common;

use cascade_layout::metrics::{count_crossings, count_order_crossings};
use cascade_layout::{EdgeEnd, LayoutEdge, LayoutGraph, LayoutHints, LayoutNode, LayoutOptions, Port, PortSide, Size};
use common::*;

#[test]
fn crossed_matching_is_uncrossed() {
    let mut g = LayoutGraph::new();
    let a = node(&mut g, "a", 40.0, 20.0);
    let b = node(&mut g, "b", 40.0, 20.0);
    let c = node(&mut g, "c", 40.0, 20.0);
    let d = node(&mut g, "d", 40.0, 20.0);
    edge(&mut g, a, d);
    edge(&mut g, b, c);
    let r = run(&g);
    assert_ok(&g, &LayoutOptions::default(), &LayoutHints::default(), &r);
    assert_eq!(count_order_crossings(&g, &r), 0);
    assert_eq!(count_crossings(&g, &r), 0);
}

#[test]
fn complete_bipartite_graphs_hit_their_minimum() {
    // K(m,n) drawn on two layers always has C(m,2) * C(n,2) crossings.
    for (m, n, expected) in [(2usize, 2usize, 1usize), (2, 3, 3), (3, 3, 9)] {
        let mut g = LayoutGraph::new();
        let top: Vec<_> = (0..m).map(|i| node(&mut g, &format!("t{i}"), 30.0, 20.0)).collect();
        let bottom: Vec<_> = (0..n).map(|i| node(&mut g, &format!("b{i}"), 30.0, 20.0)).collect();
        for t in &top {
            for b in &bottom {
                edge(&mut g, *t, *b);
            }
        }
        let r = run(&g);
        assert_ok(&g, &LayoutOptions::default(), &LayoutHints::default(), &r);
        assert_eq!(count_order_crossings(&g, &r), expected, "K({m},{n})");
    }
}

#[test]
fn scrambled_tree_is_drawn_without_crossings() {
    // A binary tree of depth 3, with nodes and edges inserted in a scrambled
    // order so the initial ordering is poor.
    let mut g = LayoutGraph::new();
    let order = [9, 3, 12, 0, 14, 7, 1, 11, 5, 13, 2, 8, 4, 10, 6];
    let mut ids = vec![None; 15];
    for &i in &order {
        ids[i] = Some(node(&mut g, &format!("n{i}"), 30.0, 16.0));
    }
    let ids: Vec<_> = ids.into_iter().map(|x| x.expect("all nodes")).collect();
    let mut edges: Vec<(usize, usize)> = (1..15).map(|i| ((i - 1) / 2, i)).collect();
    edges.reverse();
    edges.swap(0, 7);
    edges.swap(3, 11);
    for (p, c) in edges {
        edge(&mut g, ids[p], ids[c]);
    }
    let r = run(&g);
    assert_ok(&g, &LayoutOptions::default(), &LayoutHints::default(), &r);
    assert_eq!(count_order_crossings(&g, &r), 0);
    assert_eq!(count_crossings(&g, &r), 0);
}

#[test]
fn long_edges_are_untangled() {
    // Two long edges that would cross if their dummies were ordered by
    // insertion.
    let mut g = LayoutGraph::new();
    let s1 = node(&mut g, "s1", 30.0, 20.0);
    let s2 = node(&mut g, "s2", 30.0, 20.0);
    let m1 = node(&mut g, "m1", 30.0, 20.0);
    let m2 = node(&mut g, "m2", 30.0, 20.0);
    let t1 = node(&mut g, "t1", 30.0, 20.0);
    let t2 = node(&mut g, "t2", 30.0, 20.0);
    edge(&mut g, s1, m1);
    edge(&mut g, m1, t1);
    edge(&mut g, s2, m2);
    edge(&mut g, m2, t2);
    edge(&mut g, s1, t2);
    edge(&mut g, s2, t1);
    let r = run(&g);
    assert_ok(&g, &LayoutOptions::default(), &LayoutHints::default(), &r);
    assert!(count_crossings(&g, &r) <= 1, "{}", count_crossings(&g, &r));
}

#[test]
fn port_order_drives_neighbour_order() {
    // `hub` has two East ports: port 0 (upper) feeds `x`, port 1 (lower)
    // feeds `y`. x and y are inserted in the opposite order.
    let mut g = LayoutGraph::new();
    let hub = g
        .add_node(
            LayoutNode::new("hub", Size::new(40.0, 60.0))
                .with_ports(vec![Port { side: PortSide::East }, Port { side: PortSide::East }]),
        )
        .expect("hub");
    let y = node(&mut g, "y", 30.0, 20.0);
    let x = node(&mut g, "x", 30.0, 20.0);
    g.add_edge(LayoutEdge::new(EdgeEnd::port(hub, 1), EdgeEnd::node(y))).expect("e");
    g.add_edge(LayoutEdge::new(EdgeEnd::port(hub, 0), EdgeEnd::node(x))).expect("e");
    let r = run(&g);
    assert_ok(&g, &LayoutOptions::default(), &LayoutHints::default(), &r);
    assert!(r.node(x).order < r.node(y).order);
    assert!(r.node(x).rect.top() < r.node(y).rect.top());
    assert_eq!(count_crossings(&g, &r), 0);
}

#[test]
fn many_parallel_chains_stay_uncrossed() {
    let mut g = LayoutGraph::new();
    let mut prev: Vec<_> = (0..6).map(|i| node(&mut g, &format!("c{i}_0"), 30.0, 20.0)).collect();
    for step in 1..5 {
        let next: Vec<_> = (0..6).map(|i| node(&mut g, &format!("c{i}_{step}"), 30.0, 20.0)).collect();
        // Insert edges in a rotated order each step.
        for k in 0..6 {
            let i = (k + step * 2) % 6;
            edge(&mut g, prev[i], next[i]);
        }
        prev = next;
    }
    let r = run(&g);
    assert_ok(&g, &LayoutOptions::default(), &LayoutHints::default(), &r);
    assert_eq!(count_crossings(&g, &r), 0);
}
