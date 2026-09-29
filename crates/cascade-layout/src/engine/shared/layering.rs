//! One layering for the whole graph, so layers mean the same in every band.
//!
//! Edges between bands take part like any other edge (minimum length 1;
//! labelled edges inside a band keep length 2 for their label column).
//! A relayout keeps previous layers as soft fixes. An acyclic edge between
//! two nodes kept at their previous layers can then point backwards (a new
//! edge, or a new node pushing its successors right). Here that would send
//! an edge between bands against the flow, so the soft fix of its target
//! (else its source) is released and the layering runs again, until no
//! acyclic edge points backwards or only hard fixes remain in the way.

use super::super::LayoutError;
use super::super::layering::{self as band_layering, LayerEdge};
use super::super::problem::{EdgeKind, Fix, Problem};

/// Layers of every node and which edges lie on a cycle.
pub(crate) fn assign(p: &Problem<'_>) -> Result<(Vec<u32>, Vec<bool>), LayoutError> {
    let mut fixes: Vec<Fix> = p.nodes.iter().map(|n| n.fix).collect();
    let mut ids = Vec::with_capacity(p.edges.len());
    let mut edges = Vec::with_capacity(p.edges.len());
    for (e, edge) in p.edges.iter().enumerate() {
        let (s, t) = (edge.source.node, edge.target.node);
        if s == t {
            continue;
        }
        let labelled = edge.label.is_some() && p.kinds[e] == EdgeKind::Chain;
        ids.push(e);
        edges.push(LayerEdge { source: s, target: t, minlen: if labelled { 2 } else { 1 } });
    }
    let key = |v: usize| p.key(v).to_string();
    loop {
        let layering = band_layering::assign(&fixes, &edges, &key)?;
        let mut released = false;
        for (k, le) in edges.iter().enumerate() {
            if layering.on_cycle[k] || layering.layers[le.target] > layering.layers[le.source] {
                continue;
            }
            for v in [le.target, le.source] {
                if matches!(fixes[v], Fix::Soft(_)) {
                    fixes[v] = Fix::Free;
                    released = true;
                    break;
                }
            }
        }
        if !released {
            let mut on_cycle = vec![false; p.edges.len()];
            for (k, &e) in ids.iter().enumerate() {
                on_cycle[e] = layering.on_cycle[k];
            }
            return Ok((layering.layers, on_cycle));
        }
    }
}
