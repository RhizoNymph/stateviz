//! Type checking expressions and assignments against declared fields and
//! payloads.
//!
//! - `self.x` must be a declared field when the machine in scope declares
//!   fields, and is an error where there is no instance in scope (rule
//!   conditions, spawn assignments).
//! - `event.x` must be a declared payload field when the event in scope is
//!   known and declares a payload. Where the event is open (a command
//!   trigger's guard: any firing event or external step may supply it),
//!   `event.x` is `any`.
//! - Both sides of a test must have compatible types (`any` is compatible
//!   with everything); enum literals must be members; ordering needs numbers
//!   (or `any`); `in` lists follow the equality rules.
//! - An assignment's value must fit the assigned field's type.

use crate::error::DiagnosticKind;
use crate::expr::{Assignment, CmpOp, Expr, FieldDecl, FieldType, Literal, Operand, literal_fits};
use crate::model::{Event, Machine};

/// What `self` means where an expression appears.
#[derive(Clone, Copy, Debug)]
pub(super) enum SelfScope<'a> {
    /// The instance of this machine.
    Instance(&'a Machine),
    /// No instance is in scope; the string names the place, for messages
    /// ("a rule's `when`").
    None(&'static str),
}

/// What `event` means where an expression appears.
#[derive(Clone, Copy, Debug)]
pub(super) enum EventScope<'a> {
    /// This event's payload.
    Known(&'a Event),
    /// Not known statically: every `event.x` is `any`.
    Open,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Site<'a> {
    pub self_scope: SelfScope<'a>,
    pub event: EventScope<'a>,
}

/// The type an operand reads as, or the literal itself.
enum Side<'a> {
    Field(FieldType),
    Lit(&'a Literal),
}

fn describe_side(side: &Side<'_>) -> String {
    match side {
        Side::Field(ty) => format!("a field of type `{ty}`"),
        Side::Lit(lit) if lit.as_number().is_some() => format!("the number `{lit}`"),
        Side::Lit(lit) if lit.as_bool().is_some() => format!("the bool `{lit}`"),
        Side::Lit(lit) => format!("the text `{lit}`"),
    }
}

impl<'a> Site<'a> {
    /// Diagnostics for `expr`; `text` is how the containing string reads in
    /// messages.
    pub fn check_expr(&self, expr: &Expr, text: &str) -> Vec<DiagnosticKind> {
        let mut out = Vec::new();
        self.walk(expr, text, &mut out);
        out
    }

    fn walk(&self, expr: &Expr, text: &str, out: &mut Vec<DiagnosticKind>) {
        match expr {
            Expr::Cmp { left, op, right } => {
                let (Some(l), Some(r)) = (self.side(left, text, out), self.side(right, text, out)) else {
                    return;
                };
                out.extend(compare(&l, *op, &r, text));
            }
            Expr::In { operand, set, .. } => {
                let Some(side) = self.side(operand, text, out) else {
                    return;
                };
                for lit in set {
                    out.extend(compare(&side, CmpOp::Eq, &Side::Lit(lit), text));
                }
            }
            Expr::Not(inner) => self.walk(inner, text, out),
            Expr::And(items) | Expr::Or(items) => items.iter().for_each(|e| self.walk(e, text, out)),
        }
    }

    /// The operand's side, reporting undeclared fields and `self` where not
    /// allowed (`None` then).
    fn side<'o>(&self, operand: &'o Operand, text: &str, out: &mut Vec<DiagnosticKind>) -> Option<Side<'o>> {
        match operand {
            Operand::Lit(lit) => Some(Side::Lit(lit)),
            Operand::SelfField(field) => self.self_type(field, text, out).map(Side::Field),
            Operand::Event(field) => self.event_type(field, out).map(Side::Field),
        }
    }

    fn self_type(&self, field: &str, text: &str, out: &mut Vec<DiagnosticKind>) -> Option<FieldType> {
        match self.self_scope {
            SelfScope::None(context) => {
                out.push(DiagnosticKind::SelfNotAllowed { text: text.to_owned(), context: context.to_owned() });
                None
            }
            SelfScope::Instance(machine) => declared(&machine.fields, field).or_else(|| {
                if machine.fields.is_empty() {
                    Some(FieldType::Any)
                } else {
                    out.push(unknown_field(machine, field));
                    None
                }
            }),
        }
    }

    fn event_type(&self, field: &str, out: &mut Vec<DiagnosticKind>) -> Option<FieldType> {
        match self.event {
            EventScope::Open => Some(FieldType::Any),
            EventScope::Known(event) => declared(&event.payload, field).or_else(|| {
                if event.declared && !event.payload.is_empty() {
                    out.push(unknown_payload_field(event, field));
                    None
                } else {
                    Some(FieldType::Any)
                }
            }),
        }
    }

    /// Diagnostics for an assignment of `value` to a field of type `target`
    /// (`None` when the field is not declared: anything fits).
    pub fn check_value(&self, target: Option<&FieldType>, assignment: &Assignment) -> Vec<DiagnosticKind> {
        let text = assignment.to_string();
        let mut out = Vec::new();
        let Some(side) = self.side(&assignment.value, &text, &mut out) else {
            return out;
        };
        let Some(target) = target else {
            return out;
        };
        match side {
            Side::Lit(lit) if literal_fits(lit, target) => {}
            Side::Lit(lit) => out.push(match target {
                FieldType::Enum(_) => DiagnosticKind::NotAMember { value: lit.text.clone(), ty: target.to_string() },
                _ => DiagnosticKind::TypeMismatch {
                    text,
                    left: format!("a field of type `{target}`"),
                    right: describe_side(&Side::Lit(lit)),
                },
            }),
            Side::Field(ty) => {
                if !fields_compatible(target, &ty) {
                    out.push(DiagnosticKind::TypeMismatch {
                        text,
                        left: format!("a field of type `{target}`"),
                        right: describe_side(&Side::Field(ty)),
                    });
                }
            }
        }
        out
    }
}

