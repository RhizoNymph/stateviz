//! Shared layers (`LayoutOptions::shared_layers`): every group lays its
//! layers out at the same positions, edges between groups always head with
//! the flow unless reversed to break a cycle, no side corridor is used, and
//! stability, pins and determinism still hold.

mod common;

use cascade_layout::{
    EdgeEnd, EdgeRoute, EdgeRouting, FlowDirection, LayerConstraint, LayoutEdge, LayoutGraph, LayoutHints, LayoutNode,
    LayoutOptions, LayoutResult, Point, Port, PortSide, Rect, Size,
};
use common::*;

fn shared() -> LayoutOptions {
    LayoutOptions { shared_layers: true, ..LayoutOptions::default() }
}

/// Three lanes: a long chain in the first, a short one in the second fed
/// from the first, and a third lane fed from the second and the first.
fn three_lanes() -> Spec {
    Spec::new()
        .group("a")
        .group("b")
        .group("c")
        .node_in("a0", 60.0, 24.0, 0)
        .node_in("a1", 90.0, 24.0, 0)
        .node_in("a2", 40.0, 24.0, 0)
        .node_in("a3", 70.0, 24.0, 0)
        .node_in("b0", 50.0, 24.0, 1)
        .node_in("b1", 120.0, 24.0, 1)
        .node_in("c0", 50.0, 24.0, 2)
        .node_in("c1", 30.0, 24.0, 2)
        .edge("a0", "a1")
        .edge("a1", "a2")
        .edge("a2", "a3")
        .edge("a1", "b0")
        .edge("b0", "b1")
        .edge("b1", "c0")
        .edge("a0", "c1")
        .edge("c1", "a3")
}

/// Random grouped graphs, with some labels, ports and first-layer nodes.
fn random_case(seed: u64) -> LayoutGraph {
    let mut rng = Lcg::new(seed);
    let mut g = LayoutGraph::new();
    let groups: Vec<_> = (0..2 + rng.below(4))
        .map(|i| {
            g.add_group(cascade_layout::LayoutGroup {
                key: format!("g{i}"),
                padding: cascade_layout::Insets::uniform(8.0 + rng.below(8) as f32),
                header: 18.0,
            })
        })
        .collect();
    let n = 6 + rng.below(26) as usize;
    let ported = rng.chance(50);
    let mut ids = Vec::new();
    for i in 0..n {
        let size = Size::new(20.0 + rng.below(100) as f32, 16.0 + rng.below(20) as f32);
        let mut node = LayoutNode::new(format!("n{i}"), size).in_group(groups[rng.below(groups.len() as u32) as usize]);
        if ported {
            node = node.with_ports(vec![Port { side: PortSide::West }, Port { side: PortSide::East }]);
        }
        if i < 2 && rng.chance(50) {
            node = node.with_layer(LayerConstraint::First);
        }
        ids.push(g.add_node(node).expect("node"));
    }
    let m = n + rng.below(2 * n as u32) as usize;
    for _ in 0..m {
        let a = rng.below(n as u32) as usize;
        let b = rng.below(n as u32) as usize;
        if a == b || (b < 2 && g.node(ids[b]).layer == LayerConstraint::First) {
            continue;
        }
        let (s, t) = if ported {
            (EdgeEnd::port(ids[a], 1), EdgeEnd::port(ids[b], 0))
        } else {
            (EdgeEnd::node(ids[a]), EdgeEnd::node(ids[b]))
        };
        let mut e = LayoutEdge::new(s, t);
        if rng.chance(20) {
            e = e.with_label(Size::new(20.0 + rng.below(50) as f32, 12.0));
        }
        g.add_edge(e).expect("edge");
    }
    g
}

