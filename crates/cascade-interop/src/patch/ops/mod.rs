//! Surgical text edits, one module per group of [`EditOp`]s.
//!
//! Every op is one or more *steps*: parse the current text into a
//! [`Doc`], compute non-overlapping [`Splice`]s, apply them. Ops that need
//! several dependent edits (cascading removals, style conversions, field by
//! field updates) run several steps, re-parsing in between, so each step
//! sees exact spans.

mod controllers;
mod events;
mod machines;
pub(crate) mod nav;
mod refs;
mod states;
mod transitions;

use cascade_core::Definition;
use cascade_core::edit::{EditError, EditOp};
use cascade_core::parse::grammar::is_valid_name;

use super::block;
use super::collection::{child_col, empty_to_block, flow_insert, spans};
use super::doc::{Doc, Entry, Kind};
use super::error::{Result, SurgeryError, Unsupported, name_taken, out_of_range};
use super::render::{Block, Style};
use super::splice::{self, Splice};

/// Apply `op` to `text` in place.
pub(crate) fn apply(text: &str, op: &EditOp) -> Result<String> {
    match op {
        EditOp::AddMachine { machine, index } => machines::add(text, machine, *index),
        EditOp::RemoveMachine { machine } => machines::remove(text, machine),
        EditOp::RenameMachine { from, to } => machines::rename(text, from, to),
        EditOp::SetMachineColor { machine, color } => {
            machines::set_scalar(text, machine, "color", color.map(|c| c.name().to_owned()))
        }
        EditOp::SetMachineDomain { machine, domain } => machines::set_scalar(text, machine, "domain", domain.clone()),
        EditOp::SetMachineInitial { machine, initial } => {
            machines::set_scalar(text, machine, "initial", initial.clone())
        }
        EditOp::SetMachineFields { machine, fields } => machines::set_fields(text, machine, fields),
        EditOp::AddState { machine, parent, state, index } => {
            states::add(text, machine, parent.as_deref(), state, *index)
        }
        EditOp::RemoveState { machine, path } => states::remove(text, machine, path),
        EditOp::RenameState { machine, path, to } => states::rename(text, machine, path, to),
        EditOp::SetStateKind { machine, path, kind } => states::set_kind(text, machine, path, *kind),
        EditOp::SetStateInitial { machine, path, initial } => {
            states::set_initial(text, machine, path, initial.as_deref())
        }
        EditOp::AddTransition { machine, transition, index } => transitions::add(text, machine, transition, *index),
        EditOp::UpdateTransition { machine, index, transition } => {
            transitions::update(text, machine, *index, transition)
        }
        EditOp::RemoveTransition { machine, index } => transitions::remove(text, machine, *index),
        EditOp::DeclareEvent { event, index } => events::declare(text, event, *index),
        EditOp::RemoveEventDeclaration { event } => events::remove(text, event),
        EditOp::RenameEvent { from, to } => events::rename(text, from, to),
        EditOp::AddController { controller, index } => controllers::add(text, controller, *index),
        EditOp::RemoveController { controller } => controllers::remove(text, controller),
        EditOp::RenameController { from, to } => controllers::rename(text, from, to),
        EditOp::AddHandler { controller, handler, index } => {
            controllers::add_handler(text, controller, handler, *index)
        }
        EditOp::RemoveHandler { controller, event } => controllers::remove_handler(text, controller, event),
        EditOp::AddRule { controller, event, rule, index } => {
            controllers::add_rule(text, controller, event, rule, *index)
        }
        EditOp::UpdateRule { controller, event, index, rule } => {
            controllers::update_rule(text, controller, event, *index, rule)
        }
        EditOp::RemoveRule { controller, event, index } => controllers::remove_rule(text, controller, event, *index),
        EditOp::AddExternal { external, index } => events::add_external(text, external, *index),
        EditOp::RemoveExternal { external } => events::remove_external(text, external),
        EditOp::RenameExternal { from, to } => events::rename_external(text, from, to),
        EditOp::SetExternalTriggers { external, triggers } => events::set_external_triggers(text, external, triggers),
        EditOp::SetSystemName { name } => machines::set_system(text, name.as_deref()),
        EditOp::Batch(ops) => batch(text, ops),
    }
}

/// Apply ops in order. Nothing is validated between them (a batch is only
/// valid as a whole). `RemoveEventDeclaration(e)` directly followed by
/// `DeclareEvent(e, …)` at the same position is how a payload changes; it
/// is patched in place so the declaration keeps its comments.
fn batch(text: &str, ops: &[EditOp]) -> Result<String> {
    let mut text = text.to_owned();
    let mut rest = ops;
    while let Some((op, tail)) = rest.split_first() {
        if let (EditOp::RemoveEventDeclaration { event }, Some(EditOp::DeclareEvent { event: declared, index })) =
            (op, tail.first())
            && declared.name.value == *event
            && let Some(patched) = events::set_payload(&text, event, &declared.payload, *index)?
        {
            text = patched;
            rest = &tail[1..];
            continue;
        }
        text = apply(&text, op)?;
        rest = tail;
    }
    Ok(text)
}

