//! Name checks for content an op brings in.
//!
//! A parsed definition only ever holds valid names, but ops are built by the
//! app, so everything they introduce is checked with the parser's grammar
//! before it is written. References (unknown states, machines, events) are
//! left to the resolver, which runs on the edited definition.

use super::EditError;
use crate::definition::{
    ControllerDef, EventDef, ExternalDef, FieldDef, HandlerDef, MachineDef, RuleDef, StateDef, TargetSpec,
    TransitionDef, TriggerRef, TriggerSpec,
};
use crate::error::{DiagnosticKind, LoadError};
use crate::expr::{Assignment, Condition, Expr, FieldType, Guard, Operand};
use crate::parse::grammar::{is_valid_name, is_valid_path};
use crate::span::SourceSpan;

pub(super) fn name(name: &str) -> Result<(), EditError> {
    if is_valid_name(name) { Ok(()) } else { Err(EditError::InvalidName(name.to_owned())) }
}

/// A state reference: a local name or a dotted path.
pub(super) fn path(path: &str) -> Result<(), EditError> {
    if is_valid_path(path) { Ok(()) } else { Err(EditError::InvalidName(path.to_owned())) }
}

/// Something that would not read back from the file as written.
fn unwritable(text: String, message: &str) -> EditError {
    EditError::Invalid(LoadError::single(
        DiagnosticKind::InvalidExpression { text, message: message.to_owned() },
        SourceSpan::unknown(),
    ))
}

/// Field names, and enum types that can be written: at least one member,
/// none repeated.
pub(super) fn fields(list: &[FieldDef]) -> Result<(), EditError> {
    for field in list {
        name(&field.name.value)?;
        if let Some(ty) = &field.ty
            && let FieldType::Enum(members) = &ty.value
        {
            let repeated = members.iter().enumerate().any(|(i, m)| members[..i].contains(m));
            if members.is_empty() || repeated {
                return Err(EditError::Invalid(LoadError::single(
                    DiagnosticKind::InvalidFieldType {
                        text: ty.value.to_string(),
                        message: "an enum needs at least one member and no repeats".to_owned(),
                    },
                    SourceSpan::unknown(),
                )));
            }
        }
    }
    Ok(())
}

fn operand(o: &Operand) -> Result<(), EditError> {
    match o {
        Operand::Event(field) | Operand::SelfField(field) => name(field),
        Operand::Lit(_) => Ok(()),
    }
}

/// Field names are names, and `and`/`or` hold at least two items (so the
/// canonical text reads back as the same expression).
pub(super) fn expr(e: &Expr) -> Result<(), EditError> {
    match e {
        Expr::Cmp { left, right, .. } => {
            operand(left)?;
            operand(right)
        }
        Expr::In { operand: o, set, .. } => {
            if set.is_empty() {
                return Err(unwritable(e.to_string(), "an `in` list needs at least one literal"));
            }
            operand(o)
        }
        Expr::Not(inner) => expr(inner),
        Expr::And(items) | Expr::Or(items) => {
            if items.len() < 2 {
                return Err(unwritable(e.to_string(), "`and`/`or` need at least two items"));
            }
            items.iter().try_for_each(expr)
        }
    }
}

fn label(label: &str) -> Result<(), EditError> {
    if label.is_empty() || label.trim() != label {
        Err(unwritable(format!("?{label}"), "a choice label must be non-empty, without surrounding spaces"))
    } else {
        Ok(())
    }
}

fn assignment(a: &Assignment) -> Result<(), EditError> {
    name(&a.field)?;
    operand(&a.value)
}

pub(super) fn machine(m: &MachineDef) -> Result<(), EditError> {
    name(&m.name.value)?;
    if let Some(initial) = &m.initial {
        path(&initial.value)?;
    }
    fields(&m.fields)?;
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
    match &t.trigger {
        TriggerSpec::Command(n) => name(&n.value)?,
        TriggerSpec::Event { event, filter } => {
            name(&event.value)?;
            if let Some(filter) = filter {
                expr(&filter.value)?;
            }
        }
        TriggerSpec::After(_) => {}
    }
    match t.guard.as_ref().map(|g| &g.value) {
        Some(Guard::Expr(e)) => expr(e)?,
        Some(Guard::Choice(l)) => label(l)?,
        Some(Guard::Else) | None => {}
    }
    t.sets.iter().try_for_each(|a| assignment(&a.value))?;
    for emit in &t.emits {
        name(&emit.event.value)?;
        emit.with.iter().try_for_each(|a| assignment(&a.value))?;
    }
    Ok(())
}

pub(super) fn event(e: &EventDef) -> Result<(), EditError> {
    name(&e.name.value)?;
    fields(&e.payload)
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
    match r.when.as_ref().map(|w| &w.value) {
        Some(Condition::Expr(e)) => expr(e)?,
        Some(Condition::Choice(l)) => label(l)?,
        None => {}
    }
    match &r.target {
        Some(target) => target_spec(&target.value),
        None => Ok(()),
    }
}

fn target_spec(t: &TargetSpec) -> Result<(), EditError> {
    name(t.machine())?;
    match t {
        TargetSpec::One { filter, .. } | TargetSpec::All { filter, .. } => filter.as_ref().map_or(Ok(()), expr),
        TargetSpec::Spawn { assignments, .. } => assignments.iter().try_for_each(assignment),
    }
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
    use crate::definition::StateKindDef;
    use crate::span::Spanned;

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
        use crate::expr::{CmpOp, Literal};
        let bad = Expr::Cmp {
            left: Operand::SelfField("id".into()),
            op: CmpOp::Eq,
            right: Operand::Event("bad field".into()),
        };
        let target = TargetSpec::One { machine: "M".into(), filter: Some(bad) };
        assert_eq!(target_spec(&target), Err(EditError::InvalidName("bad field".into())));
        let literal = TargetSpec::One {
            machine: "M".into(),
            filter: Some(Expr::Cmp {
                left: Operand::SelfField("id".into()),
                op: CmpOp::Eq,
                right: Operand::Lit(Literal::new("any text at all")),
            }),
        };
        assert!(target_spec(&literal).is_ok());
        let spawn = TargetSpec::Spawn {
            machine: "M".into(),
            assignments: vec![Assignment::new("bad name", Operand::Lit(Literal::new("x")))],
        };
        assert_eq!(target_spec(&spawn), Err(EditError::InvalidName("bad name".into())));
    }

    #[test]
    fn expressions_must_read_back() {
        use crate::expr::{CmpOp, Literal};
        let test =
            Expr::Cmp { left: Operand::SelfField("a".into()), op: CmpOp::Eq, right: Operand::Lit(Literal::new("1")) };
        assert!(expr(&test).is_ok());
        assert!(matches!(expr(&Expr::And(vec![test.clone()])), Err(EditError::Invalid(_))));
        assert!(matches!(
            expr(&Expr::In { operand: Operand::SelfField("a".into()), set: vec![], negated: false }),
            Err(EditError::Invalid(_))
        ));
        assert!(label("ok label").is_ok());
        assert!(matches!(label(" padded"), Err(EditError::Invalid(_))));
        assert!(matches!(label(""), Err(EditError::Invalid(_))));
        assert!(matches!(
            fields(&[FieldDef::typed("t", FieldType::Enum(vec!["a".into(), "a".into()]))]),
            Err(EditError::Invalid(_))
        ));
        assert!(fields(&[FieldDef::typed("t", FieldType::Enum(vec!["a".into()]))]).is_ok());
    }
}
