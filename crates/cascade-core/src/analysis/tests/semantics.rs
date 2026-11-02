//! The checks over the semantics-phase language: rule conditions and target
//! filters in races, subscriptions and timers in races, reachability,
//! cycles, event handling and state-dependent fires.

use super::*;
use crate::analysis::RaceParty;

/// `(origin, machine, first, second)` keys of every race finding, rule
/// pairs and subscription pairs alike.
fn races(text: &str) -> Vec<[String; 4]> {
    let (model, findings) = run(text);
    let party = |p: RaceParty| key(&model, p.element());
    of(&findings, Check::RaceCandidate)
        .iter()
        .map(|f| match f.detail {
            FindingDetail::RaceCandidate { origin, machine, first, second } => [
                key(&model, ElementRef::Event(origin)),
                key(&model, ElementRef::Machine(machine)),
                key(&model, ElementRef::Rule(first)),
                key(&model, ElementRef::Rule(second)),
            ],
            FindingDetail::SubscriptionRace { origin, machine, first, second } => [
                key(&model, ElementRef::Event(origin)),
                key(&model, ElementRef::Machine(machine)),
                party(first),
                party(second),
            ],
            ref other => panic!("{other:?}"),
        })
        .collect()
}

fn race(origin: &str, machine: &str, first: &str, second: &str) -> [String; 4] {
    [origin.to_owned(), machine.to_owned(), first.to_owned(), second.to_owned()]
}

/// Screening decides the policy; Sanction and Release react to it on the
/// same Case. `{sanction}` and `{release}` are their rule bodies.
fn policy(sanction: &str, release: &str) -> String {
    format!(
        r#"
machines:
  Screening:
    states: [open, decided]
    transitions:
      - {{ from: open, to: decided, on: decide, emits: [Decided] }}
  Case:
    fields: {{ caseId: string, policy: "enum [sanctioned, unsanctioned]" }}
    states: [pending, blocked, released]
    transitions:
      - {{ from: pending, to: blocked, on: block }}
      - {{ from: pending, to: released, on: release }}
events:
  Decided: {{ payload: {{ caseId: string, policy: "enum [sanctioned, unsanctioned]", score: number }} }}
controllers:
  Sanction:
    on:
      Decided: [{sanction}]
  Release:
    on:
      Decided: [{release}]
external:
  Analyst: [Screening.decide]
"#
    )
}

const CASE: &str = "Case where caseId == event.caseId";

#[test]
fn rules_without_conditions_race() {
    let text = policy(
        &format!("{{ fire: Case.block, target: {CASE} }}"),
        &format!("{{ fire: Case.release, target: {CASE} }}"),
    );
    assert_eq!(
        races(&text),
        [race("event:Decided", "machine:Case", "rule:Sanction/Decided#0", "rule:Release/Decided#0")]
    );
}

#[test]
fn exclusive_conditions_do_not_race() {
    let text = policy(
        &format!("{{ fire: Case.block, target: {CASE}, when: \"event.policy == sanctioned\" }}"),
        &format!("{{ fire: Case.release, target: {CASE}, when: \"event.policy == unsanctioned\" }}"),
    );
    assert!(races(&text).is_empty());
    // Numeric conditions too.
    let text = policy(
        &format!("{{ fire: Case.block, target: {CASE}, when: \"event.score >= 80\" }}"),
        &format!("{{ fire: Case.release, target: {CASE}, when: \"event.score < 80\" }}"),
    );
    assert!(races(&text).is_empty());
}

#[test]
fn overlapping_or_choice_conditions_still_race() {
    let text = policy(
        &format!("{{ fire: Case.block, target: {CASE}, when: \"event.score >= 50\" }}"),
        &format!("{{ fire: Case.release, target: {CASE}, when: \"event.score < 80\" }}"),
    );
    assert_eq!(races(&text).len(), 1);
    // Two decisions are independent, whatever their labels.
    let text = policy(
        &format!("{{ fire: Case.block, target: {CASE}, when: \"?looks bad\" }}"),
        &format!("{{ fire: Case.release, target: {CASE}, when: \"?looks fine\" }}"),
    );
    assert_eq!(races(&text).len(), 1);
}

