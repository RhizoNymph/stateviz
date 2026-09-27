//! Controllers, events and external sources for an imported definition.
//!
//! Formats without Cascade's wiring ([`Wiring::Synthesize`]) get:
//!
//! - one routing controller (default `EventRouter`) with a handler per
//!   emitted event that some machine handles, firing that trigger on each
//!   machine the emit reaches: `raise` → the emitter; `sendTo`/`#_id` → the
//!   named machine; `sendParent` and `emit` → the other machines; `emit:`
//!   actions and SCXML `<raise>`/`<send>` → every machine (the regions of a
//!   parallel document share one event queue);
//! - external sources so every trigger has a cause: the environment source
//!   (default `Environment`) fires every named-event trigger no routed emit
//!   covers (and approximated completion triggers), the clock source
//!   (default `Clock`) fires `after_*` triggers, and each invoked actor is a
//!   source firing its `<actor>_done`/`<actor>_error` triggers.
//!
//! Cascade's own SCXML export carries its controllers, declared events and
//! sources as `cascade:` annotations ([`Wiring::Annotated`]); those are
//! restored as written.

use std::collections::HashSet;

use cascade_core::definition::{ControllerDef, EventDef, ExternalDef, HandlerDef, RuleDef, TriggerRef};
use cascade_core::parse::grammar::parse_target;
use cascade_core::{SourceSpan, Spanned};

use crate::error::InteropError;
use crate::import::chart::Route;
use crate::import::lower::{LoweredMachine, TriggerOrigin};
use crate::import::names::{KeyedNames, NameTable, sanitize};
use crate::import::{ImportOptions, NameKind, Warnings};

pub(crate) enum Wiring {
    Synthesize,
    Annotated(Annotations),
}

/// Cascade elements read back from annotated input, with raw names.
#[derive(Debug, Default)]
pub(crate) struct Annotations {
    pub events: Vec<AnnotatedEvent>,
    pub controllers: Vec<AnnotatedController>,
    pub sources: Vec<AnnotatedSource>,
}

#[derive(Debug)]
pub(crate) struct AnnotatedEvent {
    pub name: String,
    pub payload: Vec<String>,
    pub location: String,
}

#[derive(Debug)]
pub(crate) struct AnnotatedController {
    pub name: String,
    pub location: String,
    pub handlers: Vec<AnnotatedHandler>,
}

#[derive(Debug)]
pub(crate) struct AnnotatedHandler {
    pub event: String,
    pub rules: Vec<AnnotatedRule>,
}

#[derive(Debug)]
pub(crate) struct AnnotatedRule {
    pub machine: String,
    pub trigger: String,
    /// Target selector text, as `parse_target` reads it.
    pub target: Option<String>,
    pub when: Option<String>,
    pub bounded: bool,
    pub location: String,
}

#[derive(Debug)]
pub(crate) struct AnnotatedSource {
    pub name: String,
    /// (raw machine, raw trigger) pairs.
    pub triggers: Vec<(String, String)>,
    pub location: String,
}

/// The wiring half of a definition.
pub(crate) struct Wired {
    pub events: Vec<EventDef>,
    pub controllers: Vec<ControllerDef>,
    pub external: Vec<ExternalDef>,
}

pub(crate) fn wire(
    wiring: Wiring,
    machines: &[LoweredMachine],
    options: &ImportOptions,
    warnings: &mut Warnings,
    events: &mut KeyedNames<String>,
    format: &'static str,
) -> Result<Wired, InteropError> {
    match wiring {
        Wiring::Synthesize => Ok(synthesize(machines, options, warnings, events)),
        Wiring::Annotated(annotations) => annotated(annotations, machines, warnings, events, format),
    }
}

fn synthetic(value: impl Into<String>) -> Spanned<String> {
    Spanned::synthetic(value.into())
}

fn trigger_ref(machine: &LoweredMachine, trigger: &str) -> Spanned<TriggerRef> {
    Spanned::synthetic(TriggerRef { machine: machine.def.name.value.clone(), trigger: trigger.to_owned() })
}

fn rule(fire: Spanned<TriggerRef>) -> RuleDef {
    RuleDef { fire, target: None, when: None, bounded: false, span: SourceSpan::unknown() }
}

// --- Synthesized wiring ------------------------------------------------------------

