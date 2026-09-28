//! Model → one SCXML document.
//!
//! Representation (read back by [`super::read`] without loss, apart from
//! the losses listed in the module docs of [`super`]):
//!
//! ```text
//! <scxml name="<system>" xmlns:cascade="urn:x-cascade:scxml">
//!   <cascade:event name="OrderPaid" payload="orderId amount"/>   declared events
//!   <cascade:source name="Customer" fires="Order.submit"/>      external sources
//!   <parallel id="system">                                      the system
//!     <state id="Order" initial="Order.draft" cascade:color="blue">   a machine
//!       <state id="Order.draft">                                state ids: Machine.path
//!         <transition event="Order.submit" target="Order.pending" cond="<guard>">
//!           <send event="OrderSubmitted"/>                      emits
//!         </transition>
//!       </state>
//!       …
//!     </state>
//!     <state id="Fulfillment" cascade:controller="Fulfillment">       a controller
//!       <transition event="OrderPaid">                          a handler
//!         <send event="Shipment.start" cascade:target="Shipment where …"/>  a rule
//!         <if cond="<when>"><send event="…"/></if>              a rule with a condition
//!       </transition>
//!     </state>
//!   </parallel>
//! </scxml>
//! ```
//!
//! A model with one machine and no controllers is written flat (the
//! machine's states directly under `<scxml name="Machine">`), which other
//! SCXML tools handle best.

use std::collections::{HashMap, HashSet};

use cascade_core::definition::{TargetMode, TargetSpec};
use cascade_core::model::{StateKind, Target};
use cascade_core::{MachineId, Model, StateId, TransitionId, TriggerId};

use crate::scxml::xml::XmlWriter;
use crate::scxml::{CASCADE_NS, SCXML_NS};
use crate::yaml::selector;

const HEADER: &str = "\
Exported by Cascade (docs/features/interop-and-diff.md).
Each machine is a region of the top-level <parallel>; state ids are Machine.path and
triggers are events named Machine.trigger. Emitted events are <send>s. Each controller
is a region marked cascade:controller: one transition per subscribed event, one <send>
per rule (inside <if> when the rule has a condition). Guards and conditions are
Cascade's free text, not expressions. External sources, declared events, colors,
domains, fields, selectors and bounded flags live in the cascade: namespace.";

const FLAT_HEADER: &str = "\
Exported by Cascade (docs/features/interop-and-diff.md).
One machine: its states are the document's states, with ids Machine.path, and its
triggers are events named Machine.trigger. Emitted events are <send>s. Guards are
Cascade's free text, not expressions. External sources, declared events, colors,
domains, fields and bounded flags live in the cascade: namespace.";

/// Unique XML ids across the document.
struct Ids {
    used: HashSet<String>,
}

impl Ids {
    fn claim(&mut self, base: &str) -> String {
        let mut id = base.to_owned();
        let mut n = 2;
        while !self.used.insert(id.clone()) {
            id = format!("{base}_{n}");
            n += 1;
        }
        id
    }
}

struct Writer<'m> {
    model: &'m Model,
    xml: XmlWriter,
    state_ids: HashMap<StateId, String>,
    outgoing: HashMap<StateId, Vec<TransitionId>>,
}

