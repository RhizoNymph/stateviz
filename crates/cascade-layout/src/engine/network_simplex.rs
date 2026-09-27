//! Network simplex for layer assignment (Gansner et al., "A Technique for
//! Drawing Directed Graphs", 1993): minimise `Σ weight · (rank[head] −
//! rank[tail])` subject to `rank[head] − rank[tail] ≥ minlen`.
//!
//! Cut values are recomputed from scratch after every pivot (O(V + E) each),
//! which keeps the implementation simple; the pivot count is capped, and any
//! intermediate solution is feasible, so hitting the cap only costs
//! compactness.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct NsEdge {
    pub tail: usize,
    pub head: usize,
    pub minlen: i64,
    pub weight: i64,
}

const NONE: usize = usize::MAX;

struct Tree {
    parent: Vec<usize>,
    parent_edge: Vec<usize>,
    low: Vec<usize>,
    lim: Vec<usize>,
    /// Nodes in DFS preorder from the root.
    preorder: Vec<usize>,
    /// Nodes in postorder (children before parents).
    postorder: Vec<usize>,
}

fn slack(rank: &[i64], e: &NsEdge) -> i64 {
    rank[e.head] - rank[e.tail] - e.minlen
}

fn other(e: &NsEdge, v: usize) -> usize {
    if e.tail == v { e.head } else { e.tail }
}

/// Solve the layering LP. `init` must be feasible and the graph connected
/// (ignoring edge directions); otherwise `init` is returned unchanged
/// except for the tight-tree shifts, which preserve feasibility.
pub(crate) fn solve(n: usize, edges: &[NsEdge], init: &[i64], max_pivots: usize) -> Vec<i64> {
    let mut rank = init.to_vec();
    if n <= 1 {
        return rank;
    }
    let m = edges.len();
    let mut inc: Vec<Vec<usize>> = vec![Vec::new(); n];
    for (i, e) in edges.iter().enumerate() {
        inc[e.tail].push(i);
        inc[e.head].push(i);
    }

    // Feasible spanning tree of tight edges.
    let mut in_tree = vec![false; n];
    let mut tree_edge = vec![false; m];
    let mut count = 1usize;
    in_tree[0] = true;
    let grow = |start: usize, in_tree: &mut [bool], tree_edge: &mut [bool], rank: &[i64], count: &mut usize| {
        let mut stack = vec![start];
        while let Some(v) = stack.pop() {
            for &ei in &inc[v] {
                let e = &edges[ei];
                let w = other(e, v);
                if !in_tree[w] && slack(rank, e) == 0 {
                    tree_edge[ei] = true;
                    in_tree[w] = true;
                    *count += 1;
                    stack.push(w);
                }
            }
        }
    };
    grow(0, &mut in_tree, &mut tree_edge, &rank, &mut count);
    while count < n {
        let mut best: Option<(i64, usize)> = None;
        for (i, e) in edges.iter().enumerate() {
            if in_tree[e.tail] != in_tree[e.head] {
                let s = slack(&rank, e);
                if best.is_none_or(|(bs, _)| s < bs) {
                    best = Some((s, i));
                }
            }
        }
        let Some((s, i)) = best else { return rank };
        let e = edges[i];
        let delta = if in_tree[e.tail] { s } else { -s };
        for v in 0..n {
            if in_tree[v] {
                rank[v] += delta;
            }
        }
        let w = if in_tree[e.tail] { e.head } else { e.tail };
        tree_edge[i] = true;
        in_tree[w] = true;
        count += 1;
        grow(w, &mut in_tree, &mut tree_edge, &rank, &mut count);
    }

    let mut tree = build_tree(n, edges, &inc, &tree_edge);
    let mut cut = vec![0i64; m];
    cut_values(edges, &inc, &tree_edge, &tree, &mut cut);

    for _ in 0..max_pivots {
        let Some(leave) = (0..m).find(|&e| tree_edge[e] && cut[e] < 0) else { break };
        let Some(enter) = enter_edge(edges, &tree_edge, &tree, &rank, leave) else { break };
        tree_edge[leave] = false;
        tree_edge[enter] = true;
        tree = build_tree(n, edges, &inc, &tree_edge);
        for &v in &tree.preorder {
            let p = tree.parent[v];
            if p == NONE {
                continue;
            }
            let e = &edges[tree.parent_edge[v]];
            rank[v] = if e.tail == p { rank[p] + e.minlen } else { rank[p] - e.minlen };
        }
        cut_values(edges, &inc, &tree_edge, &tree, &mut cut);
    }
    rank
}

fn build_tree(n: usize, edges: &[NsEdge], inc: &[Vec<usize>], tree_edge: &[bool]) -> Tree {
    let mut tree = Tree {
        parent: vec![NONE; n],
        parent_edge: vec![NONE; n],
        low: vec![0; n],
        lim: vec![0; n],
        preorder: Vec::with_capacity(n),
        postorder: Vec::with_capacity(n),
    };
    let mut visited = vec![false; n];
    let mut next_lim = 1usize;
    let mut stack: Vec<(usize, usize)> = vec![(0, 0)];
    visited[0] = true;
    tree.low[0] = next_lim;
    tree.preorder.push(0);
    while let Some(&mut (v, ref mut i)) = stack.last_mut() {
        if let Some(&ei) = inc[v].get(*i) {
            *i += 1;
            if !tree_edge[ei] {
                continue;
            }
            let w = other(&edges[ei], v);
            if visited[w] {
                continue;
            }
            visited[w] = true;
            tree.parent[w] = v;
            tree.parent_edge[w] = ei;
            tree.low[w] = next_lim;
            tree.preorder.push(w);
            stack.push((w, 0));
        } else {
            tree.lim[v] = next_lim;
            next_lim += 1;
            tree.postorder.push(v);
            stack.pop();
        }
    }
    tree
}

