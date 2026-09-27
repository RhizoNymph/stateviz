//! The layout entry point.
//!
//! Stub engine: longest-path layering after DFS cycle breaking, layers
//! ordered by previous order then insertion order, straight two-point edges.
//! Owner: the `feat/layered-layout` workstream replaces this with the full
//! layered pipeline (crossing minimisation, coordinate assignment, orthogonal
//! routing, ports, groups routed around, stability, pins).

use std::collections::HashMap;

use crate::geometry::{Point, Rect};
use crate::graph::{GroupId, LayerConstraint, LayoutGraph, NodeId, PortSide};
use crate::options::{FlowDirection, LayoutHints, LayoutOptions};
use crate::result::{EdgeRoute, LayoutResult, NodePlacement};

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum LayoutError {
    /// Two constraints cannot both hold, e.g. an `Exact` layer that an edge
    /// forces to be earlier than its source.
    #[error("cannot satisfy layout constraints for `{key}`: {reason}")]
    Unsatisfiable { key: String, reason: String },
}

/// Lay out `graph`. Deterministic: the same inputs always give the same
/// output.
pub fn layout(graph: &LayoutGraph, options: &LayoutOptions, hints: &LayoutHints) -> Result<LayoutResult, LayoutError> {
    let n = graph.node_count();
    let reversed = back_edges(graph);

    // Longest-path layering over the acyclic orientation.
    let mut preds: Vec<Vec<usize>> = vec![Vec::new(); n];
    for (eid, edge) in graph.edges() {
        let (s, t) = (edge.source.node.index(), edge.target.node.index());
        if s == t {
            continue;
        }
        if reversed[eid.index()] {
            preds[s].push(t);
        } else {
            preds[t].push(s);
        }
    }
    let mut layer: Vec<Option<u32>> = vec![None; n];
    for i in 0..n {
        assign_layer(i, graph, &preds, &mut layer);
    }
    let layer: Vec<u32> = layer.into_iter().map(|l| l.unwrap_or(0)).collect();

    // Partition: ungrouped nodes first, then each group in order.
    let mut bands: Vec<(Option<GroupId>, Vec<usize>)> = vec![(None, Vec::new())];
    for (gid, _) in graph.groups() {
        bands.push((Some(gid), Vec::new()));
    }
    for (id, node) in graph.nodes() {
        let band = node.group.map_or(0, |g| g.index() + 1);
        bands[band].1.push(id.index());
    }

    let mut rects = vec![Rect::default(); n];
    let mut order = vec![0u32; n];
    let mut groups = vec![Rect::default(); graph.group_count()];
    let mut band_top = 0.0f32;
    for (group, members) in &bands {
        if members.is_empty() {
            if let Some(g) = group {
                groups[g.index()] = Rect::new(0.0, band_top, 0.0, 0.0);
            }
            continue;
        }
        let (pad, header) = group.map_or((Default::default(), 0.0), |g| {
            let g = graph.group(g);
            (g.padding, g.header)
        });

        let mut by_layer: HashMap<u32, Vec<usize>> = HashMap::new();
        for &m in members {
            by_layer.entry(layer[m]).or_default().push(m);
        }
        let mut layers: Vec<u32> = by_layer.keys().copied().collect();
        layers.sort_unstable();
        let max_layer = layers.last().copied().unwrap_or(0);

        // Main-axis extent of each layer across the whole graph keeps
        // layers aligned between bands.
        let mut layer_extent = vec![0.0f32; (max_layer + 1) as usize];
        for (i, &l) in layer.iter().enumerate() {
            if let Some(ext) = layer_extent.get_mut(l as usize) {
                let size = graph.node(NodeId::new(i)).size;
                let main = match options.direction {
                    FlowDirection::LeftToRight => size.width,
                    FlowDirection::TopToBottom => size.height,
                };
                *ext = ext.max(main);
            }
        }
        let mut layer_start = vec![0.0f32; layer_extent.len()];
        let mut acc = 0.0;
        for (l, ext) in layer_extent.iter().enumerate() {
            layer_start[l] = acc;
            acc += ext + options.layer_spacing;
        }

        for l in &layers {
            let Some(nodes) = by_layer.get_mut(l) else { continue };
            nodes.sort_by_key(|&i| {
                let key = &graph.node(NodeId::new(i)).key;
                let prev = hints.previous.as_ref().and_then(|p| p.nodes.get(key)).map(|p| p.order);
                (prev.unwrap_or(u32::MAX), i)
            });
            let mut cross = 0.0f32;
            for (pos, &i) in nodes.iter().enumerate() {
                let size = graph.node(NodeId::new(i)).size;
                let main = layer_start[*l as usize];
                let (x, y, cross_size) = match options.direction {
                    FlowDirection::LeftToRight => (main, cross, size.height),
                    FlowDirection::TopToBottom => (cross, main, size.width),
                };
                rects[i] = Rect::new(x + pad.left, y + band_top + header + pad.top, size.width, size.height);
                order[i] = u32::try_from(pos).unwrap_or(u32::MAX);
                cross += cross_size + options.node_spacing;
            }
        }

        let band_rect = members.iter().map(|&i| rects[i]).reduce(|a, b| a.union(&b)).unwrap_or_default();
        if let Some(g) = group {
            let r = Rect::new(0.0, band_top, band_rect.right() + pad.right, band_rect.bottom() - band_top + pad.bottom);
            groups[g.index()] = r;
            band_top = r.bottom() + options.group_spacing;
        } else {
            band_top = band_top.max(band_rect.bottom()) + options.group_spacing;
        }
    }

    for (id, node) in graph.nodes() {
        if let Some(&pin) = hints.pins.get(&node.key) {
            rects[id.index()].origin = pin;
        }
    }

    let edges = graph
        .edges()
        .map(|(eid, edge)| {
            let from = attach(graph, &rects, edge.source.node, edge.source.port, true, options.direction);
            let to = attach(graph, &rects, edge.target.node, edge.target.port, false, options.direction);
            EdgeRoute {
                points: vec![from, to],
                reversed: reversed[eid.index()],
                label: edge.label.map(|size| {
                    let mid = Point::new((from.x + to.x) / 2.0, (from.y + to.y) / 2.0);
                    Rect::new(mid.x - size.width / 2.0, mid.y - size.height, size.width, size.height)
                }),
            }
        })
        .collect::<Vec<_>>();

    let mut bounds: Option<Rect> = None;
    let mut grow = |r: Rect| bounds = Some(bounds.map_or(r, |b| b.union(&r)));
    rects.iter().copied().for_each(&mut grow);
    groups.iter().copied().for_each(&mut grow);
    for route in &edges {
        for p in &route.points {
            grow(Rect::from_origin_size(*p, Default::default()));
        }
    }

    let nodes = (0..n).map(|i| NodePlacement { rect: rects[i], layer: layer[i], order: order[i] }).collect();
    Ok(LayoutResult::new(nodes, edges, groups, bounds.unwrap_or_default()))
}

