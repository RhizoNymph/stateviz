//! Layer constraints, their conflicts and input validation.

mod common;

use cascade_layout::{
    EdgeEnd, EdgeRouting, Insets, LayerConstraint, LayoutEdge, LayoutError, LayoutGraph, LayoutGroup, LayoutHints,
    LayoutNode, LayoutOptions, Point, Size,
};
use common::*;

fn constrained(g: &mut LayoutGraph, key: &str, layer: LayerConstraint) -> cascade_layout::NodeId {
    g.add_node(LayoutNode::new(key, Size::new(40.0, 20.0)).with_layer(layer)).expect("node")
}

#[test]
fn first_nodes_sit_in_layer_zero() {
    let mut g = LayoutGraph::new();
    let a = node(&mut g, "a", 40.0, 20.0);
    let b = node(&mut g, "b", 40.0, 20.0);
    let ext = constrained(&mut g, "ext", LayerConstraint::First);
    let late = constrained(&mut g, "late", LayerConstraint::First);
    edge(&mut g, a, b);
    edge(&mut g, ext, b);
    edge(&mut g, b, late);
    let r = run(&g);
    assert_ok(&g, &LayoutOptions::default(), &LayoutHints::default(), &r);
    assert_eq!(r.node(ext).layer, 0);
    assert_eq!(r.node(late).layer, 0);
    assert!(r.node(b).layer >= 1);
}

#[test]
fn exact_layers_are_honoured() {
    let mut g = LayoutGraph::new();
    let a = constrained(&mut g, "a", LayerConstraint::Exact(0));
    let b = constrained(&mut g, "b", LayerConstraint::Exact(3));
    let c = node(&mut g, "c", 40.0, 20.0);
    let d = constrained(&mut g, "d", LayerConstraint::Exact(5));
    edge(&mut g, a, b);
    edge(&mut g, b, c);
    edge(&mut g, c, d);
    let r = run(&g);
    assert_ok(&g, &LayoutOptions::default(), &LayoutHints::default(), &r);
    assert_eq!(r.node(a).layer, 0);
    assert_eq!(r.node(b).layer, 3);
    assert_eq!(r.node(d).layer, 5);
    let lc = r.node(c).layer;
    assert!(lc > 3 && lc < 5, "free node between fixed layers got {lc}");
    // Layers stay in left-to-right order even with empty layers between.
    assert!(r.node(b).rect.left() > r.node(a).rect.right());
    assert!(r.node(c).rect.left() > r.node(b).rect.right());
    assert!(r.node(d).rect.left() > r.node(c).rect.right());
}

#[test]
fn huge_exact_layer_does_not_blow_up() {
    let mut g = LayoutGraph::new();
    let a = constrained(&mut g, "a", LayerConstraint::Exact(0));
    let b = constrained(&mut g, "b", LayerConstraint::Exact(4_000_000_000));
    edge(&mut g, a, b);
    let r = run(&g);
    assert_eq!(r.node(b).layer, 4_000_000_000);
    assert!(r.bounds.size.width < 10_000.0);
}

#[test]
fn acyclic_edge_against_exact_layers_is_unsatisfiable() {
    let mut g = LayoutGraph::new();
    let a = constrained(&mut g, "a", LayerConstraint::Exact(3));
    let b = constrained(&mut g, "b", LayerConstraint::Exact(1));
    edge(&mut g, a, b);
    let err = cascade_layout::layout(&g, &LayoutOptions::default(), &LayoutHints::default());
    assert!(matches!(err, Err(LayoutError::Unsatisfiable { .. })), "{err:?}");

    let mut flat = LayoutGraph::new();
    let a = constrained(&mut flat, "a", LayerConstraint::First);
    let b = constrained(&mut flat, "b", LayerConstraint::Exact(0));
    edge(&mut flat, a, b);
    let err = cascade_layout::layout(&flat, &LayoutOptions::default(), &LayoutHints::default());
    assert!(matches!(err, Err(LayoutError::Unsatisfiable { .. })), "{err:?}");
}

#[test]
fn backward_exact_edge_on_a_cycle_is_reversed_not_an_error() {
    let mut g = LayoutGraph::new();
    let a = constrained(&mut g, "a", LayerConstraint::Exact(1));
    let b = constrained(&mut g, "b", LayerConstraint::Exact(2));
    let c = constrained(&mut g, "c", LayerConstraint::Exact(3));
    edge(&mut g, a, b);
    edge(&mut g, b, c);
    let back = edge(&mut g, c, a);
    let r = run(&g);
    assert_ok(&g, &LayoutOptions::default(), &LayoutHints::default(), &r);
    assert!(r.edge(back).reversed);
    assert_eq!((r.node(a).layer, r.node(b).layer, r.node(c).layer), (1, 2, 3));
}

