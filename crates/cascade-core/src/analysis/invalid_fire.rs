//! Invalid fire (error): a rule fires a trigger that no transition accepts,
//! or an external source exposes one (a dead command).
//!
//! A trigger is accepted when any transition of its machine takes it, from
//! any state. Whether the fire is dropped in the target's *current* state is
//! the separate state-dependent-fire note.

use super::describe;
use super::{Finding, FindingDetail};
use crate::model::Model;

pub(super) fn check(model: &Model) -> Vec<Finding> {
    let mut findings = Vec::new();
    for (rule, r) in model.rules() {
        let trigger = model.trigger(r.trigger);
        if trigger.accepted_by.is_empty() {
            let machine = &model.machine(trigger.machine).name;
            findings.push(Finding::new(
                FindingDetail::InvalidFire { rule, trigger: r.trigger },
                format!("{}, but no {machine} transition accepts `{}`", describe::rule(model, rule), trigger.name),
            ));
        }
    }
    for (source, x) in model.externals() {
        for &t in &x.triggers {
            let trigger = model.trigger(t);
            if trigger.accepted_by.is_empty() {
                let machine = &model.machine(trigger.machine).name;
                findings.push(Finding::new(
                    FindingDetail::DeadExternalTrigger { source, trigger: t },
                    format!(
                        "{} can fire {}, but no {machine} transition accepts `{}`",
                        x.name,
                        describe::trigger(model, t),
                        trigger.name
                    ),
                ));
            }
        }
    }
    findings
}
