//! Transition ops. A new transition is written like its neighbours: a
//! one-line flow mapping padded to their columns, or a block mapping with
//! their key indentation.

use cascade_core::definition::TransitionDef;

use super::{Child, add_child, definition, insert_at, nav, step};
use crate::patch::collection::{Value, remove_child, replace_scalar, set_key, set_names, set_text_key};
use crate::patch::doc::{Doc, Node, Seq};
use crate::patch::error::{Result, SurgeryError, not_found, out_of_range};
use crate::patch::render::{Block, RowLayout, Style, flow_list, transition_block, transition_row};
use crate::patch::splice::Splice;

/// Transition `index` of a machine (its mapping node).
pub(super) fn node<'d>(doc: &'d Doc, machine: &str, index: usize) -> Result<&'d Node> {
    let (_, m) = nav::machine(doc, machine)?;
    let items: Vec<&Node> = m.value.as_map().and_then(|b| b.value("transitions")).map(Node::items).unwrap_or_default();
    let len = items.len();
    items.get(index).copied().ok_or_else(|| out_of_range("transitions", index, len))
}

/// Whether a node is a flow mapping on one line.
fn one_line_flow(doc: &Doc, node: &Node) -> bool {
    node.as_map().is_some_and(|m| m.flow) && doc.text().get(node.start..node.end).is_some_and(|t| !t.contains('\n'))
}

/// Column layout shared by the one-line flow transitions of a list, if
/// they are aligned.
fn layout(doc: &Doc, style: Style, seq: &Seq) -> RowLayout {
    let rows: Vec<&Node> = seq.items.iter().map(|i| &i.node).filter(|n| one_line_flow(doc, n)).collect();
    let mut layout = RowLayout { quote_text: style.quote_text, ..RowLayout::default() };
    if let Some(first) = rows.first() {
        layout.brace_pad = doc.text().get(first.start + 1..first.start + 2) == Some(" ");
    }
    let rel = |row: &Node, key: usize| -> Option<usize> {
        let entry = row.as_map()?.entries.get(key)?;
        Some(doc.col(entry.key.start) - doc.col(row.start))
    };
    let canonical = |row: &&Node| {
        row.as_map().is_some_and(|m| {
            let keys: Vec<Option<&str>> = m.entries.iter().take(3).map(|e| e.key.text()).collect();
            keys == [Some("from"), Some("to"), Some("on")]
        })
    };
    if rows.len() < 2 || !rows.iter().all(canonical) {
        return layout;
    }
    let shared = |rows: &[&Node], key: usize| -> Option<usize> {
        let cols: Vec<Option<usize>> = rows.iter().map(|r| rel(r, key)).collect();
        let first = cols.first().copied().flatten()?;
        cols.iter().all(|c| *c == Some(first)).then_some(first)
    };
    layout.to_col = shared(&rows, 1);
    layout.on_col = shared(&rows, 2);
    if layout.to_col.is_some() && layout.on_col.is_some() {
        let with_rest: Vec<&Node> =
            rows.iter().copied().filter(|r| r.as_map().is_some_and(|m| m.entries.len() > 3)).collect();
        if with_rest.len() >= 2 {
            layout.rest_col = shared(&with_rest, 3);
        }
    }
    layout
}

pub(super) fn add(text: &str, machine: &str, t: &TransitionDef, index: Option<usize>) -> Result<String> {
    step(text, |doc, style| {
        let (_, m) = nav::machine(doc, machine)?;
        let body = &m.value;
        let Some((_, list)) = body.as_map().and_then(|b| b.get("transitions")) else {
            let mut item = Block::new();
            let layout = RowLayout { quote_text: style.quote_text, ..RowLayout::default() };
            item.line(style.seq_offset, format!("- {}", transition_row(t, &layout)));
            return set_key(doc, body, "transitions", Some(Value::Lines(item)), nav::MACHINE_ORDER);
        };
        let len = list.value.as_seq().map_or(0, |s| s.items.len());
        let index = insert_at(index, len, "transitions")?;
        let child = child(doc, style, &list.value, t);
        add_child(doc, style, list, index, &child, false)
    })
}

