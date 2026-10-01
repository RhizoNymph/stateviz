//! Two selections mean a path query in View mode, but only "these two are
//! selected" in Build mode (where shift-click + Connect uses them).

use cascade_core::{CausalGraph, load_str};
use cascade_scene::emphasis::Interaction;
use cascade_scene::{SceneMode, ViewState};

const ONE_MACHINE: &str = "\
machines:
  Job:
    states: [idle, running, done]
    transitions:
      - { from: idle, to: running, on: start }
      - { from: running, to: done, on: finish }
";

const TWO_MACHINES: &str = "\
machines:
  A:
    states: [a0, a1]
    transitions:
      - { from: a0, to: a1, on: go }
  B:
    states: [b0, b1]
    transitions:
      - { from: b0, to: b1, on: start }
";

fn select(keys: &[&str]) -> ViewState {
    ViewState { selection: keys.iter().map(|k| k.parse().expect("key")).collect(), ..ViewState::default() }
}

#[test]
fn build_mode_two_selections_are_not_a_path_query() {
    let model = load_str(ONE_MACHINE).expect("loads");
    let graph = CausalGraph::build(&model);
    let view = select(&["state:Job:idle", "state:Job:done"]);
    let i = Interaction::for_mode(&model, &graph, &view, SceneMode::Edit);
    assert_eq!(i.selected().len(), 2, "both stay selected");
    assert!(i.focus().is_none(), "no path focus, so nothing is dimmed");
    assert!(i.notes().is_empty(), "no 'no causal path' note while building");
}

#[test]
fn view_mode_explains_that_transitions_in_one_machine_are_not_causal_links() {
    let model = load_str(ONE_MACHINE).expect("loads");
    let graph = CausalGraph::build(&model);
    let view = select(&["state:Job:idle", "state:Job:done"]);
    let i = Interaction::for_mode(&model, &graph, &view, SceneMode::View);
    assert!(i.focus().is_some(), "View mode still runs the path query");
    let note = i.notes().first().expect("a note");
    assert!(note.starts_with("No causal path between Job.idle and Job.done"), "{note}");
    assert!(note.contains("within one machine"), "{note}");
    assert!(note.contains("event"), "{note}");
}

#[test]
fn view_mode_across_machines_says_how_causality_flows() {
    let model = load_str(TWO_MACHINES).expect("loads");
    let graph = CausalGraph::build(&model);
    let view = select(&["transition:A:a0->a1@go", "transition:B:b0->b1@start"]);
    let i = Interaction::for_mode(&model, &graph, &view, SceneMode::View);
    let note = i.notes().first().expect("a note");
    assert!(note.starts_with("No causal path between"), "{note}");
    assert!(note.contains("controller"), "{note}");
    assert!(!note.contains("within one machine"), "{note}");
}

#[test]
fn new_keeps_view_mode_behaviour() {
    let model = load_str(ONE_MACHINE).expect("loads");
    let graph = CausalGraph::build(&model);
    let view = select(&["state:Job:idle", "state:Job:done"]);
    assert_eq!(Interaction::new(&model, &graph, &view), Interaction::for_mode(&model, &graph, &view, SceneMode::View));
}
