//! One-dimensional placement within a column.
//!
//! - [`place_l1`]: items in a fixed order with minimum gaps, each pulled
//!   toward weighted targets; minimises `Σ w·|top − target|` exactly by pool
//!   adjacent violators with weighted medians. L1 (rather than squared)
//!   distances make edges exactly straight whenever the constraints allow.
//! - [`Occupancy`]: the stable-mode free-space finder: place a new item as
//!   close to where it wants to be as possible without moving anything.
//! - [`push_off`]: move a column's items off obstacles (pinned nodes) with
//!   the least displacement, keeping their order and gaps.

/// A weighted pull toward `value`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Target {
    pub value: f32,
    pub weight: f32,
}

struct Block {
    first: usize,
    last: usize,
    /// Candidate positions for the block's first item, sorted by value.
    cands: Vec<(f32, f32)>,
    pos: f32,
}

fn weighted_median(cands: &[(f32, f32)], reference: f32) -> f32 {
    let total: f64 = cands.iter().map(|c| f64::from(c.1)).sum();
    if cands.is_empty() || total <= 0.0 {
        return reference;
    }
    let half = total / 2.0;
    let eps = total * 1e-9;
    let mut cum = 0.0f64;
    for (i, &(v, w)) in cands.iter().enumerate() {
        cum += f64::from(w);
        if cum > half + eps {
            return v;
        }
        if cum >= half - eps {
            // Every point between this candidate and the next is optimal.
            let next = cands.get(i + 1).map_or(v, |c| c.0);
            return reference.clamp(v, next.max(v));
        }
    }
    cands[cands.len() - 1].0
}

fn merge_sorted(a: Vec<(f32, f32)>, b: &[(f32, f32)], shift: f32) -> Vec<(f32, f32)> {
    let mut out = Vec::with_capacity(a.len() + b.len());
    let (mut i, mut j) = (0, 0);
    while i < a.len() || j < b.len() {
        let take_a = match (a.get(i), b.get(j)) {
            (Some(x), Some(y)) => x.0 <= y.0 - shift,
            (Some(_), None) => true,
            _ => false,
        };
        if take_a {
            out.push(a[i]);
            i += 1;
        } else {
            out.push((b[j].0 - shift, b[j].1));
            j += 1;
        }
    }
    out
}

/// Place `n` items in order. `gaps[k]` is the minimum distance from item
/// `k − 1`'s top to item `k`'s top (`gaps[0]` is ignored), `targets[k]` the
/// pulls on item `k`'s top, and `current[k]` breaks ties (an item with no
/// pull stays put unless pushed).
pub(crate) fn place_l1(gaps: &[f32], targets: &[Vec<Target>], current: &[f32]) -> Vec<f32> {
    let n = gaps.len();
    let mut prefix = vec![0.0f32; n];
    for k in 1..n {
        prefix[k] = prefix[k - 1] + gaps[k];
    }
    let mut blocks: Vec<Block> = Vec::with_capacity(n);
    for k in 0..n {
        let mut cands: Vec<(f32, f32)> =
            targets[k].iter().filter(|t| t.weight > 0.0).map(|t| (t.value, t.weight)).collect();
        cands.sort_by(|a, b| a.0.total_cmp(&b.0));
        let pos = weighted_median(&cands, current[k]);
        let mut block = Block { first: k, last: k, cands, pos };
        while let Some(prev) = blocks.last() {
            let offset = prefix[block.first] - prefix[prev.first];
            if block.pos >= prev.pos + offset {
                break;
            }
            let Some(prev) = blocks.pop() else { break };
            let cands = merge_sorted(prev.cands, &block.cands, offset);
            let pos = weighted_median(&cands, current[prev.first]);
            block = Block { first: prev.first, last: block.last, cands, pos };
        }
        blocks.push(block);
    }
    let mut out = vec![0.0f32; n];
    for b in &blocks {
        for k in b.first..=b.last {
            out[k] = b.pos + prefix[k] - prefix[b.first];
        }
    }
    out
}

/// An occupied stretch of a column.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Occupied {
    pub top: f32,
    pub bottom: f32,
    pub is_node: bool,
}

/// Occupied stretches of one column, sorted and non-overlapping.
#[derive(Clone, Debug, Default)]
pub(crate) struct Occupancy {
    entries: Vec<Occupied>,
}

impl Occupancy {
    pub(crate) fn insert(&mut self, top: f32, bottom: f32, is_node: bool) {
        let i = self.entries.partition_point(|e| e.top < top);
        self.entries.insert(i, Occupied { top, bottom, is_node });
    }

    /// The top closest to `desired` where a box of `height` fits, keeping
    /// `sep(other_is_node)` from every occupied stretch.
    pub(crate) fn nearest_free(&self, desired: f32, height: f32, sep: &dyn Fn(bool) -> f32) -> f32 {
        let n = self.entries.len();
        if n == 0 {
            return desired;
        }
        // lo[i]: lowest top allowed below entries[..i]; hi[i]: highest top
        // allowed above entries[i..].
        let mut lo = vec![f32::NEG_INFINITY; n + 1];
        for i in 0..n {
            let e = self.entries[i];
            lo[i + 1] = lo[i].max(e.bottom + sep(e.is_node));
        }
        let mut hi = vec![f32::INFINITY; n + 1];
        for i in (0..n).rev() {
            let e = self.entries[i];
            hi[i] = hi[i + 1].min(e.top - sep(e.is_node) - height);
        }
        let mut best: Option<f32> = None;
        for i in 0..=n {
            if lo[i] <= hi[i] {
                let c = desired.clamp(lo[i], hi[i]);
                if best.is_none_or(|b| (c - desired).abs() < (b - desired).abs()) {
                    best = Some(c);
                }
            }
        }
        best.unwrap_or(lo[n])
    }

