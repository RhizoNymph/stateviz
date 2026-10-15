//! Connect handles (edit mode): a small circle on the east edge of every
//! connectable node (states, transition pills, controllers, external
//! sources) that the app starts a connect drag from, and a subscribe handle
//! on the point of every event tag, dragged onto a transition to subscribe
//! it to the event.
//!
//! Handles are `Overlay::Rect`s with `HitTarget::ConnectHandle` or
//! `HitTarget::SubscribeHandle`, painted over nodes and winning hit tests
//! over them (`Scene::hit_test`). SVG and PNG export leave connect handles
//! out (an editing affordance, not part of the picture); subscribe handles
//! stay, marking the point of each tag where its subscriptions leave.

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

/// Add a connect handle for every connectable node and a subscribe handle
/// for every event tag, following the node's opacity.
pub(crate) fn add_handles(scene: &mut Scene, theme: &Theme) {
    let handles: Vec<Overlay> = scene
        .nodes
        .iter()
        .filter_map(|node| {
            let target = match &node.target {
                HitTarget::Element(key) if connectable(key) => HitTarget::ConnectHandle { element: key.clone() },
                // The tag's point (its east end) is where a subscription
                // leaves for the transition it drives.
                HitTarget::Element(ElementKey::Event { event }) => HitTarget::SubscribeHandle { event: event.clone() },
                _ => return None,
            };
            let (x, y) = (node.rect.right(), node.rect.center().y);
            Some(Overlay::Rect {
                rect: Rect::new(x - HANDLE_RADIUS, y - HANDLE_RADIUS, 2.0 * HANDLE_RADIUS, 2.0 * HANDLE_RADIUS),
                fill: Some(theme.background),
                stroke: Some(Stroke::solid(theme.text_muted, theme.stroke_width)),
                radius: HANDLE_RADIUS,
                opacity: node.opacity,
                layer: Layer::Over,
                target,
            })
        })
        .collect();
    scene.overlays.extend(handles);
}
