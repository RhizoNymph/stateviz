//! Pinned node positions, saved next to the definition.
//!
//! Dragging a node pins it. Pins are per view and keyed by element key, and
//! live in a sidecar file: `cascade.yaml` → `cascade.layout.json`,
//! `shop.cascade.yaml` → `shop.cascade.layout.json`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use cascade_core::ElementKey;
use cascade_layout::Point;

use crate::view_state::ViewKind;

pub const SIDECAR_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LayoutSidecar {
    pub version: u32,
    /// Top-left corner of each pinned node, per view.
    #[serde(default)]
    pub pins: BTreeMap<ViewKind, BTreeMap<ElementKey, Point>>,
}

impl Default for LayoutSidecar {
    fn default() -> Self {
        Self { version: SIDECAR_VERSION, pins: BTreeMap::new() }
    }
}

impl LayoutSidecar {
    pub fn pins_for(&self, view: ViewKind) -> impl Iterator<Item = (&ElementKey, Point)> {
        self.pins.get(&view).into_iter().flatten().map(|(k, p)| (k, *p))
    }

    pub fn pin(&mut self, view: ViewKind, key: ElementKey, at: Point) {
        self.pins.entry(view).or_default().insert(key, at);
    }

    pub fn unpin(&mut self, view: ViewKind, key: &ElementKey) {
        if let Some(pins) = self.pins.get_mut(&view) {
            pins.remove(key);
            if pins.is_empty() {
                self.pins.remove(&view);
            }
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SidecarError {
    #[error("cannot read {}: {source}", path.display())]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("cannot write {}: {source}", path.display())]
    Write {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("{} is not a valid layout sidecar: {source}", path.display())]
    Parse {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("{} has unsupported version {found} (expected {SIDECAR_VERSION})", path.display())]
    Version { path: PathBuf, found: u32 },
}

/// The sidecar path for a definition file.
pub fn sidecar_path(definition: &Path) -> PathBuf {
    let stem = definition.file_stem().map_or_else(|| "cascade".into(), |s| s.to_string_lossy().into_owned());
    definition.with_file_name(format!("{stem}.layout.json"))
}

/// Load the sidecar; a missing file is an empty sidecar.
pub fn load_sidecar(path: &Path) -> Result<LayoutSidecar, SidecarError> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(LayoutSidecar::default()),
        Err(source) => {
            return Err(SidecarError::Read { path: path.to_owned(), source });
        }
    };
    let sidecar: LayoutSidecar =
        serde_json::from_str(&text).map_err(|source| SidecarError::Parse { path: path.to_owned(), source })?;
    if sidecar.version != SIDECAR_VERSION {
        return Err(SidecarError::Version { path: path.to_owned(), found: sidecar.version });
    }
    Ok(sidecar)
}

/// Write the sidecar atomically (write to a temporary file, then rename).
pub fn save_sidecar(path: &Path, sidecar: &LayoutSidecar) -> Result<(), SidecarError> {
    let write_err = |source| SidecarError::Write { path: path.to_owned(), source };
    let text = serde_json::to_string_pretty(sidecar)
        .map_err(|source| SidecarError::Parse { path: path.to_owned(), source })?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, text + "\n").map_err(write_err)?;
    std::fs::rename(&tmp, path).map_err(write_err)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sidecar_names() {
        assert_eq!(sidecar_path(Path::new("/x/cascade.yaml")), PathBuf::from("/x/cascade.layout.json"));
        assert_eq!(sidecar_path(Path::new("shop.cascade.yaml")), PathBuf::from("shop.cascade.layout.json"));
    }

    #[test]
    fn json_shape_is_stable() {
        let mut sidecar = LayoutSidecar::default();
        sidecar.pin(ViewKind::Causal, ElementKey::Event { event: "Paid".into() }, Point::new(10.0, 20.0));
        let json = serde_json::to_string(&sidecar).expect("serializes");
        assert_eq!(json, r#"{"version":1,"pins":{"causal":{"event:Paid":{"x":10.0,"y":20.0}}}}"#);
        let back: LayoutSidecar = serde_json::from_str(&json).expect("parses");
        assert_eq!(back, sidecar);
        sidecar.unpin(ViewKind::Causal, &ElementKey::Event { event: "Paid".into() });
        assert!(sidecar.pins.is_empty());
    }
}
