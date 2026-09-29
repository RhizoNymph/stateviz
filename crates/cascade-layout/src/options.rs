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
    /// is laid out, shift its nodes horizontally (keeping their in-group
    /// order and spacing) toward the median x of the nodes they connect to
    /// in other groups, so edges between stacked groups run as straight as
    /// possible. Off by default.
    ///
    /// Owner: `feat/readable-routing` implements it; until then it is
    /// accepted and ignored.
    pub align_across_groups: bool,
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
