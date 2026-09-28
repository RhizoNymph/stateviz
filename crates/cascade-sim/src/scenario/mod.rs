//! Scenario files: the instances present when a run starts and an ordered
//! list of external triggers to feed the simulator.
//!
//! ```yaml
//! scenario: happy path
//! instances:
//!   o1: { machine: Order, fields: { orderId: "1" } }
//!   s1: { machine: Shipment, fields: { orderId: "1" }, state: idle }
//!   log: Log                                  # shorthand for { machine: Log }
//! steps:
//!   - { source: Customer, fire: Order.submit, target: o1 }
//!   - { source: PaymentGateway, fire: Order.capture_ok, target: o1, payload: { amount: "42" } }
//!   - { source: Clock, fire: Order.timeout, target: o1, timing: immediate }
//!   - step                                    # deliver the queue's head
//!   - { step: 1 }                             # deliver the item behind it first
//!   - run                                     # deliver until the queue is empty
//!   - { create: o2, machine: Order, fields: { orderId: "2" } }
//!   - { remove: o2 }
//! end: pause                                  # optional: leave the queue as it is
//! ```
//!
//! [`parse_scenario`] turns text into a spanned [`Scenario`] (names are still
//! strings); [`validate`] checks it against a [`Model`](cascade_core::Model)
//! and produces a [`ResolvedScenario`] with typed ids, which is what the
//! simulator runs. [`scenario_to_yaml`] writes a scenario back out.

mod file;
mod node;
mod parse;
mod validate;
mod write;

use std::collections::BTreeMap;

use cascade_core::definition::TriggerRef;
use cascade_core::span::{SourceSpan, Spanned};

pub use file::{DiscoverError, ScenarioFileError, discover_scenarios, load_scenario_file};
pub use parse::parse_scenario;
pub(crate) use validate::{ResolvedEntry, ResolvedInstance, ResolvedStep, StepTarget, lookup_state, spawn_prefix};
pub use validate::{ResolvedScenario, validate};
pub use write::scenario_to_yaml;

/// Event payloads and instance fields: string values by name.
pub type Payload = BTreeMap<String, String>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Scenario {
    /// Display name (`scenario:` key), also used by view links.
    pub name: String,
    /// Instances in declaration order.
    pub instances: Vec<InstanceDecl>,
    /// External triggers in the order they happen. Each carries the
    /// [`Directive`]s written before it; see [`Scenario::entries`] for the
    /// whole list in file order.
    pub steps: Vec<Step>,
    /// Directives written after the last external trigger.
    pub trailing: Vec<Directive>,
    /// What happens to the queue after the last entry.
    pub end: ScenarioEnd,
}

impl Scenario {
    /// Every entry of `steps:` in file order: external fires and directives.
    pub fn entries(&self) -> impl Iterator<Item = ScenarioEntry<'_>> + '_ {
        self.steps
            .iter()
            .flat_map(|step| step.before.iter().map(ScenarioEntry::Directive).chain([ScenarioEntry::Fire(step)]))
            .chain(self.trailing.iter().map(ScenarioEntry::Directive))
    }
}

/// One entry of a scenario's `steps:` list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScenarioEntry<'a> {
    Fire(&'a Step),
    Directive(&'a Directive),
}

/// A `steps:` entry that is not an external trigger: it drives the queue by
/// hand or changes which instances exist. Saved play sessions use these to
/// reproduce exactly what the player did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Directive {
    /// `- step` delivers the queue's head; `- { step: n }` delivers the item
    /// at position `n` (0 is the head) ahead of the ones before it.
    Deliver { choice: Option<u32>, span: SourceSpan },
    /// `- run`: deliver queue heads until the queue is empty.
    Run { span: SourceSpan },
    /// `- { create: name, machine: M, fields: {…}, state: s }`: an instance
    /// that appears mid-run. The declaration's span covers the whole entry.
    Create(InstanceDecl),
    /// `- { remove: name }`: the instance leaves; fires queued for it are
    /// discarded. Its name is never reused.
    Remove { name: Spanned<String>, span: SourceSpan },
}

