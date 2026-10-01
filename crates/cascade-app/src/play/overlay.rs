//! A [`PlayOverlay`] from a play session's state, keyed by [`ElementKey`]
//! so it does not depend on which `Model` instance the scene builder gets.

use std::ops::Range;

use cascade_core::{ElementKey, ElementRef, Model};
use cascade_scene::{PlayMarker, PlayOverlay};
use cascade_sim::{InstanceState, PendingItem, PendingKind, Trace, TraceStepKind};

/// The element a step "went through", for emphasis: transitions taken,
/// events emitted, handlers run, rules fired. External fires, drops and
/// selector failures emphasise nothing.
fn active_element(kind: &TraceStepKind) -> Option<ElementRef> {
    match kind {
        TraceStepKind::Transition { transition, .. } => Some(ElementRef::Transition(*transition)),
        TraceStepKind::Emit { event, .. } => Some(ElementRef::Event(*event)),
        TraceStepKind::Deliver { handler, .. } => Some(ElementRef::Handler(*handler)),
        TraceStepKind::Fire { rule, .. } | TraceStepKind::Spawn { rule, .. } => Some(ElementRef::Rule(*rule)),
        TraceStepKind::ExternalFire { .. }
        | TraceStepKind::Dropped { .. }
        | TraceStepKind::NoTarget { .. }
        | TraceStepKind::Ambiguous { .. } => None,
    }
}

fn pending_element(kind: &PendingKind) -> ElementRef {
    match kind {
        PendingKind::Event { event } => ElementRef::Event(*event),
        PendingKind::Fire { rule, .. } => ElementRef::Rule(*rule),
    }
}

fn push_unique(out: &mut Vec<ElementKey>, key: ElementKey) {
    if !out.contains(&key) {
        out.push(key);
    }
}

/// Build the overlay. `last` is the step range the last action appended
/// (out-of-range steps are ignored). `pending` keeps queue order; an
/// element queued twice appears once, at its first position.
pub fn build_overlay(
    model: &Model,
    trace: &Trace,
    instances: &[InstanceState],
    pending: &[PendingItem],
    last: Option<Range<usize>>,
) -> PlayOverlay {
    let markers = instances
        .iter()
        .map(|i| PlayMarker {
            instance: i.name.clone(),
            machine: model.machine(i.machine).name.clone(),
            state: model.key_of(ElementRef::State(i.state)),
        })
        .collect();
    let mut active = Vec::new();
    for step in last.and_then(|range| trace.steps.get(range)).unwrap_or_default() {
        if let Some(element) = active_element(&step.kind) {
            push_unique(&mut active, model.key_of(element));
        }
    }
    let mut queued = Vec::new();
    for item in pending {
        push_unique(&mut queued, model.key_of(pending_element(&item.kind)));
    }
    PlayOverlay { markers, active, pending: queued }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use cascade_core::ids::{EventId, HandlerId, MachineId, RuleId, StateId, TransitionId, TriggerId};
    use cascade_sim::{LifelineIx, PendingId, StepIx, TraceStep};

    use super::*;

    const TEXT: &str = "\
machines:
  Order:
    states: [draft, paid]
    transitions:
      - { from: draft, to: paid, on: pay, emits: [Paid] }
  Shipment:
    states: [idle, moving]
    transitions:
      - { from: idle, to: moving, on: start }
controllers:
  Fulfil:
    on:
      Paid:
        - fire: Shipment.start
external:
  User: [Order.pay]
";

    fn model() -> Model {
        cascade_core::load_str(TEXT).expect("loads")
    }

    fn step(kind: TraceStepKind) -> TraceStep {
        TraceStep { cause: None, kind }
    }

    fn trace(steps: Vec<TraceStep>) -> Trace {
        Trace { scenario: String::new(), ordering: None, lifelines: Vec::new(), steps, final_states: BTreeMap::new() }
    }

    fn ids(m: &Model) -> (MachineId, StateId, TransitionId, TriggerId, EventId, HandlerId, RuleId) {
        let order = m.machine_by_name("Order").expect("order");
        let paid = m.state_by_path(order, "paid").expect("paid");
        let pay = m.trigger_by_name(order, "pay").expect("pay");
        let transition = m.trigger(pay).accepted_by[0];
        let event = m.event_by_name("Paid").expect("event");
        let handler = m.event(event).handlers[0];
        let rule = m.handler(handler).rules[0];
        (order, paid, transition, pay, event, handler, rule)
    }

    #[test]
    fn markers_active_and_pending_are_keys() {
        let m = model();
        let (order, paid, transition, pay, event, handler, rule) = ids(&m);
        let l = LifelineIx(0);
        let t = trace(vec![
            step(TraceStepKind::ExternalFire { source: l, target: l, trigger: pay }),
            step(TraceStepKind::Transition { instance: l, transition, from: paid, to: paid }),
            step(TraceStepKind::Emit { instance: l, event }),
            step(TraceStepKind::Deliver { controller: l, event, handler }),
            step(TraceStepKind::Fire { controller: l, target: l, rule }),
            step(TraceStepKind::Emit { instance: l, event }),
        ]);
        let instances =
            [InstanceState { lifeline: l, name: "o1".into(), machine: order, state: paid, fields: BTreeMap::new() }];
        let pending = [
            PendingItem {
                id: PendingId(1),
                kind: PendingKind::Fire { rule, target: l },
                cause: StepIx(4),
                label: "x".into(),
            },
            PendingItem { id: PendingId(2), kind: PendingKind::Event { event }, cause: StepIx(5), label: "y".into() },
            PendingItem {
                id: PendingId(3),
                kind: PendingKind::Fire { rule, target: l },
                cause: StepIx(4),
                label: "z".into(),
            },
        ];
        let overlay = build_overlay(&m, &t, &instances, &pending, Some(0..6));
        assert_eq!(
            overlay.markers,
            [PlayMarker {
                instance: "o1".into(),
                machine: "Order".into(),
                state: ElementKey::State { machine: "Order".into(), path: "paid".into() }
            }]
        );
        assert_eq!(
            overlay.active,
            [
                m.key_of(ElementRef::Transition(transition)),
                ElementKey::Event { event: "Paid".into() },
                ElementKey::Handler { controller: "Fulfil".into(), event: "Paid".into() },
                ElementKey::Rule { controller: "Fulfil".into(), event: "Paid".into(), ordinal: 0 },
            ],
            "in step order, once each, external fires skipped"
        );
        assert_eq!(
            overlay.pending,
            [
                ElementKey::Rule { controller: "Fulfil".into(), event: "Paid".into(), ordinal: 0 },
                ElementKey::Event { event: "Paid".into() },
            ]
        );
    }

    #[test]
    fn only_the_last_actions_steps_are_active() {
        let m = model();
        let (_, paid, transition, _, event, _, _) = ids(&m);
        let l = LifelineIx(0);
        let t = trace(vec![
            step(TraceStepKind::Transition { instance: l, transition, from: paid, to: paid }),
            step(TraceStepKind::Emit { instance: l, event }),
        ]);
        let overlay = build_overlay(&m, &t, &[], &[], Some(1..2));
        assert_eq!(overlay.active, [ElementKey::Event { event: "Paid".into() }]);
        assert!(build_overlay(&m, &t, &[], &[], None).active.is_empty());
        assert!(build_overlay(&m, &t, &[], &[], Some(1..9)).active.is_empty(), "out of range is ignored");
    }

    #[test]
    fn an_empty_session_draws_nothing() {
        let m = model();
        assert_eq!(build_overlay(&m, &trace(Vec::new()), &[], &[], None), PlayOverlay::default());
    }
}
