//! Adding, removing and renaming states; setting a state's kind and initial
//! child.

use super::engine::{Effect, InverseMode, push_unique};
use super::keys::{self, machine_key, state_key};
use super::lookup::{self, name_taken, not_found};
use super::refs::{self, StateIndex, is_within, join, split};
use super::{EditError, EditOp, Index, restore, validate};
use crate::definition::{Definition, StateDef, StateKindDef};

fn state_not_found(machine: &str, path: &str) -> EditError {
    not_found("state", format!("{machine}.{path}"))
}

pub(super) fn add(
    def: &mut Definition,
    machine: &str,
    parent: Option<&str>,
    state: &StateDef,
    index: Index,
    mode: InverseMode,
) -> Result<Effect, EditError> {
    validate::state(state)?;
    if let Some(parent) = parent {
        validate::path(parent)?;
    }
    let m = lookup::machine_index(def, machine)?;
    let original = def.machines[m].clone();
    let before = StateIndex::of(&original.states);
    let path = join(parent, &state.name.value);

    let mdef = &mut def.machines[m];
    let siblings =
        refs::children_mut(mdef, parent).ok_or_else(|| state_not_found(machine, parent.unwrap_or_default()))?;
    if siblings.iter().any(|s| s.name.value == state.name.value) {
        return Err(name_taken("state", format!("{machine}.{path}")));
    }
    lookup::insert(siblings, state.clone(), index, "state")?;
    let changed = refs::retarget(mdef, &before, &|p| p.to_owned());

    let mut touched = keys::subtree_keys(machine, state, &path);
    push_unique(&mut touched, keys::entries_keys(mdef, &changed.transitions));
    if changed.initial {
        push_unique(&mut touched, [machine_key(machine)]);
    }
    let primary = vec![EditOp::RemoveState { machine: machine.to_owned(), path }];
    let inverse = restore::machine_inverse(def, m, &original, primary, mode)?;
    Ok(Effect::new(inverse, touched))
}

pub(super) fn remove(def: &mut Definition, machine: &str, path: &str, mode: InverseMode) -> Result<Effect, EditError> {
    let m = lookup::machine_index(def, machine)?;
    let original = def.machines[m].clone();
    let before = StateIndex::of(&original.states);
    let (parent, name) = split(path);

    let mdef = &mut def.machines[m];
    let siblings = refs::children_mut(mdef, parent).ok_or_else(|| state_not_found(machine, path))?;
    let at = siblings.iter().position(|s| s.name.value == name).ok_or_else(|| state_not_found(machine, path))?;
    let removed = siblings.remove(at);
    let mut touched = keys::subtree_keys(machine, &removed, path);

    // Transitions whose target is inside the subtree go; sources inside it
    // are dropped from `from` lists, and entries left without one go.
    let inside = |reference: &str| before.resolve(reference).is_some_and(|p| is_within(p, path));
    let mut removed_entries = Vec::new();
    let mut kept = Vec::with_capacity(original.transitions.len());
    for (i, t) in original.transitions.iter().enumerate() {
        let from: Vec<_> = t.from.iter().filter(|f| !inside(&f.value)).cloned().collect();
        if inside(&t.to.value) || from.is_empty() {
            removed_entries.push(i);
        } else {
            let mut t = t.clone();
            t.from = from;
            kept.push(t);
        }
    }
    mdef.transitions = kept;
    let gone = keys::expansions(&original)
        .into_iter()
        .filter(|e| is_within(&e.from, path) || is_within(&e.to, path))
        .map(|e| e.key(machine));
    push_unique(&mut touched, gone);

    // Initials that named the state fall back to the default (first) state.
    if mdef.initial.as_ref().is_some_and(|i| inside(&i.value)) {
        mdef.initial = None;
        push_unique(&mut touched, [machine_key(machine)]);
    }
    if let Some(parent) = parent
        && let Some(p) = refs::find_mut(&mut mdef.states, parent)
        && p.initial.as_ref().is_some_and(|i| i.value == name)
    {
        p.initial = None;
        push_unique(&mut touched, [state_key(machine, parent)]);
    }

    let mut primary = vec![EditOp::AddState {
        machine: machine.to_owned(),
        parent: parent.map(str::to_owned),
        state: removed,
        index: Some(at),
    }];
    primary.extend(removed_entries.into_iter().map(|i| EditOp::AddTransition {
        machine: machine.to_owned(),
        transition: original.transitions[i].clone(),
        index: Some(i),
    }));
    let inverse = restore::machine_inverse(def, m, &original, primary, mode)?;
    Ok(Effect::new(inverse, touched))
}

