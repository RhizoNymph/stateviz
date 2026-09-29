//! Readability of routes on graphs shaped like the build canvas: stacked
//! machine lanes with the wiring either in gutters between them or in one
//! band below. `cargo test -p cascade-layout --test readability -- --nocapture`
//! prints the report.

mod canvas;
mod common;

use canvas::{Wiring, canvas};
use cascade_layout::metrics::{RouteMetrics, measure};
use cascade_layout::{FlowDirection, LayoutHints, LayoutOptions};
use common::*;

fn options(align: bool) -> LayoutOptions {
    LayoutOptions { layer_spacing: 48.0, align_across_groups: align, ..LayoutOptions::default() }
}

fn metrics_of(lanes: usize, wiring: Wiring, seed: u64, align: bool) -> RouteMetrics {
    let c = canvas(lanes, wiring, seed);
    let opts = options(align);
    let r = run_with(&c.graph, &opts, &LayoutHints::default());
    let checks = Checks { labels: false, ..Checks::ALL };
    if let Err(msg) = check(&c.graph, &opts, &LayoutHints::default(), &r, checks) {
        panic!("{lanes} lanes {wiring:?} seed {seed} align {align}: {msg}");
    }
    if let Some(dir) = std::env::var_os("CASCADE_LAYOUT_SVG") {
        let name = format!("canvas-{lanes}-{wiring:?}-{seed}-{}.svg", if align { "aligned" } else { "plain" });
        let path = std::path::Path::new(&dir).join(name);
        if let Err(err) = std::fs::write(&path, canvas::to_svg(&c.graph, &r)) {
            panic!("cannot write {}: {err}", path.display());
        }
    }
    measure(&c.graph, &r, FlowDirection::LeftToRight)
}

fn total(lanes: usize, wiring: Wiring, align: bool) -> RouteMetrics {
    let mut sum = RouteMetrics::default();
    for seed in 1..=4 {
        let m = metrics_of(lanes, wiring, seed, align);
        sum.edges += m.edges;
        sum.crossings += m.crossings;
        sum.corridor_edges += m.corridor_edges;
        sum.total_length += m.total_length;
        sum.bends += m.bends;
        sum.label_overlaps += m.label_overlaps;
        sum.unplaced_labels += m.unplaced_labels;
    }
    sum
}

#[test]
fn report() {
    for wiring in [Wiring::Band, Wiring::Gutters] {
        for align in [false, true] {
            println!("6 lanes {:<8} align {:<5} {}", format!("{wiring:?}"), align, total(6, wiring, align));
        }
    }
}

/// Before this work (one left corridor for every edge between groups that
/// are not neighbours, labels beside the longest segment; the option was
/// accepted and ignored): the sums over seeds 1–4 of the 6-lane canvases.
const BAND_BEFORE: RouteMetrics = RouteMetrics {
    edges: 492,
    crossings: 2801,
    corridor_edges: 111,
    total_length: 419_600.0,
    bends: 1442,
    label_overlaps: 7,
    unplaced_labels: 0,
};
const GUTTERS_BEFORE: RouteMetrics = RouteMetrics {
    edges: 492,
    crossings: 699,
    corridor_edges: 40,
    total_length: 271_588.0,
    bends: 1316,
    label_overlaps: 3,
    unplaced_labels: 0,
};

#[test]
fn canvases_need_no_corridor_and_cross_less() {
    for (wiring, before) in [(Wiring::Band, BAND_BEFORE), (Wiring::Gutters, GUTTERS_BEFORE)] {
        let m = total(6, wiring, true);
        assert_eq!(m.corridor_edges, 0, "{wiring:?}: {m}");
        assert!(m.crossings < before.crossings, "{wiring:?}: {m} vs {before}");
        assert!(m.total_length < before.total_length, "{wiring:?}: {m} vs {before}");
        assert_eq!((m.label_overlaps, m.unplaced_labels), (0, 0), "{wiring:?}: {m}");
    }
}

#[test]
fn alignment_shortens_routes_between_gutters_and_lanes() {
    let plain = total(6, Wiring::Gutters, false);
    let aligned = total(6, Wiring::Gutters, true);
    assert!(aligned.total_length < plain.total_length, "{aligned} vs {plain}");
    assert!(aligned.bends <= plain.bends, "{aligned} vs {plain}");
}

