//! The spec's scale target (about 20 machines, 200 states, 50 controllers)
//! must analyze quickly, even in an unoptimized build.

use std::fmt::Write as _;
use std::time::{Duration, Instant};

use super::*;

const MACHINES: usize = 20;
const CONTROLLERS: usize = 50;
const HANDLERS_PER_CONTROLLER: usize = 3;
const RULES_PER_HANDLER: usize = 2;

/// Deterministic pseudo-random numbers (PCG-style LCG); no dependency.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self, bound: usize) -> usize {
        self.0 = self.0.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        let bound = u64::try_from(bound).unwrap_or(u64::MAX).max(1);
        usize::try_from((self.0 >> 33) % bound).unwrap_or(0)
    }
}

/// Per machine: `s0 … s6` in a chain with a back edge, then a compound
/// `busy { x, y }` (10 states), guarded forks, a transition on the compound
/// state and a bounded self-loop. Every transition emits its own event;
/// controllers subscribe to random events and fire random triggers with
/// every kind of target.
fn generated_model() -> String {
    let mut rng = Lcg(0x5eed);
    let mut text = String::from("machines:\n");
    let mut triggers: Vec<(usize, String)> = Vec::new();
    let mut events: Vec<String> = Vec::new();
    for m in 0..MACHINES {
        let _ = write!(
            text,
            "  M{m}:\n    fields: [id]\n    states:\n      - s0\n      - s1\n      - s2\n      - s3\n      - s4\n      - s5\n      - s6\n      - busy:\n          states: [x, y]\n    transitions:\n"
        );
        let mut transition = |text: &mut String, from: &str, to: &str, extra: &str| {
            let t = triggers.iter().filter(|(owner, _)| *owner == m).count();
            let name = format!("t{t}");
            let event = format!("E{m}_{t}");
            let _ = writeln!(text, "      - {{ from: {from}, to: {to}, on: {name}, emits: [{event}]{extra} }}");
            triggers.push((m, name));
            events.push(event);
        };
        for i in 0..6 {
            transition(&mut text, &format!("s{i}"), &format!("s{}", i + 1), "");
        }
        transition(&mut text, "s6", "busy", "");
        transition(&mut text, "busy.x", "busy.y", "");
        transition(&mut text, "busy", "s0", "");
        transition(&mut text, "s3", "s1", "");
        transition(&mut text, "s2", "s4", ", guard: \"n > 0\"");
        transition(&mut text, "s2", "s5", ", guard: else");
        transition(&mut text, "s5", "s5", ", bounded: true");
    }

    text.push_str("controllers:\n");
    for c in 0..CONTROLLERS {
        let _ = writeln!(text, "  C{c}:\n    on:");
        let mut used = Vec::new();
        while used.len() < HANDLERS_PER_CONTROLLER {
            let e = rng.next(events.len());
            if used.contains(&e) {
                continue;
            }
            used.push(e);
            let _ = writeln!(text, "      {}:", events[e]);
            for _ in 0..RULES_PER_HANDLER {
                let (machine, trigger) = &triggers[rng.next(triggers.len())];
                let target = match rng.next(4) {
                    0 => format!("M{machine}"),
                    1 => format!("M{machine} where id == event.id"),
                    2 => format!("all M{machine} where id == event.id"),
                    _ => format!("new M{machine} with id = event.id"),
                };
                let bounded = if rng.next(5) == 0 { ", bounded: true" } else { "" };
                let _ = writeln!(text, "        - {{ fire: M{machine}.{trigger}, target: {target}{bounded} }}");
            }
        }
    }

    text.push_str("external:\n");
    for x in 0..5 {
        let picks: Vec<String> = (0..4)
            .map(|_| {
                let (machine, trigger) = &triggers[rng.next(triggers.len())];
                format!("M{machine}.{trigger}")
            })
            .collect();
        let _ = writeln!(text, "  X{x}: [{}]", picks.join(", "));
    }
    text
}

#[test]
fn spec_scale_model_analyzes_quickly() {
    let model = load(&generated_model());
    assert_eq!(model.machine_count(), MACHINES);
    assert!(model.state_count() >= 200, "{} states", model.state_count());
    assert_eq!(model.controller_count(), CONTROLLERS);

    let graph = CausalGraph::build(&model);
    let started = Instant::now();
    let findings = analyze(&model, &graph);
    let elapsed = started.elapsed();

    assert!(elapsed < Duration::from_millis(500), "analysis took {elapsed:?} for {} findings", findings.len());
    // The generated wiring is dense enough to exercise every graph check.
    for check in [Check::CascadeCycle, Check::RaceCandidate, Check::StateDependentFire] {
        assert!(findings.iter().any(|f| f.check() == check), "no {check} finding");
    }
    ordering::assert_sorted(&model, &findings);
}
