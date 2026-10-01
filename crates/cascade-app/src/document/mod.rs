//! The open definition: the last good model plus everything derived from
//! it, and the outcome of the latest load attempt.
//!
//! Loading is a pure function of the file's text ([`analyze_text`]); the
//! reload state machine ([`Document::apply`]) keeps the last good model on
//! screen when a later save does not parse, and reports the diagnostics.

pub mod scenarios;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::{DateTime, Local};

use cascade_core::{CausalGraph, Diagnostic, Finding, Model, Severity, analyze, load_str};

use crate::build::disk::{DiskChange, DiskSync};

/// A definition that loaded: the model and what is derived from it once
/// per load.
#[derive(Debug)]
pub struct Analyzed {
    pub model: Model,
    pub graph: CausalGraph,
    pub findings: Vec<Finding>,
    /// The text the model was loaded from; build mode patches it.
    pub text: String,
}

/// Why a load attempt failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoadFailure {
    /// The file could not be read.
    Io { message: String },
    /// The file does not parse or resolve.
    Invalid { diagnostics: Vec<Diagnostic> },
}

impl LoadFailure {
    /// One line per problem, `line:col: message` for diagnostics.
    pub fn lines(&self) -> Vec<String> {
        match self {
            LoadFailure::Io { message } => vec![message.clone()],
            LoadFailure::Invalid { diagnostics } => diagnostics.iter().map(ToString::to_string).collect(),
        }
    }

    pub fn count(&self) -> usize {
        match self {
            LoadFailure::Io { .. } => 1,
            LoadFailure::Invalid { diagnostics } => diagnostics.len(),
        }
    }
}

/// Parse, resolve, derive the causal graph and run the checks.
pub fn analyze_text(text: &str) -> Result<Analyzed, LoadFailure> {
    let model = load_str(text).map_err(|e| LoadFailure::Invalid { diagnostics: e.diagnostics })?;
    let graph = CausalGraph::build(&model);
    let findings = analyze(&model, &graph);
    Ok(Analyzed { model, graph, findings, text: text.to_owned() })
}

/// Read `path` as text.
pub fn read_text(path: &Path) -> Result<String, LoadFailure> {
    std::fs::read_to_string(path)
        .map_err(|e| LoadFailure::Io { message: format!("cannot read {}: {e}", path.display()) })
}

/// Read `path` and analyze it.
pub fn load_definition(path: &Path) -> Result<Analyzed, LoadFailure> {
    analyze_text(&read_text(path)?)
}

/// A re-read of the definition, compared with what the app knows is on disk.
#[derive(Debug)]
pub enum Reread {
    /// The text the app last read or wrote (its own save echoing back).
    Unchanged,
    /// Different text (or an unreadable file): `text` is what was read.
    Changed { text: Option<String>, result: Box<Result<Analyzed, LoadFailure>> },
}

/// Read `path`; analyze it only when it differs from what `known` says is
/// on disk.
pub fn reread(path: &Path, known: DiskSync) -> Reread {
    match read_text(path) {
        Ok(text) if known.classify(&text) == DiskChange::Own => Reread::Unchanged,
        Ok(text) => {
            let result = Box::new(analyze_text(&text));
            Reread::Changed { text: Some(text), result }
        }
        Err(failure) => Reread::Changed { text: None, result: Box::new(Err(failure)) },
    }
}

/// A successful load, shared cheaply with scene builds and background work.
#[derive(Clone, Debug)]
pub struct Loaded {
    /// Increases with every successful load. Anything computed from a
    /// model (traces, diffs) records the generation it used, so stale
    /// results are never paired with a newer model.
    pub generation: u64,
    pub model: Arc<Model>,
    pub graph: Arc<CausalGraph>,
    pub findings: Arc<Vec<Finding>>,
    /// The file text of this load.
    pub text: Arc<str>,
    pub loaded_at: DateTime<Local>,
}

impl Loaded {
    pub fn count(&self, severity: Severity) -> usize {
        self.findings.iter().filter(|f| f.severity == severity).count()
    }
}

/// Where the document stands after the latest load attempt.
#[derive(Clone, Debug)]
pub enum DocState {
    /// Nothing has been attempted yet.
    Empty,
    /// No load has succeeded; nothing to show.
    Failed {
        failure: LoadFailure,
        at: DateTime<Local>,
    },
    Ready {
        loaded: Loaded,
    },
    /// The latest reload failed; the last good load is still shown.
    Stale {
        last_good: Loaded,
        failure: LoadFailure,
        at: DateTime<Local>,
    },
}

/// The open definition file and its load state.
#[derive(Debug)]
pub struct Document {
    path: PathBuf,
    state: DocState,
    next_generation: u64,
}

