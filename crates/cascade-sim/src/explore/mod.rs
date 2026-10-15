//! Exploration: every delivery ordering (and, optionally, every choice and
//! timer expiry) a scenario or a play position allows, deduplicated into
//! distinct outcomes, each with a witness that replays in a
//! [`PlaySession`].
//!
//! ```text
//! scenario ──validate──▶ inputs (fires, waits; choose/create/remove chained)
//! start state ──DFS over SearchStates (visited keys, sleep sets)──▶ state graph
//!   moves: deliver an eligible item to one consumer (ordering model)
//!          | the next input (immediate: any time; otherwise when quiet)
//!          | expire the earliest timers (timers_anytime)
//!          each × decisions at choice points (choices)
//! terminal nodes ──group──▶ outcomes ──BFS──▶ shortest witness, path counts
//! ```
//!
//! - `options`: [`OrderingModel`], [`ExploreOptions`].
//! - `inputs`: a scenario as inputs.
//! - `moves`: enabled moves, footprints and independence, performing one.
//! - `key`: the search key, with canonical queue orders under reduction.
//! - `search`: the DFS with sleep sets.
//! - `graph`: shortest paths and path counts over the explored graph.
//! - `outcome`: [`Exploration`], [`Outcome`], [`Anomaly`].
//! - `report`: text for hosts that list outcomes.
//!
//! See `docs/features/exploration.md`.

mod graph;
mod inputs;
mod key;
mod moves;
mod options;
mod outcome;
mod report;
mod search;

use std::collections::BTreeSet;

use cascade_core::Model;

pub use options::{DEFAULT_MAX_STATES, ExploreOptions, OrderingModel};
pub use outcome::{Anomaly, Exploration, Outcome, OutcomeTimer};
pub use report::{differing_instances, instance_text, timer_text};

use self::graph::Graph;
use self::inputs::Inputs;
use self::moves::{Base, MoveDesc, SearchState, Step, enabled, perform};
use crate::engine::core::Core;
use crate::engine::exec::{Schedule, exec};
use crate::error::SimError;
use crate::scenario::{Scenario, validate};
use crate::session::{PlayAction, PlaySession};

/// Explore `scenario`: its declared instances, then its external fires and
/// waits in file order at their timing, with every ordering of the cascades
/// in between that `options` allows. Witnesses start from
/// [`PlaySession::new`] (the declared instances are their first actions).
///
/// Errors: the scenario's problems as [`SimError::Scenario`].
pub fn explore(model: &Model, scenario: &Scenario, options: &ExploreOptions) -> Result<Exploration, SimError> {
    let resolved = validate(model, scenario)?;
    let inputs = inputs::from_scenario(model, &resolved, options.choices);
    let mut core = Core::default();
    let mut start = Vec::with_capacity(inputs.start.len());
    for action in &inputs.start {
        start.push(exec(&mut core, model, action, &mut Schedule::Fifo)?);
    }
    core.forget_trace();
    tracing::debug!(scenario = %scenario.name, inputs = inputs.inputs.len(), "exploring a scenario");
    run(model, SearchState { core, input: 0, anomalies: BTreeSet::new() }, start, &inputs, options)
}

/// Explore from a play session's current position: every ordering of what
/// is queued (and of what it causes). There are no inputs. Witnesses are
/// actions to apply to `session` from where it is (e.g. as a branch of its
/// timeline, with [`replay_witness`]).
pub fn explore_from(session: &PlaySession, model: &Model, options: &ExploreOptions) -> Result<Exploration, SimError> {
    let mut core = session.core().clone();
    core.forget_trace();
    run(model, SearchState { core, input: 0, anomalies: BTreeSet::new() }, Vec::new(), &Inputs::default(), options)
}

/// `start` with `witness` applied: for [`explore`], start from
/// [`PlaySession::new`]; for [`explore_from`], from the explored session.
pub fn replay_witness(start: &PlaySession, model: &Model, witness: &[PlayAction]) -> Result<PlaySession, SimError> {
    let mut session = start.clone();
    for action in witness {
        session.apply(model, action.clone())?;
    }
    Ok(session)
}

fn run(
    model: &Model,
    root: SearchState,
    start: Vec<PlayAction>,
    inputs: &Inputs,
    options: &ExploreOptions,
) -> Result<Exploration, SimError> {
    let searched = search::search(model, root.clone(), inputs, options)?;
    let shortest = searched.graph.shortest();
    let counts = searched.graph.path_counts();
    let mut outcomes = Vec::with_capacity(searched.found.len());
    for found in searched.found {
        let best = found
            .nodes
            .iter()
            .filter_map(|&n| shortest.get(n as usize).copied().flatten().map(|(dist, _)| (dist, n)))
            .min()
            .map(|(_, n)| n);
        let path: Vec<&MoveDesc> = best
            .map(|n| Graph::path_to(&shortest, n))
            .unwrap_or_default()
            .into_iter()
            .filter_map(|d| searched.descs.get(d as usize))
            .collect();
        let mut witness = start.clone();
        witness.extend(rebuild(model, &root, inputs, options, &path)?);
        let paths = found.nodes.iter().fold(0u64, |sum, &n| sum.saturating_add(counts[n as usize]));
        outcomes.push(Outcome {
            instances: found.instances,
            timers: found.timers,
            anomalies: found.anomalies,
            witness,
            paths,
        });
    }
    outcomes.sort_by_cached_key(|o| (o.witness.len(), o.sort_text()));
    tracing::info!(
        outcomes = outcomes.len(),
        states = searched.graph.nodes.len(),
        complete = searched.complete,
        "exploration finished"
    );
    let mut notes = inputs.notes.clone();
    if searched.complete && outcomes.is_empty() {
        notes.push("no ordering settles: every path cycles without the queue emptying".to_owned());
    }
    Ok(Exploration { outcomes, states: searched.graph.nodes.len(), complete: searched.complete, notes })
}

/// Replay a path of move descriptions from the start, turning each into the
/// play actions it takes in the states actually reached (positions differ
/// between equivalent states).
fn rebuild(
    model: &Model,
    root: &SearchState,
    inputs: &Inputs,
    options: &ExploreOptions,
    path: &[&MoveDesc],
) -> Result<Vec<PlayAction>, SimError> {
    let mut state = root.clone();
    let mut actions = Vec::new();
    for (at, desc) in path.iter().enumerate() {
        let decisions: &[String] = match desc {
            MoveDesc::Halt => break,
            MoveDesc::Deliver { decisions, .. } | MoveDesc::Input { decisions, .. } => decisions,
            MoveDesc::Advance => &[],
        };
        let base = enabled(model, &state, inputs, options).into_iter().find_map(|e| {
            let matches = match (&e.base, desc) {
                (Base::Deliver { consumer, .. }, MoveDesc::Deliver { item, consumer: c, .. }) => {
                    e.item.as_ref() == Some(item) && consumer == c
                }
                (Base::Input(i), MoveDesc::Input { index, .. }) => i == index,
                (Base::Advance, MoveDesc::Advance) => true,
                _ => false,
            };
            matches.then_some(e.base)
        });
        let Some(base) = base else {
            tracing::error!(at, "exploration lost a witness move");
            return Err(SimError::WitnessLost { at });
        };
        match perform(model, &state, inputs, &base, decisions)? {
            Step::Done(performed) => {
                actions.extend(performed.actions);
                state = performed.state;
            }
            Step::Halt => break,
        }
    }
    Ok(actions)
}
