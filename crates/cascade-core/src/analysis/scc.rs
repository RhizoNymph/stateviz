//! Strongly connected components (Tarjan), iterative so that long causal
//! chains cannot overflow the call stack.

/// The strongly connected components of the graph whose node `v` has the
/// successors `successors[v]` (every entry must be `< successors.len()`).
///
/// Components come out in reverse topological order of the condensation
/// (Tarjan's natural order); nodes inside a component are unordered. Every
/// node appears in exactly one component.
pub(super) fn strongly_connected(successors: &[Vec<usize>]) -> Vec<Vec<usize>> {
    let n = successors.len();
    let mut index: Vec<Option<usize>> = vec![None; n];
    let mut low = vec![0usize; n];
    let mut on_stack = vec![false; n];
    let mut stack: Vec<usize> = Vec::new();
    let mut components = Vec::new();
    let mut next_index = 0usize;
    // Simulated call stack: (node, position of the next successor to visit).
    let mut frames: Vec<(usize, usize)> = Vec::new();

    for root in 0..n {
        if index[root].is_some() {
            continue;
        }
        index[root] = Some(next_index);
        low[root] = next_index;
        next_index += 1;
        stack.push(root);
        on_stack[root] = true;
        frames.push((root, 0));

        while let Some(frame) = frames.last_mut() {
            let v = frame.0;
            let next = successors[v].get(frame.1).copied();
            if next.is_some() {
                frame.1 += 1;
            }
            match next {
                Some(w) => match index[w] {
                    None => {
                        index[w] = Some(next_index);
                        low[w] = next_index;
                        next_index += 1;
                        stack.push(w);
                        on_stack[w] = true;
                        frames.push((w, 0));
                    }
                    Some(w_index) if on_stack[w] => low[v] = low[v].min(w_index),
                    Some(_) => {}
                },
                None => {
                    frames.pop();
                    if let Some(&(parent, _)) = frames.last() {
                        low[parent] = low[parent].min(low[v]);
                    }
                    if index[v] == Some(low[v]) {
                        let mut component = Vec::new();
                        while let Some(w) = stack.pop() {
                            on_stack[w] = false;
                            component.push(w);
                            if w == v {
                                break;
                            }
                        }
                        components.push(component);
                    }
                }
            }
        }
    }
    components
}

#[cfg(test)]
mod tests {
    use super::strongly_connected;

    fn sorted(mut components: Vec<Vec<usize>>) -> Vec<Vec<usize>> {
        for c in &mut components {
            c.sort_unstable();
        }
        components.sort();
        components
    }

    #[test]
    fn empty_graph_has_no_components() {
        assert!(strongly_connected(&[]).is_empty());
    }

    #[test]
    fn dag_nodes_are_singletons() {
        let g = vec![vec![1, 2], vec![2], vec![]];
        assert_eq!(sorted(strongly_connected(&g)), vec![vec![0], vec![1], vec![2]]);
    }

    #[test]
    fn finds_cycles_and_self_loops() {
        // 0 → 1 → 2 → 0, 2 → 3, 3 → 3, 4 → 5 → 4, 5 → 0
        let g = vec![vec![1], vec![2], vec![0, 3], vec![3], vec![5], vec![4, 0]];
        assert_eq!(sorted(strongly_connected(&g)), vec![vec![0, 1, 2], vec![3], vec![4, 5]]);
    }

    #[test]
    fn reverse_topological_order() {
        // 0 → 1 → 2: sinks come first.
        let g = vec![vec![1], vec![2], vec![]];
        assert_eq!(strongly_connected(&g), vec![vec![2], vec![1], vec![0]]);
    }

    #[test]
    fn long_chains_do_not_overflow_the_stack() {
        let n = 200_000;
        let mut g: Vec<Vec<usize>> = (0..n).map(|i| vec![i + 1]).collect();
        g[n - 1] = vec![0];
        let components = strongly_connected(&g);
        assert_eq!(components.len(), 1);
        assert_eq!(components[0].len(), n);

        let chain: Vec<Vec<usize>> = (0..n).map(|i| if i + 1 < n { vec![i + 1] } else { vec![] }).collect();
        assert_eq!(strongly_connected(&chain).len(), n);
    }
}
