//! Scenario files next to a definition.
//!
//! Local discovery until the simulator workstream ships its own: every
//! `*.yaml`/`*.yml` in a `scenarios/` directory beside the definition, and
//! every `*.scenario.yaml`/`*.scenario.yml` beside it. A scenario's id is
//! its file name without the extension and without a `.scenario` suffix;
//! ids are what `ViewState::scenario` stores.

use std::path::{Path, PathBuf};

/// The directory scenario files live in, next to the definition.
pub const SCENARIO_DIR: &str = "scenarios";

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ScenarioFile {
    pub id: String,
    pub path: PathBuf,
}

fn yaml_stem(name: &str) -> Option<&str> {
    name.strip_suffix(".yaml").or_else(|| name.strip_suffix(".yml"))
}

/// The id of a scenario file, or `None` when `path` is not one.
/// `in_scenario_dir` says whether it sits in the `scenarios/` directory.
pub fn scenario_id(path: &Path, in_scenario_dir: bool) -> Option<String> {
    let name = path.file_name()?.to_str()?;
    let stem = yaml_stem(name)?;
    let id = match stem.strip_suffix(".scenario") {
        Some(id) => id,
        None if in_scenario_dir => stem,
        None => return None,
    };
    (!id.is_empty() && !id.starts_with('.')).then(|| id.to_owned())
}

/// Whether `path` is a scenario file for the definition in `dir`.
pub fn is_scenario_path(path: &Path, dir: &Path) -> bool {
    let scenario_dir = dir.join(SCENARIO_DIR);
    if path == scenario_dir {
        return true;
    }
    match path.parent() {
        Some(parent) if parent == dir => scenario_id(path, false).is_some(),
        Some(parent) if parent == scenario_dir => scenario_id(path, true).is_some(),
        _ => false,
    }
}

/// Every scenario for `definition`, sorted by id. When two files share an
/// id, the one in `scenarios/` wins.
pub fn discover(definition: &Path) -> Vec<ScenarioFile> {
    let Some(dir) = definition.parent() else {
        return Vec::new();
    };
    let mut found: Vec<ScenarioFile> = Vec::new();
    let mut scan = |dir: &Path, in_scenario_dir: bool| {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            if let Some(id) = scenario_id(&path, in_scenario_dir)
                && !found.iter().any(|s| s.id == id)
            {
                found.push(ScenarioFile { id, path });
            }
        }
    };
    scan(&dir.join(SCENARIO_DIR), true);
    scan(dir, false);
    found.sort();
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids() {
        assert_eq!(scenario_id(Path::new("/d/scenarios/happy.yaml"), true), Some("happy".into()));
        assert_eq!(scenario_id(Path::new("/d/scenarios/race.scenario.yml"), true), Some("race".into()));
        assert_eq!(scenario_id(Path::new("/d/happy.scenario.yaml"), false), Some("happy".into()));
        assert_eq!(scenario_id(Path::new("/d/cascade.yaml"), false), None);
        assert_eq!(scenario_id(Path::new("/d/scenarios/notes.txt"), true), None);
        assert_eq!(scenario_id(Path::new("/d/scenarios/.hidden.yaml"), true), None);
        assert_eq!(scenario_id(Path::new("/d/.scenario.yaml"), false), None);
    }

    #[test]
    fn scenario_paths_for_watching() {
        let dir = Path::new("/d");
        assert!(is_scenario_path(Path::new("/d/scenarios/a.yaml"), dir));
        assert!(is_scenario_path(Path::new("/d/a.scenario.yaml"), dir));
        assert!(is_scenario_path(Path::new("/d/scenarios"), dir));
        assert!(!is_scenario_path(Path::new("/d/cascade.yaml"), dir));
        assert!(!is_scenario_path(Path::new("/e/scenarios/a.yaml"), dir));
    }

    #[test]
    fn discovery_prefers_the_scenario_dir_and_sorts() {
        let dir = std::env::temp_dir().join(format!("cascade-scenarios-test-{}", std::process::id()));
        let sdir = dir.join(SCENARIO_DIR);
        std::fs::create_dir_all(&sdir).expect("temp dir");
        let files = [
            sdir.join("b.yaml"),
            sdir.join("dup.yaml"),
            dir.join("dup.scenario.yaml"),
            dir.join("a.scenario.yml"),
            dir.join("cascade.yaml"),
        ];
        for f in &files {
            std::fs::write(f, "").expect("write");
        }
        let found = discover(&dir.join("cascade.yaml"));
        let ids: Vec<&str> = found.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, ["a", "b", "dup"]);
        assert_eq!(found[2].path, sdir.join("dup.yaml"));
        for f in &files {
            let _ = std::fs::remove_file(f);
        }
        let _ = std::fs::remove_dir(&sdir);
        let _ = std::fs::remove_dir(&dir);
    }

    #[test]
    fn discovery_without_a_directory_is_empty() {
        assert!(discover(Path::new("/definitely/not/here/cascade.yaml")).is_empty());
    }
}
