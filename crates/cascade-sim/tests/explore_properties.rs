//! Exploration properties over the examples, fixtures and small random
//! models: every witness replays to exactly its outcome, and partial-order
//! reduction never loses an outcome.

mod explore_support;

use cascade_sim::{ExploreOptions, PlaySession};
use explore_support::*;

#[test]
fn every_witness_replays_to_its_outcome() {
    for (definition, scenario_text) in corpus() {
        let model = model(definition);
        for options in option_grid() {
            for reduction in [true, false] {
                let options = ExploreOptions { reduction, max_states: 20_000, ..options.clone() };
                let exploration = run(&model, scenario_text, &options);
                let context = format!("{} {options:?}", scenario(scenario_text).name);
                check_distinct(&exploration, &context);
                check_witnesses(&model, &PlaySession::new(&model), &exploration, &context);
            }
        }
    }
}

#[test]
fn reduction_never_loses_an_outcome() {
    for (definition, scenario_text) in corpus() {
        let model = model(definition);
        for options in option_grid() {
            let reduced = run(&model, scenario_text, &ExploreOptions { max_states: 20_000, ..options.clone() });
            let full =
                run(&model, scenario_text, &ExploreOptions { reduction: false, max_states: 20_000, ..options.clone() });
            let context = format!("{} {options:?}", scenario(scenario_text).name);
            if reduced.complete && full.complete {
                assert_eq!(identities(&reduced), identities(&full), "{context}");
                assert!(reduced.states <= full.states, "{context}");
            }
        }
    }
}

/// A tiny deterministic generator (64-bit LCG).
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        self.0 >> 33
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }

    fn chance(&mut self, percent: u64) -> bool {
        self.below(100) < percent
    }
}

/// A small random system: 2–3 machines with 3 states and triggers t0/t1
/// (and sometimes a ?choice guard, a subscription or a timer), 1–3
/// controllers routing events E0–E2 to one or all instances of a machine,
/// one external source; 1–2 instances per machine and 1–3 fires.
fn random_system(rng: &mut Rng) -> (String, String) {
    let machines = 2 + rng.below(2) as usize;
    let mut def = String::from("machines:\n");
    for m in 0..machines {
        def.push_str(&format!("  M{m}:\n    initial: s0\n    states: [s0, s1, s2]\n    transitions:\n"));
        for t in 0..2 {
            let count = 1 + rng.below(2);
            for _ in 0..count {
                let from = rng.below(3);
                let to = rng.below(3);
                let emits = if rng.chance(60) { format!(", emits: [E{}]", rng.below(3)) } else { String::new() };
                def.push_str(&format!("      - {{ from: s{from}, to: s{to}, on: t{t}{emits} }}\n"));
            }
        }
        if rng.chance(25) {
            let from = rng.below(3);
            def.push_str(&format!(
                "      - {{ from: s{from}, to: s{}, on: t0, guard: \"?left\" }}\n      - {{ from: s{from}, to: s{}, on: t0, guard: \"?right\" }}\n",
                rng.below(3),
                rng.below(3)
            ));
        }
        if rng.chance(25) {
            def.push_str(&format!(
                "      - {{ from: s{}, to: s{}, on: {{ event: E{} }} }}\n",
                rng.below(3),
                rng.below(3),
                rng.below(3)
            ));
        }
        if rng.chance(20) {
            def.push_str(&format!("      - {{ from: s{}, to: s{}, after: 1m }}\n", rng.below(3), rng.below(3)));
        }
    }
    def.push_str("controllers:\n");
    let controllers = 1 + rng.below(3);
    for c in 0..controllers {
        def.push_str(&format!("  C{c}:\n    on:\n"));
        let events = 1 + rng.below(2);
        let mut used = Vec::new();
        for _ in 0..events {
            let e = rng.below(3);
            if used.contains(&e) {
                continue;
            }
            used.push(e);
            let m = rng.below(machines as u64);
            let all = if rng.chance(40) { "all " } else { "" };
            def.push_str(&format!("      E{e}:\n        - {{ fire: M{m}.t{}, target: {all}M{m} }}\n", rng.below(2)));
        }
    }
    def.push_str("external:\n  Src: [");
    let triggers: Vec<String> = (0..machines).flat_map(|m| (0..2).map(move |t| format!("M{m}.t{t}"))).collect();
    def.push_str(&triggers.join(", "));
    def.push_str("]\n");

    let mut scenario = String::from("scenario: random\ninstances:\n");
    let mut names = Vec::new();
    for m in 0..machines {
        for i in 0..(1 + rng.below(2)) {
            let name = format!("m{m}x{i}");
            scenario.push_str(&format!("  {name}: M{m}\n"));
            names.push((m, name));
        }
    }
    scenario.push_str("steps:\n");
    for _ in 0..(1 + rng.below(3)) {
        let (m, name) = &names[rng.below(names.len() as u64) as usize];
        let timing = if rng.chance(40) { ", timing: immediate" } else { "" };
        scenario.push_str(&format!("  - {{ source: Src, fire: M{m}.t{}, target: {name}{timing} }}\n", rng.below(2)));
    }
    if rng.chance(20) {
        scenario.push_str("  - wait\n");
    }
    (def, scenario)
}

#[test]
fn random_models_keep_both_properties() {
    let mut rng = Rng(0x5eed_cafe);
    let mut checked = 0;
    for _ in 0..150 {
        let (def, scenario_text) = random_system(&mut rng);
        let Ok(model) = cascade_core::load_str(&def) else {
            continue;
        };
        let Ok(parsed) = cascade_sim::parse_scenario(&scenario_text) else {
            continue;
        };
        if cascade_sim::validate(&model, &parsed).is_err() {
            continue;
        }
        for options in option_grid() {
            let base = ExploreOptions { max_states: 400, ..options };
            let reduced = run(&model, &scenario_text, &base);
            let full = run(&model, &scenario_text, &ExploreOptions { reduction: false, ..base.clone() });
            let context = format!("{def}\n{scenario_text}\n{base:?}");
            check_distinct(&reduced, &context);
            check_witnesses(&model, &PlaySession::new(&model), &reduced, &context);
            check_witnesses(&model, &PlaySession::new(&model), &full, &context);
            if reduced.complete && full.complete {
                assert_eq!(identities(&reduced), identities(&full), "{context}");
                checked += 1;
            }
        }
    }
    eprintln!("compared {checked} random explorations");
    assert!(checked > 200, "only {checked} random explorations were compared");
}
