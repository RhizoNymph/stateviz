//! Performance of pinning a busy node: the build canvas's wiring band with
//! the controller that has the most edges dragged to spots across the
//! canvas, so its edges and every route it lands on go through the obstacle
//! router.

mod canvas;
mod common;

use std::time::{Duration, Instant};

use canvas::{Wiring, canvas};
use cascade_layout::{LayoutGraph, LayoutHints, LayoutOptions, Point};
use common::*;

fn options() -> LayoutOptions {
    LayoutOptions { layer_spacing: 48.0, ..LayoutOptions::default() }
}

/// The canvas, its busiest controller's key and where to pin it: inside the
/// lanes, beside them, and at the bottom.
fn pinned_cases() -> (LayoutGraph, String, Vec<Point>) {
    let c = canvas(6, Wiring::Band, 3);
    let g = c.graph;
    let mut degree = vec![0usize; g.node_count()];
    for (_, e) in g.edges() {
        degree[e.source.node.index()] += 1;
        degree[e.target.node.index()] += 1;
    }
    let busiest = g
        .nodes()
        .filter(|(_, n)| n.key.starts_with('c'))
        .max_by_key(|(id, _)| (degree[id.index()], std::cmp::Reverse(id.index())))
        .map(|(_, n)| n.key.clone())
        .expect("a controller");
    let free = run_with(&g, &options(), &LayoutHints::default());
    let b = free.bounds;
    let at = |fx: f32, fy: f32| Point::new(b.left() + fx * b.size.width, b.top() + fy * b.size.height);
    let spots = vec![at(0.5, 0.35), at(0.8, 0.2), at(0.25, 0.6), at(-0.1, 0.5), at(0.6, 0.95)];
    (g, busiest, spots)
}

fn timed(g: &LayoutGraph, key: &str, spot: Point) -> Duration {
    let mut hints = LayoutHints::default();
    hints.pins.insert(key.to_string(), spot);
    let start = Instant::now();
    let r = run_with(g, &options(), &hints);
    let elapsed = start.elapsed();
    let checks = Checks { foreign_groups: false, groups: false, labels: false, ..Checks::ALL };
    if let Err(msg) = check(g, &options(), &hints, &r, checks) {
        panic!("pinned at {spot:?}: {msg}");
    }
    elapsed
}

#[test]
fn pinning_a_busy_controller_stays_fast() {
    let budget = if cfg!(debug_assertions) { Duration::from_secs(2) } else { Duration::from_millis(100) };
    let (g, key, spots) = pinned_cases();
    for spot in spots {
        let elapsed = timed(&g, &key, spot);
        assert!(elapsed < budget, "pinned at {spot:?}: {elapsed:?}");
    }
}

/// Run with `cargo test -p cascade-layout --release --test pins_performance -- --ignored --nocapture`.
#[test]
#[ignore = "timing report; run in release"]
fn pinning_release_timing() {
    let (g, key, spots) = pinned_cases();
    let unpinned = {
        let start = Instant::now();
        let _ = run_with(&g, &options(), &LayoutHints::default());
        start.elapsed()
    };
    println!("{} nodes / {} edges, unpinned {unpinned:?}", g.node_count(), g.edge_count());
    for spot in spots {
        let best = (0..3).map(|_| timed(&g, &key, spot)).min().unwrap_or_default();
        println!("  {key} pinned at ({:.0}, {:.0}): {best:?}", spot.x, spot.y);
    }
}
