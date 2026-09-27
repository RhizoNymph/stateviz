//! Cascade layout: a layered (Sugiyama-style) graph layout engine.
//!
//! Generic over what the boxes mean: callers build a [`LayoutGraph`] of sized
//! nodes, optional ports and optional groups (lanes), and get back node
//! rectangles and edge routes. It stands in for ELK's layered algorithm in
//! the spec, since there is no ELK for native Rust.
//!
//! ```text
//! LayoutGraph + LayoutOptions + LayoutHints ──layout()──▶ LayoutResult
//!                                   ▲                         │
//!                                   └────── to_previous() ────┘   (stability)
//! ```

mod engine;
pub mod geometry;
pub mod graph;
pub mod options;
pub mod result;

pub use engine::{LayoutError, layout};
pub use geometry::{Insets, Point, Rect, Size};
pub use graph::{
    EdgeEnd, EdgeId, GraphError, GroupId, LayerConstraint, LayoutEdge, LayoutGraph, LayoutGroup, LayoutNode, NodeId,
    Port, PortSide,
};
pub use options::{EdgeRouting, FlowDirection, LayoutHints, LayoutOptions, PreviousLayout, PreviousPlacement};
pub use result::{EdgeRoute, LayoutResult, NodePlacement};
