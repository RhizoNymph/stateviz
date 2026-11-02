//! Depth-first search over search states with a visited set and sleep sets.
//!
//! ```text
//! frame(state, sleep Z) ──▶ candidates: enabled bases × decision variants, performed
//!   each candidate t not asleep ──▶ child = t(state), Z' = {u ∈ Z | u independent of t}
//!        child key seen?  no ──▶ new node (terminal: outcome) ──▶ frame(child, Z')
//!                         yes ─▶ revisit: explore only what slept there before
//!                                 but is awake now (Godefroid's sleep sets
//!                                 with state caching)
//!   Z += t
//! ```
//!
//! Sleep sets only skip *transitions* whose result another order reaches
//! too; with the revisit rule every reachable state is still visited, so
//! no outcome is lost. Stored sleep sets use move descriptions (content),
//! since equivalent states reached along different lines have different
//! pending ids.

use std::collections::{BTreeMap, HashMap};

use cascade_core::Model;

use super::graph::{Graph, NodeIx};
use super::inputs::Inputs;
use super::key::{item_text, search_key};
use super::moves::{Base, Footprint, LineId, MoveDesc, SearchState, Step, enabled, independent, perform};
use super::options::ExploreOptions;
use super::outcome::{Anomaly, OutcomeTimer};
use super::progress::ExploreProgress;
use crate::error::SimError;

/// A terminal state's identity and the nodes that end in it.
#[derive(Clone, Debug)]
pub(crate) struct Found {
    pub instances: BTreeMap<String, (cascade_core::ids::StateId, BTreeMap<String, String>)>,
    pub timers: Vec<OutcomeTimer>,
    pub anomalies: std::collections::BTreeSet<Anomaly>,
    pub nodes: Vec<NodeIx>,
}

/// What the search leaves for building the result.
pub(crate) struct Searched {
    pub graph: Graph,
    pub descs: Vec<MoveDesc>,
    pub found: Vec<Found>,
    pub complete: bool,
}

/// A move ready to explore from a frame's state.
struct Candidate {
    line: LineId,
    desc: MoveDesc,
    footprint: Footprint,
    step: Option<Step>,
}

struct Frame {
    node: NodeIx,
    state: SearchState,
    candidates: Vec<Candidate>,
    /// Indices into `candidates` to explore, in order.
    todo: Vec<usize>,
    next: usize,
    /// The sleep set: moves whose interleavings from here are covered
    /// elsewhere (line id and footprint).
    sleep: Vec<(LineId, Footprint)>,
}

struct Searcher<'a> {
    model: &'a Model,
    inputs: &'a Inputs,
    options: &'a ExploreOptions,
    graph: Graph,
    visited: HashMap<String, NodeIx>,
    /// The sleep set stored at each node, as sorted descriptions.
    stored_sleep: Vec<Vec<MoveDesc>>,
    descs: Vec<MoveDesc>,
    desc_ix: HashMap<MoveDesc, u32>,
    found: Vec<Found>,
    found_ix: HashMap<String, usize>,
}