fn declared(fields: &[FieldDecl], name: &str) -> Option<FieldType> {
    fields.iter().find(|f| f.name == name).map(|f| f.ty.clone())
}

pub(super) fn unknown_field(machine: &Machine, field: &str) -> DiagnosticKind {
    DiagnosticKind::UnknownField {
        machine: machine.name.clone(),
        field: field.to_owned(),
        declared: machine.fields.iter().map(|f| f.name.clone()).collect(),
    }
}

pub(super) fn unknown_payload_field(event: &Event, field: &str) -> DiagnosticKind {
    DiagnosticKind::UnknownPayloadField {
        event: event.name.clone(),
        field: field.to_owned(),
        declared: event.payload.iter().map(|f| f.name.clone()).collect(),
    }
}

fn fields_compatible(a: &FieldType, b: &FieldType) -> bool {
    *a == FieldType::Any || *b == FieldType::Any || a == b
}

fn orderable(side: &Side<'_>) -> bool {
    match side {
        Side::Field(ty) => matches!(ty, FieldType::Number | FieldType::Any),
        Side::Lit(_) => true,
    }
}

fn compare(left: &Side<'_>, op: CmpOp, right: &Side<'_>, text: &str) -> Option<DiagnosticKind> {
    let mismatch = || DiagnosticKind::TypeMismatch {
        text: text.to_owned(),
        left: describe_side(left),
        right: describe_side(right),
    };
    if op.is_ordering() {
        for side in [left, right] {
            if !orderable(side) {
                return Some(DiagnosticKind::TypeMismatch {
                    text: text.to_owned(),
                    left: describe_side(side),
                    right: format!("the ordering operator `{op}`"),
                });
            }
        }
    }
    match (left, right) {
        (Side::Lit(_), Side::Lit(_)) => None,
        (Side::Field(a), Side::Field(b)) => (!fields_compatible(a, b)).then(mismatch),
        (Side::Field(ty), Side::Lit(lit)) | (Side::Lit(lit), Side::Field(ty)) => match ty {
            FieldType::Any | FieldType::String => None,
            FieldType::Enum(_) if !literal_fits(lit, ty) => {
                Some(DiagnosticKind::NotAMember { value: lit.text.clone(), ty: ty.to_string() })
            }
            _ if literal_fits(lit, ty) => None,
            _ => Some(mismatch()),
        },
    }
}