/// Every way a forward (not reversed) route runs against the flow: its end
/// left of its start, or a horizontal segment heading left.
fn leftward(g: &LayoutGraph, r: &LayoutResult) -> Vec<String> {
    let mut out = Vec::new();
    for (e, edge) in g.edges() {
        let route: &EdgeRoute = r.edge(e);
        if route.reversed || edge.source.node == edge.target.node {
            continue;
        }
        let name = format!("{} -> {}", g.node(edge.source.node).key, g.node(edge.target.node).key);
        let (first, last) = (route.points[0], route.points[route.points.len() - 1]);
        if last.x < first.x - EPS {
            out.push(format!("{name} ends left of its start: {:?}", route.points));
        }
        for w in route.points.windows(2) {
            if (w[0].y - w[1].y).abs() < EPS && w[1].x < w[0].x - EPS {
                out.push(format!("{name} heads left {:?} -> {:?}", w[0], w[1]));
            }
        }
    }
    out
}

fn lanes_extent(g: &LayoutGraph, r: &LayoutResult) -> (f32, f32) {
    g.groups()
        .map(|(id, _)| r.group(id))
        .fold((f32::INFINITY, f32::NEG_INFINITY), |(l, h), rect: Rect| (l.min(rect.left()), h.max(rect.right())))
}

#[test]
fn shared_layers_are_off_by_default() {
    assert!(!LayoutOptions::default().shared_layers);
}

#[test]
fn every_group_puts_a_layer_at_the_same_place() {
    let g = three_lanes().build();
    let r = run_with(&g, &shared(), &LayoutHints::default());
    assert_ok(&g, &shared(), &LayoutHints::default(), &r);
    // Layers are global: b0 follows a1 and c1 follows a0, across groups.
    let layer = |k: &str| r.node(g.node_by_key(k).expect("key")).layer;
    assert!(layer("b0") > layer("a1") && layer("c1") > layer("a0") && layer("a3") > layer("c1"));
    // Nodes of one layer share a centre, whatever their group.
    let mut by_layer: std::collections::BTreeMap<u32, Vec<f32>> = std::collections::BTreeMap::new();
    for (id, _) in g.nodes() {
        let p = r.node(id);
        by_layer.entry(p.layer).or_default().push(p.rect.center().x);
    }
    for (l, centres) in &by_layer {
        assert!(centres.iter().all(|c| (c - centres[0]).abs() < EPS), "layer {l}: {centres:?}");
    }
    // Columns do not overlap: every node of a later layer lies right of every
    // node of an earlier one.
    for (a, na) in g.nodes() {
        for (b, _) in g.nodes() {
            let (pa, pb) = (r.node(a), r.node(b));
            if pa.layer < pb.layer {
                assert!(pa.rect.right() < pb.rect.left(), "{} {:?} vs {:?}", na.key, pa.rect, pb.rect);
            }
        }
    }
}

#[test]
fn edges_between_groups_point_forward_and_head_right() {
    let g = three_lanes().build();
    let r = run_with(&g, &shared(), &LayoutHints::default());
    assert_eq!(leftward(&g, &r), Vec::<String>::new());
    for (e, edge) in g.edges() {
        let (s, t) = (r.node(edge.source.node), r.node(edge.target.node));
        assert!(t.layer > s.layer && !r.edge(e).reversed, "edge {} runs backwards", e.index());
    }
}

#[test]
fn no_forward_edge_heads_left_in_random_grouped_graphs() {
    for seed in 0..300u64 {
        let g = random_case(seed);
        for options in [shared(), LayoutOptions { routing: EdgeRouting::Polyline, ..shared() }] {
            let r = run_with(&g, &options, &LayoutHints::default());
            if let Err(msg) = check(&g, &options, &LayoutHints::default(), &r, Checks::ALL) {
                panic!("seed {seed}: {msg}");
            }
            // Polyline chains are straight lines, and a polyline crossing a
            // node is rerouted around it by the obstacle router: the
            // guarantee is for orthogonal routing.
            if options.routing == EdgeRouting::Orthogonal {
                let bad = leftward(&g, &r);
                assert!(bad.is_empty(), "seed {seed}: {bad:#?}");
            }
            // No side corridors: every route stays within the lanes' width.
            let (lo, hi) = lanes_extent(&g, &r);
            for (e, _) in g.edges() {
                assert!(
                    r.edge(e).points.iter().all(|p: &Point| p.x >= lo - EPS && p.x <= hi + EPS),
                    "seed {seed}: edge {} leaves the lanes: {:?}",
                    e.index(),
                    r.edge(e).points
                );
            }
        }
    }
}

