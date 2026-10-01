//! Shared layers (`LayoutOptions::shared_layers`): every band uses one set
//! of layers and puts each layer at the same position along the flow.
//!
//! ```text
//! layering::assign   one layering over the whole graph (edges between
//!                    bands included), soft previous layers released where
//!                    they would make an acyclic edge point backwards
//! columns::place     global columns (as wide as their widest item in any
//!                    band), channel zones sized over every band, column
//!                    positions (fresh, or kept from the previous layout)
//! positions          channel tracks placed zone by zone
//! cross::plans       edges between bands: legs, gap runs, and a reserved
//!                    vertical through the bands in between
//! ```
//!
//! Each global channel (the space between two neighbouring global columns)
//! is split into zones, left to right, with the same slots in every band:
//!
//! | Zone | Holds |
//! | --- | --- |
//! | `Exit` | legs of edges leaving the column on the left for another band |
//! | `Pass` | verticals of edges passing bands in between (reserved slots) |
//! | `Link` | the band's own edges (chain links, East/West self-loops) |
//! | `Entry` | legs of edges from another band entering the column on the right |
//!
//! So an edge between bands whose target layer is higher than its source
//! layer leaves in an exit zone, passes in a pass zone to its right and
//! arrives in an entry zone further right: every horizontal run heads with
//! the flow. A band's channel between two of its columns that are not
//! neighbours globally spans several global channels: exits use the first,
//! everything else the last.

pub(crate) mod columns;
pub(crate) mod cross;
pub(crate) mod layering;

use std::collections::BTreeMap;

use super::routing::channels::{ChannelSeg, SegKey};
use super::tracks::{Toward, TrackSeg, assign};

/// Which part of a global channel a vertical segment uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Zone {
    Exit,
    Pass,
    Link,
    Entry,
}

impl Zone {
    const ALL: [Zone; 4] = [Zone::Exit, Zone::Pass, Zone::Link, Zone::Entry];

    const fn index(self) -> usize {
        match self {
            Zone::Exit => 0,
            Zone::Pass => 1,
            Zone::Link => 2,
            Zone::Entry => 3,
        }
    }
}

/// Slots of every zone of one global channel: the most tracks any band
/// needs there.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Caps([usize; 4]);

impl Caps {
    pub(crate) fn get(&self, zone: Zone) -> usize {
        self.0[zone.index()]
    }

    pub(crate) fn raise(&mut self, zone: Zone, n: usize) {
        let slot = &mut self.0[zone.index()];
        *slot = (*slot).max(n);
    }

    /// Slots in all zones.
    pub(crate) fn total(&self) -> usize {
        self.0.iter().sum()
    }

    /// First slot of `zone`.
    fn start(&self, zone: Zone) -> usize {
        Zone::ALL.iter().take_while(|z| **z != zone).map(|z| self.get(*z)).sum()
    }
}

/// The shared main-axis geometry.
#[derive(Clone, Debug, Default)]
pub(crate) struct Shared {
    /// Global column of every band's local columns.
    pub global_of: Vec<Vec<usize>>,
    /// Main-axis extent of every global channel: channel `g` lies left of
    /// global column `g`, the last one right of every column.
    pub channels: Vec<(f32, f32)>,
    /// Zone slots of every global channel.
    pub caps: Vec<Caps>,
    /// Reserved vertical of every edge that passes bands in between:
    /// (global channel, pass slot).
    pub through: BTreeMap<usize, (usize, usize)>,
}

/// The global channels a band's local channel `c` spans: (first, last).
/// `global_of` maps the band's local columns to global ones.
pub(crate) fn channel_span(global_of: &[usize], c: usize) -> (usize, usize) {
    let left = c.checked_sub(1).and_then(|l| global_of.get(l)).copied();
    let right = global_of.get(c).copied();
    match (left, right) {
        (Some(a), Some(b)) => (a + 1, b),
        (None, Some(b)) => (b, b),
        (Some(a), None) => (a + 1, a + 1),
        (None, None) => (0, 0),
    }
}

/// The global channel and zone of one of a band's channel segments. Exit
/// legs go to the first global channel the local channel spans (right
/// beside the column they leave); a link or loop whose ends both lie left
/// of the channel too; everything else to the last one.
pub(crate) fn zone_of(global_of: &[usize], seg: &ChannelSeg) -> (usize, Zone) {
    let (first, last) = channel_span(global_of, seg.channel);
    match seg.key {
        SegKey::Exit { .. } => (first, Zone::Exit),
        SegKey::Entry { .. } => (last, Zone::Entry),
        SegKey::Link { .. } | SegKey::Loop { .. } => {
            if seg.seg.joins.iter().all(|(_, t)| *t == Toward::Low) {
                (first, Zone::Link)
            } else {
                (last, Zone::Link)
            }
        }
    }
}

/// A global channel and one of its zones.
pub(crate) type ZoneKey = (usize, Zone);