/// Two lanes: a chain in the upper one ends in a pill whose South port
/// feeds the first node of the lower one, far to the left.
fn stacked(with_middle: bool) -> (cascade_layout::LayoutGraph, cascade_layout::EdgeId) {
    use cascade_layout::{EdgeEnd, Insets, LayoutEdge, LayoutGraph, LayoutGroup, LayoutNode, Port, PortSide, Size};
    let mut g = LayoutGraph::new();
    let lane = |g: &mut LayoutGraph, key: &str| {
        g.add_group(LayoutGroup { key: key.to_string(), padding: Insets::uniform(8.0), header: 20.0 })
    };
    let top = lane(&mut g, "top");
    let middle = with_middle.then(|| lane(&mut g, "middle"));
    let bottom = lane(&mut g, "bottom");
    let ports = || {
        vec![
            Port { side: PortSide::West },
            Port { side: PortSide::East },
            Port { side: PortSide::North },
            Port { side: PortSide::South },
        ]
    };
    let chain: Vec<_> = (0..4)
        .map(|i| {
            g.add_node(LayoutNode::new(format!("t{i}"), Size::new(80.0, 30.0)).in_group(top).with_ports(ports()))
                .expect("t")
        })
        .collect();
    for w in chain.windows(2) {
        g.add_edge(LayoutEdge::new(EdgeEnd::port(w[0], 1), EdgeEnd::port(w[1], 0))).expect("chain");
    }
    if let Some(m) = middle {
        let a = g.add_node(LayoutNode::new("m0", Size::new(60.0, 30.0)).in_group(m)).expect("m0");
        let b = g.add_node(LayoutNode::new("m1", Size::new(60.0, 30.0)).in_group(m)).expect("m1");
        edge(&mut g, a, b);
    }
    let b0 = g.add_node(LayoutNode::new("b0", Size::new(80.0, 30.0)).in_group(bottom).with_ports(ports())).expect("b0");
    let b1 = g.add_node(LayoutNode::new("b1", Size::new(80.0, 30.0)).in_group(bottom).with_ports(ports())).expect("b1");
    g.add_edge(LayoutEdge::new(EdgeEnd::port(b0, 1), EdgeEnd::port(b1, 0))).expect("b chain");
    let cross = g.add_edge(LayoutEdge::new(EdgeEnd::port(chain[3], 3), EdgeEnd::port(b0, 2))).expect("cross");
    (g, cross)
}

#[test]
fn aligned_neighbouring_groups_get_one_straight_run() {
    let (g, cross) = stacked(false);
    let plain = run_with(&g, &options(false), &LayoutHints::default());
    assert!(plain.edge(cross).points.len() > 2, "unaligned: {:?}", plain.edge(cross).points);
    let aligned = run_with(&g, &options(true), &LayoutHints::default());
    assert_ok(&g, &options(true), &LayoutHints::default(), &aligned);
    let pts = &aligned.edge(cross).points;
    assert_eq!(pts.len(), 2, "one vertical run: {pts:?}");
    assert!((pts[0].x - pts[1].x).abs() < 0.01, "{pts:?}");
}

#[test]
fn a_group_in_between_is_crossed_through_a_passage_not_a_corridor() {
    let (g, cross) = stacked(true);
    for align in [false, true] {
        let r = run_with(&g, &options(align), &LayoutHints::default());
        assert_ok(&g, &options(align), &LayoutHints::default(), &r);
        assert_eq!(cascade_layout::metrics::corridor_edges(&g, &r, FlowDirection::LeftToRight), 0);
        let middle = r.group(g.groups().nth(1).expect("middle").0);
        let pts = &r.edge(cross).points;
        let across = pts.windows(2).any(|w| {
            (w[0].x - w[1].x).abs() < 0.01
                && w[0].y.min(w[1].y) <= middle.top()
                && w[0].y.max(w[1].y) >= middle.bottom()
        });
        assert!(across, "align {align}: {pts:?} does not cross {middle:?}");
        if align {
            // Aligned, the whole route is one straight vertical.
            assert_eq!(pts.len(), 2, "{pts:?}");
        }
    }
}
