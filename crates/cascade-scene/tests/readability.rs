//! Readability report: `cargo test -p cascade-scene --test readability -- --nocapture`
//! prints the metrics of every example in the structure view (view and build
//! modes) and the causal view. The numbers are the yardstick for layout and
//! routing work; see docs/features/readability.md.

use cascade_core::{CausalGraph, analyze, load_str};
use cascade_scene::metrics::{SceneMetrics, measure};
use cascade_scene::{LayoutSidecar, MonoMeasure, SceneBuilder, SceneInput, SceneMode, Theme, ViewKind, ViewState};

pub const EXAMPLES: [(&str, &str); 2] = [
    ("order-fulfillment", include_str!("../../../examples/order-fulfillment/cascade.yaml")),
    ("shop", include_str!("../../../examples/shop/cascade.yaml")),
];

pub fn metrics_of(yaml: &str, view: ViewKind, mode: SceneMode) -> SceneMetrics {
    let model = load_str(yaml).expect("example loads");
    let graph = CausalGraph::build(&model);
    let findings = analyze(&model, &graph);
    let state = ViewState { view, ..ViewState::default() };
    let theme = Theme::light();
    let sidecar = LayoutSidecar::default();
    let input = SceneInput {
        model: &model,
        graph: &graph,
        findings: &findings,
        view: &state,
        theme: &theme,
        measure: &MonoMeasure::default(),
        sidecar: &sidecar,
        traces: &[],
        mode,
        play: None,
        diff: None,
    };
    measure(&SceneBuilder::new().build(&input).expect("scene builds"))
}

#[test]
fn report() {
    for (name, yaml) in EXAMPLES {
        for (label, view, mode) in [
            ("structure/view", ViewKind::Structure, SceneMode::View),
            ("structure/build", ViewKind::Structure, SceneMode::Edit),
            ("causal", ViewKind::Causal, SceneMode::View),
        ] {
            println!("{name:<18} {label:<16} {}", metrics_of(yaml, view, mode));
        }
    }
}