#[test]
fn only_edges_on_a_cycle_are_reversed_and_they_may_cross_groups() {
    // a0 -> b0 -> a1 -> a0 across two groups.
    let g = Spec::new()
        .group("a")
        .group("b")
        .node_in("a0", 40.0, 20.0, 0)
        .node_in("a1", 40.0, 20.0, 0)
        .node_in("b0", 40.0, 20.0, 1)
        .node_in("b1", 40.0, 20.0, 1)
        .edge("a0", "b0")
        .edge("b0", "a1")
        .edge("a1", "a0")
        .edge("b0", "b1")
        .build();
    let r = run_with(&g, &shared(), &LayoutHints::default());
    assert_ok(&g, &shared(), &LayoutHints::default(), &r);
    let reversed: Vec<usize> = g.edges().filter(|(e, _)| r.edge(*e).reversed).map(|(e, _)| e.index()).collect();
    assert_eq!(reversed.len(), 1, "one back edge breaks the cycle");
    assert!(reversed[0] < 3, "the acyclic edge is never reversed");
    assert!(leftward(&g, &r).is_empty());
}

#[test]
fn top_to_bottom_flow_heads_down() {
    let g = three_lanes().build();
    let options = LayoutOptions { direction: FlowDirection::TopToBottom, ..shared() };
    let r = run_with(&g, &options, &LayoutHints::default());
    assert_ok(&g, &options, &LayoutHints::default(), &r);
    for (e, _) in g.edges() {
        let pts = &r.edge(e).points;
        for w in pts.windows(2) {
            if (w[0].x - w[1].x).abs() < EPS {
                assert!(w[1].y >= w[0].y - EPS, "edge {} heads up: {pts:?}", e.index());
            }
        }
    }
}

#[test]
fn unchanged_layout_comes_back_exactly() {
    for seed in [3u64, 17, 42] {
        let g = random_case(seed);
        let first = run_with(&g, &shared(), &LayoutHints::default());
        let hints = LayoutHints { previous: Some(first.to_previous(&g)), ..LayoutHints::default() };
        let again = run_with(&g, &shared(), &hints);
        assert_eq!(first, again, "seed {seed}");
        let hints = LayoutHints { previous: Some(again.to_previous(&g)), ..LayoutHints::default() };
        assert_eq!(first, run_with(&g, &shared(), &hints), "seed {seed}, second relayout");
    }
}

#[test]
fn an_edit_in_one_lane_leaves_the_other_lanes_in_place() {
    let before = three_lanes();
    let g0 = before.build();
    let r0 = run_with(&g0, &shared(), &LayoutHints::default());
    // A new node in lane c at the layer of c0, fed by c0's predecessor.
    let after = three_lanes().node_in("c2", 40.0, 24.0, 2).edge("b1", "c2");
    let g1 = after.build();
    let hints = LayoutHints { previous: Some(r0.to_previous(&g0)), ..LayoutHints::default() };
    let r1 = run_with(&g1, &shared(), &hints);
    assert_ok(&g1, &shared(), &hints, &r1);
    for key in ["a0", "a1", "a2", "a3", "b0", "b1", "c0", "c1"] {
        assert_eq!(rect_of(&g0, &r0, key), rect_of(&g1, &r1, key), "{key} moved");
    }
    assert!(leftward(&g1, &r1).is_empty());
    // Removing it again returns to the first layout's node positions.
    let hints = LayoutHints { previous: Some(r1.to_previous(&g1)), ..LayoutHints::default() };
    let r2 = run_with(&g0, &shared(), &hints);
    for key in ["a0", "a1", "a2", "a3", "b0", "b1", "c0", "c1"] {
        assert_eq!(rect_of(&g0, &r0, key), rect_of(&g0, &r2, key), "{key} moved back");
    }
}

