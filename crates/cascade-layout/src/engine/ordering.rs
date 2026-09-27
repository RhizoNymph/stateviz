//! Crossing minimisation: order the items of every column so that links
//! between neighbouring columns cross as little as possible.
//!
//! Layer-by-layer sweeps (alternately left to right and right to left)
//! sort each column by the barycentre of its neighbours in the reference
//! column, port-aware: a neighbour's position is shifted by where on its
//! side the link attaches, so edges on a node's ports keep the ports'
//! order. A transpose pass then swaps adjacent items while that removes
//! crossings. The best ordering seen (by exact crossing count, using an
//! accumulator tree) is kept.

use super::layered::{BandGraph, ItemId, ItemKind};
use super::problem::Problem;
use super::slots::Slots;

const MAX_SWEEPS: usize = 24;
const MAX_STALE: usize = 4;
const TRANSPOSE_PASSES: usize = 4;
const TRANSPOSE_MAX_LAYER: usize = 400;

/// A link between neighbouring columns: `a` in the left column, `b` in the
/// right one, with where each end attaches (`off_*` in order units for
/// barycentres, `sub_*` as a sub-position for crossing counts).
#[derive(Clone, Copy, Debug)]
struct Link {
    a: ItemId,
    b: ItemId,
    off_a: f64,
    off_b: f64,
    sub_a: u64,
    sub_b: u64,
}

struct Links {
    /// Links between column `l` and `l + 1`, indexed by `l`.
    by_layer: Vec<Vec<Link>>,
    /// Per item: (link layer, link index) for links to the left column.
    left: Vec<Vec<(usize, usize)>>,
    /// Per item: links to the right column.
    right: Vec<Vec<(usize, usize)>>,
    /// Per item: partners in the same column (turnaround links).
    same: Vec<Vec<ItemId>>,
    /// Sub-position radix.
    radix: u64,
}

/// Where along its side a link attaches, for crossing counts.
#[derive(Clone, Copy, Debug)]
enum Sub {
    /// A North port: before everything on the side.
    Top,
    /// An explicit port, by its slot index.
    Port(usize),
    /// An unported end; these get sorted to match their neighbours.
    Implicit,
    /// A South port: after everything on the side.
    Bottom,
}

impl Sub {
    fn value(self, radix: u64) -> u64 {
        match self {
            Sub::Top => 0,
            Sub::Port(i) => (i as u64 + 1).min(radix - 3),
            Sub::Implicit => radix - 2,
            Sub::Bottom => radix - 1,
        }
    }
}

fn end_attachment(problem: &Problem<'_>, slots: &Slots, edge: usize, source: bool) -> (f64, Sub) {
    use super::frame::Side;
    let end = problem.edges[edge].end(source);
    let k = usize::from(!source);
    match (end.side, end.port.is_some()) {
        (Side::North, _) => (-0.5, Sub::Top),
        (Side::South, _) => (0.5, Sub::Bottom),
        (_, true) => (slots.fraction(problem, edge, source) - 0.5, Sub::Port(slots.index[edge][k])),
        (_, false) => (0.0, Sub::Implicit),
    }
}

impl Links {
    fn build(bg: &BandGraph, problem: &Problem<'_>, slots: &Slots) -> Self {
        let n = bg.items.len();
        let columns = bg.columns();
        let mut by_layer: Vec<Vec<Link>> = vec![Vec::new(); columns.saturating_sub(1)];
        let mut left = vec![Vec::new(); n];
        let mut right = vec![Vec::new(); n];
        let mut same = vec![Vec::new(); n];
        let mut radix = 2u64;
        for count in &slots.count {
            for c in count {
                radix = radix.max(*c as u64 + 3);
            }
        }
        for chain in &bg.chains {
            let last = chain.items.len() - 1;
            for (k, w) in chain.items.windows(2).enumerate() {
                let (x, y) = (w[0], w[1]);
                let attach = |item: ItemId, is_source_end: bool| -> (f64, u64) {
                    match bg.items[item].kind {
                        ItemKind::Node(_) => {
                            let (off, sub) = end_attachment(problem, slots, chain.edge, is_source_end);
                            (off, sub.value(radix))
                        }
                        _ => (0.0, radix / 2),
                    }
                };
                let (off_x, sub_x) = if k == 0 { attach(x, true) } else { (0.0, radix / 2) };
                let (off_y, sub_y) = if k + 1 == last { attach(y, false) } else { (0.0, radix / 2) };
                let (lx, ly) = (bg.items[x].layer, bg.items[y].layer);
                if lx == ly {
                    same[x].push(y);
                    same[y].push(x);
                    continue;
                }
                let link = if lx < ly {
                    Link { a: x, b: y, off_a: off_x, off_b: off_y, sub_a: sub_x, sub_b: sub_y }
                } else {
                    Link { a: y, b: x, off_a: off_y, off_b: off_x, sub_a: sub_y, sub_b: sub_x }
                };
                let l = bg.items[link.a].layer;
                right[link.a].push((l, by_layer[l].len()));
                left[link.b].push((l, by_layer[l].len()));
                by_layer[l].push(link);
            }
        }
        Self { by_layer, left, right, same, radix }
    }
}

