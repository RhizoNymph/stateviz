//! The definition document: a spanned, unresolved mirror of the YAML file.
//!
//! A [`Definition`] is what the parser produces and what importers build. Names
//! are still strings here; [`crate::resolve`] turns a definition into a
//! [`Model`](crate::Model) with typed ids, reporting every dangling reference.
//! Small embedded grammars (trigger references, target selectors, state
//! kinds) are already parsed, so a `Definition` is always syntactically valid.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::color::PaletteColor;
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
    /// Instance fields that target selectors may match on, e.g. `orderId`.
    /// Optional; when present, selectors are checked against it.
    pub fields: Vec<Spanned<String>>,
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

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TransitionDef {
    /// One transition per source state; `from: [a, b]` expands to two.
    pub from: Vec<Spanned<String>>,
    pub to: Spanned<String>,
    /// The trigger name (`on:` key).
    pub on: Spanned<String>,
    /// Free-text guard; displayed, never evaluated.
    pub guard: Option<Spanned<String>>,
    pub emits: Vec<Spanned<String>>,
    /// Marks cascade cycles through this transition as bounded (retry loops).
    pub bounded: bool,
    pub span: SourceSpan,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EventDef {
    pub name: Spanned<String>,
    pub payload: Vec<Spanned<String>>,
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
    /// Free-text condition (`when:` key); displayed, never evaluated.
    pub when: Option<Spanned<String>>,
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
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TargetSpec {
    pub mode: TargetMode,
    pub machine: String,
    /// `where` predicates for `One`/`All` (joined by `and`), `with`
    /// assignments for `Spawn`.
    pub clauses: Vec<FieldClause>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FieldClause {
    pub field: String,
    pub value: ValueExpr,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ValueExpr {
    /// `event.<field>`: a payload field of the event being handled.
    EventField(String),
    /// A literal, stored as written (quotes removed).
    Literal(String),
}

impl fmt::Display for ValueExpr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ValueExpr::EventField(field) => write!(f, "event.{field}"),
            ValueExpr::Literal(lit) => {
                if lit.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '-' || c == '.') && !lit.is_empty() {
                    f.write_str(lit)
                } else {
                    write!(f, "{lit:?}")
                }
            }
        }
    }
}

impl fmt::Display for TargetSpec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.mode {
            TargetMode::One => {}
            TargetMode::All => f.write_str("all ")?,
            TargetMode::Spawn => f.write_str("new ")?,
        }
        f.write_str(&self.machine)?;
        for (i, clause) in self.clauses.iter().enumerate() {
            let (lead, joiner, op) = match self.mode {
                TargetMode::Spawn => (" with ", ", ", "="),
                TargetMode::One | TargetMode::All => (" where ", " and ", "=="),
            };
            f.write_str(if i == 0 { lead } else { joiner })?;
            write!(f, "{} {op} {}", clause.field, clause.value)?;
        }
        Ok(())
    }
}
