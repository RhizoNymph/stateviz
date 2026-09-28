//! Play-mode inputs: field and payload assignments, the trigger palette's
//! grouping, and saving a session as a scenario file.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use cascade_core::parse::grammar::is_valid_name;
use cascade_sim::{AvailableFire, PlaySession, SimError, scenario_to_yaml};

use crate::document::scenarios::SCENARIO_DIR;

/// Parse `orderId=42, note="two words"` (also `key: value`). Blank text is
/// an empty map.
pub fn parse_assignments(text: &str) -> Result<BTreeMap<String, String>, String> {
    let mut out = BTreeMap::new();
    for part in text.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        let (key, value) = part
            .split_once('=')
            .or_else(|| part.split_once(':'))
            .ok_or_else(|| format!("`{part}` is not key=value"))?;
        let (key, value) = (key.trim(), value.trim());
        if !is_valid_name(key) {
            return Err(format!("`{key}` is not a valid field name"));
        }
        let value = value.strip_prefix('"').and_then(|v| v.strip_suffix('"')).unwrap_or(value).to_owned();
        if out.insert(key.to_owned(), value).is_some() {
            return Err(format!("`{key}` is given twice"));
        }
    }
    Ok(out)
}

/// Palette buttons grouped by source, in the order sources first appear.
pub fn group_fires(fires: Vec<AvailableFire>) -> Vec<(String, Vec<AvailableFire>)> {
    let mut groups: Vec<(String, Vec<AvailableFire>)> = Vec::new();
    for fire in fires {
        match groups.iter_mut().find(|(source, _)| *source == fire.source) {
            Some((_, group)) => group.push(fire),
            None => groups.push((fire.source.clone(), vec![fire])),
        }
    }
    groups
}

/// Why a session could not be saved as a scenario.
#[derive(Debug, thiserror::Error)]
pub enum SaveScenarioError {
    #[error("`{0}` is not a usable scenario name (letters, digits, _ and -)")]
    BadName(String),
    #[error("{0}")]
    Sim(#[from] SimError),
    #[error("the scenario came out empty (scenario export is not implemented yet)")]
    Empty,
    #[error("cannot write {}: {source}", path.display())]
    Io { path: PathBuf, source: std::io::Error },
}

/// `scenarios/<name>.yaml` next to the definition.
pub fn scenario_path(definition: &Path, name: &str) -> Result<PathBuf, SaveScenarioError> {
    let name = name.trim();
    if !is_valid_name(name) {
        return Err(SaveScenarioError::BadName(name.to_owned()));
    }
    let dir = definition.parent().unwrap_or_else(|| Path::new("."));
    Ok(dir.join(SCENARIO_DIR).join(format!("{name}.yaml")))
}

/// The session up to its position, as scenario YAML.
pub fn scenario_text(session: &PlaySession, name: &str) -> Result<String, SaveScenarioError> {
    let scenario = session.to_scenario(name.trim())?;
    let text = scenario_to_yaml(&scenario);
    if text.trim().is_empty() {
        return Err(SaveScenarioError::Empty);
    }
    Ok(text)
}

/// Write `text` to `path`, creating `scenarios/`.
pub fn write_scenario(path: &Path, text: &str) -> Result<(), SaveScenarioError> {
    let io = |source| SaveScenarioError::Io { path: path.to_owned(), source };
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(io)?;
    }
    crate::build::disk::write_atomic(path, text).map_err(io)
}

#[cfg(test)]
mod tests {
    use cascade_core::definition::TriggerRef;

    use super::*;

    fn fire(source: &str, trigger: &str, target: &str, accepted: bool) -> AvailableFire {
        AvailableFire {
            source: source.into(),
            trigger: TriggerRef { machine: "Order".into(), trigger: trigger.into() },
            target: target.into(),
            accepted,
        }
    }

    #[test]
    fn assignments_parse() {
        let map = parse_assignments(" orderId=42, note = \"two words\", kind: gift ").expect("parses");
        assert_eq!(map.get("orderId").map(String::as_str), Some("42"));
        assert_eq!(map.get("note").map(String::as_str), Some("two words"));
        assert_eq!(map.get("kind").map(String::as_str), Some("gift"));
        assert!(parse_assignments("   ").expect("blank").is_empty());
        assert_eq!(parse_assignments("orderId"), Err("`orderId` is not key=value".into()));
        assert_eq!(parse_assignments("a b=1"), Err("`a b` is not a valid field name".into()));
        assert_eq!(parse_assignments("a=1, a=2"), Err("`a` is given twice".into()));
    }

    #[test]
    fn fires_group_by_source_in_order() {
        let groups = group_fires(vec![
            fire("User", "pay", "o1", true),
            fire("Clock", "tick", "o1", false),
            fire("User", "pay", "o2", false),
        ]);
        assert_eq!(groups.iter().map(|(s, g)| (s.as_str(), g.len())).collect::<Vec<_>>(), [("User", 2), ("Clock", 1)]);
        assert!(group_fires(Vec::new()).is_empty());
    }

    #[test]
    fn scenario_paths() {
        assert_eq!(
            scenario_path(Path::new("/w/cascade.yaml"), " happy-path ").expect("valid"),
            PathBuf::from("/w/scenarios/happy-path.yaml")
        );
        assert!(matches!(scenario_path(Path::new("/w/cascade.yaml"), "../x"), Err(SaveScenarioError::BadName(_))));
        assert!(matches!(scenario_path(Path::new("/w/cascade.yaml"), ""), Err(SaveScenarioError::BadName(_))));
    }

    #[test]
    fn an_empty_session_saves_or_says_why_not() {
        let model = cascade_core::load_str("machines:\n  A:\n    states: [x]\n").expect("loads");
        let session = PlaySession::new(&model);
        match scenario_text(&session, "empty") {
            Ok(text) => assert!(text.contains("empty")),
            Err(SaveScenarioError::Sim(SimError::NotImplemented) | SaveScenarioError::Empty) => {}
            Err(other) => panic!("unexpected {other}"),
        }
    }

    #[test]
    fn writing_creates_the_scenario_directory() {
        let dir = std::env::temp_dir().join(format!("cascade-scenario-test-{}", std::process::id()));
        let path = dir.join(SCENARIO_DIR).join("s.yaml");
        write_scenario(&path, "scenario: s\n").expect("writes");
        assert_eq!(std::fs::read_to_string(&path).expect("reads"), "scenario: s\n");
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(dir.join(SCENARIO_DIR));
        let _ = std::fs::remove_dir(&dir);
    }
}
