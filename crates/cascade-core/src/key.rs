//! Two ways to point at a model element.
//!
//! [`ElementRef`] is an id into one specific [`Model`](crate::Model): cheap,
//! but meaningless once the file is reloaded. [`ElementKey`] is built from
//! names and is stable across reloads, git revisions and imports, so it is
//! what selections, layout pins, view links and diffs are keyed by.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::ids::{ControllerId, EventId, ExternalId, HandlerId, MachineId, RuleId, StateId, TransitionId, TriggerId};
use crate::parse::grammar::is_valid_name;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ElementRef {
    Machine(MachineId),
    State(StateId),
    Transition(TransitionId),
    Trigger(TriggerId),
    Event(EventId),
    Controller(ControllerId),
    Handler(HandlerId),
    Rule(RuleId),
    External(ExternalId),
}

/// What kind of element a key or ref names.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ElementKind {
    Machine,
    State,
    Transition,
    Trigger,
    Event,
    Controller,
    Handler,
    Rule,
    External,
}

impl ElementKind {
    pub const fn prefix(self) -> &'static str {
        match self {
            ElementKind::Machine => "machine",
            ElementKind::State => "state",
            ElementKind::Transition => "transition",
            ElementKind::Trigger => "trigger",
            ElementKind::Event => "event",
            ElementKind::Controller => "controller",
            ElementKind::Handler => "handler",
            ElementKind::Rule => "rule",
            ElementKind::External => "external",
        }
    }
}

impl ElementRef {
    pub const fn kind(self) -> ElementKind {
        match self {
            ElementRef::Machine(_) => ElementKind::Machine,
            ElementRef::State(_) => ElementKind::State,
            ElementRef::Transition(_) => ElementKind::Transition,
            ElementRef::Trigger(_) => ElementKind::Trigger,
            ElementRef::Event(_) => ElementKind::Event,
            ElementRef::Controller(_) => ElementKind::Controller,
            ElementRef::Handler(_) => ElementKind::Handler,
            ElementRef::Rule(_) => ElementKind::Rule,
            ElementRef::External(_) => ElementKind::External,
        }
    }
}

/// A name-based, reload-stable identity for an element.
///
/// String form (used in view links and `cascade.layout.json`):
///
/// | Kind | Form |
/// | --- | --- |
/// | machine | `machine:Order` |
/// | state | `state:Order:pending.waiting` |
/// | transition | `transition:Order:pending->paid@capture_ok` (`#n` suffix when `ordinal > 0`) |
/// | trigger | `trigger:Order.capture_ok` |
/// | event | `event:OrderPaid` |
/// | controller | `controller:Fulfillment` |
/// | handler | `handler:Fulfillment/OrderPaid` |
/// | rule | `rule:Fulfillment/OrderPaid#0` |
/// | external | `external:Clock` |
///
/// Names can only contain letters, digits, `_` and `-`, so the separators
/// are unambiguous.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ElementKey {
    Machine { machine: String },
    State { machine: String, path: String },
    Transition { machine: String, from: String, to: String, trigger: String, ordinal: u32 },
    Trigger { machine: String, trigger: String },
    Event { event: String },
    Controller { controller: String },
    Handler { controller: String, event: String },
    Rule { controller: String, event: String, ordinal: u32 },
    External { source: String },
}

impl ElementKey {
    pub const fn kind(&self) -> ElementKind {
        match self {
            ElementKey::Machine { .. } => ElementKind::Machine,
            ElementKey::State { .. } => ElementKind::State,
            ElementKey::Transition { .. } => ElementKind::Transition,
            ElementKey::Trigger { .. } => ElementKind::Trigger,
            ElementKey::Event { .. } => ElementKind::Event,
            ElementKey::Controller { .. } => ElementKind::Controller,
            ElementKey::Handler { .. } => ElementKind::Handler,
            ElementKey::Rule { .. } => ElementKind::Rule,
            ElementKey::External { .. } => ElementKind::External,
        }
    }

    /// The machine this element belongs to, for elements that belong to one.
    pub fn machine(&self) -> Option<&str> {
        match self {
            ElementKey::Machine { machine }
            | ElementKey::State { machine, .. }
            | ElementKey::Transition { machine, .. }
            | ElementKey::Trigger { machine, .. } => Some(machine),
            ElementKey::Event { .. }
            | ElementKey::Controller { .. }
            | ElementKey::Handler { .. }
            | ElementKey::Rule { .. }
            | ElementKey::External { .. } => None,
        }
    }
}

impl fmt::Display for ElementKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:", self.kind().prefix())?;
        match self {
            ElementKey::Machine { machine } => f.write_str(machine),
            ElementKey::State { machine, path } => write!(f, "{machine}:{path}"),
            ElementKey::Transition { machine, from, to, trigger, ordinal } => {
                write!(f, "{machine}:{from}->{to}@{trigger}")?;
                if *ordinal > 0 {
                    write!(f, "#{ordinal}")?;
                }
                Ok(())
            }
            ElementKey::Trigger { machine, trigger } => write!(f, "{machine}.{trigger}"),
            ElementKey::Event { event } => f.write_str(event),
            ElementKey::Controller { controller } => f.write_str(controller),
            ElementKey::Handler { controller, event } => write!(f, "{controller}/{event}"),
            ElementKey::Rule { controller, event, ordinal } => write!(f, "{controller}/{event}#{ordinal}"),
            ElementKey::External { source } => f.write_str(source),
        }
    }
}

