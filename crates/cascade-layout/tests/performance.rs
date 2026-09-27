//! Performance: the spec's target is under one second for 500 nodes in a
//! release build. Debug builds get a generous budget so the test is stable
//! on slow machines; the ignored test prints the release number.

mod common;

use std::time::{Duration, Instant};

use cascade_layout::{
    Insets, LayerConstraint, LayoutGraph, LayoutGroup, LayoutHints, LayoutNode, LayoutOptions, LayoutResult, Size,
};
use common::*;

/// A causal-view-like graph: 500 nodes in rough causal depth order, ~800
/// edges mostly one to three layers forward, a few back edges (cascade
/// cycles) and some external sources in the first layer.
fn causal_like(groups: usize) -> LayoutGraph {
    let mut rng = Lcg::new(42);
    let mut g = LayoutGraph::new();
    let lanes: Vec<_> = (0..groups)
        .map(|i| g.add_group(LayoutGroup { key: format!("lane{i}"), padding: Insets::uniform(10.0), header: 20.0 }))
        .collect();
    let n = 500usize;
    let mut ids = Vec::with_capacity(n);
    for i in 0..n {
        let size = Size::new(60.0 + rng.below(120) as f32, 24.0 + rng.below(20) as f32);
        let mut node = LayoutNode::new(format!("n{i}"), size);
        if i % 50 == 0 {
            node = node.with_layer(LayerConstraint::First);
        }
        if !lanes.is_empty() {
            node = node.in_group(lanes[i * lanes.len() / n]);
        }
        ids.push(g.add_node(node).expect("node"));
    }
    let mut added = 0;
    while added < 800 {
        let a = rng.below(n as u32) as usize;
        let b = if rng.chance(4) {
            a.saturating_sub(1 + rng.below(40) as usize)
        } else {
            (a + 1 + rng.below(25) as usize).min(n - 1)
        };
        if a == b || (b == n - 1 && rng.chance(80)) {
            continue;
        }
        edge(&mut g, ids[a], ids[b]);
        added += 1;
    }
    g
}

fn timed(g: &LayoutGraph, hints: &LayoutHints) -> (Duration, LayoutResult) {
    let start = Instant::now();
    let r = cascade_layout::layout(g, &LayoutOptions::default(), hints).expect("layout");
    (start.elapsed(), r)
}

#[test]
fn five_hundred_nodes_within_debug_budget() {
    let budget = if cfg!(debug_assertions) { Duration::from_secs(5) } else { Duration::from_secs(1) };
    for groups in [0, 8] {
        let g = causal_like(groups);
        let (elapsed, r) = timed(&g, &LayoutHints::default());
        assert!(elapsed < budget, "{groups} groups: fresh layout took {elapsed:?}");
        let checks = Checks { labels: false, ..Checks::ALL };
        if let Err(msg) = check(&g, &LayoutOptions::default(), &LayoutHints::default(), &r, checks) {
            panic!("{groups} groups: {msg}");
        }
        let hints = LayoutHints { previous: Some(r.to_previous(&g)), ..LayoutHints::default() };
        let (elapsed, _) = timed(&g, &hints);
        assert!(elapsed < budget, "{groups} groups: stable relayout took {elapsed:?}");
    }
}

/// Run with `cargo test -p cascade-layout --release --test performance -- --ignored --nocapture`.
#[test]
#[ignore = "timing report; run in release"]
fn five_hundred_nodes_release_timing() {
    for groups in [0, 8] {
        let g = causal_like(groups);
        let mut best = Duration::MAX;
        let mut r = None;
        for _ in 0..5 {
            let (elapsed, result) = timed(&g, &LayoutHints::default());
            best = best.min(elapsed);
            r = Some(result);
        }
        let r = r.expect("ran");
        let hints = LayoutHints { previous: Some(r.to_previous(&g)), ..LayoutHints::default() };
        let (stable, _) = timed(&g, &hints);
        println!("500 nodes / {} edges, {groups} groups: fresh {best:?}, stable relayout {stable:?}", g.edge_count());
        assert!(best < Duration::from_secs(1));
    }
}