#[test]
fn target_filters_testing_the_event_against_exclusive_literals_do_not_race() {
    let text = policy(
        "{ fire: Case.block, target: \"Case where caseId == event.caseId and event.policy == sanctioned\" }",
        "{ fire: Case.release, target: \"Case where caseId == event.caseId and event.policy == unsanctioned\" }",
    );
    assert!(races(&text).is_empty());
    // A condition on one side and a filter on the other combine.
    let text = policy(
        &format!("{{ fire: Case.block, target: {CASE}, when: \"event.policy == sanctioned\" }}"),
        "{ fire: Case.release, target: \"Case where caseId == event.caseId and event.policy != sanctioned\" }",
    );
    assert!(races(&text).is_empty());
}

#[test]
fn instance_predicates_are_not_compared() {
    // Selecting by different instance fields may still hit one instance.
    let text = policy(
        "{ fire: Case.block, target: \"Case where policy == sanctioned\" }",
        "{ fire: Case.release, target: \"Case where policy == unsanctioned\" }",
    );
    assert_eq!(races(&text).len(), 1);
}

#[test]
fn conditions_on_different_events_are_not_compared() {
    // Release reacts to a later event (relayed from Decided); its condition
    // reads that event, not Decided.
    let text = r#"
machines:
  Screening:
    states: [open, decided, closed]
    transitions:
      - { from: open, to: decided, on: decide, emits: [Decided] }
      - { from: decided, to: closed, on: note, emits: [Noted] }
  Case:
    states: [pending, blocked, released]
    transitions:
      - { from: pending, to: blocked, on: block }
      - { from: pending, to: released, on: release }
events:
  Decided: { payload: { policy: "enum [sanctioned, unsanctioned]" } }
  Noted: { payload: { policy: "enum [sanctioned, unsanctioned]" } }
controllers:
  Relay:
    on:
      Decided: [{ fire: Screening.note }]
  Sanction:
    on:
      Decided: [{ fire: Case.block, when: "event.policy == sanctioned" }]
  Release:
    on:
      Noted: [{ fire: Case.release, when: "event.policy == unsanctioned" }]
external:
  Analyst: [Screening.decide]
"#;
    assert_eq!(races(text).len(), 1);
}

/// Checkout subscribes to Paid; Billing fires Checkout.close on Paid too.
const SUB_AND_FIRE: &str = r#"
machines:
  Order:
    fields: [orderId]
    states: [placed, paid]
    transitions:
      - { from: placed, to: paid, on: pay, emits: [Paid] }
  Checkout:
    fields: [orderId]
    states: [waiting, paid, closed]
    transitions:
      - { from: waiting, to: paid, on: { event: Paid, where: "orderId == event.orderId" } }
      - { from: [waiting, paid], to: closed, on: close }
controllers:
  Billing:
    on:
      Paid: [{ fire: Checkout.close, target: Checkout where orderId == event.orderId }]
external:
  Customer: [Order.pay]
"#;

#[test]
fn a_subscription_and_a_fire_from_one_event_race() {
    let (model, findings) = run(SUB_AND_FIRE);
    let f = single(&findings, Check::RaceCandidate);
    assert_eq!(f.severity, Severity::Info);
    assert_eq!(keys(&model, &f.detail.subjects()), ["rule:Billing/Paid#0", "trigger:Checkout.Paid"]);
    assert_eq!(key(&model, f.detail.primary()), "rule:Billing/Paid#0");
    assert_eq!(
        f.message,
        "Paid leads Billing to fire Checkout.close and Checkout to receive Paid, which may hit the same Checkout instance in either order"
    );
    assert_eq!(
        races(SUB_AND_FIRE),
        [race("event:Paid", "machine:Checkout", "rule:Billing/Paid#0", "trigger:Checkout.Paid")]
    );
}

#[test]
fn a_fire_caused_by_the_subscription_is_ordered() {
    // Billing reacts to what the subscription's transition emits.
    let text = SUB_AND_FIRE
        .replace(
            "on: { event: Paid, where: \"orderId == event.orderId\" } }",
            "on: { event: Paid, where: \"orderId == event.orderId\" }, emits: [Settled] }",
        )
        .replace("      Paid: [{ fire: Checkout.close", "      Settled: [{ fire: Checkout.close");
    assert!(races(&text).is_empty(), "{text}");
}

#[test]
fn a_subscription_and_a_fire_with_exclusive_conditions_do_not_race() {
    let text = SUB_AND_FIRE
        .replace(
            "where: \"orderId == event.orderId\" } }",
            "where: \"orderId == event.orderId and event.method == card\" } }",
        )
        .replace(
            "target: Checkout where orderId == event.orderId }",
            "target: Checkout where orderId == event.orderId, when: \"event.method == wire\" }",
        );
    assert!(races(&text).is_empty(), "{text}");
}

