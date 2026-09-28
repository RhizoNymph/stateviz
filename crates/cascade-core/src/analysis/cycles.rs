//! Cascade cycle (warning): a transition can eventually re-trigger itself.
//!
//! Works on the transition-level causal relation: `T1 → T2` (carrying the
//! rule) whenever [`CausalGraph::transition_successors`] says T1 causes T2.
//! `bounded: true` silences cycles through a step: bounded transitions are
//! removed as nodes and bounded rules' edges are removed, before strongly
//! connected components are computed. Each remaining non-trivial component
//! (more than one transition, or a self-loop) yields one finding: a shortest
//! cycle through the component's first transition in model order.

use std::collections::VecDeque;

use super::scc::strongly_connected;
use super::{CycleStep, Finding, FindingDetail};
use crate::causal::CausalGraph;
use crate::ids::{RuleId, TransitionId};
use crate::model::Model;

/// Unbounded causal edges between transitions, indexed by transition.
struct TransitionGraph {
    /// `edges[t]`: `(successor, rule)` sorted and deduplicated, so the cycle
    /// chosen among equally short ones does not depend on emission order.
    edges: Vec<Vec<(TransitionId, RuleId)>>,
}

impl TransitionGraph {
    fn build(model: &Model, graph: &CausalGraph) -> Self {
        let edges = model
            .transitions()
            .map(|(id, t)| {
                if t.bounded {
                    return Vec::new();
                }
                let mut out: Vec<(TransitionId, RuleId)> = graph
                    .transition_successors(id)
                    .into_iter()
                    .filter(|&(next, rule)| !model.rule(rule).bounded && !model.transition(next).bounded)
                    .collect();
                out.sort_unstable();
                out.dedup();
                out
            })
            .collect();
        Self { edges }
    }

    fn successors(&self, t: TransitionId) -> &[(TransitionId, RuleId)] {
        &self.edges[t.index()]
    }

    fn plain(&self) -> Vec<Vec<usize>> {
        self.edges.iter().map(|out| out.iter().map(|(t, _)| t.index()).collect()).collect()
    }
}

pub(super) fn check(model: &Model, graph: &CausalGraph) -> Vec<Finding> {
    let tg = TransitionGraph::build(model, graph);
    let components = strongly_connected(&tg.plain());

    let mut component_of = vec![0usize; tg.edges.len()];
    for (c, members) in components.iter().enumerate() {
        for &m in members {
            component_of[m] = c;
        }
    }

    let mut search = CycleSearch::new(tg.edges.len());
    let mut findings = Vec::new();
    for members in &components {
        // Model order is transition id order; indices come from
        // `TransitionGraph`, which has one slot per transition.
        let Some(start) = members.iter().copied().min().map(TransitionId::new) else {
            continue;
        };
        let nontrivial = members.len() > 1 || tg.successors(start).iter().any(|&(next, _)| next == start);
        if !nontrivial {
            continue;
        }
        if let Some(steps) = search.shortest_cycle(&tg, &component_of, start)
            && let Some((&first, rest)) = steps.split_first()
        {
            let message = message(model, &steps);
            findings.push(Finding::new(FindingDetail::CascadeCycle { first, rest: rest.to_vec() }, message));
        }
    }
    findings
}

/// Breadth-first search state reused across components, so the total work
/// stays linear in the size of the non-trivial components.
struct CycleSearch {
    /// `parent[v] = (u, rule)`: `v` was first reached from `u` through `rule`.
    parent: Vec<Option<(TransitionId, RuleId)>>,
    seen: Vec<bool>,
    touched: Vec<usize>,
}

impl CycleSearch {
    fn new(n: usize) -> Self {
        Self { parent: vec![None; n], seen: vec![false; n], touched: Vec::new() }
    }

    fn visit(&mut self, v: usize, parent: Option<(TransitionId, RuleId)>) {
        self.seen[v] = true;
        self.parent[v] = parent;
        self.touched.push(v);
    }

    fn reset(&mut self) {
        for v in self.touched.drain(..) {
            self.seen[v] = false;
            self.parent[v] = None;
        }
    }

    /// A shortest cycle through `start` inside its component, as steps in
    /// causal order beginning at `start`.
    fn shortest_cycle(
        &mut self,
        tg: &TransitionGraph,
        component_of: &[usize],
        start: TransitionId,
    ) -> Option<Vec<CycleStep>> {
        let component = component_of[start.index()];
        let mut queue = VecDeque::from([start]);
        self.visit(start.index(), None);
        let mut closing = None;
        'bfs: while let Some(u) = queue.pop_front() {
            for &(w, rule) in tg.successors(u) {
                if w == start {
                    closing = Some((u, rule));
                    break 'bfs;
                }
                if component_of[w.index()] != component || self.seen[w.index()] {
                    continue;
                }
                self.visit(w.index(), Some((u, rule)));
                queue.push_back(w);
            }
        }

        let steps = closing.map(|(last, rule)| {
            let mut steps = vec![CycleStep { transition: last, rule }];
            let mut current = last;
            while let Some((prev, rule)) = self.parent[current.index()] {
                steps.push(CycleStep { transition: prev, rule });
                current = prev;
            }
            steps.reverse();
            steps
        });
        self.reset();
        steps
    }
}

/// `Order: a → b can re-trigger itself: Order: a → b ⇒ Sync ⇒ Payment: c → d ⇒ Orders ⇒ Order: a → b`.
fn message(model: &Model, steps: &[CycleStep]) -> String {
    let Some(first) = steps.first() else {
        return String::new();
    };
    let first_label = model.transition_label(first.transition);
    let mut path = String::new();
    for step in steps {
        path.push_str(&model.transition_label(step.transition));
        path.push_str(" ⇒ ");
        path.push_str(&model.controller(model.rule(step.rule).controller).name);
        path.push_str(" ⇒ ");
    }
    path.push_str(&first_label);
    format!("{first_label} can re-trigger itself: {path}")
}
