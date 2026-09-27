//! Scenario files: instances to create and an ordered list of external
//! triggers to feed the simulator.
//!
//! Owner: `feat/simulator` defines the YAML format and implements
//! [`parse_scenario`]. The types are the contract.

use std::collections::BTreeMap;

use cascade_core::definition::TriggerRef;
use cascade_core::span::SourceSpan;

use crate::error::ScenarioError;

#[derive(Clone, Debug, PartialEq)]
pub struct Scenario {
    pub name: String,
    pub instances: Vec<InstanceDecl>,
    pub steps: Vec<Step>,
}

/// One machine instance present when the scenario starts.
#[derive(Clone, Debug, PartialEq)]
pub struct InstanceDecl {
    /// Unique within the scenario, e.g. `o1`.
    pub name: String,
    pub machine: String,
    /// Field values target selectors match against, e.g. `orderId: 42`.
    pub fields: BTreeMap<String, String>,
    /// Starting state path; the machine's initial state when absent.
    pub state: Option<String>,
    pub span: SourceSpan,
}

/// When a step's trigger is delivered relative to the cascade before it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum StepTiming {
    /// Wait until the event queue has drained.
    #[default]
    AfterQuiescence,
    /// Deliver right away, interleaving with any cascade still queued.
    Immediate,
}

/// An external source fires a trigger at an instance.
#[derive(Clone, Debug, PartialEq)]
pub struct Step {
    pub source: String,
    pub fire: TriggerRef,
    /// Instance name.
    pub target: String,
    /// Payload for events the resulting transition emits.
    pub payload: BTreeMap<String, String>,
    pub timing: StepTiming,
    pub span: SourceSpan,
}

/// Parse a scenario file.
///
/// Stub until `feat/simulator` lands.
pub fn parse_scenario(text: &str) -> Result<Scenario, ScenarioError> {
    let _ = text;
    Err(ScenarioError::NotImplemented)
}
