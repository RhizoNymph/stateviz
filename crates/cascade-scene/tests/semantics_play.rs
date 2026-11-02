//! Armed timers in the play overlay: a small clock chip with the time left,
//! next to the instance's marker.

mod build_play;
mod semantics;

use build_play::{Bench, k, marker};
use cascade_core::expr::Duration;
use cascade_layout::Rect;
use cascade_scene::{Overlay, PlayOverlay, PlayTimer, Scene, SceneBuilder, SceneMode, ViewKind, ViewState};
use semantics::*;

fn play(timers: Vec<PlayTimer>) -> PlayOverlay {
    PlayOverlay {
        markers: vec![marker("o1", "state:Order:placed"), marker("p1", "state:Payment:charging")],
        timers,
        ..PlayOverlay::default()
    }
}

fn timer(instance: &str, trigger: &str, secs: u64) -> PlayTimer {
    PlayTimer { instance: instance.to_owned(), trigger: k(trigger), remaining: Duration::from_secs(secs) }
}

fn scene(view: ViewKind, overlay: &PlayOverlay) -> Scene {
    let state = ViewState { view, ..ViewState::default() };
    Bench::new(SAGA).build_with(&mut SceneBuilder::new(), &state, SceneMode::View, Some(overlay))
}

/// The chip rect behind each overlay text equal to `text`.
fn chips(scene: &Scene, text: &str) -> Vec<Rect> {
    overlay_texts(scene)
        .into_iter()
        .filter(|l| l.text == text)
        .map(|l| {
            scene
                .overlays
                .iter()
                .filter_map(|o| match o {
                    Overlay::Rect { rect, .. }
                        if rect.outset(cascade_layout::Insets::uniform(0.01)).contains(l.origin) =>
                    {
                        Some(*rect)
                    }
                    _ => None,
                })
                .min_by(|a, b| a.size.width.total_cmp(&b.size.width))
                .expect("a chip")
        })
        .collect()
}

#[test]
fn structure_view_shows_time_left_next_to_the_marker() {
    let overlay = play(vec![timer("p1", "trigger:Payment.after-10s", 4), timer("o1", "trigger:Order.after-30m", 1800)]);
    let scene = scene(ViewKind::Structure, &overlay);
    let charging = node(&scene, "state:Payment:charging").rect;
    let marker = chips(&scene, "p1");
    let clock = chips(&scene, "◷ 4s");
    assert_eq!((marker.len(), clock.len()), (1, 1));
    assert!(clock[0].left() >= marker[0].right(), "right of the marker");
    assert!((clock[0].center().y - marker[0].center().y).abs() < 0.01, "on the same edge");
    assert!(clock[0].top() < charging.top() && clock[0].bottom() > charging.top());
    assert_eq!(chips(&scene, "◷ 30m").len(), 1, "o1's timer on placed");
}

#[test]
fn causal_view_puts_the_clock_on_the_timer_pill_only() {
    let overlay = play(vec![timer("p1", "trigger:Payment.after-10s", 4)]);
    let scene = scene(ViewKind::Causal, &overlay);
    let clock = chips(&scene, "◷ 4s");
    assert_eq!(clock.len(), 1, "only on the pill the timer fires");
    let retry = node(&scene, RETRY).rect;
    assert!(clock[0].left() >= retry.left() && clock[0].right() <= retry.right() + 40.0);
    assert!(clock[0].top() < retry.top() && clock[0].bottom() > retry.top());
}

#[test]
fn due_timers_and_unknown_instances() {
    let overlay = play(vec![timer("p1", "trigger:Payment.after-10s", 0), timer("ghost", "trigger:Order.after-30m", 5)]);
    let scene = scene(ViewKind::Structure, &overlay);
    assert_eq!(chips(&scene, "◷ due").len(), 1);
    assert!(chips(&scene, "◷ 5s").is_empty(), "no marker, no clock");
}

#[test]
fn timers_never_relayout() {
    let bench = Bench::new(SAGA);
    let state = ViewState { view: ViewKind::Structure, ..ViewState::default() };
    let mut builder = SceneBuilder::new();
    let without = bench.build_with(&mut builder, &state, SceneMode::View, Some(&play(Vec::new())));
    let with = bench.build_with(
        &mut builder,
        &state,
        SceneMode::View,
        Some(&play(vec![timer("p1", "trigger:Payment.after-10s", 4)])),
    );
    assert_eq!(builder.layouts_run(), 1);
    assert_eq!(without.nodes, with.nodes);
    assert_eq!(without.edges, with.edges);
}
