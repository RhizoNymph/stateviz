//! Edge labels reserve room and come back as boxes.

mod common;

use cascade_layout::{EdgeEnd, LayoutEdge, LayoutGraph, LayoutHints, LayoutOptions, Size};
use common::*;

#[test]
fn label_box_sits_between_its_nodes_without_overlap() {
    let mut g = LayoutGraph::new();
    let a = node(&mut g, "a", 40.0, 20.0);
    let b = node(&mut g, "b", 40.0, 20.0);
    let c = node(&mut g, "c", 40.0, 20.0);
    let labeled =
        g.add_edge(LayoutEdge::new(EdgeEnd::node(a), EdgeEnd::node(b)).with_label(Size::new(120.0, 14.0))).expect("e");
    edge(&mut g, b, c);
    edge(&mut g, a, c);
    let r = run(&g);
    assert_ok(&g, &LayoutOptions::default(), &LayoutHints::default(), &r);
    let label = r.edge(labeled).label.expect("label box");
    assert_eq!(label.size, Size::new(120.0, 14.0));
    let (ra, rb) = (r.node(a).rect, r.node(b).rect);
    assert!(label.left() >= ra.right() - EPS, "{label:?} {ra:?}");
    assert!(label.right() <= rb.left() + EPS, "{label:?} {rb:?}");
    // Room was reserved: the gap between a and b is wider than the label.
    assert!(rb.left() - ra.right() >= 120.0);
}

#[test]
fn many_labels_never_overlap_nodes_or_each_other() {
    let mut g = LayoutGraph::new();
    let n: Vec<_> = (0..6).map(|i| node(&mut g, &format!("n{i}"), 50.0, 24.0)).collect();
    let mut labeled = Vec::new();
    for (i, (x, y)) in [(0, 1), (0, 2), (1, 3), (2, 3), (3, 4), (4, 5), (5, 0), (1, 1)].iter().enumerate() {
        let size = Size::new(30.0 + 10.0 * i as f32, 12.0);
        labeled
            .push(g.add_edge(LayoutEdge::new(EdgeEnd::node(n[*x]), EdgeEnd::node(n[*y])).with_label(size)).expect("e"));
    }
    let r = run(&g);
    assert_ok(&g, &LayoutOptions::default(), &LayoutHints::default(), &r);
    // Labels of edges that pass through layers get their own space.
    let boxes: Vec<_> = labeled[..7].iter().map(|e| r.edge(*e).label.expect("box")).collect();
    for (i, a) in boxes.iter().enumerate() {
        for b in &boxes[i + 1..] {
            assert!(!a.intersects(b), "{a:?} overlaps {b:?}");
        }
    }
}
