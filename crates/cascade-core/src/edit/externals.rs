//! External sources of triggers.

use super::engine::Effect;
use super::keys::external_key;
use super::lookup::{self, name_taken};
use super::{EditError, EditOp, Index, validate};
use crate::definition::{Definition, ExternalDef, TriggerRef};

pub(super) fn add(def: &mut Definition, external: &ExternalDef, index: Index) -> Result<Effect, EditError> {
    validate::external(external)?;
    let name = &external.name.value;
    if def.external.iter().any(|x| x.name.value == *name) {
        return Err(name_taken("external source", name.clone()));
    }
    lookup::insert(&mut def.external, external.clone(), index, "external source")?;
    Ok(Effect::new(EditOp::RemoveExternal { external: name.clone() }, vec![external_key(name)]))
}

pub(super) fn remove(def: &mut Definition, external: &str) -> Result<Effect, EditError> {
    let at = lookup::external_index(def, external)?;
    let removed = def.external.remove(at);
    Ok(Effect::new(EditOp::AddExternal { external: removed, index: Some(at) }, vec![external_key(external)]))
}

pub(super) fn rename(def: &mut Definition, from: &str, to: &str) -> Result<Effect, EditError> {
    validate::name(to)?;
    let at = lookup::external_index(def, from)?;
    if from != to {
        if def.external.iter().any(|x| x.name.value == to) {
            return Err(name_taken("external source", to));
        }
        lookup::set_value(&mut def.external[at].name, to.to_owned());
    }
    Ok(Effect::new(EditOp::RenameExternal { from: to.to_owned(), to: from.to_owned() }, vec![external_key(to)]))
}

pub(super) fn set_triggers(def: &mut Definition, external: &str, triggers: &[TriggerRef]) -> Result<Effect, EditError> {
    triggers.iter().try_for_each(validate::trigger_ref)?;
    let at = lookup::external_index(def, external)?;
    let x = &mut def.external[at];
    let old = lookup::values(&x.triggers);
    x.triggers = lookup::synthetic_list(triggers.iter().cloned());
    Ok(Effect::new(
        EditOp::SetExternalTriggers { external: external.to_owned(), triggers: old },
        vec![external_key(external)],
    ))
}
