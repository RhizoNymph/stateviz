//! Evaluating expressions against string-valued fields.
//!
//! Semantics:
//!
//! - A test with a missing operand is false (`!=` and `not in` included),
//!   so `not (event.x == a)` holds when `x` is missing.
//! - Each test compares under the declared type of its field operand(s)
//!   ([`Types`]); a value that does not read as that type makes the test
//!   false. `number` compares as `f64` (total order); `bool` and `enum`
//!   support equality only; `string` compares text; `any` compares text,
//!   except that ordering compares numbers when both sides are numbers.

use std::cmp::Ordering;
use std::collections::BTreeMap;

use super::fields::{FieldDecl, FieldType};
use super::lexer::parse_number;
use super::types::{CmpOp, Expr, Literal, Operand};

/// Where operand values come from.
pub trait Scope {
    /// A field of the payload in scope (`event.x`).
    fn event(&self, field: &str) -> Option<&str>;
    /// A field of the instance in scope (`self.x`).
    fn self_field(&self, field: &str) -> Option<&str>;
}

/// A scope over two string maps, the simulator's representation.
#[derive(Clone, Copy, Debug)]
pub struct MapScope<'a> {
    pub event: &'a BTreeMap<String, String>,
    pub self_fields: &'a BTreeMap<String, String>,
}

impl Scope for MapScope<'_> {
    fn event(&self, field: &str) -> Option<&str> {
        self.event.get(field).map(String::as_str)
    }

    fn self_field(&self, field: &str) -> Option<&str> {
        self.self_fields.get(field).map(String::as_str)
    }
}

/// The declared types of `self.x` and `event.x` fields; anything not
/// listed is `any`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Types {
    self_fields: BTreeMap<String, FieldType>,
    event: BTreeMap<String, FieldType>,
}

static ANY: FieldType = FieldType::Any;

impl Types {
    /// Everything `any`.
    pub fn untyped() -> Self {
        Self::default()
    }

    /// Types from declarations: the instance's fields and the payload's.
    pub fn new(self_fields: &[FieldDecl], event: &[FieldDecl]) -> Self {
        let map = |decls: &[FieldDecl]| decls.iter().map(|d| (d.name.clone(), d.ty.clone())).collect();
        Self { self_fields: map(self_fields), event: map(event) }
    }

    pub fn self_type(&self, field: &str) -> &FieldType {
        self.self_fields.get(field).unwrap_or(&ANY)
    }

    pub fn event_type(&self, field: &str) -> &FieldType {
        self.event.get(field).unwrap_or(&ANY)
    }

    /// The declared type of a field operand; `None` for a literal.
    pub fn operand_type(&self, operand: &Operand) -> Option<&FieldType> {
        match operand {
            Operand::Event(f) => Some(self.event_type(f)),
            Operand::SelfField(f) => Some(self.self_type(f)),
            Operand::Lit(_) => None,
        }
    }

    /// The type a test between `a` and `b` compares under: a declared
    /// non-`any` type of either side (the left one first), else `any`.
    pub fn comparison_type(&self, a: &Operand, b: &Operand) -> &FieldType {
        [a, b].into_iter().filter_map(|o| self.operand_type(o)).find(|t| **t != FieldType::Any).unwrap_or(&ANY)
    }
}

/// The value an operand stands for: a field's value (absent when the scope
/// lacks it), or a literal's text.
pub fn value<'a>(operand: &'a Operand, scope: &'a dyn Scope) -> Option<&'a str> {
    match operand {
        Operand::Event(field) => scope.event(field),
        Operand::SelfField(field) => scope.self_field(field),
        Operand::Lit(lit) => Some(&lit.text),
    }
}

/// Whether `expr` holds in `scope`.
pub fn eval(expr: &Expr, scope: &dyn Scope, types: &Types) -> bool {
    match expr {
        Expr::Cmp { left, op, right } => match (value(left, scope), value(right, scope)) {
            (Some(l), Some(r)) => compare(l, *op, r, types.comparison_type(left, right)),
            _ => false,
        },
        Expr::In { operand, set, negated } => {
            let Some(v) = value(operand, scope) else {
                return false;
            };
            let ty = types.operand_type(operand).unwrap_or(&ANY);
            if !ty.accepts(v) {
                return false;
            }
            let found = set.iter().any(|lit| compare(v, CmpOp::Eq, &lit.text, ty));
            found != *negated
        }
        Expr::Not(inner) => !eval(inner, scope, types),
        Expr::And(items) => items.iter().all(|e| eval(e, scope, types)),
        Expr::Or(items) => items.iter().any(|e| eval(e, scope, types)),
    }
}

/// Compare two values under `ty`; false when either does not read as `ty`
/// or the operator does not apply to it.
pub fn compare(left: &str, op: CmpOp, right: &str, ty: &FieldType) -> bool {
    let ordering = match ty {
        FieldType::Number => match (parse_number(left), parse_number(right)) {
            (Some(l), Some(r)) => l.total_cmp(&r),
            _ => return false,
        },
        FieldType::Bool | FieldType::Enum(_) => {
            if op.is_ordering() || !ty.accepts(left) || !ty.accepts(right) {
                return false;
            }
            left.cmp(right)
        }
        FieldType::String => left.cmp(right),
        FieldType::Any => match (op.is_ordering(), parse_number(left), parse_number(right)) {
            (true, Some(l), Some(r)) => l.total_cmp(&r),
            _ => left.cmp(right),
        },
    };
    holds(op, ordering)
}

pub(crate) fn holds(op: CmpOp, ordering: Ordering) -> bool {
    match op {
        CmpOp::Eq => ordering == Ordering::Equal,
        CmpOp::Ne => ordering != Ordering::Equal,
        CmpOp::Lt => ordering == Ordering::Less,
        CmpOp::Le => ordering != Ordering::Greater,
        CmpOp::Gt => ordering == Ordering::Greater,
        CmpOp::Ge => ordering != Ordering::Less,
    }
}

/// Whether a literal is valid text for `ty` (numbers unquoted, bools
/// unquoted `true`/`false`, enum members).
pub fn literal_fits(lit: &Literal, ty: &FieldType) -> bool {
    match ty {
        FieldType::Any | FieldType::String => true,
        FieldType::Number => lit.as_number().is_some(),
        FieldType::Bool => lit.as_bool().is_some(),
        FieldType::Enum(members) => members.contains(&lit.text),
    }
}
