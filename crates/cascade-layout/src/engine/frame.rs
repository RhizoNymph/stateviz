//! The canonical frame: the whole pipeline works left to right, with layers
//! along `x` and positions within a layer along `y`. A top-to-bottom layout
//! is the same layout transposed (reflected across `y = x`), which maps
//! top-left corners to top-left corners and swaps widths with heights.

use crate::geometry::{Insets, Point, Rect, Size};
use crate::graph::{LayoutGroup, PortSide};
use crate::options::FlowDirection;

/// A node side in the canonical frame. `East` faces the next layer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) enum Side {
    North,
    East,
    South,
    West,
}

impl Side {
    pub(crate) const fn index(self) -> usize {
        match self {
            Side::North => 0,
            Side::East => 1,
            Side::South => 2,
            Side::West => 3,
        }
    }

    /// Unit vector pointing out of a node through this side.
    pub(crate) const fn outward(self) -> (f32, f32) {
        match self {
            Side::North => (0.0, -1.0),
            Side::East => (1.0, 0.0),
            Side::South => (0.0, 1.0),
            Side::West => (-1.0, 0.0),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Frame {
    transpose: bool,
}

impl Frame {
    pub(crate) const fn new(direction: FlowDirection) -> Self {
        Self { transpose: matches!(direction, FlowDirection::TopToBottom) }
    }

    /// Real ↔ canonical (the transform is its own inverse).
    pub(crate) const fn size(&self, s: Size) -> Size {
        if self.transpose { Size::new(s.height, s.width) } else { s }
    }

    pub(crate) const fn point(&self, p: Point) -> Point {
        if self.transpose { Point::new(p.y, p.x) } else { p }
    }

    pub(crate) const fn rect(&self, r: Rect) -> Rect {
        Rect::from_origin_size(self.point(r.origin), self.size(r.size))
    }

    /// A real port side in the canonical frame.
    pub(crate) const fn side(&self, s: PortSide) -> Side {
        match (self.transpose, s) {
            (false, PortSide::North) | (true, PortSide::West) => Side::North,
            (false, PortSide::East) | (true, PortSide::South) => Side::East,
            (false, PortSide::South) | (true, PortSide::East) => Side::South,
            (false, PortSide::West) | (true, PortSide::North) => Side::West,
        }
    }

    /// A group's canonical insets, with the header band on the real top.
    pub(crate) fn group_insets(&self, g: &LayoutGroup) -> Insets {
        let p = g.padding;
        if self.transpose {
            Insets { top: p.left, right: p.bottom, bottom: p.right, left: p.top + g.header }
        } else {
            Insets { top: p.top + g.header, right: p.right, bottom: p.bottom, left: p.left }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transpose_round_trips_and_maps_sides() {
        let f = Frame::new(FlowDirection::TopToBottom);
        let r = Rect::new(1.0, 2.0, 30.0, 40.0);
        assert_eq!(f.rect(f.rect(r)), r);
        assert_eq!(f.rect(r), Rect::new(2.0, 1.0, 40.0, 30.0));
        assert_eq!(f.side(PortSide::South), Side::East);
        assert_eq!(f.side(PortSide::North), Side::West);
        let id = Frame::new(FlowDirection::LeftToRight);
        assert_eq!(id.rect(r), r);
        assert_eq!(id.side(PortSide::West), Side::West);
    }

    #[test]
    fn header_stays_on_the_real_top() {
        let g = LayoutGroup {
            key: "g".into(),
            padding: Insets { top: 1.0, right: 2.0, bottom: 3.0, left: 4.0 },
            header: 10.0,
        };
        let ltr = Frame::new(FlowDirection::LeftToRight).group_insets(&g);
        assert_eq!(ltr.top, 11.0);
        let ttb = Frame::new(FlowDirection::TopToBottom).group_insets(&g);
        // Canonical left is the real top.
        assert_eq!(ttb.left, 11.0);
        assert_eq!(ttb.top, 4.0);
    }
}
