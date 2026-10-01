//! Free passages: vertical strips through a band along which a cross-band
//! route can pass the band without touching its nodes, labels, channel
//! tracks or any pinned node.
//!
//! A passage is what is left of the band's width (the lanes' shared
//! extent) once every column's extent and every channel track is blocked,
//! each widened by the edge spacing. The strip left of the first column is
//! never a passage: that is where a lane's title sits. Pinned nodes block
//! their extent in every band, since where they end up vertically is only
//! known after stacking.

use std::collections::BTreeMap;

use super::super::super::context::{Columns, Ctx};
use super::super::channels::SegKey;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Passage {
    /// Where a vertical may run: every x in `lo..=hi` is at least the edge
    /// spacing from anything in the band.
    pub lo: f32,
    pub hi: f32,
    /// In-band links a vertical here crosses.
    pub crossings: usize,
    /// How many verticals fit, the edge spacing apart.
    pub capacity: usize,
}

impl Passage {
    pub(crate) fn clamp(&self, x: f32) -> f32 {
        x.clamp(self.lo, self.hi)
    }
}

/// Links running through each channel of the band (channel `c` is left of
/// column `c`; the last one right of every column).
fn links_per_channel(ctx: &Ctx<'_, '_>, band: usize, channels: usize) -> Vec<usize> {
    let mut count = vec![0usize; channels];
    for chain in &ctx.bands[band].chains {
        for &c in &chain.channels {
            if let Some(n) = count.get_mut(c) {
                *n += 1;
            }
        }
    }
    count
}

/// The band's passages, left to right. `tracks` holds the band's channel
/// track positions, `span` the lanes' shared extent and `pins` the extents
/// of pinned nodes.
pub(crate) fn of_band(
    ctx: &Ctx<'_, '_>,
    band: usize,
    cols: &Columns,
    tracks: &BTreeMap<SegKey, f32>,
    span: (f32, f32),
    pins: &[(f32, f32)],
) -> Vec<Passage> {
    let es = ctx.p.spacing.edge;
    let n = cols.count();
    let mut start = span.0 + es;
    if n > 0 {
        start = start.max(cols.left[0]);
    }
    let end = span.1 - es;
    let mut blocked: Vec<(f32, f32)> = (0..n).map(|l| (cols.left[l] - es, cols.right[l] + es)).collect();
    blocked.extend(tracks.values().map(|&x| (x - es, x + es)));
    blocked.extend(pins.iter().map(|&(a, b)| (a - es, b + es)));
    blocked.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));

    let links = links_per_channel(ctx, band, n + 1);
    let channel_of = |x: f32| cols.left.iter().position(|&l| l > x).unwrap_or(n);
    let mut out = Vec::new();
    let mut push = |lo: f32, hi: f32| {
        if hi >= lo {
            let capacity = ((hi - lo) / es).floor() as usize + 1;
            let crossings = links.get(channel_of((lo + hi) / 2.0)).copied().unwrap_or(0);
            out.push(Passage { lo, hi, crossings, capacity });
        }
    };
    let mut cursor = start;
    for (a, b) in blocked {
        if a > cursor {
            push(cursor, a.min(end));
        }
        cursor = cursor.max(b);
        if cursor >= end {
            break;
        }
    }
    if cursor < end {
        push(cursor, end);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamp_stays_inside() {
        let p = Passage { lo: 10.0, hi: 20.0, crossings: 0, capacity: 2 };
        assert_eq!(p.clamp(0.0), 10.0);
        assert_eq!(p.clamp(15.0), 15.0);
        assert_eq!(p.clamp(30.0), 20.0);
    }
}
