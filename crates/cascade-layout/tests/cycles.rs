//! Cycle breaking: every cycle shows at least one reversed edge, acyclic
//! edges never do, and self-loops are small loops on the node's side.

mod common;

use cascade_layout::{EdgeEnd, LayoutEdge, LayoutGraph, LayoutHints, LayoutNode, LayoutOptions, Port, PortSide, Size};
use common::*;

#[test]
fn three_cycle_reverses_exactly_one_edge() {
    let mut g = LayoutGraph::new();
    let a = node(&mut g, "a", 40.0, 20.0);
    let b = node(&mut g, "b", 40.0, 20.0);
    let c = node(&mut g, "c", 40.0, 20.0);
    let ids = [edge(&mut g, a, b), edge(&mut g, b, c), edge(&mut g, c, a)];
    let r = run(&g);
    assert_ok(&g, &LayoutOptions::default(), &LayoutHints::default(), &r);
    let reversed: Vec<_> = ids.iter().filter(|e| r.edge(**e).reversed).collect();
    assert_eq!(reversed.len(), 1, "{reversed:?}");
    // The reversed edge runs against the layer order.
    let e = g.edge(*reversed[0]);
    assert!(r.node(e.target.node).layer <= r.node(e.source.node).layer);
}

#[test]
fn acyclic_edges_are_never_reversed() {
    // A cycle b -> c -> d -> b with an acyclic tail a -> b and exit d -> e.
    let mut g = LayoutGraph::new();
    let a = node(&mut g, "a", 40.0, 20.0);
    let b = node(&mut g, "b", 40.0, 20.0);
    let c = node(&mut g, "c", 40.0, 20.0);
    let d = node(&mut g, "d", 40.0, 20.0);
    let e = node(&mut g, "e", 40.0, 20.0);
    let tail = edge(&mut g, a, b);
    let cycle = [edge(&mut g, b, c), edge(&mut g, c, d), edge(&mut g, d, b)];
    let exit = edge(&mut g, d, e);
    let r = run(&g);
    assert_ok(&g, &LayoutOptions::default(), &LayoutHints::default(), &r);
    assert!(!r.edge(tail).reversed && !r.edge(exit).reversed);
    assert_eq!(cycle.iter().filter(|x| r.edge(**x).reversed).count(), 1);

    // A DAG with converging paths has no reversed edges at all.
    let mut dag = LayoutGraph::new();
    let n: Vec<_> = (0..6).map(|i| node(&mut dag, &format!("n{i}"), 30.0, 20.0)).collect();
    for (x, y) in [(0, 1), (0, 2), (1, 3), (2, 3), (3, 4), (0, 4), (5, 4), (0, 5)] {
        edge(&mut dag, n[x], n[y]);
    }
    let r = run(&dag);
    assert!(dag.edges().all(|(e, _)| !r.edge(e).reversed));
}

#[test]
fn every_cycle_contains_a_reversed_edge() {
    // Two interlocking cycles plus a tail.
    let mut g = LayoutGraph::new();
    let n: Vec<_> = (0..7).map(|i| node(&mut g, &format!("n{i}"), 30.0, 20.0)).collect();
    let pairs = [(0, 1), (1, 2), (2, 0), (2, 3), (3, 4), (4, 2), (4, 5), (5, 6)];
    let ids: Vec<_> = pairs.iter().map(|&(x, y)| edge(&mut g, n[x], n[y])).collect();
    let r = run(&g);
    assert_ok(&g, &LayoutOptions::default(), &LayoutHints::default(), &r);
    let rev = |i: usize| r.edge(ids[i]).reversed;
    assert!(rev(0) || rev(1) || rev(2), "cycle 0-1-2 has no reversed edge");
    assert!(rev(3) || rev(4) || rev(5), "cycle 2-3-4 has no reversed edge");
    assert!(!rev(6) && !rev(7), "tail edges are acyclic");
    for (i, &(x, y)) in pairs.iter().enumerate() {
        let (lx, ly) = (r.node(n[x]).layer, r.node(n[y]).layer);
        if rev(i) {
            assert!(ly <= lx);
        } else {
            assert!(ly > lx, "non-reversed edge {x}->{y} is not forward ({lx} -> {ly})");
        }
    }
}

