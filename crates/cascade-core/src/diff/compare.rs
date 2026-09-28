//! The attributes [`diff_models`](super::diff_models) compares, per element,
//! expressed in names and paths so they are comparable across models.

use std::collections::BTreeMap;

use crate::color::PaletteColor;
use crate::ids::TriggerId;
use crate::key::{ElementKey, ElementRef};
use crate::model::{Model, StateKind, Target};

/// What makes two elements with the same key equal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Attributes {
    Machine {
        color: Option<PaletteColor>,
        domain: Option<String>,
        initial: String,
        fields: Vec<String>,
    },
    State(StateShape),
    Transition {
        guard: Option<String>,
        emits: Vec<String>,
        bounded: bool,
    },
    Event {
        payload: Vec<String>,
        declared: bool,
    },
    Rule {
        fire: String,
        target: Target,
        condition: Option<String>,
        bounded: bool,
    },
    External {
        triggers: Vec<String>,
    },
    /// Triggers, controllers and handlers: nothing beyond their key.
    KeyOnly,
}

/// A state's kind, with a compound state's initial child by path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum StateShape {
    Atomic,
    Compound { initial: String },
    Final,
    History { deep: bool },
}

/// Every element of `model` with its attributes.
pub(super) fn attributes_by_key(model: &Model) -> BTreeMap<ElementKey, Attributes> {
    model.all_elements().into_iter().map(|element| (model.key_of(element), attributes(model, element))).collect()
}

fn attributes(model: &Model, element: ElementRef) -> Attributes {
    match element {
        ElementRef::Machine(id) => {
            let m = model.machine(id);
            Attributes::Machine {
                color: m.color,
                domain: m.domain.clone(),
                initial: model.state(m.initial).path.clone(),
                fields: m.fields.clone(),
            }
        }
        ElementRef::State(id) => Attributes::State(match &model.state(id).kind {
            StateKind::Atomic => StateShape::Atomic,
            StateKind::Compound { initial, .. } => StateShape::Compound { initial: model.state(*initial).path.clone() },
            StateKind::Final => StateShape::Final,
            StateKind::History { deep } => StateShape::History { deep: *deep },
        }),
        ElementRef::Transition(id) => {
            let t = model.transition(id);
            Attributes::Transition {
                guard: t.guard.clone(),
                emits: t.emits.iter().map(|&e| model.event(e).name.clone()).collect(),
                bounded: t.bounded,
            }
        }
        ElementRef::Event(id) => {
            let e = model.event(id);
            Attributes::Event { payload: e.payload.clone(), declared: e.declared }
        }
        ElementRef::Rule(id) => {
            let r = model.rule(id);
            Attributes::Rule {
                fire: trigger_name(model, r.trigger),
                target: r.target.clone(),
                condition: r.condition.clone(),
                bounded: r.bounded,
            }
        }
        ElementRef::External(id) => {
            let mut triggers: Vec<String> =
                model.external(id).triggers.iter().map(|&t| trigger_name(model, t)).collect();
            triggers.sort();
            Attributes::External { triggers }
        }
        ElementRef::Trigger(_) | ElementRef::Controller(_) | ElementRef::Handler(_) => Attributes::KeyOnly,
    }
}

/// `Machine.trigger`.
fn trigger_name(model: &Model, trigger: TriggerId) -> String {
    let t = model.trigger(trigger);
    format!("{}.{}", model.machine(t.machine).name, t.name)
}
