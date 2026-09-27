//! Ports: edges attach at the port's side, ports on one side are spread
//! evenly, edges sharing a port converge, unported edges use the flow-facing
//! sides.

mod common;

use cascade_layout::{
    EdgeEnd, FlowDirection, LayoutEdge, LayoutGraph, LayoutHints, LayoutNode, LayoutOptions, Port, PortSide, Size,
};
use common::*;

fn ported(g: &mut LayoutGraph, key: &str, w: f32, h: f32, sides: &[PortSide]) -> cascade_layout::NodeId {
    g.add_node(LayoutNode::new(key, Size::new(w, h)).with_ports(sides.iter().map(|&side| Port { side }).collect()))
        .expect("node")
}

fn port_edge(g: &mut LayoutGraph, a: EdgeEnd, b: EdgeEnd) -> cascade_layout::EdgeId {
    g.add_edge(LayoutEdge::new(a, b)).expect("edge")
}

#[test]
fn ports_on_one_side_are_spread_evenly_in_index_order() {
    let mut g = LayoutGraph::new();
    let hub = ported(&mut g, "hub", 50.0, 80.0, &[PortSide::East, PortSide::East, PortSide::East]);
    let targets: Vec<_> = (0..3).map(|i| node(&mut g, &format!("t{i}"), 30.0, 20.0)).collect();
    let edges: Vec<_> =
        (0..3u16).map(|p| port_edge(&mut g, EdgeEnd::port(hub, p), EdgeEnd::node(targets[p as usize]))).collect();
    let r = run(&g);
    assert_ok(&g, &LayoutOptions::default(), &LayoutHints::default(), &r);
    let rect = r.node(hub).rect;
    for (i, e) in edges.iter().enumerate() {
        let p = r.edge(*e).points[0];
        assert!((p.x - rect.right()).abs() < EPS);
        let expected = rect.top() + rect.size.height * (i as f32 + 1.0) / 4.0;
        assert!((p.y - expected).abs() < EPS, "port {i}: {} vs {expected}", p.y);
    }
}

#[test]
fn edges_on_one_port_converge() {
    let mut g = LayoutGraph::new();
    let src = ported(&mut g, "src", 40.0, 40.0, &[PortSide::East]);
    let a = node(&mut g, "a", 30.0, 20.0);
    let b = node(&mut g, "b", 30.0, 20.0);
    let c = node(&mut g, "c", 30.0, 20.0);
    let es: Vec<_> = [a, b, c].iter().map(|t| port_edge(&mut g, EdgeEnd::port(src, 0), EdgeEnd::node(*t))).collect();
    let r = run(&g);
    assert_ok(&g, &LayoutOptions::default(), &LayoutHints::default(), &r);
    let first = r.edge(es[0]).points[0];
    for e in &es {
        assert_eq!(r.edge(*e).points[0], first);
    }
    // And converging on a target port.
    let mut g = LayoutGraph::new();
    let dst = ported(&mut g, "dst", 40.0, 40.0, &[PortSide::West]);
    let srcs: Vec<_> = (0..3).map(|i| node(&mut g, &format!("s{i}"), 30.0, 20.0)).collect();
    let es: Vec<_> = srcs.iter().map(|s| port_edge(&mut g, EdgeEnd::node(*s), EdgeEnd::port(dst, 0))).collect();
    let r = run(&g);
    assert_ok(&g, &LayoutOptions::default(), &LayoutHints::default(), &r);
    let last = *r.edge(es[0]).points.last().expect("points");
    for e in &es {
        assert_eq!(*r.edge(*e).points.last().expect("points"), last);
    }
}

#[test]
fn pills_with_input_and_output_ports() {
    // The causal/structure views' pill: West input port, East output port.
    let mut g = LayoutGraph::new();
    let pills: Vec<_> =
        (0..4).map(|i| ported(&mut g, &format!("p{i}"), 90.0, 26.0, &[PortSide::West, PortSide::East])).collect();
    for (a, b) in [(0, 1), (0, 2), (1, 3), (2, 3), (3, 0)] {
        port_edge(&mut g, EdgeEnd::port(pills[a], 1), EdgeEnd::port(pills[b], 0));
    }
    let r = run(&g);
    assert_ok(&g, &LayoutOptions::default(), &LayoutHints::default(), &r);
}

