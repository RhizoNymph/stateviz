//! App settings remembered between runs, in a small JSON file:
//! `$XDG_CONFIG_HOME/cascade/settings.json`, or
//! `~/.config/cascade/settings.json` when `XDG_CONFIG_HOME` is unset.
//!
//! ```json
//! { "transition_pills": false }
//! ```
//!
//! Missing keys take their defaults and unknown keys are ignored, so older
//! and newer files both load. A missing or invalid file falls back to the
//! defaults with a logged warning; it is never fatal. Saving writes the
//! whole file atomically (temp file + rename), creating the directory.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Directory under the config home.
const APP_DIR: &str = "cascade";
/// File name in [`APP_DIR`].
const FILE_NAME: &str = "settings.json";

/// Everything the app remembers between runs.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppSettings {
    /// Structure view: transitions as pills (`true`) or as arrows.
    pub transition_pills: bool,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self { transition_pills: true }
    }
}

/// Why the settings file could not be read or written.
#[derive(Debug, thiserror::Error)]
pub enum SettingsError {
    #[error("no config directory: neither XDG_CONFIG_HOME nor HOME is set")]
    NoConfigDir,
    #[error("cannot read {path}: {source}")]
    Read { path: PathBuf, source: std::io::Error },
    #[error("invalid settings in {path}: {source}")]
    Parse { path: PathBuf, source: serde_json::Error },
    #[error("cannot encode settings: {0}")]
    Encode(#[source] serde_json::Error),
    #[error("cannot write {path}: {source}")]
    Write { path: PathBuf, source: std::io::Error },
}

/// The settings file for these environment values: under
/// `XDG_CONFIG_HOME` when it is set to an absolute path (the XDG spec
/// ignores relative ones), else under `HOME/.config`.
pub fn path_from(xdg_config_home: Option<OsString>, home: Option<OsString>) -> Result<PathBuf, SettingsError> {
    let xdg = xdg_config_home.map(PathBuf::from).filter(|p| p.is_absolute());
    let base = match xdg {
        Some(dir) => dir,
        None => home
            .map(PathBuf::from)
            .filter(|p| !p.as_os_str().is_empty())
            .ok_or(SettingsError::NoConfigDir)?
            .join(".config"),
    };
    Ok(base.join(APP_DIR).join(FILE_NAME))
}

/// The settings file for this process's environment.
pub fn default_path() -> Result<PathBuf, SettingsError> {
    path_from(std::env::var_os("XDG_CONFIG_HOME"), std::env::var_os("HOME"))
}

/// Parse a settings file's text.
pub fn parse(text: &str, path: &Path) -> Result<AppSettings, SettingsError> {
    serde_json::from_str(text).map_err(|source| SettingsError::Parse { path: path.to_owned(), source })
}

/// Read the settings at `path`.
pub fn load(path: &Path) -> Result<AppSettings, SettingsError> {
    let text = std::fs::read_to_string(path).map_err(|source| SettingsError::Read { path: path.to_owned(), source })?;
    parse(&text, path)
}

/// The settings to start with: the file's, or the defaults (with a
/// warning) when there is no file, no config directory, or it does not
/// parse.
pub fn load_or_default(path: Result<&Path, &SettingsError>) -> AppSettings {
    let path = match path {
        Ok(path) => path,
        Err(error) => {
            tracing::warn!(%error, "settings: using defaults");
            return AppSettings::default();
        }
    };
    match load(path) {
        Ok(settings) => {
            tracing::debug!(path = %path.display(), ?settings, "settings loaded");
            settings
        }
        Err(error) => {
            tracing::warn!(path = %path.display(), %error, "settings: using defaults");
            AppSettings::default()
        }
    }
}

/// The text written for `settings`: pretty JSON with a trailing newline.
pub fn to_text(settings: &AppSettings) -> Result<String, SettingsError> {
    let mut text = serde_json::to_string_pretty(settings).map_err(SettingsError::Encode)?;
    text.push('\n');
    Ok(text)
}

/// Write `settings` to `path`, creating its directory.
pub fn save(path: &Path, settings: &AppSettings) -> Result<(), SettingsError> {
    let text = to_text(settings)?;
    let write_error = |source| SettingsError::Write { path: path.to_owned(), source };
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(write_error)?;
    }
    crate::build::disk::write_atomic(path, &text).map_err(write_error)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("cascade-settings-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn xdg_config_home_wins_over_home() {
        let path = path_from(Some("/cfg".into()), Some("/home/me".into())).expect("a path");
        assert_eq!(path, PathBuf::from("/cfg/cascade/settings.json"));
    }

    #[test]
    fn home_dot_config_without_xdg_or_with_a_relative_one() {
        let expected = PathBuf::from("/home/me/.config/cascade/settings.json");
        assert_eq!(path_from(None, Some("/home/me".into())).expect("a path"), expected);
        assert_eq!(path_from(Some("relative".into()), Some("/home/me".into())).expect("a path"), expected);
        assert_eq!(path_from(Some("".into()), Some("/home/me".into())).expect("a path"), expected);
    }

    #[test]
    fn no_home_and_no_xdg_is_an_error() {
        assert!(matches!(path_from(None, None), Err(SettingsError::NoConfigDir)));
        assert!(matches!(path_from(None, Some("".into())), Err(SettingsError::NoConfigDir)));
    }

    #[test]
    fn pills_are_on_by_default() {
        assert!(AppSettings::default().transition_pills);
        let path = Path::new("s.json");
        assert_eq!(parse("{}", path).expect("parses"), AppSettings::default());
        assert!(!parse(r#"{ "transition_pills": false }"#, path).expect("parses").transition_pills);
        assert!(parse(r#"{ "transition_pills": true, "later": 1 }"#, path).is_ok(), "unknown keys are ignored");
    }

    #[test]
    fn invalid_text_is_a_typed_parse_error() {
        let path = Path::new("s.json");
        assert!(matches!(parse("not json", path), Err(SettingsError::Parse { .. })));
        assert!(matches!(parse(r#"{ "transition_pills": "no" }"#, path), Err(SettingsError::Parse { .. })));
    }

    #[test]
    fn save_then_load_round_trips_and_creates_the_directory() {
        let dir = temp_dir("roundtrip");
        let path = dir.join("cascade").join("settings.json");
        let off = AppSettings { transition_pills: false };
        save(&path, &off).expect("saves");
        assert_eq!(load(&path).expect("loads"), off);
        assert_eq!(std::fs::read_to_string(&path).expect("reads"), "{\n  \"transition_pills\": false\n}\n");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_or_invalid_files_fall_back_to_defaults() {
        let dir = temp_dir("fallback");
        let path = dir.join("settings.json");
        assert!(matches!(load(&path), Err(SettingsError::Read { .. })));
        assert_eq!(load_or_default(Ok(&path)), AppSettings::default());
        std::fs::create_dir_all(&dir).expect("dir");
        std::fs::write(&path, "{ broken").expect("writes");
        assert_eq!(load_or_default(Ok(&path)), AppSettings::default());
        assert_eq!(load_or_default(Err(&SettingsError::NoConfigDir)), AppSettings::default());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
