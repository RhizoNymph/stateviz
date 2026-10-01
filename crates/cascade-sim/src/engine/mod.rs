//! The simulator proper: a steppable core (instances, one global FIFO
//! queue, trace recording), the play actions performed on it, and the batch
//! run that drives it with a scenario.
//!
//! ```text
//! external fire ──▶ ExternalFire ──▶ Transition | Dropped ──▶ Emit ──▶ queue
//! queue item: Event ──▶ Deliver (per handler) ──▶ Fire | Spawn+Fire | NoTarget | Ambiguous
//!                                                   └──▶ queue
//! queue item: Fire  ──▶ Transition | Dropped ──▶ Emit ──▶ queue
//! ```
//!
//! - `core`: the state and one operation per thing that can happen.
//! - `exec`: a [`PlayAction`](crate::PlayAction) → checked core operations.
//! - `drive`: a scenario → play actions, for the batch run and
//!   `PlaySession::from_scenario` alike.

pub(crate) mod core;
pub(crate) mod drive;
pub(crate) mod exec;
pub(crate) mod instance;
pub(crate) mod record;
mod schedule;
mod select;

use cascade_core::Model;
use cascade_core::ids::RuleId;

use self::core::Core;
use self::drive::Player;
use self::exec::{Schedule, exec};
use crate::SimRun;
use crate::error::SimError;
use crate::scenario::ResolvedScenario;
use crate::session::PlayAction;

/// At most this many queue items (events and controller fires) are
/// delivered in one run or play session; past it the run fails with
/// [`SimError::StepLimit`].
pub const STEP_LIMIT: usize = 10_000;

/// Identifies one fire across deterministic reruns of a scenario: the
/// `occurrence`-th (0-based) fire of `rule` at the instance named
/// `instance`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FireKey {
    pub rule: RuleId,
    pub instance: String,
    pub occurrence: u32,
}

/// Reorder two contested fires: `yielder` lets the next fire of `overtaker`
/// at the same instance go first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Swap {
    pub yielder: FireKey,
    pub overtaker: RuleId,
}

pub(crate) struct Outcome {
    pub run: SimRun,
    /// With a [`Swap`]: whether the overtaker really went first.
    pub swapped: bool,
}

/// The batch simulator: a core driven by a scenario, draining in FIFO order
/// or with one race swap.
struct Batch {
    core: Core,
    schedule: Schedule,
}

impl Player for Batch {
    fn core(&self) -> &Core {
        &self.core
    }

    fn is_quiet(&self) -> bool {
        self.core.queue().is_empty() && !self.schedule.is_holding()
    }

    fn act(&mut self, model: &Model, action: PlayAction) -> Result<(), SimError> {
        exec(&mut self.core, model, &action, &mut self.schedule).map(drop)
    }
}

/// Run a validated scenario, in FIFO order or with one swap.
pub(crate) fn run(model: &Model, scenario: &ResolvedScenario, swap: Option<Swap>) -> Result<Outcome, SimError> {
    let mut batch = Batch { core: Core::default(), schedule: Schedule::new(swap) };
    drive::drive(model, scenario, &mut batch)?;
    let swapped = batch.schedule.swapped();
    tracing::debug!(
        scenario = %scenario.name,
        steps = batch.core.step_count(),
        instances = batch.core.instances().len(),
        swapped,
        "simulation finished"
    );
    Ok(Outcome { run: batch.core.finish(&scenario.name).run, swapped })
}
