//! The resolved model's element types. Every cross reference is a typed id
//! into the owning [`Model`](crate::Model), and reverse indexes (which
//! transitions accept a trigger, which handlers receive an event, …) are
//! filled in by the resolver so consumers never recompute them.

use crate::color::PaletteColor;
use crate::definition::FieldClause;
use crate::ids::{ControllerId, EventId, ExternalId, HandlerId, MachineId, RuleId, StateId, TransitionId, TriggerId};
use crate::span::SourceSpan;

#[derive(Clone, Debug, PartialEq)]
pub struct Machine {
    pub name: String,
    /// The declared color, if any. Automatic assignment is presentation.
    pub color: Option<PaletteColor>,
    pub domain: Option<String>,
    /// The state an instance starts in (may be compound; enter it via
    /// [`Model::default_entry`](crate::Model::default_entry)).
    pub initial: StateId,
    /// Declared instance fields; empty means "not declared, not checked".
    pub fields: Vec<String>,
    /// Every state of the machine in pre-order (parents before children),
    /// siblings in definition order.
    pub states: Vec<StateId>,
    /// Top-level states in definition order.
    pub top_states: Vec<StateId>,
    pub transitions: Vec<TransitionId>,
    /// Every trigger named for this machine, whether or not a transition
    /// accepts it.
    pub triggers: Vec<TriggerId>,
    /// Span of the machine's name.
    pub span: SourceSpan,
}

/// A state's structure. Only compound states have children, and a compound
/// state always has an initial child.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StateKind {
    Atomic,
    Compound { children: Vec<StateId>, initial: StateId },
    Final,
    History { deep: bool },
}

#[derive(Clone, Debug, PartialEq)]
pub struct State {
    pub machine: MachineId,
    /// Local name, unique among siblings.
    pub name: String,
    /// Dotted path from the machine's top level, e.g. `running.fetching`.
    /// Unique within the machine.
    pub path: String,
    pub parent: Option<StateId>,
    pub kind: StateKind,
    /// 0 for top-level states.
    pub depth: u32,
    pub span: SourceSpan,
}

impl State {
    pub fn children(&self) -> &[StateId] {
        match &self.kind {
            StateKind::Compound { children, .. } => children,
            StateKind::Atomic | StateKind::Final | StateKind::History { .. } => &[],
        }
    }

    pub fn is_compound(&self) -> bool {
        matches!(self.kind, StateKind::Compound { .. })
    }

    pub fn is_final(&self) -> bool {
        matches!(self.kind, StateKind::Final)
    }

    pub fn is_history(&self) -> bool {
        matches!(self.kind, StateKind::History { .. })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Transition {
    pub machine: MachineId,
    pub from: StateId,
    pub to: StateId,
    pub trigger: TriggerId,
    pub guard: Option<String>,
    pub emits: Vec<EventId>,
    pub bounded: bool,
    /// Distinguishes transitions that share `(from, to, trigger)`; 0 for the
    /// first in definition order. Part of the transition's stable key.
    pub ordinal: u32,
    pub span: SourceSpan,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Trigger {
    pub machine: MachineId,
    pub name: String,
    /// Transitions that take this trigger. Empty when the trigger is fired or
    /// exposed but no transition accepts it (an invalid fire).
    pub accepted_by: Vec<TransitionId>,
    /// Rules that fire this trigger.
    pub fired_by: Vec<RuleId>,
    /// External sources that can fire this trigger.
    pub sources: Vec<ExternalId>,
    /// Span of the first mention.
    pub span: SourceSpan,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Event {
    pub name: String,
    /// Declared payload fields; empty when undeclared.
    pub payload: Vec<String>,
    /// Whether the event appears under `events:`.
    pub declared: bool,
    pub emitted_by: Vec<TransitionId>,
    pub handlers: Vec<HandlerId>,
    /// Span of the declaration, or of the first mention when undeclared.
    pub span: SourceSpan,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Controller {
    pub name: String,
    pub handlers: Vec<HandlerId>,
    pub span: SourceSpan,
}

/// One controller's subscription to one event.
#[derive(Clone, Debug, PartialEq)]
pub struct Handler {
    pub controller: ControllerId,
    pub event: EventId,
    pub rules: Vec<RuleId>,
    pub span: SourceSpan,
}

/// Which instance(s) a rule fires on. The machine is always the fired
/// trigger's machine.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Target {
    /// Exactly one instance matching every predicate. No predicates means
    /// "the one instance" (a singleton machine).
    One { predicates: Vec<FieldClause> },
    /// Every instance matching every predicate (fan-out).
    All { predicates: Vec<FieldClause> },
    /// A newly spawned instance with these field values.
    Spawn { assignments: Vec<FieldClause> },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Rule {
    pub controller: ControllerId,
    pub handler: HandlerId,
    pub event: EventId,
    pub trigger: TriggerId,
    pub target: Target,
    /// Free-text condition (`when:`), displayed but never evaluated.
    pub condition: Option<String>,
    pub bounded: bool,
    /// Position within the handler; part of the rule's stable key.
    pub ordinal: u32,
    pub span: SourceSpan,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ExternalSource {
    pub name: String,
    pub triggers: Vec<TriggerId>,
    pub span: SourceSpan,
}
