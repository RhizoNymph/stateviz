//! Pins: pinned nodes sit exactly at their pin, other nodes are pushed
//! aside, and edges route to the pinned node's ports.

mod common;

use cascade_layout::{
    EdgeEnd, LayoutEdge, LayoutGraph, LayoutHints, LayoutNode, LayoutOptions, Point, Port, PortSide, Size,
};
use common::*;

fn chain() -> (LayoutGraph, Vec<cascade_layout::NodeId>) {
    let mut g = LayoutGraph::new();
    let ids: Vec<_> = (0..5).map(|i| node(&mut g, &format!("n{i}"), 60.0, 24.0)).collect();
    for w in ids.windows(2) {
        edge(&mut g, w[0], w[1]);
    }
    let extra = node(&mut g, "side", 60.0, 24.0);
    edge(&mut g, ids[1], extra);
    edge(&mut g, extra, ids[3]);
    (g, ids)
}

#[test]
fn pinned_node_sits_exactly_at_its_pin() {
    let (g, ids) = chain();
    let mut hints = LayoutHints::default();
    hints.pins.insert("n2".into(), Point::new(517.25, -140.5));
    let r = run_with(&g, &LayoutOptions::default(), &hints);
    assert_ok(&g, &LayoutOptions::default(), &hints, &r);
    assert_eq!(r.node(ids[2]).rect.origin, Point::new(517.25, -140.5));
}

#[test]
fn nodes_are_pushed_off_a_pin_dropped_on_them() {
    let (g, ids) = chain();
    let free = run(&g);
    // Drop n4 right on top of n1's natural position.
    let target = free.node(ids[1]).rect.origin;
    let mut hints = LayoutHints::default();
    hints.pins.insert("n4".into(), Point::new(target.x + 5.0, target.y + 3.0));
    let r = run_with(&g, &LayoutOptions::default(), &hints);
    assert_ok(&g, &LayoutOptions::default(), &hints, &r);
    assert_eq!(r.node(ids[4]).rect.origin, Point::new(target.x + 5.0, target.y + 3.0));
    assert!(!r.node(ids[1]).rect.intersects(&r.node(ids[4]).rect));
}

#[test]
fn edges_attach_to_pinned_ports() {
    let mut g = LayoutGraph::new();
    let a = node(&mut g, "a", 40.0, 20.0);
    let p = g
        .add_node(
            LayoutNode::new("pinned", Size::new(80.0, 40.0))
                .with_ports(vec![Port { side: PortSide::North }, Port { side: PortSide::South }]),
        )
        .expect("p");
    let b = node(&mut g, "b", 40.0, 20.0);
    let into = g.add_edge(LayoutEdge::new(EdgeEnd::node(a), EdgeEnd::port(p, 0))).expect("e");
    let out = g.add_edge(LayoutEdge::new(EdgeEnd::port(p, 1), EdgeEnd::node(b))).expect("e");
    let mut hints = LayoutHints::default();
    hints.pins.insert("pinned".into(), Point::new(-300.0, 200.0));
    let r = run_with(&g, &LayoutOptions::default(), &hints);
    assert_ok(&g, &LayoutOptions::default(), &hints, &r);
    let pr = r.node(p).rect;
    assert!(on_side(*r.edge(into).points.last().expect("points"), &pr, PortSide::North));
    assert!(on_side(r.edge(out).points[0], &pr, PortSide::South));
}

#[test]
fn several_pins_and_groups() {
    let spec = Spec::new()
        .group("A")
        .group("B")
        .node_in("a1", 60.0, 24.0, 0)
        .node_in("a2", 60.0, 24.0, 0)
        .node_in("a3", 60.0, 24.0, 0)
        .node_in("b1", 60.0, 24.0, 1)
        .node_in("b2", 60.0, 24.0, 1)
        .edge("a1", "a2")
        .edge("a2", "a3")
        .edge("b1", "b2")
        .edge("a2", "b2");
    let g = spec.build();
    let free = run(&g);
    let mut hints = LayoutHints::default();
    // Pin a3 somewhere inside lane A's area, over a2.
    let a2 = rect_of(&g, &free, "a2");
    hints.pins.insert("a3".into(), Point::new(a2.left() + 10.0, a2.top()));
    // And pin b1 where it already is: nothing should change for it.
    let b1 = rect_of(&g, &free, "b1");
    hints.pins.insert("b1".into(), b1.origin);
    let r = run_with(&g, &LayoutOptions::default(), &hints);
    let checks = Checks { groups: false, ..Checks::ALL };
    if let Err(msg) = check(&g, &LayoutOptions::default(), &hints, &r, checks) {
        panic!("{msg}");
    }
    assert_eq!(rect_of(&g, &r, "b1").origin, b1.origin);
    // A pinned node still lies inside its group.
    let lane_a = r.group(g.groups().next().expect("A").0);
    let a3 = rect_of(&g, &r, "a3");
    assert!(lane_a.left() <= a3.left() && lane_a.right() >= a3.right());
    assert!(lane_a.top() <= a3.top() && lane_a.bottom() >= a3.bottom());
}

#[test]
fn pins_survive_relayout_with_previous() {
    let (g, ids) = chain();
    let mut hints = LayoutHints::default();
    hints.pins.insert("n3".into(), Point::new(900.0, 400.0));
    let r1 = run_with(&g, &LayoutOptions::default(), &hints);
    hints.previous = Some(r1.to_previous(&g));
    let r2 = run_with(&g, &LayoutOptions::default(), &hints);
    assert_ok(&g, &LayoutOptions::default(), &hints, &r2);
    assert_eq!(r2.node(ids[3]).rect.origin, Point::new(900.0, 400.0));
    for id in &ids {
        assert_eq!(r1.node(*id).rect, r2.node(*id).rect);
    }
}
