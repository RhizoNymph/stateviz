//! Pointer gestures on the canvas: click, pan, node drag, connect.
//!
//! Pure state machine over screen positions. A press on the background or
//! with a modifier becomes a pan once the pointer moves past
//! [`DRAG_THRESHOLD`]; a plain press on a pinnable node becomes a node drag;
//! a press on a build-mode connect handle ([`Gesture::press_handle`])
//! becomes a connect drag with a rubber band. A release without moving is a
//! click, classified by [`classify_click`].

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
    /// Button down, not yet moved past the threshold. `handle` is the
    /// element whose connect handle was pressed.
    Pressed {
        start: ScreenPoint,
        pick: Pick,
        draggable: Option<Draggable>,
        mods: Mods,
        clicks: usize,
        handle: Option<Box<ElementKey>>,
    },
    /// Dragging the background.
    Panning { last: ScreenPoint },
    /// Dragging a node; `zoom` is fixed for the gesture.
    Dragging { node: Draggable, start: ScreenPoint, current: ScreenPoint, zoom: f32 },
    /// Dragging a rubber band from a connect handle.
    Connecting { from: ElementKey, start: ScreenPoint, current: ScreenPoint },
}

/// What a pointer move means for the host.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MoveOutcome {
    Nothing,
    /// Pan the viewport by this screen delta.
    Pan(f32, f32),
    /// The dragged node moved; repaint the preview.
    DragPreview,
    /// The rubber band moved; repaint it and the drop highlight.
    ConnectPreview,
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
    /// A connect drag from `from` ended at `at` (screen coordinates); the
    /// host hit tests there for the target.
    Connect {
        from: ElementKey,
        at: ScreenPoint,
    },
}

impl Gesture {
    /// Start a gesture. `draggable` is honoured only without modifiers, so
    /// modifier clicks never move nodes.
    pub fn press(start: ScreenPoint, pick: Pick, draggable: Option<Draggable>, mods: Mods, clicks: usize) -> Self {
        let draggable = draggable.filter(|_| !mods.any());
        Gesture::Pressed { start, pick, draggable, mods, clicks, handle: None }
    }

    /// A press on the connect handle of `element`: a drag connects, a click
    /// selects the element.
    pub fn press_handle(start: ScreenPoint, element: ElementKey, mods: Mods, clicks: usize) -> Self {
        Gesture::Pressed {
            start,
            pick: Pick::Element(element.clone()),
            draggable: None,
            mods,
            clicks,
            handle: Some(Box::new(element)),
        }
    }

    pub fn moved(&mut self, at: ScreenPoint, zoom: f32) -> MoveOutcome {
        match self {
            Gesture::Idle => MoveOutcome::Nothing,
            Gesture::Pressed { start, draggable, handle, .. } => {
                if start.distance(at) < DRAG_THRESHOLD {
                    return MoveOutcome::Nothing;
                }
                let start = *start;
                if let Some(from) = handle.take() {
                    *self = Gesture::Connecting { from: *from, start, current: at };
                    return MoveOutcome::ConnectPreview;
                }
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
            Gesture::Connecting { current, .. } => {
                *current = at;
                MoveOutcome::ConnectPreview
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
            Gesture::Connecting { from, current, .. } => ReleaseOutcome::Connect { from, at: current },
        }
    }

    /// The rubber band: the element dragged from, where the drag started and
    /// where the pointer is.
    pub fn connect_preview(&self) -> Option<(&ElementKey, ScreenPoint, ScreenPoint)> {
        match self {
            Gesture::Connecting { from, start, current } => Some((from, *start, *current)),
            _ => None,
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

    #[test]
    fn dragging_a_connect_handle_draws_a_rubber_band() {
        let from = ElementKey::State { machine: "Order".into(), path: "draft".into() };
        let mut g = Gesture::press_handle(at(0.0, 0.0), from.clone(), Mods::default(), 1);
        assert_eq!(g.moved(at(2.0, 0.0), 1.0), MoveOutcome::Nothing, "below the threshold");
        assert!(g.connect_preview().is_none());
        assert_eq!(g.moved(at(30.0, 5.0), 1.0), MoveOutcome::ConnectPreview);
        assert_eq!(g.moved(at(60.0, 10.0), 2.0), MoveOutcome::ConnectPreview);
        assert_eq!(g.connect_preview(), Some((&from, at(0.0, 0.0), at(60.0, 10.0))));
        assert!(g.drag_preview().is_none());
        assert_eq!(g.release(), ReleaseOutcome::Connect { from, at: at(60.0, 10.0) });
        assert_eq!(g, Gesture::Idle);
    }

    #[test]
    fn clicking_a_connect_handle_selects_its_element() {
        let from = ElementKey::Controller { controller: "Fulfil".into() };
        let mut g = Gesture::press_handle(at(5.0, 5.0), from.clone(), Mods::default(), 1);
        assert_eq!(g.release(), ReleaseOutcome::Click { pick: Pick::Element(from), mods: Mods::default(), clicks: 1 });
    }
}
