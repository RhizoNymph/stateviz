//! Scene → image files: standalone SVG, and PNG rasterised from that SVG
//! with resvg. Both draw exactly what the scene holds, the same picture the
//! app paints.

mod png;
mod svg;

use crate::scene::Scene;

#[derive(Debug, thiserror::Error)]
pub enum ExportError {
    /// A coordinate, size or width in the scene is NaN or infinite.
    #[error("the scene has non-finite geometry")]
    NonFinite,
    #[error("PNG scale must be a positive finite number, got {0}")]
    InvalidScale(f32),
    #[error("a {width}×{height} pixel image is too large to export")]
    TooLarge { width: u64, height: u64 },
    #[error("writing SVG failed")]
    Format(#[from] std::fmt::Error),
    #[error("rasterising the SVG failed: {0}")]
    Svg(#[from] resvg::usvg::Error),
    #[error("encoding PNG failed: {0}")]
    Png(#[source] Box<dyn std::error::Error + Send + Sync>),
}

/// Render a scene as a standalone SVG document: every shape, border, dash,
/// arrowhead, label, badge, overlay, opacity and the background.
pub fn to_svg(scene: &Scene) -> Result<String, ExportError> {
    svg::to_svg(scene)
}

/// Render a scene as PNG bytes at `scale` device pixels per scene unit.
pub fn to_png(scene: &Scene, scale: f32) -> Result<Vec<u8>, ExportError> {
    png::to_png(scene, scale)
}
