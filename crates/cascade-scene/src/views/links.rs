//! Derived facts about the causal graph that several views share.

use cascade_core::{CausalEdgeKind, CausalGraph, CausalNode, EdgeIx, RuleId, TransitionId};

/// One derived causal link `from → event → handler → to`, the unit the
/// matrix counts and the structure view draws between lanes. Mirrors
/// [`CausalGraph::transition_successors`], plus the edges it runs along.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CausalLink {
    pub from: TransitionId,
    pub to: TransitionId,
    pub rule: RuleId,
    /// Emit, subscribe and fire edges, in causal order.
    pub chain: [EdgeIx; 3],
}

/// Every derived transition-to-transition link, deduplicated by
/// `(from, to, rule)`, in graph order.
pub(crate) fn causal_links(graph: &CausalGraph) -> Vec<CausalLink> {
    let mut out: Vec<CausalLink> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for (ix, node) in graph.nodes() {
        let CausalNode::Transition(from) = node else { continue };
        for &emit in graph.outgoing(ix) {
            let event = graph.edge(emit).to;
            for &sub in graph.outgoing(event) {
                let handler = graph.edge(sub).to;
                for &fire in graph.outgoing(handler) {
                    let edge = graph.edge(fire);
                    if let (CausalEdgeKind::Fire { rule }, CausalNode::Transition(to)) =
                        (edge.kind, graph.node(edge.to))
                        && seen.insert((from, to, rule))
                    {
                        out.push(CausalLink { from, to, rule, chain: [emit, sub, fire] });
                    }
                }
            }
        }
    }
    out
}

/// For each causal edge, whether it lies on a cycle (both ends in the same
/// strongly connected component). Only these can be cascade-cycle back
/// edges.
pub(crate) fn cycle_edges(graph: &CausalGraph) -> Vec<bool> {
    let component = strongly_connected(graph);
    graph.edges().map(|(_, e)| component[e.from.index()] == component[e.to.index()]).collect()
}

/// Tarjan's algorithm, iterative: the component id of every node.
fn strongly_connected(graph: &CausalGraph) -> Vec<usize> {
    const UNSEEN: usize = usize::MAX;
    let n = graph.node_count();
    let mut index = vec![UNSEEN; n];
    let mut low = vec![0usize; n];
    let mut on_stack = vec![false; n];
    let mut component = vec![UNSEEN; n];
    let mut stack: Vec<usize> = Vec::new();
    let mut next_index = 0;
    let mut next_component = 0;
    let nodes: Vec<_> = graph.nodes().map(|(ix, _)| ix).collect();

    for &root in &nodes {
        if index[root.index()] != UNSEEN {
            continue;
        }
        // (node, position in its outgoing list)
        let mut work: Vec<(usize, usize)> = vec![(root.index(), 0)];
        index[root.index()] = next_index;
        low[root.index()] = next_index;
        next_index += 1;
        stack.push(root.index());
        on_stack[root.index()] = true;
        while let Some(&mut (v, ref mut pos)) = work.last_mut() {
            let out = graph.outgoing(nodes[v]);
            if let Some(&e) = out.get(*pos) {
                *pos += 1;
                let w = graph.edge(e).to.index();
                if index[w] == UNSEEN {
                    index[w] = next_index;
                    low[w] = next_index;
                    next_index += 1;
                    stack.push(w);
                    on_stack[w] = true;
                    work.push((w, 0));
                } else if on_stack[w] {
                    low[v] = low[v].min(index[w]);
                }
            } else {
                work.pop();
                if let Some(&(parent, _)) = work.last() {
                    low[parent] = low[parent].min(low[v]);
                }
                if low[v] == index[v] {
                    while let Some(w) = stack.pop() {
                        on_stack[w] = false;
                        component[w] = next_component;
                        if w == v {
                            break;
                        }
                    }
                    next_component += 1;
                }
            }
        }
    }
    component
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::emphasis::tests::{CHAIN, load};

    #[test]
    fn links_match_transition_successors() {
        let model = load(CHAIN);
        let graph = CausalGraph::build(&model);
        let links = causal_links(&graph);
        for t in model.transition_ids() {
            let mut expected = graph.transition_successors(t);
            expected.sort();
            expected.dedup();
            let mut got: Vec<_> = links.iter().filter(|l| l.from == t).map(|l| (l.to, l.rule)).collect();
            got.sort();
            assert_eq!(got, expected);
        }
        assert_eq!(links.len(), 4);
        for link in &links {
            let [emit, sub, fire] = link.chain.map(|e| graph.edge(e));
            assert_eq!(emit.to, sub.from);
            assert_eq!(sub.to, fire.from);
        }
    }

    #[test]
    fn only_cycle_edges_are_flagged() {
        let model = load(CHAIN);
        let graph = CausalGraph::build(&model);
        assert!(cycle_edges(&graph).iter().all(|c| !c), "the chain is acyclic");

        let retry = load(
            r#"
machines:
  R:
    states: [s, t]
    transitions:
      - { from: s, to: s, on: retry, emits: [Failed] }
      - { from: s, to: t, on: go, emits: [Went] }
controllers:
  Retrier:
    on:
      Failed: [{ fire: R.retry }]
external:
  User: [R.go, R.retry]
"#,
        );
        let graph = CausalGraph::build(&retry);
        let flags = cycle_edges(&graph);
        let on_cycle = flags.iter().filter(|c| **c).count();
        assert_eq!(on_cycle, 3, "emit, subscribe and fire around the retry loop");
    }
}
