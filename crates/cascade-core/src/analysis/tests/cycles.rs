use super::*;
use crate::analysis::CycleStep;

/// A ⇄ B: `A: a0 → a1` emits Pinged, CB fires B.pong; `B: b0 → b1` emits
/// Ponged, CA fires A.ping.
const PING_PONG: &str = r#"
machines:
  A:
    states: [a0, a1]
    transitions:
      - { from: a0, to: a1, on: ping, emits: [Pinged] }
  B:
    states: [b0, b1]
    transitions:
      - { from: b0, to: b1, on: pong, emits: [Ponged] }
controllers:
  CA:
    on:
      Ponged: [{ fire: A.ping }]
  CB:
    on:
      Pinged: [{ fire: B.pong }]
external:
  User: [A.ping]
"#;

/// Human-readable steps: `transition | rule`.
fn steps(model: &Model, finding: &Finding) -> Vec<String> {
    finding
        .detail
        .cycle_steps()
        .iter()
        .map(|s: &CycleStep| {
            format!("{} | {}", key(model, ElementRef::Transition(s.transition)), key(model, ElementRef::Rule(s.rule)))
        })
        .collect()
}

#[test]
fn two_machine_cycle_is_one_warning_in_causal_order() {
    let (model, findings) = run(PING_PONG);
    let f = single(&findings, Check::CascadeCycle);
    assert_eq!(f.severity, Severity::Warning);
    assert_eq!(
        steps(&model, f),
        ["transition:A:a0->a1@ping | rule:CB/Pinged#0", "transition:B:b0->b1@pong | rule:CA/Ponged#0"]
    );
    assert_eq!(key(&model, f.detail.primary()), "transition:A:a0->a1@ping");
    assert_eq!(
        keys(&model, &f.detail.subjects()),
        ["transition:A:a0->a1@ping", "rule:CB/Pinged#0", "transition:B:b0->b1@pong", "rule:CA/Ponged#0"]
    );
    assert_eq!(f.message, "A: a0 → a1 can re-trigger itself: A: a0 → a1 ⇒ CB ⇒ B: b0 → b1 ⇒ CA ⇒ A: a0 → a1");
}

#[test]
fn cycle_starts_at_the_first_transition_in_model_order() {
    // Same cycle, machines declared B first: B's transition leads.
    let text = PING_PONG.replacen("  A:\n    states: [a0, a1]\n    transitions:\n      - { from: a0, to: a1, on: ping, emits: [Pinged] }\n", "", 1)
        .replacen(
            "      - { from: b0, to: b1, on: pong, emits: [Ponged] }\n",
            "      - { from: b0, to: b1, on: pong, emits: [Ponged] }\n  A:\n    states: [a0, a1]\n    transitions:\n      - { from: a0, to: a1, on: ping, emits: [Pinged] }\n",
            1,
        );
    let (model, findings) = run(&text);
    let f = single(&findings, Check::CascadeCycle);
    assert_eq!(
        steps(&model, f),
        ["transition:B:b0->b1@pong | rule:CA/Ponged#0", "transition:A:a0->a1@ping | rule:CB/Pinged#0"]
    );
}

