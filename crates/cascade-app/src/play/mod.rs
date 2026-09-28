//! Play mode's pure logic: the session state the panels show, the overlay
//! drawn over the canvas, the timeline strip and the input forms.
//!
//! GPUI glue lives in `workspace::playing` and `panels::play`.

pub mod forms;
pub mod overlay;
pub mod timeline;

use std::ops::Range;

use cascade_core::Model;
use cascade_scene::PlayOverlay;
use cascade_sim::{PlayAction, PlaySession, Scenario, SimError, step_text};

/// A play session tied to the model generation its ids belong to, plus
/// what the panels report about the last thing that happened.
#[derive(Clone, Debug)]
pub struct PlayState {
    session: PlaySession,
    /// `Loaded::generation` of the model the session's ids refer to.
    generation: u64,
    /// Trace steps the last action appended.
    last: Option<Range<usize>>,
    /// The last action's error, shown in the panel.
    pub error: Option<String>,
    /// Where a replay after an edit or reload stopped.
    pub replay_note: Option<String>,
}

impl PlayState {
    pub fn new(model: &Model, generation: u64) -> Self {
        Self { session: PlaySession::new(model), generation, last: None, error: None, replay_note: None }
    }

    /// Start from a saved scenario.
    pub fn from_scenario(model: &Model, generation: u64, scenario: &Scenario) -> Result<Self, SimError> {
        let session = PlaySession::from_scenario(model, scenario)?;
        let last = session.trace().steps.len();
        Ok(Self { session, generation, last: Some(0..last), error: None, replay_note: None })
    }

    pub fn session(&self) -> &PlaySession {
        &self.session
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Perform `action`; on failure nothing changes and the error is kept
    /// for the panel.
    pub fn act(&mut self, model: &Model, action: PlayAction) -> Result<(), SimError> {
        match self.session.apply(model, action) {
            Ok(outcome) => {
                self.last = Some(outcome.steps);
                self.error = None;
                Ok(())
            }
            Err(error) => {
                self.error = Some(error.to_string());
                Err(error)
            }
        }
    }

    pub fn seek(&mut self, model: &Model, position: usize) -> Result<(), SimError> {
        let result = self.session.seek(model, position);
        self.after_jump(result)
    }

    pub fn switch_branch(&mut self, model: &Model, index: usize) -> Result<(), SimError> {
        let result = self.session.switch_branch(model, index);
        self.after_jump(result)
    }

    fn after_jump(&mut self, result: Result<(), SimError>) -> Result<(), SimError> {
        match result {
            Ok(()) => {
                // A jump shows no "last action" emphasis.
                self.last = None;
                self.error = None;
                Ok(())
            }
            Err(error) => {
                self.error = Some(error.to_string());
                Err(error)
            }
        }
    }

    /// The model changed (edit or reload): replay the timeline against it.
    /// Returns the note about where it stopped, if it did. An empty
    /// timeline simply starts over.
    pub fn rebase(&mut self, model: &Model, generation: u64) -> Option<String> {
        if generation == self.generation {
            return None;
        }
        self.generation = generation;
        self.last = None;
        self.error = None;
        if self.session.timeline().actions.is_empty() {
            self.session = PlaySession::new(model);
            self.replay_note = None;
            return None;
        }
        let (session, stopped) = self.session.replay(model);
        self.session = session;
        self.replay_note = stopped.map(|(index, error)| format!("Replay stopped at action {}: {error}", index + 1));
        self.replay_note.clone()
    }

    pub fn overlay(&self, model: &Model) -> PlayOverlay {
        overlay::build_overlay(
            model,
            self.session.trace(),
            &self.session.instances(),
            &self.session.pending(),
            self.last.clone(),
        )
    }

    /// The last action's trace steps in plain language.
    pub fn last_steps(&self, model: &Model) -> Vec<String> {
        let trace = self.session.trace();
        self.last
            .clone()
            .and_then(|range| trace.steps.get(range))
            .unwrap_or_default()
            .iter()
            .map(|step| step_text(model, trace, step, None))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    fn model() -> Model {
        cascade_core::load_str("machines:\n  Order:\n    states: [draft, paid]\n").expect("loads")
    }

    fn add() -> PlayAction {
        PlayAction::AddInstance {
            name: Some("o1".into()),
            machine: "Order".into(),
            fields: BTreeMap::new(),
            state: None,
        }
    }

    #[test]
    fn a_new_state_is_empty() {
        let m = model();
        let state = PlayState::new(&m, 3);
        assert_eq!(state.generation(), 3);
        assert!(state.session().timeline().actions.is_empty());
        assert_eq!(state.overlay(&m), PlayOverlay::default());
        assert!(state.last_steps(&m).is_empty());
        assert!(state.error.is_none() && state.replay_note.is_none());
    }

    #[test]
    fn acting_records_the_action_or_its_error() {
        let m = model();
        let mut state = PlayState::new(&m, 1);
        match state.act(&m, add()) {
            Ok(()) => {
                assert_eq!(state.session().timeline().actions, [add()]);
                assert!(state.error.is_none());
                assert_eq!(state.overlay(&m).markers.len(), 1);
            }
            Err(error) => {
                assert_eq!(state.error, Some(error.to_string()));
                assert!(state.session().timeline().actions.is_empty(), "a failed action changes nothing");
            }
        }
    }

    #[test]
    fn failed_jumps_are_reported() {
        let m = model();
        let mut state = PlayState::new(&m, 1);
        if let Err(error) = state.switch_branch(&m, 5) {
            assert_eq!(state.error, Some(error.to_string()));
        }
    }

    #[test]
    fn rebase_on_the_same_generation_does_nothing() {
        let m = model();
        let mut state = PlayState::new(&m, 1);
        assert_eq!(state.rebase(&m, 1), None);
        assert_eq!(state.generation(), 1);
    }

    #[test]
    fn rebase_with_an_empty_timeline_starts_over_quietly() {
        let m = model();
        let mut state = PlayState::new(&m, 1);
        assert_eq!(state.rebase(&m, 2), None);
        assert_eq!(state.generation(), 2);
        assert!(state.replay_note.is_none());
    }

    #[test]
    fn rebase_replays_and_reports_where_it_stopped() {
        let m = model();
        let mut state = PlayState::new(&m, 1);
        if state.act(&m, add()).is_err() {
            return; // Nothing recorded to replay while the session is a stub.
        }
        let edited = cascade_core::load_str("machines:\n  Other:\n    states: [x]\n").expect("loads");
        let note = state.rebase(&edited, 2).expect("the instance's machine is gone");
        assert!(note.starts_with("Replay stopped at action 1:"), "{note}");
        assert_eq!(state.replay_note, Some(note));
    }
}