/// Tracks of a band's segments, assigned per zone.
pub(crate) struct ZoneTracks {
    /// The zone of every segment.
    pub zones: Vec<ZoneKey>,
    /// The track of every segment within its zone.
    pub track: Vec<usize>,
    /// Tracks used per zone.
    pub counts: BTreeMap<ZoneKey, usize>,
}

/// Tracks of a band's segments, assigned per (global channel, zone).
pub(crate) fn zone_tracks(global_of: &[usize], segs: &[ChannelSeg]) -> ZoneTracks {
    let zones: Vec<ZoneKey> = segs.iter().map(|s| zone_of(global_of, s)).collect();
    let mut members: BTreeMap<ZoneKey, Vec<usize>> = BTreeMap::new();
    for (i, z) in zones.iter().enumerate() {
        members.entry(*z).or_default().push(i);
    }
    let mut track = vec![0usize; segs.len()];
    let mut counts = BTreeMap::new();
    for (zone, list) in members {
        let input: Vec<TrackSeg> = list.iter().map(|&i| segs[i].seg.clone()).collect();
        let (t, n) = assign(&input);
        for (k, &i) in list.iter().enumerate() {
            track[i] = t[k];
        }
        counts.insert(zone, n);
    }
    ZoneTracks { zones, track, counts }
}

impl Shared {
    /// Main-axis position of track `t` of `n` in `zone` of global channel
    /// `g`. Slots are spread evenly over the channel; a band with fewer
    /// tracks than the zone has slots is centred in the zone, and one with
    /// more (only possible around pins) is squeezed into the zone.
    pub(crate) fn position(&self, g: usize, zone: Zone, t: usize, n: usize) -> f32 {
        let (Some(&(lo, hi)), Some(caps)) = (self.channels.get(g), self.caps.get(g)) else { return 0.0 };
        let pitch = (hi - lo) / (caps.total() as f32 + 1.0);
        let (start, cap) = (caps.start(zone) as f32, caps.get(zone));
        if n <= cap {
            let slot = start + (cap - n) as f32 / 2.0 + t as f32;
            lo + (slot + 1.0) * pitch
        } else {
            let a = lo + (start + 0.5) * pitch;
            let b = lo + (start + cap as f32 + 0.5) * pitch;
            a + (t as f32 + 1.0) * (b - a) / (n as f32 + 1.0)
        }
    }

    /// Track positions of band `band`'s channel segments.
    pub(crate) fn positions(&self, band: usize, segs: &[ChannelSeg]) -> BTreeMap<SegKey, f32> {
        let Some(global_of) = self.global_of.get(band) else { return BTreeMap::new() };
        let ZoneTracks { zones, track, counts } = zone_tracks(global_of, segs);
        segs.iter()
            .enumerate()
            .map(|(i, s)| {
                let (g, zone) = zones[i];
                let n = counts.get(&(g, zone)).copied().unwrap_or(1);
                (s.key, self.position(g, zone, track[i], n))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_spans_cover_the_global_channels_between_neighbours() {
        let global_of = [1, 4];
        assert_eq!(channel_span(&global_of, 0), (1, 1));
        assert_eq!(channel_span(&global_of, 1), (2, 4));
        assert_eq!(channel_span(&global_of, 2), (5, 5));
        assert_eq!(channel_span(&[], 0), (0, 0));
    }

    #[test]
    fn zones_run_exit_pass_link_entry_left_to_right() {
        let mut caps = Caps::default();
        caps.raise(Zone::Exit, 2);
        caps.raise(Zone::Pass, 1);
        caps.raise(Zone::Link, 3);
        caps.raise(Zone::Entry, 2);
        caps.raise(Zone::Exit, 1);
        let shared = Shared { channels: vec![(0.0, 90.0)], caps: vec![caps], ..Shared::default() };
        let exits: Vec<f32> = (0..2).map(|t| shared.position(0, Zone::Exit, t, 2)).collect();
        let pass = shared.position(0, Zone::Pass, 0, 1);
        let links: Vec<f32> = (0..3).map(|t| shared.position(0, Zone::Link, t, 3)).collect();
        let entries: Vec<f32> = (0..2).map(|t| shared.position(0, Zone::Entry, t, 2)).collect();
        let all: Vec<f32> = exits.iter().chain([&pass]).chain(&links).chain(&entries).copied().collect();
        assert_eq!(all, (1..=8).map(|k| k as f32 * 10.0).collect::<Vec<_>>());
        // One track in a two-slot zone sits between the slots.
        assert_eq!(shared.position(0, Zone::Entry, 0, 1), 75.0);
        // More tracks than slots stay inside the zone.
        let squeezed: Vec<f32> = (0..3).map(|t| shared.position(0, Zone::Pass, t, 3)).collect();
        assert!(squeezed.iter().all(|&x| x > 20.0 && x < 40.0), "{squeezed:?}");
    }
}
