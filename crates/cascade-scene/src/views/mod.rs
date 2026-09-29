//! Scene builders, one per view, plus the passes they share.
//!
//! ```text
//! ViewState ──Interaction::new──▶ Interaction (selection, cone/path focus, search)
//!                                        │
//! Model + CausalGraph ──view builder──▶ DraftGraph (looks + metas)
//!        (filters: machine pair, hidden machines, collapse, hide mode)
//!                                        │ realize (LayoutCache: same input ⇒ same layout)
//!                                        ▼
//!                                      Scene ──Decor::apply──▶ emphasis, badges, diff
//! ```
//!
//! The trace and matrix views place items directly (no layered layout) and
//! share only the decoration pass.

mod cache;
mod causal;
mod decorate;
mod draft;
mod filters;
mod links;
mod matrix;
mod overlays;
mod structure;
mod style;
mod trace;

use cascade_core::diff::ModelDiff;
use cascade_core::{CausalGraph, Finding, Model};
use cascade_sim::Trace;

use crate::color::Theme;
use crate::emphasis::Interaction;
use crate::pins::LayoutSidecar;
use crate::scene::Scene;
use crate::text::TextMeasure;
use crate::view_state::{ViewKind, ViewState};

/// Everything a view needs. Hosts derive `graph` and `findings` once per
/// model load and reuse them across rebuilds.
pub struct SceneInput<'a> {
    pub model: &'a Model,
    pub graph: &'a CausalGraph,
    pub findings: &'a [Finding],
    pub view: &'a ViewState,
    pub theme: &'a Theme,
    pub measure: &'a dyn TextMeasure,
    pub sidecar: &'a LayoutSidecar,
    /// Trace view: one trace, or the two orderings of a race side by side.
    pub traces: &'a [Trace],
    /// Diff mode: statuses for the (merged) model.
    pub diff: Option<&'a ModelDiff>,
    /// Build mode adds wiring and connect handles to the structure view.
    pub mode: crate::play::SceneMode,
    /// A live play session to draw over the causal and structure views.
    pub play: Option<&'a crate::play::PlayOverlay>,
}

#[derive(Debug, thiserror::Error)]
pub enum SceneError {
    #[error("layout failed: {0}")]
    Layout(#[from] cascade_layout::LayoutError),
    #[error("layout graph: {0}")]
    Graph(#[from] cascade_layout::GraphError),
}

/// Builds scenes and remembers each view's recent layouts, so emphasis
/// changes never relayout and an edit to one transition does not move
/// unrelated nodes.
#[derive(Debug, Default)]
pub struct SceneBuilder {
    cache: cache::LayoutCache,
    /// The build canvas's gutter columns, kept across edits.
    wiring: structure::WiringMemo,
}

impl SceneBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Forget previous layouts (e.g. when opening a different file).
    pub fn reset(&mut self) {
        self.cache.clear();
        self.wiring.clear();
    }

    /// How many times the layout engine has actually run (cache misses).
    /// Selection, cone, search and diff changes never add to it.
    pub fn layouts_run(&self) -> u64 {
        self.cache.runs()
    }

    pub fn build(&mut self, input: &SceneInput<'_>) -> Result<Scene, SceneError> {
        let interaction = Interaction::new(input.model, input.graph, input.view);
        match input.view.view {
            ViewKind::Causal => causal::build(input, &interaction, &mut self.cache),
            ViewKind::Structure => structure::build(input, &interaction, &mut self.cache, &mut self.wiring),
            ViewKind::Trace => Ok(trace::build(input, &interaction)),
            ViewKind::Matrix => Ok(matrix::build(input, &interaction)),
        }
    }
}
