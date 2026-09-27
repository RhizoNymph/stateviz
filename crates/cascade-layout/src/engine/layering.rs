//! Layer assignment for one band, honouring fixed layers.
//!
//! 1. Strongly connected components tell which edges lie on a cycle.
//! 2. An acyclic edge between two hard-fixed nodes that does not point
//!    forward is a contradiction: `LayoutError::Unsatisfiable`.
//! 3. The Eades–Lin–Smyth arrangement (per component, components in
//!    topological order) orients every other edge: the desired direction
//!    "`a` before `b`". Edges between two fixed nodes take no part.
//! 4. A relaxation pass in arrangement order computes each node's earliest
//!    layer; a desired edge into a fixed node that cannot be satisfied is
//!    dropped (the edge will run against the flow) rather than failing.
//! 5. Network simplex minimises total edge length, with fixed nodes merged
//!    into one super-node per fixed layer tied to a root by heavy edges.
//!
//! Fixed layers are compressed first (gaps capped by what free nodes could
//! ever need), so `Exact(4_000_000_000)` costs nothing, and mapped back
//! afterwards.

use std::collections::BTreeMap;

use super::LayoutError;
use super::cycles::{greedy_arrangement, strongly_connected};
use super::network_simplex::{NsEdge, solve};
use super::problem::Fix;

/// An edge between two band-local nodes (never a self-loop).
#[derive(Clone, Copy, Debug)]
pub(crate) struct LayerEdge {
    pub source: usize,
    pub target: usize,
    /// Minimum layer distance when the edge points forward: 2 for labelled
    /// edges, so the label gets a layer of its own.
    pub minlen: u32,
}

#[derive(Clone, Debug)]
pub(crate) struct Layering {
    /// Reported layer of each band-local node.
    pub layers: Vec<u32>,
    /// Whether each edge lies on a cycle (both ends in one component).
    pub on_cycle: Vec<bool>,
}

const MAX_PIVOTS_PER_ELEMENT: usize = 8;

