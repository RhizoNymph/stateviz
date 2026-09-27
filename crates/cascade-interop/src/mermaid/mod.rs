//! Mermaid export: `stateDiagram-v2` for structure, `flowchart LR` for the
//! causal graph.

use cascade_core::Model;

/// Every machine's structure as a Mermaid `stateDiagram-v2`.
pub(crate) fn structure(model: &Model) -> String {
    let _ = model;
    String::from("stateDiagram-v2\n")
}

/// The causal graph as a Mermaid `flowchart LR`.
pub(crate) fn causal(model: &Model) -> String {
    let _ = model;
    String::from("flowchart LR\n")
}
