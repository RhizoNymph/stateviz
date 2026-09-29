//! The validated layout problem in the canonical frame: sizes, sides, pins
//! and previous positions already transposed, ports resolved to sides,
//! nodes partitioned into bands (the ungrouped band, then one per group).

use crate::geometry::{Insets, Point, Size};
use crate::graph::{LayerConstraint, LayoutGraph};
use crate::options::{EdgeRouting, LayoutHints, LayoutOptions};

use super::LayoutError;
use super::frame::{Frame, Side};

/// Smallest effective edge spacing: stubs, loops and tracks need room.
pub(crate) const MIN_EDGE_SPACING: f32 = 2.0;

/// Spacing options, validated finite and non-negative.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Spacing {
    pub node: f32,
    pub layer: f32,
    pub edge: f32,
    pub group: f32,
}

/// How a node's layer is decided.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Fix {
    Free,
    /// From a `First`/`Exact` constraint; conflicts are errors.
    Hard(u32),
    /// From the previous layout; conflicts relax the edge instead.
    Soft(u32),
}

impl Fix {
    pub(crate) const fn value(self) -> Option<u32> {
        match self {
            Fix::Free => None,
            Fix::Hard(l) | Fix::Soft(l) => Some(l),
        }
    }
}

/// A node's previous placement, in the canonical frame.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Prev {
    pub layer: u32,
    pub order: u32,
    pub origin: Point,
}

#[derive(Clone, Debug)]
pub(crate) struct PNode {
    pub size: Size,
    pub band: usize,
    pub fix: Fix,
    /// Canonical side of each port, by port index.
    pub ports: Vec<Side>,
    /// Canonical top-left corner the user pinned the node to.
    pub pin: Option<Point>,
    pub prev: Option<Prev>,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct PEnd {
    pub node: usize,
    pub port: Option<usize>,
    pub side: Side,
}

#[derive(Clone, Debug)]
pub(crate) struct PEdge {
    pub source: PEnd,
    pub target: PEnd,
    /// Canonical label size.
    pub label: Option<Size>,
}

impl PEdge {
    pub(crate) fn is_self_loop(&self) -> bool {
        self.source.node == self.target.node
    }

    pub(crate) fn end(&self, source: bool) -> &PEnd {
        if source { &self.source } else { &self.target }
    }
}

/// How an edge is routed. Every edge is exactly one kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EdgeKind {
    /// Source and target are the same node: a small loop on its side.
    SelfLoop,
    /// Both ends in one band, neither pinned: routed through the band's
    /// layers and channels.
    Chain,
    /// Ends in different bands, neither pinned: out of the source band,
    /// through the gaps between bands (and a side corridor), into the target
    /// band.
    CrossBand,
    /// Touches a pinned node: routed around obstacles.
    Pinned,
}

/// A band: the ungrouped nodes (band 0) or one group (band `g + 1`).
#[derive(Clone, Debug)]
pub(crate) struct PBand {
    pub group: Option<usize>,
    /// Canonical insets, header included.
    pub insets: Insets,
    pub nodes: Vec<usize>,
}

/// How much of the previous layout applies.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Mode {
    /// No previous layout.
    Fresh,
    /// A previous layout matching too few nodes to keep positions: it only
    /// seeds the ordering.
    Seeded,
    /// Keep previous layers and positions for existing nodes.
    Stable,
}

pub(crate) struct Problem<'g> {
    pub graph: &'g LayoutGraph,
    pub frame: Frame,
    pub spacing: Spacing,
    pub routing: EdgeRouting,
    /// Shift freshly placed bands to line up their cross-band edges.
    pub align: bool,
    pub nodes: Vec<PNode>,
    pub edges: Vec<PEdge>,
    pub bands: Vec<PBand>,
    pub kinds: Vec<EdgeKind>,
    pub mode: Mode,
}

fn valid_len(v: f32) -> bool {
    v.is_finite() && v >= 0.0
}

