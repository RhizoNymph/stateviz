//! Event declarations and external sources: top-level sections of one-line
//! entries (`Name: { payload: [a] }`, `Name: [Machine.trigger]`).

use cascade_core::Spanned;
use cascade_core::definition::{Definition, EventDef, ExternalDef, TriggerRef};
use cascade_core::span::SourceSpan;

use super::{Child, add_child, add_section, check_new_name, definition, insert_at, nav, remove_section, step};
use crate::patch::collection::{Value, remove_child, replace_scalar, replace_value, set_key, set_names};
use crate::patch::doc::{Doc, Entry, Kind, Node};
use crate::patch::error::{Result, Unsupported, not_found};
use crate::patch::render::{Block, event_entry, external_entry, flow_list, plain_or_quoted};
use crate::patch::splice::Splice;

/// Events emitted or subscribed to, in first-mention order (transitions,
/// then handlers), as the resolver creates them.
fn used_events(def: &Definition) -> Vec<String> {
    let mut used: Vec<String> = Vec::new();
    let mentions = def
        .machines
        .iter()
        .flat_map(|m| m.transitions.iter().flat_map(|t| t.emits.iter()))
        .chain(def.controllers.iter().flat_map(|c| c.on.iter().map(|h| &h.event)));
    for event in mentions {
        if !used.contains(&event.value) {
            used.push(event.value.clone());
        }
    }
    used
}

/// The declarations section's entries: `(name node, index)`.
fn declared(section: &Entry) -> Vec<(&Node, usize)> {
    match &section.value.kind {
        Kind::Map(m) => m.entries.iter().enumerate().map(|(i, e)| (&e.key, i)).collect(),
        Kind::Seq(s) => s.items.iter().enumerate().map(|(i, item)| (&item.node, i)).collect(),
        Kind::Scalar(_) | Kind::Null => Vec::new(),
    }
}

/// Declares an event. A file without declarations switches to strict mode,
/// so every event it already emits or subscribes to is declared too, in
/// first-mention order, with the new one at `index`.
pub(super) fn declare(text: &str, event: &EventDef, index: Option<usize>) -> Result<String> {
    let def = definition(text)?;
    let name = &event.name.value;
    if def.events.is_empty() {
        let mut events: Vec<EventDef> = used_events(&def)
            .into_iter()
            .filter(|e| e != name)
            .map(|e| EventDef { name: Spanned::synthetic(e), payload: Vec::new(), span: SourceSpan::unknown() })
            .collect();
        check_new_name("event", name, false)?;
        let at = insert_at(index, events.len(), "events")?;
        events.insert(at, event.clone());
        let mut body = Block::new();
        for e in &events {
            body.line(0, event_entry(e));
        }
        return step(text, |doc, style| match nav::section(doc, "events")? {
            Some(section) => {
                let mut lines = Block::new();
                lines.nest(style.unit, body.clone());
                replace_value(doc, section, Value::Lines(lines))
            }
            None => add_section(doc, style, "events", body.clone()),
        });
    }
    let text = match (event.payload.is_empty(), step(text, |doc, _| list_to_mapping(doc))) {
        (false, Ok(converted)) => converted,
        _ => text.to_owned(),
    };
    step(&text, |doc, style| {
        let section = nav::section(doc, "events")?.ok_or(Unsupported::Shape)?;
        let names = declared(section);
        check_new_name("event", name, names.iter().any(|(n, _)| n.text() == Some(name)))?;
        let at = insert_at(index, names.len(), "events")?;
        let child = match &section.value.kind {
            Kind::Seq(_) if event.payload.is_empty() => {
                Child::line(plain_or_quoted(name, section.value.is_flow()), true)
            }
            Kind::Seq(_) => return Err(Unsupported::Shape.into()),
            _ => Child::line(event_entry(event), false),
        };
        add_child(doc, style, section, at, &child, false)
    })
}

