//! Track assignment for parallel segments in a channel (between two
//! columns, in the gap between two bands, or in a side corridor).
//!
//! Segments whose spans overlap need different tracks. For every overlapping
//! pair we count the crossings each relative order would cause (a
//! perpendicular join heading across the other segment's span crosses it)
//! and prefer the cheaper order. The preferences form a dependency graph;
//! cycles are broken with the greedy feedback arc set, and each segment's
//! track is its longest-path depth. Segments sharing a net (edges on one
//! port) merge into one and share a track.

use std::collections::BTreeMap;

use super::cycles::greedy_arrangement;

/// Which side of the channel a join heads to: `Low` is left (or up in a
/// horizontal channel), `High` right (or down).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Toward {
    Low,
    High,
}

#[derive(Clone, Debug)]
pub(crate) struct TrackSeg {
    pub lo: f32,
    pub hi: f32,
    /// Where perpendicular segments join, and which way they head.
    pub joins: Vec<(f32, Toward)>,
    /// Net keys: segments sharing a key share a track.
    pub nets: [Option<u64>; 2],
}

const TOUCH: f32 = 0.5;

struct Group {
    lo: f32,
    hi: f32,
    joins: Vec<(f32, Toward)>,
}

fn find(parent: &mut [usize], mut x: usize) -> usize {
    while parent[x] != x {
        parent[x] = parent[parent[x]];
        x = parent[x];
    }
    x
}

fn crossings(left: &Group, right: &Group) -> usize {
    let within = |p: f32, g: &Group| p >= g.lo - TOUCH && p <= g.hi + TOUCH;
    left.joins.iter().filter(|(p, t)| *t == Toward::High && within(*p, right)).count()
        + right.joins.iter().filter(|(p, t)| *t == Toward::Low && within(*p, left)).count()
}

/// Track of every segment (0 = nearest the `Low` side) and the number of
/// tracks used.
pub(crate) fn assign(segs: &[TrackSeg]) -> (Vec<usize>, usize) {
    let n = segs.len();
    if n == 0 {
        return (Vec::new(), 0);
    }
    let mut parent: Vec<usize> = (0..n).collect();
    let mut first_with_net: BTreeMap<u64, usize> = BTreeMap::new();
    for (i, s) in segs.iter().enumerate() {
        for net in s.nets.iter().flatten() {
            match first_with_net.get(net) {
                Some(&j) => {
                    let (a, b) = (find(&mut parent, i), find(&mut parent, j));
                    if a != b {
                        parent[a.max(b)] = a.min(b);
                    }
                }
                None => {
                    first_with_net.insert(*net, i);
                }
            }
        }
    }
    let mut group_of = vec![0usize; n];
    let mut groups: Vec<Group> = Vec::new();
    let mut root_group: BTreeMap<usize, usize> = BTreeMap::new();
    for i in 0..n {
        let r = find(&mut parent, i);
        let g = *root_group.entry(r).or_insert_with(|| {
            groups.push(Group { lo: f32::INFINITY, hi: f32::NEG_INFINITY, joins: Vec::new() });
            groups.len() - 1
        });
        group_of[i] = g;
        let s = &segs[i];
        groups[g].lo = groups[g].lo.min(s.lo);
        groups[g].hi = groups[g].hi.max(s.hi);
        groups[g].joins.extend_from_slice(&s.joins);
    }

    let m = groups.len();
    let mut by_lo: Vec<usize> = (0..m).collect();
    by_lo.sort_by(|&a, &b| {
        groups[a].lo.total_cmp(&groups[b].lo).then(groups[a].hi.total_cmp(&groups[b].hi)).then(a.cmp(&b))
    });
    let mut deps: Vec<(usize, usize)> = Vec::new();
    for (ii, &i) in by_lo.iter().enumerate() {
        for &j in &by_lo[ii + 1..] {
            if groups[j].lo > groups[i].hi + TOUCH {
                break;
            }
            let (ij, ji) = (crossings(&groups[i], &groups[j]), crossings(&groups[j], &groups[i]));
            deps.push(if ji < ij { (j, i) } else { (i, j) });
        }
    }
    let order = greedy_arrangement(m, &deps);
    let mut pos = vec![0usize; m];
    for (k, &g) in order.iter().enumerate() {
        pos[g] = k;
    }
    let mut preds: Vec<Vec<usize>> = vec![Vec::new(); m];
    for &(a, b) in &deps {
        if pos[a] < pos[b] {
            preds[b].push(a);
        } else {
            preds[a].push(b);
        }
    }
    let mut track = vec![0usize; m];
    for &g in &order {
        track[g] = preds[g].iter().map(|&p| track[p] + 1).max().unwrap_or(0);
    }
    let count = track.iter().max().map_or(0, |t| t + 1);
    (group_of.iter().map(|&g| track[g]).collect(), count)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(lo: f32, hi: f32, joins: &[(f32, Toward)]) -> TrackSeg {
        TrackSeg { lo, hi, joins: joins.to_vec(), nets: [None, None] }
    }

    #[test]
    fn disjoint_segments_share_a_track() {
        let (t, n) = assign(&[seg(0.0, 10.0, &[]), seg(20.0, 30.0, &[])]);
        assert_eq!(n, 1);
        assert_eq!(t, vec![0, 0]);
    }

    #[test]
    fn overlapping_segments_are_ordered_to_avoid_crossings() {
        use Toward::{High, Low};
        // Two "down" staircase segments: a from 10 (left) to 50 (right),
        // b from 20 (left) to 60 (right). b must sit left of a.
        let a = seg(10.0, 50.0, &[(10.0, Low), (50.0, High)]);
        let b = seg(20.0, 60.0, &[(20.0, Low), (60.0, High)]);
        let (t, n) = assign(&[a, b]);
        assert_eq!(n, 2);
        assert!(t[1] < t[0], "{t:?}");
    }

    #[test]
    fn nested_loops_nest() {
        use Toward::Low;
        let outer = seg(0.0, 100.0, &[(0.0, Low), (100.0, Low)]);
        let inner = seg(40.0, 60.0, &[(40.0, Low), (60.0, Low)]);
        let (t, _) = assign(&[outer, inner]);
        assert!(t[1] < t[0], "inner loop hugs the node: {t:?}");
    }

    #[test]
    fn shared_nets_share_a_track() {
        use Toward::{High, Low};
        let a = TrackSeg { lo: 10.0, hi: 30.0, joins: vec![(10.0, Low), (30.0, High)], nets: [Some(7), None] };
        let b = TrackSeg { lo: 5.0, hi: 10.0, joins: vec![(10.0, Low), (5.0, High)], nets: [Some(7), None] };
        let c = seg(0.0, 40.0, &[(0.0, Low), (40.0, High)]);
        let (t, n) = assign(&[a, b, c]);
        assert_eq!(t[0], t[1]);
        assert_eq!(n, 2);
    }
}