#[test]
fn self_loop_is_a_cycle() {
    let (model, findings) = run(r#"
machines:
  Job:
    states: [running]
    transitions:
      - { from: running, to: running, on: retry, emits: [Retried] }
controllers:
  Retrier:
    on:
      Retried: [{ fire: Job.retry }]
external:
  Clock: [Job.retry]
"#);
    let f = single(&findings, Check::CascadeCycle);
    assert_eq!(steps(&model, f), ["transition:Job:running->running@retry | rule:Retrier/Retried#0"]);
    assert_eq!(
        f.message,
        "Job: running → running can re-trigger itself: Job: running → running ⇒ Retrier ⇒ Job: running → running"
    );
}

#[test]
fn a_bounded_transition_silences_the_cycle() {
    let text = PING_PONG.replace("on: pong, emits: [Ponged] }", "on: pong, emits: [Ponged], bounded: true }");
    let (_, findings) = run(&text);
    assert!(of(&findings, Check::CascadeCycle).is_empty(), "{findings:#?}");
}

#[test]
fn a_bounded_rule_silences_the_cycle() {
    let text = PING_PONG.replace("Pinged: [{ fire: B.pong }]", "Pinged: [{ fire: B.pong, bounded: true }]");
    let (_, findings) = run(&text);
    assert!(of(&findings, Check::CascadeCycle).is_empty(), "{findings:#?}");
}

#[test]
fn a_bounded_rule_does_not_silence_a_parallel_unbounded_one() {
    let text =
        PING_PONG.replace("Pinged: [{ fire: B.pong }]", "Pinged: [{ fire: B.pong, bounded: true }, { fire: B.pong }]");
    let (model, findings) = run(&text);
    let f = single(&findings, Check::CascadeCycle);
    assert_eq!(
        steps(&model, f),
        ["transition:A:a0->a1@ping | rule:CB/Pinged#1", "transition:B:b0->b1@pong | rule:CA/Ponged#0"]
    );
}

#[test]
fn acyclic_chains_have_no_cycle() {
    let (_, findings) = run(examples::SPEC_EXAMPLE);
    assert!(of(&findings, Check::CascadeCycle).is_empty());
}

#[test]
fn one_finding_per_strongly_connected_component() {
    let (model, findings) = run(r#"
machines:
  M:
    states: [s0, s1, s2, s3]
    transitions:
      - { from: s0, to: s1, on: t0, emits: [E0] }
      - { from: s1, to: s0, on: t1, emits: [E1] }
      - { from: s2, to: s3, on: t2, emits: [E2] }
      - { from: s3, to: s2, on: t3, emits: [E3] }
controllers:
  C:
    on:
      E0: [{ fire: M.t1 }]
      E1: [{ fire: M.t0 }]
      E2: [{ fire: M.t3 }]
      E3: [{ fire: M.t2 }]
external:
  User: [M.t0, M.t2]
"#);
    let cycles = of(&findings, Check::CascadeCycle);
    assert_eq!(
        cycles.iter().map(|f| key(&model, f.detail.primary())).collect::<Vec<_>>(),
        ["transition:M:s0->s1@t0", "transition:M:s2->s3@t2"]
    );
}

#[test]
fn the_shortest_cycle_through_the_first_transition_is_chosen() {
    // X0 → X1 → X2 → X0 (length 3) and X0 → X3 → X0 (length 2).
    let (model, findings) = run(r#"
machines:
  M:
    states: [s0, s1, s2, s3]
    transitions:
      - { from: s0, to: s1, on: t0, emits: [E0] }
      - { from: s1, to: s2, on: t1, emits: [E1] }
      - { from: s2, to: s0, on: t2, emits: [E2] }
      - { from: s1, to: s3, on: t3, emits: [E3] }
controllers:
  C:
    on:
      E0: [{ fire: M.t1 }, { fire: M.t3 }]
      E1: [{ fire: M.t2 }]
      E2: [{ fire: M.t0 }]
      E3: [{ fire: M.t0 }]
external:
  User: [M.t0]
"#);
    let f = single(&findings, Check::CascadeCycle);
    assert_eq!(steps(&model, f), ["transition:M:s0->s1@t0 | rule:C/E0#1", "transition:M:s1->s3@t3 | rule:C/E3#0"]);
}

#[test]
fn long_cycles_are_found_without_recursion() {
    let n = 400;
    let mut text = String::from("machines:\n  Ring:\n    states: [only]\n    transitions:\n");
    for i in 0..n {
        text.push_str(&format!("      - {{ from: only, to: only, on: t{i}, emits: [E{i}] }}\n"));
    }
    text.push_str("controllers:\n  Relay:\n    on:\n");
    for i in 0..n {
        text.push_str(&format!("      E{i}: [{{ fire: Ring.t{} }}]\n", (i + 1) % n));
    }
    text.push_str("external:\n  User: [Ring.t0]\n");
    let (model, findings) = run(&text);
    let f = single(&findings, Check::CascadeCycle);
    let cycle = f.detail.cycle_steps();
    assert_eq!(cycle.len(), n);
    assert_eq!(key(&model, ElementRef::Transition(cycle[0].transition)), "transition:Ring:only->only@t0");
    assert_eq!(
        key(&model, ElementRef::Transition(cycle[n - 1].transition)),
        format!("transition:Ring:only->only@t{}", n - 1)
    );
}

#[test]
fn fires_into_every_accepting_transition_count() {
    // `retry` is accepted from two states; only one of them loops.
    let (model, findings) = run(r#"
machines:
  M:
    states: [idle, busy, done]
    transitions:
      - { from: idle, to: busy, on: retry, emits: [Again] }
      - { from: busy, to: done, on: retry }
controllers:
  C:
    on:
      Again: [{ fire: M.retry }]
external:
  User: [M.retry]
"#);
    let f = single(&findings, Check::CascadeCycle);
    assert_eq!(steps(&model, f), ["transition:M:idle->busy@retry | rule:C/Again#0"]);
}
