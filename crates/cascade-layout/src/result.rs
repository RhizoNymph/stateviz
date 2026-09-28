//! Layout output.

use crate::geometry::{Point, Rect};
use crate::graph::{EdgeId, GroupId, LayoutGraph, NodeId};
use crate::options::{PreviousLayout, PreviousPlacement};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NodePlacement {
    pub rect: Rect,
    pub layer: u32,
    /// Position within the layer, 0 first.
    pub order: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct EdgeRoute {
    /// The polyline from source attachment point to target attachment point,
    /// inclusive. Always at least two points.
    pub points: Vec<Point>,
    /// The edge was reversed to break a cycle: it runs against the flow.
    /// The causal view draws these red.
    pub reversed: bool,
    /// Where the edge's label box goes, when the edge has one.
    pub label: Option<Rect>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LayoutResult {
    nodes: Vec<NodePlacement>,
    edges: Vec<EdgeRoute>,
    groups: Vec<Rect>,
    /// Bounding box of everything, including edge routes.
    pub bounds: Rect,
}

impl LayoutResult {
    /// Assemble a result. Every vector is indexed by the matching graph id.
    pub fn new(nodes: Vec<NodePlacement>, edges: Vec<EdgeRoute>, groups: Vec<Rect>, bounds: Rect) -> Self {
        Self { nodes, edges, groups, bounds }
    }

    pub fn node(&self, id: NodeId) -> &NodePlacement {
        &self.nodes[id.index()]
    }

    pub fn edge(&self, id: EdgeId) -> &EdgeRoute {
        &self.edges[id.index()]
    }

    pub fn group(&self, id: GroupId) -> Rect {
        self.groups[id.index()]
    }

    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    /// Snapshot for the next layout's [`crate::LayoutHints::previous`].
    pub fn to_previous(&self, graph: &LayoutGraph) -> PreviousLayout {
        PreviousLayout {
            nodes: graph
                .nodes()
                .map(|(id, node)| {
                    let p = self.node(id);
                    (node.key.clone(), PreviousPlacement { layer: p.layer, order: p.order, position: p.rect.origin })
                })
                .collect(),
        }
    }
}
