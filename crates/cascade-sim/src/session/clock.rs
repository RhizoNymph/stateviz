//! The clock, armed timers, pending decisions, choice points and the
//! session's state key.
//!
//! Timers, choices and the clock do not run yet: the clock stays at zero,
//! nothing is armed, no decision is pending and no step hits a choice
//! point.

use std::fmt::Write as _;

use cascade_core::Model;
use cascade_core::expr::Duration;

use super::{ArmedTimer, ChoicePoint, PlaySession, StateKey};
use crate::engine::core::QueueItem;
use crate::error::SimError;

impl PlaySession {
    /// The current clock time (zero at the start).
    pub fn now(&self) -> Duration {
        Duration::ZERO
    }

    /// Armed timers in deadline order (ties: arming order).
    pub fn timers(&self) -> Vec<ArmedTimer> {
        Vec::new()
    }

    /// Decisions pushed by `Choose` and not yet taken, oldest first.
    pub fn pending_decisions(&self) -> &[String] {
        &[]
    }

    /// The choice points `Step { choice, consumer }` would hit with the
    /// current pending decisions, without changing anything.
    pub fn choice_points(
        &self,
        model: &Model,
        choice: Option<u32>,
        consumer: Option<&str>,
    ) -> Result<Vec<ChoicePoint>, SimError> {
        let _ = (model, choice, consumer);
        Ok(Vec::new())
    }

    /// See [`StateKey`].
    pub fn state_key(&self) -> StateKey {
        let mut key = String::new();
        for instance in self.core.instances() {
            if instance.removed {
                let _ = write!(key, "removed {};", instance.name);
                continue;
            }
            let _ = write!(
                key,
                "instance {} {:?} {:?} {:?} {:?};",
                instance.name,
                instance.machine,
                instance.leaf(),
                instance.fields,
                instance.history_entries()
            );
        }
        for queued in self.core.queue() {
            let _ = match &queued.item {
                QueueItem::Event { event, payload, .. } => write!(key, "event {event:?} {payload:?}"),
                QueueItem::Fire { rule, target, payload, .. } => {
                    write!(key, "fire {rule:?} {} {payload:?}", self.core.instance(*target).name)
                }
            };
            let _ = write!(key, " {:?};", queued.consumers_left);
        }
        StateKey(key)
    }
}
