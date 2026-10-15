//! Explore panel glue: running an exploration on the background executor
//! (with a ticker that repaints the "states explored" readout), cancelling,
//! previewing an outcome and opening it as a branch.

use std::sync::Arc;
use std::time::Duration;

use cascade_sim::{ExploreProgress, OrderingModel, SimError};
use gpui::{AppContext as _, Context, Task};

use super::Workspace;
use super::building::capitalize;
use crate::play::explore::branch;
use crate::play::explore::{ExploreResult, Finished};

/// How often the readout repaints while a run is going.
const TICK: Duration = Duration::from_millis(150);

/// The running job's foreground tasks. Replacing the run task drops the old
/// job's result; the job itself stops at its next move once cancelled.
#[derive(Default)]
pub struct ExploreTasks {
    run: Option<Task<()>>,
    ticker: Option<Task<()>>,
}

impl Workspace {
    /// `E`: open or close the Explore panel (Play mode only).
    pub(crate) fn toggle_explore(&mut self, cx: &mut Context<Self>) {
        if !self.in_play("explore orderings", cx) {
            return;
        }
        let explore = &mut self.play.explore;
        explore.open = !explore.open;
        self.scene_dirty = true;
        cx.notify();
    }

    pub(crate) fn set_explore_ordering(&mut self, ordering: OrderingModel, cx: &mut Context<Self>) {
        self.play.explore.controls.ordering = ordering;
        cx.notify();
    }

    pub(crate) fn toggle_explore_choices(&mut self, cx: &mut Context<Self>) {
        let controls = &mut self.play.explore.controls;
        controls.choices = !controls.choices;
        cx.notify();
    }

    pub(crate) fn toggle_explore_timers(&mut self, cx: &mut Context<Self>) {
        let controls = &mut self.play.explore.controls;
        controls.timers_anytime = !controls.timers_anytime;
        cx.notify();
    }

    /// Explore from the session's position with the panel's controls.
    pub(crate) fn run_explore(&mut self, cx: &mut Context<Self>) {
        self.ensure_play();
        let (Some(loaded), Some(state)) = (self.doc.loaded().cloned(), self.play.state.as_ref()) else {
            self.set_status("Nothing to explore: no model is loaded", true);
            cx.notify();
            return;
        };
        let job = self.play.explore.start(state, Arc::clone(&loaded.model));
        let progress = Arc::clone(&job.progress);
        tracing::info!(controls = %job.controls.label(), position = job.origin.position(), "exploring from the session");
        let work = cx.background_spawn(async move { job.run() });
        self.play.explore_tasks.run = Some(cx.spawn(async move |this, cx| {
            let result = work.await;
            if let Err(error) = this.update(cx, |ws, cx| ws.finish_explore(&progress, result, cx)) {
                tracing::debug!(%error, "workspace gone before the exploration finished");
            }
        }));
        self.play.explore_tasks.ticker = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(TICK).await;
                let running = this.update(cx, |ws, cx| {
                    cx.notify();
                    ws.play.explore.running_states().is_some()
                });
                if !matches!(running, Ok(true)) {
                    break;
                }
            }
        }));
        self.scene_dirty = true;
        cx.notify();
    }

    pub(crate) fn cancel_explore(&mut self, cx: &mut Context<Self>) {
        self.play.explore.cancel();
        self.set_status("Cancelling the exploration…", false);
        cx.notify();
    }

    fn finish_explore(
        &mut self,
        progress: &Arc<ExploreProgress>,
        result: Result<ExploreResult, SimError>,
        cx: &mut Context<Self>,
    ) {
        match self.play.explore.finish(progress, result) {
            Finished::Ignored => return,
            Finished::Done { summary } => {
                tracing::info!(%summary, "exploration finished");
                self.set_status(format!("Explored: {summary}"), false);
            }
            Finished::Cancelled { states } => {
                tracing::info!(states, "exploration cancelled");
                self.set_status(format!("Exploration cancelled after {states} states"), false);
            }
            Finished::Failed { error } => {
                tracing::warn!(%error, "exploration failed");
                self.set_status(format!("Exploration failed: {error}"), true);
            }
        }
        self.play.explore_tasks.ticker = None;
        self.scene_dirty = true;
        cx.notify();
    }

    /// Click on an outcome: preview it on the canvas (again: stop).
    pub(crate) fn select_outcome(&mut self, index: usize, cx: &mut Context<Self>) {
        let (Some(loaded), Some(state)) = (self.doc.loaded().cloned(), self.play.state.as_ref()) else {
            return;
        };
        if let Err(error) = self.play.explore.toggle_select(index, state, &loaded.model) {
            tracing::warn!(%error, outcome = index + 1, "cannot preview the outcome");
            self.set_status(capitalize(&error.to_string()), true);
        }
        self.scene_dirty = true;
        cx.notify();
    }

    /// "Open as branch": apply the outcome's witness from where the run
    /// started, forking the timeline.
    pub(crate) fn open_outcome(&mut self, index: usize, cx: &mut Context<Self>) {
        self.play.prompt = None;
        let Some(result) = self.play.explore.result().cloned() else {
            return;
        };
        let (Some(loaded), Some(state)) = (self.doc.loaded().cloned(), self.play.state.as_mut()) else {
            return;
        };
        match branch::open(&result, index, state, &loaded.model) {
            Ok(()) => {
                tracing::info!(outcome = index + 1, "outcome opened as a branch");
                self.play.explore.clear_selection();
                self.set_status(
                    format!("Opened outcome {} as a branch; step back through it with the timeline", index + 1),
                    false,
                );
            }
            Err(error) => {
                tracing::warn!(%error, outcome = index + 1, "cannot open the outcome");
                self.set_status(capitalize(&error.to_string()), true);
            }
        }
        self.changed(cx);
    }
}