impl Directive {
    /// The whole entry.
    pub fn span(&self) -> SourceSpan {
        match self {
            Directive::Deliver { span, .. } | Directive::Run { span } | Directive::Remove { span, .. } => *span,
            Directive::Create(decl) => decl.span,
        }
    }
}

/// What happens after a scenario's last entry.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ScenarioEnd {
    /// Deliver queue items until the queue is empty (`end: drain`, the
    /// default).
    #[default]
    Drain,
    /// Stop with whatever is still queued (`end: pause`), as a play session
    /// saved mid-cascade does.
    Pause,
}

impl ScenarioEnd {
    /// The spelling used in scenario files.
    pub const fn name(self) -> &'static str {
        match self {
            ScenarioEnd::Drain => "drain",
            ScenarioEnd::Pause => "pause",
        }
    }
}

/// One machine instance present when the scenario starts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstanceDecl {
    /// Unique within the scenario, e.g. `o1`.
    pub name: Spanned<String>,
    pub machine: Spanned<String>,
    /// Field values target selectors match against, e.g. `orderId: 42`.
    pub fields: ValueMap,
    /// Starting state (a path or a unique local name). The machine's initial
    /// state when absent. A compound state is entered by default entry.
    pub state: Option<Spanned<String>>,
    /// The whole declaration.
    pub span: SourceSpan,
}

/// When a step's trigger is delivered relative to the cascade before it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum StepTiming {
    /// Wait until the event queue has drained, then deliver.
    #[default]
    AfterQuiescence,
    /// Deliver right after the previous step, before the cascade it queued
    /// is processed, so the trigger interleaves with that cascade.
    Immediate,
}

impl StepTiming {
    /// The spelling used in scenario files.
    pub const fn name(self) -> &'static str {
        match self {
            StepTiming::AfterQuiescence => "after-quiescence",
            StepTiming::Immediate => "immediate",
        }
    }
}

/// An external source fires a trigger at an instance.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Step {
    pub source: Spanned<String>,
    pub fire: Spanned<TriggerRef>,
    /// Instance name. When absent the fired machine must have exactly one
    /// instance when the step runs (a singleton).
    pub target: Option<Spanned<String>>,
    /// Laid over the target's fields to form the payload of events the
    /// resulting transition emits.
    pub payload: ValueMap,
    pub timing: StepTiming,
    /// The whole step.
    pub span: SourceSpan,
    /// Directives written between the previous external trigger (or the
    /// start of `steps:`) and this one, in order. They run before this
    /// step's timing applies.
    pub before: Vec<Directive>,
}

/// A string map whose entries remember where their key and value were
/// written. Keys are unique; inserting an existing key replaces its entry.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ValueMap {
    entries: BTreeMap<String, ValueEntry>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValueEntry {
    pub key_span: SourceSpan,
    pub value: Spanned<String>,
}

impl ValueMap {
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert an entry, returning the one it replaced.
    pub fn insert(&mut self, key: Spanned<String>, value: Spanned<String>) -> Option<ValueEntry> {
        self.entries.insert(key.value, ValueEntry { key_span: key.span, value })
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.entries.get(key).map(|e| e.value.as_str())
    }

    pub fn entry(&self, key: &str) -> Option<&ValueEntry> {
        self.entries.get(key)
    }

    /// Entries sorted by key.
    pub fn iter(&self) -> impl ExactSizeIterator<Item = (&str, &ValueEntry)> + '_ {
        self.entries.iter().map(|(k, v)| (k.as_str(), v))
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The plain key → value map.
    pub fn to_payload(&self) -> Payload {
        self.entries.iter().map(|(k, v)| (k.clone(), v.value.value.clone())).collect()
    }
}

/// Build a map without source positions, for scenarios made in code.
impl<K: Into<String>, V: Into<String>> FromIterator<(K, V)> for ValueMap {
    fn from_iter<I: IntoIterator<Item = (K, V)>>(iter: I) -> Self {
        let mut map = ValueMap::new();
        for (k, v) in iter {
            map.insert(Spanned::synthetic(k.into()), Spanned::synthetic(v.into()));
        }
        map
    }
}
