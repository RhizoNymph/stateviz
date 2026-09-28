//! Edit mode and play overlays stay interactive at the spec's scale (about
//! 20 machines, 200 states, 50 controllers): under a second per build even
//! in a debug build, and an overlay change costs no layout at all.

mod build_play;

use std::fmt::Write;
use std::time::{Duration, Instant};

use build_play::*;
use cascade_scene::{PlayOverlay, SceneBuilder, SceneMode, Shape, ViewKind, ViewState};

const MACHINES: usize = 20;
const STATES: usize = 10;
const CONTROLLERS: usize = 50;
const LIMIT: Duration = Duration::from_secs(1);

/// The same generated model as `performance.rs`: 20 machines of 10 states
/// (a nested pair in every fifth), 15 transitions each, 150 events, 50
/// controllers handling three events each, 5 external sources.
fn large_model() -> String {
    let mut y = String::from("machines:\n");
    for m in 0..MACHINES {
        let _ = writeln!(y, "  M{m}:\n    domain: d{}\n    states:", m % 4);
        for s in 0..STATES {
            if m % 5 == 0 && s == 5 {
                let _ = writeln!(y, "      - s5: {{ states: [inner0, inner1] }}");
            } else {
                let _ = writeln!(y, "      - s{s}");
            }
        }
        let _ = writeln!(y, "    transitions:");
        for s in 0..STATES {
            let emits = if s % 2 == 0 { format!(", emits: [E{m}_{s}]") } else { String::new() };
            let _ = writeln!(y, "      - {{ from: s{s}, to: s{}, on: t{s}{emits} }}", (s + 1) % STATES);
        }
        for s in 0..5 {
            let _ = writeln!(y, "      - {{ from: s{s}, to: s{}, on: jump{s}, guard: \"x > {s}\" }}", (s + 3) % STATES);
        }
        if m % 5 == 0 {
            let _ = writeln!(y, "      - {{ from: s5.inner0, to: s5.inner1, on: step }}");
        }
    }
    let _ = writeln!(y, "controllers:");
    for c in 0..CONTROLLERS {
        let _ = writeln!(y, "  C{c}:\n    on:");
        for k in 0..3 {
            let source = (c * 3 + k) % MACHINES;
            let event = (c + k * 2) % 5 * 2;
            let target = (source + 1 + c % 7) % MACHINES;
            let _ = writeln!(y, "      E{source}_{event}: [{{ fire: M{target}.t{} }}]", (c + k) % STATES);
        }
    }
    let _ = writeln!(y, "external:");
    for x in 0..5 {
        let triggers: Vec<String> = (0..4).map(|i| format!("M{}.t0", (x * 4 + i) % MACHINES)).collect();
        let _ = writeln!(y, "  X{x}: [{}]", triggers.join(", "));
    }
    y
}

#[test]
fn edit_mode_on_the_large_model_builds_in_time() {
    let bench = Bench::new(&large_model());
    // Best of three fresh builds: the cost, not scheduler noise when the
    // whole workspace's tests run in parallel.
    let mut best = Duration::MAX;
    let mut scene = None;
    for _ in 0..3 {
        let mut builder = SceneBuilder::new();
        let start = Instant::now();
        let built = bench.build_with(&mut builder, &structure(), SceneMode::Edit, None);
        best = best.min(start.elapsed());
        scene = Some(built);
    }
    assert!(best < LIMIT, "edit mode took {best:?}");
    let scene = scene.unwrap_or_else(|| panic!("built"));
    assert_eq!(scene.nodes.iter().filter(|n| n.shape == Shape::Hexagon).count(), CONTROLLERS);
    assert_eq!(scene.nodes.iter().filter(|n| n.shape == Shape::Tag).count(), bench.model.event_count());
    assert!(handles(&scene).len() > 500);
}

#[test]
fn play_overlays_on_the_large_model_are_cheap() {
    let bench = Bench::new(&large_model());
    let markers = (0..MACHINES).map(|m| marker(&format!("i{m}"), &format!("state:M{m}:s{}", m % STATES))).collect();
    let play = PlayOverlay {
        markers,
        active: (0..MACHINES).map(|m| k(&format!("event:E{m}_0"))).collect(),
        pending: (0..MACHINES).map(|m| k(&format!("event:E{m}_2"))).collect(),
    };
    for (view, mode) in [
        (ViewKind::Causal, SceneMode::View),
        (ViewKind::Structure, SceneMode::View),
        (ViewKind::Structure, SceneMode::Edit),
    ] {
        let state = ViewState { view, ..ViewState::default() };
        let mut builder = SceneBuilder::new();
        bench.build_with(&mut builder, &state, mode, None);
        let start = Instant::now();
        bench.build_with(&mut builder, &state, mode, Some(&play));
        let elapsed = start.elapsed();
        assert_eq!(builder.layouts_run(), 1, "{view} {mode:?}");
        assert!(elapsed < LIMIT, "{view} {mode:?} overlay rebuild took {elapsed:?}");
    }
}