impl Document {
    pub fn new(path: PathBuf) -> Self {
        Self { path, state: DocState::Empty, next_generation: 1 }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn state(&self) -> &DocState {
        &self.state
    }

    /// Record the outcome of a load attempt made at `at`.
    pub fn apply(&mut self, result: Result<Analyzed, LoadFailure>, at: DateTime<Local>) {
        let previous = std::mem::replace(&mut self.state, DocState::Empty);
        self.state = match result {
            Ok(analyzed) => {
                let generation = self.next_generation;
                self.next_generation += 1;
                DocState::Ready {
                    loaded: Loaded {
                        generation,
                        model: Arc::new(analyzed.model),
                        graph: Arc::new(analyzed.graph),
                        findings: Arc::new(analyzed.findings),
                        text: Arc::from(analyzed.text),
                        loaded_at: at,
                    },
                }
            }
            Err(failure) => match previous {
                DocState::Ready { loaded } => DocState::Stale { last_good: loaded, failure, at },
                DocState::Stale { last_good, .. } => DocState::Stale { last_good, failure, at },
                DocState::Empty | DocState::Failed { .. } => DocState::Failed { failure, at },
            },
        };
    }

    /// The model on screen: the latest good load, even when a later reload
    /// failed.
    pub fn loaded(&self) -> Option<&Loaded> {
        match &self.state {
            DocState::Ready { loaded } | DocState::Stale { last_good: loaded, .. } => Some(loaded),
            DocState::Empty | DocState::Failed { .. } => None,
        }
    }

    /// The latest failure, if the latest attempt failed.
    #[cfg(test)]
    fn failure(&self) -> Option<(&LoadFailure, DateTime<Local>)> {
        match &self.state {
            DocState::Failed { failure, at } | DocState::Stale { failure, at, .. } => Some((failure, *at)),
            DocState::Empty | DocState::Ready { .. } => None,
        }
    }

    /// `generation` of the model on screen, 0 when there is none.
    pub fn generation(&self) -> u64 {
        self.loaded().map_or(0, |l| l.generation)
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    const GOOD: &str = "machines:\n  Order:\n    states: [draft, paid]\n    transitions:\n      - { from: draft, to: paid, on: pay }\n";
    const BAD: &str =
        "machines:\n  Order:\n    states: [draft]\n    transitions:\n      - { from: draft, to: nowhere, on: pay }\n";

    fn t(secs: i64) -> DateTime<Local> {
        Local.timestamp_opt(1_700_000_000 + secs, 0).single().expect("valid time")
    }

    #[test]
    fn analyze_text_reports_diagnostics_with_lines() {
        let err = analyze_text(BAD).expect_err("does not resolve");
        let lines = err.lines();
        assert_eq!(err.count(), lines.len());
        assert!(lines[0].starts_with("5:"), "{lines:?}");
        assert!(lines[0].contains("nowhere"), "{lines:?}");
    }

    #[test]
    fn first_failure_has_nothing_to_show() {
        let mut doc = Document::new("x.yaml".into());
        doc.apply(analyze_text(BAD), t(0));
        assert!(doc.loaded().is_none());
        assert!(matches!(doc.state(), DocState::Failed { .. }));
        assert_eq!(doc.generation(), 0);
    }

    #[test]
    fn failed_reload_keeps_the_last_good_model() {
        let mut doc = Document::new("x.yaml".into());
        doc.apply(analyze_text(GOOD), t(0));
        assert_eq!(doc.generation(), 1);
        assert!(doc.failure().is_none());
        doc.apply(analyze_text(BAD), t(5));
        let loaded = doc.loaded().expect("last good kept");
        assert_eq!(loaded.generation, 1);
        assert_eq!(loaded.loaded_at, t(0));
        let (failure, at) = doc.failure().expect("failure reported");
        assert_eq!(at, t(5));
        assert!(failure.count() > 0);
        doc.apply(Err(LoadFailure::Io { message: "gone".into() }), t(9));
        assert_eq!(doc.loaded().map(|l| l.generation), Some(1));
        assert_eq!(doc.failure().map(|(f, _)| f.lines()), Some(vec!["gone".to_owned()]));
    }

    #[test]
    fn a_good_reload_clears_the_failure_and_bumps_the_generation() {
        let mut doc = Document::new("x.yaml".into());
        doc.apply(analyze_text(BAD), t(0));
        doc.apply(analyze_text(GOOD), t(1));
        assert_eq!(doc.generation(), 1);
        doc.apply(analyze_text(BAD), t(2));
        doc.apply(analyze_text(GOOD), t(3));
        assert_eq!(doc.generation(), 2);
        assert!(doc.failure().is_none());
        assert!(matches!(doc.state(), DocState::Ready { .. }));
    }

    #[test]
    fn missing_file_is_an_io_failure() {
        let err = load_definition(Path::new("/definitely/not/here.yaml")).expect_err("missing");
        assert!(matches!(err, LoadFailure::Io { .. }));
    }

    #[test]
    fn example_loads() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/order-fulfillment/cascade.yaml");
        let analyzed = load_definition(&path).expect("example loads");
        assert_eq!(analyzed.model.machine_count(), 2);
    }

    #[test]
    fn reread_skips_the_apps_own_text() {
        let dir = std::env::temp_dir().join(format!("cascade-reread-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("dir");
        let path = dir.join("cascade.yaml");
        std::fs::write(&path, GOOD).expect("write");
        let mut known = DiskSync::default();
        match reread(&path, known) {
            Reread::Changed { text: Some(text), result } => {
                assert_eq!(text, GOOD);
                assert_eq!(result.map(|a| a.text).ok().as_deref(), Some(GOOD));
            }
            other => panic!("unexpected {other:?}"),
        }
        known.note(GOOD);
        assert!(matches!(reread(&path, known), Reread::Unchanged));
        std::fs::write(&path, BAD).expect("write");
        assert!(matches!(reread(&path, known), Reread::Changed { text: Some(_), result } if result.is_err()));
        let _ = std::fs::remove_file(&path);
        assert!(matches!(
            reread(&path, known),
            Reread::Changed { text: None, result } if matches!(*result, Err(LoadFailure::Io { .. }))
        ));
        let _ = std::fs::remove_dir(&dir);
    }

    #[test]
    fn loaded_keeps_the_text() {
        let mut doc = Document::new("x.yaml".into());
        doc.apply(analyze_text(GOOD), t(0));
        assert_eq!(doc.loaded().map(|l| l.text.as_ref()), Some(GOOD));
    }
}
