//! The causal graph, derived from a [`Model`] and never authored.
//!
//! Nodes are external sources, transitions, events and controller handlers
//! (one controller's subscription to one event). Edges follow causality:
//!
//! ```text
//! External ──Trigger──▶ Transition ──Emit──▶ Event ──Subscribe──▶ Handler ──Fire──▶ Transition
//! ```
//!
//! A `Trigger` or `Fire` edge goes to *every* transition that accepts the
//! trigger, since which one is taken depends on the target's current state.
//! Transition T1 causes T2 exactly when there is a path
//! `T1 → Event → Handler → T2`.

mod cone;

use std::collections::HashMap;

pub use cone::{Cone, Direction};

use crate::ids::{EventId, ExternalId, HandlerId, RuleId, TransitionId, TriggerId};
use crate::key::ElementRef;
use crate::model::Model;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CausalNode {
    External(ExternalId),
    Transition(TransitionId),
    Event(EventId),
    Handler(HandlerId),
}

impl CausalNode {
    pub const fn element(self) -> ElementRef {
        match self {
            CausalNode::External(id) => ElementRef::External(id),
            CausalNode::Transition(id) => ElementRef::Transition(id),
            CausalNode::Event(id) => ElementRef::Event(id),
            CausalNode::Handler(id) => ElementRef::Handler(id),
        }
    }

    /// The causal node for a model element, if that kind of element appears
    /// in the causal graph.
    pub const fn from_element(element: ElementRef) -> Option<Self> {
        match element {
            ElementRef::External(id) => Some(CausalNode::External(id)),
            ElementRef::Transition(id) => Some(CausalNode::Transition(id)),
            ElementRef::Event(id) => Some(CausalNode::Event(id)),
            ElementRef::Handler(id) => Some(CausalNode::Handler(id)),
            ElementRef::Machine(_)
            | ElementRef::State(_)
            | ElementRef::Trigger(_)
            | ElementRef::Controller(_)
            | ElementRef::Rule(_) => None,
        }
    }