/// One step: parse, compute splices, apply.
pub(crate) fn step(text: &str, edit: impl FnOnce(&Doc, Style) -> Result<Vec<Splice>>) -> Result<String> {
    let doc = Doc::parse(text)?;
    let style = Style::detect(&doc);
    let splices = edit(&doc, style)?;
    splice::apply(text, splices)
}

/// The parsed definition of the current text.
pub(crate) fn definition(text: &str) -> Result<Definition> {
    cascade_core::parse_definition(text).map_err(|_| Unsupported::Unparseable.into())
}

/// A new name must be valid and free.
pub(crate) fn check_new_name(what: &'static str, name: &str, taken: bool) -> Result<()> {
    if !is_valid_name(name) {
        return Err(SurgeryError::Edit(EditError::InvalidName(name.to_owned())));
    }
    if taken {
        return Err(name_taken(what, name));
    }
    Ok(())
}

/// Resolve an insertion index: `None` appends.
pub(crate) fn insert_at(index: Option<usize>, len: usize, what: &'static str) -> Result<usize> {
    match index {
        None => Ok(len),
        Some(i) if i <= len => Ok(i),
        Some(i) => Err(out_of_range(what, i, len)),
    }
}

/// A new child of a collection.
#[derive(Clone, Debug)]
pub(crate) struct Child {
    /// Block lines relative to the column of its siblings (starting with
    /// `- ` for sequence items).
    pub block: Block,
    /// One-line form for flow collections, when there is one.
    pub inline: Option<String>,
    /// A sequence item rather than a mapping entry.
    pub item: bool,
}

impl Child {
    /// A child that is one line in either style.
    pub fn line(text: String, item: bool) -> Self {
        let mut block = Block::new();
        block.line(0, if item { format!("- {text}") } else { text.clone() });
        Self { block, inline: Some(text), item }
    }
}

/// Add `child` at `index` to the collection held by `parent` (a mapping or
/// a sequence). An empty or missing collection in a block context becomes a
/// block collection; in a flow context it gets the inline form.
pub(crate) fn add_child(
    doc: &Doc,
    style: Style,
    parent: &Entry,
    index: usize,
    child: &Child,
    separate_default: bool,
) -> Result<Vec<Splice>> {
    let coll = &parent.value;
    let children = spans(coll);
    let in_flow = parent.key.as_scalar().is_some_and(|k| k.in_flow);
    let empty = children.is_empty() && !matches!(coll.kind, Kind::Scalar(_));
    if coll.is_flow() && (!empty || in_flow) {
        let inline = child.inline.as_deref().ok_or(Unsupported::Shape)?;
        return Ok(vec![flow_insert(coll, index, inline)?]);
    }
    if empty {
        let mut lines = Block::new();
        lines.nest(if child.item { style.seq_offset } else { style.unit }, child.block.clone());
        return empty_to_block(doc, parent, lines);
    }
    if matches!(coll.kind, Kind::Scalar(_)) {
        return Err(Unsupported::Shape.into());
    }
    let col = child_col(doc, coll)?;
    let separate = block::separated(doc, &children, separate_default)?;
    Ok(vec![block::insert(doc, &children, index, &child.block.at(col), separate)?])
}

/// Add a top-level section (`events:`, `controllers:`, `external:`) holding
/// `body` (relative to the section key), in canonical order.
pub(crate) fn add_section(doc: &Doc, style: Style, key: &str, body: Block) -> Result<Vec<Splice>> {
    let root = nav::root(doc)?;
    let map = root.as_map().ok_or(Unsupported::Shape)?;
    let rank = nav::ROOT_ORDER.iter().position(|k| *k == key).unwrap_or(nav::ROOT_ORDER.len());
    let index = map
        .entries
        .iter()
        .enumerate()
        .filter(|(_, e)| e.key.text().is_some_and(|k| nav::ROOT_ORDER[..rank].contains(&k)))
        .map(|(i, _)| i + 1)
        .max()
        .unwrap_or(0);
    let mut section = Block::new();
    section.line(0, format!("{key}:"));
    section.nest(style.unit, body);
    let children = map.spans();
    let separate = block::separated(doc, &children, true)?;
    let col = child_col(doc, root)?;
    Ok(vec![block::insert(doc, &children, index, &section.at(col), separate)?])
}

/// Remove top-level section `key` with its comments.
pub(crate) fn remove_section(doc: &Doc, key: &str) -> Result<Vec<Splice>> {
    let root = nav::root(doc)?;
    let map = root.as_map().ok_or(Unsupported::Shape)?;
    let (index, _) = map.get(key).ok_or(Unsupported::Shape)?;
    Ok(vec![super::collection::remove_child(doc, root, index)?])
}