pub(crate) fn export(model: &Model) -> String {
    let mut ids = Ids { used: HashSet::new() };
    let mut state_ids = HashMap::new();
    for (mid, machine) in model.machines() {
        ids.claim(&machine.name);
        for &s in &model.machine(mid).states {
            state_ids.insert(s, ids.claim(&format!("{}.{}", machine.name, model.state(s).path)));
        }
    }
    let mut outgoing: HashMap<StateId, Vec<TransitionId>> = HashMap::new();
    for (tid, t) in model.transitions() {
        outgoing.entry(t.from).or_default().push(tid);
    }
    let mut w = Writer { model, xml: XmlWriter::new(), state_ids, outgoing };
    let flat = model.machine_count() == 1 && model.controller_count() == 0;
    w.xml.comment(if flat { FLAT_HEADER } else { HEADER });
    let system = model.definition().system.as_ref().map(|s| s.value.clone());
    let mut root_attrs: Vec<(&str, String)> =
        vec![("xmlns", SCXML_NS.to_owned()), ("xmlns:cascade", CASCADE_NS.to_owned()), ("version", "1.0".to_owned())];
    let only = model.machine_ids().next();
    match (flat, only) {
        (true, Some(mid)) => {
            root_attrs.push(("name", model.machine(mid).name.clone()));
            root_attrs.push(("initial", w.state_id(model.machine(mid).initial)));
            if let Some(system) = system {
                root_attrs.push(("cascade:system", system));
            }
            root_attrs.extend(w.machine_attrs(mid));
        }
        _ => {
            if let Some(system) = system {
                root_attrs.push(("name", system));
            }
        }
    }
    let attrs: Vec<(&str, &str)> = root_attrs.iter().map(|(k, v)| (*k, v.as_str())).collect();
    w.xml.start("scxml", &attrs);
    w.declarations();
    match (flat, only) {
        (true, Some(mid)) => w.states(&model.machine(mid).top_states),
        _ => {
            let system_id = ids.claim("system");
            w.xml.start("parallel", &[("id", &system_id)]);
            for mid in model.machine_ids() {
                w.machine_region(mid);
            }
            for (cid, controller) in model.controllers() {
                let id = ids.claim(&controller.name);
                w.controller_region(cid, &id);
            }
            w.xml.end("parallel");
        }
    }
    w.xml.end("scxml");
    w.xml.finish()
}