#[test]
fn north_and_south_ports_leave_vertically() {
    let mut g = LayoutGraph::new();
    let a = ported(&mut g, "a", 60.0, 30.0, &[PortSide::South, PortSide::North]);
    let b = node(&mut g, "b", 40.0, 20.0);
    let c = node(&mut g, "c", 40.0, 20.0);
    let down = port_edge(&mut g, EdgeEnd::port(a, 0), EdgeEnd::node(b));
    let up = port_edge(&mut g, EdgeEnd::port(a, 1), EdgeEnd::node(c));
    let into = port_edge(&mut g, EdgeEnd::node(c), EdgeEnd::port(a, 1));
    let r = run(&g);
    assert_ok(&g, &LayoutOptions::default(), &LayoutHints::default(), &r);
    let ra = r.node(a).rect;
    assert!(on_side(r.edge(down).points[0], &ra, PortSide::South));
    assert!(on_side(r.edge(up).points[0], &ra, PortSide::North));
    assert!(on_side(*r.edge(into).points.last().expect("points"), &ra, PortSide::North));
}

#[test]
fn every_side_combination_routes_cleanly() {
    let sides = [PortSide::North, PortSide::East, PortSide::South, PortSide::West];
    for s in sides {
        for t in sides {
            let mut g = LayoutGraph::new();
            let a = ported(&mut g, "a", 50.0, 30.0, &[s]);
            let m = node(&mut g, "m", 40.0, 20.0);
            let b = ported(&mut g, "b", 50.0, 30.0, &[t]);
            edge(&mut g, a, m);
            edge(&mut g, m, b);
            port_edge(&mut g, EdgeEnd::port(a, 0), EdgeEnd::port(b, 0));
            port_edge(&mut g, EdgeEnd::port(b, 0), EdgeEnd::port(a, 0));
            let r = run(&g);
            if let Err(msg) = check(&g, &LayoutOptions::default(), &LayoutHints::default(), &r, Checks::ALL) {
                panic!("{s:?} -> {t:?}: {msg}");
            }
        }
    }
}

#[test]
fn unported_edges_use_flow_facing_sides_top_to_bottom() {
    let mut g = LayoutGraph::new();
    let a = node(&mut g, "a", 40.0, 20.0);
    let b = node(&mut g, "b", 40.0, 20.0);
    let e = edge(&mut g, a, b);
    let options = LayoutOptions { direction: FlowDirection::TopToBottom, ..LayoutOptions::default() };
    let r = run_with(&g, &options, &LayoutHints::default());
    assert_ok(&g, &options, &LayoutHints::default(), &r);
    assert!(on_side(r.edge(e).points[0], &r.node(a).rect, PortSide::South));
    assert!(on_side(*r.edge(e).points.last().expect("points"), &r.node(b).rect, PortSide::North));
}

#[test]
fn several_unported_edges_on_a_side_get_distinct_points() {
    let mut g = LayoutGraph::new();
    let a = node(&mut g, "a", 40.0, 60.0);
    let ts: Vec<_> = (0..4).map(|i| node(&mut g, &format!("t{i}"), 30.0, 20.0)).collect();
    let es: Vec<_> = ts.iter().map(|t| edge(&mut g, a, *t)).collect();
    let r = run(&g);
    assert_ok(&g, &LayoutOptions::default(), &LayoutHints::default(), &r);
    let mut ys: Vec<f32> = es.iter().map(|e| r.edge(*e).points[0].y).collect();
    ys.sort_by(f32::total_cmp);
    for w in ys.windows(2) {
        assert!(w[1] - w[0] > 1.0, "{ys:?}");
    }
}
