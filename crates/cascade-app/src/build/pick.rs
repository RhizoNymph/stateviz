//! What a press or a drop on the canvas lands on (pure).
//!
//! Hit testing runs in scene units, so pick distances given in screen
//! pixels are divided by the zoom. Normally the canvas uses
//! `Scene::hit_test` with [`HIT_TOLERANCE_PX`] for edges. On the build
//! canvas with transition arrows (pills off) it uses
//! `Scene::hit_test_arrows`: a press within [`ARROW_PICK_PX`] of a
//! transition arrow is a press on that transition's connect handle (the
//! whole arrow is the handle), and a connect drag dropped there targets the
//! transition. Connect handles and nodes still win over arrows.

use cascade_core::ElementKey;
use cascade_layout::Point;
use cascade_scene::{HitTarget, Scene, ViewKind, ViewState};

use crate::build::connect;
use crate::gesture::Mods;
use crate::mode::AppMode;

/// Edge pick distance in screen pixels.
pub const HIT_TOLERANCE_PX: f32 = 5.0;
/// How close to a transition arrow, in screen pixels, a press starts a
/// connect drag from it (or a drop targets it) on the build canvas.
pub const ARROW_PICK_PX: f32 = 8.0;

/// A distance in screen pixels as scene units at `zoom`. A zoom that is
/// not a positive number counts as 1.
pub fn scene_units(px: f32, zoom: f32) -> f32 {
    if zoom.is_finite() && zoom > 0.0 { px / zoom } else { px }
}

/// How the canvas hit tests.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Picking {
    /// `Scene::hit_test`.
    Plain,
    /// The build canvas with transition arrows: arrows are connect handles.
    Arrows,
}

impl Picking {
    /// Arrows only on the build canvas (Build mode, structure view) with
    /// transition pills off.
    pub fn for_view(mode: AppMode, view: &ViewState) -> Picking {
        if mode == AppMode::Build && view.view == ViewKind::Structure && !view.transition_pills {
            Picking::Arrows
        } else {
            Picking::Plain
        }
    }
}

/// What is under scene point `p` at `zoom`.
pub fn hit(scene: &Scene, p: Point, zoom: f32, picking: Picking) -> Option<HitTarget> {
    let tolerance = scene_units(HIT_TOLERANCE_PX, zoom);
    match picking {
        Picking::Plain => scene.hit_test(p, tolerance).cloned(),
        Picking::Arrows => scene.hit_test_arrows(p, tolerance, scene_units(ARROW_PICK_PX, zoom)),
    }
}

