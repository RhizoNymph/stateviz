//! Layout parameters and the hints that keep layouts stable.

use std::collections::{BTreeMap, HashMap};

use serde::{Deserialize, Serialize};

use crate::geometry::Point;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FlowDirection {
    /// Layers run left to right (causal view, lanes of the structure view).
    #[default]
    LeftToRight,
    TopToBottom,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EdgeRouting {
    /// Horizontal and vertical segments only.
    #[default]
    Orthogonal,
    /// Straight segments through layer-crossing dummy points.
    Polyline,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct LayoutOptions {
    pub direction: FlowDirection,
    pub routing: EdgeRouting,
    /// Gap between neighbouring nodes in one layer.
    pub node_spacing: f32,
    /// Gap between consecutive layers.
    pub layer_spacing: f32,
    /// Gap between parallel edge segments.
    pub edge_spacing: f32,
    /// Gap between stacked groups (lanes).
    pub group_spacing: f32,
    /// Pull nodes toward their neighbours in other groups: after each group
    /// is laid out, shift its layers along the flow (keeping their order and
    /// spacing; a layer of only edge bends and labels moves with the layer
    /// before it) toward the other ends of their edges into other groups,
    /// so edges between stacked groups run as straight as possible. Layers
    /// only spread (right with left-to-right flow); groups kept from
    /// `LayoutHints::previous` and pinned nodes do not move. Off by default.
    pub align_across_groups: bool,
    /// Lay the groups out on one shared set of layers: layering runs over
    /// the whole graph (edges between groups included), and every group
    /// puts a layer at the same position along the flow, each layer as wide
    /// as its widest item in any group. An edge between groups then always
    /// runs from a lower to a higher layer, and every segment of it heads
    /// with the flow (or across it), unless it was reversed to break a
    /// cycle (`EdgeRoute::reversed`, which can now flag edges between
    /// groups too). Channel tracks keep edges leaving a layer before edges
    /// arriving at the next, and every edge crossing groups in between gets
    /// a reserved vertical of its own, so no side corridor is used.
    /// Takes precedence over `align_across_groups`. Off by default.
    #[serde(default)]
    pub shared_layers: bool,
}

impl Default for LayoutOptions {
    fn default() -> Self {
        Self {
            direction: FlowDirection::LeftToRight,
            routing: EdgeRouting::Orthogonal,
            node_spacing: 24.0,
            layer_spacing: 64.0,
            edge_spacing: 8.0,
            group_spacing: 32.0,
            align_across_groups: false,
            shared_layers: false,
        }
    }
}

/// Where a node sat in the previous layout, by node key.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct PreviousPlacement {
    pub layer: u32,
    /// Position within the layer, 0 first.
    pub order: u32,
    pub position: Point,
}

/// The previous layout of (roughly) the same graph. Fed back on every edit so
/// adding one transition does not reshuffle the whole diagram: nodes keep
/// their layer and relative order where the new graph allows it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PreviousLayout {
    pub nodes: HashMap<String, PreviousPlacement>,
}

/// Inputs beyond the graph that steer placement.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LayoutHints {
    pub previous: Option<PreviousLayout>,
    /// Nodes the user dragged, by key: their top-left corner is placed
    /// exactly here and everything else routes around them.
    pub pins: BTreeMap<String, Point>,
}