pub(crate) fn assign(
    fixes: &[Fix],
    edges: &[LayerEdge],
    key: &dyn Fn(usize) -> String,
) -> Result<Layering, LayoutError> {
    let n = fixes.len();
    let mut out: Vec<Vec<usize>> = vec![Vec::new(); n];
    for e in edges {
        out[e.source].push(e.target);
    }
    let (comp, comps) = strongly_connected(&out);
    let on_cycle: Vec<bool> = edges.iter().map(|e| comp[e.source] == comp[e.target]).collect();

    for (e, cyc) in edges.iter().zip(&on_cycle) {
        if let (Fix::Hard(a), Fix::Hard(b)) = (fixes[e.source], fixes[e.target])
            && b <= a
            && !cyc
        {
            return Err(LayoutError::Unsatisfiable {
                key: key(e.target),
                reason: format!(
                    "it is fixed at layer {b}, but an edge from `{}` (fixed at layer {a}) must point forward",
                    key(e.source)
                ),
            });
        }
    }

    // Arrangement: components in topological order (descending number),
    // each ordered by the greedy heuristic.
    let mut members: Vec<Vec<usize>> = vec![Vec::new(); comps];
    for v in 0..n {
        members[comp[v]].push(v);
    }
    let mut arrangement = Vec::with_capacity(n);
    let mut local = vec![0usize; n];
    for c in (0..comps).rev() {
        let nodes = &members[c];
        if nodes.len() == 1 {
            arrangement.push(nodes[0]);
            continue;
        }
        for (i, &v) in nodes.iter().enumerate() {
            local[v] = i;
        }
        let inner: Vec<(usize, usize)> = edges
            .iter()
            .filter(|e| comp[e.source] == c && comp[e.target] == c)
            .map(|e| (local[e.source], local[e.target]))
            .collect();
        arrangement.extend(greedy_arrangement(nodes.len(), &inner).into_iter().map(|i| nodes[i]));
    }
    let mut pos = vec![0usize; n];
    for (i, &v) in arrangement.iter().enumerate() {
        pos[v] = i;
    }

    // Desired orientation of every edge with at least one free end.
    let desired: Vec<(usize, usize, i64)> = edges
        .iter()
        .zip(&on_cycle)
        .filter(|(e, _)| fixes[e.source].value().is_none() || fixes[e.target].value().is_none())
        .map(|(e, &cyc)| {
            let (a, b) =
                if !cyc || pos[e.source] < pos[e.target] { (e.source, e.target) } else { (e.target, e.source) };
            (a, b, i64::from(e.minlen))
        })
        .collect();

    // Compress fixed layers.
    let free = fixes.iter().filter(|f| f.value().is_none()).count() as u64;
    let cap = 2 * free + 2;
    let mut fixed_values: Vec<u32> = fixes.iter().filter_map(|f| f.value()).collect();
    fixed_values.sort_unstable();
    fixed_values.dedup();
    let mut compressed: Vec<u64> = Vec::with_capacity(fixed_values.len());
    for (i, &v) in fixed_values.iter().enumerate() {
        let c = match i {
            0 => u64::from(v).min(cap),
            _ => compressed[i - 1] + u64::from(v - fixed_values[i - 1]).min(cap),
        };
        compressed.push(c);
    }
    let compress = |v: u32| -> i64 {
        let i = fixed_values.binary_search(&v).unwrap_or(0);
        i64::try_from(compressed[i]).unwrap_or(i64::MAX / 4)
    };

    // Relaxation: earliest layers, dropping unsatisfiable edges into fixed
    // nodes. Every desired edge points forward in the arrangement, so the
    // arrangement is a topological order.
    let mut incoming: Vec<Vec<usize>> = vec![Vec::new(); n];
    for (i, &(_, b, _)) in desired.iter().enumerate() {
        incoming[b].push(i);
    }
    let mut lb = vec![0i64; n];
    let mut dropped = vec![false; desired.len()];
    for &v in &arrangement {
        match fixes[v].value() {
            Some(f) => {
                let cv = compress(f);
                for &d in &incoming[v] {
                    let (a, _, len) = desired[d];
                    if lb[a] + len > cv {
                        dropped[d] = true;
                    }
                }
                lb[v] = cv;
            }
            None => {
                lb[v] = incoming[v].iter().map(|&d| lb[desired[d].0] + desired[d].2).max().unwrap_or(0);
            }
        }
    }

    // Network simplex over: root, one super-node per fixed value, free
    // nodes.
    let supers = fixed_values.len();
    let ns_index = |v: usize, free_index: &[usize]| match fixes[v].value() {
        Some(f) => 1 + fixed_values.binary_search(&f).unwrap_or(0),
        None => 1 + supers + free_index[v],
    };
    let mut free_index = vec![0usize; n];
    let mut free_nodes = Vec::new();
    for v in 0..n {
        if fixes[v].value().is_none() {
            free_index[v] = free_nodes.len();
            free_nodes.push(v);
        }
    }
    let ns_n = 1 + supers + free_nodes.len();
    let mut merged: BTreeMap<(usize, usize), (i64, i64)> = BTreeMap::new();
    let mut total_weight = 0i64;
    for (i, &(a, b, len)) in desired.iter().enumerate() {
        if dropped[i] {
            continue;
        }
        let (ta, tb) = (ns_index(a, &free_index), ns_index(b, &free_index));
        if ta == tb {
            continue;
        }
        let entry = merged.entry((ta, tb)).or_insert((0, 0));
        entry.0 = entry.0.max(len);
        entry.1 += 1;
        total_weight += 1;
    }
    let max_lb = lb.iter().copied().max().unwrap_or(0);
    let heavy = (total_weight + 1).saturating_mul(max_lb + 2);
    for (s, &c) in compressed.iter().enumerate() {
        merged.insert((0, 1 + s), (i64::try_from(c).unwrap_or(i64::MAX / 4), heavy));
    }
    for (i, _) in free_nodes.iter().enumerate() {
        merged.entry((0, 1 + supers + i)).or_insert((0, 0));
    }
    let ns_edges: Vec<NsEdge> =
        merged.into_iter().map(|((tail, head), (minlen, weight))| NsEdge { tail, head, minlen, weight }).collect();
    let mut init = vec![0i64; ns_n];
    for (s, &c) in compressed.iter().enumerate() {
        init[1 + s] = i64::try_from(c).unwrap_or(i64::MAX / 4);
    }
    for (i, &v) in free_nodes.iter().enumerate() {
        init[1 + supers + i] = lb[v];
    }
    let pivots = MAX_PIVOTS_PER_ELEMENT * (ns_n + ns_edges.len()) + 64;
    let mut rank = solve(ns_n, &ns_edges, &init, pivots);
    let tight = compressed.iter().enumerate().all(|(s, &c)| i64::try_from(c).is_ok_and(|c| rank[1 + s] - rank[0] == c));
    let feasible = ns_edges.iter().all(|e| rank[e.head] - rank[e.tail] >= e.minlen);
    if !(tight && feasible) {
        rank = init;
    }

    // Map compressed ranks back to reported layers.
    let decompress = |c: i64| -> u32 {
        let c = u64::try_from(c.max(0)).unwrap_or(0);
        if fixed_values.is_empty() {
            return u32::try_from(c).unwrap_or(u32::MAX);
        }
        let i = compressed.partition_point(|&x| x <= c);
        let layer = if i == 0 {
            u64::from(fixed_values[0]).saturating_sub(compressed[0] - c)
        } else {
            u64::from(fixed_values[i - 1]) + (c - compressed[i - 1])
        };
        u32::try_from(layer).unwrap_or(u32::MAX)
    };
    let layers = (0..n)
        .map(|v| match fixes[v].value() {
            Some(f) => f,
            None => decompress(rank[1 + supers + free_index[v]] - rank[0]),
        })
        .collect();
    Ok(Layering { layers, on_cycle })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(source: usize, target: usize) -> LayerEdge {
        LayerEdge { source, target, minlen: 1 }
    }

    fn key(i: usize) -> String {
        format!("n{i}")
    }

    #[test]
    fn free_chain_gets_consecutive_layers() {
        let l = assign(&[Fix::Free; 4], &[e(0, 1), e(1, 2), e(2, 3)], &key).expect("layers");
        assert_eq!(l.layers, vec![0, 1, 2, 3]);
        assert!(l.on_cycle.iter().all(|c| !c));
    }

    #[test]
    fn sources_are_pulled_to_their_successors() {
        // 0->1->2->3 and 4->3: 4 should sit at layer 2, not 0.
        let l = assign(&[Fix::Free; 5], &[e(0, 1), e(1, 2), e(2, 3), e(4, 3)], &key).expect("layers");
        assert_eq!(l.layers[4], 2);
    }

    #[test]
    fn labelled_edges_span_two_layers() {
        let edges = [LayerEdge { source: 0, target: 1, minlen: 2 }];
        let l = assign(&[Fix::Free; 2], &edges, &key).expect("layers");
        assert_eq!(l.layers[1] - l.layers[0], 2);
    }

    #[test]
    fn cycles_are_detected_and_broken() {
        let l = assign(&[Fix::Free; 3], &[e(0, 1), e(1, 2), e(2, 0)], &key).expect("layers");
        assert!(l.on_cycle.iter().all(|&c| c));
        let backward = [(0, 1), (1, 2), (2, 0)].iter().filter(|&&(a, b)| l.layers[b] <= l.layers[a]).count();
        assert_eq!(backward, 1);
    }

    #[test]
    fn exact_layers_with_free_nodes_between() {
        let fixes = [Fix::Hard(0), Fix::Free, Fix::Free, Fix::Hard(6)];
        let l = assign(&fixes, &[e(0, 1), e(1, 2), e(2, 3)], &key).expect("layers");
        assert_eq!(l.layers[0], 0);
        assert_eq!(l.layers[3], 6);
        assert!(l.layers[1] > 0 && l.layers[1] < l.layers[2] && l.layers[2] < 6);
    }

    #[test]
    fn huge_exact_values_are_compressed() {
        let fixes = [Fix::Hard(3_000_000_000), Fix::Free, Fix::Hard(4_000_000_000)];
        let l = assign(&fixes, &[e(0, 1), e(1, 2)], &key).expect("layers");
        assert_eq!(l.layers[0], 3_000_000_000);
        assert_eq!(l.layers[2], 4_000_000_000);
        assert!(l.layers[1] > 3_000_000_000 && l.layers[1] < 4_000_000_000);
    }

    #[test]
    fn contradicting_hard_layers_fail_unless_on_a_cycle() {
        let err = assign(&[Fix::Hard(2), Fix::Hard(1)], &[e(0, 1)], &key);
        assert!(matches!(err, Err(LayoutError::Unsatisfiable { key, .. }) if key == "n1"));
        let ok = assign(&[Fix::Hard(2), Fix::Hard(1)], &[e(0, 1), e(1, 0)], &key).expect("cycle");
        assert_eq!(ok.layers, vec![2, 1]);
    }

    #[test]
    fn soft_conflicts_relax() {
        // A new edge between soft-fixed nodes pointing backwards is kept.
        let l = assign(&[Fix::Soft(3), Fix::Soft(1), Fix::Free], &[e(0, 1), e(0, 2)], &key).expect("layers");
        assert_eq!((l.layers[0], l.layers[1]), (3, 1));
        assert!(l.layers[2] > 3);
    }

    #[test]
    fn free_node_squeezed_between_adjacent_fixed_layers_is_relaxed() {
        let fixes = [Fix::Hard(1), Fix::Free, Fix::Hard(2)];
        let l = assign(&fixes, &[e(0, 1), e(1, 2)], &key).expect("relaxed");
        assert_eq!((l.layers[0], l.layers[2]), (1, 2));
    }
}
