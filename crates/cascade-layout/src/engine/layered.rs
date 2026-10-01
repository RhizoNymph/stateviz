//! The layered graph of one band: real nodes and dummy nodes ("items") in
//! geometric layers (columns), and the chain of items each edge is routed
//! through.
//!
//! Channels are the gaps between columns: channel `c` lies left of column
//! `c`, so channel 0 is left of everything and channel `L` right of
//! everything. An edge end leaves its node into a channel chosen by its
//! side (East: the channel right of the node; West: left; North/South:
//! toward the other end), and the chain gets one dummy in every column
//! between the two end channels. That single rule covers forward edges,
//! reversed edges (which leave East, turn back through their own column and
//! arrive West), flat edges and ports facing any way.

use crate::geometry::Size;

use super::frame::Side;
use super::problem::{EdgeKind, Problem};

pub(crate) type ItemId = usize;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ItemKind {
    /// A real node (global node index).
    Node(usize),
    /// A point an edge passes through horizontally.
    Dummy { chain: usize },
    /// A dummy that also carries the edge's label box above the line.
    Label { chain: usize },
}

#[derive(Clone, Debug)]
pub(crate) struct Item {
    pub kind: ItemKind,
    /// Geometric layer (column).
    pub layer: usize,
    /// Size along the main axis.
    pub width: f32,
    /// Main-axis room the item needs in its column (at least `width`; more
    /// for a node with wide self-loop labels above it).
    pub reserve: f32,
    /// Core size along the cross axis (the node, or the label box plus gap).
    pub height: f32,
    /// Room above the core for north stubs, loops and loop labels.
    pub margin_top: f32,
    /// Room below the core for south stubs and loops.
    pub margin_bottom: f32,
    /// Offset from `top` to the line a dummy's edge passes along.
    pub anchor: f32,
    /// Cross position of the core's top edge.
    pub top: f32,
    /// Main-axis position of the item's left edge.
    pub x: f32,
}

impl Item {
    pub(crate) fn is_node(&self) -> bool {
        matches!(self.kind, ItemKind::Node(_))
    }

    pub(crate) fn node(&self) -> Option<usize> {
        match self.kind {
            ItemKind::Node(n) => Some(n),
            _ => None,
        }
    }

    pub(crate) fn box_top(&self) -> f32 {
        self.top - self.margin_top
    }

    pub(crate) fn box_bottom(&self) -> f32 {
        self.top + self.height + self.margin_bottom
    }

    /// Total cross extent including margins.
    pub(crate) fn extent(&self) -> f32 {
        self.margin_top + self.height + self.margin_bottom
    }

    /// Where a dummy's edge passes.
    pub(crate) fn line(&self) -> f32 {
        self.top + self.anchor
    }
}

/// The route of one edge through a band: `items[0]` is the source node,
/// the last item the target node, dummies between. `channels[k]` is the
/// channel link `k` (from `items[k]` to `items[k + 1]`) runs through.
#[derive(Clone, Debug)]
pub(crate) struct Chain {
    pub edge: usize,
    pub items: Vec<ItemId>,
    pub channels: Vec<usize>,
}

#[derive(Clone, Debug)]
pub(crate) struct BandGraph {
    /// Reported layer of each column, ascending.
    pub values: Vec<u32>,
    pub items: Vec<Item>,
    /// Items of each column, in order.
    pub layers: Vec<Vec<ItemId>>,
    pub chains: Vec<Chain>,
}

impl BandGraph {
    pub(crate) fn columns(&self) -> usize {
        self.layers.len()
    }
}

/// Channel an edge end leaves (`source`) or enters through. North and South
/// ends head toward the other end's column; when both ends share a column
/// they both use the channel on the right, so no dummy is needed.
pub(crate) fn end_channel(side: Side, column: usize, other_column: usize, source: bool) -> usize {
    match side {
        Side::East => column + 1,
        Side::West => column,
        Side::North | Side::South => {
            if source {
                if other_column < column { column } else { column + 1 }
            } else if other_column >= column {
                column + 1
            } else {
                column
            }
        }
    }
}

