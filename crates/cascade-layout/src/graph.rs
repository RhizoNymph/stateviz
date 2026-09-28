//! The layout input: a directed graph of sized boxes with optional ports and
//! groups.
//!
//! Ids are only handed out by [`LayoutGraph`] and edges are validated when
//! added, so a finished graph never references a missing node or port.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::geometry::{Insets, Size};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NodeId(u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EdgeId(u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GroupId(u32);

macro_rules! index_id {
    ($t:ty) => {
        impl $t {
            pub(crate) fn new(i: usize) -> Self {
                Self(u32::try_from(i).unwrap_or(u32::MAX))
            }

            pub const fn index(self) -> usize {
                self.0 as usize
            }
        }
    };
}
index_id!(NodeId);
index_id!(EdgeId);
index_id!(GroupId);

/// Which side of a node a port sits on. With left-to-right flow, input
/// ports go `West` and output ports `East`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PortSide {
    North,
    East,
    South,
    West,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Port {
    pub side: PortSide,
}

/// Where a node may be placed in the layer order.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum LayerConstraint {
    /// Let the engine choose.
    #[default]
    Free,
    /// The first layer (external sources in the causal view).
    First,
    /// Exactly this layer, counted from 0 (causal depth).
    Exact(u32),
}

#[derive(Clone, Debug, PartialEq)]
pub struct LayoutNode {
    /// Stable identity across relayouts: an `ElementKey` string or similar.
    /// Used to match previous positions and pins. Must be unique per graph.
    pub key: String,
    pub size: Size,
    /// The group (lane) this node belongs to, if any.
    pub group: Option<GroupId>,
    pub layer: LayerConstraint,
    /// Ports edges can attach to, addressed by index.
    pub ports: Vec<Port>,
}

impl LayoutNode {
    pub fn new(key: impl Into<String>, size: Size) -> Self {
        Self { key: key.into(), size, group: None, layer: LayerConstraint::Free, ports: Vec::new() }
    }

    pub fn in_group(mut self, group: GroupId) -> Self {
        self.group = Some(group);
        self
    }

    pub fn with_layer(mut self, layer: LayerConstraint) -> Self {
        self.layer = layer;
        self
    }

    pub fn with_ports(mut self, ports: Vec<Port>) -> Self {
        self.ports = ports;
        self
    }
}

/// One end of an edge: a node, optionally a specific port on it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct EdgeEnd {
    pub node: NodeId,
    pub port: Option<u16>,
}

impl EdgeEnd {
    pub const fn node(node: NodeId) -> Self {
        Self { node, port: None }
    }

    pub const fn port(node: NodeId, port: u16) -> Self {
        Self { node, port: Some(port) }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct LayoutEdge {
    pub source: EdgeEnd,
    pub target: EdgeEnd,
    /// Space to reserve for a label (e.g. a guard) along the edge.
    pub label: Option<Size>,
}

impl LayoutEdge {
    pub const fn new(source: EdgeEnd, target: EdgeEnd) -> Self {
        Self { source, target, label: None }
    }

    pub const fn with_label(mut self, size: Size) -> Self {
        self.label = Some(size);
        self
    }
}

/// A compound node: a lane that contains other nodes. Groups are laid out
/// independently and stacked top to bottom in the order they were added;
/// edges between groups are routed around them, never through.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LayoutGroup {
    pub key: String,
    pub padding: Insets,
    /// Height of the header band above the group's contents (for the lane
    /// label).
    pub header: f32,
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum GraphError {
    #[error("node `{key}` has no port {port}")]
    NoSuchPort { key: String, port: u16 },
    #[error("duplicate node key `{0}`")]
    DuplicateKey(String),
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct LayoutGraph {
    nodes: Vec<LayoutNode>,
    edges: Vec<LayoutEdge>,
    groups: Vec<LayoutGroup>,
    keys: HashMap<String, NodeId>,
}

impl LayoutGraph {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_group(&mut self, group: LayoutGroup) -> GroupId {
        self.groups.push(group);
        GroupId::new(self.groups.len() - 1)
    }

    pub fn add_node(&mut self, node: LayoutNode) -> Result<NodeId, GraphError> {
        if self.keys.contains_key(&node.key) {
            return Err(GraphError::DuplicateKey(node.key));
        }
        let id = NodeId::new(self.nodes.len());
        self.keys.insert(node.key.clone(), id);
        self.nodes.push(node);
        Ok(id)
    }

    pub fn add_edge(&mut self, edge: LayoutEdge) -> Result<EdgeId, GraphError> {
        for end in [edge.source, edge.target] {
            if let Some(port) = end.port {
                let node = &self.nodes[end.node.index()];
                if usize::from(port) >= node.ports.len() {
                    return Err(GraphError::NoSuchPort { key: node.key.clone(), port });
                }
            }
        }
        self.edges.push(edge);
        Ok(EdgeId::new(self.edges.len() - 1))
    }

    pub fn node(&self, id: NodeId) -> &LayoutNode {
        &self.nodes[id.index()]
    }

    pub fn edge(&self, id: EdgeId) -> &LayoutEdge {
        &self.edges[id.index()]
    }

    pub fn group(&self, id: GroupId) -> &LayoutGroup {
        &self.groups[id.index()]
    }

    pub fn nodes(&self) -> impl ExactSizeIterator<Item = (NodeId, &LayoutNode)> {
        self.nodes.iter().enumerate().map(|(i, n)| (NodeId::new(i), n))
    }

    pub fn edges(&self) -> impl ExactSizeIterator<Item = (EdgeId, &LayoutEdge)> {
        self.edges.iter().enumerate().map(|(i, e)| (EdgeId::new(i), e))
    }

    pub fn groups(&self) -> impl ExactSizeIterator<Item = (GroupId, &LayoutGroup)> {
        self.groups.iter().enumerate().map(|(i, g)| (GroupId::new(i), g))
    }

    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    pub fn edge_count(&self) -> usize {
        self.edges.len()
    }

    pub fn group_count(&self) -> usize {
        self.groups.len()
    }

    pub fn node_by_key(&self, key: &str) -> Option<NodeId> {
        self.keys.get(key).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_duplicate_keys_and_missing_ports() {
        let mut g = LayoutGraph::new();
        let a = g
            .add_node(LayoutNode::new("a", Size::new(10.0, 10.0)).with_ports(vec![Port { side: PortSide::East }]))
            .expect("a");
        let b = g.add_node(LayoutNode::new("b", Size::new(10.0, 10.0))).expect("b");
        assert_eq!(g.add_node(LayoutNode::new("a", Size::default())), Err(GraphError::DuplicateKey("a".into())));
        assert!(g.add_edge(LayoutEdge::new(EdgeEnd::port(a, 0), EdgeEnd::node(b))).is_ok());
        assert_eq!(
            g.add_edge(LayoutEdge::new(EdgeEnd::node(a), EdgeEnd::port(b, 0))),
            Err(GraphError::NoSuchPort { key: "b".into(), port: 0 })
        );
        assert_eq!(g.node_by_key("b"), Some(b));
    }
}
