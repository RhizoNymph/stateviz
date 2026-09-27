//! Groups (lanes): stacked in insertion order, containing their nodes, never
//! overlapping, with edges routed around foreign groups.

mod common;

use cascade_layout::{
    EdgeEnd, FlowDirection, Insets, LayoutEdge, LayoutGraph, LayoutGroup, LayoutHints, LayoutNode, LayoutOptions, Size,
};
use common::*;

fn lanes(n: usize, per_lane: usize) -> (LayoutGraph, Vec<Vec<cascade_layout::NodeId>>) {
    let mut g = LayoutGraph::new();
    let groups: Vec<_> = (0..n)
        .map(|i| g.add_group(LayoutGroup { key: format!("lane{i}"), padding: Insets::uniform(12.0), header: 22.0 }))
        .collect();
    let mut members = Vec::new();
    for (i, gid) in groups.iter().enumerate() {
        let ids: Vec<_> = (0..per_lane)
            .map(|j| {
                g.add_node(LayoutNode::new(format!("l{i}n{j}"), Size::new(50.0, 24.0)).in_group(*gid)).expect("node")
            })
            .collect();
        for w in ids.windows(2) {
            edge(&mut g, w[0], w[1]);
        }
        members.push(ids);
    }
    (g, members)
}

#[test]
fn groups_stack_in_insertion_order_and_contain_their_nodes() {
    let (g, members) = lanes(3, 4);
    let r = run(&g);
    assert_ok(&g, &LayoutOptions::default(), &LayoutHints::default(), &r);
    let rects: Vec<_> = g.groups().map(|(id, _)| r.group(id)).collect();
    for w in rects.windows(2) {
        assert!(w[1].top() >= w[0].bottom() + LayoutOptions::default().group_spacing - EPS, "{rects:?}");
    }
    for (i, (gid, group)) in g.groups().enumerate() {
        let gr = r.group(gid);
        for id in &members[i] {
            let nr = r.node(*id).rect;
            assert!(nr.top() >= gr.top() + group.header + group.padding.top - EPS, "header space: {gr:?} {nr:?}");
            assert!(nr.left() >= gr.left() + group.padding.left - EPS);
            assert!(nr.right() <= gr.right() - group.padding.right + EPS);
            assert!(nr.bottom() <= gr.bottom() - group.padding.bottom + EPS);
        }
    }
    // Lanes share a common width.
    for w in rects.windows(2) {
        assert!((w[0].left() - w[1].left()).abs() < EPS && (w[0].right() - w[1].right()).abs() < EPS);
    }
}

#[test]
fn groups_are_laid_out_independently() {
    let (g, members) = lanes(2, 3);
    let r = run(&g);
    // Each lane starts at layer 0.
    assert_eq!(r.node(members[0][0]).layer, 0);
    assert_eq!(r.node(members[1][0]).layer, 0);
}

#[test]
fn ungrouped_nodes_form_their_own_band_above_the_groups() {
    let (mut g, members) = lanes(2, 3);
    let free = node(&mut g, "free", 40.0, 20.0);
    edge(&mut g, free, members[1][1]);
    let r = run(&g);
    assert_ok(&g, &LayoutOptions::default(), &LayoutHints::default(), &r);
    let first_group = r.group(g.groups().next().expect("group").0);
    assert!(r.node(free).rect.bottom() <= first_group.top() + EPS);
}

#[test]
fn cross_group_edges_route_around_foreign_groups() {
    let (mut g, members) = lanes(4, 4);
    // Skip over lanes in both directions, from the middle of the lanes.
    edge(&mut g, members[0][1], members[3][2]);
    edge(&mut g, members[3][0], members[0][3]);
    edge(&mut g, members[1][2], members[2][1]);
    edge(&mut g, members[2][3], members[0][0]);
    edge(&mut g, members[0][2], members[2][2]);
    let r = run(&g);
    assert_ok(&g, &LayoutOptions::default(), &LayoutHints::default(), &r);
}

#[test]
fn cross_group_edges_with_ports_and_labels() {
    let (mut g, members) = lanes(3, 3);
    let hub = g
        .add_node(
            LayoutNode::new("hub", Size::new(60.0, 40.0))
                .with_ports(vec![cascade_layout::Port { side: cascade_layout::PortSide::East }]),
        )
        .expect("hub");
    for lane in &members {
        g.add_edge(LayoutEdge::new(EdgeEnd::port(hub, 0), EdgeEnd::node(lane[1])).with_label(Size::new(40.0, 12.0)))
            .expect("edge");
    }
    let r = run(&g);
    let checks = Checks { labels: false, ..Checks::ALL };
    if let Err(msg) = check(&g, &LayoutOptions::default(), &LayoutHints::default(), &r, checks) {
        panic!("{msg}");
    }
}

#[test]
fn empty_groups_still_get_a_lane() {
    let mut g = LayoutGraph::new();
    let a = g.add_group(LayoutGroup { key: "a".into(), padding: Insets::uniform(5.0), header: 20.0 });
    let empty = g.add_group(LayoutGroup { key: "empty".into(), padding: Insets::uniform(5.0), header: 20.0 });
    let c = g.add_group(LayoutGroup { key: "c".into(), padding: Insets::uniform(5.0), header: 20.0 });
    g.add_node(LayoutNode::new("x", Size::new(30.0, 30.0)).in_group(a)).expect("x");
    g.add_node(LayoutNode::new("y", Size::new(30.0, 30.0)).in_group(c)).expect("y");
    let r = run(&g);
    assert_ok(&g, &LayoutOptions::default(), &LayoutHints::default(), &r);
    let (ra, re, rc) = (r.group(a), r.group(empty), r.group(c));
    assert!(re.top() >= ra.bottom() && rc.top() >= re.bottom());
    assert!(re.size.height >= 30.0 - EPS, "header and padding: {re:?}");
}

#[test]
fn groups_top_to_bottom_sit_side_by_side() {
    let (g, _) = lanes(3, 3);
    let options = LayoutOptions { direction: FlowDirection::TopToBottom, ..LayoutOptions::default() };
    let r = run_with(&g, &options, &LayoutHints::default());
    assert_ok(&g, &options, &LayoutHints::default(), &r);
    let rects: Vec<_> = g.groups().map(|(id, _)| r.group(id)).collect();
    for w in rects.windows(2) {
        assert!(w[1].left() >= w[0].right() - EPS, "{rects:?}");
    }
}
