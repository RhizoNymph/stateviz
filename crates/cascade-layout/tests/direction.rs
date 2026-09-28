//! Top-to-bottom flow is the left-to-right layout transposed.

mod common;

use cascade_layout::{FlowDirection, LayoutGraph, LayoutHints, LayoutOptions};
use common::*;

#[test]
fn top_to_bottom_chain_runs_downward() {
    let mut g = LayoutGraph::new();
    let ids: Vec<_> = (0..4).map(|i| node(&mut g, &format!("n{i}"), 80.0, 20.0)).collect();
    for w in ids.windows(2) {
        edge(&mut g, w[0], w[1]);
    }
    edge(&mut g, ids[3], ids[0]);
    let options = LayoutOptions { direction: FlowDirection::TopToBottom, ..LayoutOptions::default() };
    let r = run_with(&g, &options, &LayoutHints::default());
    assert_ok(&g, &options, &LayoutHints::default(), &r);
    for w in ids.windows(2) {
        let (a, b) = (r.node(w[0]).rect, r.node(w[1]).rect);
        assert!(b.top() >= a.bottom() + options.layer_spacing - EPS, "{a:?} {b:?}");
    }
}

#[test]
fn top_to_bottom_spreads_layers_horizontally() {
    let mut g = LayoutGraph::new();
    let root = node(&mut g, "root", 40.0, 20.0);
    let kids: Vec<_> = (0..3).map(|i| node(&mut g, &format!("k{i}"), 60.0, 20.0)).collect();
    for k in &kids {
        edge(&mut g, root, *k);
    }
    let options = LayoutOptions { direction: FlowDirection::TopToBottom, ..LayoutOptions::default() };
    let r = run_with(&g, &options, &LayoutHints::default());
    assert_ok(&g, &options, &LayoutHints::default(), &r);
    let mut rects: Vec<_> = kids.iter().map(|k| r.node(*k).rect).collect();
    rects.sort_by(|a, b| a.left().total_cmp(&b.left()));
    for w in rects.windows(2) {
        assert!(w[1].left() - w[0].right() >= options.node_spacing - EPS);
        assert!((w[0].top() - w[1].top()).abs() < EPS || (w[0].center().y - w[1].center().y).abs() < EPS);
    }
}
