//! Diff mode: compare the definition at a base git ref with a head ref or
//! the working tree, and show the merged model with diff decorations.
//!
//! The merged model has its own ids, so it gets its own causal graph. It is
//! drawn with no findings (the head's findings name ids of the head model).

use std::path::Path;
use std::sync::Arc;

use cascade_core::diff::{DiffStatus, ModelDiff, merge_for_display};
use cascade_core::{CausalGraph, LoadError, Model, load_str};
use cascade_interop::{GitError, read_at_rev};
use cascade_scene::{DiffRefs, ViewKind};

/// The merged model to draw and the statuses of its elements.
#[derive(Debug)]
pub struct DiffDisplay {
    pub model: Model,
    pub graph: CausalGraph,
    pub diff: ModelDiff,
}

impl DiffDisplay {
    /// `+added ~changed −removed`, for the status bar.
    pub fn summary(&self) -> String {
        format!(
            "+{} ~{} −{}",
            self.diff.count(DiffStatus::Added),
            self.diff.count(DiffStatus::Changed),
            self.diff.count(DiffStatus::Removed)
        )
    }
}

/// What diff mode asks for, and the working-tree generation it depends on
/// (0 when the head is a git ref and the working tree does not matter).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffRequest {
    pub refs: DiffRefs,
    pub generation: u64,
}

impl DiffRequest {
    pub fn new(refs: DiffRefs, working_generation: u64) -> Self {
        let generation = if refs.head.is_none() { working_generation } else { 0 };
        Self { refs, generation }
    }
}

#[derive(Clone, Debug, Default)]
pub enum DiffRun {
    #[default]
    Off,
    /// Computing; `previous` is the last display for the same refs, shown
    /// meanwhile so a save does not flicker the diff off and on.
    Loading {
        request: DiffRequest,
        previous: Option<Arc<DiffDisplay>>,
    },
    Ready {
        request: DiffRequest,
        display: Arc<DiffDisplay>,
    },
    Failed {
        request: DiffRequest,
        error: String,
    },
}

impl DiffRun {
    pub fn request(&self) -> Option<&DiffRequest> {
        match self {
            DiffRun::Off => None,
            DiffRun::Loading { request, .. } | DiffRun::Ready { request, .. } | DiffRun::Failed { request, .. } => {
                Some(request)
            }
        }
    }

    /// The display to draw for `refs`. A display is self-consistent (its
    /// own merged model, graph and statuses), so one computed from an older
    /// working tree is still safe to draw while its replacement loads.
    pub fn display_for(&self, refs: &DiffRefs) -> Option<&Arc<DiffDisplay>> {
        match self {
            DiffRun::Ready { request, display } if request.refs == *refs => Some(display),
            DiffRun::Loading { request, previous: Some(display) } if request.refs == *refs => Some(display),
            _ => None,
        }
    }

    /// The error of the last run for `refs`, if it failed.
    pub fn error_for(&self, refs: &DiffRefs) -> Option<&str> {
        match self {
            DiffRun::Failed { request, error } if request.refs == *refs => Some(error),
            _ => None,
        }
    }

