//! Core geometry: layers run left to right, nodes never overlap, spacing and
//! sizes are honoured, routes are orthogonal and attach on the flow-facing
//! sides.

mod common;

use cascade_layout::{LayoutGraph, LayoutHints, LayoutOptions, Rect};
use common::*;

#[test]
fn empty_graph_lays_out_to_nothing() {
    let g = LayoutGraph::new();
    let r = run(&g);
    assert_eq!(r.node_count(), 0);
    assert_eq!(r.bounds, Rect::default());
}

#[test]
fn single_node_keeps_its_size() {
    let mut g = LayoutGraph::new();
    let a = node(&mut g, "a", 80.0, 30.0);
    let r = run(&g);
    assert_eq!(r.node(a).rect.size.width, 80.0);
    assert_eq!(r.node(a).rect.size.height, 30.0);
    assert_eq!(r.node(a).layer, 0);
    assert_eq!(r.node(a).order, 0);
    assert!(r.bounds.contains(r.node(a).rect.origin));
}

#[test]
fn chain_runs_left_to_right_one_layer_per_node() {
    let mut g = LayoutGraph::new();
    let ids: Vec<_> = (0..5).map(|i| node(&mut g, &format!("n{i}"), 60.0, 24.0)).collect();
    for w in ids.windows(2) {
        edge(&mut g, w[0], w[1]);
    }
    let r = run(&g);
    assert_ok(&g, &LayoutOptions::default(), &LayoutHints::default(), &r);
    for (i, id) in ids.iter().enumerate() {
        assert_eq!(r.node(*id).layer, i as u32);
    }
    for w in ids.windows(2) {
        let (a, b) = (r.node(w[0]).rect, r.node(w[1]).rect);
        assert!(b.left() >= a.right() + LayoutOptions::default().layer_spacing - EPS, "{a:?} {b:?}");
    }
    // A straight chain of equal nodes is drawn straight.
    for (e, _) in g.edges() {
        let route = r.edge(e);
        assert_eq!(route.points.len(), 2, "{:?}", route.points);
        assert!(!route.reversed);
    }
}

#[test]
fn forward_edges_go_left_to_right_monotonically() {
    let mut g = LayoutGraph::new();
    let a = node(&mut g, "a", 40.0, 20.0);
    let b = node(&mut g, "b", 40.0, 60.0);
    let c = node(&mut g, "c", 90.0, 20.0);
    let d = node(&mut g, "d", 40.0, 20.0);
    let e = node(&mut g, "e", 40.0, 20.0);
    for (x, y) in [(a, b), (a, c), (b, d), (c, d), (a, d), (a, e), (e, d)] {
        edge(&mut g, x, y);
    }
    let r = run(&g);
    assert_ok(&g, &LayoutOptions::default(), &LayoutHints::default(), &r);
    for (eid, _) in g.edges() {
        let route = r.edge(eid);
        assert!(!route.reversed);
        for w in route.points.windows(2) {
            assert!(w[1].x >= w[0].x - EPS, "edge {} goes backwards: {:?}", eid.index(), route.points);
        }
    }
}

#[test]
fn nodes_in_a_layer_are_separated_by_node_spacing() {
    let mut g = LayoutGraph::new();
    let root = node(&mut g, "root", 40.0, 20.0);
    let kids: Vec<_> = (0..4).map(|i| node(&mut g, &format!("k{i}"), 50.0, 10.0 + 10.0 * i as f32)).collect();
    for k in &kids {
        edge(&mut g, root, *k);
    }
    let options = LayoutOptions::default();
    let r = run(&g);
    assert_ok(&g, &options, &LayoutHints::default(), &r);
    let mut rects: Vec<Rect> = kids.iter().map(|k| r.node(*k).rect).collect();
    rects.sort_by(|a, b| a.top().total_cmp(&b.top()));
    for w in rects.windows(2) {
        assert!(w[1].top() - w[0].bottom() >= options.node_spacing - EPS, "{:?}", rects);
    }
    let orders: Vec<u32> = kids.iter().map(|k| r.node(*k).order).collect();
    let mut sorted = orders.clone();
    sorted.sort_unstable();
    assert_eq!(sorted, vec![0, 1, 2, 3]);
}