    /// Whether a box fits at `top`.
    pub(crate) fn fits(&self, top: f32, height: f32, sep: &dyn Fn(bool) -> f32) -> bool {
        self.entries.iter().all(|e| {
            let s = sep(e.is_node);
            top + height + s <= e.top || top >= e.bottom + s
        })
    }
}

/// A column item for [`push_off`]: its box relative to its `top`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Stacked {
    pub top: f32,
    /// Box extends this far above `top`.
    pub above: f32,
    /// Box extends this far below `top`.
    pub below: f32,
    /// Minimum distance from the previous item's box.
    pub gap: f32,
    /// Separation to keep from obstacles.
    pub clearance: f32,
}

/// Move items (in order) off the obstacle intervals, keeping order and
/// gaps. An item moves up only if that is closer and clear of every
/// obstacle and of the item before it; otherwise it moves down past the
/// obstacles. Returns the new tops.
pub(crate) fn push_off(items: &[Stacked], obstacles: &[(f32, f32)]) -> Vec<f32> {
    // A little slack so a position that exactly clears an obstacle (as a
    // previous layout's did) is not nudged by rounding.
    const SLACK: f32 = 0.01;
    let hits = |top: f32, it: &Stacked| {
        obstacles
            .iter()
            .find(|o| top - it.above < o.1 + it.clearance - SLACK && top + it.below > o.0 - it.clearance + SLACK)
            .copied()
    };
    let mut out = Vec::with_capacity(items.len());
    let mut floor = f32::NEG_INFINITY;
    for it in items {
        let mut top = it.top.max(floor + it.gap + it.above);
        if let Some(o) = hits(top, it) {
            let up = o.0 - it.clearance - it.below;
            let down_first = o.1 + it.clearance + it.above;
            let up_ok = up - it.above >= floor + it.gap && hits(up, it).is_none();
            if up_ok && (top - up) <= (down_first - top) {
                top = up;
            } else {
                top = down_first;
                let mut guard = obstacles.len() + 1;
                while let Some(o) = hits(top, it) {
                    top = o.1 + it.clearance + it.above;
                    guard -= 1;
                    if guard == 0 {
                        break;
                    }
                }
            }
        }
        floor = top + it.below;
        out.push(top);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(value: f32) -> Vec<Target> {
        vec![Target { value, weight: 1.0 }]
    }

    #[test]
    fn unconstrained_items_reach_their_targets() {
        let tops = place_l1(&[0.0, 10.0, 10.0], &[t(0.0), t(50.0), t(100.0)], &[0.0; 3]);
        assert_eq!(tops, vec![0.0, 50.0, 100.0]);
    }

    #[test]
    fn colliding_items_share_the_displacement_by_median() {
        // Both want 10, need 20 apart: any split is L1-optimal; the result
        // keeps the gap and lands one item on its target.
        let tops = place_l1(&[0.0, 20.0], &[t(10.0), t(10.0)], &[0.0, 0.0]);
        assert_eq!(tops[1] - tops[0], 20.0);
        assert!(tops[0] <= 10.0 && tops[1] >= 10.0);
        // Heavier pull wins.
        let heavy = vec![Target { value: 10.0, weight: 5.0 }];
        let tops = place_l1(&[0.0, 20.0], &[t(10.0), heavy], &[0.0, 0.0]);
        assert_eq!(tops, vec![-10.0, 10.0]);
    }

    #[test]
    fn chains_of_violations_merge() {
        let tops = place_l1(&[0.0, 10.0, 10.0, 10.0], &[t(5.0), t(5.0), t(5.0), t(100.0)], &[0.0; 4]);
        assert_eq!(tops[1] - tops[0], 10.0);
        assert_eq!(tops[2] - tops[1], 10.0);
        assert_eq!(tops[3], 100.0);
        assert_eq!(tops[1], 5.0);
    }

    #[test]
    fn free_space_is_found_nearest_to_desire() {
        let mut occ = Occupancy::default();
        occ.insert(0.0, 20.0, true);
        occ.insert(40.0, 60.0, true);
        let sep = |_: bool| 5.0;
        // Gap between 25 and 35 fits a 10-high box exactly.
        assert_eq!(occ.nearest_free(10.0, 10.0, &sep), 25.0);
        // Too tall for the gap: goes above or below, whichever is closer.
        assert_eq!(occ.nearest_free(10.0, 12.0, &sep), -17.0);
        assert_eq!(occ.nearest_free(45.0, 12.0, &sep), 65.0);
        assert!(occ.fits(25.0, 10.0, &sep));
        assert!(!occ.fits(24.0, 10.0, &sep));
    }

    #[test]
    fn items_are_pushed_off_obstacles() {
        let item = |top: f32| Stacked { top, above: 0.0, below: 10.0, gap: 5.0, clearance: 2.0 };
        let tops = push_off(&[item(0.0), item(15.0), item(30.0)], &[(12.0, 20.0)]);
        // First item clear; second hits [12, 20]: up would be 0 (collides
        // with the first item's gap), so it goes down to 22; third follows.
        assert_eq!(tops, vec![0.0, 22.0, 37.0]);
        // Moving up is closer here.
        let tops = push_off(&[item(100.0)], &[(105.0, 115.0)]);
        assert_eq!(tops, vec![93.0]);
        // And down here.
        let tops = push_off(&[item(100.0)], &[(95.0, 103.0)]);
        assert_eq!(tops, vec![105.0]);
    }
}
