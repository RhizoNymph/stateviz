//! Cycle handling: strongly connected components and the Eades–Lin–Smyth
//! greedy linear arrangement, which orders each component so that few edges
//! point backwards (a small feedback arc set).

/// Strongly connected components by Tarjan's algorithm, iterative. Returns
/// the component of every node and the number of components. Components are
/// numbered in reverse topological order: every edge between two components
/// goes from a higher number to a lower one.
pub(crate) fn strongly_connected(out: &[Vec<usize>]) -> (Vec<usize>, usize) {
    const UNSEEN: usize = usize::MAX;
    let n = out.len();
    let mut index = vec![UNSEEN; n];
    let mut low = vec![0usize; n];
    let mut on_stack = vec![false; n];
    let mut stack = Vec::new();
    let mut comp = vec![UNSEEN; n];
    let mut next = 0usize;
    let mut count = 0usize;
    for root in 0..n {
        if index[root] != UNSEEN {
            continue;
        }
        let mut calls: Vec<(usize, usize)> = vec![(root, 0)];
        index[root] = next;
        low[root] = next;
        next += 1;
        stack.push(root);
        on_stack[root] = true;
        while let Some(&mut (v, ref mut i)) = calls.last_mut() {
            if let Some(&w) = out[v].get(*i) {
                *i += 1;
                if index[w] == UNSEEN {
                    index[w] = next;
                    low[w] = next;
                    next += 1;
                    stack.push(w);
                    on_stack[w] = true;
                    calls.push((w, 0));
                } else if on_stack[w] {
                    low[v] = low[v].min(index[w]);
                }
            } else {
                calls.pop();
                if let Some(&(p, _)) = calls.last() {
                    low[p] = low[p].min(low[v]);
                }
                if low[v] == index[v] {
                    while let Some(x) = stack.pop() {
                        on_stack[x] = false;
                        comp[x] = count;
                        if x == v {
                            break;
                        }
                    }
                    count += 1;
                }
            }
        }
    }
    (comp, count)
}

/// Eades–Lin–Smyth greedy arrangement of nodes `0..n` given directed edges
/// (parallel edges count with multiplicity; self-loops must be excluded).
/// Sinks go to the end, sources to the front, and otherwise the node with
/// the largest out-degree minus in-degree goes to the front. Ties break by
/// lowest index, so the result is deterministic. Edges pointing backwards in
/// the result form a feedback arc set.
pub(crate) fn greedy_arrangement(n: usize, edges: &[(usize, usize)]) -> Vec<usize> {
    let mut outs: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut ins: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut outdeg = vec![0i64; n];
    let mut indeg = vec![0i64; n];
    for &(u, v) in edges {
        outs[u].push(v);
        ins[v].push(u);
        outdeg[u] += 1;
        indeg[v] += 1;
    }
    let mut removed = vec![false; n];
    let mut front = Vec::with_capacity(n);
    let mut back = Vec::new();
    let mut remaining = n;
    let remove = |u: usize, removed: &mut [bool], outdeg: &mut [i64], indeg: &mut [i64]| {
        removed[u] = true;
        for &w in &outs[u] {
            indeg[w] -= 1;
        }
        for &x in &ins[u] {
            outdeg[x] -= 1;
        }
    };
    while remaining > 0 {
        let mut changed = true;
        while changed {
            changed = false;
            for u in 0..n {
                if !removed[u] && outdeg[u] == 0 {
                    back.push(u);
                    remove(u, &mut removed, &mut outdeg, &mut indeg);
                    remaining -= 1;
                    changed = true;
                }
            }
            for u in 0..n {
                if !removed[u] && indeg[u] == 0 {
                    front.push(u);
                    remove(u, &mut removed, &mut outdeg, &mut indeg);
                    remaining -= 1;
                    changed = true;
                }
            }
        }
        if remaining == 0 {
            break;
        }
        let pick = (0..n).filter(|&u| !removed[u]).max_by_key(|&u| (outdeg[u] - indeg[u], std::cmp::Reverse(u)));
        if let Some(u) = pick {
            front.push(u);
            remove(u, &mut removed, &mut outdeg, &mut indeg);
            remaining -= 1;
        }
    }
    back.reverse();
    front.extend(back);
    front
}

#[cfg(test)]
mod tests {
    use super::*;

    fn adjacency(n: usize, edges: &[(usize, usize)]) -> Vec<Vec<usize>> {
        let mut out = vec![Vec::new(); n];
        for &(u, v) in edges {
            out[u].push(v);
        }
        out
    }

    #[test]
    fn components_in_reverse_topological_order() {
        // 0 -> {1 <-> 2} -> 3, 4 alone.
        let edges = [(0, 1), (1, 2), (2, 1), (2, 3)];
        let (comp, count) = strongly_connected(&adjacency(5, &edges));
        assert_eq!(count, 4);
        assert_eq!(comp[1], comp[2]);
        assert!(comp[0] > comp[1] && comp[1] > comp[3]);
        for &(u, v) in &edges {
            assert!(comp[u] >= comp[v]);
        }
    }

    #[test]
    fn arrangement_of_a_dag_is_topological() {
        let edges = [(0, 2), (2, 1), (1, 3), (0, 3), (4, 0)];
        let order = greedy_arrangement(5, &edges);
        let pos: Vec<usize> = (0..5).map(|v| order.iter().position(|&x| x == v).expect("present")).collect();
        for &(u, v) in &edges {
            assert!(pos[u] < pos[v], "{order:?}");
        }
    }

    #[test]
    fn arrangement_breaks_a_cycle_with_one_backward_edge() {
        let edges = [(0, 1), (1, 2), (2, 3), (3, 0), (1, 3)];
        let order = greedy_arrangement(4, &edges);
        let pos: Vec<usize> = (0..4).map(|v| order.iter().position(|&x| x == v).expect("present")).collect();
        let backward = edges.iter().filter(|&&(u, v)| pos[u] > pos[v]).count();
        assert_eq!(backward, 1, "{order:?}");
    }

    #[test]
    fn deep_graphs_do_not_overflow_the_stack() {
        let n = 50_000;
        let edges: Vec<_> = (0..n - 1).map(|i| (i, i + 1)).chain([(n - 1, 0)]).collect();
        let (comp, count) = strongly_connected(&adjacency(n, &edges));
        assert_eq!(count, 1);
        assert!(comp.iter().all(|&c| c == 0));
    }
}
