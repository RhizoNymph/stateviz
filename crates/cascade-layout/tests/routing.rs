//! Edge routing: orthogonal channel routing with separated parallel segments,
//! and polyline routing through dummy points.

mod common;

use cascade_layout::{EdgeRouting, LayoutGraph, LayoutHints, LayoutOptions, Point};
use common::*;

#[test]
fn polyline_routes_pass_through_layers() {
    let mut g = LayoutGraph::new();
    let a = node(&mut g, "a", 40.0, 20.0);
    let b = node(&mut g, "b", 40.0, 20.0);
    let c = node(&mut g, "c", 40.0, 20.0);
    let d = node(&mut g, "d", 40.0, 20.0);
    edge(&mut g, a, b);
    edge(&mut g, b, c);
    edge(&mut g, c, d);
    let long = edge(&mut g, a, d);
    let back = edge(&mut g, d, b);
    let options = LayoutOptions { routing: EdgeRouting::Polyline, ..LayoutOptions::default() };
    let r = run_with(&g, &options, &LayoutHints::default());
    assert_ok(&g, &options, &LayoutHints::default(), &r);
    // Straight segments through the layers: the long edge bends toward its
    // target's port rather than running orthogonally.
    assert!(r.edge(long).points.len() >= 3, "{:?}", r.edge(long).points);
    let diagonal = g.edges().any(|(e, _)| {
        r.edge(e).points.windows(2).any(|w| (w[0].x - w[1].x).abs() > EPS && (w[0].y - w[1].y).abs() > EPS)
    });
    assert!(diagonal, "polyline routing produced only orthogonal segments");
    assert!(r.edge(back).reversed);
}

#[test]
fn parallel_vertical_segments_are_separated() {
    // A fan-out and fan-in between two layers forces several vertical
    // segments into one channel.
    let mut g = LayoutGraph::new();
    let srcs: Vec<_> = (0..4).map(|i| node(&mut g, &format!("s{i}"), 40.0, 20.0)).collect();
    let dsts: Vec<_> = (0..4).map(|i| node(&mut g, &format!("d{i}"), 40.0, 20.0)).collect();
    for (i, s) in srcs.iter().enumerate() {
        edge(&mut g, *s, dsts[(i + 1) % 4]);
        edge(&mut g, *s, dsts[(i + 2) % 4]);
    }
    let options = LayoutOptions::default();
    let r = run_with(&g, &options, &LayoutHints::default());
    assert_ok(&g, &options, &LayoutHints::default(), &r);
    let mut verticals: Vec<(f32, f32, f32, usize)> = Vec::new();
    for (e, _) in g.edges() {
        for w in r.edge(e).points.windows(2) {
            if (w[0].x - w[1].x).abs() < EPS && (w[0].y - w[1].y).abs() > EPS {
                verticals.push((w[0].x, w[0].y.min(w[1].y), w[0].y.max(w[1].y), e.index()));
            }
        }
    }
    for (i, a) in verticals.iter().enumerate() {
        for b in &verticals[i + 1..] {
            if a.3 == b.3 {
                continue;
            }
            let overlap = a.1 < b.2 - EPS && b.1 < a.2 - EPS;
            if overlap {
                assert!((a.0 - b.0).abs() >= options.edge_spacing - EPS, "segments {a:?} and {b:?} too close");
            }
        }
    }
}

#[test]
fn routes_have_no_redundant_points() {
    let mut g = LayoutGraph::new();
    let n: Vec<_> = (0..6).map(|i| node(&mut g, &format!("n{i}"), 40.0, 20.0 + 6.0 * i as f32)).collect();
    for (x, y) in [(0, 1), (1, 2), (2, 3), (0, 3), (3, 4), (4, 5), (5, 1), (0, 5)] {
        edge(&mut g, n[x], n[y]);
    }
    let r = run(&g);
    assert_ok(&g, &LayoutOptions::default(), &LayoutHints::default(), &r);
    for (e, _) in g.edges() {
        let pts = &r.edge(e).points;
        for w in pts.windows(2) {
            assert!(w[0] != w[1], "duplicate point in {pts:?}");
        }
        for w in pts.windows(3) {
            let collinear_x = (w[0].x - w[1].x).abs() < EPS && (w[1].x - w[2].x).abs() < EPS;
            let collinear_y = (w[0].y - w[1].y).abs() < EPS && (w[1].y - w[2].y).abs() < EPS;
            let between = |a: f32, m: f32, b: f32| (a - m) * (m - b) >= 0.0;
            let redundant =
                (collinear_x && between(w[0].y, w[1].y, w[2].y)) || (collinear_y && between(w[0].x, w[1].x, w[2].x));
            assert!(!redundant, "redundant bend in {pts:?}");
        }
    }
}

#[test]
fn long_edges_are_straight_when_unobstructed() {
    let mut g = LayoutGraph::new();
    let a = node(&mut g, "a", 40.0, 20.0);
    let b = node(&mut g, "b", 40.0, 20.0);
    let c = node(&mut g, "c", 40.0, 20.0);
    let d = node(&mut g, "d", 40.0, 20.0);
    let x = node(&mut g, "x", 40.0, 20.0);
    edge(&mut g, a, b);
    edge(&mut g, b, c);
    edge(&mut g, c, d);
    let long = edge(&mut g, x, d);
    edge(&mut g, a, x);
    let r = run(&g);
    assert_ok(&g, &LayoutOptions::default(), &LayoutHints::default(), &r);
    // A long edge from x to d bends at most twice.
    assert!(r.edge(long).points.len() <= 4, "{:?}", r.edge(long).points);
    let _ = Point::default();
}