/// A list of event names (`- A`, `[A, B]`) as a mapping (`A: {}`), so a
/// declaration with a payload can join it. Fails when there is no list.
fn list_to_mapping(doc: &Doc) -> Result<Vec<Splice>> {
    let section = nav::section(doc, "events")?.ok_or(Unsupported::Shape)?;
    let seq = section.value.as_seq().ok_or(Unsupported::Shape)?;
    let entry = |node: &Node| -> Result<String> {
        Ok(format!("{}: {{}}", plain_or_quoted(node.text().ok_or(Unsupported::Shape)?, seq.flow)))
    };
    if seq.flow {
        let entries: Vec<String> = seq.items.iter().map(|i| entry(&i.node)).collect::<Result<_>>()?;
        return Ok(vec![Splice::replace(
            section.value.start,
            section.value.end,
            format!("{{ {} }}", entries.join(", ")),
        )]);
    }
    if seq.items.first().is_none_or(|first| doc.col(first.head) <= doc.col(section.key.start)) {
        return Err(Unsupported::Shape.into());
    }
    seq.items.iter().map(|i| Ok(Splice::replace(i.head, i.node.end, entry(&i.node)?))).collect()
}

pub(super) fn remove(text: &str, event: &str) -> Result<String> {
    step(text, |doc, _| {
        let section = nav::section(doc, "events")?.ok_or_else(|| not_found("event", event))?;
        let names = declared(section);
        let (_, index) =
            names.iter().find(|(n, _)| n.text() == Some(event)).ok_or_else(|| not_found("event", event))?;
        match names.len() {
            1 => remove_section(doc, "events"),
            _ => Ok(vec![remove_child(doc, &section.value, *index)?]),
        }
    })
}

/// Change a declaration's payload in place, when the declaration is at
/// `index` (`None`: last) of a mapping-form `events:` with other
/// declarations. `None` otherwise, so the caller removes and re-declares.
pub(super) fn set_payload(
    text: &str,
    event: &str,
    payload: &[Spanned<String>],
    index: Option<usize>,
) -> Result<Option<String>> {
    let doc = Doc::parse(text)?;
    let Some(section) = nav::section(&doc, "events")? else {
        return Ok(None);
    };
    let Some(map) = section.value.as_map() else {
        return Ok(None);
    };
    let Some((position, _)) = map.get(event) else {
        return Ok(None);
    };
    // Removing the only declaration makes the file lenient, and declaring
    // into an empty list then declares every event in use: not in place.
    if map.entries.len() < 2 || index.unwrap_or(map.entries.len() - 1) != position {
        return Ok(None);
    }
    let payload: Vec<String> = payload.iter().map(|p| p.value.clone()).collect();
    let patched = step(text, |doc, _| {
        let section = nav::section(doc, "events")?.ok_or(Unsupported::Shape)?;
        let (_, entry) = nav::named(&section.value, event, "event")?;
        let body = &entry.value;
        let existing = body.as_map().and_then(|m| m.get("payload")).map(|(_, e)| e);
        let only_key = body.as_map().is_some_and(|m| m.entries.len() == 1);
        match (existing, payload.is_empty()) {
            (Some(_), true) if only_key => replace_value(doc, entry, Value::Inline("{}".to_owned())),
            (Some(_), true) => set_key(doc, body, "payload", None, &["payload"]),
            (Some(list), false) => set_names(doc, list, &payload),
            (None, true) => Ok(Vec::new()),
            (None, false) if body.as_map().is_some_and(|m| !m.entries.is_empty()) => {
                set_key(doc, body, "payload", Some(Value::Inline(flow_list(&payload))), &["payload"])
            }
            (None, false) => {
                replace_value(doc, entry, Value::Inline(format!("{{ payload: {} }}", flow_list(&payload))))
            }
        }
    })?;
    Ok(Some(patched))
}

