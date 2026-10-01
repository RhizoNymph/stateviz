//! Build mode shows only error badges: warnings and notes (unreachable
//! states, unhandled events, state-dependent fires) are what any
//! half-built system has. View mode keeps every badge.

use cascade_core::{CausalGraph, Severity, analyze, load_str};
use cascade_scene::{
    LayoutSidecar, MonoMeasure, Scene, SceneBuilder, SceneInput, SceneMode, Theme, ViewKind, ViewState,
};

/// `done` is unreachable (nothing triggers `finish`), `Started` is
/// unhandled, and controller `C` fires a trigger `Job` does not accept
/// (an error).
const HALF_BUILT: &str = "\
machines:
  Job:
    states: [idle, running, done]
    transitions:
      - { from: idle, to: running, on: start, emits: [Started] }
      - { from: running, to: done, on: finish, emits: [Finished] }
controllers:
  C:
    on:
      Finished: [{ fire: Job.nope }]
external:
  User: [Job.start]
";

fn scene(mode: SceneMode) -> (Scene, Vec<Severity>) {
    let model = load_str(HALF_BUILT).expect("loads");
    let graph = CausalGraph::build(&model);
    let findings = analyze(&model, &graph);
    let severities = findings.iter().map(|f| f.severity).collect();
    let view = ViewState { view: ViewKind::Structure, ..ViewState::default() };
    let theme = Theme::light();
    let sidecar = LayoutSidecar::default();
    let input = SceneInput {
        model: &model,
        graph: &graph,
        findings: &findings,
        view: &view,
        theme: &theme,
        measure: &MonoMeasure::default(),
        sidecar: &sidecar,
        traces: &[],
        mode,
        play: None,
        diff: None,
    };
    (SceneBuilder::new().build(&input).expect("builds"), severities)
}

fn badge_severities(scene: &Scene) -> Vec<Severity> {
    scene.nodes.iter().filter_map(|n| n.badge.as_ref().map(|b| b.severity)).collect()
}

#[test]
fn the_fixture_has_errors_and_warnings() {
    let (_, severities) = scene(SceneMode::View);
    assert!(severities.contains(&Severity::Error));
    assert!(severities.contains(&Severity::Warning));
}

#[test]
fn view_mode_badges_warnings() {
    let (scene, _) = scene(SceneMode::View);
    assert!(badge_severities(&scene).contains(&Severity::Warning), "{:?}", badge_severities(&scene));
}

#[test]
fn build_mode_badges_only_errors() {
    let (scene, _) = scene(SceneMode::Edit);
    let badges = badge_severities(&scene);
    assert!(!badges.is_empty(), "the invalid fire is still badged");
    assert!(badges.iter().all(|s| *s == Severity::Error), "{badges:?}");
}