pub(crate) fn search(
    model: &Model,
    root: SearchState,
    inputs: &Inputs,
    options: &ExploreOptions,
    progress: &ExploreProgress,
) -> Result<Searched, SimError> {
    let mut s = Searcher {
        model,
        inputs,
        options,
        graph: Graph::default(),
        visited: HashMap::new(),
        stored_sleep: Vec::new(),
        descs: Vec::new(),
        desc_ix: HashMap::new(),
        found: Vec::new(),
        found_ix: HashMap::new(),
    };
    let mut complete = true;
    let root_key = s.key(&root);
    let root_node = s.new_node(root_key);
    let mut stack: Vec<Frame> = Vec::new();
    if s.is_terminal(&root) {
        s.record_outcome(root_node, &root);
    } else if let Some(frame) = s.frame(root_node, root, Vec::new(), false)? {
        stack.push(frame);
    }

    while let Some(top) = stack.last_mut() {
        progress.set_states(s.graph.nodes.len());
        if progress.is_cancelled() {
            tracing::info!(states = s.graph.nodes.len(), "exploration cancelled");
            return Err(SimError::ExploreCancelled { states: s.graph.nodes.len() });
        }
        let Some(&ci) = top.todo.get(top.next) else {
            stack.pop();
            continue;
        };
        top.next += 1;
        let parent = top.node;
        let candidate = &mut top.candidates[ci];
        let performed = match candidate.step.take() {
            Some(Step::Done(performed)) => performed,
            Some(Step::Halt) => {
                let mut halted = top.state.clone();
                halted.anomalies.insert(Anomaly::StepLimit);
                let key = format!("{} halt", s.key(&halted));
                let node = match s.visited.get(&key) {
                    Some(&node) => node,
                    None => {
                        if s.graph.nodes.len() >= s.options.max_states {
                            complete = false;
                            break;
                        }
                        let node = s.new_node(key);
                        s.record_outcome(node, &halted);
                        node
                    }
                };
                let halt = s.intern(MoveDesc::Halt);
                s.graph.node_mut(parent).edges.push((node, halt));
                continue;
            }
            None => continue,
        };
        let desc = s.intern(candidate.desc.clone());
        let footprint = candidate.footprint.clone();
        let line = candidate.line.clone();
        let reduction = s.options.reduction;
        let child_sleep: Vec<LineId> = if reduction {
            top.sleep
                .iter()
                .filter(|(_, u)| independent(s.options.ordering, &footprint, u))
                .map(|(l, _)| l.clone())
                .collect()
        } else {
            Vec::new()
        };
        if reduction {
            top.sleep.push((line, footprint));
        }

        let child = performed.state;
        let key = s.key(&child);
        match s.visited.get(&key).copied() {
            Some(node) => {
                s.graph.node_mut(parent).edges.push((node, desc));
                if reduction && let Some(frame) = s.frame(node, child, child_sleep, true)? {
                    stack.push(frame);
                }
            }
            None => {
                if s.graph.nodes.len() >= s.options.max_states {
                    complete = false;
                    tracing::info!(limit = s.options.max_states, "exploration stopped at the state limit");
                    break;
                }
                let node = s.new_node(key);
                s.graph.node_mut(parent).edges.push((node, desc));
                if s.is_terminal(&child) {
                    s.record_outcome(node, &child);
                } else if let Some(frame) = s.frame(node, child, child_sleep, false)? {
                    stack.push(frame);
                }
            }
        }
    }

    progress.set_states(s.graph.nodes.len());
    Ok(Searched { graph: s.graph, descs: s.descs, found: s.found, complete })
}