/// Cut values of all tree edges, computed bottom-up from each child's
/// incident edges and its children's cut values.
fn cut_values(edges: &[NsEdge], inc: &[Vec<usize>], tree_edge: &[bool], tree: &Tree, cut: &mut [i64]) {
    for &child in &tree.postorder {
        let pe = tree.parent_edge[child];
        if pe == NONE {
            continue;
        }
        let child_is_tail = edges[pe].tail == child;
        let mut value = edges[pe].weight;
        for &fi in &inc[child] {
            if fi == pe {
                continue;
            }
            let f = &edges[fi];
            let is_out = f.tail == child;
            let points_to_head = is_out == child_is_tail;
            value += if points_to_head { f.weight } else { -f.weight };
            if tree_edge[fi] {
                value += if points_to_head { -cut[fi] } else { cut[fi] };
            }
        }
        cut[pe] = value;
    }
}

fn enter_edge(edges: &[NsEdge], tree_edge: &[bool], tree: &Tree, rank: &[i64], leave: usize) -> Option<usize> {
    let e = &edges[leave];
    let tail_is_child = tree.parent_edge[e.tail] == leave;
    let child = if tail_is_child { e.tail } else { e.head };
    let (low, lim) = (tree.low[child], tree.lim[child]);
    let inside = |x: usize| low <= tree.lim[x] && tree.lim[x] <= lim;
    let mut best: Option<(i64, usize)> = None;
    for (i, f) in edges.iter().enumerate() {
        if tree_edge[i] {
            continue;
        }
        let ok = if tail_is_child { !inside(f.tail) && inside(f.head) } else { inside(f.tail) && !inside(f.head) };
        if ok {
            let s = slack(rank, f);
            if best.is_none_or(|(bs, _)| s < bs) {
                best = Some((s, i));
            }
        }
    }
    best.map(|(_, i)| i)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cost(edges: &[NsEdge], rank: &[i64]) -> i64 {
        edges.iter().map(|e| e.weight * (rank[e.head] - rank[e.tail])).sum()
    }

    fn feasible(edges: &[NsEdge], rank: &[i64]) -> bool {
        edges.iter().all(|e| rank[e.head] - rank[e.tail] >= e.minlen)
    }

    fn longest_path(n: usize, edges: &[NsEdge]) -> Vec<i64> {
        let mut rank = vec![0i64; n];
        for _ in 0..n {
            for e in edges {
                rank[e.head] = rank[e.head].max(rank[e.tail] + e.minlen);
            }
        }
        rank
    }

    fn brute_force(n: usize, edges: &[NsEdge], max: i64) -> i64 {
        let mut best = i64::MAX;
        let mut rank = vec![0i64; n];
        loop {
            if feasible(edges, &rank) {
                best = best.min(cost(edges, &rank));
            }
            let mut i = 1;
            loop {
                if i == n {
                    return best;
                }
                rank[i] += 1;
                if rank[i] <= max {
                    break;
                }
                rank[i] = 0;
                i += 1;
            }
        }
    }

    fn edge(tail: usize, head: usize) -> NsEdge {
        NsEdge { tail, head, minlen: 1, weight: 1 }
    }

    #[test]
    fn pulls_sources_toward_their_successors() {
        // 0 -> 1 -> 2 -> 3, 0 -> 4 and a heavier 4 -> 3: longest path puts
        // 4 at 1; the optimum pulls it next to 3.
        let heavy = NsEdge { tail: 4, head: 3, minlen: 1, weight: 2 };
        let edges = [edge(0, 1), edge(1, 2), edge(2, 3), heavy, edge(0, 4)];
        let init = longest_path(5, &edges);
        assert_eq!(init[4], 1);
        let rank = solve(5, &edges, &init, 1000);
        assert!(feasible(&edges, &rank));
        assert_eq!(rank[3] - rank[4], 1);
        assert_eq!(cost(&edges, &rank), 7);
    }

    #[test]
    fn matches_brute_force_on_small_graphs() {
        let mut seed = 7u64;
        let mut next = |n: u64| {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (seed >> 33) % n
        };
        for _ in 0..60 {
            let n = 2 + next(4) as usize;
            let mut edges = Vec::new();
            // A spanning chain keeps it connected; extra forward edges.
            for v in 1..n {
                let u = next(v as u64) as usize;
                edges.push(NsEdge { tail: u, head: v, minlen: 1 + next(2) as i64, weight: 1 + next(3) as i64 });
            }
            for _ in 0..next(4) {
                let u = next(n as u64 - 1) as usize;
                let v = u + 1 + next((n - u - 1) as u64) as usize;
                edges.push(NsEdge { tail: u, head: v, minlen: next(2) as i64, weight: next(3) as i64 });
            }
            let init = longest_path(n, &edges);
            let rank = solve(n, &edges, &init, 1000);
            assert!(feasible(&edges, &rank), "{edges:?} {rank:?}");
            assert_eq!(cost(&edges, &rank), brute_force(n, &edges, 2 * n as i64 + 2), "{edges:?}");
        }
    }
}
