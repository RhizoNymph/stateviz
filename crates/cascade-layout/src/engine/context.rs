//! Read-only views over the pipeline's state shared by the placement and
//! routing stages.

use crate::geometry::{Rect, Size};

use super::layered::{BandGraph, Item};
use super::problem::Problem;
use super::slots::Slots;

pub(crate) struct Ctx<'a, 'g> {
    pub p: &'a Problem<'g>,
    pub bands: &'a [BandGraph],
    /// Item of every non-pinned node in its band.
    pub item_of: &'a [Option<usize>],
    pub slots: &'a Slots,
}

impl Ctx<'_, '_> {
    pub(crate) fn item(&self, node: usize) -> Option<&Item> {
        let band = self.p.nodes[node].band;
        self.item_of[node].map(|i| &self.bands[band].items[i])
    }

    /// Canonical rect of a node: its pin, or its item's position.
    pub(crate) fn node_rect(&self, node: usize) -> Rect {
        let size: Size = self.p.nodes[node].size;
        match (self.p.nodes[node].pin, self.item(node)) {
            (Some(pin), _) => Rect::from_origin_size(pin, size),
            (None, Some(item)) => Rect::new(item.x, item.top, size.width, size.height),
            (None, None) => Rect::from_origin_size(Default::default(), size),
        }
    }
}

/// Main-axis geometry of a band: column extents and channel widths.
#[derive(Clone, Debug, Default)]
pub(crate) struct Columns {
    pub left: Vec<f32>,
    pub right: Vec<f32>,
    /// Width of channel 0 (left of everything) and channel `L` (right).
    pub entry: f32,
    pub exit: f32,
}

impl Columns {
    pub(crate) fn of(bg: &BandGraph, entry: f32, exit: f32) -> Self {
        let mut left = Vec::with_capacity(bg.columns());
        let mut right = Vec::with_capacity(bg.columns());
        for layer in &bg.layers {
            let (lo, hi) = layer.iter().fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), &i| {
                let it = &bg.items[i];
                let cx = it.x + it.width / 2.0;
                (lo.min(cx - it.reserve / 2.0), hi.max(cx + it.reserve / 2.0))
            });
            let (lo, hi) = if lo.is_finite() { (lo, hi) } else { (0.0, 0.0) };
            left.push(lo);
            right.push(hi);
        }
        Self { left, right, entry, exit }
    }

    pub(crate) fn count(&self) -> usize {
        self.left.len()
    }

    /// Channel `c`'s extent, left of column `c`.
    pub(crate) fn channel(&self, c: usize) -> (f32, f32) {
        let n = self.count();
        if n == 0 {
            return (0.0, 0.0);
        }
        if c == 0 {
            (self.left[0] - self.entry, self.left[0])
        } else if c >= n {
            (self.right[n - 1], self.right[n - 1] + self.exit)
        } else {
            (self.right[c - 1], self.left[c])
        }
    }

    pub(crate) fn centre(&self, l: usize) -> f32 {
        (self.left[l] + self.right[l]) / 2.0
    }

    /// Main-axis extent of everything, channels included.
    pub(crate) fn span(&self) -> Option<(f32, f32)> {
        let n = self.count();
        (n > 0).then(|| (self.left[0] - self.entry, self.right[n - 1] + self.exit))
    }
}