#[test]
fn layers_are_as_wide_as_their_widest_node() {
    let mut g = LayoutGraph::new();
    let a = node(&mut g, "a", 40.0, 20.0);
    let wide = node(&mut g, "wide", 200.0, 20.0);
    let narrow = node(&mut g, "narrow", 20.0, 20.0);
    let z = node(&mut g, "z", 40.0, 20.0);
    edge(&mut g, a, wide);
    edge(&mut g, a, narrow);
    edge(&mut g, wide, z);
    edge(&mut g, narrow, z);
    let options = LayoutOptions::default();
    let r = run(&g);
    assert_ok(&g, &options, &LayoutHints::default(), &r);
    let (w, n, zr) = (r.node(wide).rect, r.node(narrow).rect, r.node(z).rect);
    assert!(zr.left() >= w.right() + options.layer_spacing - EPS);
    assert!(zr.left() >= n.right() + options.layer_spacing - EPS);
    // Nodes in one layer share a centre line.
    assert!((w.center().x - n.center().x).abs() < EPS);
}

#[test]
fn spacing_options_are_honoured() {
    let mut g = LayoutGraph::new();
    let a = node(&mut g, "a", 40.0, 20.0);
    let b = node(&mut g, "b", 40.0, 20.0);
    let c = node(&mut g, "c", 40.0, 20.0);
    edge(&mut g, a, b);
    edge(&mut g, a, c);
    let options = LayoutOptions { node_spacing: 50.0, layer_spacing: 120.0, ..LayoutOptions::default() };
    let r = run_with(&g, &options, &LayoutHints::default());
    assert_ok(&g, &options, &LayoutHints::default(), &r);
    let (ra, rb, rc) = (r.node(a).rect, r.node(b).rect, r.node(c).rect);
    assert!(rb.left() - ra.right() >= 120.0 - EPS);
    let (upper, lower) = if rb.top() < rc.top() { (rb, rc) } else { (rc, rb) };
    assert!(lower.top() - upper.bottom() >= 50.0 - EPS);
}

#[test]
fn disconnected_components_do_not_overlap() {
    let mut g = LayoutGraph::new();
    for c in 0..4 {
        let a = node(&mut g, &format!("a{c}"), 30.0, 20.0);
        let b = node(&mut g, &format!("b{c}"), 30.0, 20.0);
        edge(&mut g, a, b);
    }
    for i in 0..3 {
        node(&mut g, &format!("lonely{i}"), 25.0, 25.0);
    }
    let r = run(&g);
    assert_ok(&g, &LayoutOptions::default(), &LayoutHints::default(), &r);
}

#[test]
fn parallel_edges_get_distinct_routes() {
    let mut g = LayoutGraph::new();
    let a = node(&mut g, "a", 40.0, 40.0);
    let b = node(&mut g, "b", 40.0, 40.0);
    let e1 = edge(&mut g, a, b);
    let e2 = edge(&mut g, a, b);
    let r = run(&g);
    assert_ok(&g, &LayoutOptions::default(), &LayoutHints::default(), &r);
    assert_ne!(r.edge(e1).points[0], r.edge(e2).points[0]);
}

#[test]
fn zero_sized_nodes_are_fine() {
    let mut g = LayoutGraph::new();
    let a = node(&mut g, "a", 0.0, 0.0);
    let b = node(&mut g, "b", 0.0, 0.0);
    let c = node(&mut g, "c", 10.0, 0.0);
    edge(&mut g, a, b);
    edge(&mut g, b, c);
    edge(&mut g, a, c);
    let r = run(&g);
    for (e, _) in g.edges() {
        assert!(r.edge(e).points.len() >= 2);
    }
}

#[test]
fn bounds_cover_nodes_and_routes() {
    let mut g = LayoutGraph::new();
    let a = node(&mut g, "a", 40.0, 20.0);
    let b = node(&mut g, "b", 40.0, 20.0);
    let c = node(&mut g, "c", 40.0, 20.0);
    edge(&mut g, a, b);
    edge(&mut g, b, c);
    edge(&mut g, c, a);
    let r = run(&g);
    assert_ok(&g, &LayoutOptions::default(), &LayoutHints::default(), &r);
    let union = [a, b, c].iter().map(|n| r.node(*n).rect).reduce(|x, y| x.union(&y)).expect("nodes");
    assert!(r.bounds.left() <= union.left() && r.bounds.right() >= union.right());
    assert!(r.bounds.top() <= union.top() && r.bounds.bottom() >= union.bottom());
}
