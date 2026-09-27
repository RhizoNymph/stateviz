//! Attachment slots: where on a node's side each edge end attaches.
//!
//! A side's slots are its explicit ports, in port-index order, followed by
//! one implicit slot per unported edge end on that side, ordered by where
//! the edge heads (so unported fans never cross themselves). Slots are
//! spread evenly along the side; edges sharing a port share its slot.
//!
//! Edges leaving through North or South first run vertically to a stub
//! "level" (a multiple of the edge spacing away from the node), then
//! horizontally to a channel. Levels are nested so stubs heading the same
//! way do not cross. Self-loops between different sides go around the
//! node's corners on a level of their own.

use crate::geometry::{Point, Rect};

use super::frame::Side;
use super::problem::Problem;

#[derive(Clone, Debug)]
pub(crate) struct Slots {
    /// Per node, per side: number of slots.
    pub count: Vec<[usize; 4]>,
    /// Per node, per side: number of explicit port slots.
    explicit: Vec<[usize; 4]>,
    /// Per edge: slot index of the source end and of the target end.
    pub index: Vec<[usize; 2]>,
    /// Per node, per port: stub level of North/South ports (0 = none).
    port_level: Vec<Vec<u32>>,
    /// Per self-loop edge: levels used on the north and south sides.
    pub loop_level: Vec<[u32; 2]>,
    /// Per node: north and south levels in use.
    levels: Vec<[u32; 2]>,
    /// Per node: total height of self-loop labels stacked above it.
    loop_label_height: Vec<f32>,
    /// Per node: widest self-loop label.
    loop_label_width: Vec<f32>,
}

/// Which corner levels a self-loop between two sides uses: (north, south).
pub(crate) fn loop_sides(s: Side, t: Side) -> (bool, bool) {
    use Side::{East, North, South, West};
    match (s, t) {
        (East, East) | (West, West) => (false, false),
        (North, North) => (true, false),
        (South, South) => (false, true),
        (North, South) | (South, North) => (true, true),
        (East | West, North) | (North, East | West) | (East, West) | (West, East) => (true, false),
        (East | West, South) | (South, East | West) => (false, true),
    }
}

impl Slots {
    /// Count slots, rank explicit ports and assign stub levels. `heads_right`
    /// tells, for an edge end routed through channels, whether it heads to
    /// the channel right of its node (`None` for ends not routed that way).
    pub(crate) fn plan(
        problem: &Problem<'_>,
        heads_right: &dyn Fn(usize, bool) -> Option<bool>,
        label_gap: f32,
    ) -> Self {
        let n = problem.nodes.len();
        let mut count = vec![[0usize; 4]; n];
        let mut port_rank: Vec<Vec<usize>> = Vec::with_capacity(n);
        for (v, node) in problem.nodes.iter().enumerate() {
            let ranks = node
                .ports
                .iter()
                .map(|side| {
                    let r = count[v][side.index()];
                    count[v][side.index()] += 1;
                    r
                })
                .collect();
            port_rank.push(ranks);
        }
        let explicit = count.clone();
        let mut index = vec![[0usize; 2]; problem.edges.len()];
        for (e, edge) in problem.edges.iter().enumerate() {
            for (k, end) in [edge.source, edge.target].iter().enumerate() {
                match end.port {
                    Some(p) => index[e][k] = port_rank[end.node][p],
                    None => count[end.node][end.side.index()] += 1,
                }
            }
        }

        // Stub levels for North/South ports that edges leave through.
        let mut direction: Vec<Vec<Option<bool>>> = problem.nodes.iter().map(|n| vec![None; n.ports.len()]).collect();
        for (e, edge) in problem.edges.iter().enumerate() {
            for (source, end) in [(true, edge.source), (false, edge.target)] {
                if let (Some(p), Side::North | Side::South) = (end.port, end.side)
                    && direction[end.node][p].is_none()
                {
                    direction[end.node][p] = heads_right(e, source);
                }
            }
        }
        let mut port_level: Vec<Vec<u32>> = problem.nodes.iter().map(|n| vec![0; n.ports.len()]).collect();
        let mut levels = vec![[0u32; 2]; n];
        for (v, node) in problem.nodes.iter().enumerate() {
            for (li, side) in [Side::North, Side::South].into_iter().enumerate() {
                let mut right: Vec<usize> = Vec::new();
                let mut left: Vec<usize> = Vec::new();
                for (p, s) in node.ports.iter().enumerate() {
                    if *s == side {
                        match direction[v][p] {
                            Some(true) => right.push(p),
                            Some(false) => left.push(p),
                            None => {}
                        }
                    }
                }
                // Nearest to the channel gets the lowest level.
                right.sort_by_key(|&p| std::cmp::Reverse(port_rank[v][p]));
                left.sort_by_key(|&p| port_rank[v][p]);
                for group in [&right, &left] {
                    for (i, &p) in group.iter().enumerate() {
                        let level = u32::try_from(i + 1).unwrap_or(u32::MAX);
                        port_level[v][p] = level;
                        levels[v][li] = levels[v][li].max(level);
                    }
                }
            }
        }

        let mut loop_level = vec![[0u32; 2]; problem.edges.len()];
        let mut loop_label_height = vec![0.0f32; n];
        let mut loop_label_width = vec![0.0f32; n];
        for (e, edge) in problem.edges.iter().enumerate() {
            if !edge.is_self_loop() {
                continue;
            }
            let v = edge.source.node;
            let (north, south) = loop_sides(edge.source.side, edge.target.side);
            if north {
                levels[v][0] += 1;
                loop_level[e][0] = levels[v][0];
            }
            if south {
                levels[v][1] += 1;
                loop_level[e][1] = levels[v][1];
            }
            if let Some(label) = edge.label {
                loop_label_height[v] += label.height + label_gap;
                loop_label_width[v] = loop_label_width[v].max(label.width);
            }
        }

        Self { count, explicit, index, port_level, loop_level, levels, loop_label_height, loop_label_width }
    }