impl<'g> Problem<'g> {
    pub(crate) fn build(
        graph: &'g LayoutGraph,
        options: &LayoutOptions,
        hints: &LayoutHints,
    ) -> Result<Self, LayoutError> {
        for (name, v) in [
            ("node_spacing", options.node_spacing),
            ("layer_spacing", options.layer_spacing),
            ("edge_spacing", options.edge_spacing),
            ("group_spacing", options.group_spacing),
        ] {
            if !valid_len(v) {
                return Err(LayoutError::InvalidOption { name });
            }
        }
        let frame = Frame::new(options.direction);
        // Routes need some room to leave a node and turn: edge and layer
        // spacings below these floors are raised to them.
        let edge = options.edge_spacing.max(MIN_EDGE_SPACING);
        let spacing = Spacing {
            node: options.node_spacing,
            layer: options.layer_spacing.max(2.0 * edge),
            edge,
            group: options.group_spacing,
        };

        let mut bands = vec![PBand { group: None, insets: Insets::default(), nodes: Vec::new() }];
        for (gid, group) in graph.groups() {
            let p = group.padding;
            if ![p.top, p.right, p.bottom, p.left, group.header].into_iter().all(valid_len) {
                return Err(LayoutError::InvalidGroup { key: group.key.clone() });
            }
            bands.push(PBand { group: Some(gid.index()), insets: frame.group_insets(group), nodes: Vec::new() });
        }

        let previous = hints.previous.as_ref();
        let mut matched = 0usize;
        let mut nodes = Vec::with_capacity(graph.node_count());
        for (id, node) in graph.nodes() {
            if !(valid_len(node.size.width) && valid_len(node.size.height)) {
                return Err(LayoutError::InvalidNodeSize { key: node.key.clone() });
            }
            let pin = match hints.pins.get(&node.key) {
                Some(p) if p.x.is_finite() && p.y.is_finite() => Some(frame.point(*p)),
                Some(_) => return Err(LayoutError::InvalidPin { key: node.key.clone() }),
                None => None,
            };
            let prev = previous
                .and_then(|p| p.nodes.get(&node.key))
                .filter(|p| p.position.x.is_finite() && p.position.y.is_finite())
                .map(|p| Prev { layer: p.layer, order: p.order, origin: frame.point(p.position) });
            matched += usize::from(prev.is_some());
            let band = node.group.map_or(0, |g| g.index() + 1);
            bands[band].nodes.push(id.index());
            let fix = match node.layer {
                LayerConstraint::First => Fix::Hard(0),
                LayerConstraint::Exact(l) => Fix::Hard(l),
                LayerConstraint::Free => Fix::Free,
            };
            nodes.push(PNode {
                size: frame.size(node.size),
                band,
                fix,
                ports: node.ports.iter().map(|p| frame.side(p.side)).collect(),
                pin,
                prev,
            });
        }

        let mode = if matched == 0 {
            Mode::Fresh
        } else if 2 * matched >= nodes.len() {
            Mode::Stable
        } else {
            Mode::Seeded
        };
        if mode == Mode::Stable {
            for n in &mut nodes {
                if let (Fix::Free, Some(prev)) = (n.fix, n.prev) {
                    n.fix = Fix::Soft(prev.layer);
                }
            }
        }

        let mut edges = Vec::with_capacity(graph.edge_count());
        for (eid, edge) in graph.edges() {
            let label = match edge.label {
                Some(s) if valid_len(s.width) && valid_len(s.height) => Some(frame.size(s)),
                Some(_) => return Err(LayoutError::InvalidEdgeLabel { edge: eid }),
                None => None,
            };
            let self_loop = edge.source.node == edge.target.node;
            let end = |node: crate::graph::NodeId, port: Option<u16>, out: bool| {
                let port = port.map(usize::from);
                let side = match port {
                    Some(p) => nodes[node.index()].ports[p],
                    None if out || self_loop => Side::East,
                    None => Side::West,
                };
                PEnd { node: node.index(), port, side }
            };
            edges.push(PEdge {
                source: end(edge.source.node, edge.source.port, true),
                target: end(edge.target.node, edge.target.port, false),
                label,
            });
        }

        let kinds = edges
            .iter()
            .map(|e| {
                let (s, t) = (&nodes[e.source.node], &nodes[e.target.node]);
                if e.is_self_loop() {
                    EdgeKind::SelfLoop
                } else if s.pin.is_some() || t.pin.is_some() {
                    EdgeKind::Pinned
                } else if s.band == t.band {
                    EdgeKind::Chain
                } else {
                    EdgeKind::CrossBand
                }
            })
            .collect();

        Ok(Self {
            graph,
            frame,
            spacing,
            routing: options.routing,
            align: options.align_across_groups,
            nodes,
            edges,
            bands,
            kinds,
            mode,
        })
    }

    pub(crate) fn key(&self, node: usize) -> &str {
        &self.graph.node(crate::graph::NodeId::new(node)).key
    }

    /// Separation between two neighbours in a layer.
    pub(crate) fn separation(&self, a_is_node: bool, b_is_node: bool) -> f32 {
        match (a_is_node, b_is_node) {
            (true, true) => self.spacing.node,
            (false, false) => self.spacing.edge,
            _ => self.spacing.edge.max(self.spacing.node / 2.0),
        }
    }
}
