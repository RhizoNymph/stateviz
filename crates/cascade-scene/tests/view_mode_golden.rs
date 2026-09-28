//! `SceneMode::View` stays exactly as it was before build and play drawing
//! existed: every scene below is fingerprinted (its full `Debug` form and its
//! SVG export) and compared with fingerprints recorded before the build and
//! play work started.
//!
//! Regenerate with `CASCADE_BLESS=1 cargo test -p cascade-scene --test
//! view_mode_golden` only when a view-mode change is intended.

mod common;

use std::fmt::Write as _;

use cascade_core::{Direction, analyze};
use cascade_scene::{
    ConeFocus, OutsideFocus, PlayOverlay, Scene, SceneBuilder, SceneInput, SceneMode, ViewKind, ViewState, to_svg,
};
use common::*;

const SHOP: &str = include_str!("../../../examples/shop/cascade.yaml");
const GOLDEN: &str = include_str!("golden/view_mode.txt");

/// FNV-1a, 64 bit: a stable fingerprint independent of the std hasher.
fn fnv(text: &str) -> u64 {
    text.bytes().fold(0xcbf2_9ce4_8422_2325, |h, b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3))
}

fn build(fx: &Fixture, view: &ViewState, play: Option<&PlayOverlay>) -> Scene {
    let findings = analyze(&fx.model, &fx.graph);
    let input = SceneInput {
        model: &fx.model,
        graph: &fx.graph,
        findings: &findings,
        view,
        theme: &fx.theme,
        measure: &cascade_scene::MonoMeasure::default(),
        sidecar: &fx.sidecar,
        traces: &fx.traces,
        diff: None,
        mode: SceneMode::View,
        play,
    };
    SceneBuilder::new().build(&input).unwrap_or_else(|err| panic!("build failed: {err}"))
}

fn cases() -> Vec<(String, &'static str, ViewState)> {
    let mut out = Vec::new();
    for (name, yaml) in [("spec", SPEC_EXAMPLE), ("shop", SHOP), ("chain", CHAIN)] {
        for kind in [ViewKind::Causal, ViewKind::Structure, ViewKind::Matrix] {
            out.push((format!("{name}-{kind}"), yaml, ViewState { view: kind, ..ViewState::default() }));
        }
    }
    let cone = Some(ConeFocus { direction: Direction::Forward, depth: None });
    for kind in [ViewKind::Causal, ViewKind::Structure] {
        out.push((
            format!("shop-{kind}-cone-dim"),
            SHOP,
            ViewState { view: kind, cone, ..view(&["transition:Order:cart->placed@checkout"]) },
        ));
        out.push((
            format!("shop-{kind}-cone-hide"),
            SHOP,
            ViewState {
                view: kind,
                cone,
                outside: OutsideFocus::Hide,
                ..view(&["transition:Order:cart->placed@checkout"])
            },
        ));
        out.push((
            format!("shop-{kind}-filters"),
            SHOP,
            ViewState {
                view: kind,
                hidden_machines: ["Notification".to_owned()].into_iter().collect(),
                collapsed: [k("state:Order:placed"), k("machine:Inventory")].into_iter().collect(),
                search: Some("Payment".to_owned()),
                ..ViewState::default()
            },
        ));
    }
    out
}

#[test]
fn view_mode_scenes_are_unchanged() {
    let mut report = String::new();
    for (name, yaml, state) in cases() {
        let fx = Fixture::new(yaml);
        let scene = build(&fx, &state, None);
        // An empty play overlay draws nothing either.
        assert_eq!(build(&fx, &state, Some(&PlayOverlay::default())), scene, "{name}: empty overlay");
        let svg = to_svg(&scene).unwrap_or_else(|err| panic!("{name}: {err}"));
        let _ = writeln!(report, "{name} {:016x} {:016x}", fnv(&format!("{scene:?}")), fnv(&svg));
    }
    if std::env::var_os("CASCADE_BLESS").is_some() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/golden/view_mode.txt");
        std::fs::write(path, &report).unwrap_or_else(|err| panic!("writing {path}: {err}"));
        return;
    }
    let expected: Vec<&str> = GOLDEN.lines().collect();
    let actual: Vec<&str> = report.lines().collect();
    let changed: Vec<String> = actual
        .iter()
        .filter(|line| !expected.contains(line))
        .map(|line| line.split(' ').next().unwrap_or_default().to_owned())
        .collect();
    assert!(changed.is_empty() && expected.len() == actual.len(), "view-mode scenes changed: {changed:?}");
}
