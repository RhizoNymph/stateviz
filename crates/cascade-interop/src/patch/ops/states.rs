//! State ops. States live in `states:` collections written as a flow list
//! of names, a block list of names and single-key mappings, or a mapping;
//! each op edits the form it finds and changes forms only when it must (a
//! flow list becomes a block list when a state in it needs a body).

use cascade_core::Spanned;
use cascade_core::definition::{MachineDef, StateDef, StateKindDef};
use cascade_core::span::SourceSpan;

use super::nav::{self, StateRef};
use super::refs::{StateTree, endpoints, renamed_reference, within};
use super::{Child, add_child, check_new_name, definition, insert_at, step, transitions};
use crate::patch::collection::{
    Value, flow_seq_to_block, remove_child, replace_scalar, replace_value, set_key, set_names, set_scalar_key,
};
use crate::patch::doc::{Doc, Entry, Kind, Node};
use crate::patch::error::{Result, Unsupported, not_found};
use crate::patch::render::{
    Block, StateSlot, Style, flow_list, has_body, plain_or_quoted, state_body, state_inline, state_item,
};
use crate::patch::splice::{self, Splice};

/// The outcome of one attempt: final text, or text converted to a form the
/// op can finish in (a flow list turned into a block list).
enum Progress {
    Done(Vec<Splice>),
    Convert(Vec<Splice>),
}

/// Run `attempt` until it is done; at most one conversion is allowed.
fn with_conversion(text: &str, mut attempt: impl FnMut(&Doc, Style) -> Result<Progress>) -> Result<String> {
    let mut text = text.to_owned();
    for _ in 0..2 {
        let doc = Doc::parse(&text)?;
        let style = Style::detect(&doc);
        match attempt(&doc, style)? {
            Progress::Done(splices) => return splice::apply(&text, splices),
            Progress::Convert(splices) => text = splice::apply(&text, splices)?,
        }
    }
    Err(Unsupported::Shape.into())
}

fn machine_def<'d>(def: &'d cascade_core::Definition, machine: &str) -> Result<&'d MachineDef> {
    def.machines.iter().find(|m| m.name.value == machine).ok_or_else(|| not_found("machine", machine))
}

fn split_path(path: &str) -> (Option<&str>, &str) {
    match path.rsplit_once('.') {
        Some((parent, name)) => (Some(parent), name),
        None => (None, path),
    }
}

fn plain_state(name: &str, children: Vec<StateDef>) -> StateDef {
    StateDef {
        name: Spanned::synthetic(name.to_owned()),
        kind: Spanned::synthetic(StateKindDef::Normal),
        initial: None,
        states: children,
        span: SourceSpan::unknown(),
    }
}

/// The mapping entry holding a state's name and body, when it has one.
fn state_entry<'d>(state: &StateRef<'d>) -> Option<&'d Entry> {
    match state.item {
        Some(item) => item.as_map().and_then(|m| m.entries.first()),
        None => state.list.value.as_map().and_then(|m| m.entries.get(state.index)),
    }
}

/// Whether a `states:` entry sits inside a flow mapping, where block
/// lists are impossible.
fn in_flow(list: &Entry) -> bool {
    list.key.as_scalar().is_some_and(|k| k.in_flow)
}

/// Replace a state's whole written form with `new` (rendered for its slot).
fn rewrite_state(doc: &Doc, style: Style, state: &StateRef<'_>, new: &StateDef) -> Result<Vec<Splice>> {
    match state.item {
        Some(item) if state.list.value.is_flow() => Ok(vec![Splice::replace(item.start, item.end, state_inline(new))]),
        Some(item) => {
            let col = doc.col(nav_head(doc, item)?);
            let text = state_item(new, style, StateSlot::SeqItem).as_item_text(col);
            Ok(vec![Splice::replace(item.start, item.end, text)])
        }
        None => {
            let entry = state_entry(state).ok_or(Unsupported::Shape)?;
            if !has_body(new) {
                return replace_value(doc, entry, Value::Inline("{}".to_owned()));
            }
            let rendered = state_item(new, style, StateSlot::MapEntry);
            match rendered.single_line().and_then(|l| l.split_once(": ")) {
                Some((_, value)) => replace_value(doc, entry, Value::Inline(value.to_owned())),
                None => {
                    let mut body = Block::new();
                    body.nest(style.unit, state_body(new, style));
                    replace_value(doc, entry, Value::Lines(body))
                }
            }
        }
    }
}

