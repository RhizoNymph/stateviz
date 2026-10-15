//! The explored state graph, and what is read off it once the search is
//! done: the shortest path to each node (for witnesses) and the number of
//! acyclic paths from the start (for `Outcome::paths`).

use std::collections::VecDeque;

/// A node's index in [`Graph::nodes`].
pub(crate) type NodeIx = u32;

#[derive(Clone, Debug, Default)]
pub(crate) struct Node {
    /// Explored moves: (child, index of the move's description).
    pub edges: Vec<(NodeIx, u32)>,
    /// The outcome this terminal node ends in.
    pub outcome: Option<usize>,
}

/// How a node is first reached breadth-first from node 0: its distance and
/// the edge (parent, move description) it is reached by (`None` for node 0).
pub(crate) type Reach = (u32, Option<(NodeIx, u32)>);

#[derive(Clone, Debug, Default)]
pub(crate) struct Graph {
    pub nodes: Vec<Node>,
}

impl Graph {
    pub fn add(&mut self) -> NodeIx {
        self.nodes.push(Node::default());
        to_ix(self.nodes.len() - 1)
    }

    pub fn node_mut(&mut self, ix: NodeIx) -> &mut Node {
        &mut self.nodes[ix as usize]
    }

    /// For every node reachable from node 0: the edge it is first reached
    /// by in breadth-first order (parent, move description), and its
    /// distance. Node 0 has distance 0 and no parent.
    pub fn shortest(&self) -> Vec<Option<Reach>> {
        let mut best: Vec<Option<Reach>> = vec![None; self.nodes.len()];
        if self.nodes.is_empty() {
            return best;
        }
        best[0] = Some((0, None));
        let mut queue = VecDeque::from([0 as NodeIx]);
        while let Some(at) = queue.pop_front() {
            let dist = best[at as usize].map_or(0, |(d, _)| d);
            for &(child, desc) in &self.nodes[at as usize].edges {
                if best[child as usize].is_none() {
                    best[child as usize] = Some((dist + 1, Some((at, desc))));
                    queue.push_back(child);
                }
            }
        }
        best
    }

    /// The descriptions along the shortest path to `target`.
    pub fn path_to(shortest: &[Option<Reach>], target: NodeIx) -> Vec<u32> {
        let mut path = Vec::new();
        let mut at = target;
        while let Some(Some((_, Some((parent, desc))))) = shortest.get(at as usize) {
            path.push(*desc);
            at = *parent;
        }
        path.reverse();
        path
    }

    /// The number of paths from node 0 to each node in the graph without
    /// its back edges (edges closing a cycle in a depth-first walk from
    /// node 0), saturating.
    pub fn path_counts(&self) -> Vec<u64> {
        let n = self.nodes.len();
        let mut counts = vec![0u64; n];
        if n == 0 {
            return counts;
        }
        // Iterative DFS: colour 0 unseen, 1 on the stack, 2 done.
        let mut colour = vec![0u8; n];
        let mut back = vec![Vec::<bool>::new(); n];
        let mut order = Vec::with_capacity(n);
        let mut stack: Vec<(usize, usize)> = vec![(0, 0)];
        colour[0] = 1;
        back[0] = vec![false; self.nodes[0].edges.len()];
        while let Some(top) = stack.last_mut() {
            let (at, edge) = *top;
            match self.nodes[at].edges.get(edge) {
                Some(&(child, _)) => {
                    top.1 += 1;
                    let child = child as usize;
                    match colour[child] {
                        0 => {
                            colour[child] = 1;
                            back[child] = vec![false; self.nodes[child].edges.len()];
                            stack.push((child, 0));
                        }
                        1 => back[at][edge] = true,
                        _ => {}
                    }
                }
                None => {
                    colour[at] = 2;
                    order.push(at);
                    stack.pop();
                }
            }
        }
        // Reverse postorder is a topological order of the DAG left.
        counts[0] = 1;
        for &at in order.iter().rev() {
            let here = counts[at];
            for (edge, &(child, _)) in self.nodes[at].edges.iter().enumerate() {
                if !back[at].get(edge).copied().unwrap_or(true) {
                    let c = &mut counts[child as usize];
                    *c = c.saturating_add(here);
                }
            }
        }
        counts
    }
}

pub(crate) fn to_ix(n: usize) -> NodeIx {
    NodeIx::try_from(n).unwrap_or(NodeIx::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn graph(edges: &[&[NodeIx]]) -> Graph {
        Graph {
            nodes: edges
                .iter()
                .map(|out| Node { edges: out.iter().map(|&c| (c, 0)).collect(), outcome: None })
                .collect(),
        }
    }

    #[test]
    fn diamond_has_two_paths() {
        let g = graph(&[&[1, 2], &[3], &[3], &[]]);
        assert_eq!(g.path_counts(), [1, 1, 1, 2]);
        let s = g.shortest();
        assert_eq!(Graph::path_to(&s, 3).len(), 2);
    }

    #[test]
    fn cycles_are_cut() {
        // 0 → 1 → 2 → 1, 2 → 3
        let g = graph(&[&[1], &[2], &[1, 3], &[]]);
        assert_eq!(g.path_counts(), [1, 1, 1, 1]);
    }

    #[test]
    fn counts_saturate() {
        // A ladder of 70 diamonds has 2^70 paths.
        let mut edges: Vec<Vec<NodeIx>> = Vec::new();
        for i in 0..70u32 {
            let base = i * 3;
            edges.push(vec![base + 1, base + 2]);
            edges.push(vec![base + 3]);
            edges.push(vec![base + 3]);
        }
        edges.push(vec![]);
        let refs: Vec<&[NodeIx]> = edges.iter().map(Vec::as_slice).collect();
        let g = graph(&refs);
        assert_eq!(g.path_counts().last().copied(), Some(u64::MAX));
    }
}
