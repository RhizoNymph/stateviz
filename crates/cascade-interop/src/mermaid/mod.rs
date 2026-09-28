//! Mermaid export, for READMEs and design docs.
//!
//! Two diagrams from the same model:
//!
//! - [`structure`] (`--to mermaid`): a `stateDiagram-v2` with each machine as
//!   a composite state, nested states as nested composites, transitions
//!   labelled `trigger [guard]`, `[*] -->` initial markers and `--> [*]`
//!   final markers.
//! - [`causal`] (`--to mermaid-causal`): a `flowchart LR` of the causal
//!   graph: external sources, transition pills, event tags and controller
//!   hexagons, with dashed emit and fire links.
//!
//! Labels are escaped with Mermaid entity codes (`#35;` for `#`, …) so any
//! guard text is safe; ids are generated from `[A-Za-z0-9_]` only. See each
//! module for the representation and its limits.

mod causal;
mod escape;
mod ids;
mod palette;
mod structure;

pub(crate) use causal::causal;
pub(crate) use structure::structure;
