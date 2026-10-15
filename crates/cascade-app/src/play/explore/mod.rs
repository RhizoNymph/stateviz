//! The Explore panel's logic: the controls, a run's lifecycle (started off
//! the UI thread, watched, cancellable), where it started and whether the
//! session is still there, the outcome rows, previews and opening an
//! outcome as a branch.
//!
//! ```text
//! controls ──options()──▶ ExploreJob { session clone, model, options, progress }
//!     (background) job.run() ──explore_from_watched──▶ ExploreResult (rows built with the job's model)
//!     (UI) ExploreUi::finish ──▶ Done / Failed / Idle (cancelled)
//! result + PlayState ──freshness──▶ Current | Moved (origin still on the line) | Stale
//! select outcome ──branch::preview──▶ PlayOverlay (outcome instances, differing states active)
//! open outcome ──PlayState::open_witness──▶ seek to the origin, apply the witness (forks the timeline)
//! ```
//!
//! Pure: no GPUI. `workspace::exploring` runs jobs and `panels::explore`
//! draws the panel.

pub mod branch;
pub mod rows;

use std::sync::Arc;

use cascade_core::Model;
use cascade_sim::{
    Exploration, ExploreOptions, ExploreProgress, OrderingModel, PlayAction, PlaySession, SimError,
    explore_from_watched,
};

use self::rows::{OutcomeRow, outcome_rows, summary_text};
use super::PlayState;

/// The panel's switches. The state bound is the simulator's default.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ExploreControls {
    pub ordering: OrderingModel,
    /// Branch on every option at choice points.
    pub choices: bool,
    /// Let armed timers expire between any two deliveries.
    pub timers_anytime: bool,
}

impl ExploreControls {
    /// The options a run with these controls explores with.
    pub fn options(&self) -> ExploreOptions {
        ExploreOptions {
            ordering: self.ordering,
            choices: self.choices,
            timers_anytime: self.timers_anytime,
            ..ExploreOptions::default()
        }
    }

    /// `per-consumer`, `unordered, choices, timers anytime`, …
    pub fn label(&self) -> String {
        let mut parts = vec![self.ordering.name()];
        if self.choices {
            parts.push("choices");
        }
        if self.timers_anytime {
            parts.push("timers anytime");
        }
        parts.join(", ")
    }
}

/// Where a run started: the model generation and the actions applied at
/// that point of the timeline. Witnesses apply from there.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Origin {
    pub generation: u64,
    pub actions: Vec<PlayAction>,
}

impl Origin {
    pub fn of(state: &PlayState) -> Self {
        let timeline = state.session().timeline();
        Self { generation: state.generation(), actions: timeline.actions[..timeline.position].to_vec() }
    }

    /// The timeline position the run started at.
    pub fn position(&self) -> usize {
        self.actions.len()
    }
}

/// Whether a result still describes the session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Freshness {
    /// The session is where the run started.
    Current,
    /// The session moved along the timeline, but the origin is still on the
    /// current line: an outcome can be opened (seeking back first) and
    /// previewed; the results are shown as stale.
    Moved,
    /// The model changed, or the origin is no longer on the current line:
    /// results are kept for reading only.
    Stale,
}

impl Freshness {
    pub fn of(origin: &Origin, state: &PlayState) -> Self {
        if state.generation() != origin.generation {
            return Freshness::Stale;
        }
        let timeline = state.session().timeline();
        if timeline.actions[..timeline.position] == origin.actions[..] {
            Freshness::Current
        } else if timeline.actions.starts_with(&origin.actions) {
            Freshness::Moved
        } else {
            Freshness::Stale
        }
    }

    /// Outcomes can be previewed and opened.
    pub fn usable(self) -> bool {
        self != Freshness::Stale
    }
}

/// A finished run: the exploration and its rows, built with the model the
/// run explored (so they read right even after the model changes).
#[derive(Clone, Debug)]
pub struct ExploreResult {
    pub origin: Origin,
    pub controls: ExploreControls,
    pub exploration: Exploration,
    pub rows: Vec<OutcomeRow>,
    /// `2 outcomes, 7 states (complete)`.
    pub summary: String,
}

impl ExploreResult {
    pub fn new(model: &Model, origin: Origin, controls: ExploreControls, exploration: Exploration) -> Self {
        let rows = outcome_rows(model, &exploration);
        let summary = summary_text(&exploration, &controls.options());
        Self { origin, controls, exploration, rows, summary }
    }

    pub fn freshness(&self, state: &PlayState) -> Freshness {
        Freshness::of(&self.origin, state)
    }
}

/// Everything a background run needs, owned.
pub struct ExploreJob {
    pub session: PlaySession,
    pub model: Arc<Model>,
    pub origin: Origin,
    pub controls: ExploreControls,
    pub progress: Arc<ExploreProgress>,
}

impl ExploreJob {
    /// Explore from the session's position (blocking: run it off the UI
    /// thread).
    pub fn run(self) -> Result<ExploreResult, SimError> {
        let exploration = explore_from_watched(&self.session, &self.model, &self.controls.options(), &self.progress)?;
        Ok(ExploreResult::new(&self.model, self.origin, self.controls, exploration))
    }
}

