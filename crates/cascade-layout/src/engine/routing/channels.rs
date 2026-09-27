//! Vertical segments in a band's channels and their tracks.
//!
//! Every chain link whose ends are at different heights needs a vertical
//! segment in its channel; so do East/West self-loops (a C on the node's
//! side) and the in-band legs of cross-band edges (from the node to the
//! band's top or bottom boundary). Each channel's segments get tracks,
//! spread evenly across the channel's width.

use std::collections::BTreeMap;

use super::super::context::{Columns, Ctx};
use super::super::frame::Side;
use super::super::problem::EdgeKind;
use super::super::tracks::{Toward, TrackSeg, assign};

/// Identity of a segment whose track position routing looks up.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum SegKey {
    /// Link `link` of chain `chain` in its band.
    Link { chain: usize, link: usize },
    /// An East/East or West/West self-loop.
    Loop { edge: usize },
    /// A cross-band edge's leg in its source band.
    Exit { edge: usize },
    /// A cross-band edge's leg in its target band.
    Entry { edge: usize },
}

#[derive(Clone, Debug)]
pub(crate) struct ChannelSeg {
    pub channel: usize,
    pub key: SegKey,
    pub seg: TrackSeg,
}

/// Channel a cross-band edge's end uses in its own band.
pub(crate) fn cross_channel(side: Side, column: usize, source: bool) -> usize {
    match side {
        Side::East => column + 1,
        Side::West => column,
        Side::North | Side::South => {
            if source {
                column + 1
            } else {
                column
            }
        }
    }
}

/// Net key of an explicit port: segments at one port share a track.
pub(crate) fn net(node: usize, port: usize) -> u64 {
    ((node as u64) << 16) | port as u64
}

fn toward(item_column: usize, channel: usize) -> Toward {
    if item_column < channel { Toward::Low } else { Toward::High }
}

/// Line (cross position) an edge end leaves its node along.
pub(crate) fn end_line(ctx: &Ctx<'_, '_>, edge: usize, source: bool) -> f32 {
    let node = ctx.p.edges[edge].end(source).node;
    ctx.node_rect(node).top() + ctx.slots.line_offset(ctx.p, edge, source, ctx.p.spacing.edge)
}

/// All channel segments of `band`. `top` and `bottom` are the band's
/// boundaries (where cross-band legs leave), and `below(b)` tells whether
/// band `b` is stacked below this one.
pub(crate) fn collect(
    ctx: &Ctx<'_, '_>,
    band: usize,
    top: f32,
    bottom: f32,
    below: &dyn Fn(usize) -> bool,
) -> Vec<ChannelSeg> {
    let bg = &ctx.bands[band];
    let p = ctx.p;
    let mut out = Vec::new();
    for (ci, chain) in bg.chains.iter().enumerate() {
        let last = chain.items.len() - 1;
        let edge = &p.edges[chain.edge];
        for k in 0..last {
            let (a, b) = (chain.items[k], chain.items[k + 1]);
            let channel = chain.channels[k];
            let ya = if k == 0 { end_line(ctx, chain.edge, true) } else { bg.items[a].line() };
            let yb = if k + 1 == last { end_line(ctx, chain.edge, false) } else { bg.items[b].line() };
            let (ta, tb) = (toward(bg.items[a].layer, channel), toward(bg.items[b].layer, channel));
            if (ya - yb).abs() < 1e-3 && ta != tb {
                continue;
            }
            let nets = [
                (k == 0).then_some(edge.source.port.map(|port| net(edge.source.node, port))).flatten(),
                (k + 1 == last).then_some(edge.target.port.map(|port| net(edge.target.node, port))).flatten(),
            ];
            out.push(ChannelSeg {
                channel,
                key: SegKey::Link { chain: ci, link: k },
                seg: TrackSeg { lo: ya.min(yb), hi: ya.max(yb), joins: vec![(ya, ta), (yb, tb)], nets },
            });
        }
    }
    for (e, edge) in p.edges.iter().enumerate() {
        let s_band = p.nodes[edge.source.node].band;
        let t_band = p.nodes[edge.target.node].band;
        match p.kinds[e] {
            EdgeKind::SelfLoop if s_band == band => {
                let Some(item) = ctx.item(edge.source.node) else { continue };
                let (channel, join) = match (edge.source.side, edge.target.side) {
                    (Side::East, Side::East) => (item.layer + 1, Toward::Low),
                    (Side::West, Side::West) => (item.layer, Toward::High),
                    _ => continue,
                };
                let (y1, y2) = (end_line(ctx, e, true), end_line(ctx, e, false));
                let nets = [edge.source.port.map(|q| net(edge.source.node, q)), None];
                out.push(ChannelSeg {
                    channel,
                    key: SegKey::Loop { edge: e },
                    seg: TrackSeg { lo: y1.min(y2), hi: y1.max(y2), joins: vec![(y1, join), (y2, join)], nets },
                });
            }
            EdgeKind::CrossBand => {
                for (source, node_band, other_band) in [(true, s_band, t_band), (false, t_band, s_band)] {
                    if node_band != band {
                        continue;
                    }
                    let end = edge.end(source);
                    let Some(item) = ctx.item(end.node) else { continue };
                    let channel = cross_channel(end.side, item.layer, source);
                    let y = end_line(ctx, e, source);
                    let edge_y = if below(other_band) { bottom } else { top };
                    let key = if source { SegKey::Exit { edge: e } } else { SegKey::Entry { edge: e } };
                    out.push(ChannelSeg {
                        channel,
                        key,
                        seg: TrackSeg {
                            lo: y.min(edge_y),
                            hi: y.max(edge_y),
                            joins: vec![(y, toward(item.layer, channel))],
                            nets: [end.port.map(|q| net(end.node, q)), None],
                        },
                    });
                }
            }
            _ => {}
        }
    }
    out
}

/// Tracks per segment and the number of tracks in each of `channels`
/// channels.
pub(crate) fn assign_tracks(segs: &[ChannelSeg], channels: usize) -> (Vec<usize>, Vec<usize>) {
    let mut by_channel: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for (i, s) in segs.iter().enumerate() {
        by_channel.entry(s.channel).or_default().push(i);
    }
    let mut track = vec![0usize; segs.len()];
    let mut counts = vec![0usize; channels];
    for (c, members) in by_channel {
        let input: Vec<TrackSeg> = members.iter().map(|&i| segs[i].seg.clone()).collect();
        let (t, n) = assign(&input);
        for (k, &i) in members.iter().enumerate() {
            track[i] = t[k];
        }
        if let Some(slot) = counts.get_mut(c) {
            *slot = n;
        }
    }
    (track, counts)
}

/// Main-axis position of every segment: tracks spread evenly across their
/// channel.
pub(crate) fn positions(
    segs: &[ChannelSeg],
    track: &[usize],
    counts: &[usize],
    cols: &Columns,
) -> BTreeMap<SegKey, f32> {
    segs.iter()
        .zip(track)
        .map(|(s, &t)| {
            let (l, r) = cols.channel(s.channel);
            let n = counts.get(s.channel).copied().unwrap_or(1).max(1);
            (s.key, l + (t as f32 + 1.0) * (r - l) / (n as f32 + 1.0))
        })
        .collect()
}