/// Renames an event in its declaration, every `emits:` and every handler.
pub(super) fn rename(text: &str, from: &str, to: &str) -> Result<String> {
    let def = definition(text)?;
    let used = used_events(&def);
    let declared_names: Vec<&str> = def.events.iter().map(|e| e.name.value.as_str()).collect();
    if !used.iter().any(|u| u == from) && !declared_names.contains(&from) {
        return Err(not_found("event", from));
    }
    if from == to {
        return Ok(text.to_owned());
    }
    check_new_name("event", to, used.iter().any(|u| u == to) || declared_names.contains(&to))?;
    step(text, |doc, _| {
        let mut splices = Vec::new();
        let mut rename = |node: &Node| -> Result<()> {
            if node.text() == Some(from) {
                splices.push(replace_scalar(doc, node, to)?);
            }
            Ok(())
        };
        if let Some(section) = nav::section(doc, "events")? {
            for (node, _) in declared(section) {
                rename(node)?;
            }
        }
        for machine in nav::machines(doc)?.value.as_map().map(|m| m.entries.as_slice()).unwrap_or_default() {
            let transitions = machine.value.as_map().and_then(|b| b.value("transitions")).map(Node::items);
            for t in transitions.unwrap_or_default() {
                if let Some(emits) = t.as_map().and_then(|m| m.value("emits")) {
                    for node in emits.scalar_items() {
                        rename(node)?;
                    }
                }
            }
        }
        for_each_handler(doc, |handler| rename(&handler.key))?;
        Ok(splices)
    })
}

/// Visit every handler entry of every controller.
fn for_each_handler(doc: &Doc, mut visit: impl FnMut(&Entry) -> Result<()>) -> Result<()> {
    let Some(section) = nav::section(doc, "controllers")? else {
        return Ok(());
    };
    for controller in section.value.as_map().map(|m| m.entries.as_slice()).unwrap_or_default() {
        let on = controller.value.as_map().and_then(|b| b.value("on")).and_then(Node::as_map);
        for handler in on.map(|m| m.entries.as_slice()).unwrap_or_default() {
            visit(handler)?;
        }
    }
    Ok(())
}

// --- External sources ------------------------------------------------------------------

pub(super) fn add_external(text: &str, external: &ExternalDef, index: Option<usize>) -> Result<String> {
    step(text, |doc, style| {
        let entry = external_entry(external);
        let name = &external.name.value;
        let Some(section) = nav::section(doc, "external")? else {
            check_new_name("external source", name, false)?;
            insert_at(index, 0, "external sources")?;
            let mut body = Block::new();
            body.line(0, entry);
            return add_section(doc, style, "external", body);
        };
        let taken = section.value.as_map().is_some_and(|m| m.get(name).is_some());
        check_new_name("external source", name, taken)?;
        let len = section.value.as_map().map_or(0, |m| m.entries.len());
        let at = insert_at(index, len, "external sources")?;
        add_child(doc, style, section, at, &Child::line(entry, false), false)
    })
}

fn source<'d>(doc: &'d Doc, name: &str) -> Result<(&'d Entry, usize, &'d Entry)> {
    let section = nav::section(doc, "external")?.ok_or_else(|| not_found("external source", name))?;
    let (index, entry) = nav::named(&section.value, name, "external source")?;
    Ok((section, index, entry))
}

pub(super) fn remove_external(text: &str, name: &str) -> Result<String> {
    step(text, |doc, _| {
        let (section, index, _) = source(doc, name)?;
        match section.value.as_map().map_or(0, |m| m.entries.len()) {
            1 => remove_section(doc, "external"),
            _ => Ok(vec![remove_child(doc, &section.value, index)?]),
        }
    })
}

pub(super) fn rename_external(text: &str, from: &str, to: &str) -> Result<String> {
    step(text, |doc, _| {
        let (section, _, entry) = source(doc, from)?;
        if from == to {
            return Ok(Vec::new());
        }
        let taken = section.value.as_map().is_some_and(|m| m.get(to).is_some());
        check_new_name("external source", to, taken)?;
        Ok(vec![replace_scalar(doc, &entry.key, to)?])
    })
}

pub(super) fn set_external_triggers(text: &str, name: &str, triggers: &[TriggerRef]) -> Result<String> {
    let triggers: Vec<String> = triggers.iter().map(TriggerRef::to_string).collect();
    step(text, |doc, _| {
        let (_, _, entry) = source(doc, name)?;
        set_names(doc, entry, &triggers)
    })
}