    pub fn needs_run(&self, request: &DiffRequest) -> bool {
        self.request() != Some(request)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum DiffError {
    #[error("reading `{rev}`: {source}")]
    Git {
        rev: String,
        #[source]
        source: GitError,
    },
    #[error("the definition at `{rev}` does not load:\n{source}")]
    Invalid {
        rev: String,
        #[source]
        source: LoadError,
    },
    #[error("the working tree definition does not load; fix it or compare two refs")]
    NoWorkingTree,
    #[error("merging the two versions failed:\n{0}")]
    Merge(#[source] LoadError),
}

fn model_at(file: &Path, rev: &str) -> Result<Model, DiffError> {
    let text = read_at_rev(file, rev).map_err(|source| DiffError::Git { rev: rev.to_owned(), source })?;
    load_str(&text).map_err(|source| DiffError::Invalid { rev: rev.to_owned(), source })
}

/// Load both sides and merge them. `working` is the working-tree model,
/// used when `refs.head` is `None`. Blocking (runs git): call it from a
/// background task.
pub fn compute(file: &Path, refs: &DiffRefs, working: Option<Arc<Model>>) -> Result<DiffDisplay, DiffError> {
    let base = model_at(file, &refs.base)?;
    let head: Arc<Model> = match &refs.head {
        Some(rev) => Arc::new(model_at(file, rev)?),
        None => working.ok_or(DiffError::NoWorkingTree)?,
    };
    let (model, diff) = merge_for_display(&base, &head).map_err(DiffError::Merge)?;
    let graph = CausalGraph::build(&model);
    Ok(DiffDisplay { model, graph, diff })
}

/// Views that draw diff decorations. The trace view runs the simulator on
/// the working tree and the matrix counts its links, so both keep showing
/// the working tree while diff mode is on.
pub fn applies_to(view: ViewKind) -> bool {
    matches!(view, ViewKind::Causal | ViewKind::Structure)
}

/// Parse the head field: blank means the working tree.
pub fn refs_from_fields(base: &str, head: &str) -> Option<DiffRefs> {
    let base = base.trim();
    if base.is_empty() {
        return None;
    }
    let head = head.trim();
    Some(DiffRefs { base: base.to_owned(), head: (!head.is_empty()).then(|| head.to_owned()) })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn working_tree_requests_track_the_generation() {
        let wt = DiffRefs { base: "main".into(), head: None };
        assert_eq!(DiffRequest::new(wt.clone(), 7).generation, 7);
        let refs = DiffRefs { base: "main".into(), head: Some("HEAD".into()) };
        assert_eq!(DiffRequest::new(refs, 7).generation, 0);
        let run = DiffRun::Loading { request: DiffRequest::new(wt.clone(), 7), previous: None };
        assert!(!run.needs_run(&DiffRequest::new(wt.clone(), 7)));
        assert!(run.needs_run(&DiffRequest::new(wt.clone(), 8)));
        assert!(run.display_for(&wt).is_none());
        assert!(DiffRun::Off.request().is_none());
    }

    #[test]
    fn loading_keeps_showing_the_previous_display_for_the_same_refs() {
        let model = load_str(include_str!("../../../examples/order-fulfillment/cascade.yaml")).expect("loads");
        let graph = CausalGraph::build(&model);
        let display = Arc::new(DiffDisplay { model, graph, diff: ModelDiff::default() });
        let wt = DiffRefs { base: "main".into(), head: None };
        let other = DiffRefs { base: "dev".into(), head: None };
        let run = DiffRun::Loading { request: DiffRequest::new(wt.clone(), 2), previous: Some(display) };
        assert!(run.display_for(&wt).is_some());
        assert!(run.display_for(&other).is_none());
        let failed = DiffRun::Failed { request: DiffRequest::new(wt.clone(), 2), error: "boom".into() };
        assert_eq!(failed.error_for(&wt), Some("boom"));
        assert_eq!(failed.error_for(&other), None);
    }

    #[test]
    fn diff_applies_to_graph_views_only() {
        assert!(applies_to(ViewKind::Causal));
        assert!(applies_to(ViewKind::Structure));
        assert!(!applies_to(ViewKind::Trace));
        assert!(!applies_to(ViewKind::Matrix));
    }

    #[test]
    fn fields_to_refs() {
        assert_eq!(refs_from_fields("  ", "x"), None);
        assert_eq!(refs_from_fields("main", " "), Some(DiffRefs { base: "main".into(), head: None }));
        assert_eq!(
            refs_from_fields(" HEAD~1 ", "feature"),
            Some(DiffRefs { base: "HEAD~1".into(), head: Some("feature".into()) })
        );
    }

    #[test]
    fn git_errors_surface_as_values() {
        let file = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/order-fulfillment/cascade.yaml");
        let refs = DiffRefs { base: "definitely-not-a-ref-xyz".into(), head: None };
        let err = compute(&file, &refs, None).expect_err("no such ref");
        assert!(matches!(err, DiffError::Git { .. }), "{err}");
    }

    #[test]
    fn summary_counts_statuses() {
        let model = load_str(include_str!("../../../examples/order-fulfillment/cascade.yaml")).expect("loads");
        let graph = CausalGraph::build(&model);
        let diff = ModelDiff::from_statuses([
            ("event:OrderPaid".parse().expect("key"), DiffStatus::Added),
            ("event:Shipped".parse().expect("key"), DiffStatus::Removed),
        ]);
        assert_eq!(DiffDisplay { model, graph, diff }.summary(), "+1 ~0 −1");
    }
}