    /// Order each side's implicit slots by `key` (ties by edge, then source
    /// before target).
    pub(crate) fn order_implicit(&mut self, problem: &Problem<'_>, key: &dyn Fn(usize, bool) -> f64) {
        let mut ends: Vec<(usize, usize, f64, usize, bool)> = Vec::new();
        for (e, edge) in problem.edges.iter().enumerate() {
            for (source, end) in [(true, edge.source), (false, edge.target)] {
                if end.port.is_none() {
                    ends.push((end.node, end.side.index(), key(e, source), e, source));
                }
            }
        }
        ends.sort_by(|a, b| {
            (a.0, a.1).cmp(&(b.0, b.1)).then(a.2.total_cmp(&b.2)).then(a.3.cmp(&b.3)).then(b.4.cmp(&a.4))
        });
        let mut i = 0;
        while i < ends.len() {
            let (node, side) = (ends[i].0, ends[i].1);
            let mut rank = self.explicit[node][side];
            while i < ends.len() && ends[i].0 == node && ends[i].1 == side {
                let (_, _, _, e, source) = ends[i];
                self.index[e][usize::from(!source)] = rank;
                rank += 1;
                i += 1;
            }
        }
    }

    /// Room a node needs above and below itself for stubs, loops and loop
    /// labels.
    pub(crate) fn margins(&self, node: usize, edge_spacing: f32) -> (f32, f32) {
        let [n, s] = self.levels[node];
        let level_room = |l: u32| if l == 0 { 0.0 } else { (l as f32 + 0.5) * edge_spacing };
        (level_room(n) + self.loop_label_height[node], level_room(s))
    }

    /// Widest self-loop label on the node.
    pub(crate) fn loop_label_width(&self, node: usize) -> f32 {
        self.loop_label_width[node]
    }

    /// Stub level of an end on a North/South side (at least 1).
    pub(crate) fn level(&self, problem: &Problem<'_>, edge: usize, source: bool) -> u32 {
        let end = problem.edges[edge].end(source);
        end.port.map_or(1, |p| self.port_level[end.node][p].max(1))
    }

    /// Height of the north stub levels (without loop labels).
    pub(crate) fn north_levels(&self, node: usize) -> u32 {
        self.levels[node][0]
    }

    /// Cross offset, from the node's top, of the horizontal line the end
    /// leaves (or arrives) along.
    pub(crate) fn line_offset(&self, problem: &Problem<'_>, edge: usize, source: bool, edge_spacing: f32) -> f32 {
        let end = problem.edges[edge].end(source);
        let size = problem.nodes[end.node].size;
        let k = usize::from(!source);
        match end.side {
            Side::East | Side::West => {
                slot_fraction(self.index[edge][k], self.count[end.node][end.side.index()]) * size.height
            }
            Side::North => -(self.level(problem, edge, source) as f32) * edge_spacing,
            Side::South => size.height + self.level(problem, edge, source) as f32 * edge_spacing,
        }
    }

    /// Attachment point of an edge end on its node's rect.
    pub(crate) fn attach(&self, problem: &Problem<'_>, edge: usize, source: bool, rect: Rect) -> Point {
        let end = problem.edges[edge].end(source);
        let k = usize::from(!source);
        attach_point(rect, end.side, self.index[edge][k], self.count[end.node][end.side.index()])
    }

    /// The end's position along its side, in (0, 1) — for ordering.
    pub(crate) fn fraction(&self, problem: &Problem<'_>, edge: usize, source: bool) -> f64 {
        let end = problem.edges[edge].end(source);
        f64::from(slot_fraction(self.index[edge][usize::from(!source)], self.count[end.node][end.side.index()]))
    }
}

pub(crate) fn slot_fraction(index: usize, count: usize) -> f32 {
    (index as f32 + 1.0) / (count.max(1) as f32 + 1.0)
}

pub(crate) fn attach_point(rect: Rect, side: Side, index: usize, count: usize) -> Point {
    let f = slot_fraction(index, count);
    match side {
        Side::East => Point::new(rect.right(), rect.top() + f * rect.size.height),
        Side::West => Point::new(rect.left(), rect.top() + f * rect.size.height),
        Side::North => Point::new(rect.left() + f * rect.size.width, rect.top()),
        Side::South => Point::new(rect.left() + f * rect.size.width, rect.bottom()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slots_spread_evenly() {
        let r = Rect::new(0.0, 0.0, 40.0, 80.0);
        assert_eq!(attach_point(r, Side::East, 0, 3), Point::new(40.0, 20.0));
        assert_eq!(attach_point(r, Side::East, 2, 3), Point::new(40.0, 60.0));
        assert_eq!(attach_point(r, Side::North, 0, 1), Point::new(20.0, 0.0));
        assert_eq!(attach_point(r, Side::South, 1, 3), Point::new(20.0, 80.0));
    }

    #[test]
    fn loop_corner_sides() {
        assert_eq!(loop_sides(Side::East, Side::East), (false, false));
        assert_eq!(loop_sides(Side::East, Side::West), (true, false));
        assert_eq!(loop_sides(Side::South, Side::West), (false, true));
        assert_eq!(loop_sides(Side::North, Side::South), (true, true));
    }
}