/// The element a press on `target` starts a connect drag from: a connect
/// handle of a drag source, pressed in Build mode without modifiers.
/// Anything else is an ordinary press (click, pin drag or pan).
pub fn connect_source(target: Option<&HitTarget>, mode: AppMode, mods: Mods) -> Option<ElementKey> {
    match target {
        Some(HitTarget::ConnectHandle { element })
            if mode == AppMode::Build && connect::is_source(element) && !mods.any() =>
        {
            Some(element.clone())
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use cascade_core::edit::EditOp;
    use cascade_core::{Definition, parse_definition};
    use cascade_layout::Rect;
    use cascade_scene::{Arrow, Border, EdgeKind, Emphasis, Rgba, SceneEdge, SceneNode, Shape, Stroke};

    use super::*;
    use crate::gesture::{ClickAction, Gesture, Pick, ReleaseOutcome, classify_click};
    use crate::locate::drop_key;
    use crate::viewport::ScreenPoint;

    const TEXT: &str = "\
machines:
  Order:
    states: [draft, paid]
    transitions:
      - { from: draft, to: paid, on: pay }
  Shipment:
    states: [idle, moving]
    transitions:
      - { from: idle, to: moving, on: start }
controllers:
  Fulfil:
    on:
      PayDone:
        - fire: Shipment.start
external:
  Clock: [Order.pay]
";

    fn def() -> Definition {
        parse_definition(TEXT).expect("parses")
    }

    fn key(s: &str) -> ElementKey {
        s.parse().expect("valid key")
    }

    fn pay() -> ElementKey {
        key("transition:Order:draft->paid@pay")
    }

    fn start() -> ElementKey {
        key("transition:Shipment:idle->moving@start")
    }

    fn node(key: ElementKey, rect: Rect) -> SceneNode {
        SceneNode {
            target: HitTarget::Element(key),
            shape: Shape::Rect,
            rect,
            fill: None,
            stroke: Stroke::solid(Rgba::hex(0), 1.0),
            border: Border::Single,
            labels: Vec::new(),
            badge: None,
            opacity: 1.0,
            emphasis: Emphasis::Normal,
            diff: None,
        }
    }

    fn arrow(key: ElementKey, y: f32) -> SceneEdge {
        SceneEdge {
            target: HitTarget::Element(key),
            kind: EdgeKind::Transition,
            points: vec![Point::new(0.0, y), Point::new(200.0, y)],
            stroke: Stroke::solid(Rgba::hex(0), 1.0),
            arrow: Arrow::End,
            label: None,
            opacity: 1.0,
            emphasis: Emphasis::Normal,
            back_edge: false,
            diff: None,
        }
    }

    /// The build canvas in arrow mode: `Order`'s `pay` arrow at y = 0 with
    /// its source state on it, `Shipment`'s `start` arrow at y = 200, and
    /// the controller and the source in between.
    fn canvas() -> Scene {
        let mut scene = Scene::empty(ViewKind::Structure, Rgba::hex(0xFFFFFF));
        scene.edges.push(arrow(pay(), 0.0));
        scene.edges.push(arrow(start(), 200.0));
        scene.nodes.push(node(key("state:Order:draft"), Rect::new(-40.0, -10.0, 44.0, 20.0)));
        scene.nodes.push(node(key("controller:Fulfil"), Rect::new(60.0, 90.0, 60.0, 24.0)));
        scene.nodes.push(node(key("external:Clock"), Rect::new(140.0, 90.0, 50.0, 24.0)));
        scene
    }

    fn at(x: f32, y: f32) -> ScreenPoint {
        ScreenPoint::new(x, y)
    }

    #[test]
    fn pick_distances_scale_with_zoom() {
        assert_eq!(scene_units(ARROW_PICK_PX, 1.0), 8.0);
        assert_eq!(scene_units(ARROW_PICK_PX, 2.0), 4.0);
        assert_eq!(scene_units(ARROW_PICK_PX, 0.5), 16.0);
        assert_eq!(scene_units(ARROW_PICK_PX, 0.0), 8.0, "no division by zero");
        assert_eq!(scene_units(ARROW_PICK_PX, f32::NAN), 8.0);
        const { assert!(ARROW_PICK_PX > HIT_TOLERANCE_PX, "arrows are wider to pick than other edges") };
    }

    #[test]
    fn arrows_pick_only_on_the_build_canvas_without_pills() {
        let arrows = ViewState { view: ViewKind::Structure, transition_pills: false, ..ViewState::default() };
        assert_eq!(Picking::for_view(AppMode::Build, &arrows), Picking::Arrows);
        assert_eq!(Picking::for_view(AppMode::View, &arrows), Picking::Plain);
        assert_eq!(Picking::for_view(AppMode::Play, &arrows), Picking::Plain);
        let pills = ViewState { view: ViewKind::Structure, ..ViewState::default() };
        assert_eq!(Picking::for_view(AppMode::Build, &pills), Picking::Plain);
    }

    #[test]
    fn the_arrow_pick_radius_is_eight_screen_pixels() {
        let scene = canvas();
        let handle = Some(HitTarget::ConnectHandle { element: pay() });
        // 6 scene units off the arrow: 6 px at zoom 1, 12 px at zoom 2.
        assert_eq!(hit(&scene, Point::new(100.0, 6.0), 1.0, Picking::Arrows), handle);
        assert_eq!(hit(&scene, Point::new(100.0, 6.0), 2.0, Picking::Arrows), None);
        assert_eq!(hit(&scene, Point::new(100.0, 3.0), 2.0, Picking::Arrows), handle);
        // Plain picking reaches only 5 px, and selects rather than connects.
        assert_eq!(hit(&scene, Point::new(100.0, 6.0), 1.0, Picking::Plain), None);
        assert_eq!(hit(&scene, Point::new(100.0, 4.0), 1.0, Picking::Plain), Some(HitTarget::Element(pay())));
    }

    #[test]
    fn nodes_win_over_arrows() {
        let scene = canvas();
        // On the source state, which the arrow leaves: the state.
        let state = Some(HitTarget::Element(key("state:Order:draft")));
        assert_eq!(hit(&scene, Point::new(0.0, 2.0), 1.0, Picking::Arrows), state);
    }

    #[test]
    fn pressing_an_arrow_starts_a_connect_drag_from_its_transition() {
        let scene = canvas();
        let target = hit(&scene, Point::new(100.0, -7.0), 1.0, Picking::Arrows);
        let from = connect_source(target.as_ref(), AppMode::Build, Mods::default()).expect("a connect source");
        assert_eq!(from, pay());
        assert_eq!(connect_source(target.as_ref(), AppMode::View, Mods::default()), None);
        let shift = Mods { shift: true, ..Mods::default() };
        assert_eq!(connect_source(target.as_ref(), AppMode::Build, shift), None, "modifiers never connect");

        // Drag to the controller: the transition emits a new event it handles.
        let mut gesture = Gesture::press_handle(at(100.0, 93.0), from, Mods::default(), 1);
        gesture.moved(at(130.0, 150.0), 1.0);
        let ReleaseOutcome::Connect { from, .. } = gesture.release() else { panic!("a connect drag") };
        let dropped = hit(&scene, Point::new(90.0, 100.0), 1.0, Picking::Arrows);
        let to = dropped.as_ref().and_then(drop_key).expect("a drop target").clone();
        assert!(connect::can_connect(&from, &to));
        let planned = connect::connect(&def(), &from, &to).expect("plans");
        let EditOp::Batch(ops) = planned.op else { panic!("a batch") };
        assert!(matches!(&ops[0], EditOp::UpdateTransition { transition, .. }
            if transition.emits.iter().any(|e| e.value == "PayDone2")));
        assert!(matches!(&ops[1], EditOp::AddHandler { controller, .. } if controller == "Fulfil"));
    }

    #[test]
    fn clicking_an_arrow_without_dragging_selects_its_transition() {
        let scene = canvas();
        let target = hit(&scene, Point::new(100.0, 5.0), 1.0, Picking::Arrows);
        let from = connect_source(target.as_ref(), AppMode::Build, Mods::default()).expect("a connect source");
        let mut gesture = Gesture::press_handle(at(100.0, 95.0), from, Mods::default(), 1);
        gesture.moved(at(101.0, 96.0), 1.0);
        let ReleaseOutcome::Click { pick, mods, clicks } = gesture.release() else { panic!("a click") };
        assert_eq!(pick, Pick::Element(pay()));
        assert_eq!(classify_click(pick, mods, clicks), ClickAction::Select(pay()));
    }

    #[test]
    fn dropping_on_an_arrow_targets_its_transition() {
        let scene = canvas();
        // Controller → arrow: a rule firing the transition's trigger.
        let dropped = hit(&scene, Point::new(150.0, 193.0), 1.0, Picking::Arrows);
        let to = dropped.as_ref().and_then(drop_key).expect("a drop target").clone();
        assert_eq!(to, start());
        let fire = connect::connect(&def(), &key("controller:Fulfil"), &to).expect("plans");
        assert!(matches!(fire.op, EditOp::AddRule { ref controller, ref rule, .. }
            if controller == "Fulfil" && rule.fire.value.to_string() == "Shipment.start"));
        // Source → arrow: the source exposes the trigger.
        let expose = connect::connect(&def(), &key("external:Clock"), &to).expect("plans");
        assert!(matches!(expose.op, EditOp::SetExternalTriggers { ref external, ref triggers }
            if external == "Clock" && triggers.len() == 2));
        // Beyond the arrow pick: nothing to drop on.
        assert_eq!(hit(&scene, Point::new(150.0, 180.0), 1.0, Picking::Arrows), None);
    }
}