/// Order the band's columns in place. `seed` gives previous orders of real
/// nodes, which (when present) seed the initial order.
pub(crate) fn minimize(bg: &mut BandGraph, problem: &Problem<'_>, slots: &Slots, seed: &dyn Fn(usize) -> Option<u32>) {
    let links = Links::build(bg, problem, slots);
    initial_order(bg, &links, seed);
    let mut pos = positions(bg);
    let mut best = count_all(&links, &pos);
    let mut best_layers = bg.layers.clone();
    let mut stale = 0;
    for sweep in 0..MAX_SWEEPS {
        if best == 0 {
            break;
        }
        let down = sweep % 2 == 0;
        let columns = bg.columns();
        let order: Vec<usize> =
            if down { (1..columns).collect() } else { (0..columns.saturating_sub(1)).rev().collect() };
        for l in order {
            reorder(bg, &links, &mut pos, l, down);
        }
        for l in 0..bg.columns() {
            transpose(bg, &links, &mut pos, l);
        }
        let c = count_all(&links, &pos);
        if c < best {
            best = c;
            best_layers = bg.layers.clone();
            stale = 0;
        } else {
            stale += 1;
            if stale >= MAX_STALE {
                break;
            }
        }
    }
    bg.layers = best_layers;
}

fn positions(bg: &BandGraph) -> Vec<usize> {
    let mut pos = vec![0usize; bg.items.len()];
    for layer in &bg.layers {
        for (i, &item) in layer.iter().enumerate() {
            pos[item] = i;
        }
    }
    pos
}

/// Depth-first order along links, starting from the leftmost columns;
/// previous orders (if any) then decide the relative order of old nodes.
fn initial_order(bg: &mut BandGraph, links: &Links, seed: &dyn Fn(usize) -> Option<u32>) {
    let n = bg.items.len();
    let mut roots: Vec<ItemId> = (0..n).collect();
    roots.sort_by_key(|&i| (bg.items[i].layer, !bg.items[i].is_node(), i));
    let mut visited = vec![false; n];
    let mut layers: Vec<Vec<ItemId>> = vec![Vec::new(); bg.columns()];
    for root in roots {
        if visited[root] {
            continue;
        }
        let mut stack = vec![root];
        while let Some(v) = stack.pop() {
            if visited[v] {
                continue;
            }
            visited[v] = true;
            layers[bg.items[v].layer].push(v);
            for &(l, i) in links.right[v].iter().rev() {
                let b = links.by_layer[l][i].b;
                if !visited[b] {
                    stack.push(b);
                }
            }
        }
    }
    for layer in &mut layers {
        let slots_of_old: Vec<usize> =
            (0..layer.len()).filter(|&i| bg.items[layer[i]].node().and_then(seed).is_some()).collect();
        let mut old: Vec<ItemId> = slots_of_old.iter().map(|&i| layer[i]).collect();
        old.sort_by_key(|&item| (bg.items[item].node().and_then(seed), item));
        for (slot, item) in slots_of_old.into_iter().zip(old) {
            layer[slot] = item;
        }
    }
    bg.layers = layers;
}

fn reorder(bg: &mut BandGraph, links: &Links, pos: &mut [usize], l: usize, down: bool) {
    let layer = &bg.layers[l];
    let mut keyed: Vec<(usize, Option<f64>)> = Vec::with_capacity(layer.len());
    for (i, &item) in layer.iter().enumerate() {
        let refs = if down { &links.left[item] } else { &links.right[item] };
        let bary = if refs.is_empty() {
            let partners = &links.same[item];
            if partners.is_empty() {
                None
            } else {
                Some(partners.iter().map(|&p| pos[p] as f64).sum::<f64>() / partners.len() as f64)
            }
        } else {
            let sum: f64 = refs
                .iter()
                .map(|&(ll, li)| {
                    let link = &links.by_layer[ll][li];
                    if down { pos[link.a] as f64 + link.off_a } else { pos[link.b] as f64 + link.off_b }
                })
                .sum();
            Some(sum / refs.len() as f64)
        };
        keyed.push((i, bary));
    }
    // Items without a barycentre keep their slots; the others are sorted
    // (stably) into the remaining slots.
    let mut movable: Vec<(f64, usize)> = keyed.iter().filter_map(|&(i, b)| b.map(|b| (b, i))).collect();
    movable.sort_by(|x, y| x.0.total_cmp(&y.0).then(x.1.cmp(&y.1)));
    let old = layer.clone();
    let mut next = movable.into_iter();
    let mut new_layer = Vec::with_capacity(old.len());
    for &(i, b) in &keyed {
        if b.is_none() {
            new_layer.push(old[i]);
        } else if let Some((_, j)) = next.next() {
            new_layer.push(old[j]);
        }
    }
    for (i, &item) in new_layer.iter().enumerate() {
        pos[item] = i;
    }
    bg.layers[l] = new_layer;
}

