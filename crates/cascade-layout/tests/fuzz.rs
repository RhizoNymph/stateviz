//! Generated graphs: every combination of cycles, self-loops, parallel edges,
//! ports, labels, groups, pins, directions and routing modes must satisfy the
//! layout invariants, before and after an edit fed back as the previous
//! layout.

mod common;

use cascade_layout::{
    EdgeEnd, EdgeRouting, FlowDirection, Insets, LayerConstraint, LayoutEdge, LayoutGraph, LayoutGroup, LayoutHints,
    LayoutNode, LayoutOptions, Point, Port, PortSide, Size,
};
use common::*;

const SIDES: [PortSide; 4] = [PortSide::North, PortSide::East, PortSide::South, PortSide::West];

struct Case {
    graph: LayoutGraph,
    options: LayoutOptions,
    pins: Vec<String>,
}

fn generate(seed: u64, extra_nodes: u32) -> Case {
    let mut rng = Lcg::new(seed);
    let mut g = LayoutGraph::new();
    let groups: Vec<_> = (0..rng.below(4))
        .map(|i| {
            g.add_group(LayoutGroup {
                key: format!("g{i}"),
                padding: Insets::uniform(rng.below(16) as f32),
                header: rng.below(30) as f32,
            })
        })
        .collect();
    let n = 3 + rng.below(22) + extra_nodes;
    let mut ids = Vec::new();
    let mut first = Vec::new();
    for i in 0..n {
        let size = Size::new(10.0 + rng.below(90) as f32, 10.0 + rng.below(50) as f32);
        let mut node = LayoutNode::new(format!("n{i}"), size);
        if !groups.is_empty() && rng.chance(80) {
            node = node.in_group(groups[rng.below(groups.len() as u32) as usize]);
        }
        if rng.chance(25) {
            let ports = (0..1 + rng.below(3)).map(|_| Port { side: SIDES[rng.below(4) as usize] }).collect();
            node = node.with_ports(ports);
        }
        let is_first = i < 3 && rng.chance(20);
        if is_first {
            node = node.with_layer(LayerConstraint::First);
        }
        first.push(is_first);
        ids.push(g.add_node(node).expect("node"));
    }
    let m = n + rng.below(2 * n);
    for _ in 0..m {
        let a = ids[rng.below(n) as usize];
        let b = if rng.chance(5) { a } else { ids[rng.below(n) as usize] };
        // Two First nodes joined by an edge contradict each other (the
        // constraints test covers that error).
        if a != b && first[a.index()] && first[b.index()] {
            continue;
        }
        let end = |g: &LayoutGraph, rng: &mut Lcg, id| {
            let ports = g.node(id).ports.len() as u32;
            if ports > 0 && rng.chance(70) { EdgeEnd::port(id, rng.below(ports) as u16) } else { EdgeEnd::node(id) }
        };
        let source = end(&g, &mut rng, a);
        let target = end(&g, &mut rng, b);
        let mut e = LayoutEdge::new(source, target);
        if rng.chance(15) {
            e = e.with_label(Size::new(10.0 + rng.below(60) as f32, 8.0 + rng.below(10) as f32));
        }
        g.add_edge(e).expect("edge");
    }
    let options = LayoutOptions {
        direction: if rng.chance(25) { FlowDirection::TopToBottom } else { FlowDirection::LeftToRight },
        routing: if rng.chance(20) { EdgeRouting::Polyline } else { EdgeRouting::Orthogonal },
        ..LayoutOptions::default()
    };
    let pins = (0..n).filter(|_| rng.chance(6)).map(|i| format!("n{i}")).collect();
    Case { graph: g, options, pins }
}

fn checks_for(pinned: bool, stable: bool) -> Checks {
    Checks {
        overlaps: true,
        avoidance: true,
        foreign_groups: !pinned,
        // Pins may drag a group's rect over its neighbours.
        groups: !pinned,
        labels: !stable && !pinned,
    }
}

#[test]
fn generated_graphs_satisfy_invariants() {
    for seed in 0..120u64 {
        let case = generate(seed, 0);
        let mut hints = LayoutHints::default();
        let r = cascade_layout::layout(&case.graph, &case.options, &hints)
            .unwrap_or_else(|e| panic!("seed {seed}: layout failed: {e}"));
        if let Err(msg) = check(&case.graph, &case.options, &hints, &r, checks_for(false, false)) {
            panic!("seed {seed}: {msg}");
        }
        // Same graph with pins.
        if !case.pins.is_empty() {
            let mut rng = Lcg::new(seed ^ 0xABCD);
            for key in &case.pins {
                let p = Point::new(rng.below(800) as f32 - 100.0, rng.below(600) as f32 - 100.0);
                hints.pins.insert(key.clone(), p);
            }
            let rp = cascade_layout::layout(&case.graph, &case.options, &hints)
                .unwrap_or_else(|e| panic!("seed {seed}: pinned layout failed: {e}"));
            if let Err(msg) = check(&case.graph, &case.options, &hints, &rp, checks_for(true, false)) {
                panic!("seed {seed} (pinned): {msg}");
            }
        }
    }
}

#[test]
fn generated_edits_satisfy_invariants_and_determinism() {
    for seed in 200..260u64 {
        let before = generate(seed, 0);
        let after = generate(seed, 1);
        let r1 = cascade_layout::layout(&before.graph, &before.options, &LayoutHints::default())
            .unwrap_or_else(|e| panic!("seed {seed}: {e}"));
        let hints = LayoutHints { previous: Some(r1.to_previous(&before.graph)), ..LayoutHints::default() };
        let r2 = cascade_layout::layout(&after.graph, &after.options, &hints)
            .unwrap_or_else(|e| panic!("seed {seed}: relayout failed: {e}"));
        if let Err(msg) = check(&after.graph, &after.options, &hints, &r2, checks_for(false, true)) {
            panic!("seed {seed} (relayout): {msg}");
        }
        let again = cascade_layout::layout(&after.graph, &after.options, &hints).expect("relayout");
        assert_eq!(r2, again, "seed {seed}: relayout is not deterministic");
    }
}

#[test]
fn zero_spacings_still_satisfy_invariants() {
    let zero = LayoutOptions {
        node_spacing: 0.0,
        layer_spacing: 0.0,
        edge_spacing: 0.0,
        group_spacing: 0.0,
        ..LayoutOptions::default()
    };
    let no_edge_spacing = LayoutOptions { edge_spacing: 0.0, ..LayoutOptions::default() };
    for seed in 300..340u64 {
        let case = generate(seed, 0);
        // With zero node spacing nodes may touch, so a port facing a
        // neighbour cannot avoid it; everything else still holds.
        for (options, avoidance) in [(zero, false), (no_edge_spacing, true)] {
            let options = LayoutOptions { direction: case.options.direction, routing: case.options.routing, ..options };
            let r = cascade_layout::layout(&case.graph, &options, &LayoutHints::default())
                .unwrap_or_else(|e| panic!("seed {seed}: {e}"));
            let checks = Checks { labels: false, avoidance, ..Checks::ALL };
            if let Err(msg) = check(&case.graph, &options, &LayoutHints::default(), &r, checks) {
                panic!("seed {seed} {options:?}: {msg}");
            }
        }
    }
}
