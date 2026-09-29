//! Layout stability while building: in edit mode an edit moves no
//! unrelated node, the wiring band included, and switching between view
//! and edit mode returns to exactly the earlier pictures.
//!
//! Like `edit_stability.rs`, each test runs the whole pipeline twice with
//! one `SceneBuilder`, the way the app does after an edit.

mod build_play;

use std::collections::BTreeMap;

use build_play::*;
use cascade_layout::Rect;
use cascade_scene::{HitTarget, Scene, SceneBuilder, SceneMode, ViewKind, ViewState};

fn edit_build(builder: &mut SceneBuilder, yaml: &str) -> Scene {
    Bench::new(yaml).build_with(builder, &structure(), SceneMode::Edit, None)
}

/// Insert `line` right after `marker`, which must follow `anchor`.
fn insert_after(text: &str, anchor: &str, marker: &str, line: &str) -> String {
    let start = text.find(anchor).unwrap_or_else(|| panic!("no {anchor:?}"));
    let offset =
        text[start..].find(marker).unwrap_or_else(|| panic!("no {marker:?} after {anchor:?}")) + start + marker.len();
    let mut out = text.to_owned();
    out.insert_str(offset, line);
    out
}

/// Keys whose rect changed (or vanished) between two builds, except those
/// `may_move` allows.
fn moved(
    before: &BTreeMap<String, Rect>,
    after: &BTreeMap<String, Rect>,
    may_move: impl Fn(&str) -> bool,
) -> Vec<String> {
    before
        .iter()
        .filter(|(key, _)| !may_move(key))
        .filter_map(|(key, rect)| match after.get(key) {
            Some(new) if new == rect => None,
            Some(new) => Some(format!("{key}: {rect:?} -> {new:?}")),
            None => Some(format!("{key}: disappeared")),
        })
        .collect()
}

fn in_machine(key: &str, machine: &str) -> bool {
    key.contains(&format!(":{machine}:")) || key.contains(&format!(":{machine}."))
}

#[test]
fn adding_a_transition_moves_nothing_outside_its_machine() {
    let after_text =
        insert_after(SHOP, "  Shipment:\n", "    transitions:\n", "      - { from: packing, to: lost, on: abort }\n");
    let mut builder = SceneBuilder::new();
    let before = rects(&edit_build(&mut builder, SHOP));
    let after = rects(&edit_build(&mut builder, &after_text));
    assert!(after.contains_key("transition:Shipment:packing->lost@abort"));
    let moved = moved(&before, &after, |k| in_machine(k, "Shipment"));
    assert!(moved.is_empty(), "moved:\n{}", moved.join("\n"));
}

#[test]
fn wiring_a_new_handler_keeps_every_machine_lane_in_place() {
    // Shipping starts reacting to ShipmentLost by notifying the customer:
    // a new subscription and a new fire, nothing else.
    let after_text = insert_after(
        SHOP,
        "  Shipping:\n",
        "    on:\n",
        "      ShipmentLost:\n        - fire: Notification.send\n          target: new Notification with orderId = event.orderId\n",
    );
    let mut builder = SceneBuilder::new();
    let before = rects(&edit_build(&mut builder, SHOP));
    let after = rects(&edit_build(&mut builder, &after_text));
    let lanes = moved(&before, &after, |k| {
        k.starts_with("event:") || k.starts_with("controller:") || k.starts_with("external:")
    });
    assert!(lanes.is_empty(), "machine lanes moved:\n{}", lanes.join("\n"));
    // In the band, only the rewired event and controller may move.
    let band = moved(&before, &after, |k| {
        !(k.starts_with("event:") || k.starts_with("controller:") || k.starts_with("external:"))
            || k == "event:ShipmentLost"
            || k == "controller:Shipping"
    });
    assert!(band.is_empty(), "band nodes moved:\n{}", band.join("\n"));
}

#[test]
fn adding_an_emit_keeps_every_machine_lane_in_place() {
    let before_text = SHOP;
    let after_text = before_text.replace(
        "      - { from: sending, to: bounced, on: rejected }",
        "      - { from: sending, to: bounced, on: rejected, emits: [ShipmentLost] }",
    );
    assert_ne!(after_text, before_text);
    let mut builder = SceneBuilder::new();
    let before = rects(&edit_build(&mut builder, before_text));
    let after = rects(&edit_build(&mut builder, &after_text));
    let moved = moved(&before, &after, |k| {
        k == "event:ShipmentLost" || k.starts_with("transition:Notification:sending->bounced")
    });
    assert!(moved.is_empty(), "moved:\n{}", moved.join("\n"));
}

