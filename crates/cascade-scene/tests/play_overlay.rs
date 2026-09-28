//! `PlayOverlay` over the structure and causal views: instance markers,
//! the last step (`active`) and the queue (`pending`). Overlays are
//! decoration: they never relayout and never change a hue.

mod build_play;
mod common;

use build_play::*;
use cascade_layout::{Point, Rect};
use cascade_scene::{
    Dash, EdgeKind, Layer, Overlay, PlayOverlay, Rgba, Scene, SceneBuilder, SceneMode, ViewKind, ViewState, to_svg,
};
use common::SPEC_EXAMPLE;

const OKABE_BLUE: Rgba = Rgba::hex(0x0072B2);

/// Rects of the chips labelled `text` (the chip is the overlay rect that
/// contains the text's origin).
fn chips(scene: &Scene, text: &str) -> Vec<Rect> {
    let origins: Vec<Point> = texts(scene).into_iter().filter(|(t, _)| t == text).map(|(_, p)| p).collect();
    origins
        .iter()
        .map(|p| {
            // The smallest overlay rect holding the text: glow rings are
            // larger and also contain it.
            scene
                .overlays
                .iter()
                .filter_map(|o| match o {
                    Overlay::Rect { rect, .. } if rect.outset(cascade_layout::Insets::uniform(0.01)).contains(*p) => {
                        Some(*rect)
                    }
                    _ => None,
                })
                .min_by(|a, b| a.size.width.total_cmp(&b.size.width))
                .unwrap_or_else(|| panic!("no chip behind {text}"))
        })
        .collect()
}

/// A chip sits on the node's top edge, within its width.
fn on_top_edge(chip: Rect, node: Rect) -> bool {
    chip.left() >= node.left() - 0.01
        && chip.left() < node.right()
        && chip.top() < node.top()
        && chip.bottom() > node.top()
        && chip.bottom() < node.bottom()
}

fn overlay(markers: Vec<cascade_scene::PlayMarker>, active: &[&str], pending: &[&str]) -> PlayOverlay {
    PlayOverlay {
        markers,
        active: active.iter().map(|s| k(s)).collect(),
        pending: pending.iter().map(|s| k(s)).collect(),
    }
}

fn scene(bench: &Bench, view: ViewKind, mode: SceneMode, play: &PlayOverlay) -> Scene {
    bench.scene(view, mode, Some(play))
}

#[test]
fn structure_markers_sit_on_the_current_state() {
    let bench = Bench::new(SPEC_EXAMPLE);
    let play = overlay(
        vec![
            marker("o1", "state:Order:pending"),
            marker("o2", "state:Order:pending"),
            marker("s1", "state:Shipment:idle"),
        ],
        &[],
        &[],
    );
    for mode in [SceneMode::View, SceneMode::Edit] {
        let scene = scene(&bench, ViewKind::Structure, mode, &play);
        let pending = node_rect(&scene, "state:Order:pending");
        let o1 = chips(&scene, "o1");
        let o2 = chips(&scene, "o2");
        assert_eq!((o1.len(), o2.len()), (1, 1), "{mode:?}");
        assert!(on_top_edge(o1[0], pending), "{mode:?}: {:?} on {pending:?}", o1[0]);
        assert!(on_top_edge(o2[0], pending));
        assert!(!overlaps(o1[0], o2[0]), "two instances on one state side by side");
        assert!(o1[0].left() < o2[0].left(), "in marker order");
        let s1 = chips(&scene, "s1");
        assert!(on_top_edge(s1[0], node_rect(&scene, "state:Shipment:idle")));
    }
}

#[test]
fn marker_chips_take_the_machine_hue() {
    let bench = Bench::new(SPEC_EXAMPLE);
    let play = overlay(vec![marker("o1", "state:Order:pending")], &[], &[]);
    let scene = scene(&bench, ViewKind::Structure, SceneMode::View, &play);
    let chip = chips(&scene, "o1")[0];
    let fill = scene.overlays.iter().find_map(|o| match o {
        Overlay::Rect { rect, fill, layer, .. } if *rect == chip => Some((*fill, *layer)),
        _ => None,
    });
    assert_eq!(fill, Some((Some(OKABE_BLUE), Layer::Over)), "an instance is machine-owned");
}

