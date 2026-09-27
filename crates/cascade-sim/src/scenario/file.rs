//! Finding and reading scenario files next to a definition.

use std::io;
use std::path::{Path, PathBuf};

use super::{Scenario, parse_scenario};
use crate::error::ScenarioError;

#[derive(Debug, thiserror::Error)]
pub enum DiscoverError {
    #[error("cannot list {}: {source}", path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

/// The scenario files that belong to the definition at `definition`, sorted
/// by path:
///
/// - every `*.yaml` / `*.yml` file directly inside a sibling `scenarios/`
///   directory (a missing directory is not an error), and
/// - every `*.scenario.yaml` / `*.scenario.yml` file next to the definition.
///
/// Files are not opened; parse them with [`load_scenario_file`].
pub fn discover_scenarios(definition: &Path) -> Result<Vec<PathBuf>, DiscoverError> {
    let dir = match definition.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    };

    let mut found = Vec::new();
    let scenarios_dir = dir.join("scenarios");
    if scenarios_dir.is_dir() {
        found.extend(files_in(&scenarios_dir, |name| has_extension(name, &[".yaml", ".yml"]))?);
    }
    found.extend(files_in(dir, |name| has_extension(name, &[".scenario.yaml", ".scenario.yml"]))?);

    found.sort();
    found.dedup();
    tracing::debug!(definition = %definition.display(), count = found.len(), "discovered scenario files");
    Ok(found)
}

fn has_extension(name: &str, suffixes: &[&str]) -> bool {
    suffixes.iter().any(|suffix| name.len() > suffix.len() && name.ends_with(suffix))
}

/// Regular files (following symlinks) directly in `dir` whose name passes
/// `keep`.
fn files_in(dir: &Path, keep: impl Fn(&str) -> bool) -> Result<Vec<PathBuf>, DiscoverError> {
    let io_error = |source| DiscoverError::Io { path: dir.to_owned(), source };
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).map_err(io_error)? {
        let entry = entry.map_err(io_error)?;
        let path = entry.path();
        let wanted = entry.file_name().to_str().is_some_and(&keep);
        if wanted && path.is_file() {
            out.push(path);
        }
    }
    Ok(out)
}

#[derive(Debug, thiserror::Error)]
pub enum ScenarioFileError {
    #[error("cannot read {}: {source}", path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("{}:\n{source}", path.display())]
    Invalid {
        path: PathBuf,
        #[source]
        source: ScenarioError,
    },
}

/// Read and parse a scenario file.
pub fn load_scenario_file(path: &Path) -> Result<Scenario, ScenarioFileError> {
    let text =
        std::fs::read_to_string(path).map_err(|source| ScenarioFileError::Io { path: path.to_owned(), source })?;
    parse_scenario(&text).map_err(|source| ScenarioFileError::Invalid { path: path.to_owned(), source })
}