#[test]
fn a_new_backward_edge_between_kept_nodes_still_points_forward() {
    // c1 -> a1 would point backwards if a1 kept its layer; the layering
    // releases the kept layer rather than send the edge against the flow.
    let g0 = three_lanes().build();
    let r0 = run_with(&g0, &shared(), &LayoutHints::default());
    let g1 = three_lanes().edge("b1", "a2").build();
    let hints = LayoutHints { previous: Some(r0.to_previous(&g0)), ..LayoutHints::default() };
    let r1 = run_with(&g1, &shared(), &hints);
    assert_ok(&g1, &shared(), &hints, &r1);
    assert!(leftward(&g1, &r1).is_empty(), "{:#?}", leftward(&g1, &r1));
}

#[test]
fn pins_are_honoured() {
    let g = three_lanes().build();
    let mut hints = LayoutHints::default();
    hints.pins.insert("b1".to_string(), Point::new(700.0, 40.0));
    let r = run_with(&g, &shared(), &hints);
    assert_eq!(rect_of(&g, &r, "b1").origin, Point::new(700.0, 40.0));
    let checks = Checks { foreign_groups: false, groups: false, ..Checks::ALL };
    if let Err(msg) = check(&g, &shared(), &hints, &r, checks) {
        panic!("{msg}");
    }
}

#[test]
fn shared_layouts_are_deterministic() {
    for seed in 200..220u64 {
        let g = random_case(seed);
        let a = run_with(&g, &shared(), &LayoutHints::default());
        let b = run_with(&g, &shared(), &LayoutHints::default());
        assert_eq!(a, b, "seed {seed}");
    }
}

#[test]
fn relayouts_after_random_edits_keep_every_guarantee() {
    for seed in 0..120u64 {
        let g0 = random_case(seed);
        let r0 = run_with(&g0, &shared(), &LayoutHints::default());
        // Rebuild with one node dropped and one edge added.
        let mut rng = Lcg::new(seed ^ 0xABCD);
        let drop = format!("n{}", rng.below(g0.node_count() as u32));
        let mut g1 = LayoutGraph::new();
        let groups: Vec<_> = g0.groups().map(|(_, grp)| g1.add_group(grp.clone())).collect();
        for (_, n) in g0.nodes().filter(|(_, n)| n.key != drop) {
            let mut node = n.clone();
            node.group = n.group.map(|gid| groups[gid.index()]);
            g1.add_node(node).expect("node");
        }
        for (_, e) in g0.edges() {
            let (s, t) = (&g0.node(e.source.node).key, &g0.node(e.target.node).key);
            if let (Some(a), Some(b)) = (g1.node_by_key(s), g1.node_by_key(t)) {
                g1.add_edge(LayoutEdge {
                    source: EdgeEnd { node: a, ..e.source },
                    target: EdgeEnd { node: b, ..e.target },
                    ..e.clone()
                })
                .expect("edge");
            }
        }
        let ids: Vec<_> = g1.nodes().map(|(id, _)| id).collect();
        let (a, b) = (ids[rng.below(ids.len() as u32) as usize], ids[rng.below(ids.len() as u32) as usize]);
        if a != b && g1.node(b).layer != LayerConstraint::First {
            let ported = !g1.node(a).ports.is_empty();
            let (s, t) =
                if ported { (EdgeEnd::port(a, 1), EdgeEnd::port(b, 0)) } else { (EdgeEnd::node(a), EdgeEnd::node(b)) };
            g1.add_edge(LayoutEdge::new(s, t)).expect("edge");
        }
        let hints = LayoutHints { previous: Some(r0.to_previous(&g0)), ..LayoutHints::default() };
        let r1 = run_with(&g1, &shared(), &hints);
        // As for every relayout (tests/fuzz.rs), kept positions can leave a
        // reserved label box on a moved node.
        if let Err(msg) = check(&g1, &shared(), &hints, &r1, Checks { labels: false, ..Checks::ALL }) {
            panic!("seed {seed}: {msg}");
        }
        let bad = leftward(&g1, &r1);
        assert!(bad.is_empty(), "seed {seed}: {bad:#?}");
    }
}