#[test]
fn markers_fall_back_to_collapsed_states_and_hidden_machine_stubs() {
    let bench = Bench::new(SHOP);
    let play =
        overlay(vec![marker("o1", "state:Order:placed.paid"), marker("n1", "state:Notification:sending")], &[], &[]);
    let state = ViewState {
        collapsed: [k("state:Order:placed")].into_iter().collect(),
        hidden_machines: ["Notification".to_owned()].into_iter().collect(),
        ..structure()
    };
    let scene = bench.build_with(&mut SceneBuilder::new(), &state, SceneMode::View, Some(&play));
    assert!(on_top_edge(chips(&scene, "o1")[0], node_rect(&scene, "state:Order:placed")));
    let stub = scene
        .nodes
        .iter()
        .find(
            |n| matches!(&n.target, cascade_scene::HitTarget::MachineStub { machine, .. } if machine == "Notification"),
        )
        .expect("stub");
    assert!(on_top_edge(chips(&scene, "n1")[0], stub.rect));
}

#[test]
fn causal_markers_sit_on_the_transitions_leaving_the_state() {
    let bench = Bench::new(SPEC_EXAMPLE);
    let play = overlay(vec![marker("o1", "state:Order:pending")], &[], &[]);
    let scene = scene(&bench, ViewKind::Causal, SceneMode::View, &play);
    let chips = chips(&scene, "o1");
    assert_eq!(chips.len(), 2, "pending → paid and pending → cancelled");
    for key in ["transition:Order:pending->paid@capture_ok", "transition:Order:pending->cancelled@timeout"] {
        let pill = node_rect(&scene, key);
        assert_eq!(chips.iter().filter(|c| on_top_edge(**c, pill)).count(), 1, "{key}");
    }
}

#[test]
fn causal_markers_in_a_dead_end_sit_hollow_on_the_way_in() {
    let bench = Bench::new(SPEC_EXAMPLE);
    let play = overlay(vec![marker("s1", "state:Shipment:shipped")], &[], &[]);
    let scene = scene(&bench, ViewKind::Causal, SceneMode::View, &play);
    let chips = chips(&scene, "s1");
    assert_eq!(chips.len(), 1);
    assert!(on_top_edge(chips[0], node_rect(&scene, "transition:Shipment:picking->shipped@handoff")));
    let fill = scene.overlays.iter().find_map(|o| match o {
        Overlay::Rect { rect, fill, .. } if *rect == chips[0] => Some(*fill),
        _ => None,
    });
    assert_eq!(fill, Some(Some(bench.theme.background)), "hollow: arrived, nothing leaves");
}

#[test]
fn nested_leaf_states_mark_transitions_leaving_their_ancestors() {
    let bench = Bench::new(SHOP);
    // `placed → cancelled` leaves the compound parent of `awaiting_payment`.
    let play = overlay(vec![marker("o1", "state:Order:placed.awaiting_payment")], &[], &[]);
    let scene = scene(&bench, ViewKind::Causal, SceneMode::View, &play);
    let chips = chips(&scene, "o1");
    assert_eq!(chips.len(), 2);
    for key in [
        "transition:Order:placed.awaiting_payment->placed.paid@payment_captured",
        "transition:Order:placed->cancelled@cancel",
    ] {
        assert!(chips.iter().any(|c| on_top_edge(*c, node_rect(&scene, key))), "{key}");
    }
}

#[test]
fn active_items_get_weight_and_a_glow_but_keep_their_hue() {
    let bench = Bench::new(SPEC_EXAMPLE);
    let play = overlay(
        Vec::new(),
        &["transition:Order:pending->paid@capture_ok", "event:OrderPaid", "handler:Fulfillment/OrderPaid"],
        &[],
    );
    let plain = bench.scene(ViewKind::Causal, SceneMode::View, None);
    let lit = scene(&bench, ViewKind::Causal, SceneMode::View, &play);
    let theme = &bench.theme;
    for (a, b) in plain.nodes.iter().zip(&lit.nodes) {
        assert_eq!(a.fill, b.fill, "no fill change");
        assert_eq!(a.stroke.color, b.stroke.color, "no hue change");
        assert_eq!(a.rect, b.rect);
    }
    for key in ["transition:Order:pending->paid@capture_ok", "event:OrderPaid", "handler:Fulfillment/OrderPaid"] {
        let n = lit.nodes.iter().find(|n| n.target == target(key)).expect("node");
        assert!(n.stroke.width >= theme.selected_stroke_width, "{key}: strong outline");
        let glow = lit.overlays.iter().any(|o| match o {
            Overlay::Rect { rect, stroke: Some(s), layer: Layer::Under, fill: None, .. } => {
                inside(*rect, n.rect)
                    && *rect != n.rect
                    && (s.color.r, s.color.g, s.color.b) == (n.stroke.color.r, n.stroke.color.g, n.stroke.color.b)
                    && s.color.a < 255
            }
            _ => false,
        });
        assert!(glow, "{key}: a glow ring in its own color");
    }
    let quiet = lit.nodes.iter().find(|n| n.target == target("transition:Order:draft->pending@submit")).expect("pill");
    assert_eq!(quiet.stroke.width, theme.stroke_width, "inactive items unchanged");
    // The emit between the active pill and the active event is active too;
    // the fire out of the handler is not (its rule did not run).
    let emit = lit
        .edges
        .iter()
        .find(|e| {
            e.kind == EdgeKind::Emit
                && touches(node_rect(&lit, "transition:Order:pending->paid@capture_ok"), e.points[0])
        })
        .expect("emit");
    assert!(emit.stroke.width > theme.stroke_width);
    assert_eq!(emit.stroke.color, theme.neutral);
    let fire = lit.edges.iter().find(|e| e.kind == EdgeKind::Fire).expect("fire");
    assert_eq!(fire.stroke.width, theme.stroke_width);
}