/// The new transition in the style of the list's current items.
fn child(doc: &Doc, style: Style, list: &Node, t: &TransitionDef) -> Child {
    let Some(seq) = list.as_seq() else {
        let layout = RowLayout { quote_text: style.quote_text, ..RowLayout::default() };
        return Child::line(transition_row(t, &layout), true);
    };
    let layout = layout(doc, style, seq);
    let row = transition_row(t, &layout);
    if seq.flow {
        return Child::line(row, true);
    }
    let flow = seq.items.iter().filter(|i| one_line_flow(doc, &i.node)).count();
    let block = seq.items.iter().find(|i| i.node.as_map().is_some_and(|m| !m.flow));
    match block {
        Some(item) if seq.items.len() - flow > flow => {
            let key_offset = doc.col(item.node.start).saturating_sub(doc.col(item.head)).max(2);
            Child { block: transition_block(t, key_offset, style.quote_text), inline: Some(row), item: true }
        }
        _ => Child::line(row, true),
    }
}

/// Replace a transition field by field, touching only what changed.
pub(super) fn update(text: &str, machine: &str, index: usize, new: &TransitionDef) -> Result<String> {
    let def = definition(text)?;
    let mdef = def.machines.iter().find(|m| m.name.value == machine).ok_or_else(|| not_found("machine", machine))?;
    let old = mdef.transitions.get(index).ok_or_else(|| out_of_range("transitions", index, mdef.transitions.len()))?;
    let values = |items: &[cascade_core::Spanned<String>]| items.iter().map(|s| s.value.clone()).collect::<Vec<_>>();
    let mut text = text.to_owned();
    let edit = |text: &mut String, f: &dyn Fn(&Doc, &Node) -> Result<Vec<Splice>>| {
        *text = step(text, |doc, _| f(doc, node(doc, machine, index)?))?;
        Ok::<(), SurgeryError>(())
    };
    if values(&old.from) != values(&new.from) {
        let from = values(&new.from);
        edit(&mut text, &|doc, n| set_names(doc, nav::require(n, "from")?, &from))?;
    }
    if old.to.value != new.to.value {
        edit(&mut text, &|doc, n| Ok(vec![replace_scalar(doc, &nav::require(n, "to")?.value, &new.to.value)?]))?;
    }
    if old.on.value != new.on.value {
        edit(&mut text, &|doc, n| Ok(vec![replace_scalar(doc, &nav::require(n, "on")?.value, &new.on.value)?]))?;
    }
    if old.guard.as_ref().map(|g| &g.value) != new.guard.as_ref().map(|g| &g.value) {
        let guard = new.guard.as_ref().map(|g| g.value.as_str());
        edit(&mut text, &|doc, n| set_text_key(doc, n, "guard", guard, nav::TRANSITION_ORDER))?;
    }
    if values(&old.emits) != values(&new.emits) {
        let emits = values(&new.emits);
        edit(&mut text, &|doc, n| match (n.as_map().and_then(|m| m.get("emits")), emits.is_empty()) {
            (_, true) => set_key(doc, n, "emits", None, nav::TRANSITION_ORDER),
            (Some((_, entry)), false) => set_names(doc, entry, &emits),
            (None, false) => set_key(doc, n, "emits", Some(Value::Inline(flow_list(&emits))), nav::TRANSITION_ORDER),
        })?;
    }
    if old.bounded != new.bounded {
        let bounded = new.bounded.then(|| Value::Inline("true".to_owned()));
        edit(&mut text, &|doc, n| set_key(doc, n, "bounded", bounded.clone(), nav::TRANSITION_ORDER))?;
    }
    Ok(text)
}

/// Remove a transition; removing the last one removes the `transitions:`
/// key.
pub(super) fn remove(text: &str, machine: &str, index: usize) -> Result<String> {
    step(text, |doc, _| {
        let (_, m) = nav::machine(doc, machine)?;
        let body = &m.value;
        let list =
            body.as_map().and_then(|b| b.value("transitions")).ok_or_else(|| out_of_range("transitions", index, 0))?;
        let len = list.as_seq().map_or(0, |s| s.items.len());
        if index >= len {
            return Err(out_of_range("transitions", index, len));
        }
        if len == 1 && !list.is_flow() {
            return set_key(doc, body, "transitions", None, nav::MACHINE_ORDER);
        }
        Ok(vec![remove_child(doc, list, index)?])
    })
}
