//! Mapping any selected element onto causal-graph nodes.
//!
//! Only external sources, transitions, events and handlers are causal
//! nodes. Everything else a reader can select stands for a set of them, so
//! cone tracing and path queries work from any selection.

use cascade_core::{CausalGraph, ElementRef, Model, NodeIx};

/// The causal nodes that stand for `element`, in graph order without
/// duplicates:
///
/// | Element | Causal nodes |
/// | --- | --- |
/// | external, transition, event, handler | itself |
/// | machine | all its transitions |
/// | state | transitions leaving or entering it or any state nested in it |
/// | controller | all its handlers |
/// | rule | its handler |
/// | trigger | transitions accepting it; if none, the handlers of rules firing it and the sources that can fire it |
pub fn causal_seeds(model: &Model, graph: &CausalGraph, element: ElementRef) -> Vec<NodeIx> {
    let mut seeds: Vec<NodeIx> = match element {
        ElementRef::External(_) | ElementRef::Transition(_) | ElementRef::Event(_) | ElementRef::Handler(_) => {
            graph.ix_of_element(element).into_iter().collect()
        }
        ElementRef::Machine(m) => model
            .machine(m)
            .transitions
            .iter()
            .filter_map(|&t| graph.ix_of_element(ElementRef::Transition(t)))
            .collect(),
        ElementRef::State(s) => {
            let machine = model.state(s).machine;
            model
                .machine(machine)
                .transitions
                .iter()
                .copied()
                .filter(|&t| {
                    let tr = model.transition(t);
                    model.is_ancestor_or_self(s, tr.from) || model.is_ancestor_or_self(s, tr.to)
                })
                .filter_map(|t| graph.ix_of_element(ElementRef::Transition(t)))
                .collect()
        }
        ElementRef::Controller(c) => {
            model.controller(c).handlers.iter().filter_map(|&h| graph.ix_of_element(ElementRef::Handler(h))).collect()
        }
        ElementRef::Rule(r) => graph.ix_of_element(ElementRef::Handler(model.rule(r).handler)).into_iter().collect(),
        ElementRef::Trigger(t) => {
            let trigger = model.trigger(t);
            if trigger.accepted_by.is_empty() {
                trigger
                    .fired_by
                    .iter()
                    .map(|&r| ElementRef::Handler(model.rule(r).handler))
                    .chain(trigger.sources.iter().map(|&x| ElementRef::External(x)))
                    .filter_map(|e| graph.ix_of_element(e))
                    .collect()
            } else {
                trigger.accepted_by.iter().filter_map(|&tr| graph.ix_of_element(ElementRef::Transition(tr))).collect()
            }
        }
    };
    seeds.sort_unstable();
    seeds.dedup();
    seeds
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::emphasis::tests::{CHAIN, load, node_labels};

    fn seeds_of(text: &str, key: &str) -> Vec<String> {
        let model = load(text);
        let graph = CausalGraph::build(&model);
        let element = model.resolve_key(&key.parse().expect("key")).expect("element");
        node_labels(&model, &graph, causal_seeds(&model, &graph, element))
    }

    #[test]
    fn causal_elements_map_to_themselves() {
        assert_eq!(seeds_of(CHAIN, "event:Go"), ["Go"]);
        assert_eq!(seeds_of(CHAIN, "external:User"), ["User"]);
        assert_eq!(seeds_of(CHAIN, "handler:C2/Done"), ["C2 on Done"]);
        assert_eq!(seeds_of(CHAIN, "transition:A:a0->a1@go"), ["A: a0 → a1"]);
    }

    #[test]
    fn machines_and_states_map_to_transitions() {
        assert_eq!(seeds_of(CHAIN, "machine:B"), ["B: b0 → b1", "B: b1 → b0"]);
        assert_eq!(seeds_of(CHAIN, "state:D:d1"), ["D: d0 → d1"]);
        let nested = r#"
machines:
  J:
    states:
      - idle
      - running: { states: [fetching, parsing] }
      - done: { kind: final }
    transitions:
      - { from: idle, to: running, on: go }
      - { from: running.fetching, to: running.parsing, on: fetched }
      - { from: running, to: done, on: finish }
"#;
        assert_eq!(
            seeds_of(nested, "state:J:running"),
            ["J: idle → running", "J: running → done", "J: running.fetching → running.parsing"]
        );
        assert_eq!(seeds_of(nested, "state:J:running.parsing"), ["J: running.fetching → running.parsing"]);
    }

    #[test]
    fn controllers_rules_and_triggers_map_to_handlers_and_transitions() {
        assert_eq!(seeds_of(CHAIN, "controller:C2"), ["C2 on Done"]);
        assert_eq!(seeds_of(CHAIN, "rule:C2/Done#1"), ["C2 on Done"]);
        assert_eq!(seeds_of(CHAIN, "trigger:B.start"), ["B: b0 → b1"]);
        let invalid = r#"
machines:
  A:
    states: [a0, a1]
    transitions: [{ from: a0, to: a1, on: go, emits: [Go] }]
controllers:
  C:
    on:
      Go: [{ fire: A.missing }]
external:
  User: [A.go, A.missing]
"#;
        assert_eq!(seeds_of(invalid, "trigger:A.missing"), ["C on Go", "User"]);
    }
}
