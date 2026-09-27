//! Behavioural tests for the checks: small definitions written for each
//! case, the two pinned examples, ordering, and a performance bound.

mod cycles;
mod events;
mod examples;
mod invalid_fire;
mod nondeterminism;
mod ordering;
mod perf;
mod races;
mod reachability;
mod state_dependent;

use crate::analysis::{Check, Finding, FindingDetail, Severity, analyze};
use crate::causal::CausalGraph;
use crate::key::ElementRef;
use crate::model::Model;

pub(super) fn load(text: &str) -> Model {
    match crate::load_str(text) {
        Ok(model) => model,
        Err(err) => panic!("expected the definition to load:\n{err}"),
    }
}

/// Load `text` and run every check.
pub(super) fn run(text: &str) -> (Model, Vec<Finding>) {
    let model = load(text);
    let graph = CausalGraph::build(&model);
    let findings = analyze(&model, &graph);
    (model, findings)
}

/// The findings of one check, in analysis order.
pub(super) fn of(findings: &[Finding], check: Check) -> Vec<&Finding> {
    findings.iter().filter(|f| f.check() == check).collect()
}

/// Exactly one finding of `check`; panics with the full list otherwise.
pub(super) fn single(findings: &[Finding], check: Check) -> &Finding {
    match of(findings, check).as_slice() {
        [one] => one,
        other => panic!("expected exactly one {check} finding, got {}: {findings:#?}", other.len()),
    }
}

/// The stable key string of an element, e.g. `rule:Billing/Paid#0`.
pub(super) fn key(model: &Model, element: ElementRef) -> String {
    model.key_of(element).to_string()
}

pub(super) fn keys(model: &Model, elements: &[ElementRef]) -> Vec<String> {
    elements.iter().map(|&e| key(model, e)).collect()
}

/// `(check, primary key, severity)` per finding, in analysis order.
pub(super) fn summary(model: &Model, findings: &[Finding]) -> Vec<(Check, String, Severity)> {
    findings.iter().map(|f| (f.check(), key(model, f.detail.primary()), f.severity)).collect()
}

/// Resolve a key string such as `state:Order:paid` in `model`.
pub(super) fn element(model: &Model, text_key: &str) -> ElementRef {
    let parsed: crate::key::ElementKey = match text_key.parse() {
        Ok(k) => k,
        Err(err) => panic!("{err}"),
    };
    match model.resolve_key(&parsed) {
        Some(e) => e,
        None => panic!("no element {text_key}"),
    }
}

/// The detail's check matches the variant for every variant (sanity for the
/// contract's exhaustive matches).
#[test]
fn detail_check_matches_variant() {
    let (model, findings) = run(examples::SHOP);
    for f in &findings {
        let expected = match f.detail {
            FindingDetail::InvalidFire { .. } | FindingDetail::DeadExternalTrigger { .. } => Check::InvalidFire,
            FindingDetail::Nondeterminism { .. } => Check::Nondeterminism,
            FindingDetail::CascadeCycle { .. } => Check::CascadeCycle,
            FindingDetail::UnhandledEvent { .. } => Check::UnhandledEvent,
            FindingDetail::OrphanController { .. } => Check::OrphanController,
            FindingDetail::UnreachableState { .. } => Check::UnreachableState,
            FindingDetail::RaceCandidate { .. } => Check::RaceCandidate,
            FindingDetail::StateDependentFire { .. } => Check::StateDependentFire,
        };
        assert_eq!(f.check(), expected);
        assert_eq!(f.severity, expected.default_severity());
        assert!(
            f.detail.subjects().contains(&f.detail.primary()),
            "primary is badged: {}",
            key(&model, f.detail.primary())
        );
        assert!(!f.message.contains('\n'), "messages are one line");
    }
}