    /// Whether moving onto this node counts as one causal hop: a new
    /// transition taken, or an external source acting.
    pub const fn is_hop(self) -> bool {
        matches!(self, CausalNode::Transition(_) | CausalNode::External(_))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NodeIx(u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EdgeIx(u32);

impl NodeIx {
    fn new(i: usize) -> Self {
        Self(u32::try_from(i).unwrap_or(u32::MAX))
    }

    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

impl EdgeIx {
    fn new(i: usize) -> Self {
        Self(u32::try_from(i).unwrap_or(u32::MAX))
    }

    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CausalEdgeKind {
    /// External source → a transition that accepts a trigger it can fire.
    Trigger { trigger: TriggerId },
    /// Transition → an event it emits.
    Emit,
    /// Event → a controller handler subscribed to it.
    Subscribe,
    /// Handler → a transition that accepts the trigger one of its rules fires.
    Fire { rule: RuleId },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CausalEdge {
    pub from: NodeIx,
    pub to: NodeIx,
    pub kind: CausalEdgeKind,
}

#[derive(Clone, Debug)]
pub struct CausalGraph {
    nodes: Vec<CausalNode>,
    edges: Vec<CausalEdge>,
    outgoing: Vec<Vec<EdgeIx>>,
    incoming: Vec<Vec<EdgeIx>>,
    index: HashMap<CausalNode, NodeIx>,
}

impl CausalGraph {
    /// Derive the causal graph. Node order is deterministic: external
    /// sources, transitions, events, then handlers, each in model order.
    pub fn build(model: &Model) -> Self {
        let mut graph = CausalGraph {
            nodes: Vec::new(),
            edges: Vec::new(),
            outgoing: Vec::new(),
            incoming: Vec::new(),
            index: HashMap::new(),
        };
        for id in model.external_ids() {
            graph.add_node(CausalNode::External(id));
        }
        for id in model.transition_ids() {
            graph.add_node(CausalNode::Transition(id));
        }
        for id in model.event_ids() {
            graph.add_node(CausalNode::Event(id));
        }
        for id in model.handler_ids() {
            graph.add_node(CausalNode::Handler(id));
        }

        for (xid, source) in model.externals() {
            for &trigger in &source.triggers {
                for &t in &model.trigger(trigger).accepted_by {
                    graph.add_edge(
                        CausalNode::External(xid),
                        CausalNode::Transition(t),
                        CausalEdgeKind::Trigger { trigger },
                    );
                }
            }
        }
        for (tid, transition) in model.transitions() {
            for &event in &transition.emits {
                graph.add_edge(CausalNode::Transition(tid), CausalNode::Event(event), CausalEdgeKind::Emit);
            }
        }
        for (hid, handler) in model.handlers() {
            graph.add_edge(CausalNode::Event(handler.event), CausalNode::Handler(hid), CausalEdgeKind::Subscribe);
            for &rule in &handler.rules {
                for &t in &model.trigger(model.rule(rule).trigger).accepted_by {
                    graph.add_edge(CausalNode::Handler(hid), CausalNode::Transition(t), CausalEdgeKind::Fire { rule });
                }
            }
        }
        graph
    }

    fn add_node(&mut self, node: CausalNode) -> NodeIx {
        let ix = NodeIx::new(self.nodes.len());
        self.nodes.push(node);
        self.outgoing.push(Vec::new());
        self.incoming.push(Vec::new());
        self.index.insert(node, ix);
        ix
    }

    fn add_edge(&mut self, from: CausalNode, to: CausalNode, kind: CausalEdgeKind) {
        // Both endpoints were added in `build` before any edge.
        let (Some(&from), Some(&to)) = (self.index.get(&from), self.index.get(&to)) else {
            return;
        };
        let ix = EdgeIx::new(self.edges.len());
        self.edges.push(CausalEdge { from, to, kind });
        self.outgoing[from.index()].push(ix);
        self.incoming[to.index()].push(ix);
    }

    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    pub fn edge_count(&self) -> usize {
        self.edges.len()
    }

    pub fn node(&self, ix: NodeIx) -> CausalNode {
        self.nodes[ix.index()]
    }

    pub fn edge(&self, ix: EdgeIx) -> CausalEdge {
        self.edges[ix.index()]
    }

    pub fn nodes(&self) -> impl ExactSizeIterator<Item = (NodeIx, CausalNode)> + '_ {
        self.nodes.iter().enumerate().map(|(i, &n)| (NodeIx::new(i), n))
    }

    pub fn edges(&self) -> impl ExactSizeIterator<Item = (EdgeIx, CausalEdge)> + '_ {
        self.edges.iter().enumerate().map(|(i, &e)| (EdgeIx::new(i), e))
    }

    pub fn outgoing(&self, ix: NodeIx) -> &[EdgeIx] {
        &self.outgoing[ix.index()]
    }

    pub fn incoming(&self, ix: NodeIx) -> &[EdgeIx] {
        &self.incoming[ix.index()]
    }

    pub fn ix_of(&self, node: CausalNode) -> Option<NodeIx> {
        self.index.get(&node).copied()
    }

    /// The node for a model element, if the element is in the causal graph.
    pub fn ix_of_element(&self, element: ElementRef) -> Option<NodeIx> {
        CausalNode::from_element(element).and_then(|n| self.ix_of(n))
    }

    /// Transitions directly caused by `transition`, each with the rule that
    /// fires it: `transition → event → handler → rule → successor`.
    pub fn transition_successors(&self, transition: TransitionId) -> Vec<(TransitionId, RuleId)> {
        let mut out = Vec::new();
        let Some(start) = self.ix_of(CausalNode::Transition(transition)) else {
            return out;
        };
        for &emit in self.outgoing(start) {
            let event = self.edge(emit).to;
            for &sub in self.outgoing(event) {
                let handler = self.edge(sub).to;
                for &fire in self.outgoing(handler) {
                    let edge = self.edge(fire);
                    if let (CausalEdgeKind::Fire { rule }, CausalNode::Transition(next)) =
                        (edge.kind, self.node(edge.to))
                    {
                        out.push((next, rule));
                    }
                }
            }
        }
        out
    }

    /// Causal depth of every node: the fewest edges from any external source,
    /// or `None` when no external source reaches the node. This is the layer
    /// order of the causal view.
    pub fn depths(&self) -> Vec<Option<u32>> {
        let mut depth = vec![None; self.nodes.len()];
        let mut queue = std::collections::VecDeque::new();
        for (ix, node) in self.nodes() {
            if matches!(node, CausalNode::External(_)) {
                depth[ix.index()] = Some(0);
                queue.push_back(ix);
            }
        }
        while let Some(ix) = queue.pop_front() {
            let next = depth[ix.index()].map_or(0, |d: u32| d + 1);
            for &e in self.outgoing(ix) {
                let to = self.edge(e).to;
                if depth[to.index()].is_none() {
                    depth[to.index()] = Some(next);
                    queue.push_back(to);
                }
            }
        }
        depth
    }
}