/// The text is not a valid [`ElementKey`].
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("invalid element key `{0}`")]
pub struct InvalidElementKey(pub String);

fn name(s: &str) -> Option<String> {
    is_valid_name(s).then(|| s.to_owned())
}

fn path(s: &str) -> Option<String> {
    s.split('.').all(is_valid_name).then(|| s.to_owned())
}

fn ordinal_suffix(s: &str) -> Option<(&str, u32)> {
    match s.split_once('#') {
        Some((head, n)) => Some((head, n.parse().ok()?)),
        None => Some((s, 0)),
    }
}

fn parse_key(s: &str) -> Option<ElementKey> {
    let (prefix, rest) = s.split_once(':')?;
    Some(match prefix {
        "machine" => ElementKey::Machine { machine: name(rest)? },
        "state" => {
            let (machine, state_path) = rest.split_once(':')?;
            ElementKey::State { machine: name(machine)?, path: path(state_path)? }
        }
        "transition" => {
            let (machine, rest) = rest.split_once(':')?;
            let (from, rest) = rest.split_once("->")?;
            let (to, rest) = rest.split_once('@')?;
            let (trigger, ordinal) = ordinal_suffix(rest)?;
            ElementKey::Transition {
                machine: name(machine)?,
                from: path(from)?,
                to: path(to)?,
                trigger: name(trigger)?,
                ordinal,
            }
        }
        "trigger" => {
            let (machine, trigger) = rest.split_once('.')?;
            ElementKey::Trigger { machine: name(machine)?, trigger: name(trigger)? }
        }
        "event" => ElementKey::Event { event: name(rest)? },
        "controller" => ElementKey::Controller { controller: name(rest)? },
        "handler" => {
            let (controller, event) = rest.split_once('/')?;
            ElementKey::Handler { controller: name(controller)?, event: name(event)? }
        }
        "rule" => {
            let (controller, rest) = rest.split_once('/')?;
            let (event, ordinal) = rest.split_once('#')?;
            ElementKey::Rule { controller: name(controller)?, event: name(event)?, ordinal: ordinal.parse().ok()? }
        }
        "external" => ElementKey::External { source: name(rest)? },
        _ => return None,
    })
}

impl FromStr for ElementKey {
    type Err = InvalidElementKey;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        parse_key(s).ok_or_else(|| InvalidElementKey(s.to_owned()))
    }
}

impl Serialize for ElementKey {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for ElementKey {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        text.parse().map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all_keys() -> Vec<ElementKey> {
        vec![
            ElementKey::Machine { machine: "Order".into() },
            ElementKey::State { machine: "Job".into(), path: "running.fetching".into() },
            ElementKey::Transition {
                machine: "Order".into(),
                from: "pending".into(),
                to: "paid".into(),
                trigger: "capture_ok".into(),
                ordinal: 0,
            },
            ElementKey::Transition {
                machine: "Order".into(),
                from: "a-".into(),
                to: "b.c".into(),
                trigger: "go-now".into(),
                ordinal: 2,
            },
            ElementKey::Trigger { machine: "Shipment".into(), trigger: "start".into() },
            ElementKey::Event { event: "OrderPaid".into() },
            ElementKey::Controller { controller: "Fulfillment".into() },
            ElementKey::Handler { controller: "Fulfillment".into(), event: "OrderPaid".into() },
            ElementKey::Rule { controller: "Fulfillment".into(), event: "OrderPaid".into(), ordinal: 1 },
            ElementKey::External { source: "Clock".into() },
        ]
    }

    #[test]
    fn string_form_round_trips() {
        for key in all_keys() {
            let text = key.to_string();
            assert_eq!(text.parse::<ElementKey>(), Ok(key.clone()), "{text}");
        }
    }

    #[test]
    fn documented_forms() {
        let texts: Vec<String> = all_keys().iter().map(ToString::to_string).collect();
        assert_eq!(
            texts,
            [
                "machine:Order",
                "state:Job:running.fetching",
                "transition:Order:pending->paid@capture_ok",
                "transition:Order:a-->b.c@go-now#2",
                "trigger:Shipment.start",
                "event:OrderPaid",
                "controller:Fulfillment",
                "handler:Fulfillment/OrderPaid",
                "rule:Fulfillment/OrderPaid#1",
                "external:Clock",
            ]
        );
    }

    #[test]
    fn serde_uses_the_string_form() {
        let key = ElementKey::Event { event: "Shipped".into() };
        let json = serde_json::to_string(&key).expect("serializes");
        assert_eq!(json, "\"event:Shipped\"");
        assert_eq!(serde_json::from_str::<ElementKey>(&json).expect("deserializes"), key);
    }

    #[test]
    fn rejects_malformed_keys() {
        for text in [
            "",
            "machine",
            "machine:",
            "planet:Earth",
            "state:Order",
            "transition:Order:a->b",
            "transition:Order:a@go",
            "trigger:Order",
            "rule:C/E",
            "rule:C/E#x",
            "event:has space",
            "state:Order:a..b",
        ] {
            assert!(text.parse::<ElementKey>().is_err(), "should reject {text:?}");
        }
    }
}
