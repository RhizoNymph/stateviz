//! Watched explorations: progress and cancellation (for hosts that explore
//! off their UI thread).

mod explore_support;

use cascade_sim::{ExploreOptions, ExploreProgress, PlaySession, SimError, explore_from, explore_from_watched};
use explore_support::*;

fn paused_race(model: &cascade_core::Model) -> PlaySession {
    let paused = format!("{RACE_SCENARIO}end: pause\n");
    PlaySession::from_scenario(model, &scenario(&paused)).unwrap_or_else(|e| panic!("{e}"))
}

#[test]
fn a_watched_exploration_reports_its_states_and_matches_the_plain_one() {
    let model = model(RACE);
    let session = paused_race(&model);
    let progress = ExploreProgress::new();
    assert_eq!(progress.states(), 0);
    let watched =
        explore_from_watched(&session, &model, &ExploreOptions::default(), &progress).unwrap_or_else(|e| panic!("{e}"));
    let plain = explore_from(&session, &model, &ExploreOptions::default()).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(watched, plain);
    assert_eq!(progress.states(), watched.states);
    assert!(!progress.is_cancelled());
}

#[test]
fn a_cancelled_exploration_stops_with_a_typed_error() {
    let model = model(RACE);
    let session = paused_race(&model);
    let progress = ExploreProgress::new();
    progress.cancel();
    assert!(progress.is_cancelled());
    let result = explore_from_watched(&session, &model, &ExploreOptions::default(), &progress);
    match result {
        Err(SimError::ExploreCancelled { states }) => assert_eq!(states, 1, "only the root was visited"),
        other => panic!("expected a cancellation, got {other:?}"),
    }
}
