//! The statechart IR shared by the importers.
//!
//! A [`Chart`] is one machine as the input format describes it: raw names,
//! targets already resolved to nodes, entry/exit actions and transition
//! actions reduced to the events they emit. Everything Cascade-specific
//! (valid names, state paths, trigger names, where emitted events go) is
//! decided later by [`super::lower`].

use cascade_core::PaletteColor;

/// A node of one chart. Index 0 is the root: the machine itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct NodeIx(pub usize);

impl NodeIx {
    pub const ROOT: NodeIx = NodeIx(0);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NodeKind {
    Normal,
    Final,
    History { deep: bool },
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Delay {
    Millis(u64),
    Named(String),
}

/// What takes a transition, before it gets a Cascade trigger name.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum TriggerSpec {
    /// A named event (`on: { FETCH: … }`, `<transition event="fetch">`).
    Event(String),
    /// A delayed transition (`after: { 1000: … }`).
    After(Delay),
    /// An invoked actor finished (`invoke.onDone`).
    ActorDone(String),
    /// An invoked actor failed (`invoke.onError`).
    ActorError(String),
    /// The source compound state reached a final child (`onDone`).
    StateDone,
}

/// Where an emitted event is delivered, which decides the routing rules a
/// synthesized controller gets.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Route {
    /// Back to the emitting machine (XState `raise`).
    Own,
    /// To the other machines (`sendParent`, `emit`).
    Others,
    /// To the machine with this raw name (`sendTo`, `<send target="#_id">`).
    Machine(String),
    /// To every machine, the emitter included (`emit:` actions, and SCXML
    /// `<raise>`/`<send>`, which every region of a parallel document sees).
    All,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Emit {
    /// Raw event name.
    pub event: String,
    pub route: Route,
}

#[derive(Clone, Debug)]
pub(crate) struct Edge {
    pub trigger: TriggerSpec,
    /// `None` for a targetless (internal) transition.
    pub target: Option<NodeIx>,
    pub guard: Option<String>,
    /// Events the transition's own actions emit.
    pub emits: Vec<Emit>,
    /// Whether a transition to the source or a descendant exits and
    /// re-enters the source (XState `reenter: true`, SCXML
    /// `type="external"`).
    pub reenter: bool,
    /// Cascade's `bounded: true`, from annotations.
    pub bounded: bool,
    pub location: String,
}

#[derive(Clone, Debug)]
pub(crate) struct Node {
    /// Raw local name (XState key, SCXML id without its parent's prefix).
    pub name: String,
    pub location: String,
    pub parent: Option<NodeIx>,
    pub kind: NodeKind,
    /// Explicit initial state: a child, or for the root any descendant.
    /// `None` means the first child.
    pub initial: Option<NodeIx>,
    pub children: Vec<NodeIx>,
    /// Events emitted by entry and exit actions.
    pub entry: Vec<Emit>,
    pub exit: Vec<Emit>,
    pub edges: Vec<Edge>,
}

impl Node {
    fn new(name: String, location: String, parent: Option<NodeIx>, kind: NodeKind) -> Self {
        Self {
            name,
            location,
            parent,
            kind,
            initial: None,
            children: Vec::new(),
            entry: Vec::new(),
            exit: Vec::new(),
            edges: Vec::new(),
        }
    }
}

/// Cascade-only machine attributes carried by annotated input.
#[derive(Clone, Debug, Default)]
pub(crate) struct MachineAttrs {
    pub color: Option<PaletteColor>,
    pub domain: Option<String>,
    pub fields: Vec<String>,
}

/// One machine.
#[derive(Clone, Debug)]
pub(crate) struct Chart {
    pub nodes: Vec<Node>,
    pub attrs: MachineAttrs,
}

impl Chart {
    /// A chart whose root is named `name` (the raw machine name).
    pub fn new(name: String, location: String) -> Self {
        Self { nodes: vec![Node::new(name, location, None, NodeKind::Normal)], attrs: MachineAttrs::default() }
    }

    pub fn name(&self) -> &str {
        &self.root().name
    }

    pub fn root(&self) -> &Node {
        &self.nodes[0]
    }

    pub fn node(&self, ix: NodeIx) -> &Node {
        &self.nodes[ix.0]
    }

    pub fn node_mut(&mut self, ix: NodeIx) -> &mut Node {
        &mut self.nodes[ix.0]
    }

    pub fn add_child(&mut self, parent: NodeIx, name: String, location: String, kind: NodeKind) -> NodeIx {
        let ix = NodeIx(self.nodes.len());
        self.nodes.push(Node::new(name, location, Some(parent), kind));
        self.nodes[parent.0].children.push(ix);
        ix
    }

    /// `node` and its ancestors, nearest first, ending at the root.
    pub fn ancestors_or_self(&self, node: NodeIx) -> Vec<NodeIx> {
        let mut out = vec![node];
        let mut current = node;
        while let Some(parent) = self.node(current).parent {
            out.push(parent);
            current = parent;
        }
        out
    }

    pub fn is_descendant_or_self(&self, node: NodeIx, ancestor: NodeIx) -> bool {
        self.ancestors_or_self(node).contains(&ancestor)
    }

    /// The child of `ancestor` on the way down to `node`, if `node` is a
    /// proper descendant of `ancestor`.
    pub fn child_toward(&self, ancestor: NodeIx, node: NodeIx) -> Option<NodeIx> {
        let chain = self.ancestors_or_self(node);
        let at = chain.iter().position(|&n| n == ancestor)?;
        at.checked_sub(1).map(|i| chain[i])
    }

    /// The child a compound node enters by default: its explicit initial
    /// (lifted to a direct child), else its first non-history child.
    pub fn default_child(&self, node: NodeIx) -> Option<NodeIx> {
        let n = self.node(node);
        if let Some(initial) = n.initial
            && let Some(child) = self.child_toward(node, initial)
        {
            return Some(child);
        }
        n.children.iter().copied().find(|&c| !matches!(self.node(c).kind, NodeKind::History { .. }))
    }

    /// The raw path of names from the root (exclusive) to `node`, for
    /// messages.
    pub fn display_path(&self, node: NodeIx) -> String {
        let mut names: Vec<&str> =
            self.ancestors_or_self(node).iter().rev().skip(1).map(|&n| self.node(n).name.as_str()).collect();
        if names.is_empty() {
            names.push(self.name());
        }
        names.join(".")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> (Chart, NodeIx, NodeIx, NodeIx) {
        let mut chart = Chart::new("M".into(), "m".into());
        let a = chart.add_child(NodeIx::ROOT, "a".into(), "m.a".into(), NodeKind::Normal);
        let h = chart.add_child(a, "h".into(), "m.a.h".into(), NodeKind::History { deep: false });
        let b = chart.add_child(a, "b".into(), "m.a.b".into(), NodeKind::Normal);
        (chart, a, h, b)
    }

    #[test]
    fn structure_queries() {
        let (chart, a, h, b) = sample();
        assert_eq!(chart.ancestors_or_self(b), vec![b, a, NodeIx::ROOT]);
        assert!(chart.is_descendant_or_self(b, NodeIx::ROOT));
        assert!(!chart.is_descendant_or_self(a, b));
        assert_eq!(chart.child_toward(NodeIx::ROOT, b), Some(a));
        assert_eq!(chart.child_toward(a, a), None);
        // The history child is skipped when picking a default.
        assert_eq!(chart.default_child(a), Some(b));
        assert_eq!(chart.default_child(h), None);
        assert_eq!(chart.display_path(b), "a.b");
        assert_eq!(chart.display_path(NodeIx::ROOT), "M");
    }
}
