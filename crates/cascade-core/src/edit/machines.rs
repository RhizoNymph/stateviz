//! Adding, removing and renaming machines, and the machine-level setters.

use super::engine::{Effect, push_unique, sequence};
use super::keys::{self, machine_key};
use super::lookup::{self, name_taken};
use super::{EditError, EditOp, Index, validate};
use crate::color::PaletteColor;
use crate::definition::{Definition, MachineDef};

pub(super) fn add(def: &mut Definition, machine: &MachineDef, index: Index) -> Result<Effect, EditError> {
    validate::machine(machine)?;
    let name = &machine.name.value;
    if def.machines.iter().any(|m| m.name.value == *name) {
        return Err(name_taken("machine", name.clone()));
    }
    lookup::insert(&mut def.machines, machine.clone(), index, "machine")?;
    Ok(Effect::new(EditOp::RemoveMachine { machine: name.clone() }, keys::machine_keys(machine)))
}

/// Removes the machine, every rule that fires into it and every external
/// trigger on it. Handlers and external sources themselves stay, possibly
/// empty.
pub(super) fn remove(def: &mut Definition, machine: &str) -> Result<Effect, EditError> {
    let at = lookup::machine_index(def, machine)?;
    let removed = def.machines.remove(at);
    let mut touched = keys::machine_keys(&removed);
    let mut inverse = vec![EditOp::AddMachine { machine: removed, index: Some(at) }];

    for c in &mut def.controllers {
        for h in &mut c.on {
            let (fired, kept): (Vec<_>, Vec<_>) = std::mem::take(&mut h.rules)
                .into_iter()
                .enumerate()
                .partition(|(_, r)| r.fire.value.machine == machine);
            h.rules = kept.into_iter().map(|(_, r)| r).collect();
            for (i, rule) in fired {
                touched.push(keys::rule_key(&c.name.value, &h.event.value, i));
                inverse.push(EditOp::AddRule {
                    controller: c.name.value.clone(),
                    event: h.event.value.clone(),
                    rule,
                    index: Some(i),
                });
            }
        }
    }

    for x in &mut def.external {
        if x.triggers.iter().any(|t| t.value.machine == machine) {
            let old = lookup::values(&x.triggers);
            x.triggers.retain(|t| t.value.machine != machine);
            touched.push(keys::external_key(&x.name.value));
            inverse.push(EditOp::SetExternalTriggers { external: x.name.value.clone(), triggers: old });
        }
    }
    Ok(Effect::new(sequence(inverse), touched))
}

/// Renames the machine and every reference to it: `fire:` refs, target
/// selectors and external triggers.
pub(super) fn rename(def: &mut Definition, from: &str, to: &str) -> Result<Effect, EditError> {
    validate::name(to)?;
    let at = lookup::machine_index(def, from)?;
    let inverse = EditOp::RenameMachine { from: to.to_owned(), to: from.to_owned() };
    if from == to {
        return Ok(Effect::new(inverse, vec![machine_key(from)]));
    }
    if def.machines.iter().any(|m| m.name.value == to) {
        return Err(name_taken("machine", to));
    }
    lookup::set_value(&mut def.machines[at].name, to.to_owned());
    let mut touched = keys::machine_keys(&def.machines[at]);

    for c in &mut def.controllers {
        for h in &mut c.on {
            for (i, r) in h.rules.iter_mut().enumerate() {
                let mut changed = false;
                if r.fire.value.machine == from {
                    r.fire.value.machine = to.to_owned();
                    changed = true;
                }
                if let Some(target) = r.target.as_mut()
                    && target.value.machine == from
                {
                    target.value.machine = to.to_owned();
                    changed = true;
                }
                if changed {
                    push_unique(&mut touched, [keys::rule_key(&c.name.value, &h.event.value, i)]);
                }
            }
        }
    }
    for x in &mut def.external {
        let mut changed = false;
        for t in x.triggers.iter_mut().filter(|t| t.value.machine == from) {
            t.value.machine = to.to_owned();
            changed = true;
        }
        if changed {
            push_unique(&mut touched, [keys::external_key(&x.name.value)]);
        }
    }
    Ok(Effect::new(inverse, touched))
}

pub(super) fn set_color(def: &mut Definition, machine: &str, color: Option<PaletteColor>) -> Result<Effect, EditError> {
    let mdef = lookup::machine_mut(def, machine)?;
    let old = mdef.color.as_ref().map(|c| c.value);
    lookup::set_optional(&mut mdef.color, color);
    Ok(Effect::new(EditOp::SetMachineColor { machine: machine.to_owned(), color: old }, vec![machine_key(machine)]))
}

pub(super) fn set_domain(def: &mut Definition, machine: &str, domain: Option<&str>) -> Result<Effect, EditError> {
    let mdef = lookup::machine_mut(def, machine)?;
    let old = mdef.domain.as_ref().map(|d| d.value.clone());
    lookup::set_optional(&mut mdef.domain, domain.map(str::to_owned));
    Ok(Effect::new(EditOp::SetMachineDomain { machine: machine.to_owned(), domain: old }, vec![machine_key(machine)]))
}

pub(super) fn set_initial(def: &mut Definition, machine: &str, initial: Option<&str>) -> Result<Effect, EditError> {
    if let Some(initial) = initial {
        validate::path(initial)?;
    }
    let mdef = lookup::machine_mut(def, machine)?;
    let old = mdef.initial.as_ref().map(|i| i.value.clone());
    lookup::set_optional(&mut mdef.initial, initial.map(str::to_owned));
    Ok(Effect::new(EditOp::SetMachineInitial { machine: machine.to_owned(), initial: old }, vec![machine_key(machine)]))
}

pub(super) fn set_fields(def: &mut Definition, machine: &str, fields: &[String]) -> Result<Effect, EditError> {
    fields.iter().try_for_each(|f| validate::name(f))?;
    let mdef = lookup::machine_mut(def, machine)?;
    let old = lookup::values(&mdef.fields);
    mdef.fields = lookup::synthetic_list(fields.iter().cloned());
    Ok(Effect::new(EditOp::SetMachineFields { machine: machine.to_owned(), fields: old }, vec![machine_key(machine)]))
}
