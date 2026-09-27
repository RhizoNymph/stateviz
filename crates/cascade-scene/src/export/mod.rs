//! Scene → image files.
//!
//! Owner: `feat/view-scenes` implements SVG (every shape, dash, arrow,
//! badge, opacity) and PNG (rasterised SVG).

use crate::scene::Scene;

#[derive(Debug, thiserror::Error)]
pub enum ExportError {
    #[error("{0} export is not implemented yet")]
    NotImplemented(&'static str),
}

/// Render a scene as a standalone SVG document.
///
/// Stub until `feat/view-scenes` lands.
pub fn to_svg(scene: &Scene) -> Result<String, ExportError> {
    let _ = scene;
    Err(ExportError::NotImplemented("SVG"))
}

/// Render a scene as PNG bytes at `scale` device pixels per scene unit.
///
/// Stub until `feat/view-scenes` lands.
pub fn to_png(scene: &Scene, scale: f32) -> Result<Vec<u8>, ExportError> {
    let _ = (scene, scale);
    Err(ExportError::NotImplemented("PNG"))
}
