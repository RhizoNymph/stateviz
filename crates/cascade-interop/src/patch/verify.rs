//! Comparing definitions while ignoring where things were written.

use cascade_core::Spanned;
use cascade_core::definition::{ControllerDef, Definition, MachineDef, StateDef};
use cascade_core::span::SourceSpan;

/// Whether two definitions are equal apart from their source spans.
pub(crate) fn same_definition(a: &Definition, b: &Definition) -> bool {
    stripped(a) == stripped(b)
}

/// `def` with every span unknown.
pub(crate) fn stripped(def: &Definition) -> Definition {
    let mut def = def.clone();
    if let Some(system) = &mut def.system {
        clear(system);
    }
    def.machines.iter_mut().for_each(strip_machine);
    for event in &mut def.events {
        clear(&mut event.name);
        event.payload.iter_mut().for_each(clear);
        event.span = SourceSpan::unknown();
    }
    def.controllers.iter_mut().for_each(strip_controller);
    for source in &mut def.external {
        clear(&mut source.name);
        source.triggers.iter_mut().for_each(clear);
        source.span = SourceSpan::unknown();
    }
    def
}

fn clear<T>(value: &mut Spanned<T>) {
    value.span = SourceSpan::unknown();
}

fn strip_machine(machine: &mut MachineDef) {
    clear(&mut machine.name);
    machine.color.iter_mut().for_each(clear);
    machine.domain.iter_mut().for_each(clear);
    machine.initial.iter_mut().for_each(clear);
    machine.fields.iter_mut().for_each(clear);
    machine.states.iter_mut().for_each(strip_state);
    for t in &mut machine.transitions {
        t.from.iter_mut().for_each(clear);
        clear(&mut t.to);
        clear(&mut t.on);
        t.guard.iter_mut().for_each(clear);
        t.emits.iter_mut().for_each(clear);
        t.span = SourceSpan::unknown();
    }
    machine.span = SourceSpan::unknown();
}

fn strip_state(state: &mut StateDef) {
    clear(&mut state.name);
    clear(&mut state.kind);
    state.initial.iter_mut().for_each(clear);
    state.states.iter_mut().for_each(strip_state);
    state.span = SourceSpan::unknown();
}

fn strip_controller(controller: &mut ControllerDef) {
    clear(&mut controller.name);
    for handler in &mut controller.on {
        clear(&mut handler.event);
        for rule in &mut handler.rules {
            clear(&mut rule.fire);
            rule.target.iter_mut().for_each(clear);
            rule.when.iter_mut().for_each(clear);
            rule.span = SourceSpan::unknown();
        }
        handler.span = SourceSpan::unknown();
    }
    controller.span = SourceSpan::unknown();
}

#[cfg(test)]
mod tests {
    use cascade_core::parse_definition;

    use super::*;

    #[test]
    fn formatting_does_not_matter_but_content_does() {
        let a = parse_definition("machines:\n  A:\n    states: [x, y]\n").expect("parses");
        let b = parse_definition("# c\nmachines: { A: { states: [ x,\n  y ] } }\n").expect("parses");
        let c = parse_definition("machines:\n  A:\n    states: [x, z]\n").expect("parses");
        assert!(same_definition(&a, &b));
        assert!(!same_definition(&a, &c));
    }
}
