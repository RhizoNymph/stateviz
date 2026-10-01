//! Machine ops and the system name.

use cascade_core::definition::{MachineDef, TargetSpec};
use cascade_core::parse::grammar::{parse_target, parse_trigger_ref};

use super::{Child, add_child, check_new_name, controllers, definition, insert_at, nav, step};
use crate::patch::block;
use crate::patch::collection::{
    Value, child_col, remove_child, replace_scalar, replace_value, set_key, set_names, set_scalar_key,
};
use crate::patch::doc::{Doc, Node};
use crate::patch::error::{Result, Unsupported, not_found};
use crate::patch::render::{self, flow_list, plain_or_quoted};
use crate::patch::splice::Splice;

pub(super) fn add(text: &str, machine: &MachineDef, index: Option<usize>) -> Result<String> {
    step(text, |doc, style| {
        let machines = nav::machines(doc)?;
        let taken = machines.value.as_map().is_some_and(|m| m.get(&machine.name.value).is_some());
        check_new_name("machine", &machine.name.value, taken)?;
        let len = machines.value.as_map().map_or(0, |m| m.entries.len());
        let index = insert_at(index, len, "machines")?;
        let child = Child { block: render::machine(machine, style), inline: None, item: false };
        add_child(doc, style, machines, index, &child, true)
    })
}

/// Removes the machine, the rules that fire into it and its triggers in
/// external sources (handlers and sources are kept, possibly empty).
pub(super) fn remove(text: &str, name: &str) -> Result<String> {
    let def = definition(text)?;
    if !def.machines.iter().any(|m| m.name.value == name) {
        return Err(not_found("machine", name));
    }
    let mut text = text.to_owned();
    // Rules firing into the machine, last first so positions stay valid.
    for controller in def.controllers.iter().rev() {
        for handler in controller.on.iter().rev() {
            for (index, rule) in handler.rules.iter().enumerate().rev() {
                if rule.fire.value.machine == name {
                    text = controllers::remove_rule(&text, &controller.name.value, &handler.event.value, index)?;
                }
            }
        }
    }
    for source in &def.external {
        let kept: Vec<String> =
            source.triggers.iter().filter(|t| t.value.machine != name).map(|t| t.value.to_string()).collect();
        if kept.len() != source.triggers.len() {
            text = step(&text, |doc, _| {
                let section = nav::section(doc, "external")?.ok_or_else(|| not_found("external source", name))?;
                let (_, entry) = nav::named(&section.value, &source.name.value, "external source")?;
                set_names(doc, entry, &kept)
            })?;
        }
    }
    step(&text, |doc, _| {
        let machines = nav::machines(doc)?;
        let (index, _) = nav::named(&machines.value, name, "machine")?;
        match machines.value.as_map().map_or(0, |m| m.entries.len()) {
            1 => replace_value(doc, machines, Value::Inline("{}".to_owned())),
            _ => Ok(vec![remove_child(doc, &machines.value, index)?]),
        }
    })
}

/// Renames the machine key and every `Machine.trigger` and target selector
/// that names it.
pub(super) fn rename(text: &str, from: &str, to: &str) -> Result<String> {
    if from == to {
        return step(text, |doc, _| nav::machine(doc, from).map(|_| Vec::new()));
    }
    step(text, |doc, _| {
        let machines = nav::machines(doc)?;
        let (_, entry) = nav::named(&machines.value, from, "machine")?;
        let taken = machines.value.as_map().is_some_and(|m| m.get(to).is_some());
        check_new_name("machine", to, taken)?;
        let mut splices = vec![replace_scalar(doc, &entry.key, to)?];
        splices.extend(trigger_ref_splices(doc, from, to)?);
        Ok(splices)
    })
}