fn synthesize(
    machines: &[LoweredMachine],
    options: &ImportOptions,
    warnings: &mut Warnings,
    events: &mut KeyedNames<String>,
) -> Wired {
    // Event name → (machine index, trigger) fired, in first-seen order.
    let mut routes: Vec<(String, Vec<(usize, String)>)> = Vec::new();
    for (from, machine) in machines.iter().enumerate() {
        for (emit, location) in &machine.emits {
            let receivers: Vec<usize> = match &emit.route {
                Route::Own => vec![from],
                Route::Others => (0..machines.len()).filter(|&i| i != from).collect(),
                Route::All => (0..machines.len()).collect(),
                Route::Machine(name) => {
                    let found = machines.iter().position(|m| &m.raw_name == name || &m.def.name.value == name);
                    if found.is_none() {
                        warnings.ignored(
                            location.as_str(),
                            format!("delivery of `{}` to `{name}`, which is not an imported machine", emit.event),
                        );
                    }
                    found.into_iter().collect()
                }
            };
            let event = events.name(&emit.event, &emit.event, location, warnings);
            let index = match routes.iter().position(|(e, _)| *e == event) {
                Some(i) => i,
                None => {
                    routes.push((event, Vec::new()));
                    routes.len() - 1
                }
            };
            for receiver in receivers {
                if let Some(trigger) = machines[receiver].event_triggers.get(&emit.event) {
                    let fire = (receiver, trigger.clone());
                    if !routes[index].1.contains(&fire) {
                        routes[index].1.push(fire);
                    }
                }
            }
        }
    }

    let fired: HashSet<(usize, String)> = routes.iter().flat_map(|(_, fires)| fires.iter().cloned()).collect();
    let handlers: Vec<HandlerDef> = routes
        .into_iter()
        .filter(|(_, fires)| !fires.is_empty())
        .map(|(event, fires)| HandlerDef {
            event: synthetic(event),
            rules: fires.into_iter().map(|(m, trigger)| rule(trigger_ref(&machines[m], &trigger))).collect(),
            span: SourceSpan::unknown(),
        })
        .collect();
    let mut controllers = Vec::new();
    if !handlers.is_empty() {
        let mut names = NameTable::new(NameKind::Controller);
        let name = names.claim(&options.router_controller, "options.router_controller", warnings);
        controllers.push(ControllerDef { name: synthetic(name), on: handlers, span: SourceSpan::unknown() });
    }

    let mut environment = Vec::new();
    let mut clock = Vec::new();
    let mut actors: Vec<(String, Vec<Spanned<TriggerRef>>)> = Vec::new();
    for (index, machine) in machines.iter().enumerate() {
        for (trigger, origin, _) in &machine.triggers {
            let fire = trigger_ref(machine, trigger);
            match origin {
                TriggerOrigin::Event => {
                    if !fired.contains(&(index, trigger.clone())) {
                        environment.push(fire);
                    }
                }
                TriggerOrigin::Completion => environment.push(fire),
                TriggerOrigin::Delay => clock.push(fire),
                TriggerOrigin::Actor(actor) => match actors.iter_mut().find(|(a, _)| a == actor) {
                    Some((_, triggers)) => triggers.push(fire),
                    None => actors.push((actor.clone(), vec![fire])),
                },
            }
        }
    }
    let mut names = NameTable::new(NameKind::Source);
    let mut external = Vec::new();
    let mut add = |raw: &str, location: &str, triggers: Vec<Spanned<TriggerRef>>, warnings: &mut Warnings| {
        if !triggers.is_empty() {
            let name = names.claim(raw, location, warnings);
            external.push(ExternalDef { name: synthetic(name), triggers, span: SourceSpan::unknown() });
        }
    };
    add(&options.environment_source, "options.environment_source", environment, warnings);
    add(&options.clock_source, "options.clock_source", clock, warnings);
    for (actor, triggers) in actors {
        add(&actor, "invoke", triggers, warnings);
    }

    Wired { events: Vec::new(), controllers, external }
}

// --- Annotated wiring ------------------------------------------------------------------

fn annotated(
    annotations: Annotations,
    machines: &[LoweredMachine],
    warnings: &mut Warnings,
    events: &mut KeyedNames<String>,
    format: &'static str,
) -> Result<Wired, InteropError> {
    let resolve_ref = |machine: &str, trigger: &str, location: &str| -> Result<Spanned<TriggerRef>, InteropError> {
        let Some(m) = machines.iter().find(|m| m.raw_name == machine || m.def.name.value == machine) else {
            return Err(InteropError::Invalid {
                format,
                location: location.to_owned(),
                message: format!("`{machine}.{trigger}` names no machine of this document"),
            });
        };
        let name = m.event_triggers.get(trigger).cloned().unwrap_or_else(|| sanitize(trigger));
        Ok(trigger_ref(m, &name))
    };

    let declared = annotations
        .events
        .iter()
        .map(|e| EventDef {
            name: synthetic(events.name(&e.name, &e.name, &e.location, warnings)),
            payload: e.payload.iter().map(|p| synthetic(sanitize(p))).collect(),
            span: SourceSpan::unknown(),
        })
        .collect();

    let mut controller_names = NameTable::new(NameKind::Controller);
    let mut controllers = Vec::with_capacity(annotations.controllers.len());
    for c in &annotations.controllers {
        let mut handlers = Vec::with_capacity(c.handlers.len());
        for h in &c.handlers {
            let mut rules = Vec::with_capacity(h.rules.len());
            for r in &h.rules {
                let target = match &r.target {
                    None => None,
                    Some(text) => {
                        Some(Spanned::synthetic(parse_target(text).map_err(|reason| InteropError::Invalid {
                            format,
                            location: r.location.clone(),
                            message: format!("invalid target selector `{text}`: {reason}"),
                        })?))
                    }
                };
                rules.push(RuleDef {
                    fire: resolve_ref(&r.machine, &r.trigger, &r.location)?,
                    target,
                    when: r.when.clone().map(Spanned::synthetic),
                    bounded: r.bounded,
                    span: SourceSpan::unknown(),
                });
            }
            handlers.push(HandlerDef {
                event: synthetic(events.name(&h.event, &h.event, &c.location, warnings)),
                rules,
                span: SourceSpan::unknown(),
            });
        }
        controllers.push(ControllerDef {
            name: synthetic(controller_names.claim(&c.name, &c.location, warnings)),
            on: handlers,
            span: SourceSpan::unknown(),
        });
    }

    let mut source_names = NameTable::new(NameKind::Source);
    let mut external = Vec::with_capacity(annotations.sources.len());
    for s in &annotations.sources {
        let triggers = s.triggers.iter().map(|(m, t)| resolve_ref(m, t, &s.location)).collect::<Result<Vec<_>, _>>()?;
        external.push(ExternalDef {
            name: synthetic(source_names.claim(&s.name, &s.location, warnings)),
            triggers,
            span: SourceSpan::unknown(),
        });
    }

    Ok(Wired { events: declared, controllers, external })
}