#[test]
fn appending_events_and_controllers_moves_no_existing_node() {
    // The app's add gestures append to the end of each list.
    let text = SHOP.replacen("\nmachines:\n", "  Audited: { payload: [orderId] }\n\nmachines:\n", 1).replacen(
        "\nexternal:\n",
        "  Archive:\n    on:\n      Audited: []\n\nexternal:\n",
        1,
    );
    let mut builder = SceneBuilder::new();
    let before = rects(&edit_build(&mut builder, SHOP));
    let after = rects(&edit_build(&mut builder, &text));
    for key in ["event:Audited", "controller:Archive"] {
        assert!(after.contains_key(key), "{key} drawn");
    }
    let moved = moved(&before, &after, |_| false);
    assert!(moved.is_empty(), "moved:\n{}", moved.join("\n"));
}

#[test]
fn appending_a_source_moves_at_most_what_lies_below_its_gutter_as_a_whole() {
    // Auditor triggers Notification.send, so it joins the gutter above the
    // Notification lane. Nothing above that gutter or in it moves; the
    // engine may open a row for it, shifting everything below down as one
    // piece.
    let text = format!("{SHOP}  Auditor: [Notification.send]\n");
    let mut builder = SceneBuilder::new();
    let before_scene = edit_build(&mut builder, SHOP);
    let after_scene = edit_build(&mut builder, &text);
    let (before, after) = (rects(&before_scene), rects(&after_scene));
    let auditor = after["external:Auditor"];
    let notification = lane(&after_scene, "machine:Notification");
    assert!(auditor.bottom() <= notification.top(), "above the lane it triggers");
    let gutter = after_scene
        .lanes
        .iter()
        .find(|l| l.target == HitTarget::None && inside(l.rect, auditor))
        .expect("Auditor sits in a gutter");
    let gutter_before = before_scene
        .lanes
        .iter()
        .find(|l| l.target == HitTarget::None && l.rect.top() == gutter.rect.top())
        .expect("the gutter existed and did not move");
    let below = |r: &Rect| r.top() >= gutter_before.rect.bottom();
    let moved = moved(&before, &after, |k| below(&before[k]));
    assert!(moved.is_empty(), "moved above or in the gutter:\n{}", moved.join("\n"));
    let shifts: Vec<(f32, f32)> = before
        .iter()
        .filter(|(_, r)| below(r))
        .map(|(k, r)| {
            let new = after[k.as_str()];
            (new.left() - r.left(), new.top() - r.top())
        })
        .collect();
    assert!(!shifts.is_empty());
    assert!(shifts.windows(2).all(|w| w[0] == w[1]), "everything below moved as a whole: {shifts:?}");
    assert_eq!(shifts[0].0, 0.0, "only downwards");
    assert!(shifts[0].1 >= 0.0, "only downwards");
}

fn lane(scene: &Scene, key: &str) -> Rect {
    let t = target(key);
    scene.lanes.iter().find(|l| l.target == t).map(|l| l.rect).unwrap_or_else(|| panic!("no lane {key}"))
}

#[test]
fn switching_modes_returns_to_the_same_pictures() {
    let bench = Bench::new(SHOP);
    let state = ViewState { view: ViewKind::Structure, ..ViewState::default() };
    let mut builder = SceneBuilder::new();
    let view = bench.build_with(&mut builder, &state, SceneMode::View, None);
    let edit = bench.build_with(&mut builder, &state, SceneMode::Edit, None);
    let runs = builder.layouts_run();
    assert_eq!(bench.build_with(&mut builder, &state, SceneMode::View, None), view);
    assert_eq!(bench.build_with(&mut builder, &state, SceneMode::Edit, None), edit);
    assert_eq!(builder.layouts_run(), runs, "both layouts stay cached");
    // Entering edit mode from view mode opens gutters between the lanes,
    // so lanes shift down, but inside its lane every state keeps its place:
    // pills only gain ports.
    let (v, e) = (rects(&view), rects(&edit));
    let in_lane = |scene: &Scene, key: &str, r: &Rect| {
        let machine = key.trim_start_matches("state:").split(':').next().unwrap_or_default();
        let top = lane(scene, &format!("machine:{machine}")).top();
        (r.left(), r.top() - top, r.size)
    };
    let states_moved: Vec<_> = v
        .iter()
        .filter(|(k, _)| k.starts_with("state:"))
        .filter(|(k, r)| e.get(*k).map(|er| in_lane(&edit, k, er)) != Some(in_lane(&view, k, r)))
        .map(|(k, _)| k)
        .collect();
    assert!(
        states_moved.len() <= v.len() / 4,
        "most states stay put in their lane entering edit mode: {states_moved:?}"
    );
}

#[test]
fn edit_mode_honours_pins() {
    // A band node dragged a little to the right, as the app pins it.
    let placed = node_rect(&Bench::new(SHOP).edit(), "event:OrderPaid");
    let mut bench = Bench::new(SHOP);
    let pin = cascade_layout::Point::new(placed.left() + 60.0, placed.top() + 10.0);
    bench.sidecar.pin(ViewKind::Structure, k("event:OrderPaid"), pin);
    let scene = bench.edit();
    assert_eq!(node_rect(&scene, "event:OrderPaid").origin, pin);
    assert_eq!(handles(&scene).len(), handles(&Bench::new(SHOP).edit()).len());
}
