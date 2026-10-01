//! Span-insensitive comparison of definitions.
//!
//! Edits cannot know where their results will land in the file, so the edit
//! laws hold "ignoring spans": compare `without_spans(a) == without_spans(b)`.

use crate::definition::{
    ControllerDef, Definition, EventDef, ExternalDef, HandlerDef, MachineDef, RuleDef, StateDef, TransitionDef,
};
use crate::span::{SourceSpan, Spanned};

/// A copy of `definition` with every span set to [`SourceSpan::unknown`].
pub fn without_spans(definition: &Definition) -> Definition {
    Definition {
        system: definition.system.as_ref().map(bare),
        machines: definition.machines.iter().map(machine).collect(),
        events: definition.events.iter().map(event).collect(),
        controllers: definition.controllers.iter().map(controller).collect(),
        external: definition.external.iter().map(external).collect(),
    }
}

fn bare<T: Clone>(value: &Spanned<T>) -> Spanned<T> {
    Spanned::synthetic(value.value.clone())
}

fn bare_all<T: Clone>(values: &[Spanned<T>]) -> Vec<Spanned<T>> {
    values.iter().map(bare).collect()
}

fn machine(m: &MachineDef) -> MachineDef {
    MachineDef {
        name: bare(&m.name),
        color: m.color.as_ref().map(bare),
        domain: m.domain.as_ref().map(bare),
        initial: m.initial.as_ref().map(bare),
        fields: bare_all(&m.fields),
        states: m.states.iter().map(state).collect(),
        transitions: m.transitions.iter().map(transition).collect(),
        span: SourceSpan::unknown(),
    }
}

fn state(s: &StateDef) -> StateDef {
    StateDef {
        name: bare(&s.name),
        kind: bare(&s.kind),
        initial: s.initial.as_ref().map(bare),
        states: s.states.iter().map(state).collect(),
        span: SourceSpan::unknown(),
    }
}

pub(super) fn transition(t: &TransitionDef) -> TransitionDef {
    TransitionDef {
        from: bare_all(&t.from),
        to: bare(&t.to),
        on: bare(&t.on),
        guard: t.guard.as_ref().map(bare),
        emits: bare_all(&t.emits),
        bounded: t.bounded,
        span: SourceSpan::unknown(),
    }
}

fn event(e: &EventDef) -> EventDef {
    EventDef { name: bare(&e.name), payload: bare_all(&e.payload), span: SourceSpan::unknown() }
}

fn controller(c: &ControllerDef) -> ControllerDef {
    ControllerDef { name: bare(&c.name), on: c.on.iter().map(handler).collect(), span: SourceSpan::unknown() }
}

fn handler(h: &HandlerDef) -> HandlerDef {
    HandlerDef { event: bare(&h.event), rules: h.rules.iter().map(rule).collect(), span: SourceSpan::unknown() }
}

fn rule(r: &RuleDef) -> RuleDef {
    RuleDef {
        fire: bare(&r.fire),
        target: r.target.as_ref().map(bare),
        when: r.when.as_ref().map(bare),
        bounded: r.bounded,
        span: SourceSpan::unknown(),
    }
}

fn external(x: &ExternalDef) -> ExternalDef {
    ExternalDef { name: bare(&x.name), triggers: bare_all(&x.triggers), span: SourceSpan::unknown() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_every_span_and_nothing_else() {
        let text = include_str!("../../../../examples/shop/cascade.yaml");
        let def = match crate::parse_definition(text) {
            Ok(def) => def,
            Err(err) => panic!("{err}"),
        };
        let stripped = without_spans(&def);
        assert_ne!(stripped, def);
        assert_eq!(without_spans(&stripped), stripped);
        let debug = format!("{stripped:?}");
        assert_eq!(debug.matches("line: 0,").count(), debug.matches("line: ").count(), "a span survived");
        assert_eq!(stripped.machines.len(), def.machines.len());
        assert_eq!(stripped.machines[0].states[1].states.len(), 2);
        assert_eq!(stripped.controllers[0].on[0].rules[0].fire.value, def.controllers[0].on[0].rules[0].fire.value);
    }
}
