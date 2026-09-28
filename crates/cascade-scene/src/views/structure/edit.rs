//! Edit-mode extras on the structure view after layout: the wiring band's
//! lanes and the empty-state hints that make building from scratch work.

use cascade_core::{ElementRef, Model};
use cascade_layout::{Point, Rect};

use crate::scene::{FontWeight, HitTarget, Label, Lane, Layer, Overlay, Scene, Stroke};
use crate::views::structure::machines::MachinePlan;
use crate::views::structure::wiring::{SOURCES_TITLE, WIRING_TITLE, WiringPlan};
use crate::views::style::Painter;

/// Shown (and noted) when the definition has no machines.
pub(super) const EMPTY_NOTE: &str = "Empty system: add a machine to start building.";
/// Shown in the header of a machine lane without transitions.
pub(super) const NO_TRANSITIONS_HINT: &str = "no transitions yet: drag between state handles";
/// Gap between a lane title and its hint.
const HINT_GAP: f32 = 16.0;

/// Neutral lanes for the band's groups (only those holding nodes).
pub(super) fn band_lanes(
    scene: &mut Scene,
    painter: &Painter<'_>,
    band: &WiringPlan,
    group_rect: impl Fn(usize) -> Option<Rect>,
) {
    let theme = painter.theme;
    for (group, title) in [(band.sources, SOURCES_TITLE), (band.wiring, WIRING_TITLE)] {
        let Some(rect) = group_rect(group) else { continue };
        scene.lanes.push(Lane {
            target: HitTarget::None,
            rect,
            fill: theme.neutral.mix(theme.background, 0.92),
            stroke: Stroke::solid(theme.rule, 1.0),
            title: Label {
                text: title.to_owned(),
                origin: Point::new(rect.left() + 12.0, rect.top() + 5.0),
                font_size: theme.font_size,
                color: theme.text_muted,
                weight: FontWeight::Bold,
            },
            opacity: 1.0,
            collapsed: false,
        });
    }
}

/// The empty-definition note, and a hint in the header of every expanded
/// machine lane that has no transitions yet.
pub(super) fn hints(
    scene: &mut Scene,
    model: &Model,
    painter: &Painter<'_>,
    plans: &[MachinePlan],
    notes: &mut Vec<String>,
) {
    let theme = painter.theme;
    if model.machine_count() == 0 {
        notes.push(EMPTY_NOTE.to_owned());
        let height = painter.line_height(theme.font_size);
        let origin = Point::new(scene.bounds.left(), scene.bounds.top() - height - 12.0);
        scene.overlays.push(Overlay::Text {
            label: Label {
                text: EMPTY_NOTE.to_owned(),
                origin,
                font_size: theme.font_size,
                color: theme.text_muted,
                weight: FontWeight::Normal,
            },
            opacity: 1.0,
            layer: Layer::Over,
        });
    }
    for plan in plans {
        let MachinePlan::Expanded { machine, .. } = plan else { continue };
        if !model.machine(*machine).transitions.is_empty() {
            continue;
        }
        let target = HitTarget::Element(model.key_of(ElementRef::Machine(*machine)));
        let Some(lane) = scene.lanes.iter().find(|l| l.target == target) else { continue };
        let title = &lane.title;
        let small = theme.small_font_size;
        let origin = Point::new(
            title.origin.x + painter.text_width(&title.text, title.font_size) + HINT_GAP,
            title.origin.y + (painter.line_height(title.font_size) - painter.line_height(small)) / 2.0,
        );
        let opacity = lane.opacity;
        scene.overlays.push(Overlay::Text {
            label: Label {
                text: NO_TRANSITIONS_HINT.to_owned(),
                origin,
                font_size: small,
                color: theme.text_muted,
                weight: FontWeight::Normal,
            },
            opacity,
            layer: Layer::Over,
        });
    }
}