impl Writer<'_> {
    fn state_id(&self, state: StateId) -> String {
        self.state_ids.get(&state).cloned().unwrap_or_default()
    }

    fn trigger_event(&self, trigger: TriggerId) -> String {
        let t = self.model.trigger(trigger);
        format!("{}.{}", self.model.machine(t.machine).name, t.name)
    }

    fn machine_attrs(&self, mid: MachineId) -> Vec<(&'static str, String)> {
        let m = self.model.machine(mid);
        let mut attrs = Vec::new();
        if let Some(color) = m.color {
            attrs.push(("cascade:color", color.name().to_owned()));
        }
        if let Some(domain) = &m.domain {
            attrs.push(("cascade:domain", domain.clone()));
        }
        if !m.fields.is_empty() {
            attrs.push(("cascade:fields", m.fields.join(" ")));
        }
        attrs
    }

    /// Declared events and external sources, before the states.
    fn declarations(&mut self) {
        let model = self.model;
        if !model.definition().events.is_empty() {
            for (_, event) in model.events().filter(|(_, e)| e.declared) {
                let payload = event.payload.join(" ");
                let mut attrs = vec![("name", event.name.as_str())];
                if !payload.is_empty() {
                    attrs.push(("payload", payload.as_str()));
                }
                self.xml.empty("cascade:event", &attrs);
            }
        }
        for (_, source) in model.externals() {
            let fires: Vec<String> = source.triggers.iter().map(|&t| self.trigger_event(t)).collect();
            self.xml.empty("cascade:source", &[("name", &source.name), ("fires", &fires.join(" "))]);
        }
    }

    fn machine_region(&mut self, mid: MachineId) {
        let machine = self.model.machine(mid);
        let initial = self.state_id(machine.initial);
        let mut attrs: Vec<(&str, String)> = vec![("id", machine.name.clone()), ("initial", initial)];
        attrs.extend(self.machine_attrs(mid));
        let attrs: Vec<(&str, &str)> = attrs.iter().map(|(k, v)| (*k, v.as_str())).collect();
        self.xml.start("state", &attrs);
        self.states(&machine.top_states);
        self.xml.end("state");
    }

    fn states(&mut self, states: &[StateId]) {
        for &s in states {
            self.state(s);
        }
    }

    fn state(&mut self, sid: StateId) {
        let model = self.model;
        let id = self.state_id(sid);
        match &model.state(sid).kind {
            StateKind::Final => self.xml.empty("final", &[("id", &id)]),
            StateKind::History { deep } => {
                self.xml.empty("history", &[("id", &id), ("type", if *deep { "deep" } else { "shallow" })]);
            }
            StateKind::Atomic => {
                let transitions = self.outgoing.get(&sid).cloned().unwrap_or_default();
                if transitions.is_empty() {
                    self.xml.empty("state", &[("id", &id)]);
                } else {
                    self.xml.start("state", &[("id", &id)]);
                    self.transitions(&transitions);
                    self.xml.end("state");
                }
            }
            StateKind::Compound { children, initial } => {
                let initial = self.state_id(*initial);
                self.xml.start("state", &[("id", &id), ("initial", &initial)]);
                let transitions = self.outgoing.get(&sid).cloned().unwrap_or_default();
                self.transitions(&transitions);
                self.states(children);
                self.xml.end("state");
            }
        }
    }

    fn transitions(&mut self, transitions: &[TransitionId]) {
        let model = self.model;
        for &tid in transitions {
            let t = model.transition(tid);
            let event = self.trigger_event(t.trigger);
            let target = self.state_id(t.to);
            let mut attrs = vec![("event", event.as_str()), ("target", target.as_str())];
            if let Some(guard) = &t.guard {
                attrs.push(("cond", guard.as_str()));
            }
            if t.bounded {
                attrs.push(("cascade:bounded", "true"));
            }
            if t.emits.is_empty() {
                self.xml.empty("transition", &attrs);
            } else {
                self.xml.start("transition", &attrs);
                for &e in &t.emits {
                    self.xml.empty("send", &[("event", &model.event(e).name)]);
                }
                self.xml.end("transition");
            }
        }
    }

    fn controller_region(&mut self, cid: cascade_core::ControllerId, id: &str) {
        let model = self.model;
        let controller = model.controller(cid);
        if controller.handlers.is_empty() {
            self.xml.empty("state", &[("id", id), ("cascade:controller", &controller.name)]);
            return;
        }
        self.xml.start("state", &[("id", id), ("cascade:controller", &controller.name)]);
        for &h in &controller.handlers {
            let handler = model.handler(h);
            let event = &model.event(handler.event).name;
            if handler.rules.is_empty() {
                self.xml.empty("transition", &[("event", event)]);
                continue;
            }
            self.xml.start("transition", &[("event", event)]);
            for &r in &handler.rules {
                let rule = model.rule(r);
                let fire = self.trigger_event(rule.trigger);
                let machine = &model.machine(model.trigger(rule.trigger).machine).name;
                let target = target_text(machine, &rule.target);
                let mut attrs = vec![("event", fire.as_str())];
                if let Some(target) = &target {
                    attrs.push(("cascade:target", target.as_str()));
                }
                if rule.bounded {
                    attrs.push(("cascade:bounded", "true"));
                }
                match &rule.condition {
                    Some(when) => {
                        self.xml.start("if", &[("cond", when)]);
                        self.xml.empty("send", &attrs);
                        self.xml.end("if");
                    }
                    None => self.xml.empty("send", &attrs),
                }
            }
            self.xml.end("transition");
        }
        self.xml.end("state");
    }
}

/// The selector text for a rule target, or `None` for "the one instance".
fn target_text(machine: &str, target: &Target) -> Option<String> {
    let (mode, clauses) = match target {
        Target::One { predicates } if predicates.is_empty() => return None,
        Target::One { predicates } => (TargetMode::One, predicates),
        Target::All { predicates } => (TargetMode::All, predicates),
        Target::Spawn { assignments } => (TargetMode::Spawn, assignments),
    };
    Some(selector(&TargetSpec { mode, machine: machine.to_owned(), clauses: clauses.clone() }))
}
