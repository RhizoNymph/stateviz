//! Name checks for content an op brings in.
//!
//! A parsed definition only ever holds valid names, but ops are built by the
//! app, so everything they introduce is checked with the parser's grammar
//! before it is written. References (unknown states, machines, events) are
//! left to the resolver, which runs on the edited definition.

use super::EditError;
use crate::definition::{
    ControllerDef, EventDef, ExternalDef, HandlerDef, MachineDef, RuleDef, StateDef, TargetSpec, TransitionDef,
    TriggerRef, ValueExpr,
};
use crate::error::{DiagnosticKind, LoadError};
use crate::parse::grammar::{is_valid_name, is_valid_path};
use crate::span::{SourceSpan, Spanned};

pub(super) fn name(name: &str) -> Result<(), EditError> {
    if is_valid_name(name) { Ok(()) } else { Err(EditError::InvalidName(name.to_owned())) }
}

/// A state reference: a local name or a dotted path.
pub(super) fn path(path: &str) -> Result<(), EditError> {
    if is_valid_path(path) { Ok(()) } else { Err(EditError::InvalidName(path.to_owned())) }
}

fn names<'a>(list: impl IntoIterator<Item = &'a Spanned<String>>) -> Result<(), EditError> {
    list.into_iter().try_for_each(|n| name(&n.value))
}

pub(super) fn machine(m: &MachineDef) -> Result<(), EditError> {
    name(&m.name.value)?;
    if let Some(initial) = &m.initial {
        path(&initial.value)?;
    }
    names(&m.fields)?;
    m.states.iter().try_for_each(state)?;
    m.transitions.iter().try_for_each(transition)
}

pub(super) fn state(s: &StateDef) -> Result<(), EditError> {
    name(&s.name.value)?;
    if let Some(initial) = &s.initial {
        name(&initial.value)?;
    }
    s.states.iter().try_for_each(state)
}

pub(super) fn transition(t: &TransitionDef) -> Result<(), EditError> {
    if t.from.is_empty() {
        // The file format has no way to write a transition without a source.
        return Err(EditError::Invalid(LoadError::single(
            DiagnosticKind::MissingKey { context: "transition".to_owned(), key: "from".to_owned() },
            SourceSpan::unknown(),
        )));
    }
    t.from.iter().try_for_each(|f| path(&f.value))?;
    path(&t.to.value)?;
    name(&t.on.value)?;
    names(&t.emits)
}

pub(super) fn event(e: &EventDef) -> Result<(), EditError> {
    name(&e.name.value)?;
    names(&e.payload)
}

pub(super) fn controller(c: &ControllerDef) -> Result<(), EditError> {
    name(&c.name.value)?;
    c.on.iter().try_for_each(handler)
}

pub(super) fn handler(h: &HandlerDef) -> Result<(), EditError> {
    name(&h.event.value)?;
    h.rules.iter().try_for_each(rule)
}

pub(super) fn rule(r: &RuleDef) -> Result<(), EditError> {
    trigger_ref(&r.fire.value)?;
    match &r.target {
        Some(target) => target_spec(&target.value),
        None => Ok(()),
    }
}

fn target_spec(t: &TargetSpec) -> Result<(), EditError> {
    name(&t.machine)?;
    t.clauses.iter().try_for_each(|clause| {
        name(&clause.field)?;
        match &clause.value {
            ValueExpr::EventField(field) => name(field),
            ValueExpr::Literal(_) => Ok(()),
        }
    })
}

pub(super) fn trigger_ref(t: &TriggerRef) -> Result<(), EditError> {
    name(&t.machine)?;
    name(&t.trigger)
}

pub(super) fn external(x: &ExternalDef) -> Result<(), EditError> {
    name(&x.name.value)?;
    x.triggers.iter().try_for_each(|t| trigger_ref(&t.value))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::definition::{StateKindDef, TargetMode};

    fn s(v: &str) -> Spanned<String> {
        Spanned::synthetic(v.to_owned())
    }

    #[test]
    fn names_and_paths() {
        assert!(name("ok_name-2").is_ok());
        assert_eq!(name("2bad"), Err(EditError::InvalidName("2bad".into())));
        assert_eq!(name("a.b"), Err(EditError::InvalidName("a.b".into())));
        assert!(path("a.b").is_ok());
        assert_eq!(path("a..b"), Err(EditError::InvalidName("a..b".into())));
    }

    #[test]
    fn nested_state_names_are_checked() {
        let child = StateDef {
            name: s("bad child"),
            kind: Spanned::synthetic(StateKindDef::Normal),
            initial: None,
            states: Vec::new(),
            span: SourceSpan::unknown(),
        };
        let parent = StateDef { name: s("parent"), states: vec![child], ..parent_template() };
        assert_eq!(state(&parent), Err(EditError::InvalidName("bad child".into())));
        let with_bad_initial = StateDef { initial: Some(s("a.b")), ..parent_template() };
        assert_eq!(state(&with_bad_initial), Err(EditError::InvalidName("a.b".into())));
    }

    fn parent_template() -> StateDef {
        StateDef {
            name: s("p"),
            kind: Spanned::synthetic(StateKindDef::Normal),
            initial: None,
            states: Vec::new(),
            span: SourceSpan::unknown(),
        }
    }

    #[test]
    fn selector_fields_are_checked() {
        let target = TargetSpec {
            mode: TargetMode::One,
            machine: "M".into(),
            clauses: vec![crate::definition::FieldClause {
                field: "id".into(),
                value: ValueExpr::EventField("bad field".into()),
            }],
        };
        assert_eq!(target_spec(&target), Err(EditError::InvalidName("bad field".into())));
        let literal = TargetSpec {
            clauses: vec![crate::definition::FieldClause {
                field: "id".into(),
                value: ValueExpr::Literal("any text at all".into()),
            }],
            ..target
        };
        assert!(target_spec(&literal).is_ok());
    }
}
