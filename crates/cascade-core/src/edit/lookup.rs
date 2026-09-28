//! Finding elements by name, checking positions, and writing values while
//! keeping the spans of values that stay where they were.

use super::{EditError, Index};
use crate::definition::{ControllerDef, Definition, HandlerDef, MachineDef};
use crate::span::{SourceSpan, Spanned};

pub(super) fn not_found(what: &'static str, name: impl Into<String>) -> EditError {
    EditError::NotFound { what, name: name.into() }
}

pub(super) fn name_taken(what: &'static str, name: impl Into<String>) -> EditError {
    EditError::NameTaken { what, name: name.into() }
}

pub(super) fn machine_index(def: &Definition, name: &str) -> Result<usize, EditError> {
    def.machines.iter().position(|m| m.name.value == name).ok_or_else(|| not_found("machine", name))
}

pub(super) fn machine_mut<'a>(def: &'a mut Definition, name: &str) -> Result<&'a mut MachineDef, EditError> {
    let index = machine_index(def, name)?;
    def.machines.get_mut(index).ok_or_else(|| not_found("machine", name))
}

pub(super) fn controller_index(def: &Definition, name: &str) -> Result<usize, EditError> {
    def.controllers.iter().position(|c| c.name.value == name).ok_or_else(|| not_found("controller", name))
}

pub(super) fn controller_mut<'a>(def: &'a mut Definition, name: &str) -> Result<&'a mut ControllerDef, EditError> {
    let index = controller_index(def, name)?;
    def.controllers.get_mut(index).ok_or_else(|| not_found("controller", name))
}

pub(super) fn handler_name(controller: &str, event: &str) -> String {
    format!("{controller}/{event}")
}

pub(super) fn handler_index(controller: &ControllerDef, event: &str) -> Result<usize, EditError> {
    controller
        .on
        .iter()
        .position(|h| h.event.value == event)
        .ok_or_else(|| not_found("handler", handler_name(&controller.name.value, event)))
}

pub(super) fn handler_mut<'a>(
    def: &'a mut Definition,
    controller: &str,
    event: &str,
) -> Result<&'a mut HandlerDef, EditError> {
    let c = controller_mut(def, controller)?;
    let index = handler_index(c, event)?;
    c.on.get_mut(index).ok_or_else(|| not_found("handler", handler_name(controller, event)))
}

pub(super) fn external_index(def: &Definition, name: &str) -> Result<usize, EditError> {
    def.external.iter().position(|x| x.name.value == name).ok_or_else(|| not_found("external source", name))
}

/// Where an insertion at `index` lands in a list of length `len`: `None`
/// appends, `Some(i)` needs `i <= len`.
pub(super) fn insertion_point(index: Index, len: usize, what: &'static str) -> Result<usize, EditError> {
    match index {
        None => Ok(len),
        Some(i) if i <= len => Ok(i),
        Some(i) => Err(EditError::IndexOutOfRange { what, index: i, len }),
    }
}

/// An existing position: `index < len`.
pub(super) fn existing(index: usize, len: usize, what: &'static str) -> Result<usize, EditError> {
    if index < len { Ok(index) } else { Err(EditError::IndexOutOfRange { what, index, len }) }
}

/// Insert at `index` (see [`insertion_point`]) and return the position.
pub(super) fn insert<T>(list: &mut Vec<T>, item: T, index: Index, what: &'static str) -> Result<usize, EditError> {
    let at = insertion_point(index, list.len(), what)?;
    list.insert(at, item);
    Ok(at)
}

/// Replace a value, keeping its span: it is still written at the same place.
pub(super) fn set_value<T>(slot: &mut Spanned<T>, value: T) {
    slot.value = value;
}

/// Set or clear an optional value, keeping the span of a value that is
/// replaced; a value that did not exist gets an unknown span.
pub(super) fn set_optional<T>(slot: &mut Option<Spanned<T>>, value: Option<T>) {
    *slot = match (slot.take(), value) {
        (Some(old), Some(value)) => Some(Spanned::new(value, old.span)),
        (None, Some(value)) => Some(Spanned::new(value, SourceSpan::unknown())),
        (_, None) => None,
    };
}

pub(super) fn values<T: Clone>(list: &[Spanned<T>]) -> Vec<T> {
    list.iter().map(|v| v.value.clone()).collect()
}

pub(super) fn synthetic_list<T>(values: impl IntoIterator<Item = T>) -> Vec<Spanned<T>> {
    values.into_iter().map(Spanned::synthetic).collect()
}