#[test]
fn free_nodes_are_relaxed_instead_of_failing() {
    // a (1) -> f -> b (2): f cannot sit strictly between 1 and 2.
    let mut g = LayoutGraph::new();
    let a = constrained(&mut g, "a", LayerConstraint::Exact(1));
    let f = node(&mut g, "f", 40.0, 20.0);
    let b = constrained(&mut g, "b", LayerConstraint::Exact(2));
    let af = edge(&mut g, a, f);
    let fb = edge(&mut g, f, b);
    let r = run(&g);
    assert_ok(&g, &LayoutOptions::default(), &LayoutHints::default(), &r);
    assert_eq!(r.node(a).layer, 1);
    assert_eq!(r.node(b).layer, 2);
    // Neither edge is on a cycle, so neither is flagged.
    assert!(!r.edge(af).reversed && !r.edge(fb).reversed);

    // A free node pointing into a First node cannot precede it; the edge
    // is kept but runs against the flow, unflagged.
    let mut g = LayoutGraph::new();
    let x = node(&mut g, "x", 40.0, 20.0);
    let first = constrained(&mut g, "first", LayerConstraint::First);
    let e = edge(&mut g, x, first);
    let r = run(&g);
    assert_ok(&g, &LayoutOptions::default(), &LayoutHints::default(), &r);
    assert_eq!(r.node(first).layer, 0);
    assert!(!r.edge(e).reversed);
}

#[test]
fn constraints_apply_per_group() {
    let mut g = LayoutGraph::new();
    let lane = g.add_group(LayoutGroup { key: "lane".into(), padding: Insets::uniform(8.0), header: 16.0 });
    let a = g
        .add_node(LayoutNode::new("a", Size::new(40.0, 20.0)).in_group(lane).with_layer(LayerConstraint::Exact(2)))
        .expect("a");
    let b = g.add_node(LayoutNode::new("b", Size::new(40.0, 20.0)).in_group(lane)).expect("b");
    edge(&mut g, a, b);
    let r = run(&g);
    assert_ok(&g, &LayoutOptions::default(), &LayoutHints::default(), &r);
    assert_eq!(r.node(a).layer, 2);
    assert_eq!(r.node(b).layer, 3);
}

#[test]
fn invalid_sizes_and_options_are_rejected() {
    let mut g = LayoutGraph::new();
    g.add_node(LayoutNode::new("nan", Size::new(f32::NAN, 10.0))).expect("node");
    assert!(matches!(
        cascade_layout::layout(&g, &LayoutOptions::default(), &LayoutHints::default()),
        Err(LayoutError::InvalidNodeSize { .. })
    ));

    let mut g = LayoutGraph::new();
    g.add_node(LayoutNode::new("neg", Size::new(10.0, -1.0))).expect("node");
    assert!(matches!(
        cascade_layout::layout(&g, &LayoutOptions::default(), &LayoutHints::default()),
        Err(LayoutError::InvalidNodeSize { .. })
    ));

    let mut g = LayoutGraph::new();
    let a = node(&mut g, "a", 10.0, 10.0);
    g.add_edge(LayoutEdge::new(EdgeEnd::node(a), EdgeEnd::node(a)).with_label(Size::new(f32::INFINITY, 1.0)))
        .expect("edge");
    assert!(matches!(
        cascade_layout::layout(&g, &LayoutOptions::default(), &LayoutHints::default()),
        Err(LayoutError::InvalidEdgeLabel { .. })
    ));

    let mut g = LayoutGraph::new();
    g.add_group(LayoutGroup { key: "bad".into(), padding: Insets::uniform(-3.0), header: 0.0 });
    assert!(matches!(
        cascade_layout::layout(&g, &LayoutOptions::default(), &LayoutHints::default()),
        Err(LayoutError::InvalidGroup { .. })
    ));

    let g = LayoutGraph::new();
    for options in [
        LayoutOptions { node_spacing: -1.0, ..LayoutOptions::default() },
        LayoutOptions { layer_spacing: f32::NAN, ..LayoutOptions::default() },
        LayoutOptions { edge_spacing: f32::INFINITY, ..LayoutOptions::default() },
        LayoutOptions { group_spacing: -0.5, ..LayoutOptions::default() },
    ] {
        assert!(matches!(
            cascade_layout::layout(&g, &options, &LayoutHints::default()),
            Err(LayoutError::InvalidOption { .. })
        ));
    }

    let mut g = LayoutGraph::new();
    node(&mut g, "p", 10.0, 10.0);
    let mut hints = LayoutHints::default();
    hints.pins.insert("p".into(), Point::new(f32::NAN, 0.0));
    assert!(matches!(
        cascade_layout::layout(&g, &LayoutOptions::default(), &hints),
        Err(LayoutError::InvalidPin { .. })
    ));
}

#[test]
fn unknown_pins_and_bad_previous_hints_are_ignored() {
    let mut g = LayoutGraph::new();
    let a = node(&mut g, "a", 10.0, 10.0);
    let b = node(&mut g, "b", 10.0, 10.0);
    edge(&mut g, a, b);
    let mut hints = LayoutHints::default();
    hints.pins.insert("ghost".into(), Point::new(5.0, 5.0));
    let mut prev = cascade_layout::PreviousLayout::default();
    prev.nodes.insert(
        "a".into(),
        cascade_layout::PreviousPlacement { layer: 0, order: 0, position: Point::new(f32::NAN, 3.0) },
    );
    hints.previous = Some(prev);
    let options = LayoutOptions { routing: EdgeRouting::Orthogonal, ..LayoutOptions::default() };
    let r = run_with(&g, &options, &hints);
    assert_ok(&g, &options, &hints, &r);
}