#[test]
fn active_links_in_the_structure_view() {
    let bench = Bench::new(SPEC_EXAMPLE);
    let play = overlay(Vec::new(), &["transition:Shipment:idle->picking@start", "rule:Fulfillment/OrderPaid#0"], &[]);
    let lit = scene(&bench, ViewKind::Structure, SceneMode::View, &play);
    let theme = &bench.theme;
    let link = lit.edges.iter().find(|e| e.kind == EdgeKind::Fire).expect("link");
    assert!(link.stroke.width >= theme.selected_stroke_width, "the rule that fired");
    let arrows: Vec<_> =
        lit.edges.iter().filter(|e| e.target == target("transition:Shipment:idle->picking@start")).collect();
    assert_eq!(arrows.len(), 2);
    assert!(arrows.iter().all(|e| e.stroke.width >= theme.selected_stroke_width), "the transition's arrows");
}

#[test]
fn pending_items_are_dotted_and_numbered_by_queue_position() {
    let bench = Bench::new(SPEC_EXAMPLE);
    let play = overlay(Vec::new(), &[], &["event:OrderPaid", "rule:Fulfillment/OrderPaid#0"]);
    let scene = scene(&bench, ViewKind::Causal, SceneMode::View, &play);
    let tag = scene.nodes.iter().find(|n| n.target == target("event:OrderPaid")).expect("tag");
    assert_eq!(tag.stroke.dash, Dash::Dotted);
    let head = chips(&scene, "1");
    assert_eq!(head.len(), 1, "the head is marked 1");
    assert!(head[0].contains(Point::new(tag.rect.left(), tag.rect.top())) || overlaps(head[0], tag.rect));
    let fire = scene.edges.iter().find(|e| e.kind == EdgeKind::Fire).expect("fire");
    assert_eq!(fire.stroke.dash, Dash::Dotted);
    let second = chips(&scene, "2");
    assert_eq!(second.len(), 1);
    let end = *fire.points.last().expect("end");
    assert!(second[0].center().distance(end) < 40.0, "numbered near where the fire arrives");
    // The head's chip is filled, the rest hollow.
    let fill_of = |r: Rect| {
        scene.overlays.iter().find_map(|o| match o {
            Overlay::Rect { rect, fill, .. } if *rect == r => Some(*fill),
            _ => None,
        })
    };
    assert_eq!(fill_of(head[0]), Some(Some(bench.theme.text)));
    assert_eq!(fill_of(second[0]), Some(Some(bench.theme.background)));
}

#[test]
fn repeated_queue_entries_list_every_position() {
    let bench = Bench::new(SPEC_EXAMPLE);
    let play = overlay(Vec::new(), &[], &["event:OrderPaid", "event:Shipped", "event:OrderPaid"]);
    let scene = scene(&bench, ViewKind::Causal, SceneMode::View, &play);
    assert_eq!(chips(&scene, "1, 3").len(), 1);
    assert_eq!(chips(&scene, "2").len(), 1);
}

