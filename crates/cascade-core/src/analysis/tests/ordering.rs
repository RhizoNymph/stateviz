//! Result ordering, determinism and the contract helpers.

use std::cmp::Reverse;

use super::*;
use crate::analysis::{CycleStep, has_errors};
use crate::ids::{EventId, ExternalId, HandlerId, MachineId, RuleId, StateId, TransitionId, TriggerId};

/// Severity descending, then check, then primary position: never decreasing.
pub(super) fn assert_sorted(model: &Model, findings: &[Finding]) {
    let keys: Vec<_> =
        findings.iter().map(|f| (Reverse(f.severity), f.check(), model.span_of(f.detail.primary()).start)).collect();
    for pair in keys.windows(2) {
        assert!(pair[0] <= pair[1], "out of order: {:?} before {:?}\n{findings:#?}", pair[0], pair[1]);
    }
}

#[test]
fn examples_are_sorted_by_severity_check_and_position() {
    for text in [examples::SPEC_EXAMPLE, examples::SHOP] {
        let (model, findings) = run(text);
        assert_sorted(&model, &findings);
    }
}

#[test]
fn analysis_is_deterministic() {
    let (model, first) = run(examples::SHOP);
    let graph = CausalGraph::build(&model);
    for _ in 0..3 {
        assert_eq!(analyze(&model, &graph), first);
    }
    // A fresh load of the same text gives the same findings.
    let (_, again) = run(examples::SHOP);
    assert_eq!(again, first);
}

#[test]
fn findings_on_one_element_are_ordered_by_their_subjects() {
    let (model, findings) = run(r#"
machines:
  A:
    states: [a0, a1]
    transitions:
      - { from: a0, to: a1, on: go }
external:
  User: [A.zeta, A.go, A.alpha]
"#);
    let dead: Vec<String> =
        of(&findings, Check::InvalidFire).iter().map(|f| key(&model, f.detail.subjects()[1])).collect();
    // Trigger ids follow first mention: `zeta` before `alpha`.
    assert_eq!(dead, ["trigger:A.zeta", "trigger:A.alpha"]);
}

#[test]
fn errors_come_first_even_when_later_in_the_file() {
    let (model, findings) = run(r#"
machines:
  A:
    states: [a0, a1, a2]
    transitions:
      - { from: a0, to: a1, on: go, emits: [Went] }
controllers:
  C:
    on:
      Went: [{ fire: A.nowhere }]
external:
  User: [A.go]
"#);
    assert_eq!(
        summary(&model, &findings),
        [
            (Check::InvalidFire, "rule:C/Went#0".to_owned(), Severity::Error),
            (Check::UnreachableState, "state:A:a2".to_owned(), Severity::Warning),
        ]
    );
}

#[test]
fn has_errors_only_counts_errors() {
    let (_, spec) = run(examples::SPEC_EXAMPLE);
    assert!(!has_errors(&spec));
    let (_, shop) = run(examples::SHOP);
    assert!(has_errors(&shop));
    assert!(!has_errors(&[]));
}

#[test]
fn new_findings_take_their_checks_default_severity() {
    let step = CycleStep { transition: TransitionId::new(0), rule: RuleId::new(0) };
    let details = [
        FindingDetail::InvalidFire { rule: RuleId::new(0), trigger: TriggerId::new(0) },
        FindingDetail::DeadExternalTrigger { source: ExternalId::new(0), trigger: TriggerId::new(0) },
        FindingDetail::Nondeterminism { state: StateId::new(0), trigger: TriggerId::new(0), transitions: vec![] },
        FindingDetail::CascadeCycle { first: step, rest: vec![step] },
        FindingDetail::UnhandledEvent { event: EventId::new(0) },
        FindingDetail::OrphanController { handler: HandlerId::new(0) },
        FindingDetail::UnreachableState { state: StateId::new(0) },
        FindingDetail::RaceCandidate {
            origin: EventId::new(0),
            machine: MachineId::new(0),
            first: RuleId::new(0),
            second: RuleId::new(1),
        },
        FindingDetail::StateDependentFire { rule: RuleId::new(0), trigger: TriggerId::new(0), dropped_in: vec![] },
    ];
    let expected = [
        Severity::Error,
        Severity::Error,
        Severity::Error,
        Severity::Warning,
        Severity::Warning,
        Severity::Warning,
        Severity::Warning,
        Severity::Info,
        Severity::Info,
    ];
    for (detail, severity) in details.into_iter().zip(expected) {
        let f = Finding::new(detail, "m");
        assert_eq!(f.severity, severity, "{:?}", f.detail);
        assert_eq!(f.message, "m");
    }
}

#[test]
fn dead_external_trigger_contract() {
    let detail = FindingDetail::DeadExternalTrigger { source: ExternalId::new(2), trigger: TriggerId::new(5) };
    assert_eq!(detail.check(), Check::InvalidFire);
    assert_eq!(detail.primary(), ElementRef::External(ExternalId::new(2)));
    assert_eq!(detail.subjects(), [ElementRef::External(ExternalId::new(2)), ElementRef::Trigger(TriggerId::new(5))]);
    assert!(detail.cycle_steps().is_empty());
}