/// Splices renaming machine `from` in every `fire:`, `target:` and external
/// trigger reference.
fn trigger_ref_splices(doc: &Doc, from: &str, to: &str) -> Result<Vec<Splice>> {
    let mut splices = Vec::new();
    let retarget = |node: &Node, splices: &mut Vec<Splice>| -> Result<()> {
        if let Some(r) = node.text().and_then(parse_trigger_ref)
            && r.machine == from
        {
            splices.push(replace_scalar(doc, node, &format!("{to}.{}", r.trigger))?);
        }
        Ok(())
    };
    if let Some(section) = nav::section(doc, "controllers")?
        && let Some(controllers) = section.value.as_map()
    {
        for controller in &controllers.entries {
            let Some(on) = controller.value.as_map().and_then(|m| m.value("on")).and_then(|n| n.as_map()) else {
                continue;
            };
            for handler in &on.entries {
                for rule in nav::rules(&handler.value) {
                    let Some(rule) = rule.as_map() else { continue };
                    if let Some(fire) = rule.value("fire") {
                        retarget(fire, &mut splices)?;
                    }
                    if let Some(target) = rule.value("target")
                        && let Some(spec) = target.text().and_then(|t| parse_target(t).ok())
                        && spec.machine == from
                    {
                        let text = target.text().unwrap_or_default();
                        splices.push(replace_scalar(doc, target, &renamed_selector(text, &spec, to))?);
                    }
                }
            }
        }
    }
    if let Some(section) = nav::section(doc, "external")?
        && let Some(sources) = section.value.as_map()
    {
        for source in &sources.entries {
            for node in source.value.scalar_items() {
                retarget(node, &mut splices)?;
            }
        }
    }
    Ok(splices)
}

/// A selector with its machine renamed, keeping the author's spacing when
/// the machine word can be swapped in place.
fn renamed_selector(text: &str, spec: &TargetSpec, to: &str) -> String {
    let mut expected = spec.clone();
    expected.machine = to.to_owned();
    for &(offset, word) in words(text).iter().take(2) {
        if word == spec.machine {
            let candidate = format!("{}{to}{}", &text[..offset], &text[offset + word.len()..]);
            if parse_target(&candidate).as_ref() == Ok(&expected) {
                return candidate;
            }
        }
    }
    crate::yaml::selector(&expected)
}

/// Whitespace-separated words with their byte offsets.
fn words(text: &str) -> Vec<(usize, &str)> {
    let mut found = Vec::new();
    let mut start = None;
    for (i, c) in text.char_indices() {
        match (c.is_whitespace(), start) {
            (true, Some(s)) => {
                found.push((s, &text[s..i]));
                start = None;
            }
            (false, None) => start = Some(i),
            _ => {}
        }
    }
    if let Some(s) = start {
        found.push((s, &text[s..]));
    }
    found
}

pub(super) fn set_scalar(text: &str, machine: &str, key: &str, value: Option<String>) -> Result<String> {
    step(text, |doc, _| {
        let (_, entry) = nav::machine(doc, machine)?;
        set_scalar_key(doc, &entry.value, key, value.as_deref(), nav::MACHINE_ORDER)
    })
}

pub(super) fn set_fields(text: &str, machine: &str, fields: &[String]) -> Result<String> {
    step(text, |doc, _| {
        let (_, entry) = nav::machine(doc, machine)?;
        let body = &entry.value;
        match (body.as_map().and_then(|m| m.get("fields")), fields.is_empty()) {
            (Some(_), true) | (None, true) => set_key(doc, body, "fields", None, nav::MACHINE_ORDER),
            (Some((_, existing)), false) => set_names(doc, existing, fields),
            (None, false) => set_key(doc, body, "fields", Some(Value::Inline(flow_list(fields))), nav::MACHINE_ORDER),
        }
    })
}

/// Sets the system name; a new `system:` line opens the file (after its
/// header comments), separated like the other top-level sections.
pub(super) fn set_system(text: &str, name: Option<&str>) -> Result<String> {
    step(text, |doc, _| {
        let root = nav::root(doc)?;
        let map = root.as_map().ok_or(Unsupported::Shape)?;
        match (map.get("system"), name) {
            (None, Some(name)) if !map.flow => {
                let spans = map.spans();
                let separate = block::separated(doc, &spans, false)?;
                let line = format!("{}system: {}\n", " ".repeat(child_col(doc, root)?), plain_or_quoted(name, false));
                Ok(vec![block::insert(doc, &spans, 0, &line, separate)?])
            }
            _ => set_scalar_key(doc, root, "system", name, nav::ROOT_ORDER),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selectors_keep_their_spacing() {
        let rename = |text: &str| {
            let spec = parse_target(text).expect("selector");
            renamed_selector(text, &spec, "Charge")
        };
        assert_eq!(rename("Payment where orderId == event.orderId"), "Charge where orderId == event.orderId");
        assert_eq!(rename("all  Payment where a == b"), "all  Charge where a == b");
        assert_eq!(rename("new Payment with orderId = event.orderId"), "new Charge with orderId = event.orderId");
        assert_eq!(rename("Payment"), "Charge");
    }
}
