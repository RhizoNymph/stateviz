//! Finding and loading scenario files next to a definition.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use cascade_sim::{DiscoverError, ScenarioFileError, discover_scenarios, load_scenario_file};

/// A fresh directory under the system temp dir (`$TMPDIR`), removed on drop.
struct TempDir(PathBuf);

impl TempDir {
    fn new(label: &str) -> Self {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("cascade-sim-{label}-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        if let Err(err) = fs::create_dir_all(&path) {
            panic!("cannot create {}: {err}", path.display());
        }
        TempDir(path)
    }

    fn write(&self, rel: &str, text: &str) -> PathBuf {
        let path = self.0.join(rel);
        if let Some(parent) = path.parent()
            && let Err(err) = fs::create_dir_all(parent)
        {
            panic!("cannot create {}: {err}", parent.display());
        }
        if let Err(err) = fs::write(&path, text) {
            panic!("cannot write {}: {err}", path.display());
        }
        path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

const SCENARIO: &str = "scenario: s\nsteps: []\n";

fn discover(definition: &Path) -> Vec<PathBuf> {
    match discover_scenarios(definition) {
        Ok(found) => found,
        Err(err) => panic!("{err}"),
    }
}

#[test]
fn finds_the_scenarios_directory_and_sibling_scenario_files_sorted() {
    let dir = TempDir::new("discover");
    let definition = dir.write("cascade.yaml", "machines: {}\n");
    dir.write("scenarios/b.yaml", SCENARIO);
    dir.write("scenarios/a.yml", SCENARIO);
    dir.write("scenarios/notes.txt", "not a scenario");
    dir.write("scenarios/nested/deep.yaml", SCENARIO);
    dir.write("checkout.scenario.yaml", SCENARIO);
    dir.write("refund.scenario.yml", SCENARIO);
    dir.write("other.yaml", "not a scenario either");
    dir.write(".scenario.yaml", "no stem");

    assert_eq!(
        discover(&definition),
        [
            dir.0.join("checkout.scenario.yaml"),
            dir.0.join("refund.scenario.yml"),
            dir.0.join("scenarios/a.yml"),
            dir.0.join("scenarios/b.yaml"),
        ]
    );
}

#[test]
fn a_missing_scenarios_directory_is_not_an_error() {
    let dir = TempDir::new("no-scenarios");
    let definition = dir.write("cascade.yaml", "machines: {}\n");
    assert!(discover(&definition).is_empty());
    dir.write("x.scenario.yaml", SCENARIO);
    assert_eq!(discover(&definition), [dir.0.join("x.scenario.yaml")]);
}

#[test]
fn a_missing_definition_directory_is_an_error() {
    let dir = TempDir::new("gone");
    let definition = dir.0.join("missing/cascade.yaml");
    assert!(matches!(discover_scenarios(&definition), Err(DiscoverError::Io { .. })));
}

#[test]
fn the_example_ships_scenarios() {
    let definition = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/order-fulfillment/cascade.yaml");
    let found = discover(&definition);
    let names: Vec<String> =
        found.iter().filter_map(|p| p.file_name().and_then(|n| n.to_str()).map(str::to_owned)).collect();
    assert!(names.contains(&"happy-path.yaml".to_owned()), "{names:?}");
    for path in &found {
        if let Err(err) = load_scenario_file(path) {
            panic!("{err}");
        }
    }
}

#[test]
fn loading_reports_io_and_parse_errors_with_the_path() {
    let dir = TempDir::new("load");
    let missing = dir.0.join("nope.yaml");
    assert!(matches!(load_scenario_file(&missing), Err(ScenarioFileError::Io { .. })));

    let bad = dir.write("bad.yaml", "scenario: bad\nsteps:\n  - { source: A, fire: nope }\n");
    match load_scenario_file(&bad) {
        Err(err @ ScenarioFileError::Invalid { .. }) => {
            let text = err.to_string();
            assert!(text.contains("bad.yaml"), "{text}");
            assert!(text.contains("3:"), "{text}");
        }
        other => panic!("expected a parse error, got {other:?}"),
    }

    let good = dir.write("good.yaml", SCENARIO);
    assert!(matches!(load_scenario_file(&good), Ok(s) if s.name == "s"));
}
