//! Outcome rows: what the panel lists for each outcome, as text.

use cascade_core::{Model, Severity};
use cascade_sim::{Anomaly, Exploration, ExploreOptions, Outcome, differing_instances, instance_text, timer_text};

/// One anomaly line, with the severity colour findings use.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AnomalyLine {
    pub severity: Severity,
    pub text: String,
}

/// One outcome in the list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutcomeRow {
    /// 1-based, as `cascade explore` numbers outcomes.
    pub number: usize,
    /// `Outcome 2 · 3 paths`.
    pub title: String,
    /// Outcome 1: every instance; the others: those that differ from
    /// outcome 1 (`o1 (Order): paid {orderId: 1}`).
    pub instances: Vec<String>,
    /// Shown when no instance differs from outcome 1 (the outcome differs in
    /// timers or anomalies only).
    pub same_instances: bool,
    /// Timers left armed (`c1: after-30m due in 30m`).
    pub timers: Vec<String>,
    pub anomalies: Vec<AnomalyLine>,
    pub paths: u64,
    /// The instances the canvas preview highlights: for outcome 1 those
    /// that differ from any other outcome, for the others those that differ
    /// from outcome 1.
    pub differs: Vec<String>,
}

/// How serious an anomaly is, in the findings' terms: a path that never
/// settles or a selector that cannot choose is an error; a trigger
/// dropped, a rule with no target or an input skipped is a warning; a
/// decision nobody took is information.
pub fn anomaly_severity(anomaly: &Anomaly) -> Severity {
    match anomaly {
        Anomaly::StepLimit | Anomaly::Ambiguous { .. } => Severity::Error,
        Anomaly::NoTarget { .. } | Anomaly::Dropped { .. } | Anomaly::InputFailed { .. } => Severity::Warning,
        Anomaly::UnusedChoice { .. } => Severity::Info,
    }
}

fn paths_text(paths: u64) -> String {
    match paths {
        1 => "1 path".to_owned(),
        u64::MAX => "very many paths".to_owned(),
        n => format!("{n} paths"),
    }
}

fn row(model: &Model, number: usize, outcome: &Outcome, first: &Outcome, all: &[Outcome]) -> OutcomeRow {
    let (shown, differs): (Vec<&str>, Vec<String>) = if number == 1 {
        let mut differs: Vec<String> =
            all.iter().skip(1).flat_map(|other| differing_instances(first, other)).map(str::to_owned).collect();
        differs.sort_unstable();
        differs.dedup();
        (outcome.instances.keys().map(String::as_str).collect(), differs)
    } else {
        let names = differing_instances(first, outcome);
        let differs = names.iter().map(|n| (*n).to_owned()).collect();
        (names, differs)
    };
    let instances: Vec<String> = shown
        .iter()
        .map(|name| match outcome.instances.get(*name) {
            Some(value) => instance_text(model, name, value),
            None => format!("{name}: not in this outcome"),
        })
        .collect();
    OutcomeRow {
        number,
        title: format!("Outcome {number} · {}", paths_text(outcome.paths)),
        same_instances: number > 1 && instances.is_empty(),
        instances,
        timers: outcome.timers.iter().map(|t| timer_text(model, t)).collect(),
        anomalies: outcome
            .anomalies
            .iter()
            .map(|a| AnomalyLine { severity: anomaly_severity(a), text: a.to_string() })
            .collect(),
        paths: outcome.paths,
        differs,
    }
}

/// One row per outcome, in the exploration's order.
pub fn outcome_rows(model: &Model, exploration: &Exploration) -> Vec<OutcomeRow> {
    let Some(first) = exploration.outcomes.first() else {
        return Vec::new();
    };
    exploration
        .outcomes
        .iter()
        .enumerate()
        .map(|(i, outcome)| row(model, i + 1, outcome, first, &exploration.outcomes))
        .collect()
}

/// `2 outcomes, 7 states (complete)` or `… (incomplete: bound of N states
/// hit)`.
pub fn summary_text(exploration: &Exploration, options: &ExploreOptions) -> String {
    let outcomes = match exploration.outcomes.len() {
        1 => "1 outcome".to_owned(),
        n => format!("{n} outcomes"),
    };
    let states = match exploration.states {
        1 => "1 state".to_owned(),
        n => format!("{n} states"),
    };
    if exploration.complete {
        format!("{outcomes}, {states} (complete)")
    } else {
        format!("{outcomes}, {states} (incomplete: bound of {} states hit)", options.max_states)
    }
}