/// A run's lifecycle.
#[derive(Clone, Debug, Default)]
pub enum ExploreRun {
    #[default]
    Idle,
    Running {
        origin: Origin,
        controls: ExploreControls,
        progress: Arc<ExploreProgress>,
    },
    Done(Arc<ExploreResult>),
    Failed {
        origin: Origin,
        error: String,
    },
}

/// What finishing a run did, for the status bar.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Finished {
    /// The result is for a run that is no longer the current one.
    Ignored,
    Done {
        summary: String,
    },
    Cancelled {
        states: usize,
    },
    Failed {
        error: String,
    },
}

/// The panel's state.
#[derive(Clone, Debug, Default)]
pub struct ExploreUi {
    pub open: bool,
    pub controls: ExploreControls,
    pub run: ExploreRun,
    /// The outcome being previewed (index into the result's outcomes).
    pub selected: Option<usize>,
    /// The selected outcome's overlay, computed on selection, with the
    /// session it was computed for: acting or moving on the timeline ends
    /// the preview, so the canvas never hides what the session does.
    pub preview: Option<Preview>,
}

/// A computed preview and where the session was when it was computed.
#[derive(Clone, Debug)]
pub struct Preview {
    pub overlay: cascade_scene::PlayOverlay,
    generation: u64,
    timeline: cascade_sim::Timeline,
}

impl Preview {
    fn applies_to(&self, state: &PlayState) -> bool {
        self.generation == state.generation() && &self.timeline == state.session().timeline()
    }
}

impl ExploreUi {
    /// Start a run from `state`'s position: the run becomes `Running` (any
    /// running job is cancelled, old results and the selection go) and the
    /// job to run in the background is returned.
    pub fn start(&mut self, state: &PlayState, model: Arc<Model>) -> ExploreJob {
        self.cancel();
        let origin = Origin::of(state);
        let progress = Arc::new(ExploreProgress::new());
        self.run =
            ExploreRun::Running { origin: origin.clone(), controls: self.controls, progress: Arc::clone(&progress) };
        self.clear_selection();
        ExploreJob { session: state.session().clone(), model, origin, controls: self.controls, progress }
    }

    /// Ask a running job to stop; it finishes with a cancellation.
    pub fn cancel(&mut self) {
        if let ExploreRun::Running { progress, .. } = &self.run {
            progress.cancel();
        }
    }

    /// States visited by the running job.
    pub fn running_states(&self) -> Option<usize> {
        match &self.run {
            ExploreRun::Running { progress, .. } => Some(progress.states()),
            _ => None,
        }
    }

    /// A job finished. Results of a job that is no longer the running one
    /// (restarted since) are ignored.
    pub fn finish(&mut self, progress: &Arc<ExploreProgress>, result: Result<ExploreResult, SimError>) -> Finished {
        let origin = match &self.run {
            ExploreRun::Running { progress: running, origin, .. } if Arc::ptr_eq(running, progress) => origin.clone(),
            _ => return Finished::Ignored,
        };
        match result {
            Ok(result) => {
                let summary = result.summary.clone();
                self.run = ExploreRun::Done(Arc::new(result));
                Finished::Done { summary }
            }
            Err(SimError::ExploreCancelled { states }) => {
                self.run = ExploreRun::Idle;
                Finished::Cancelled { states }
            }
            Err(error) => {
                let error = error.to_string();
                self.run = ExploreRun::Failed { origin, error: error.clone() };
                Finished::Failed { error }
            }
        }
    }

    pub fn result(&self) -> Option<&Arc<ExploreResult>> {
        match &self.run {
            ExploreRun::Done(result) => Some(result),
            _ => None,
        }
    }

    pub fn clear_selection(&mut self) {
        self.selected = None;
        self.preview = None;
    }

    /// Select outcome `index` (or deselect it when it is selected) and
    /// compute its preview. Errors leave nothing selected.
    pub fn toggle_select(&mut self, index: usize, state: &PlayState, model: &Model) -> Result<(), branch::OpenError> {
        if self.selected == Some(index) {
            self.clear_selection();
            return Ok(());
        }
        self.clear_selection();
        let result = self.result().ok_or(branch::OpenError::NoResult)?;
        let overlay = branch::preview(result, index, state, model)?;
        self.selected = Some(index);
        self.preview =
            Some(Preview { overlay, generation: state.generation(), timeline: state.session().timeline().clone() });
        Ok(())
    }

    /// The outcome previewed on the canvas for `state`: the panel is open,
    /// the results are usable, and the session has not changed since the
    /// outcome was selected.
    pub fn previewing(&self, state: &PlayState) -> Option<usize> {
        let result = self.result()?;
        let preview = self.preview.as_ref()?;
        (self.open && result.freshness(state).usable() && preview.applies_to(state)).then_some(self.selected?)
    }

    /// The overlay to draw instead of the session's, while
    /// [`previewing`](Self::previewing).
    pub fn preview_for(&self, state: &PlayState) -> Option<&cascade_scene::PlayOverlay> {
        self.previewing(state)?;
        self.preview.as_ref().map(|p| &p.overlay)
    }
}

#[cfg(test)]
mod tests;
