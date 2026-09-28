//! Structural filters from the view state, resolved against the model.

use std::collections::BTreeSet;

use cascade_core::{CausalEdgeKind, CausalGraph, CausalNode, MachineId, Model};

use crate::emphasis::Interaction;
use crate::views::draft::DraftGraph;
use crate::views::links::causal_links;

/// Machines toggled off in the legend that exist in the model.
pub(crate) fn hidden_machines(model: &Model, names: &BTreeSet<String>) -> BTreeSet<MachineId> {
    names.iter().filter_map(|n| model.machine_by_name(n)).collect()
}

/// The matrix-cell pair, when both names exist. Unknown names leave a note
/// and no filter.
pub(crate) fn machine_pair(
    model: &Model,
    pair: Option<&(String, String)>,
    notes: &mut Vec<String>,
) -> Option<[MachineId; 2]> {
    let (a, b) = pair?;
    match (model.machine_by_name(a), model.machine_by_name(b)) {
        (Some(x), Some(y)) => Some([x, y]),
        _ => {
            notes.push(format!("Unknown machine in pair filter {a} × {b}; showing everything."));
            None
        }
    }
}

/// Causal nodes kept by a machine-pair filter: the two machines'
/// transitions, the sources that trigger them, and the events and handlers
/// on links between them (in either direction, or within one of them).
/// Without a pair, everything.
pub(crate) fn pair_nodes(model: &Model, graph: &CausalGraph, pair: Option<[MachineId; 2]>) -> Vec<bool> {
    let Some(pair) = pair else {
        return vec![true; graph.node_count()];
    };
    let in_pair = |t| pair.contains(&model.transition(t).machine);
    let mut keep = vec![false; graph.node_count()];
    for (ix, node) in graph.nodes() {
        keep[ix.index()] = match node {
            CausalNode::Transition(t) => in_pair(t),
            CausalNode::External(_) => graph.outgoing(ix).iter().any(|&e| {
                let edge = graph.edge(e);
                matches!(edge.kind, CausalEdgeKind::Trigger { .. })
                    && matches!(graph.node(edge.to), CausalNode::Transition(t) if in_pair(t))
            }),
            CausalNode::Event(_) | CausalNode::Handler(_) => false,
        };
    }
    for link in causal_links(graph).into_iter().filter(|l| in_pair(l.from) && in_pair(l.to)) {
        let [emit, sub, _] = link.chain.map(|e| graph.edge(e));
        keep[emit.to.index()] = true;
        keep[sub.to.index()] = true;
    }
    keep
}

/// Hide mode: remove nodes outside the focus (leaving stubs for cut links,
/// returned as cuts), then edges outside the focus between the nodes that
/// remain.
pub(crate) fn apply_hide(draft: &mut DraftGraph, interaction: &Interaction) -> Vec<crate::views::draft::Cut> {
    if !interaction.hides_outside() {
        return Vec::new();
    }
    let hidden: Vec<bool> =
        draft.nodes.iter().map(|n| interaction.is_hidden(&n.meta.elements, &n.meta.anchor)).collect();
    let cuts = draft.hide(&hidden);
    draft.edges.retain(|e| !interaction.is_hidden(&e.meta.elements, &e.meta.anchor));
    cuts
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::emphasis::tests::{CHAIN, load, node_labels};

    #[test]
    fn pair_filter_keeps_links_between_the_two_machines() {
        let model = load(CHAIN);
        let graph = CausalGraph::build(&model);
        let a = model.machine_by_name("A").expect("A");
        let b = model.machine_by_name("B").expect("B");
        let keep = pair_nodes(&model, &graph, Some([a, b]));
        let kept = graph.nodes().map(|(ix, _)| ix).filter(|ix| keep[ix.index()]);
        assert_eq!(
            node_labels(&model, &graph, kept),
            [
                "A: a0 → a1",
                "A: a1 → a0",
                "B: b0 → b1",
                "B: b1 → b0",
                "Back",
                "C1 on Go",
                "C2 on Done",
                "C3 on Back",
                "Done",
                "Go",
                "User"
            ]
        );
        let mut notes = Vec::new();
        assert_eq!(machine_pair(&model, Some(&("A".into(), "Nope".into())), &mut notes), None);
        assert_eq!(notes.len(), 1);
    }
}
