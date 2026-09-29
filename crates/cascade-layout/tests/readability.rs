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
