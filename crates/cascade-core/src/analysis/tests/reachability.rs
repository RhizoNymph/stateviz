use super::*;

fn unreachable(text: &str) -> Vec<String> {
    let (model, findings) = run(text);
    of(&findings, Check::UnreachableState).iter().map(|f| key(&model, f.detail.primary())).collect()
}

#[test]
fn spec_example_never_ships() {
    let (model, findings) = run(examples::SPEC_EXAMPLE);
    let f = single(&findings, Check::UnreachableState);
    assert_eq!(key(&model, f.detail.primary()), "state:Shipment:shipped");
    assert_eq!(f.severity, Severity::Warning);
    assert_eq!(
        f.message,
        "Shipment.shipped is never entered: no path from Shipment's initial state reaches it through external triggers or controller fires"
    );
}

#[test]
fn cross_machine_fires_make_states_reachable() {
    let text = r#"
machines:
  A:
    states: [a0, a1]
    transitions:
      - { from: a0, to: a1, on: go, emits: [Went] }
  B:
    states: [b0, b1, b2]
    transitions:
      - { from: b0, to: b1, on: follow, emits: [Followed] }
      - { from: b1, to: b2, on: finish }
controllers:
  C:
    on:
      Went: [{ fire: B.follow }]
      Followed: [{ fire: B.finish }]
external:
  User: [A.go]
"#;
    assert!(unreachable(text).is_empty());
    // Without the external source nothing moves.
    assert_eq!(
        unreachable(&text.replace("external:\n  User: [A.go]\n", "")),
        ["state:A:a1", "state:B:b1", "state:B:b2"]
    );
}

#[test]
fn emits_of_dead_transitions_trigger_nothing() {
    // `a1 → a2` would emit Went, but `a1` is never entered.
    let text = r#"
machines:
  A:
    states: [a0, a1, a2]
    transitions:
      - { from: a1, to: a2, on: go, emits: [Went] }
  B:
    states: [b0, b1]
    transitions:
      - { from: b0, to: b1, on: follow }
controllers:
  C:
    on:
      Went: [{ fire: B.follow }]
external:
  User: [A.go]
"#;
    assert_eq!(unreachable(text), ["state:A:a1", "state:A:a2", "state:B:b1"]);
}

#[test]
fn a_reachable_source_state_needs_a_triggerable_trigger() {
    let text = r#"
machines:
  M:
    states: [a, b]
    transitions:
      - { from: a, to: b, on: never_fired }
"#;
    assert_eq!(unreachable(text), ["state:M:b"]);
}

#[test]
fn transitions_on_compound_states_apply_to_descendants() {
    let text = r#"
machines:
  M:
    states:
      - running:
          initial: fetching
          states: [fetching, parsing]
      - stopped
    transitions:
      - { from: running, to: stopped, on: stop }
      - { from: stopped, to: running.parsing, on: resume }
external:
  User: [M.stop, M.resume]
"#;
    assert!(unreachable(text).is_empty());
}

#[test]
fn entering_a_compound_state_follows_the_default_entry_chain() {
    let text = r#"
machines:
  M:
    states:
      - idle
      - outer:
          states:
            - inner:
                states: [leaf, other]
            - sibling
    transitions:
      - { from: idle, to: outer, on: go }
external:
  User: [M.go]
"#;
    assert_eq!(unreachable(text), ["state:M:outer.inner.other", "state:M:outer.sibling"]);
}

#[test]
fn a_nested_initial_state_marks_its_ancestors() {
    let text = r#"
machines:
  M:
    initial: outer.second
    states:
      - outer:
          states: [first, second]
"#;
    assert_eq!(unreachable(text), ["state:M:outer.first"]);
}

#[test]
fn history_targets_enter_the_parent() {
    let text = r#"
machines:
  M:
    states:
      - idle
      - running:
          states:
            - a
            - b
            - resume_point: { kind: history }
      - paused
    transitions:
      - { from: idle, to: paused, on: pause }
      - { from: paused, to: running.resume_point, on: resume }
external:
  User: [M.pause, M.resume]
"#;
    // History falls back to the parent's default entry (`a`); `b` is never
    // entered, and the history pseudo-state itself is never reported.
    assert_eq!(unreachable(text), ["state:M:running.b"]);
}

#[test]
fn history_pseudo_states_are_never_reported() {
    let text = r#"
machines:
  M:
    states:
      - running:
          states:
            - a
            - h: { kind: deep-history }
"#;
    assert!(unreachable(text).is_empty());
}

#[test]
fn only_the_outermost_unreachable_state_is_reported() {
    let text = r#"
machines:
  M:
    states:
      - idle
      - returning:
          states:
            - in_transit
            - received:
                states: [inspected, restocked]
            - h: { kind: history }
    transitions:
      - { from: idle, to: returning, on: return_requested }
"#;
    let (model, findings) = run(text);
    let f = single(&findings, Check::UnreachableState);
    assert_eq!(key(&model, f.detail.primary()), "state:M:returning");
    assert_eq!(
        f.message,
        "M.returning and its 5 nested states are never entered: no path from M's initial state reaches them through external triggers or controller fires"
    );
}

#[test]
fn a_top_level_history_state_falls_back_to_the_initial_state() {
    let text = r#"
machines:
  M:
    states:
      - idle
      - busy
      - back: { kind: history }
    transitions:
      - { from: idle, to: busy, on: go }
      - { from: busy, to: back, on: undo }
external:
  User: [M.go, M.undo]
"#;
    assert!(unreachable(text).is_empty());
}

#[test]
fn guards_and_conditions_are_assumed_satisfiable() {
    let text = r#"
machines:
  M:
    states: [a, b]
    transitions:
      - { from: a, to: b, on: go, guard: "false" }
external:
  User: [M.go]
"#;
    assert!(unreachable(text).is_empty());
}