#[test]
fn reversed_edges_attach_east_out_and_west_in() {
    let mut g = LayoutGraph::new();
    let a = node(&mut g, "a", 40.0, 20.0);
    let b = node(&mut g, "b", 40.0, 20.0);
    let c = node(&mut g, "c", 40.0, 20.0);
    edge(&mut g, a, b);
    edge(&mut g, b, c);
    let back = edge(&mut g, c, a);
    let r = run(&g);
    assert_ok(&g, &LayoutOptions::default(), &LayoutHints::default(), &r);
    let route = r.edge(back);
    assert!(route.reversed);
    let (ra, rc) = (r.node(a).rect, r.node(c).rect);
    assert!(on_side(route.points[0], &rc, PortSide::East));
    assert!(on_side(*route.points.last().expect("points"), &ra, PortSide::West));
}

#[test]
fn self_loop_is_a_small_loop_on_the_east_side() {
    let mut g = LayoutGraph::new();
    let a = node(&mut g, "a", 60.0, 30.0);
    let b = node(&mut g, "b", 60.0, 30.0);
    edge(&mut g, a, b);
    let lp = edge(&mut g, b, b);
    let r = run(&g);
    assert_ok(&g, &LayoutOptions::default(), &LayoutHints::default(), &r);
    let route = r.edge(lp);
    assert!(!route.reversed);
    let rb = r.node(b).rect;
    let first = route.points[0];
    let last = *route.points.last().expect("points");
    assert!(on_side(first, &rb, PortSide::East));
    assert!(on_side(last, &rb, PortSide::East));
    assert_ne!(first, last);
    // Small: stays within one layer spacing of the node.
    for p in &route.points {
        assert!(p.x <= rb.right() + LayoutOptions::default().layer_spacing + EPS, "{:?}", route.points);
        assert!(p.x >= rb.right() - EPS);
    }
}

#[test]
fn self_loops_on_ported_sides() {
    for (out, inn) in [
        (PortSide::East, PortSide::West),
        (PortSide::North, PortSide::North),
        (PortSide::South, PortSide::East),
        (PortSide::West, PortSide::West),
        (PortSide::North, PortSide::South),
    ] {
        let mut g = LayoutGraph::new();
        let a = g
            .add_node(
                LayoutNode::new("a", Size::new(60.0, 30.0)).with_ports(vec![Port { side: out }, Port { side: inn }]),
            )
            .expect("a");
        let b = node(&mut g, "b", 40.0, 30.0);
        let c = node(&mut g, "c", 40.0, 30.0);
        edge(&mut g, c, a);
        edge(&mut g, a, b);
        let lp = g.add_edge(LayoutEdge::new(EdgeEnd::port(a, 0), EdgeEnd::port(a, 1))).expect("loop");
        let r = run(&g);
        assert_ok(&g, &LayoutOptions::default(), &LayoutHints::default(), &r);
        let route = r.edge(lp);
        let ra = r.node(a).rect;
        assert!(on_side(route.points[0], &ra, out), "{out:?}->{inn:?}: {:?}", route.points);
        assert!(on_side(*route.points.last().expect("points"), &ra, inn), "{out:?}->{inn:?}: {:?}", route.points);
    }
}

#[test]
fn two_cycle_between_neighbours() {
    let mut g = LayoutGraph::new();
    let a = node(&mut g, "a", 40.0, 20.0);
    let b = node(&mut g, "b", 40.0, 20.0);
    let ab = edge(&mut g, a, b);
    let ba = edge(&mut g, b, a);
    let r = run(&g);
    assert_ok(&g, &LayoutOptions::default(), &LayoutHints::default(), &r);
    assert!(r.edge(ab).reversed != r.edge(ba).reversed);
}
