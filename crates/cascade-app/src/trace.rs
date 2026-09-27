//! Trace view support: running the simulator for the selected scenario, or
//! both orderings of a race candidate.
//!
//! Traces hold typed ids, so they are only valid with the model they were
//! computed from. Every run records the model generation it used; the
//! workspace hands traces to the scene builder only when that generation is
//! still the one on screen.

use std::path::Path;
use std::sync::Arc;

use cascade_core::{FindingDetail, Model};
use cascade_sim::{ScenarioError, SimError, Trace, parse_scenario, race_orderings, simulate};

/// What the trace view asks for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TraceRequest {
    /// Scenario id (see `document::scenarios`).
    pub scenario: String,
    /// Replay this race candidate in both orders (index among race
    /// candidates), or run the scenario once.
    pub race: Option<u32>,
    /// The model generation the traces are for.
    pub generation: u64,
}

#[derive(Clone, Debug, Default)]
pub enum TraceRun {
    #[default]
    Idle,
    Running {
        request: TraceRequest,
    },
    Done {
        request: TraceRequest,
        traces: Arc<Vec<Trace>>,
    },
    Failed {
        request: TraceRequest,
        error: String,
    },
}

impl TraceRun {
    pub fn request(&self) -> Option<&TraceRequest> {
        match self {
            TraceRun::Idle => None,
            TraceRun::Running { request } | TraceRun::Done { request, .. } | TraceRun::Failed { request, .. } => {
                Some(request)
            }
        }
    }

    /// Traces for `request`, or nothing if they are for anything else.
    pub fn traces_for(&self, request: &TraceRequest) -> &[Trace] {
        match self {
            TraceRun::Done { request: r, traces } if r == request => traces,
            _ => &[],
        }
    }

    /// Whether a new run is needed to satisfy `request`.
    pub fn needs_run(&self, request: &TraceRequest) -> bool {
        self.request() != Some(request)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum TraceError {
    #[error("cannot read scenario {path}: {source}")]
    Read {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("scenario `{scenario}`: {source}")]
    Scenario {
        scenario: String,
        #[source]
        source: ScenarioError,
    },
    #[error("simulating `{scenario}`: {source}")]
    Sim {
        scenario: String,
        #[source]
        source: SimError,
    },
    #[error("there is no race candidate #{0}")]
    NoSuchRace(u32),
}

/// Read and parse the scenario at `path`, then simulate it, or replay the
/// race `race` (a race-candidate finding detail) in both orders.
pub fn run(model: &Model, id: &str, path: &Path, race: Option<&FindingDetail>) -> Result<Vec<Trace>, TraceError> {
    let text = std::fs::read_to_string(path)
        .map_err(|source| TraceError::Read { path: path.display().to_string(), source })?;
    let scenario = parse_scenario(&text).map_err(|source| TraceError::Scenario { scenario: id.to_owned(), source })?;
    match race {
        None => simulate(model, &scenario)
            .map(|t| vec![t])
            .map_err(|source| TraceError::Sim { scenario: id.to_owned(), source }),
        Some(detail) => race_orderings(model, &scenario, detail)
            .map(|r| vec![r.as_queued, r.swapped])
            .map_err(|source| TraceError::Sim { scenario: id.to_owned(), source }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(generation: u64) -> TraceRequest {
        TraceRequest { scenario: "happy".into(), race: None, generation }
    }

    #[test]
    fn traces_only_match_their_request() {
        let run = TraceRun::Done { request: request(1), traces: Arc::new(Vec::new()) };
        assert!(!run.needs_run(&request(1)));
        assert!(run.needs_run(&request(2)));
        assert!(run.traces_for(&request(2)).is_empty());
        assert!(TraceRun::Idle.needs_run(&request(1)));
        assert!(!TraceRun::Running { request: request(1) }.needs_run(&request(1)));
    }

    #[test]
    fn missing_scenario_file_is_a_read_error() {
        let model =
            cascade_core::load_str(include_str!("../../../examples/order-fulfillment/cascade.yaml")).expect("loads");
        let err = run(&model, "x", Path::new("/definitely/not/here.yaml"), None).expect_err("missing");
        assert!(matches!(err, TraceError::Read { .. }));
    }

    #[test]
    fn simulator_errors_surface() {
        let model =
            cascade_core::load_str(include_str!("../../../examples/order-fulfillment/cascade.yaml")).expect("loads");
        let dir = std::env::temp_dir().join(format!("cascade-trace-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("s.scenario.yaml");
        std::fs::write(&path, "name: s\nsteps: []\n").expect("write");
        // Whatever the simulator does with this, it must come back as a
        // value (stub: NotImplemented), never a panic.
        match run(&model, "s", &path, None) {
            Ok(traces) => assert_eq!(traces.len(), 1),
            Err(err) => assert!(!err.to_string().is_empty()),
        }
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&dir);
    }
}
