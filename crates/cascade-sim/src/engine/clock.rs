//! The virtual clock and timers.
//!
//! - Entering a state arms one timer per timer trigger of the state's own
//!   transitions (in definition order), due `delay` from now. Leaving the
//!   state cancels them.
//! - Advancing the clock records `Wait`, then queues every timer due by the
//!   new time in deadline order (ties: arming order), each with a
//!   `TimerExpired` step caused by the `Wait`. A queued timer is delivered
//!   like a fire with an empty payload, and is dropped if the instance no
//!   longer accepts it.

use cascade_core::Model;
use cascade_core::expr::Duration;
use cascade_core::ids::{StateId, TriggerId};
use cascade_core::model::TriggerKind;

use super::core::{Armed, Core, QueueItem};
use super::instance::InstanceIx;
use super::record::RawKind;

/// The timer triggers of `state`'s own transitions, in definition order,
/// with their delays.
fn timers_of(model: &Model, state: StateId) -> Vec<(TriggerId, Duration)> {
    let machine = model.state(state).machine;
    let mut out: Vec<(TriggerId, Duration)> = Vec::new();
    for &t in &model.machine(machine).transitions {
        let transition = model.transition(t);
        if transition.from != state {
            continue;
        }
        if let TriggerKind::Timer(delay) = model.trigger(transition.trigger).kind
            && !out.iter().any(|(trigger, _)| *trigger == transition.trigger)
        {
            out.push((transition.trigger, delay));
        }
    }
    out
}

impl Core {
    /// Arm the timers of `entered` (outermost first) for an instance.
    pub(super) fn arm(&mut self, model: &Model, instance: InstanceIx, entered: &[StateId]) {
        for &state in entered {
            for (trigger, delay) in timers_of(model, state) {
                let deadline = self.now.saturating_add(delay);
                let seq = self.next_arm;
                self.next_arm += 1;
                let armed = Armed { instance, trigger, state, deadline, seq };
                let at = self.armed.partition_point(|a| (a.deadline, a.seq) <= (deadline, seq));
                self.armed.insert(at, armed);
            }
        }
    }

    /// Cancel an instance's timers armed by any of `exited`.
    pub(super) fn cancel(&mut self, instance: InstanceIx, exited: &[StateId]) {
        self.armed.retain(|a| a.instance != instance || !exited.contains(&a.state));
    }

    /// The earliest deadline, if any timer is armed.
    pub fn next_deadline(&self) -> Option<Duration> {
        self.armed.first().map(|a| a.deadline)
    }

    /// Advance the clock by `by` and queue the timers that came due.
    pub fn advance(&mut self, model: &Model, by: Duration) {
        self.now = self.now.saturating_add(by);
        let wait = self.recorder.push(None, RawKind::Wait { by, now: self.now });
        let due = self.armed.partition_point(|a| a.deadline <= self.now);
        let expired: Vec<Armed> = self.armed.drain(..due).collect();
        for armed in expired {
            let Armed { instance, trigger, .. } = armed;
            let step = self.recorder.push(Some(wait), RawKind::TimerExpired { instance, trigger });
            let label = format!("timer → {}: {}", self.instances[instance.0].name, model.trigger(trigger).name);
            let consumers = self.target_consumer(instance);
            self.enqueue(label, consumers, QueueItem::Timer { trigger, target: instance, expired: step });
        }
    }
}