pub(super) fn rename(
    def: &mut Definition,
    machine: &str,
    path: &str,
    to: &str,
    mode: InverseMode,
) -> Result<Effect, EditError> {
    validate::name(to)?;
    let m = lookup::machine_index(def, machine)?;
    let original = def.machines[m].clone();
    let before = StateIndex::of(&original.states);
    let (parent, name) = split(path);
    let new_path = join(parent, to);

    let mdef = &mut def.machines[m];
    let siblings = refs::children_mut(mdef, parent).ok_or_else(|| state_not_found(machine, path))?;
    let at = siblings.iter().position(|s| s.name.value == name).ok_or_else(|| state_not_found(machine, path))?;
    if name == to {
        let op = EditOp::RenameState { machine: machine.to_owned(), path: path.to_owned(), to: to.to_owned() };
        return Ok(Effect::new(op, vec![state_key(machine, path)]));
    }
    if siblings.iter().any(|s| s.name.value == to) {
        return Err(name_taken("state", format!("{machine}.{new_path}")));
    }
    lookup::set_value(&mut siblings[at].name, to.to_owned());
    let renamed = siblings[at].clone();

    let mut touched = keys::subtree_keys(machine, &renamed, &new_path);
    if let Some(parent) = parent
        && let Some(p) = refs::find_mut(&mut mdef.states, parent)
        && let Some(initial) = p.initial.as_mut()
        && initial.value == name
    {
        lookup::set_value(initial, to.to_owned());
        push_unique(&mut touched, [state_key(machine, parent)]);
    }
    let moved = |p: &str| match p.strip_prefix(path) {
        Some(rest) if rest.is_empty() || rest.starts_with('.') => format!("{new_path}{rest}"),
        _ => p.to_owned(),
    };
    let changed = refs::retarget(mdef, &before, &moved);
    push_unique(&mut touched, keys::entries_keys(mdef, &changed.transitions));
    if changed.initial {
        push_unique(&mut touched, [machine_key(machine)]);
    }

    let primary = vec![EditOp::RenameState { machine: machine.to_owned(), path: new_path, to: name.to_owned() }];
    let inverse = restore::machine_inverse(def, m, &original, primary, mode)?;
    Ok(Effect::new(inverse, touched))
}

pub(super) fn set_kind(
    def: &mut Definition,
    machine: &str,
    path: &str,
    kind: StateKindDef,
) -> Result<Effect, EditError> {
    let mdef = lookup::machine_mut(def, machine)?;
    let state = refs::find_mut(&mut mdef.states, path).ok_or_else(|| state_not_found(machine, path))?;
    let old = state.kind.value;
    lookup::set_value(&mut state.kind, kind);
    let inverse = EditOp::SetStateKind { machine: machine.to_owned(), path: path.to_owned(), kind: old };
    Ok(Effect::new(inverse, vec![state_key(machine, path)]))
}

pub(super) fn set_initial(
    def: &mut Definition,
    machine: &str,
    path: &str,
    initial: Option<&str>,
) -> Result<Effect, EditError> {
    if let Some(initial) = initial {
        validate::name(initial)?;
    }
    let mdef = lookup::machine_mut(def, machine)?;
    let state = refs::find_mut(&mut mdef.states, path).ok_or_else(|| state_not_found(machine, path))?;
    let old = state.initial.as_ref().map(|i| i.value.clone());
    lookup::set_optional(&mut state.initial, initial.map(str::to_owned));
    let inverse = EditOp::SetStateInitial { machine: machine.to_owned(), path: path.to_owned(), initial: old };
    Ok(Effect::new(inverse, vec![state_key(machine, path)]))
}
