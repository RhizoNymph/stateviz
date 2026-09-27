//! Pointer gestures on the canvas: click, pan, node drag.
//!
//! Pure state machine over screen positions. A press on the background or
//! with a modifier becomes a pan once the pointer moves past
//! [`DRAG_THRESHOLD`]; a plain press on a pinnable node becomes a node drag.
//! A release without moving is a click, classified by [`classify_click`].

use cascade_core::ElementKey;
use cascade_layout::Point;

use crate::viewport::ScreenPoint;

/// Screen pixels the pointer must travel before a press becomes a drag.
pub const DRAG_THRESHOLD: f32 = 4.0;

/// The modifiers the canvas cares about. `secondary` is cmd on macOS and
/// ctrl elsewhere.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mods {
    pub shift: bool,
    pub alt: bool,
    pub secondary: bool,
}

impl Mods {
    pub const fn any(self) -> bool {
        self.shift || self.alt || self.secondary
    }
}

/// What a press landed on, resolved by the host from the scene.
#[derive(Clone, Debug, PartialEq)]
pub enum Pick {
    /// A model element (directly, or through a stub, step or lifeline).
    Element(ElementKey),
    /// A matrix cell.
    Pair { row: String, column: String },
    /// Background or decoration.
    Nothing,
}

/// A node that may be dragged to pin it: its key and top-left corner in
/// scene coordinates when the press started.
#[derive(Clone, Debug, PartialEq)]
pub struct Draggable {
    pub key: ElementKey,
    pub origin: Point,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub enum Gesture {
    #[default]
    Idle,
    /// Button down, not yet moved past the threshold.
    Pressed { start: ScreenPoint, pick: Pick, draggable: Option<Draggable>, mods: Mods, clicks: usize },
    /// Dragging the background.
    Panning { last: ScreenPoint },
    /// Dragging a node; `zoom` is fixed for the gesture.
    Dragging { node: Draggable, start: ScreenPoint, current: ScreenPoint, zoom: f32 },
}

/// What a pointer move means for the host.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MoveOutcome {
    Nothing,
    /// Pan the viewport by this screen delta.
    Pan(f32, f32),
    /// The dragged node moved; repaint the preview.
    DragPreview,
}

/// What a release means for the host.
#[derive(Clone, Debug, PartialEq)]
pub enum ReleaseOutcome {
    Nothing,
    Click {
        pick: Pick,
        mods: Mods,
        clicks: usize,
    },
    /// Pin `key` with its top-left corner at `top_left` (scene coordinates).
    Drop {
        key: ElementKey,
        top_left: Point,
    },
}

impl Gesture {
    /// Start a gesture. `draggable` is honoured only without modifiers, so
    /// modifier clicks never move nodes.
    pub fn press(start: ScreenPoint, pick: Pick, draggable: Option<Draggable>, mods: Mods, clicks: usize) -> Self {
        let draggable = draggable.filter(|_| !mods.any());
        Gesture::Pressed { start, pick, draggable, mods, clicks }
    }

    pub fn moved(&mut self, at: ScreenPoint, zoom: f32) -> MoveOutcome {
        match self {
            Gesture::Idle => MoveOutcome::Nothing,
            Gesture::Pressed { start, draggable, .. } => {
                if start.distance(at) < DRAG_THRESHOLD {
                    return MoveOutcome::Nothing;
                }
                let start = *start;
                match draggable.take() {
                    Some(node) => {
                        *self = Gesture::Dragging { node, start, current: at, zoom };
                        MoveOutcome::DragPreview
                    }
                    None => {
                        *self = Gesture::Panning { last: at };
                        MoveOutcome::Pan(at.x - start.x, at.y - start.y)
                    }
                }
            }
            Gesture::Panning { last } => {
                let (dx, dy) = (at.x - last.x, at.y - last.y);
                *last = at;
                MoveOutcome::Pan(dx, dy)
            }
            Gesture::Dragging { current, .. } => {
                *current = at;
                MoveOutcome::DragPreview
            }
        }
    }

    pub fn release(&mut self) -> ReleaseOutcome {
        match std::mem::take(self) {
            Gesture::Idle | Gesture::Panning { .. } => ReleaseOutcome::Nothing,
            Gesture::Pressed { pick, mods, clicks, .. } => ReleaseOutcome::Click { pick, mods, clicks },
            Gesture::Dragging { node, start, current, zoom } => {
                ReleaseOutcome::Drop { top_left: drag_top_left(node.origin, start, current, zoom), key: node.key }
            }
        }
    }

    /// The node being dragged and where its top-left corner is now.
    pub fn drag_preview(&self) -> Option<(&ElementKey, Point)> {
        match self {
            Gesture::Dragging { node, start, current, zoom } => {
                Some((&node.key, drag_top_left(node.origin, *start, *current, *zoom)))
            }
            _ => None,
        }
    }

    pub fn is_active(&self) -> bool {
        !matches!(self, Gesture::Idle)
    }
}

/// Where a dragged node's top-left corner lands.
pub fn drag_top_left(origin: Point, start: ScreenPoint, current: ScreenPoint, zoom: f32) -> Point {
    let zoom = if zoom > 0.0 { zoom } else { 1.0 };
    Point::new(origin.x + (current.x - start.x) / zoom, origin.y + (current.y - start.y) / zoom)
}