/// The `-` of a block sequence item node.
fn nav_head(doc: &Doc, item: &Node) -> Result<usize> {
    let before = doc.text().get(..item.start).ok_or(Unsupported::Shape)?;
    let trimmed = before.trim_end_matches([' ', '\t']);
    match trimmed.ends_with('-') {
        true => Ok(trimmed.len() - 1),
        false => Err(Unsupported::SharedLine.into()),
    }
}

// --- Add ----------------------------------------------------------------------------

pub(super) fn add(
    text: &str,
    machine: &str,
    parent: Option<&str>,
    state: &StateDef,
    index: Option<usize>,
) -> Result<String> {
    let before = StateTree::new(&machine_def(&definition(text)?, machine)?.states);
    let added = insert_state(text, machine, parent, state, index)?;
    retarget(&added, machine, &before)
}

/// Rewrite, as full paths, the references of `machine` that resolved in
/// `before` but now resolve elsewhere or not at all (a new state made a
/// bare name ambiguous, or captured it as a top-level path).
fn retarget(text: &str, machine: &str, before: &StateTree) -> Result<String> {
    let after = StateTree::new(&machine_def(&definition(text)?, machine)?.states);
    step(text, |doc, _| {
        let (_, m) = nav::machine(doc, machine)?;
        let mut splices = Vec::new();
        for node in references(&m.value) {
            if let Some(reference) = node.text()
                && let Some(target) = before.resolve(reference)
                && after.resolve(reference) != Some(target)
            {
                splices.push(replace_scalar(doc, node, target)?);
            }
        }
        Ok(splices)
    })
}

/// Every state reference of a machine body: its `initial:` and each
/// transition's `from` and `to`.
fn references(body: &Node) -> Vec<&Node> {
    let Some(map) = body.as_map() else {
        return Vec::new();
    };
    let mut found: Vec<&Node> = map.value("initial").into_iter().collect();
    for t in map.value("transitions").map(Node::items).unwrap_or_default() {
        let Some(t) = t.as_map() else { continue };
        found.extend(t.value("from").map(Node::scalar_items).unwrap_or_default());
        found.extend(t.value("to"));
    }
    found
}

fn insert_state(
    text: &str,
    machine: &str,
    parent: Option<&str>,
    state: &StateDef,
    index: Option<usize>,
) -> Result<String> {
    with_conversion(text, |doc, style| {
        let (_, m) = nav::machine(doc, machine)?;
        let list = match parent {
            None => nav::require(&m.value, "states")?,
            Some(path) => {
                let parent_ref = nav::state(&m.value, machine, path)?;
                match parent_ref.children() {
                    Some(list) => list,
                    None => return first_child(doc, style, &parent_ref, state),
                }
            }
        };
        let siblings = nav::states_of(list)?;
        let name = &state.name.value;
        check_new_name("state", name, siblings.iter().any(|s| s.name.text() == Some(name)))?;
        let index = insert_at(index, siblings.len(), "states")?;
        let child = match &list.value.kind {
            Kind::Seq(seq) if seq.flow && has_body(state) && in_flow(list) => Child::line(state_inline(state), true),
            Kind::Seq(seq) if seq.flow && has_body(state) => {
                return Ok(Progress::Convert(flow_seq_to_block(doc, list, style)?));
            }
            Kind::Map(_) => {
                let block = state_item(state, style, StateSlot::MapEntry);
                Child { inline: block.single_line().map(str::to_owned), block, item: false }
            }
            Kind::Seq(_) | Kind::Null | Kind::Scalar(_) => Child {
                block: state_item(state, style, StateSlot::SeqItem),
                inline: (!has_body(state)).then(|| plain_or_quoted(name, true)),
                item: true,
            },
        };
        add_child(doc, style, list, index, &child, false).map(Progress::Done)
    })
}

