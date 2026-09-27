//! Scenario diagnostics and simulator errors.

use std::fmt;

use cascade_core::error::Expected;
use cascade_core::span::SourceSpan;

/// Every problem found in a scenario file, sorted by source position. Parsing
/// and validation collect as many problems as they can rather than stopping
/// at the first.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub struct ScenarioError {
    pub diagnostics: Vec<ScenarioDiagnostic>,
}

impl ScenarioError {
    pub fn single(kind: ScenarioErrorKind, span: SourceSpan) -> Self {
        Self { diagnostics: vec![ScenarioDiagnostic { kind, span }] }
    }

    /// Sort `diagnostics` by position; `Ok(())` when there are none.
    pub(crate) fn from_list(mut diagnostics: Vec<ScenarioDiagnostic>) -> Result<(), Self> {
        if diagnostics.is_empty() {
            return Ok(());
        }
        diagnostics.sort_by_key(|d| d.span);
        Err(Self { diagnostics })
    }
}

impl fmt::Display for ScenarioError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, diagnostic) in self.diagnostics.iter().enumerate() {
            if i > 0 {
                f.write_str("\n")?;
            }
            write!(f, "{diagnostic}")?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScenarioDiagnostic {
    pub kind: ScenarioErrorKind,
    pub span: SourceSpan,
}

impl fmt::Display for ScenarioDiagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.span, self.kind)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ScenarioErrorKind {
    // --- YAML shape -------------------------------------------------------
    #[error("YAML syntax error: {message}")]
    YamlSyntax { message: String },
    #[error("the scenario file contains no YAML document")]
    EmptyDocument,
    #[error("the scenario file contains more than one YAML document")]
    MultipleDocuments,
    #[error("{context}: expected {expected}")]
    WrongType { context: String, expected: Expected },
    #[error("{context}: unknown key `{key}`")]
    UnknownKey { context: String, key: String },
    #[error("{context}: missing required key `{key}`")]
    MissingKey { context: String, key: String },
    #[error(
        "{context}: `{name}` is not a valid name (use letters, digits, `_` and `-`, starting with a letter or `_`)"
    )]
    InvalidName { context: String, name: String },
    #[error("`{text}` is not a trigger reference; expected `Machine.trigger`")]
    InvalidTriggerRef { text: String },
    #[error("unknown timing `{text}`; expected `immediate` or `after-quiescence`")]
    UnknownTiming { text: String },

    // --- Against the model --------------------------------------------------
    #[error("instance `{name}` is declared more than once")]
    DuplicateInstance { name: String },
    #[error("unknown machine `{name}`")]
    UnknownMachine { name: String },
    #[error("machine `{machine}` declares no field `{field}` (declared: {})", declared.join(", "))]
    UnknownField { machine: String, field: String, declared: Vec<String> },
    #[error("machine `{machine}` has no state `{name}`")]
    UnknownState { machine: String, name: String },
    #[error("state name `{name}` in machine `{machine}` is ambiguous; use one of: {}", candidates.join(", "))]
    AmbiguousState { machine: String, name: String, candidates: Vec<String> },
    #[error("an instance cannot start in history state `{state}` of machine `{machine}`")]
    HistoryStart { machine: String, state: String },
    #[error("unknown external source `{name}`")]
    UnknownSource { name: String },
    #[error("machine `{machine}` has no trigger `{trigger}`")]
    UnknownTrigger { machine: String, trigger: String },
    #[error("source `{source_name}` cannot fire `{trigger}`")]
    SourceCannotFire { source_name: String, trigger: String },
    #[error("unknown instance `{name}`")]
    UnknownInstance { name: String },
    #[error("the step fires `{fire}` but instance `{instance}` is a `{machine}`")]
    TargetMachineMismatch { instance: String, machine: String, fire: String },
    #[error("the step names no target and there is no `{machine}` instance")]
    NoInstance { machine: String },
    #[error(
        "the step names no target and there are several `{machine}` instances ({}); add `target:`",
        candidates.join(", ")
    )]
    AmbiguousInstance { machine: String, candidates: Vec<String> },
}

/// The simulator could not run a scenario against a model.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SimError {
    /// The scenario does not match the model. Most problems are found before
    /// the run starts; a step whose target only exists if a controller
    /// spawns it is checked when the step runs.
    #[error("{0}")]
    Scenario(#[from] ScenarioError),
    /// More than this many queue items were delivered.
    #[error("the cascade did not settle within {0} queue items; is there an unbounded cycle?")]
    StepLimit(usize),
    #[error("the finding is not a race candidate")]
    NotARace,
    /// The scenario never sends fires from both rules to one instance as a
    /// result of the same `origin` event.
    #[error(
        "the scenario never reaches this race: no `{origin}` leads to fires from both `{first}` and `{second}` at one instance"
    )]
    RaceNotReached { origin: String, first: String, second: String },
    /// `later` is only fired because `earlier` was delivered, so their order
    /// is causal rather than a race.
    #[error("`{later}` only fires after `{earlier}` has been delivered, so their order cannot be swapped")]
    RaceNotSwappable { earlier: String, later: String },
}
