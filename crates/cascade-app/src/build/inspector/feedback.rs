//! Live feedback while a field is typed: the grammar first, then what the
//! resolver says about the edited definition (unknown fields, type
//! mismatches, enum members, `self` where it is not allowed, …).
//!
//! Pure. The edit is planned exactly as a commit would plan it and applied
//! to a copy of the definition; nothing is written.

use cascade_core::edit::{self, EditError};
use cascade_core::{Definition, ElementKey};

use super::{FieldId, FieldValue, field_op, validate};

/// Check `text` typed into `field` of `key`: `Err` holds the message to
/// show under the field. An unchanged value is fine.
pub fn check(definition: &Definition, key: &ElementKey, field: FieldId, text: &str) -> Result<(), String> {
    validate(field, text)?;
    let planned = match field_op(definition, key, field, &FieldValue::Text(text.to_owned())) {
        Ok(Some(planned)) => planned,
        Ok(None) => return Ok(()),
        Err(error) => return Err(error.to_string()),
    };
    match edit::apply(definition, &planned.op) {
        Ok(_) => Ok(()),
        Err(EditError::Invalid(load)) => {
            let mut messages: Vec<String> = Vec::new();
            for diagnostic in &load.diagnostics {
                let message = diagnostic.kind.to_string();
                if !messages.contains(&message) {
                    messages.push(message);
                }
            }
            Err(messages.join("; "))
        }
        Err(other) => Err(other.to_string()),
    }
}
