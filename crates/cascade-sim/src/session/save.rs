//! A session's timeline as a scenario.
//!
//! | Action | Scenario |
//! | --- | --- |
//! | leading `AddInstance`s | `instances:` |
//! | later `AddInstance` | `- { create: … }` |
//! | `RemoveInstance` | `- { remove: … }` |
//! | `Fire` into an empty queue | `- { source: …, fire: …, target: … }` |
//! | `Fire` while items are queued | the same with `timing: immediate` |
//! | `Step { choice: None }` / `Some(n)` | `- step` / `- { step: n }` |
//! | `RunUntilQuiet` | `- run` |
//! | items still queued at the end | `end: pause` |
//!
//! Replaying the scenario ([`PlaySession::from_scenario`]) gives back the
//! same actions and trace: an after-quiescence fire into an empty queue
//! adds no run, and the final drain has nothing to do unless the scenario
//! says `end: pause`.

use cascade_core::span::{SourceSpan, Spanned};

use super::{PlayAction, PlaySession};
use crate::error::{ScenarioErrorKind, SimError};
use crate::scenario::{Directive, InstanceDecl, Payload, Scenario, ScenarioEnd, Step, StepTiming, ValueMap};

pub(super) fn to_scenario(session: &PlaySession, name: &str) -> Result<Scenario, SimError> {
    let applied = &session.timeline.actions[..session.timeline.position];
    let mut scenario = Scenario {
        name: name.to_owned(),
        instances: Vec::new(),
        steps: Vec::new(),
        trailing: Vec::new(),
        end: if session.core.queue().is_empty() { ScenarioEnd::Drain } else { ScenarioEnd::Pause },
    };

    let leading = applied.iter().take_while(|a| matches!(a, PlayAction::AddInstance { .. })).count();
    for action in &applied[..leading] {
        scenario.instances.push(declaration(action)?);
    }

    let mut directives = Vec::new();
    for (i, action) in applied.iter().enumerate().skip(leading) {
        let span = SourceSpan::unknown();
        match action {
            PlayAction::AddInstance { .. } => directives.push(Directive::Create(declaration(action)?)),
            PlayAction::RemoveInstance { name } => {
                directives.push(Directive::Remove { name: Spanned::synthetic(name.clone()), span });
            }
            PlayAction::Step { choice } => directives.push(Directive::Deliver { choice: *choice, span }),
            PlayAction::RunUntilQuiet => directives.push(Directive::Run { span }),
            PlayAction::Fire { source, trigger, target, payload } => {
                let quiet = session.quiet_before.get(i).copied().unwrap_or(true);
                scenario.steps.push(Step {
                    source: Spanned::synthetic(source.clone()),
                    fire: Spanned::synthetic(trigger.clone()),
                    target: Some(Spanned::synthetic(target.clone())),
                    payload: value_map(payload),
                    timing: if quiet { StepTiming::AfterQuiescence } else { StepTiming::Immediate },
                    span,
                    before: std::mem::take(&mut directives),
                });
            }
        }
    }
    scenario.trailing = directives;
    Ok(scenario)
}

/// An `AddInstance` as recorded (always named) → an instance declaration.
fn declaration(action: &PlayAction) -> Result<InstanceDecl, SimError> {
    let PlayAction::AddInstance { name: Some(name), machine, fields, state } = action else {
        // The timeline records every added instance with the name it got.
        let kind = ScenarioErrorKind::MissingKey { context: "added instance".to_owned(), key: "name".to_owned() };
        return Err(SimError::Action(kind));
    };
    Ok(InstanceDecl {
        name: Spanned::synthetic(name.clone()),
        machine: Spanned::synthetic(machine.clone()),
        fields: value_map(fields),
        state: state.as_ref().map(|s| Spanned::synthetic(s.clone())),
        span: SourceSpan::unknown(),
    })
}

fn value_map(payload: &Payload) -> ValueMap {
    payload.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
}
