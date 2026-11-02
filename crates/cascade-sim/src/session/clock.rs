//! The clock, armed timers, pending decisions, choice points and the
//! session's state key.

use std::fmt::Write as _;

use cascade_core::Model;
use cascade_core::expr::Duration;

use super::{ArmedTimer, ChoicePoint, PlayAction, PlaySession, StateKey};
use crate::engine::core::QueueItem;
use crate::engine::exec::{Schedule, exec};
use crate::error::SimError;

impl PlaySession {
    /// The current clock time (zero at the start).
    pub fn now(&self) -> Duration {
        self.core.now()
    }

    /// Armed timers in deadline order (ties: arming order).
    pub fn timers(&self) -> Vec<ArmedTimer> {
        self.core
            .armed()
            .iter()
            .map(|a| ArmedTimer {
                instance: self.core.instance(a.instance).name.clone(),
                trigger: a.trigger,
                deadline: a.deadline,
            })
            .collect()
    }

    /// Decisions pushed by `Choose` and not yet taken, oldest first.
    pub fn pending_decisions(&self) -> &[String] {
        self.core.decisions()
    }

    /// The choice points `Step { choice, consumer }` would hit with the
    /// current pending decisions, in the order it would hit them, without
    /// changing anything. Fails like the step would.
    pub fn choice_points(
        &self,
        model: &Model,
        choice: Option<u32>,
        consumer: Option<&str>,
    ) -> Result<Vec<ChoicePoint>, SimError> {
        self.choice_points_of(model, &PlayAction::Step { choice, consumer: consumer.map(str::to_owned) })
    }

    /// The choice points any `action` would hit (an external `Fire` into
    /// `?choice` guards, a `Step`, …) with the current pending decisions,
    /// in the order it would hit them, without changing anything. Fails
    /// like the action would.
    pub fn choice_points_of(&self, model: &Model, action: &PlayAction) -> Result<Vec<ChoicePoint>, SimError> {
        let mut core = self.core.clone();
        let before = core.step_count();
        exec(&mut core, model, action, &mut Schedule::Fifo)?;
        Ok(core.choices_since(model, before).into_iter().map(|(at, options)| ChoicePoint { at, options }).collect())
    }

    /// See [`StateKey`].
    pub fn state_key(&self) -> StateKey {
        let core = &self.core;
        let mut key = String::new();
        for instance in core.instances() {
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
        for queued in core.queue() {
            let _ = match &queued.item {
                QueueItem::Event { event, payload, .. } => write!(key, "event {event:?} {payload:?}"),
                QueueItem::Fire { rule, target, payload, .. } => {
                    write!(key, "fire {rule:?} {} {payload:?}", core.instance(*target).name)
                }
                QueueItem::Timer { trigger, target, .. } => {
                    write!(key, "timer {trigger:?} {}", core.instance(*target).name)
                }
            };
            let names: Vec<&str> = queued.consumers_left.iter().map(|c| c.name.as_str()).collect();
            let _ = write!(key, " {names:?};");
        }
        // Timers relative to now, so two sessions that differ only in how
        // much time has passed share a key.
        for armed in core.armed() {
            let _ = write!(
                key,
                "armed {} {:?} {:?} +{};",
                core.instance(armed.instance).name,
                armed.trigger,
                armed.state,
                armed.deadline.saturating_sub(core.now()).millis()
            );
        }
        for decision in core.decisions() {
            let _ = write!(key, "decision {decision:?};");
        }
        StateKey(key)
    }
}
