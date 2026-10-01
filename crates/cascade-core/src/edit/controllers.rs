//! Controllers, their handlers (one per subscribed event) and rules.
//! Handlers live inside their controller and rules inside their handler, so
//! removing either takes its contents with it and the inverse re-inserts
//! the whole element.

use super::engine::Effect;
use super::keys;
use super::lookup::{self, handler_name, name_taken};
use super::{EditError, EditOp, Index, validate};
use crate::definition::{ControllerDef, Definition, HandlerDef, RuleDef};

pub(super) fn add(def: &mut Definition, controller: &ControllerDef, index: Index) -> Result<Effect, EditError> {
    validate::controller(controller)?;
    let name = &controller.name.value;
    if def.controllers.iter().any(|c| c.name.value == *name) {
        return Err(name_taken("controller", name.clone()));
    }
    lookup::insert(&mut def.controllers, controller.clone(), index, "controller")?;
    Ok(Effect::new(EditOp::RemoveController { controller: name.clone() }, keys::controller_keys(controller)))
}

pub(super) fn remove(def: &mut Definition, controller: &str) -> Result<Effect, EditError> {
    let at = lookup::controller_index(def, controller)?;
    let removed = def.controllers.remove(at);
    let touched = keys::controller_keys(&removed);
    Ok(Effect::new(EditOp::AddController { controller: removed, index: Some(at) }, touched))
}

pub(super) fn rename(def: &mut Definition, from: &str, to: &str) -> Result<Effect, EditError> {
    validate::name(to)?;
    let at = lookup::controller_index(def, from)?;
    let inverse = EditOp::RenameController { from: to.to_owned(), to: from.to_owned() };
    if from != to {
        if def.controllers.iter().any(|c| c.name.value == to) {
            return Err(name_taken("controller", to));
        }
        lookup::set_value(&mut def.controllers[at].name, to.to_owned());
    }
    Ok(Effect::new(inverse, keys::controller_keys(&def.controllers[at])))
}

pub(super) fn add_handler(
    def: &mut Definition,
    controller: &str,
    handler: &HandlerDef,
    index: Index,
) -> Result<Effect, EditError> {
    validate::handler(handler)?;
    let c = lookup::controller_mut(def, controller)?;
    let event = &handler.event.value;
    if c.on.iter().any(|h| h.event.value == *event) {
        return Err(name_taken("handler", handler_name(controller, event)));
    }
    lookup::insert(&mut c.on, handler.clone(), index, "handler")?;
    let inverse = EditOp::RemoveHandler { controller: controller.to_owned(), event: event.clone() };
    Ok(Effect::new(inverse, keys::handler_keys(controller, handler)))
}

pub(super) fn remove_handler(def: &mut Definition, controller: &str, event: &str) -> Result<Effect, EditError> {
    let c = lookup::controller_mut(def, controller)?;
    let at = lookup::handler_index(c, event)?;
    let removed = c.on.remove(at);
    let touched = keys::handler_keys(controller, &removed);
    let inverse = EditOp::AddHandler { controller: controller.to_owned(), handler: removed, index: Some(at) };
    Ok(Effect::new(inverse, touched))
}

pub(super) fn add_rule(
    def: &mut Definition,
    controller: &str,
    event: &str,
    rule: &RuleDef,
    index: Index,
) -> Result<Effect, EditError> {
    validate::rule(rule)?;
    let h = lookup::handler_mut(def, controller, event)?;
    let at = lookup::insert(&mut h.rules, rule.clone(), index, "rule")?;
    let inverse = EditOp::RemoveRule { controller: controller.to_owned(), event: event.to_owned(), index: at };
    Ok(Effect::new(inverse, vec![keys::rule_key(controller, event, at)]))
}

pub(super) fn update_rule(
    def: &mut Definition,
    controller: &str,
    event: &str,
    index: usize,
    rule: &RuleDef,
) -> Result<Effect, EditError> {
    let h = lookup::handler_mut(def, controller, event)?;
    let at = lookup::existing(index, h.rules.len(), "rule")?;
    validate::rule(rule)?;
    let old = std::mem::replace(&mut h.rules[at], rule.clone());
    let inverse =
        EditOp::UpdateRule { controller: controller.to_owned(), event: event.to_owned(), index: at, rule: old };
    Ok(Effect::new(inverse, vec![keys::rule_key(controller, event, at)]))
}

pub(super) fn remove_rule(
    def: &mut Definition,
    controller: &str,
    event: &str,
    index: usize,
) -> Result<Effect, EditError> {
    let h = lookup::handler_mut(def, controller, event)?;
    let at = lookup::existing(index, h.rules.len(), "rule")?;
    let old = h.rules.remove(at);
    let inverse =
        EditOp::AddRule { controller: controller.to_owned(), event: event.to_owned(), rule: old, index: Some(at) };
    Ok(Effect::new(inverse, vec![keys::rule_key(controller, event, at)]))
}