/// Give a state without written children its first child.
fn first_child(doc: &Doc, style: Style, parent: &StateRef<'_>, child: &StateDef) -> Result<Progress> {
    let name = parent.name.text().ok_or(Unsupported::Shape)?;
    match parent.body_map() {
        Some(body) => {
            let value = if has_body(child) && body.is_flow() {
                Value::Inline(format!("[{}]", state_inline(child)))
            } else if has_body(child) {
                let mut items = Block::new();
                items.nest(style.seq_offset, state_item(child, style, StateSlot::SeqItem));
                Value::Lines(items)
            } else {
                Value::Inline(flow_list(&[child.name.value.as_str()]))
            };
            set_key(doc, body, "states", Some(value), nav::STATE_ORDER).map(Progress::Done)
        }
        None if parent.list.value.is_flow() && parent.item.is_some() && !in_flow(parent.list) => {
            Ok(Progress::Convert(flow_seq_to_block(doc, parent.list, style)?))
        }
        None => rewrite_state(doc, style, parent, &plain_state(name, vec![child.clone()])).map(Progress::Done),
    }
}

// --- Remove ----------------------------------------------------------------------------

/// Removes the state, the transitions from or to it or its descendants
/// (a multi-source `from:` only loses the removed sources), and initials
/// that pointed at it.
pub(super) fn remove(text: &str, machine: &str, path: &str) -> Result<String> {
    let def = definition(text)?;
    let mdef = machine_def(&def, machine)?;
    let tree = StateTree::new(&mdef.states);
    if !tree.contains(path) {
        return Err(not_found("state", format!("{machine}.{path}")));
    }
    let hit = |p: Option<&str>| p.is_some_and(|p| within(p, path));
    let mut text = text.to_owned();
    for (index, t) in mdef.transitions.iter().enumerate().rev() {
        let (from, to) = endpoints(&tree, t);
        if hit(to) || from.iter().all(|f| hit(*f)) {
            text = transitions::remove(&text, machine, index)?;
        } else if from.iter().any(|f| hit(*f)) {
            let kept: Vec<String> =
                t.from.iter().zip(&from).filter(|(_, f)| !hit(**f)).map(|(s, _)| s.value.clone()).collect();
            text = step(&text, |doc, _| {
                let node = transitions::node(doc, machine, index)?;
                set_names(doc, nav::require(node, "from")?, &kept)
            })?;
        }
    }
    if mdef.initial.as_ref().is_some_and(|i| hit(tree.resolve(&i.value))) {
        text = step(&text, |doc, _| {
            let (_, m) = nav::machine(doc, machine)?;
            set_key(doc, &m.value, "initial", None, nav::MACHINE_ORDER)
        })?;
    }
    let (parent_path, name) = split_path(path);
    if let Some(parent_path) = parent_path {
        text = step(&text, |doc, _| {
            let (_, m) = nav::machine(doc, machine)?;
            let parent = nav::state(&m.value, machine, parent_path)?;
            match parent.body_map() {
                Some(body) if body.as_map().and_then(|b| b.value("initial")).and_then(Node::text) == Some(name) => {
                    set_key(doc, body, "initial", None, nav::STATE_ORDER)
                }
                _ => Ok(Vec::new()),
            }
        })?;
    }
    step(&text, |doc, style| {
        let (_, m) = nav::machine(doc, machine)?;
        let state = nav::state(&m.value, machine, path)?;
        if nav::states_of(state.list)?.len() > 1 {
            return Ok(vec![remove_child(doc, &state.list.value, state.index)?]);
        }
        let Some(parent_path) = parent_path else {
            return replace_value(doc, state.list, Value::Inline("[]".to_owned()));
        };
        let parent = nav::state(&m.value, machine, parent_path)?;
        let body = parent.body_map().ok_or(Unsupported::Shape)?;
        match body.as_map().map_or(0, |b| b.entries.len()) {
            1 => {
                let name = parent.name.text().ok_or(Unsupported::Shape)?;
                rewrite_state(doc, style, &parent, &plain_state(name, Vec::new()))
            }
            _ => set_key(doc, body, "states", None, nav::STATE_ORDER),
        }
    })
}

// --- Rename --------------------------------------------------------------------------------