#[test]
fn pending_events_in_the_structure_view_mark_their_links() {
    let bench = Bench::new(SPEC_EXAMPLE);
    let play = overlay(Vec::new(), &[], &["event:OrderPaid"]);
    let view = scene(&bench, ViewKind::Structure, SceneMode::View, &play);
    let link = view.edges.iter().find(|e| e.kind == EdgeKind::Fire).expect("link");
    assert_eq!(link.stroke.dash, Dash::Dotted, "no event node here: the link through it");
    assert_eq!(chips(&view, "1").len(), 1);
    // In edit mode the event has its own tag.
    let edit = scene(&bench, ViewKind::Structure, SceneMode::Edit, &play);
    let tag = edit.nodes.iter().find(|n| n.target == target("event:OrderPaid")).expect("tag");
    assert_eq!(tag.stroke.dash, Dash::Dotted);
    let fire = edit.edges.iter().find(|e| e.kind == EdgeKind::Fire).expect("fire");
    assert!(matches!(fire.stroke.dash, Dash::Dashed { .. }), "only the event is pending");
}

#[test]
fn overlays_never_relayout() {
    let bench = Bench::new(SHOP);
    let play = overlay(
        vec![marker("o1", "state:Order:cart"), marker("p1", "state:Payment:authorizing")],
        &["transition:Order:cart->placed@checkout", "event:OrderPlaced"],
        &["event:OrderPlaced", "rule:Checkout/OrderPlaced#0"],
    );
    for (view, mode) in [
        (ViewKind::Causal, SceneMode::View),
        (ViewKind::Structure, SceneMode::View),
        (ViewKind::Structure, SceneMode::Edit),
    ] {
        let state = ViewState { view, ..ViewState::default() };
        let mut builder = SceneBuilder::new();
        let before = bench.build_with(&mut builder, &state, mode, None);
        let runs = builder.layouts_run();
        let after = bench.build_with(&mut builder, &state, mode, Some(&play));
        assert_eq!(builder.layouts_run(), runs, "{view} {mode:?}: no relayout");
        assert_eq!(rects(&before), rects(&after));
        assert_eq!(before.lanes, after.lanes);
        let points = |s: &Scene| s.edges.iter().map(|e| e.points.clone()).collect::<Vec<_>>();
        assert_eq!(points(&before), points(&after));
        assert!(after.overlays.len() > before.overlays.len());
        // And back: the plain scene again.
        assert_eq!(bench.build_with(&mut builder, &state, mode, None), before);
    }
}

#[test]
fn unknown_keys_in_the_overlay_are_ignored() {
    let bench = Bench::new(SPEC_EXAMPLE);
    let play = overlay(
        vec![marker("x1", "state:Nope:gone"), marker("o1", "state:Order:nowhere")],
        &["event:Nope"],
        &["rule:Nobody/Nothing#0"],
    );
    for view in [ViewKind::Causal, ViewKind::Structure] {
        assert_eq!(scene(&bench, view, SceneMode::View, &play), bench.scene(view, SceneMode::View, None), "{view}");
    }
}

#[test]
fn trace_and_matrix_ignore_the_overlay() {
    let bench = Bench::new(SPEC_EXAMPLE);
    let play = overlay(vec![marker("o1", "state:Order:pending")], &["event:OrderPaid"], &["event:OrderPaid"]);
    for view in [ViewKind::Trace, ViewKind::Matrix] {
        assert_eq!(scene(&bench, view, SceneMode::View, &play), bench.scene(view, SceneMode::View, None), "{view}");
    }
}

#[test]
fn overlays_export_to_svg() {
    let bench = Bench::new(SPEC_EXAMPLE);
    let play = overlay(
        vec![marker("o1", "state:Order:pending")],
        &["transition:Order:draft->pending@submit"],
        &["event:OrderPaid"],
    );
    for view in [ViewKind::Causal, ViewKind::Structure] {
        let scene = scene(&bench, view, SceneMode::View, &play);
        let svg = to_svg(&scene).unwrap_or_else(|err| panic!("{err}"));
        assert!(svg.contains(">o1<"), "{view}: marker label");
        assert!(svg.contains(">1<"), "{view}: queue number");
        let doc = roxmltree::Document::parse(&svg).unwrap_or_else(|err| panic!("{err}"));
        let overlays = doc.descendants().filter(|n| n.attribute("class") == Some("overlay")).count();
        assert_eq!(overlays, scene.overlays.len(), "{view}: every chip, number and glow");
        // Bounds cover the chips.
        for o in &scene.overlays {
            if let Overlay::Rect { rect, .. } = o {
                assert!(inside(scene.bounds, *rect), "{view}: {rect:?} in {:?}", scene.bounds);
            }
        }
    }
}
