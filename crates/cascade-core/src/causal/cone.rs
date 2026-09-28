//! Cone tracing and path queries over the causal graph.

use std::collections::VecDeque;

use super::{CausalGraph, EdgeIx, NodeIx};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Direction {
    /// Everything the seeds can set off.
    Forward,
    /// Everything that can cause the seeds.
    Backward,
}

/// A subgraph of the causal graph: the nodes it contains, each with its
/// distance from the seeds in causal hops, and the edges it contains.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cone {
    hops: Vec<Option<u32>>,
    edges: Vec<bool>,
}

impl Cone {
    fn empty(graph: &CausalGraph) -> Self {
        Self { hops: vec![None; graph.node_count()], edges: vec![false; graph.edge_count()] }
    }

    pub fn contains(&self, node: NodeIx) -> bool {
        self.hops.get(node.index()).is_some_and(Option::is_some)
    }

    /// Distance from the seeds in causal hops: the number of transitions (or
    /// external sources) entered along the shortest path. Seeds are 0.
    pub fn hops(&self, node: NodeIx) -> Option<u32> {
        self.hops.get(node.index()).copied().flatten()
    }

    pub fn contains_edge(&self, edge: EdgeIx) -> bool {
        self.edges.get(edge.index()).copied().unwrap_or(false)
    }

    pub fn nodes(&self) -> impl Iterator<Item = (NodeIx, u32)> + '_ {
        self.hops.iter().enumerate().filter_map(|(i, h)| h.map(|h| (NodeIx::new(i), h)))
    }

    pub fn edges(&self) -> impl Iterator<Item = EdgeIx> + '_ {
        self.edges.iter().enumerate().filter(|(_, inside)| **inside).map(|(i, _)| EdgeIx::new(i))
    }

    pub fn node_count(&self) -> usize {
        self.hops.iter().filter(|h| h.is_some()).count()
    }

    pub fn is_empty(&self) -> bool {
        self.hops.iter().all(Option::is_none)
    }

    /// Nodes and edges in either cone. Hops take the smaller distance.
    pub fn union(&self, other: &Cone) -> Cone {
        Cone {
            hops: self
                .hops
                .iter()
                .zip(&other.hops)
                .map(|(a, b)| match (a, b) {
                    (Some(a), Some(b)) => Some((*a).min(*b)),
                    (a, b) => a.or(*b),
                })
                .collect(),
            edges: self.edges.iter().zip(&other.edges).map(|(a, b)| *a || *b).collect(),
        }
    }
}

impl CausalGraph {
    /// The forward or backward cone of `seeds`, limited to `max_hops` causal
    /// hops (`None` for unlimited).
    ///
    /// Hops count transitions and external sources entered; events and
    /// handlers cost nothing. So a depth of 1 forward from a transition holds
    /// the events it emits, the handlers that receive them, the transitions
    /// those handlers fire, and those transitions' own events and handlers
    /// (what would fire next), but no further transitions.
    pub fn cone(&self, seeds: &[NodeIx], direction: Direction, max_hops: Option<u32>) -> Cone {
        let mut cone = Cone::empty(self);
        let within = |h: u32| max_hops.is_none_or(|max| h <= max);
        let mut queue = VecDeque::new();
        for &seed in seeds {
            if seed.index() < self.node_count() && cone.hops[seed.index()].is_none() {
                cone.hops[seed.index()] = Some(0);
                queue.push_back(seed);
            }
        }

        // 0-1 BFS: stepping onto a hop node costs 1, anything else 0, so
        // zero-cost steps go to the front of the queue.
        while let Some(u) = queue.pop_front() {
            let Some(hu) = cone.hops[u.index()] else { continue };
            let steps = match direction {
                Direction::Forward => self.outgoing(u),
                Direction::Backward => self.incoming(u),
            };
            for &e in steps {
                let edge = self.edge(e);
                let v = match direction {
                    Direction::Forward => edge.to,
                    Direction::Backward => edge.from,
                };
                let cost = u32::from(self.node(v).is_hop());
                let hv = hu + cost;
                if !within(hv) {
                    continue;
                }
                if cone.hops[v.index()].is_none_or(|old| hv < old) {
                    cone.hops[v.index()] = Some(hv);
                    if cost == 0 {
                        queue.push_front(v);
                    } else {
                        queue.push_back(v);
                    }
                }
            }
        }

        // An edge is in the cone when it can be traversed from inside the
        // cone without exceeding the hop limit.
        for (e, edge) in self.edges() {
            let (inner, outer) = match direction {
                Direction::Forward => (edge.from, edge.to),
                Direction::Backward => (edge.to, edge.from),
            };
            if let Some(h) = cone.hops[inner.index()] {
                let cost = u32::from(self.node(outer).is_hop());
                cone.edges[e.index()] = within(h + cost);
            }
        }
        cone
    }

    /// Every causal path between `a` and `b`, in either direction: the nodes
    /// and edges that lie on some path `a ⇝ b` or `b ⇝ a`. Empty when
    /// neither can cause the other. Hops are measured from whichever
    /// endpoint is the cause.
    pub fn paths_between(&self, a: NodeIx, b: NodeIx) -> Cone {
        self.directed_paths(a, b).union(&self.directed_paths(b, a))
    }

    fn directed_paths(&self, from: NodeIx, to: NodeIx) -> Cone {
        let forward = self.cone(&[from], Direction::Forward, None);
        if !forward.contains(to) {
            return Cone::empty(self);
        }
        let backward = self.cone(&[to], Direction::Backward, None);
        let mut result = Cone::empty(self);
        for (ix, hops) in forward.nodes() {
            if backward.contains(ix) {
                result.hops[ix.index()] = Some(hops);
            }
        }
        for (e, edge) in self.edges() {
            result.edges[e.index()] = forward.contains(edge.from)
                && backward.contains(edge.to)
                && result.contains(edge.from)
                && result.contains(edge.to);
        }
        result
    }
}