/// What a click asks for.
#[derive(Clone, Debug, PartialEq)]
pub enum ClickAction {
    Select(ElementKey),
    AddToSelection(ElementKey),
    OpenSource(ElementKey),
    Unpin(ElementKey),
    OpenPair { row: String, column: String },
    Nothing,
}

/// Double-click or secondary-click opens the source; alt-click unpins;
/// shift-click adds a second selection; a plain click selects. A matrix
/// cell always opens its machine pair.
pub fn classify_click(pick: Pick, mods: Mods, clicks: usize) -> ClickAction {
    match pick {
        Pick::Nothing => ClickAction::Nothing,
        Pick::Pair { row, column } => ClickAction::OpenPair { row, column },
        Pick::Element(key) if mods.secondary || clicks >= 2 => ClickAction::OpenSource(key),
        Pick::Element(key) if mods.alt => ClickAction::Unpin(key),
        Pick::Element(key) if mods.shift => ClickAction::AddToSelection(key),
        Pick::Element(key) => ClickAction::Select(key),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key() -> ElementKey {
        ElementKey::Event { event: "OrderPaid".into() }
    }

    fn at(x: f32, y: f32) -> ScreenPoint {
        ScreenPoint::new(x, y)
    }

    fn node() -> Draggable {
        Draggable { key: key(), origin: Point::new(100.0, 50.0) }
    }

    #[test]
    fn press_and_release_in_place_is_a_click() {
        let mut g = Gesture::press(at(10.0, 10.0), Pick::Element(key()), Some(node()), Mods::default(), 1);
        assert_eq!(g.moved(at(11.0, 12.0), 1.0), MoveOutcome::Nothing);
        assert_eq!(g.release(), ReleaseOutcome::Click { pick: Pick::Element(key()), mods: Mods::default(), clicks: 1 });
        assert_eq!(g, Gesture::Idle);
    }

    #[test]
    fn background_drag_pans_incrementally() {
        let mut g = Gesture::press(at(0.0, 0.0), Pick::Nothing, None, Mods::default(), 1);
        assert_eq!(g.moved(at(10.0, 0.0), 1.0), MoveOutcome::Pan(10.0, 0.0));
        assert_eq!(g.moved(at(15.0, -5.0), 1.0), MoveOutcome::Pan(5.0, -5.0));
        assert_eq!(g.release(), ReleaseOutcome::Nothing);
    }

    #[test]
    fn node_drag_drops_at_the_scaled_offset() {
        let mut g = Gesture::press(at(0.0, 0.0), Pick::Element(key()), Some(node()), Mods::default(), 1);
        assert_eq!(g.moved(at(20.0, 10.0), 2.0), MoveOutcome::DragPreview);
        assert_eq!(g.moved(at(40.0, 20.0), 2.0), MoveOutcome::DragPreview);
        assert_eq!(g.drag_preview(), Some((&key(), Point::new(120.0, 60.0))));
        assert_eq!(g.release(), ReleaseOutcome::Drop { key: key(), top_left: Point::new(120.0, 60.0) });
    }

    #[test]
    fn modifier_press_on_a_node_pans_instead_of_dragging() {
        let mods = Mods { shift: true, ..Mods::default() };
        let mut g = Gesture::press(at(0.0, 0.0), Pick::Element(key()), Some(node()), mods, 1);
        assert_eq!(g.moved(at(10.0, 0.0), 1.0), MoveOutcome::Pan(10.0, 0.0));
        assert!(g.drag_preview().is_none());
    }

    #[test]
    fn release_when_idle_does_nothing() {
        let mut g = Gesture::Idle;
        assert_eq!(g.moved(at(1.0, 1.0), 1.0), MoveOutcome::Nothing);
        assert_eq!(g.release(), ReleaseOutcome::Nothing);
        assert!(!g.is_active());
    }

    #[test]
    fn click_classification() {
        let el = || Pick::Element(key());
        assert_eq!(classify_click(el(), Mods::default(), 1), ClickAction::Select(key()));
        assert_eq!(classify_click(el(), Mods::default(), 2), ClickAction::OpenSource(key()));
        assert_eq!(
            classify_click(el(), Mods { secondary: true, ..Mods::default() }, 1),
            ClickAction::OpenSource(key())
        );
        assert_eq!(classify_click(el(), Mods { alt: true, ..Mods::default() }, 1), ClickAction::Unpin(key()));
        assert_eq!(
            classify_click(el(), Mods { shift: true, ..Mods::default() }, 1),
            ClickAction::AddToSelection(key())
        );
        assert_eq!(
            classify_click(Pick::Pair { row: "A".into(), column: "B".into() }, Mods::default(), 1),
            ClickAction::OpenPair { row: "A".into(), column: "B".into() }
        );
        assert_eq!(classify_click(Pick::Nothing, Mods::default(), 2), ClickAction::Nothing);
    }

    #[test]
    fn drag_with_zero_zoom_does_not_divide_by_zero() {
        let p = drag_top_left(Point::new(1.0, 1.0), at(0.0, 0.0), at(2.0, 2.0), 0.0);
        assert_eq!(p, Point::new(3.0, 3.0));
    }
}
