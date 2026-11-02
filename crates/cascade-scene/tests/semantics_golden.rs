//! The saga fixture (subscriptions, timers, choices, `sets`) in every view,
//! fingerprinted like `view_mode_golden.rs` (full `Debug` form and SVG
//! export), plus export checks for what the new drawing adds.
//!
//! Regenerate with `CASCADE_BLESS=1 cargo test -p cascade-scene --test
//! semantics_golden` when a change to these pictures is intended.

mod build_play;
mod semantics;

use std::fmt::Write as _;

use build_play::Bench;
use cascade_core::analyze;
use cascade_scene::metrics::{leftward_edges, measure};
use cascade_scene::{MonoMeasure, Scene, SceneBuilder, SceneInput, SceneMode, ViewKind, ViewState, to_png, to_svg};
use semantics::*;

const GOLDEN: &str = include_str!("golden/semantics.txt");

fn fnv(text: &str) -> u64 {
    text.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3))
}

fn cases() -> Vec<(&'static str, ViewState, SceneMode)> {
    let view = |view| ViewState { view, ..ViewState::default() };
    let arrows = ViewState { transition_pills: false, ..view(ViewKind::Structure) };
    vec![
        ("causal", view(ViewKind::Causal), SceneMode::View),
        ("causal-lanes", ViewState { group_by_machine: true, ..view(ViewKind::Causal) }, SceneMode::View),
        ("structure", view(ViewKind::Structure), SceneMode::View),
        ("structure-arrows", arrows.clone(), SceneMode::View),
        ("structure-edit", view(ViewKind::Structure), SceneMode::Edit),
        ("structure-edit-arrows", arrows, SceneMode::Edit),
        ("matrix", view(ViewKind::Matrix), SceneMode::View),
        ("trace", view(ViewKind::Trace), SceneMode::View),
    ]
}

fn build(bench: &Bench, state: &ViewState, mode: SceneMode) -> Scene {
    let findings = analyze(&bench.model, &bench.graph);
    let traces = [saga_trace(&bench.model)];
    let input = SceneInput {
        model: &bench.model,
        graph: &bench.graph,
        findings: &findings,
        view: state,
        theme: &bench.theme,
        measure: &MonoMeasure::default(),
        sidecar: &bench.sidecar,
        traces: &traces,
        diff: None,
        mode,
        play: None,
    };
    SceneBuilder::new().build(&input).unwrap_or_else(|err| panic!("build failed: {err}"))
}

#[test]
fn saga_scenes_are_unchanged() {
    let bench = Bench::new(SAGA);
    let mut report = String::new();
    for (name, state, mode) in cases() {
        let scene = build(&bench, &state, mode);
        let svg = to_svg(&scene).unwrap_or_else(|err| panic!("{name}: {err}"));
        let _ = writeln!(report, "saga-{name} {:016x} {:016x}", fnv(&format!("{scene:?}")), fnv(&svg));
    }
    if std::env::var_os("CASCADE_BLESS").is_some() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/golden/semantics.txt");
        std::fs::write(path, &report).unwrap_or_else(|err| panic!("writing {path}: {err}"));
        return;
    }
    let expected: Vec<&str> = GOLDEN.lines().collect();
    let actual: Vec<&str> = report.lines().collect();
    let changed: Vec<&str> = actual
        .iter()
        .filter(|line| !expected.contains(line))
        .map(|line| line.split(' ').next().unwrap_or_default())
        .collect();
    assert!(changed.is_empty() && expected.len() == actual.len(), "saga scenes changed: {changed:?}");
}

#[test]
fn saga_scenes_read_well() {
    let bench = Bench::new(SAGA);
    for (name, state, mode) in cases() {
        let scene = build(&bench, &state, mode);
        let m = measure(&scene);
        println!("saga {name:<22} {m}");
        assert_eq!(m.edges_through_nodes, 0, "{name}: {m}");
        assert_eq!(m.label_overlaps, 0, "{name}: {m}");
        if name == "causal-lanes" {
            assert_eq!(leftward_edges(&scene), 0);
        }
    }
}

#[test]
fn exports_carry_italics_and_the_clock() {
    let bench = Bench::new(SAGA);
    let structure = build(&bench, &ViewState { view: ViewKind::Structure, ..ViewState::default() }, SceneMode::View);
    let svg = to_svg(&structure).expect("svg");
    assert!(svg.contains(r#"font-style="italic""#), "choices are italic in SVG");
    assert!(svg.contains("[?card accepted]"));
    assert!(svg.contains("◷ 30m"), "the clock glyph is plain text");
    let png = to_png(&structure, 1.0).expect("png");
    assert!(png.starts_with(b"\x89PNG"));
    // Nothing else becomes italic.
    assert_eq!(svg.matches(r#"font-style="italic""#).count(), 2, "the two choice guards");
}
