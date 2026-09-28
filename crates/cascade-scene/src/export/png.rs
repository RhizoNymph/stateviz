//! Scene → PNG, by rasterising the SVG export with resvg.
//!
//! System fonts are loaded once per process; the SVG asks for a monospace
//! font, which resolves to the first monospaced system face (DejaVu Sans
//! Mono when installed). Without fonts, shapes still render and text is
//! skipped.

use std::sync::{Arc, OnceLock};

use resvg::tiny_skia::{Pixmap, Transform};
use resvg::usvg::{self, fontdb};

use crate::export::ExportError;
use crate::export::svg;
use crate::scene::Scene;

/// Largest side of an exported PNG, in pixels.
const MAX_SIDE: u64 = 16_384;
/// Largest exported PNG, in pixels.
const MAX_PIXELS: u64 = 100_000_000;

pub(crate) fn to_png(scene: &Scene, scale: f32) -> Result<Vec<u8>, ExportError> {
    if !scale.is_finite() || scale <= 0.0 {
        return Err(ExportError::InvalidScale(scale));
    }
    let text = svg::to_svg(scene)?;
    let options = usvg::Options { fontdb: fonts(), ..usvg::Options::default() };
    let tree = usvg::Tree::from_str(&text, &options)?;
    let size = tree.size();
    let width = pixels(size.width(), scale);
    let height = pixels(size.height(), scale);
    if width > MAX_SIDE || height > MAX_SIDE || width.saturating_mul(height) > MAX_PIXELS {
        return Err(ExportError::TooLarge { width, height });
    }
    let (Ok(w), Ok(h)) = (u32::try_from(width), u32::try_from(height)) else {
        return Err(ExportError::TooLarge { width, height });
    };
    let mut pixmap = Pixmap::new(w, h).ok_or(ExportError::TooLarge { width, height })?;
    resvg::render(&tree, Transform::from_scale(scale, scale), &mut pixmap.as_mut());
    pixmap.encode_png().map_err(|err| ExportError::Png(Box::new(err)))
}

fn pixels(extent: f32, scale: f32) -> u64 {
    let v = (f64::from(extent) * f64::from(scale)).ceil();
    if v.is_finite() && v >= 1.0 { v.min(u64::MAX as f64) as u64 } else { 1 }
}

/// System fonts, loaded on first use.
fn fonts() -> Arc<fontdb::Database> {
    static FONTS: OnceLock<Arc<fontdb::Database>> = OnceLock::new();
    Arc::clone(FONTS.get_or_init(|| {
        let mut db = fontdb::Database::new();
        db.load_system_fonts();
        let family_of = |face: &fontdb::FaceInfo| face.families.first().map(|(name, _)| name.clone());
        let preferred = db
            .faces()
            .filter(|f| f.monospaced)
            .filter_map(family_of)
            .min_by_key(|name| (name != "DejaVu Sans Mono", name.clone()));
        if let Some(family) = preferred {
            db.set_monospace_family(family);
        }
        Arc::new(db)
    }))
}
