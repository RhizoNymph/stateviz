//! The definition document: a spanned, unresolved mirror of the YAML file.
//!
//! A [`Definition`] is what the parser produces and what importers build. Names
//! are still strings here; [`crate::resolve`] turns a definition into a
//! [`Model`](crate::Model) with typed ids, reporting every dangling reference.
//! Small embedded grammars (trigger references, target selectors, state
//! kinds, expressions, guards, field types, durations) are already parsed, so
//! a `Definition` is always syntactically valid.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::color::PaletteColor;
use crate::expr::{Assignment, Condition, Duration, Expr, FieldDecl, FieldType, Guard};
use crate::span::{SourceSpan, Spanned};

/// A whole system definition, in document order.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Definition {
    /// Optional display name for the system (`system:` key).
    pub system: Option<Spanned<String>>,
    pub machines: Vec<MachineDef>,
    /// Explicit event declarations (`events:` key). When this is non-empty,
    /// every emitted or subscribed event must be declared here.
    pub events: Vec<EventDef>,
    pub controllers: Vec<ControllerDef>,
    pub external: Vec<ExternalDef>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MachineDef {
    pub name: Spanned<String>,
    pub color: Option<Spanned<PaletteColor>>,
    /// Groups machines that share a hue once there are more than eight.
    pub domain: Option<Spanned<String>>,
    /// Top-level initial state. Defaults to the first state (SCXML rule).
    pub initial: Option<Spanned<String>>,
    /// Declared instance fields, e.g. `orderId`. Optional; when present,
    /// expressions reading `self.x` are checked against it.
    pub fields: Vec<FieldDef>,
    pub states: Vec<StateDef>,
    pub transitions: Vec<TransitionDef>,
    /// Span of the whole machine body.
    pub span: SourceSpan,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum StateKindDef {
    Normal,
    Final,
    /// Shallow history pseudo-state.
    History,
    /// Deep history pseudo-state.
    DeepHistory,
}

impl StateKindDef {
    pub const fn name(self) -> &'static str {
        match self {
            StateKindDef::Normal => "normal",
            StateKindDef::Final => "final",
            StateKindDef::History => "history",
            StateKindDef::DeepHistory => "deep-history",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StateDef {
    pub name: Spanned<String>,
    pub kind: Spanned<StateKindDef>,
    /// Initial child; only meaningful when `states` is non-empty. Defaults to
    /// the first child.
    pub initial: Option<Spanned<String>>,
    pub states: Vec<StateDef>,
    pub span: SourceSpan,
}

/// A declared field (instance field or payload field). `ty` is `None` when
/// the list form was used (`fields: [a, b]`, untyped); the mapping form
/// (`fields: { a: number }`) gives every field a type.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FieldDef {
    pub name: Spanned<String>,
    pub ty: Option<Spanned<FieldType>>,
}

impl FieldDef {
    pub fn untyped(name: impl Into<String>) -> Self {
        Self { name: Spanned::synthetic(name.into()), ty: None }
    }

    pub fn typed(name: impl Into<String>, ty: FieldType) -> Self {
        Self { name: Spanned::synthetic(name.into()), ty: Some(Spanned::synthetic(ty)) }
    }

    /// The declaration the model holds: untyped fields are `any`.
    pub fn decl(&self) -> FieldDecl {
        FieldDecl::new(self.name.value.clone(), self.ty.as_ref().map(|t| t.value.clone()).unwrap_or_default())
    }
}

impl From<&str> for FieldDef {
    fn from(name: &str) -> Self {
        Self::untyped(name)
    }
}

impl From<String> for FieldDef {
    fn from(name: String) -> Self {
        Self::untyped(name)
    }
}

/// Whether a field list is written in the typed (mapping) form: true when
/// any field has a type.
pub fn fields_are_typed(fields: &[FieldDef]) -> bool {
    fields.iter().any(|f| f.ty.is_some())
}

/// What makes a transition fire (`on:` or `after:`).
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TriggerSpec {
    /// `on: name`: a command, fired by controller rules and external sources.
    Command(Spanned<String>),
    /// `on: { event: E, where: … }`: a subscription; every live instance is
    /// offered each delivered `E`. The filter is in
    /// [`ExprContext::Filter`](crate::expr::ExprContext::Filter).
    Event { event: Spanned<String>, filter: Option<Spanned<Expr>> },
    /// `after: 30s`: fires that long after the instance enters the source.
    After(Spanned<Duration>),
}

impl TriggerSpec {
    pub fn command(name: impl Into<String>) -> Self {
        TriggerSpec::Command(Spanned::synthetic(name.into()))
    }

    /// The trigger's name: the command's name, the subscribed event's name,
    /// or `after-<duration>`.
    pub fn name(&self) -> String {
        match self {
            TriggerSpec::Command(name) => name.value.clone(),
            TriggerSpec::Event { event, .. } => event.value.clone(),
            TriggerSpec::After(delay) => timer_trigger_name(delay.value),
        }
    }

    /// The span of the trigger's text (the name, event or duration).
    pub fn span(&self) -> SourceSpan {
        match self {
            TriggerSpec::Command(name) => name.span,
            TriggerSpec::Event { event, .. } => event.span,
            TriggerSpec::After(delay) => delay.span,
        }
    }

    /// The command's name, when this is a command.
    pub fn command_name(&self) -> Option<&str> {
        match self {
            TriggerSpec::Command(name) => Some(&name.value),
            TriggerSpec::Event { .. } | TriggerSpec::After(_) => None,
        }
    }
}

/// The name of the trigger of an `after: <delay>` transition.
pub fn timer_trigger_name(delay: Duration) -> String {
    format!("after-{delay}")
}

/// One `emits:` entry: an event, optionally with payload assignments.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct EmitDef {
    pub event: Spanned<String>,
    /// `with: { k: v }`, applied on top of the default payload.
    pub with: Vec<Spanned<Assignment>>,
}

impl EmitDef {
    pub fn plain(event: impl Into<String>) -> Self {
        Self { event: Spanned::synthetic(event.into()), with: Vec::new() }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TransitionDef {
    /// One transition per source state; `from: [a, b]` expands to two.
    pub from: Vec<Spanned<String>>,
    pub to: Spanned<String>,
    /// `on:` (a command or a subscription) or `after:` (a timer).
    pub trigger: TriggerSpec,
    pub guard: Option<Spanned<Guard>>,
    /// `sets: { f: v }`: instance fields assigned when the transition is
    /// taken, after the guard and before the emits.
    pub sets: Vec<Spanned<Assignment>>,
    pub emits: Vec<EmitDef>,
    /// Marks cascade cycles through this transition as bounded (retry loops).
    pub bounded: bool,
    pub span: SourceSpan,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EventDef {
    pub name: Spanned<String>,
    pub payload: Vec<FieldDef>,
    pub span: SourceSpan,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ControllerDef {
    pub name: Spanned<String>,
    /// Subscriptions in document order.
    pub on: Vec<HandlerDef>,
    pub span: SourceSpan,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HandlerDef {
    pub event: Spanned<String>,
    pub rules: Vec<RuleDef>,
    pub span: SourceSpan,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RuleDef {
    pub fire: Spanned<TriggerRef>,
    /// Defaults to "the one instance of the fired machine".
    pub target: Option<Spanned<TargetSpec>>,
    /// `when:`: the rule fires only when the condition holds.
    pub when: Option<Spanned<Condition>>,
    /// Marks cascade cycles through this rule as bounded (retry loops).
    pub bounded: bool,
    pub span: SourceSpan,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExternalDef {
    pub name: Spanned<String>,
    pub triggers: Vec<Spanned<TriggerRef>>,
    pub span: SourceSpan,
}

/// `Machine.trigger`, as used by `fire:` and external sources.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TriggerRef {
    pub machine: String,
    pub trigger: String,
}

impl fmt::Display for TriggerRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}", self.machine, self.trigger)
    }
}

/// How a rule picks the instance(s) it fires on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TargetMode {
    /// `Machine where …`: exactly one matching instance.
    One,
    /// `all Machine where …`: every matching instance (fan-out).
    All,
    /// `new Machine with …`: spawn a new instance, then fire on it.
    Spawn,
}

/// A parsed target selector, e.g. `Shipment where orderId == event.orderId`.
/// Filters are in [`ExprContext::Filter`](crate::expr::ExprContext::Filter):
/// a bare name on the left of a test is the candidate's field.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TargetSpec {
    /// `Machine [where filter]`: exactly one matching instance; no filter
    /// means the one instance of a singleton.
    One { machine: String, filter: Option<Expr> },
    /// `all Machine [where filter]`: every matching instance.
    All { machine: String, filter: Option<Expr> },
    /// `new Machine [with f = v, …]`: a new instance with these fields.
    Spawn { machine: String, assignments: Vec<Assignment> },
}

impl TargetSpec {
    pub fn mode(&self) -> TargetMode {
        match self {
            TargetSpec::One { .. } => TargetMode::One,
            TargetSpec::All { .. } => TargetMode::All,
            TargetSpec::Spawn { .. } => TargetMode::Spawn,
        }
    }

    pub fn machine(&self) -> &str {
        match self {
            TargetSpec::One { machine, .. } | TargetSpec::All { machine, .. } | TargetSpec::Spawn { machine, .. } => {
                machine
            }
        }
    }

    pub fn machine_mut(&mut self) -> &mut String {
        match self {
            TargetSpec::One { machine, .. } | TargetSpec::All { machine, .. } | TargetSpec::Spawn { machine, .. } => {
                machine
            }
        }
    }

    /// The filter of a `One`/`All` selector.
    pub fn filter(&self) -> Option<&Expr> {
        match self {
            TargetSpec::One { filter, .. } | TargetSpec::All { filter, .. } => filter.as_ref(),
            TargetSpec::Spawn { .. } => None,
        }
    }
}

impl fmt::Display for TargetSpec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TargetSpec::One { machine, filter } | TargetSpec::All { machine, filter } => {
                if self.mode() == TargetMode::All {
                    f.write_str("all ")?;
                }
                f.write_str(machine)?;
                if let Some(filter) = filter {
                    write!(f, " where {}", filter.display_filter())?;
                }
                Ok(())
            }
            TargetSpec::Spawn { machine, assignments } => {
                write!(f, "new {machine}")?;
                for (i, assignment) in assignments.iter().enumerate() {
                    f.write_str(if i == 0 { " with " } else { ", " })?;
                    write!(f, "{assignment}")?;
                }
                Ok(())
            }
        }
    }
}