/// Renames the state and rewrites every reference to it or its
/// descendants: the machine's `initial:`, its parent's `initial:`, and
/// transition endpoints. References keep their form (path or local name)
/// when that still resolves to the same state.
pub(super) fn rename(text: &str, machine: &str, path: &str, to: &str) -> Result<String> {
    let def = definition(text)?;
    let mdef = machine_def(&def, machine)?;
    let tree = StateTree::new(&mdef.states);
    if !tree.contains(path) {
        return Err(not_found("state", format!("{machine}.{path}")));
    }
    let (parent_path, old_name) = split_path(path);
    if old_name == to {
        return Ok(text.to_owned());
    }
    let new_path = parent_path.map_or_else(|| to.to_owned(), |p| format!("{p}.{to}"));
    check_new_name("state", to, tree.contains(&new_path))?;
    let new_tree = tree.renamed(path, &new_path, to);
    let rewrite = |doc: &Doc, node: &Node, splices: &mut Vec<Splice>| -> Result<()> {
        if let Some(reference) = node.text()
            && let Some(new) = renamed_reference(&tree, &new_tree, reference, path, &new_path, to)
        {
            splices.push(replace_scalar(doc, node, &new)?);
        }
        Ok(())
    };
    step(text, |doc, _| {
        let (_, m) = nav::machine(doc, machine)?;
        let body = &m.value;
        let state = nav::state(body, machine, path)?;
        let mut splices = vec![replace_scalar(doc, state.name, to)?];
        for node in references(body) {
            rewrite(doc, node, &mut splices)?;
        }
        if let Some(parent_path) = parent_path {
            let parent = nav::state(body, machine, parent_path)?;
            if let Some(initial) = parent.body_map().and_then(|b| b.as_map()).and_then(|b| b.value("initial"))
                && initial.text() == Some(old_name)
            {
                splices.push(replace_scalar(doc, initial, to)?);
            }
        }
        Ok(splices)
    })
}

// --- Kind and initial --------------------------------------------------------------------------

pub(super) fn set_kind(text: &str, machine: &str, path: &str, kind: StateKindDef) -> Result<String> {
    let value = (kind != StateKindDef::Normal).then(|| kind.name());
    set_body_key(text, machine, path, "kind", value)
}

pub(super) fn set_initial(text: &str, machine: &str, path: &str, initial: Option<&str>) -> Result<String> {
    set_body_key(text, machine, path, "initial", initial)
}

/// Set or remove a key of a state's body, creating the body (`name: {
/// key: value }`) or collapsing it back to a plain name as needed.
fn set_body_key(text: &str, machine: &str, path: &str, key: &str, value: Option<&str>) -> Result<String> {
    with_conversion(text, |doc, style| {
        let (_, m) = nav::machine(doc, machine)?;
        let state = nav::state(&m.value, machine, path)?;
        let name = state.name.text().ok_or(Unsupported::Shape)?;
        match (state.body_map(), value) {
            (Some(body), None) if body.as_map().is_some_and(|b| b.entries.len() == 1 && b.get(key).is_some()) => {
                rewrite_state(doc, style, &state, &plain_state(name, Vec::new())).map(Progress::Done)
            }
            (Some(body), value) => set_scalar_key(doc, body, key, value, nav::STATE_ORDER).map(Progress::Done),
            (None, None) => Ok(Progress::Done(Vec::new())),
            (None, Some(_)) if state.list.value.is_flow() && state.item.is_some() && !in_flow(state.list) => {
                Ok(Progress::Convert(flow_seq_to_block(doc, state.list, style)?))
            }
            (None, Some(value)) => {
                let body = format!("{{ {key}: {} }}", plain_or_quoted(value, true));
                match state.item {
                    Some(item) if state.list.value.is_flow() => Ok(Progress::Done(vec![Splice::replace(
                        item.start,
                        item.end,
                        format!("{{ {}: {body} }}", plain_or_quoted(name, true)),
                    )])),
                    Some(item) => Ok(Progress::Done(vec![Splice::replace(
                        item.start,
                        item.end,
                        format!("{}: {body}", doc.text().get(state.name.start..state.name.end).unwrap_or(name)),
                    )])),
                    None => {
                        let entry = state_entry(&state).ok_or(Unsupported::Shape)?;
                        replace_value(doc, entry, Value::Inline(body)).map(Progress::Done)
                    }
                }
            }
        }
    })
}
