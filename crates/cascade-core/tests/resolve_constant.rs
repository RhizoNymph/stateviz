//! Constant tests: a guard, condition or filter test that reads no field is
//! a load error (`DiagnosticKind::ConstantTest`), since a bare word is a
//! literal outside a filter's left-hand side and `x > 0` would silently be
//! constant.

use cascade_core::{DiagnosticKind, LoadError, load_str};

fn load_err(text: &str) -> LoadError {
    match load_str(text) {
        Ok(_) => panic!("expected a load error:\n{text}"),
        Err(err) => err,
    }
}

fn constant_tests(err: &LoadError) -> Vec<(String, &'static str, Option<String>)> {
    err.diagnostics
        .iter()
        .filter_map(|d| match &d.kind {
            DiagnosticKind::ConstantTest { test, what, suggestion } => Some((test.clone(), *what, suggestion.clone())),
            _ => None,
        })
        .collect()
}

fn machine_with(transition: &str) -> String {
    format!(
        "machines:\n  M:\n    states: [x, y]\n    transitions:\n      - {transition}\nevents:\n  E: {{ payload: [n] }}\nexternal:\n  U: [M.go]\n"
    )
}

#[test]
fn a_guard_comparing_two_literals_is_an_error_with_a_hint() {
    let err = load_err(&machine_with("{ from: x, to: y, on: go, guard: \"x > 0\" }"));
    assert_eq!(
        constant_tests(&err),
        [("x > 0".to_owned(), "compares two literals", Some("`self.x` or `event.x`".to_owned()))]
    );
    assert_eq!(err.diagnostics.len(), 1, "{err}");
    let text = err.to_string();
    assert!(
        text.contains(
            "`x > 0` compares two literals, so it never depends on the data; did you mean `self.x` or `event.x`?"
        ),
        "{text}"
    );
    // Points at the guard string.
    assert_eq!(err.diagnostics[0].span.start.line, 5);
}

#[test]
fn every_constant_test_inside_a_larger_expression_is_reported() {
    let err =
        load_err(&machine_with("{ from: x, to: y, on: go, guard: \"event.n > 1 and (tier == gold or 1 == 1)\" }"));
    assert_eq!(
        constant_tests(&err),
        [
            ("tier == gold".to_owned(), "compares two literals", Some("`self.tier` or `event.tier`".to_owned())),
            ("1 == 1".to_owned(), "compares two literals", None),
        ]
    );
}

#[test]
fn in_on_a_literal_is_an_error() {
    let err = load_err(&machine_with("{ from: x, to: y, on: go, guard: \"not tier in [a, b]\" }"));
    assert_eq!(
        constant_tests(&err),
        [("tier in [a, b]".to_owned(), "tests a literal", Some("`self.tier` or `event.tier`".to_owned()))]
    );
}

#[test]
fn quoted_numbers_and_bools_are_not_suggested_as_fields() {
    let err = load_err(&machine_with("{ from: x, to: y, on: go, guard: \"\\\"a\\\" == true\" }"));
    let found = constant_tests(&err);
    assert_eq!(found.len(), 1, "{err}");
    assert_eq!(found[0].2, None);
}

#[test]
fn a_rule_condition_suggests_only_event_fields() {
    let text = "machines:\n  M:\n    states: [x, y]\n    transitions: [{ from: x, to: y, on: go, emits: [E] }]\ncontrollers:\n  C:\n    on:\n      E: { fire: M.go, when: \"policy == sanctioned\" }\nexternal:\n  U: [M.go]\n";
    let err = load_err(text);
    assert_eq!(
        constant_tests(&err),
        [("policy == sanctioned".to_owned(), "compares two literals", Some("`event.policy`".to_owned()))]
    );
}

#[test]
fn filters_read_the_left_name_as_a_field() {
    // `n == 1` in a filter is `self.n == 1`: fine.
    let ok = "machines:\n  M:\n    fields: [n]\n    states: [x, y]\n    transitions:\n      - { from: x, to: y, on: { event: E, where: \"n == 1\" } }\n  S:\n    states: [a, b]\n    transitions: [{ from: a, to: b, on: go, emits: [E] }]\nexternal:\n  U: [S.go]\n";
    assert!(load_str(ok).is_ok());
    // A literal on the left (a number cannot be a field name) is constant.
    let bad = ok.replace("n == 1", "1 == n");
    let err = load_err(&bad);
    assert_eq!(constant_tests(&err).len(), 1, "{err}");
    // A rule's target filter too.
    let target = "machines:\n  M:\n    fields: [n]\n    states: [x, y]\n    transitions: [{ from: x, to: y, on: go, emits: [E] }]\ncontrollers:\n  C:\n    on:\n      E: { fire: M.go, target: \"M where 2 > 1\" }\nexternal:\n  U: [M.go]\n";
    let err = load_err(target);
    assert_eq!(constant_tests(&err), [("2 > 1".to_owned(), "compares two literals", None)]);
}

#[test]
fn tests_reading_a_field_are_fine() {
    for guard in ["self.x > 0", "event.x > 0", "0 < event.x", "event.kind in [a, b]", "event.a == event.b"] {
        let text = machine_with(&format!("{{ from: x, to: y, on: go, guard: \"{guard}\" }}"));
        assert!(load_str(&text).is_ok(), "{guard}");
    }
}
