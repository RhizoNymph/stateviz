use super::*;

/// One machine whose `a` state has the given transitions on `go`, all
/// reachable from an external source so no other check fires.
fn with_transitions(transitions: &str) -> String {
    format!(
        r#"
machines:
  M:
    states: [a, b, c, d]
    transitions:
{transitions}
external:
  User: [M.go]
"#
    )
}

fn nondeterminism(text: &str) -> (Model, Vec<Finding>) {
    let (model, findings) = run(text);
    let only: Vec<Finding> = findings.into_iter().filter(|f| f.check() == Check::Nondeterminism).collect();
    (model, only)
}

#[test]
fn two_unguarded_transitions_on_one_trigger_are_an_error() {
    let (model, findings) =
        nondeterminism(&with_transitions("      - { from: a, to: b, on: go }\n      - { from: a, to: c, on: go }"));
    let [f] = findings.as_slice() else { panic!("{findings:#?}") };
    assert_eq!(f.severity, Severity::Error);
    assert_eq!(keys(&model, &f.detail.subjects()), ["state:M:a", "transition:M:a->b@go", "transition:M:a->c@go"]);
    assert_eq!(key(&model, f.detail.primary()), "state:M:a");
    assert_eq!(f.message, "M.a has 2 transitions on `go` without mutually exclusive guards: M: a → b has no guard");
}

#[test]
fn one_unguarded_candidate_is_enough() {
    let (_, findings) = nondeterminism(&with_transitions(
        "      - { from: a, to: b, on: go, guard: \"x > 0\" }\n      - { from: a, to: c, on: go }",
    ));
    let [f] = findings.as_slice() else { panic!("{findings:#?}") };
    assert!(f.message.ends_with("M: a → c has no guard"), "{}", f.message);
}

#[test]
fn blank_guards_count_as_missing() {
    let (_, findings) = nondeterminism(&with_transitions(
        "      - { from: a, to: b, on: go, guard: \"x > 0\" }\n      - { from: a, to: c, on: go, guard: \"   \" }",
    ));
    assert_eq!(findings.len(), 1, "{findings:#?}");
}

#[test]
fn distinct_guards_are_mutually_exclusive() {
    let (_, findings) = nondeterminism(&with_transitions(
        "      - { from: a, to: b, on: go, guard: \"x > 0\" }\n      - { from: a, to: c, on: go, guard: \"x <= 0\" }",
    ));
    assert!(findings.is_empty(), "{findings:#?}");
}

#[test]
fn one_else_with_other_guards_is_fine() {
    let (_, findings) = nondeterminism(&with_transitions(
        "      - { from: a, to: b, on: go, guard: \"x > 0\" }\n      - { from: a, to: c, on: go, guard: \"x < 0\" }\n      - { from: a, to: d, on: go, guard: else }",
    ));
    assert!(findings.is_empty(), "{findings:#?}");
}

#[test]
fn two_else_guards_are_an_error() {
    let (_, findings) = nondeterminism(&with_transitions(
        "      - { from: a, to: b, on: go, guard: else }\n      - { from: a, to: c, on: go, guard: \" else \" }",
    ));
    let [f] = findings.as_slice() else { panic!("{findings:#?}") };
    assert!(f.message.ends_with("2 of them are guarded by `else`"), "{}", f.message);
}

#[test]
fn guards_equal_after_whitespace_normalization_are_duplicates() {
    let (model, findings) = nondeterminism(&with_transitions(
        "      - { from: a, to: b, on: go, guard: \"x  >   0\" }\n      - { from: a, to: c, on: go, guard: \"y > 0\" }\n      - { from: a, to: d, on: go, guard: \" x > 0\" }",
    ));
    let [f] = findings.as_slice() else { panic!("{findings:#?}") };
    // Every candidate is listed, not only the duplicated pair.
    assert_eq!(
        keys(&model, &f.detail.subjects())[1..],
        ["transition:M:a->b@go", "transition:M:a->c@go", "transition:M:a->d@go"]
    );
    assert_eq!(
        f.message,
        "M.a has 3 transitions on `go` without mutually exclusive guards: the guard `x > 0` appears 2 times"
    );
}

#[test]
fn exact_duplicate_transitions_are_an_error() {
    let (model, findings) =
        nondeterminism(&with_transitions("      - { from: a, to: b, on: go }\n      - { from: a, to: b, on: go }"));
    let [f] = findings.as_slice() else { panic!("{findings:#?}") };
    assert_eq!(keys(&model, &f.detail.subjects())[1..], ["transition:M:a->b@go", "transition:M:a->b@go#1"]);
}

#[test]
fn different_triggers_or_states_do_not_conflict() {
    let (_, findings) = nondeterminism(&with_transitions(
        "      - { from: a, to: b, on: go }\n      - { from: a, to: c, on: stop }\n      - { from: b, to: c, on: go }\n      - { from: c, to: d, on: go }",
    ));
    assert!(findings.is_empty(), "{findings:#?}");
}

#[test]
fn from_lists_expand_to_separate_states() {
    let (_, findings) = nondeterminism(&with_transitions(
        "      - { from: [a, b], to: c, on: go }\n      - { from: c, to: d, on: go }",
    ));
    assert!(findings.is_empty(), "{findings:#?}");
}

#[test]
fn inherited_transitions_do_not_conflict_with_the_childs_own() {
    let (_, findings) = nondeterminism(
        r#"
machines:
  M:
    states:
      - running:
          states: [fetching, parsing]
      - stopped
    transitions:
      - { from: running, to: stopped, on: go }
      - { from: running.fetching, to: running.parsing, on: go }
external:
  User: [M.go]
"#,
    );
    assert!(findings.is_empty(), "{findings:#?}");
}

#[test]
fn conflicts_on_compound_states_are_reported_on_that_state() {
    let (model, findings) = nondeterminism(
        r#"
machines:
  M:
    states:
      - running:
          states: [fetching, parsing]
      - stopped
      - failed
    transitions:
      - { from: running, to: stopped, on: go }
      - { from: running, to: failed, on: go }
external:
  User: [M.go]
"#,
    );
    let [f] = findings.as_slice() else { panic!("{findings:#?}") };
    assert_eq!(key(&model, f.detail.primary()), "state:M:running");
}

#[test]
fn each_state_and_trigger_pair_is_reported_once() {
    let (model, findings) = nondeterminism(&with_transitions(
        "      - { from: a, to: b, on: go }\n      - { from: a, to: c, on: go }\n      - { from: a, to: d, on: go }\n      - { from: b, to: c, on: stop }\n      - { from: b, to: d, on: stop }",
    ));
    assert_eq!(
        findings.iter().map(|f| key(&model, f.detail.primary())).collect::<Vec<_>>(),
        ["state:M:a", "state:M:b"]
    );
    match &findings[0].detail {
        FindingDetail::Nondeterminism { transitions, .. } => assert_eq!(transitions.len(), 3),
        other => panic!("{other:?}"),
    }
}