fn assign_layer(i: usize, graph: &LayoutGraph, preds: &[Vec<usize>], layer: &mut [Option<u32>]) -> u32 {
    if let Some(l) = layer[i] {
        return l;
    }
    // Mark in progress to stay total on malformed input.
    layer[i] = Some(0);
    let from_preds = preds[i].iter().map(|&p| assign_layer(p, graph, preds, layer) + 1).max().unwrap_or(0);
    let l = match graph.node(NodeId::new(i)).layer {
        LayerConstraint::Free => from_preds,
        LayerConstraint::First => 0,
        LayerConstraint::Exact(l) => l,
    };
    layer[i] = Some(l);
    l
}

/// Edges that close a cycle in DFS order (insertion order of nodes and
/// edges), which the layering treats as reversed.
fn back_edges(graph: &LayoutGraph) -> Vec<bool> {
    #[derive(Clone, Copy, PartialEq)]
    enum Mark {
        New,
        Active,
        Done,
    }
    let n = graph.node_count();
    let mut out: Vec<Vec<(usize, usize)>> = vec![Vec::new(); n];
    for (eid, edge) in graph.edges() {
        out[edge.source.node.index()].push((eid.index(), edge.target.node.index()));
    }
    let mut mark = vec![Mark::New; n];
    let mut reversed = vec![false; graph.edge_count()];
    for root in 0..n {
        if mark[root] != Mark::New {
            continue;
        }
        let mut stack = vec![(root, 0usize)];
        mark[root] = Mark::Active;
        while let Some(&mut (v, ref mut next)) = stack.last_mut() {
            if let Some(&(e, w)) = out[v].get(*next) {
                *next += 1;
                match mark[w] {
                    Mark::New => {
                        mark[w] = Mark::Active;
                        stack.push((w, 0));
                    }
                    Mark::Active => reversed[e] = true,
                    Mark::Done => {}
                }
            } else {
                mark[v] = Mark::Done;
                stack.pop();
            }
        }
    }
    reversed
}

fn attach(
    graph: &LayoutGraph,
    rects: &[Rect],
    node: NodeId,
    port: Option<u16>,
    outgoing: bool,
    direction: FlowDirection,
) -> Point {
    let r = rects[node.index()];
    let side = port.and_then(|p| graph.node(node).ports.get(usize::from(p))).map(|p| p.side).unwrap_or(
        match (direction, outgoing) {
            (FlowDirection::LeftToRight, true) => PortSide::East,
            (FlowDirection::LeftToRight, false) => PortSide::West,
            (FlowDirection::TopToBottom, true) => PortSide::South,
            (FlowDirection::TopToBottom, false) => PortSide::North,
        },
    );
    let c = r.center();
    match side {
        PortSide::East => Point::new(r.right(), c.y),
        PortSide::West => Point::new(r.left(), c.y),
        PortSide::North => Point::new(c.x, r.top()),
        PortSide::South => Point::new(c.x, r.bottom()),
    }
}
