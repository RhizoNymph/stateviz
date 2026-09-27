use super::*;

fn notes(text: &str) -> Vec<(String, Vec<String>)> {
    let (model, findings) = run(text);
    of(&findings, Check::StateDependentFire)
        .iter()
        .map(|f| match &f.detail {
            FindingDetail::StateDependentFire { rule, dropped_in, .. } => (
                key(&model, ElementRef::Rule(*rule)),
                dropped_in.iter().map(|&s| key(&model, ElementRef::State(s))).collect(),
            ),
            other => panic!("{other:?}"),
        })
        .collect()
}

#[test]
fn spec_example_start_is_dropped_once_picking() {
    let (model, findings) = run(examples::SPEC_EXAMPLE);
    let f = single(&findings, Check::StateDependentFire);
    assert_eq!(f.severity, Severity::Info);
    assert_eq!(
        keys(&model, &f.detail.subjects()),
        ["rule:Fulfillment/OrderPaid#0", "state:Shipment:picking", "state:Shipment:shipped"]
    );
    assert_eq!(f.message, "Fulfillment fires Shipment.start on OrderPaid, which Shipment drops in picking and shipped");
}

const HIERARCHY: &str = r#"
machines:
  Src:
    states: [s0, s1]
    transitions:
      - { from: s0, to: s1, on: go, emits: [Go] }
  M:
    states:
      - idle
      - running:
          states:
            - fetching
            - parsing
            - h: { kind: history }
      - done: { kind: final }
    transitions:
      - { from: idle, to: running, on: start }
      - { from: running, to: idle, on: stop }
      - { from: running.fetching, to: running.parsing, on: next }
      - { from: running.parsing, to: done, on: finish }
controllers:
  C:
    on:
      Go:
        - fire: M.stop
        - fire: M.next
        - fire: M.missing
external:
  User: [Src.go, M.start, M.finish]
"#;

#[test]
fn transitions_on_ancestors_cover_their_leaves() {
    // `stop` is declared on `running`, so both its leaves accept it; only
    // `idle` and the final `done` drop it. History states are never listed.
    let found = notes(HIERARCHY);
    assert_eq!(found[0], ("rule:C/Go#0".to_owned(), vec!["state:M:idle".to_owned(), "state:M:done".to_owned()]));
}

#[test]
fn leaves_are_listed_in_model_order_and_final_states_count() {
    let found = notes(HIERARCHY);
    assert_eq!(
        found[1],
        (
            "rule:C/Go#1".to_owned(),
            vec!["state:M:idle".to_owned(), "state:M:running.parsing".to_owned(), "state:M:done".to_owned()]
        )
    );
}

#[test]
fn invalid_fires_get_no_note() {
    let found = notes(HIERARCHY);
    assert_eq!(found.len(), 2, "{found:?}");
    assert!(!found.iter().any(|(rule, _)| rule == "rule:C/Go#2"));
}

#[test]
fn triggers_accepted_everywhere_get_no_note() {
    let text = r#"
machines:
  Src:
    states: [s0, s1]
    transitions:
      - { from: s0, to: s1, on: go, emits: [Go] }
  Lamp:
    states: [off, on]
    transitions:
      - { from: off, to: on, on: toggle }
      - { from: on, to: off, on: toggle }
controllers:
  C:
    on:
      Go: [{ fire: Lamp.toggle }]
external:
  User: [Src.go]
"#;
    assert!(notes(text).is_empty());
}

#[test]
fn spawning_rules_only_consider_the_initial_state() {
    let text = r#"
machines:
  Src:
    states: [s0, s1]
    transitions:
      - { from: s0, to: s1, on: go, emits: [Go] }
  Job:
    fields: [id]
    states:
      - created:
          states: [fresh, primed]
      - running
      - done: { kind: final }
    transitions:
      - { from: created, to: running, on: run }
      - { from: running, to: done, on: finish }
controllers:
  Spawner:
    on:
      Go:
        - fire: Job.run
          target: new Job with id = 1
        - fire: Job.finish
          target: new Job with id = 2
external:
  User: [Src.go]
"#;
    let (model, findings) = run(text);
    let fs = of(&findings, Check::StateDependentFire);
    // `run` lands (a new Job starts in created.fresh); `finish` never does.
    let [f] = fs.as_slice() else { panic!("{fs:#?}") };
    assert_eq!(keys(&model, &f.detail.subjects()), ["rule:Spawner/Go#1", "state:Job:created.fresh"]);
    assert_eq!(f.message, "Spawner fires Job.finish on Go, but a new Job starts in created.fresh, which drops it");
}
