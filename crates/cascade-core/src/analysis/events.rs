//! Unhandled event and orphan controller (warnings): the two ends of an
//! event that do not meet.
//!
//! - An event emitted by at least one transition with no subscribed handler
//!   is unhandled. Events that are declared but neither emitted nor handled
//!   are not reported.
//! - Each handler subscribed to an event no transition emits is an orphan,
//!   reported per handler, so two controllers waiting on the same dead event
//!   give two findings.

use super::describe;
use super::{Finding, FindingDetail};
use crate::model::Model;

pub(super) fn check(model: &Model) -> Vec<Finding> {
    let mut findings = unhandled_events(model);
    findings.extend(orphan_handlers(model));
    findings
}

fn unhandled_events(model: &Model) -> Vec<Finding> {
    model
        .events()
        .filter(|(_, e)| !e.emitted_by.is_empty() && e.handlers.is_empty())
        .map(|(event, e)| {
            let emitters = match e.emitted_by.as_slice() {
                [only] => model.transition_label(*only),
                [first, rest @ ..] => format!(
                    "{} and {}",
                    model.transition_label(*first),
                    describe::count(rest.len(), "other transition")
                ),
                [] => String::new(),
            };
            Finding::new(
                FindingDetail::UnhandledEvent { event },
                format!("{} is emitted by {emitters}, but no controller subscribes to it", e.name),
            )
        })
        .collect()
}

fn orphan_handlers(model: &Model) -> Vec<Finding> {
    model
        .handlers()
        .filter(|(_, h)| model.event(h.event).emitted_by.is_empty())
        .map(|(handler, h)| {
            Finding::new(
                FindingDetail::OrphanController { handler },
                format!(
                    "{} subscribes to {}, but no transition emits it",
                    model.controller(h.controller).name,
                    model.event(h.event).name
                ),
            )
        })
        .collect()
}
