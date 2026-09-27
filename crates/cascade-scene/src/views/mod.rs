//! Scene builders, one per view.
//!
//! Owner: `feat/view-scenes` implements all four views, emphasis (cones,
//! path queries, search, entity filter stubs, collapse), badges and diff
//! decorations. The foundation ships a plain causal view so hosts can render
//! something before that lands.

mod causal;

use std::collections::HashMap;

use cascade_core::diff::ModelDiff;
use cascade_core::{CausalGraph, Finding, Model};
use cascade_layout::PreviousLayout;
use cascade_sim::Trace;

use crate::color::Theme;
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
}

#[derive(Debug, thiserror::Error)]
pub enum SceneError {
    #[error("layout failed: {0}")]
    Layout(#[from] cascade_layout::LayoutError),
    #[error("layout graph: {0}")]
    Graph(#[from] cascade_layout::GraphError),
}

/// Builds scenes and remembers each view's previous layout so an edit to
/// one transition does not move unrelated nodes.
#[derive(Debug, Default)]
pub struct SceneBuilder {
    previous: HashMap<ViewKind, PreviousLayout>,
}

impl SceneBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Forget previous layouts (e.g. when opening a different file).
    pub fn reset(&mut self) {
        self.previous.clear();
    }

    pub fn build(&mut self, input: &SceneInput<'_>) -> Result<Scene, SceneError> {
        match input.view.view {
            ViewKind::Causal => {
                let previous = self.previous.get(&ViewKind::Causal).cloned();
                let (scene, next) = causal::build(input, previous)?;
                self.previous.insert(ViewKind::Causal, next);
                Ok(scene)
            }
            ViewKind::Structure | ViewKind::Trace | ViewKind::Matrix => {
                let mut scene = Scene::empty(input.view.view, input.theme.background);
                scene.notes.push(format!("The {} view is not implemented yet.", input.view.view));
                Ok(scene)
            }
        }
    }
}