impl Searcher<'_> {
    fn key(&self, state: &SearchState) -> String {
        search_key(self.model, state, self.options.ordering, self.options.reduction)
    }

    fn new_node(&mut self, key: String) -> NodeIx {
        let node = self.graph.add();
        self.visited.insert(key, node);
        self.stored_sleep.push(Vec::new());
        node
    }

    fn intern(&mut self, desc: MoveDesc) -> u32 {
        if let Some(&ix) = self.desc_ix.get(&desc) {
            return ix;
        }
        let ix = u32::try_from(self.descs.len()).unwrap_or(u32::MAX);
        self.descs.push(desc.clone());
        self.desc_ix.insert(desc, ix);
        ix
    }

    /// Inputs used up and the queue empty (and, with `timers_anytime`, no
    /// timer armed).
    fn is_terminal(&self, state: &SearchState) -> bool {
        state.input >= self.inputs.inputs.len()
            && state.core.queue().is_empty()
            && (!self.options.timers_anytime || state.core.armed().is_empty())
    }

    fn record_outcome(&mut self, node: NodeIx, state: &SearchState) {
        let core = &state.core;
        let instances: BTreeMap<_, _> =
            core.instances().iter().map(|i| (i.name.clone(), (i.leaf(), i.fields.clone()))).collect();
        let mut timers: Vec<OutcomeTimer> = core
            .armed()
            .iter()
            .map(|a| OutcomeTimer {
                instance: core.instance(a.instance).name.clone(),
                trigger: a.trigger,
                remaining: a.deadline.saturating_sub(core.now()),
            })
            .collect();
        timers.sort();
        let mut anomalies = state.anomalies.clone();
        for option in core.decisions() {
            anomalies.insert(Anomaly::UnusedChoice { option: option.clone() });
        }
        let identity = format!("{instances:?} {timers:?} {anomalies:?}");
        let ix = match self.found_ix.get(&identity) {
            Some(&ix) => ix,
            None => {
                self.found.push(Found { instances, timers, anomalies, nodes: Vec::new() });
                self.found_ix.insert(identity, self.found.len() - 1);
                self.found.len() - 1
            }
        };
        self.found[ix].nodes.push(node);
        self.graph.node_mut(node).outcome = Some(ix);
    }

    /// The description of a move by line id in `state`.
    fn desc_of(&self, state: &SearchState, line: &LineId) -> Option<MoveDesc> {
        Some(match line {
            LineId::Deliver { id, consumer, decisions } => {
                let queued = state.core.queue().iter().find(|q| q.id == *id)?;
                MoveDesc::Deliver {
                    item: item_text(self.model, &state.core, queued),
                    consumer: consumer.clone(),
                    decisions: decisions.clone(),
                }
            }
            LineId::Input { index, decisions } => MoveDesc::Input { index: *index, decisions: decisions.clone() },
            LineId::Advance => MoveDesc::Advance,
        })
    }

    /// A frame for `node` entered with sleep set `sleep` (line ids valid in
    /// `state`). `revisit` when the node was seen before: then only moves
    /// that slept at every earlier visit but are awake now are explored,
    /// and `None` means nothing new to do.
    fn frame(
        &mut self,
        node: NodeIx,
        state: SearchState,
        sleep: Vec<LineId>,
        revisit: bool,
    ) -> Result<Option<Frame>, SimError> {
        let mut sleep_descs: Vec<MoveDesc> = sleep.iter().filter_map(|l| self.desc_of(&state, l)).collect();
        sleep_descs.sort();
        sleep_descs.dedup();
        let wake: Option<Vec<MoveDesc>> = match revisit {
            false => {
                self.stored_sleep[node as usize] = sleep_descs;
                None
            }
            true => {
                let stored = &self.stored_sleep[node as usize];
                let awake: Vec<MoveDesc> = stored.iter().filter(|d| !sleep_descs.contains(d)).cloned().collect();
                if awake.is_empty() {
                    return Ok(None);
                }
                let kept: Vec<MoveDesc> = stored.iter().filter(|d| sleep_descs.contains(d)).cloned().collect();
                self.stored_sleep[node as usize] = kept;
                Some(awake)
            }
        };

        let candidates = self.candidates(&state)?;
        let asleep: Vec<(LineId, Footprint)> = candidates
            .iter()
            .filter(|c| sleep.contains(&c.line))
            .map(|c| (c.line.clone(), c.footprint.clone()))
            .collect();
        let todo: Vec<usize> = candidates
            .iter()
            .enumerate()
            .filter(|(_, c)| match &wake {
                None => !sleep.contains(&c.line),
                Some(awake) => awake.contains(&c.desc),
            })
            .map(|(i, _)| i)
            .collect();
        Ok(Some(Frame { node, state, candidates, todo, next: 0, sleep: asleep }))
    }

    /// Every enabled move in `state`, performed: each base once per
    /// combination of decisions (with `choices`).
    fn candidates(&self, state: &SearchState) -> Result<Vec<Candidate>, SimError> {
        let mut out = Vec::new();
        for enabled in enabled(self.model, state, self.inputs, self.options) {
            let mut variants = Vec::new();
            self.variants(state, &enabled.base, Vec::new(), &mut variants)?;
            for (decisions, step) in variants {
                let (line, desc) = match &enabled.base {
                    Base::Deliver { id, consumer, .. } => (
                        LineId::Deliver { id: *id, consumer: consumer.clone(), decisions: decisions.clone() },
                        MoveDesc::Deliver {
                            item: enabled.item.clone().unwrap_or_default(),
                            consumer: consumer.clone(),
                            decisions,
                        },
                    ),
                    Base::Input(index) => (
                        LineId::Input { index: *index, decisions: decisions.clone() },
                        MoveDesc::Input { index: *index, decisions },
                    ),
                    Base::Advance => (LineId::Advance, MoveDesc::Advance),
                };
                out.push(Candidate { line, desc, footprint: enabled.footprint.clone(), step: Some(step) });
            }
        }
        Ok(out)
    }

    /// Perform `base` with `decisions`; with `choices`, branch at the first
    /// choice point that took its default, once per option.
    fn variants(
        &self,
        state: &SearchState,
        base: &Base,
        decisions: Vec<String>,
        out: &mut Vec<(Vec<String>, Step)>,
    ) -> Result<(), SimError> {
        let step = perform(self.model, state, self.inputs, base, &decisions)?;
        match &step {
            Step::Done(performed) if self.options.choices && performed.defaulted.is_some() => {
                for option in performed.defaulted.clone().unwrap_or_default() {
                    let mut more = decisions.clone();
                    more.push(option);
                    self.variants(state, base, more, out)?;
                }
            }
            _ => out.push((decisions, step)),
        }
        Ok(())
    }
}
