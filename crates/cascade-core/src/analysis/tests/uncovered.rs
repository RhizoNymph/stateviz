//! The `uncovered-guard` info check: all-expression groups without `else`
//! whose guards leave some input with nothing to take.

use super::*;

fn model(transitions: &str) -> String {
    format!(
        r#"
machines:
  Order:
    fields: {{ id: string, tier: "enum [basic, gold, vip]", n: number, gift: bool }}
    states: [pending, paid, failed, review]
    transitions:
{transitions}
  S:
    states: [s0, s1]
    transitions:
      - {{ from: s0, to: s1, on: send, emits: [Captured] }}
events:
  Captured: {{ payload: {{ id: string, method: "enum [card, wire]", amount: number }} }}
external:
  User: [Order.capture, S.send]
"#
    )
}

fn uncovered(transitions: &str) -> Vec<Finding> {
    let (_, findings) = run(&model(transitions));
    findings.into_iter().filter(|f| f.check() == Check::UncoveredGuard).collect()
}

fn gaps_of(f: &Finding) -> Vec<String> {
    match &f.detail {
        FindingDetail::UncoveredGuard { gaps, .. } => gaps.clone(),
        other => panic!("{other:?}"),
    }
}

#[test]
fn an_uncovered_enum_is_reported_with_the_missing_member() {
    let (model, findings) = run(&model(
        "      - { from: pending, to: paid, on: { event: Captured, where: \"id == event.id\" }, guard: \"event.method == card\" }",
    ));
    let f = single(&findings, Check::UncoveredGuard);
    assert_eq!(f.severity, Severity::Info);
    assert_eq!(f.check().code(), "uncovered-guard");
    assert_eq!(keys(&model, &f.detail.subjects()), ["state:Order:pending", "transition:Order:pending->paid@Captured"]);
    assert_eq!(key(&model, f.detail.primary()), "state:Order:pending");
    assert_eq!(gaps_of(f), ["event.method = wire"]);
    assert_eq!(
        f.message,
        "Order.pending drops `Captured` when event.method = wire: no guard holds and there is no `else`"
    );
}

#[test]
fn numeric_gaps_between_intervals() {
    let found = uncovered(
        "      - { from: pending, to: paid, on: capture, guard: \"self.n > 10\" }\n      - { from: pending, to: failed, on: capture, guard: \"self.n < 0\" }",
    );
    let [f] = found.as_slice() else { panic!("{found:#?}") };
    assert_eq!(gaps_of(f), ["self.n = 10"]);
    assert!(f.message.starts_with("Order.pending drops `capture` when self.n = 10:"), "{}", f.message);
}

#[test]
fn several_gaps_are_listed() {
    let found = uncovered(
        "      - { from: pending, to: paid, on: capture, guard: \"self.tier == gold and self.gift == true\" }",
    );
    let [f] = found.as_slice() else { panic!("{found:#?}") };
    let gaps = gaps_of(f);
    assert!(gaps.len() >= 2, "{gaps:?}");
    assert!(f.message.contains("; or when "), "{}", f.message);
}

#[test]
fn covered_groups_are_quiet() {
    for transitions in [
        // Complementary intervals.
        "      - { from: pending, to: paid, on: capture, guard: \"self.n > 10\" }\n      - { from: pending, to: failed, on: capture, guard: \"self.n <= 10\" }",
        // Every enum member.
        "      - { from: pending, to: paid, on: capture, guard: \"self.tier in [gold, vip]\" }\n      - { from: pending, to: failed, on: capture, guard: \"self.tier == basic\" }",
        // Bool and its negation.
        "      - { from: pending, to: paid, on: capture, guard: \"self.gift == true\" }\n      - { from: pending, to: failed, on: capture, guard: \"not self.gift == true\" }",
        // `not in` and `in`.
        "      - { from: pending, to: paid, on: capture, guard: \"self.tier not in [gold]\" }\n      - { from: pending, to: failed, on: capture, guard: \"self.tier in [gold]\" }",
    ] {
        let found = uncovered(transitions);
        assert!(found.is_empty(), "{transitions}: {found:#?}");
    }
}

#[test]
fn only_missing_values_are_not_a_design_gap() {
    // Every member is covered; only a missing `tier` drops the trigger.
    let found = uncovered(
        "      - { from: pending, to: paid, on: capture, guard: \"self.tier in [gold, vip]\" }\n      - { from: pending, to: failed, on: capture, guard: \"self.tier == basic\" }",
    );
    assert!(found.is_empty(), "{found:#?}");
}

#[test]
fn else_choices_and_unguarded_candidates_opt_out() {
    for transitions in [
        "      - { from: pending, to: paid, on: capture, guard: \"self.n > 10\" }\n      - { from: pending, to: failed, on: capture, guard: else }",
        "      - { from: pending, to: paid, on: capture, guard: \"self.n > 10\" }\n      - { from: pending, to: review, on: capture, guard: \"?manual\" }",
        "      - { from: pending, to: paid, on: capture, guard: \"self.n > 10\" }\n      - { from: pending, to: failed, on: capture }",
        "      - { from: pending, to: paid, on: capture }",
        "      - { from: pending, to: paid, on: capture, guard: \"?approved\" }",
    ] {
        let found = uncovered(transitions);
        assert!(found.is_empty(), "{transitions}: {found:#?}");
    }
}

#[test]
fn unknown_coverage_is_not_reported() {
    let found = uncovered("      - { from: pending, to: paid, on: capture, guard: \"self.id == event.id\" }");
    assert!(found.is_empty(), "{found:#?}");
}

#[test]
fn subscriptions_ignore_inputs_no_filter_accepts() {
    // Different filters: card payments over 100 and wire payments. A card
    // payment of 100 or less passes no filter, so it is ignored, not
    // dropped: covered.
    let found = uncovered(
        "      - { from: pending, to: paid, on: { event: Captured, where: \"event.method == card\" }, guard: \"event.amount > 100\" }\n      - { from: pending, to: failed, on: { event: Captured, where: \"event.method == card\" }, guard: \"event.amount <= 100\" }\n      - { from: pending, to: review, on: { event: Captured, where: \"event.method == wire\" }, guard: \"event.amount > 0\" }",
    );
    let [f] = found.as_slice() else { panic!("{found:#?}") };
    // The wire branch drops non-positive amounts, and a payment without an
    // amount passes a filter but no guard.
    assert_eq!(
        gaps_of(f),
        [
            "event.amount = 0, event.method = wire",
            "event.amount is missing, event.method = wire",
            "event.amount is missing, event.method = card"
        ]
    );
}

#[test]
fn timers_read_only_self() {
    let found = uncovered("      - { from: pending, to: failed, after: 30m, guard: \"self.gift == true\" }");
    let [f] = found.as_slice() else { panic!("{found:#?}") };
    assert_eq!(gaps_of(f), ["self.gift = false"]);
    assert!(f.message.starts_with("Order.pending drops `after-30m` when"), "{}", f.message);
}
