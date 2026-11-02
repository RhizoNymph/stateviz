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

use cascade_core::expr::{Assignment, Duration, FieldDecl, FieldType, Guard};
use cascade_core::model::{StateKind, Target, TriggerKind};
use cascade_core::{MachineId, Model, StateId, TransitionId, TriggerId};

use crate::scxml::xml::XmlWriter;
use crate::scxml::{CASCADE_NS, SCXML_NS};
use crate::yaml::{selector, target_spec};

const HEADER: &str = "\
Exported by Cascade (docs/features/interop-and-diff.md).
Each machine is a region of the top-level <parallel>; state ids are Machine.path and
triggers are events named Machine.trigger. Emitted events are <send>s. Each controller
is a region marked cascade:controller: one transition per subscribed event, one <send>
per rule (inside <if> when the rule has a condition). Guards and conditions are
Cascade's own syntax (expressions, ?choices, else), not ECMAScript. Timers are delayed
<send>s armed on entry and cancelled on exit; subscriptions are transitions on the
event itself; sets are <assign>s and emit payloads <param>s. External sources, declared
events, colors, domains, fields, selectors and bounded flags live in the cascade: namespace.";

const FLAT_HEADER: &str = "\
Exported by Cascade (docs/features/interop-and-diff.md).
One machine: its states are the document's states, with ids Machine.path, and its
triggers are events named Machine.trigger. Emitted events are <send>s. Guards are
Cascade's own syntax (expressions, ?choices, else), not ECMAScript. Timers are delayed
<send>s armed on entry and cancelled on exit; sets are <assign>s and emit payloads
<param>s. External sources, declared events, colors, domains, fields and bounded flags
live in the cascade: namespace.";

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
        (true, Some(mid)) => {
            w.typed_fields(&model.machine(mid).fields);
            w.states(&model.machine(mid).top_states);
        }
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
        if !m.fields.is_empty() && !typed(&m.fields) {
            let names: Vec<&str> = m.fields.iter().map(|f| f.name.as_str()).collect();
            attrs.push(("cascade:fields", names.join(" ")));
        }
        attrs
    }

    /// Typed fields (or a typed payload) as `<cascade:field name type/>`
    /// children; untyped ones are a `cascade:fields`/`payload` attribute.
    fn typed_fields(&mut self, fields: &[FieldDecl]) {
        if !typed(fields) {
            return;
        }
        for field in fields {
            self.xml.empty("cascade:field", &[("name", &field.name), ("type", &field.ty.to_string())]);
        }
    }

    /// Declared events and external sources, before the states.
    fn declarations(&mut self) {
        let model = self.model;
        if !model.definition().events.is_empty() {
            for (_, event) in model.events().filter(|(_, e)| e.declared) {
                let payload = event.payload.iter().map(|f| f.name.as_str()).collect::<Vec<_>>().join(" ");
                let mut attrs = vec![("name", event.name.as_str())];
                if typed(&event.payload) {
                    self.xml.start("cascade:event", &attrs);
                    self.typed_fields(&event.payload);
                    self.xml.end("cascade:event");
                    continue;
                }
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
        self.typed_fields(&machine.fields);
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
                    self.timers(&id, &transitions);
                    self.transitions(&transitions);
                    self.xml.end("state");
                }
            }
            StateKind::Compound { children, initial } => {
                let initial = self.state_id(*initial);
                self.xml.start("state", &[("id", &id), ("initial", &initial)]);
                let transitions = self.outgoing.get(&sid).cloned().unwrap_or_default();
                self.timers(&id, &transitions);
                self.transitions(&transitions);
                self.states(children);
                self.xml.end("state");
            }
        }
    }

    /// The timers a state arms: one delayed `<send>` per timer trigger on
    /// entry, cancelled on exit. Its id is `<state id>.<trigger>`.
    fn timers(&mut self, state_id: &str, transitions: &[TransitionId]) {
        let model = self.model;
        let mut timers: Vec<(TriggerId, Duration)> = Vec::new();
        for &tid in transitions {
            let trigger = model.transition(tid).trigger;
            if let TriggerKind::Timer(delay) = model.trigger(trigger).kind
                && !timers.iter().any(|(t, _)| *t == trigger)
            {
                timers.push((trigger, delay));
            }
        }
        if timers.is_empty() {
            return;
        }
        let sends: Vec<(String, String, String)> = timers
            .iter()
            .map(|&(trigger, delay)| {
                (self.trigger_event(trigger), scxml_delay(delay), format!("{state_id}.{}", model.trigger(trigger).name))
            })
            .collect();
        self.xml.start("onentry", &[]);
        for (event, delay, id) in &sends {
            self.xml.empty("send", &[("event", event), ("delay", delay), ("id", id)]);
        }
        self.xml.end("onentry");
        self.xml.start("onexit", &[]);
        for (_, _, id) in &sends {
            self.xml.empty("cancel", &[("sendid", id)]);
        }
        self.xml.end("onexit");
    }

    fn transitions(&mut self, transitions: &[TransitionId]) {
        let model = self.model;
        for &tid in transitions {
            let t = model.transition(tid);
            let target = self.state_id(t.to);
            let guard = t.guard.as_ref().map(ToString::to_string);
            let (event, cond, extra) = match model.trigger(t.trigger).kind {
                // A subscription: the event itself; `cond` holds the filter
                // and the guard for SCXML engines, the cascade: attributes
                // hold them apart for the way back.
                TriggerKind::Event(e) => {
                    let filter = t.filter.as_ref();
                    let mut extra = Vec::new();
                    if let Some(filter) = filter {
                        extra.push(("cascade:where", filter.display_filter().to_string()));
                        if let Some(guard) = &guard {
                            extra.push(("cascade:guard", guard.clone()));
                        }
                    }
                    let cond = match (filter, &t.guard) {
                        (Some(f), Some(Guard::Expr(g))) => {
                            Some(cascade_core::expr::Expr::And(vec![f.clone(), g.clone()]).to_string())
                        }
                        (Some(f), _) => Some(f.to_string()),
                        (None, _) => guard.clone(),
                    };
                    (model.event(e).name.clone(), cond, extra)
                }
                TriggerKind::Command | TriggerKind::Timer(_) => {
                    (self.trigger_event(t.trigger), guard.clone(), Vec::new())
                }
            };
            if let Some(Guard::Choice(label)) = &t.guard {
                self.xml.comment(&format!(
                    "?{label}: a choice the data does not decide (a scenario picks it); SCXML engines see it as an opaque cond"
                ));
            }
            let mut attrs = vec![("event", event.as_str()), ("target", target.as_str())];
            if let Some(cond) = &cond {
                attrs.push(("cond", cond.as_str()));
            }
            attrs.extend(extra.iter().map(|(k, v)| (*k, v.as_str())));
            if t.bounded {
                attrs.push(("cascade:bounded", "true"));
            }
            if t.emits.is_empty() && t.sets.is_empty() {
                self.xml.empty("transition", &attrs);
                continue;
            }
            self.xml.start("transition", &attrs);
            for a in &t.sets {
                self.xml.empty("assign", &[("location", &a.field), ("expr", &a.value.to_string())]);
            }
            for e in &t.emits {
                let name = &model.event(e.event).name;
                if e.with.is_empty() {
                    self.xml.empty("send", &[("event", name)]);
                } else {
                    self.xml.start("send", &[("event", name)]);
                    self.params(&e.with);
                    self.xml.end("send");
                }
            }
            self.xml.end("transition");
        }
    }

    fn params(&mut self, with: &[Assignment]) {
        for a in with {
            self.xml.empty("param", &[("name", &a.field), ("expr", &a.value.to_string())]);
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
                        self.xml.start("if", &[("cond", &when.to_string())]);
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

/// Whether a field list is written typed (any field with a type other
/// than `any`, or a list the definition wrote as a mapping).
fn typed(fields: &[FieldDecl]) -> bool {
    fields.iter().any(|f| f.ty != FieldType::Any)
}

/// An SCXML delay (CSS2 time: seconds or milliseconds).
fn scxml_delay(delay: Duration) -> String {
    let ms = delay.millis();
    if ms.is_multiple_of(1000) { format!("{}s", ms / 1000) } else { format!("{ms}ms") }
}

/// The selector text for a rule target, or `None` for "the one instance".
fn target_text(machine: &str, target: &Target) -> Option<String> {
    if matches!(target, Target::One { filter: None }) {
        return None;
    }
    Some(selector(&target_spec(machine, target)))
}
