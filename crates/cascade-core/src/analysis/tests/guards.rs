//! Nondeterminism decided by the expression solver: enums, numbers, bools,
//! `in`, `not`, choices, mixed groups, `else`, and subscription filters.

use super::*;

/// One machine `M` with typed fields whose `a` state has the given
/// transitions; `M.go` is fired externally and `E` (typed payload) is
/// emitted by `S`, which an external source drives.
fn model(transitions: &str) -> String {
    format!(
        r#"
machines:
  M:
    fields: {{ id: string, tier: "enum [basic, gold, vip]", n: number, gift: bool, note: any }}
    states: [a, b, c, d]
    transitions:
{transitions}
  S:
    states: [s0, s1]
    transitions:
      - {{ from: s0, to: s1, on: send, emits: [E] }}
events:
  E: {{ payload: {{ id: string, method: "enum [card, wire, cash]", amount: number }} }}
external:
  User: [M.go, S.send]
"#
    )
}

fn nondeterminism(transitions: &str) -> Vec<String> {
    let (_, findings) = run(&model(transitions));
    of(&findings, Check::Nondeterminism).iter().map(|f| f.message.clone()).collect()
}

fn go(guards: &[&str]) -> String {
    let targets = ["b", "c", "d"];
    guards
        .iter()
        .zip(targets)
        .map(|(g, to)| format!("      - {{ from: a, to: {to}, on: go, guard: {g} }}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn exclusive(guards: &[&str]) {
    let found = nondeterminism(&go(guards));
    assert!(found.is_empty(), "{guards:?} should be exclusive: {found:#?}");
}

fn ambiguous(guards: &[&str]) -> String {
    let found = nondeterminism(&go(guards));
    match found.as_slice() {
        [one] => one.clone(),
        other => panic!("{guards:?} should be ambiguous once: {other:#?}"),
    }
}

#[test]
fn enum_members_are_exclusive() {
    exclusive(&["\"self.tier == gold\"", "\"self.tier == basic\""]);
    exclusive(&["\"self.tier in [gold, vip]\"", "\"self.tier == basic\""]);
    exclusive(&["\"self.tier == gold\"", "\"self.tier != gold\""]);
    let msg = ambiguous(&["\"self.tier != gold\"", "\"self.tier != basic\""]);
    assert_eq!(
        msg,
        "M.a has 2 transitions on `go` without mutually exclusive guards: `self.tier != gold` and `self.tier != basic` both hold when self.tier = vip"
    );
}

#[test]
fn numeric_intervals() {
    exclusive(&["\"self.n > 10\"", "\"self.n <= 10\""]);
    exclusive(&["\"self.n < 0\"", "\"self.n >= 0 and self.n < 5\"", "\"self.n >= 5\""]);
    let msg = ambiguous(&["\"event.amount > 0\"", "\"event.amount < 10\""]);
    assert!(msg.ends_with("`event.amount > 0` and `event.amount < 10` both hold when event.amount = 1"), "{msg}");
}

#[test]
fn numeric_overlap_names_a_witness() {
    let msg = ambiguous(&["\"self.n >= 5\"", "\"self.n <= 5\""]);
    assert!(msg.ends_with("`self.n >= 5` and `self.n <= 5` both hold when self.n = 5"), "{msg}");
}

#[test]
fn bools() {
    exclusive(&["\"self.gift == true\"", "\"self.gift == false\""]);
    exclusive(&["\"self.gift == true\"", "\"not self.gift == true and self.gift == false\""]);
    let msg = ambiguous(&["\"self.gift == true\"", "\"self.gift != false\""]);
    assert!(msg.ends_with("both hold when self.gift = true"), "{msg}");
}

#[test]
fn in_lists_and_negation() {
    exclusive(&["\"self.tier in [gold, vip]\"", "\"self.tier not in [gold, vip]\""]);
    exclusive(&["\"not (self.tier == gold or self.tier == vip)\"", "\"self.tier == vip\""]);
    // Both negations hold when the field is missing.
    let msg = ambiguous(&["\"not self.n > 5\"", "\"not self.n <= 5\""]);
    assert!(msg.ends_with("both hold when self.n is missing"), "{msg}");
}

#[test]
fn different_fields_may_overlap() {
    let msg = ambiguous(&["\"self.tier == gold\"", "\"self.n > 3\""]);
    assert!(msg.contains("`self.tier == gold` and `self.n > 3` both hold when"), "{msg}");
    // A conjunction on one field is still exclusive.
    exclusive(&["\"self.tier == gold and self.n > 3\"", "\"self.tier == basic\""]);
}

#[test]
fn two_field_tests_are_conservative() {
    let msg = ambiguous(&["\"self.id == event.id\"", "\"self.id != event.id\""]);
    assert!(msg.ends_with("`self.id == event.id` and `self.id != event.id` may both hold"), "{msg}");
}

#[test]
fn identical_expressions_are_duplicates() {
    let msg = ambiguous(&["\"self.n > 1\"", "\"self.n  >  1\""]);
    assert!(msg.ends_with("the guard `self.n > 1` appears 2 times"), "{msg}");
}

#[test]
fn distinct_choices_are_exclusive_and_repeated_ones_are_not() {
    exclusive(&["\"?card declined\"", "\"?card accepted\""]);
    exclusive(&["\"?card declined\"", "\"?card accepted\"", "else"]);
    let msg = ambiguous(&["\"?card declined\"", "\"? card declined\""]);
    assert!(msg.ends_with("the guard `?card declined` appears 2 times"), "{msg}");
}

#[test]
fn an_expression_next_to_a_choice_is_mixed() {
    let msg = ambiguous(&["\"self.n > 0\"", "\"?manual review\""]);
    assert_eq!(
        msg,
        "M.a has 2 transitions on `go` without mutually exclusive guards: mixed guards: `self.n > 0` may hold while the choice `?manual review` is offered"
    );
    // A third, exclusive candidate does not hide the mixed pair.
    let msg = ambiguous(&["\"?manual review\"", "\"self.n > 0\"", "\"self.n <= 0\""]);
    assert!(msg.contains("mixed guards"), "{msg}");
}

#[test]
fn else_is_exclusive_with_everything_but_another_else() {
    exclusive(&["\"self.n > 5\"", "else"]);
    exclusive(&["\"?approve\"", "else"]);
    let msg = ambiguous(&["else", "\"self.n > 0\"", "else"]);
    assert!(msg.ends_with("2 of them are guarded by `else`"), "{msg}");
    // `else` does not make two overlapping expressions exclusive.
    let msg = ambiguous(&["\"self.n > 0\"", "\"self.n > 5\"", "else"]);
    assert!(msg.contains("`self.n > 0` and `self.n > 5` both hold when self.n = 6"), "{msg}");
}

#[test]
fn an_unguarded_candidate_still_competes() {
    let found = nondeterminism(&format!("{}\n      - {{ from: a, to: d, on: go }}", go(&["else"])));
    let [msg] = found.as_slice() else { panic!("{found:#?}") };
    assert!(msg.ends_with("M: a → d has no guard"), "{msg}");
}

#[test]
fn subscription_filters_join_the_guard() {
    // Exclusive by filter alone, no guards.
    let found = nondeterminism(
        "      - { from: a, to: b, on: { event: E, where: \"event.method == card\" } }\n      - { from: a, to: c, on: { event: E, where: \"event.method == wire\" } }",
    );
    assert!(found.is_empty(), "{found:#?}");
    // Same instance filter, guards on the payload (typed by the event).
    let found = nondeterminism(
        "      - { from: a, to: b, on: { event: E, where: \"id == event.id\" }, guard: \"event.amount > 100\" }\n      - { from: a, to: c, on: { event: E, where: \"id == event.id\" }, guard: \"event.amount <= 100\" }",
    );
    assert!(found.is_empty(), "{found:#?}");
    // A filter and a guard that can hold together.
    let found = nondeterminism(
        "      - { from: a, to: b, on: { event: E, where: \"event.amount > 5\" } }\n      - { from: a, to: c, on: { event: E }, guard: \"event.amount < 10\" }",
    );
    let [msg] = found.as_slice() else { panic!("{found:#?}") };
    assert!(msg.ends_with("`event.amount > 5` and `event.amount < 10` both hold when event.amount = 6"), "{msg}");
}

#[test]
fn a_subscription_without_filter_or_guard_is_unguarded() {
    let found = nondeterminism(
        "      - { from: a, to: b, on: { event: E } }\n      - { from: a, to: c, on: { event: E, where: \"event.method == card\" } }",
    );
    let [msg] = found.as_slice() else { panic!("{found:#?}") };
    assert!(msg.ends_with("M: a → b has no guard"), "{msg}");
}

#[test]
fn same_choice_under_overlapping_filters_is_ambiguous() {
    let found = nondeterminism(
        "      - { from: a, to: b, on: { event: E, where: \"event.amount > 0\" }, guard: \"?ok\" }\n      - { from: a, to: c, on: { event: E }, guard: \"?ok\" }",
    );
    let [msg] = found.as_slice() else { panic!("{found:#?}") };
    assert!(msg.ends_with("`event.amount > 0 and ?ok` and `?ok` may both hold"), "{msg}");
    // Exclusive filters keep the same choice apart.
    let found = nondeterminism(
        "      - { from: a, to: b, on: { event: E, where: \"event.amount > 0\" }, guard: \"?ok\" }\n      - { from: a, to: c, on: { event: E, where: \"event.amount <= 0\" }, guard: \"?ok\" }",
    );
    assert!(found.is_empty(), "{found:#?}");
}

#[test]
fn timer_groups_are_checked_too() {
    let found = nondeterminism(
        "      - { from: a, to: b, after: 5m, guard: \"self.n > 0\" }\n      - { from: a, to: c, after: 5m, guard: \"self.n < 3\" }",
    );
    let [msg] = found.as_slice() else { panic!("{found:#?}") };
    assert!(msg.contains("transitions on `after-5m`"), "{msg}");
    let found = nondeterminism(
        "      - { from: a, to: b, after: 5m, guard: \"self.gift == true\" }\n      - { from: a, to: c, after: 5m, guard: else }",
    );
    assert!(found.is_empty(), "{found:#?}");
}

#[test]
fn the_detail_lists_every_candidate() {
    let (model, findings) = run(&model(&go(&["\"self.n > 0\"", "\"self.n > 5\"", "\"self.n < 0\""])));
    let f = single(&findings, Check::Nondeterminism);
    assert_eq!(
        keys(&model, &f.detail.subjects()),
        ["state:M:a", "transition:M:a->b@go", "transition:M:a->c@go", "transition:M:a->d@go"]
    );
}