#[test]
fn subscriptions_to_events_of_unrelated_emitters_race() {
    let text = r#"
machines:
  Order:
    states: [placed, paid]
    transitions:
      - { from: placed, to: paid, on: pay, emits: [Paid] }
  Bank:
    states: [idle, captured]
    transitions:
      - { from: idle, to: captured, on: capture, emits: [Captured] }
  Stock:
    states: [idle, reserved]
    transitions:
      - { from: idle, to: reserved, on: reserve, emits: [Reserved] }
  Checkout:
    states: [waiting, half, done]
    transitions:
      - { from: waiting, to: half, on: { event: Captured } }
      - { from: waiting, to: half, on: { event: Reserved } }
      - { from: half, to: done, on: { event: Captured } }
      - { from: half, to: done, on: { event: Reserved } }
controllers:
  Fanout:
    on:
      Paid: [{ fire: Bank.capture }]
  Picking:
    on:
      Paid: [{ fire: Stock.reserve }]
external:
  Customer: [Order.pay]
"#;
    assert_eq!(
        races(text),
        [race("event:Paid", "machine:Checkout", "trigger:Checkout.Captured", "trigger:Checkout.Reserved")]
    );
}

#[test]
fn subscriptions_to_events_of_one_emitter_or_in_causal_order_do_not_race() {
    // One transition emits both: every instance sees them in that order.
    let text = r#"
machines:
  Order:
    states: [placed, paid]
    transitions:
      - { from: placed, to: paid, on: pay, emits: [Captured, Reserved] }
  Checkout:
    states: [waiting, half, done]
    transitions:
      - { from: waiting, to: half, on: { event: Captured } }
      - { from: half, to: done, on: { event: Reserved } }
external:
  Customer: [Order.pay]
"#;
    assert!(races(text).is_empty());
    // Reserved is emitted downstream of Captured (through a relay).
    let text = r#"
machines:
  Order:
    states: [placed, paid]
    transitions:
      - { from: placed, to: paid, on: pay, emits: [Captured] }
  Stock:
    states: [idle, reserved]
    transitions:
      - { from: idle, to: reserved, on: reserve, emits: [Reserved] }
  Checkout:
    states: [waiting, half, done]
    transitions:
      - { from: waiting, to: half, on: { event: Captured } }
      - { from: half, to: done, on: { event: Reserved } }
controllers:
  Relay:
    on:
      Captured: [{ fire: Stock.reserve }]
external:
  Customer: [Order.pay]
"#;
    assert!(races(text).is_empty());
}

#[test]
fn subscriptions_of_different_machines_do_not_race() {
    let text = r#"
machines:
  Order:
    states: [placed, paid]
    transitions:
      - { from: placed, to: paid, on: pay, emits: [Paid] }
  A:
    states: [x, y]
    transitions: [{ from: x, to: y, on: { event: Paid } }]
  B:
    states: [x, y]
    transitions: [{ from: x, to: y, on: { event: Paid } }]
external:
  Customer: [Order.pay]
"#;
    assert!(races(text).is_empty());
}

// --- Reachability --------------------------------------------------------

fn unreachable(text: &str) -> Vec<String> {
    let (model, findings) = run(text);
    of(&findings, Check::UnreachableState).iter().map(|f| key(&model, f.detail.primary())).collect()
}

#[test]
fn a_timer_fires_whenever_its_source_is_reachable() {
    let text = r#"
machines:
  Session:
    states: [active, idle, expired, orphan, gone]
    transitions:
      - { from: active, to: idle, after: 5m }
      - { from: idle, to: expired, after: 30m }
      - { from: orphan, to: gone, after: 1s }
"#;
    assert_eq!(unreachable(text), ["state:Session:orphan", "state:Session:gone"]);
}

#[test]
fn a_subscription_fires_when_its_event_can_be_emitted() {
    let text = r#"
machines:
  Order:
    states: [placed, paid]
    transitions:
      - { from: placed, to: paid, on: pay, emits: [Paid] }
  Checkout:
    states: [waiting, paid, refunded]
    transitions:
      - { from: waiting, to: paid, on: { event: Paid } }
      - { from: paid, to: refunded, on: { event: Refunded } }
events: [Paid, Refunded]
external:
  Customer: [Order.pay]
"#;
    assert_eq!(unreachable(text), ["state:Checkout:refunded"]);
}

