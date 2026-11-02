//! Previewing an outcome on the canvas and opening it as a branch of the
//! play timeline.
//!
//! Both replay the outcome's witness from the run's origin on a copy of the
//! play state: the preview draws the copy (with the instances that make the
//! outcome different emphasised), opening replaces the play state with it.
//! Applying the witness at the origin forks the timeline like any other
//! action after seeking back, so the line the user was on stays a branch.

use cascade_core::{ElementRef, Model};
use cascade_scene::PlayOverlay;
use cascade_sim::SimError;

use super::ExploreResult;
use crate::play::PlayState;

/// Why an outcome cannot be previewed or opened.
#[derive(Debug, thiserror::Error)]
pub enum OpenError {
    #[error("nothing explored yet: run the exploration first")]
    NoResult,
    /// 1-based, as the panel numbers outcomes.
    #[error("there is no outcome {0}")]
    NoSuchOutcome(usize),
    #[error("the results are stale (the model changed or the session left the explored line); explore again")]
    Stale,
    #[error("outcome {number} no longer replays: {error}")]
    Replay { number: usize, error: SimError },
}

/// `state` with outcome `index`'s witness applied from the run's origin.
fn branched(result: &ExploreResult, index: usize, state: &PlayState, model: &Model) -> Result<PlayState, OpenError> {
    if !result.freshness(state).usable() {
        return Err(OpenError::Stale);
    }
    let outcome = result.exploration.outcomes.get(index).ok_or(OpenError::NoSuchOutcome(index + 1))?;
    let mut next = state.clone();
    next.open_witness(model, result.origin.position(), &outcome.witness)
        .map_err(|error| OpenError::Replay { number: index + 1, error })?;
    Ok(next)
}

/// The canvas overlay of outcome `index`: where every instance ends up,
/// with the states of the instances that make it different (see
/// [`OutcomeRow::differs`](super::rows::OutcomeRow::differs)) drawn active.
pub fn preview(
    result: &ExploreResult,
    index: usize,
    state: &PlayState,
    model: &Model,
) -> Result<PlayOverlay, OpenError> {
    let next = branched(result, index, state, model)?;
    let mut overlay = next.overlay(model);
    let differs = result.rows.get(index).map(|row| row.differs.as_slice()).unwrap_or_default();
    overlay.active = next
        .session()
        .instances()
        .iter()
        .filter(|i| differs.contains(&i.name))
        .map(|i| model.key_of(ElementRef::State(i.state)))
        .fold(Vec::new(), |mut keys, key| {
            if !keys.contains(&key) {
                keys.push(key);
            }
            keys
        });
    overlay.pending.clear();
    Ok(overlay)
}

/// Open outcome `index` as a branch: on success `state` is at the end of
/// the outcome's ordering, and the timeline steps back through it. On
/// error `state` is unchanged.
pub fn open(result: &ExploreResult, index: usize, state: &mut PlayState, model: &Model) -> Result<(), OpenError> {
    *state = branched(result, index, state, model)?;
    Ok(())
}
