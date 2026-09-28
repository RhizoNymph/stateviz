//! Connect handles (edit mode): a small circle on the east edge of every
//! connectable node (states, transition pills, controllers, external
//! sources) that the app starts a connect drag from.
//!
//! Handles are `Overlay::Rect`s with `HitTarget::ConnectHandle`, painted
//! over nodes and winning hit tests over them (`Scene::hit_test`). SVG and
//! PNG export leave them out: they are an editing affordance, not part of
//! the picture.

use cascade_core::ElementKey;
use cascade_layout::Rect;

use crate::color::Theme;
use crate::scene::{HitTarget, Layer, Overlay, Scene, Stroke};

/// Radius of a connect handle.
pub(crate) const HANDLE_RADIUS: f32 = 4.5;

/// Whether drags can start from this kind of element.
fn connectable(key: &ElementKey) -> bool {
    matches!(
        key,
        ElementKey::State { .. }
            | ElementKey::Transition { .. }
            | ElementKey::Controller { .. }
            | ElementKey::External { .. }
    )
}

/// Add a handle for every connectable node, following its opacity.
pub(crate) fn add_handles(scene: &mut Scene, theme: &Theme) {
    let handles: Vec<Overlay> = scene
        .nodes
        .iter()
        .filter_map(|node| match &node.target {
            HitTarget::Element(key) if connectable(key) => {
                let (x, y) = (node.rect.right(), node.rect.center().y);
                Some(Overlay::Rect {
                    rect: Rect::new(x - HANDLE_RADIUS, y - HANDLE_RADIUS, 2.0 * HANDLE_RADIUS, 2.0 * HANDLE_RADIUS),
                    fill: Some(theme.background),
                    stroke: Some(Stroke::solid(theme.text_muted, theme.stroke_width)),
                    radius: HANDLE_RADIUS,
                    opacity: node.opacity,
                    layer: Layer::Over,
                    target: HitTarget::ConnectHandle { element: key.clone() },
                })
            }
            _ => None,
        })
        .collect();
    scene.overlays.extend(handles);
}
