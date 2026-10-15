//! Checking scenario values against declared field types.
//!
//! Values stay strings at run time; a declared type says how expressions
//! read them, and a value that does not read as its type would make every
//! test on it false. Validation reports such values up front:
//!
//! - an instance field (declared or created instance, `AddInstance`) is
//!   checked against the machine's declaration of that field;
//! - a step payload entry (`Fire`) is checked against the fired machine's
//!   field of the same name (the payload overlays the fields in emitted
//!   events) and against the declared payload field of the same name of
//!   every event a transition accepting the trigger emits.
//!
//! Undeclared names and `any`/`string` fields accept anything.

use cascade_core::Model;
use cascade_core::expr::FieldType;
use cascade_core::ids::{MachineId, TriggerId};

use crate::error::ScenarioErrorKind;

fn invalid(field: &str, value: &str, ty: &FieldType) -> ScenarioErrorKind {
    ScenarioErrorKind::InvalidValue { field: field.to_owned(), value: value.to_owned(), expected: ty.expected() }
}

/// `None` when `value` fits `machine`'s declared type for `field`.
pub(crate) fn check_field_value(
    model: &Model,
    machine: MachineId,
    field: &str,
    value: &str,
) -> Option<ScenarioErrorKind> {
    let decl = model.machine(machine).fields.iter().find(|f| f.name == field)?;
    (!decl.ty.accepts(value)).then(|| invalid(field, value, &decl.ty))
}

/// `None` when a step payload entry fits every type declared for its key
/// (see the module docs).
pub(crate) fn check_payload_value(
    model: &Model,
    trigger: TriggerId,
    key: &str,
    value: &str,
) -> Option<ScenarioErrorKind> {
    let t = model.trigger(trigger);
    if let Some(problem) = check_field_value(model, t.machine, key, value) {
        return Some(problem);
    }
    t.accepted_by
        .iter()
        .flat_map(|&transition| model.transition(transition).emits.iter())
        .filter_map(|emit| model.event(emit.event).payload.iter().find(|f| f.name == key))
        .find(|decl| !decl.ty.accepts(value))
        .map(|decl| invalid(key, value, &decl.ty))
}
