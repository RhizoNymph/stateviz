//! Transition ops. A new transition is written like its neighbours: a
//! one-line flow mapping padded to their columns, or a block mapping with
//! their key indentation.

use cascade_core::Spanned;
use cascade_core::definition::{EmitDef, TransitionDef, TriggerSpec};
use cascade_core::expr::Assignment;

use super::{Child, add_child, definition, emits, insert_at, keyed, nav, step};
use crate::patch::collection::{Value, remove_child, replace_scalar, set_key, set_names, set_text_key};
use crate::patch::doc::{Doc, Node, ScalarStyle, Seq};
use crate::patch::error::{Result, SurgeryError, Unsupported, not_found, out_of_range};
use crate::patch::render::{Block, RowLayout, Style, transition_block, transition_row};
use crate::patch::splice::Splice;
use crate::yaml::{assignments_text, emits_text, trigger_entry};

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
    let sets = |t: &TransitionDef| t.sets.iter().map(|a| a.value.clone()).collect::<Vec<_>>();
    let emits = |t: &TransitionDef| {
        t.emits
            .iter()
            .map(|e| (e.event.value.clone(), e.with.iter().map(|a| a.value.clone()).collect::<Vec<_>>()))
            .collect::<Vec<_>>()
    };
    if trigger_entry(&old.trigger) != trigger_entry(&new.trigger) {
        text = update_trigger(&text, machine, index, &old.trigger, &new.trigger)?;
    }
    if old.guard.as_ref().map(|g| &g.value) != new.guard.as_ref().map(|g| &g.value) {
        let guard = new.guard.as_ref().map(|g| g.value.to_string());
        edit(&mut text, &|doc, n| set_text_key(doc, n, "guard", guard.as_deref(), nav::TRANSITION_ORDER))?;
    }
    if sets(old) != sets(new) {
        text = update_sets(&text, machine, index, &new.sets)?;
    }
    if emits(old) != emits(new) {
        text = update_emits(&text, machine, index, &old.emits, &new.emits)?;
    }
    if old.bounded != new.bounded {
        let bounded = new.bounded.then(|| Value::Inline("true".to_owned()));
        edit(&mut text, &|doc, n| set_key(doc, n, "bounded", bounded.clone(), nav::TRANSITION_ORDER))?;
    }
    Ok(text)
}

/// The trigger: a command's name, a timer's delay, or a subscription's
/// event and filter in place when the kind stays; otherwise the old key
/// is replaced by the new one.
fn update_trigger(text: &str, machine: &str, index: usize, old: &TriggerSpec, new: &TriggerSpec) -> Result<String> {
    let edit = |text: &str, f: &dyn Fn(&Doc, &Node) -> Result<Vec<Splice>>| {
        step(text, |doc, _| f(doc, node(doc, machine, index)?))
    };
    match (old, new) {
        (TriggerSpec::Command(_), TriggerSpec::Command(name)) => {
            edit(text, &|doc, n| Ok(vec![replace_scalar(doc, &nav::require(n, "on")?.value, &name.value)?]))
        }
        (TriggerSpec::After(_), TriggerSpec::After(delay)) => {
            // Durations are written plain, as `to_yaml` writes them.
            let delay = delay.value.to_string();
            edit(text, &|doc, n| {
                let value = &nav::require(n, "after")?.value;
                match value.as_scalar().map(|s| s.style) {
                    Some(ScalarStyle::Plain) => Ok(vec![Splice::replace(value.start, value.end, delay.clone())]),
                    _ => Ok(vec![replace_scalar(doc, value, &delay)?]),
                }
            })
        }
        (
            TriggerSpec::Event { event: old_event, filter: old_filter },
            TriggerSpec::Event { event: new_event, filter: new_filter },
        ) if node(&Doc::parse(text)?, machine, index).is_ok_and(|n| on_map(n).is_some()) => {
            let mut text = text.to_owned();
            if old_event.value != new_event.value {
                text = edit(&text, &|doc, n| {
                    let on = on_map(n).ok_or(Unsupported::Shape)?;
                    Ok(vec![replace_scalar(doc, &nav::require(on, "event")?.value, &new_event.value)?])
                })?;
            }
            let filter_text = |f: &Option<cascade_core::Spanned<cascade_core::expr::Expr>>| {
                f.as_ref().map(|f| f.value.display_filter().to_string())
            };
            let (old_where, new_where) = (filter_text(old_filter), filter_text(new_filter));
            if old_where != new_where {
                text = edit(&text, &|doc, n| {
                    let on = on_map(n).ok_or(Unsupported::Shape)?;
                    set_text_key(doc, on, "where", new_where.as_deref(), &["event", "where"])
                })?;
            }
            Ok(text)
        }
        _ => {
            let (key, value) = trigger_entry(new);
            let other = if key == "on" { "after" } else { "on" };
            let text = edit(text, &|doc, n| set_key(doc, n, other, None, nav::TRANSITION_ORDER))?;
            edit(&text, &|doc, n| set_key(doc, n, key, Some(Value::Inline(value.clone())), nav::TRANSITION_ORDER))
        }
    }
}

/// The `on:` value of a transition node, when it is a mapping.
fn on_map(n: &Node) -> Option<&Node> {
    let on = &nav::require(n, "on").ok()?.value;
    on.as_map().map(|_| on)
}

/// `sets:` entry by entry (see [`keyed`]); removed when empty.
fn update_sets(text: &str, machine: &str, index: usize, sets: &[Spanned<Assignment>]) -> Result<String> {
    let has_sets = |doc: &Doc| node(doc, machine, index).is_ok_and(|n| nav::require(n, "sets").is_ok());
    if sets.is_empty() || !has_sets(&Doc::parse(text)?) {
        let value = (!sets.is_empty()).then(|| Value::Inline(assignments_text(sets)));
        return step(text, |doc, _| {
            set_key(doc, node(doc, machine, index)?, "sets", value.clone(), nav::TRANSITION_ORDER)
        });
    }
    let pairs: Vec<keyed::Pair> = sets.iter().map(|a| (a.value.field.clone(), a.value.value.to_string())).collect();
    keyed::reconcile_map(text, &|doc| nav::require(node(doc, machine, index)?, "sets"), &pairs)
}

/// `emits:` item by item (see [`emits`]); plain lists of names use
/// [`set_names`]; removed when empty.
fn update_emits(text: &str, machine: &str, index: usize, old: &[EmitDef], new: &[EmitDef]) -> Result<String> {
    let names: Vec<String> = new.iter().map(|e| e.event.value.clone()).collect();
    let plain = |list: &[EmitDef]| list.iter().all(|e| e.with.is_empty());
    let locate = |doc: &Doc| -> Option<bool> {
        let entry = nav::require(node(doc, machine, index).ok()?, "emits").ok()?;
        Some(entry.value.as_seq().is_some())
    };
    let shape = locate(&Doc::parse(text)?);
    match shape {
        _ if new.is_empty() => {
            step(text, |doc, _| set_key(doc, node(doc, machine, index)?, "emits", None, nav::TRANSITION_ORDER))
        }
        Some(_) if plain(old) && plain(new) => {
            step(text, |doc, _| set_names(doc, nav::require(node(doc, machine, index)?, "emits")?, &names))
        }
        Some(true) => emits::reconcile(text, &|doc| nav::require(node(doc, machine, index)?, "emits"), old, new),
        Some(false) | None => {
            let whole = Value::Inline(emits_text(new));
            step(text, |doc, _| {
                set_key(doc, node(doc, machine, index)?, "emits", Some(whole.clone()), nav::TRANSITION_ORDER)
            })
        }
    }
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