/// Build the band's items and chains. `layer_of` gives every node's reported
/// layer; `item_of` receives each non-pinned node's item.
pub(crate) fn build(
    problem: &Problem<'_>,
    band: usize,
    layer_of: &[u32],
    item_of: &mut [Option<ItemId>],
    label_gap: f32,
) -> BandGraph {
    let members: Vec<usize> =
        problem.bands[band].nodes.iter().copied().filter(|&n| problem.nodes[n].pin.is_none()).collect();
    let chain_edges: Vec<usize> = (0..problem.edges.len())
        .filter(|&e| problem.kinds[e] == EdgeKind::Chain && problem.nodes[problem.edges[e].source.node].band == band)
        .collect();

    // Columns: every node layer, plus the middle layer of long labelled
    // forward edges so the label has a column of its own.
    let mut values: Vec<u32> = members.iter().map(|&n| layer_of[n]).collect();
    for &e in &chain_edges {
        let edge = &problem.edges[e];
        let (ls, lt) = (layer_of[edge.source.node], layer_of[edge.target.node]);
        if edge.label.is_some() && lt >= ls.saturating_add(2) {
            values.push(ls + (lt - ls) / 2);
        }
    }
    values.sort_unstable();
    values.dedup();
    let column = |layer: u32| values.binary_search(&layer).unwrap_or(0);

    let mut items = Vec::with_capacity(members.len());
    for &n in &members {
        let size = problem.nodes[n].size;
        item_of[n] = Some(items.len());
        items.push(Item {
            kind: ItemKind::Node(n),
            layer: column(layer_of[n]),
            width: size.width,
            reserve: size.width,
            height: size.height,
            margin_top: 0.0,
            margin_bottom: 0.0,
            anchor: size.height / 2.0,
            top: 0.0,
            x: 0.0,
        });
    }

    let mut chains = Vec::with_capacity(chain_edges.len());
    for &e in &chain_edges {
        let edge = &problem.edges[e];
        let (Some(si), Some(ti)) = (item_of[edge.source.node], item_of[edge.target.node]) else { continue };
        let (ls, lt) = (items[si].layer, items[ti].layer);
        let cs = end_channel(edge.source.side, ls, lt, true);
        let ct = end_channel(edge.target.side, lt, ls, false);
        let dummy_layers: Vec<usize> = if cs < ct {
            (cs..ct).collect()
        } else if cs > ct {
            (ct..cs).rev().collect()
        } else {
            Vec::new()
        };
        let chain_index = chains.len();
        let mut chain_items = vec![si];
        let label_at = edge.label.filter(|_| !dummy_layers.is_empty()).map(|l| (dummy_layers.len() / 2, l));
        for (k, &layer) in dummy_layers.iter().enumerate() {
            let item = match label_at {
                Some((at, label)) if at == k => label_item(chain_index, layer, label, label_gap),
                _ => Item {
                    kind: ItemKind::Dummy { chain: chain_index },
                    layer,
                    width: 0.0,
                    reserve: 0.0,
                    height: 0.0,
                    margin_top: 0.0,
                    margin_bottom: 0.0,
                    anchor: 0.0,
                    top: 0.0,
                    x: 0.0,
                },
            };
            chain_items.push(items.len());
            items.push(item);
        }
        chain_items.push(ti);
        let mut channels = Vec::with_capacity(chain_items.len() - 1);
        channels.push(cs);
        for w in dummy_layers.windows(2) {
            channels.push(if w[1] > w[0] { w[1] } else { w[0] });
        }
        if !dummy_layers.is_empty() {
            channels.push(ct);
        }
        chains.push(Chain { edge: e, items: chain_items, channels });
    }

    let mut layers: Vec<Vec<ItemId>> = vec![Vec::new(); values.len()];
    for (i, item) in items.iter().enumerate() {
        layers[item.layer].push(i);
    }
    BandGraph { values, items, layers, chains }
}

fn label_item(chain: usize, layer: usize, label: Size, gap: f32) -> Item {
    Item {
        kind: ItemKind::Label { chain },
        layer,
        width: label.width,
        reserve: label.width,
        height: label.height + gap,
        margin_top: 0.0,
        margin_bottom: 0.0,
        anchor: label.height + gap,
        top: 0.0,
        x: 0.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn end_channels_follow_sides() {
        assert_eq!(end_channel(Side::East, 2, 5, true), 3);
        assert_eq!(end_channel(Side::West, 5, 2, false), 5);
        assert_eq!(end_channel(Side::North, 2, 1, true), 2);
        assert_eq!(end_channel(Side::South, 2, 4, true), 3);
        assert_eq!(end_channel(Side::North, 4, 2, false), 4);
        assert_eq!(end_channel(Side::South, 1, 4, false), 2);
    }
}