fn neighbour_keys(links: &Links, pos: &[usize], item: ItemId, left: bool) -> Vec<u64> {
    let refs = if left { &links.left[item] } else { &links.right[item] };
    refs.iter()
        .map(|&(l, i)| {
            let link = &links.by_layer[l][i];
            if left {
                pos[link.a] as u64 * links.radix + link.sub_a
            } else {
                pos[link.b] as u64 * links.radix + link.sub_b
            }
        })
        .collect()
}

fn pair_crossings(u: &[u64], v: &[u64]) -> (u64, u64) {
    let (mut uv, mut vu) = (0, 0);
    for &x in u {
        for &y in v {
            if x > y {
                uv += 1;
            } else if x < y {
                vu += 1;
            }
        }
    }
    (uv, vu)
}

fn transpose(bg: &mut BandGraph, links: &Links, pos: &mut [usize], l: usize) {
    let len = bg.layers[l].len();
    if !(2..=TRANSPOSE_MAX_LAYER).contains(&len) {
        return;
    }
    let keys: Vec<(Vec<u64>, Vec<u64>)> = bg.layers[l]
        .iter()
        .map(|&item| (neighbour_keys(links, pos, item, true), neighbour_keys(links, pos, item, false)))
        .collect();
    let mut order: Vec<usize> = (0..len).collect();
    for _ in 0..TRANSPOSE_PASSES {
        let mut improved = false;
        for i in 0..len - 1 {
            let (u, v) = (order[i], order[i + 1]);
            let (a1, b1) = pair_crossings(&keys[u].0, &keys[v].0);
            let (a2, b2) = pair_crossings(&keys[u].1, &keys[v].1);
            if b1 + b2 < a1 + a2 {
                order.swap(i, i + 1);
                improved = true;
            }
        }
        if !improved {
            break;
        }
    }
    let old = bg.layers[l].clone();
    bg.layers[l] = order.iter().map(|&i| old[i]).collect();
    for (i, &item) in bg.layers[l].iter().enumerate() {
        pos[item] = i;
    }
}

fn count_all(links: &Links, pos: &[usize]) -> u64 {
    links
        .by_layer
        .iter()
        .map(|layer| {
            let pairs: Vec<(u64, u64)> = layer
                .iter()
                .map(|lk| (pos[lk.a] as u64 * links.radix + lk.sub_a, pos[lk.b] as u64 * links.radix + lk.sub_b))
                .collect();
            count_inversions(pairs)
        })
        .sum()
}

/// Number of pairs `(i, j)` with `a_i < a_j` and `b_i > b_j` (strictly), by
/// an accumulator tree over the `b` values.
pub(crate) fn count_inversions(mut pairs: Vec<(u64, u64)>) -> u64 {
    if pairs.len() < 2 {
        return 0;
    }
    pairs.sort_unstable();
    let mut values: Vec<u64> = pairs.iter().map(|p| p.1).collect();
    values.sort_unstable();
    values.dedup();
    let mut tree = vec![0u64; values.len() + 1];
    let mut total = 0u64;
    let mut i = 0;
    let mut inserted = 0u64;
    while i < pairs.len() {
        // Pairs sharing `a` never cross each other.
        let mut j = i;
        while j < pairs.len() && pairs[j].0 == pairs[i].0 {
            let rank = values.partition_point(|&v| v <= pairs[j].1);
            let mut not_greater = 0u64;
            let mut k = rank;
            while k > 0 {
                not_greater += tree[k];
                k &= k - 1;
            }
            total += inserted - not_greater;
            j += 1;
        }
        for p in &pairs[i..j] {
            let mut k = values.partition_point(|&v| v < p.1) + 1;
            while k < tree.len() {
                tree[k] += 1;
                k += k & k.wrapping_neg();
            }
            inserted += 1;
        }
        i = j;
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;

    fn brute(pairs: &[(u64, u64)]) -> u64 {
        let mut c = 0;
        for (i, a) in pairs.iter().enumerate() {
            for b in &pairs[i + 1..] {
                if (a.0 < b.0 && a.1 > b.1) || (a.0 > b.0 && a.1 < b.1) {
                    c += 1;
                }
            }
        }
        c
    }

    #[test]
    fn inversions_match_brute_force() {
        let mut seed = 3u64;
        for _ in 0..200 {
            let mut pairs = Vec::new();
            for _ in 0..(seed % 12) {
                seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                pairs.push(((seed >> 40) % 5, (seed >> 20) % 5));
            }
            seed = seed.wrapping_add(17);
            assert_eq!(count_inversions(pairs.clone()), brute(&pairs), "{pairs:?}");
        }
    }

    #[test]
    fn pair_crossings_counts_both_orders() {
        assert_eq!(pair_crossings(&[5, 7], &[6]), (1, 1));
        assert_eq!(pair_crossings(&[1], &[2, 3]), (0, 2));
    }
}
