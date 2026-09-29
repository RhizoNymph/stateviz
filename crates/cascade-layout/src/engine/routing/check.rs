//! Route validation: does a polyline cross a node or a group it should
//! stay out of?

use std::collections::HashMap;

use crate::geometry::{Point, Rect};

/// Tolerance: touching a rect's border (within this much) is not a
/// crossing.
pub(crate) const SHRINK: f32 = 0.5;

/// Whether the segment `a`–`b` enters the interior of `rect` shrunk by
/// `shrink` on every side (Liang–Barsky clipping).
pub(crate) fn segment_hits(a: Point, b: Point, rect: &Rect, shrink: f32) -> bool {
    let (l, t, r, bt) = (rect.left() + shrink, rect.top() + shrink, rect.right() - shrink, rect.bottom() - shrink);
    if l >= r || t >= bt {
        return false;
    }
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let (mut t0, mut t1) = (0.0f32, 1.0f32);
    for (p, q) in [(-dx, a.x - l), (dx, r - a.x), (-dy, a.y - t), (dy, bt - a.y)] {
        if p.abs() < 1e-9 {
            if q <= 0.0 {
                return false;
            }
        } else {
            let v = q / p;
            if p < 0.0 {
                t0 = t0.max(v);
            } else {
                t1 = t1.min(v);
            }
        }
    }
    t0 < t1
}

/// Uniform-grid index of rects for segment queries.
pub(crate) struct RectIndex {
    rects: Vec<Rect>,
    cell: f32,
    buckets: HashMap<(i64, i64), Vec<usize>>,
}

impl RectIndex {
    pub(crate) fn new(rects: Vec<Rect>) -> Self {
        let cell = 128.0;
        let mut buckets: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
        for (i, r) in rects.iter().enumerate() {
            let (x0, y0, x1, y1) = Self::cells(cell, r.left(), r.top(), r.right(), r.bottom());
            for cx in x0..=x1 {
                for cy in y0..=y1 {
                    buckets.entry((cx, cy)).or_default().push(i);
                }
            }
        }
        Self { rects, cell, buckets }
    }

    fn cells(cell: f32, x0: f32, y0: f32, x1: f32, y1: f32) -> (i64, i64, i64, i64) {
        let f = |v: f32| (v / cell).floor().clamp(-1e9, 1e9) as i64;
        (f(x0), f(y0), f(x1), f(y1))
    }

    /// Indices of rects whose interior the segment crosses, sorted.
    pub(crate) fn hits(&self, a: Point, b: Point) -> Vec<usize> {
        let (x0, y0, x1, y1) = Self::cells(self.cell, a.x.min(b.x), a.y.min(b.y), a.x.max(b.x), a.y.max(b.y));
        let mut found = Vec::new();
        if (x1 - x0 + 1).saturating_mul(y1 - y0 + 1) > 4096 {
            found.extend((0..self.rects.len()).filter(|&i| segment_hits(a, b, &self.rects[i], SHRINK)));
            return found;
        }
        for cx in x0..=x1 {
            for cy in y0..=y1 {
                if let Some(bucket) = self.buckets.get(&(cx, cy)) {
                    found.extend(bucket.iter().copied().filter(|&i| segment_hits(a, b, &self.rects[i], SHRINK)));
                }
            }
        }
        found.sort_unstable();
        found.dedup();
        found
    }

    /// Whether `r` overlaps the interior of any indexed rect.
    pub(crate) fn overlaps(&self, r: &Rect) -> bool {
        let shrunk = |x: &Rect| {
            Rect::new(x.left() + SHRINK, x.top() + SHRINK, x.size.width - 2.0 * SHRINK, x.size.height - 2.0 * SHRINK)
        };
        let (x0, y0, x1, y1) = Self::cells(self.cell, r.left(), r.top(), r.right(), r.bottom());
        if (x1 - x0 + 1).saturating_mul(y1 - y0 + 1) > 4096 {
            return self.rects.iter().any(|x| shrunk(x).intersects(r));
        }
        (x0..=x1).any(|cx| {
            (y0..=y1).any(|cy| {
                self.buckets.get(&(cx, cy)).is_some_and(|b| b.iter().any(|&i| shrunk(&self.rects[i]).intersects(r)))
            })
        })
    }

    pub(crate) fn rects(&self) -> &[Rect] {
        &self.rects
    }
}

/// Whether segment `a`–`b` runs straight across `group` along the stacking
/// axis, from above its top to below its bottom: how a route passes a band
/// through one of its passages.
pub(crate) fn passes_across(a: Point, b: Point, group: &Rect) -> bool {
    (a.x - b.x).abs() < 1e-3 && a.y.min(b.y) <= group.top() + SHRINK && a.y.max(b.y) >= group.bottom() - SHRINK
}

/// Whether a route stays clear of every indexed node, and enters the given
/// foreign group rects only straight across them.
pub(crate) fn is_clear(points: &[Point], nodes: &RectIndex, foreign_groups: &[Rect]) -> bool {
    points.windows(2).all(|w| {
        nodes.hits(w[0], w[1]).is_empty()
            && !foreign_groups.iter().any(|g| segment_hits(w[0], w[1], g, SHRINK) && !passes_across(w[0], w[1], g))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn border_touching_is_not_a_hit() {
        let r = Rect::new(0.0, 0.0, 10.0, 10.0);
        assert!(!segment_hits(Point::new(10.0, -5.0), Point::new(10.0, 15.0), &r, SHRINK));
        assert!(segment_hits(Point::new(5.0, -5.0), Point::new(5.0, 15.0), &r, SHRINK));
        assert!(segment_hits(Point::new(-5.0, -5.0), Point::new(15.0, 15.0), &r, SHRINK));
        assert!(!segment_hits(Point::new(-5.0, 5.0), Point::new(0.0, 5.0), &r, SHRINK));
    }

    #[test]
    fn index_finds_crossed_rects() {
        let idx = RectIndex::new(vec![Rect::new(0.0, 0.0, 10.0, 10.0), Rect::new(500.0, 0.0, 10.0, 10.0)]);
        assert_eq!(idx.hits(Point::new(-5.0, 5.0), Point::new(600.0, 5.0)), vec![0, 1]);
        assert!(idx.hits(Point::new(-5.0, 20.0), Point::new(600.0, 20.0)).is_empty());
    }
}