#[test]
fn subscriptions_and_timers_chain_across_machines() {
    // A timer emits an event a subscription takes, whose transition emits
    // an event a controller turns into a fire.
    let text = r#"
machines:
  Clock:
    states: [ticking, rang]
    transitions:
      - { from: ticking, to: rang, after: 1h, emits: [Rang] }
  Sleeper:
    states: [asleep, awake]
    transitions:
      - { from: asleep, to: awake, on: { event: Rang }, emits: [Woke] }
  Coffee:
    states: [cold, brewing]
    transitions:
      - { from: cold, to: brewing, on: brew }
controllers:
  Kitchen:
    on:
      Woke: [{ fire: Coffee.brew }]
"#;
    assert!(unreachable(text).is_empty());
}

// --- Events ----------------------------------------------------------------

#[test]
fn an_event_consumed_only_by_subscriptions_is_handled() {
    let text = r#"
machines:
  Order:
    states: [placed, paid]
    transitions:
      - { from: placed, to: paid, on: pay, emits: [Paid] }
  Checkout:
    states: [waiting, paid]
    transitions:
      - { from: waiting, to: paid, on: { event: Paid } }
external:
  Customer: [Order.pay]
"#;
    let (_, findings) = run(text);
    assert!(of(&findings, Check::UnhandledEvent).is_empty(), "{findings:#?}");
}

// --- Cycles ----------------------------------------------------------------

fn cycle_keys(text: &str) -> Vec<Vec<String>> {
    let (model, findings) = run(text);
    of(&findings, Check::CascadeCycle)
        .iter()
        .map(|f| {
            f.detail
                .cycle_steps()
                .iter()
                .map(|s| {
                    let rule = s.rule.map_or_else(|| "subscription".to_owned(), |r| key(&model, ElementRef::Rule(r)));
                    format!("{} {rule}", key(&model, ElementRef::Transition(s.transition)))
                })
                .collect()
        })
        .collect()
}

#[test]
fn cycles_through_subscriptions_are_found() {
    let text = r#"
machines:
  Ping:
    states: [a, b]
    transitions:
      - { from: a, to: b, on: { event: Pong }, emits: [Ping] }
      - { from: b, to: a, on: start, emits: [Ping] }
  Pong:
    states: [a, b]
    transitions:
      - { from: a, to: b, on: { event: Ping }, emits: [Pong] }
external:
  User: [Ping.start]
"#;
    assert_eq!(
        cycle_keys(text),
        [vec![
            "transition:Ping:a->b@Pong subscription".to_owned(),
            "transition:Pong:a->b@Ping subscription".to_owned()
        ]]
    );
}

#[test]
fn mixed_subscription_and_fire_cycles_are_found() {
    let text = r#"
machines:
  Ping:
    states: [a, b]
    transitions:
      - { from: a, to: b, on: { event: Pong }, emits: [Ping] }
  Pong:
    states: [a, b]
    transitions:
      - { from: a, to: b, on: hit, emits: [Pong] }
controllers:
  Relay:
    on:
      Ping: [{ fire: Pong.hit }]
"#;
    assert_eq!(
        cycle_keys(text),
        [vec![
            "transition:Ping:a->b@Pong rule:Relay/Ping#0".to_owned(),
            "transition:Pong:a->b@hit subscription".to_owned()
        ]]
    );
}

#[test]
fn timers_do_not_create_cycles() {
    // a → b re-arms the timer, which returns to a: a loop in time, not a
    // cascade.
    let text = r#"
machines:
  Poller:
    states: [a, b]
    transitions:
      - { from: a, to: b, after: 10s }
      - { from: b, to: a, after: 10s }
"#;
    assert!(cycle_keys(text).is_empty());
    // A subscription into a timer's source does not close a cycle either.
    let text = r#"
machines:
  Poller:
    states: [idle, waiting]
    transitions:
      - { from: idle, to: waiting, on: { event: Polled } }
      - { from: waiting, to: idle, after: 10s, emits: [Polled] }
"#;
    assert!(cycle_keys(text).is_empty());
}

// --- State-dependent fire ------------------------------------------------------

#[test]
fn subscriptions_are_never_state_dependent_fires() {
    let (_, findings) = run(SUB_AND_FIRE);
    // Only Billing's rule gets the note (Checkout.closed drops close).
    let notes = of(&findings, Check::StateDependentFire);
    assert_eq!(notes.len(), 1, "{notes:#?}");
    assert!(matches!(notes[0].detail, FindingDetail::StateDependentFire { .. }));
}
