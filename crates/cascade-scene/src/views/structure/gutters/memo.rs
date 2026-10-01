//! Columns that survive edits.
//!
//! A gutter's nodes get fixed layout columns (`LayerConstraint::Exact`), so
//! the engine lines them up in one row in the order [`super::order`]
//! chose. Handing out columns by rank on every build would shift every node
//! right of an insertion, so the builder remembers them instead:
//!
//! - A node keeps the column it last had in its gutter (also after a trip
//!   to another gutter and back, so undoing an edit restores the picture).
//! - A node new to a gutter gets the next unused column there, in row
//!   order: it joins the right end of the row and moves nothing.
//! - A subscription inside a gutter must point right (both ends are fixed,
//!   and the engine rejects a fixed edge pointing backwards): a controller
//!   that would sit left of one of its events moves to the end.
//!
//! An empty memo (a fresh builder, or after `SceneBuilder::reset`) hands out
//! columns by rank, so a fresh layout is the tidy one.

use std::collections::{BTreeMap, HashMap};

/// One gutter's row for [`WiringMemo::columns`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Row {
    /// Stable identity of the gutter across builds.
    pub gutter: String,
    /// Node keys, left to right as the order wants them.
    pub nodes: Vec<String>,
    /// Pairs of indices into `nodes`: the first must end up left of the
    /// second.
    pub before: Vec<(usize, usize)>,
}

/// The columns handed out so far. Owned by the scene builder.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct WiringMemo {
    /// Per node key: its column in each gutter it has been in.
    columns: HashMap<String, BTreeMap<String, u32>>,
    /// Per gutter: the next column never handed out.
    next: HashMap<String, u32>,
}

impl WiringMemo {
    pub fn clear(&mut self) {
        self.columns.clear();
        self.next.clear();
    }

    /// Columns for every row's nodes (aligned with `rows[i].nodes`), and
    /// remember them.
    pub fn columns(&mut self, rows: &[Row]) -> Vec<Vec<u32>> {
        rows.iter().map(|row| self.row(row)).collect()
    }

    fn row(&mut self, row: &Row) -> Vec<u32> {
        let remembered = |key: &str| self.columns.get(key).and_then(|by_gutter| by_gutter.get(&row.gutter)).copied();
        let mut cols: Vec<Option<u32>> = Vec::with_capacity(row.nodes.len());
        let mut taken: Vec<u32> = Vec::new();
        for key in &row.nodes {
            // Two nodes remembering one column (possible after nodes came
            // and went): the first in row order keeps it.
            let col = remembered(key).filter(|c| !taken.contains(c));
            if let Some(c) = col {
                taken.push(c);
            }
            cols.push(col);
        }
        let mut next = self.next.get(&row.gutter).copied().unwrap_or(0);
        next = next.max(taken.iter().map(|c| c + 1).max().unwrap_or(0));
        let mut out: Vec<u32> = cols
            .into_iter()
            .map(|c| {
                c.unwrap_or_else(|| {
                    next += 1;
                    next - 1
                })
            })
            .collect();
        // Fixed edges must point right. Targets only ever move to the end,
        // past everything, so repeating until stable terminates.
        loop {
            let mut changed = false;
            for &(a, b) in &row.before {
                if let (Some(&ca), Some(&cb)) = (out.get(a), out.get(b))
                    && ca >= cb
                {
                    out[b] = next;
                    next += 1;
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        self.next.insert(row.gutter.clone(), next);
        for (key, &c) in row.nodes.iter().zip(&out) {
            self.columns.entry(key.clone()).or_default().insert(row.gutter.clone(), c);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(gutter: &str, nodes: &[&str], before: &[(usize, usize)]) -> Row {
        Row {
            gutter: gutter.to_owned(),
            nodes: nodes.iter().map(|s| (*s).to_owned()).collect(),
            before: before.to_vec(),
        }
    }

    #[test]
    fn a_fresh_memo_hands_out_ranks() {
        let mut memo = WiringMemo::default();
        assert_eq!(memo.columns(&[row("g", &["a", "b", "c"], &[])]), vec![vec![0, 1, 2]]);
    }

    #[test]
    fn remembered_nodes_keep_their_columns_and_new_ones_join_the_end() {
        let mut memo = WiringMemo::default();
        memo.columns(&[row("g", &["a", "b", "c"], &[])]);
        // `n` belongs between `a` and `b` by order, but moves nothing.
        assert_eq!(memo.columns(&[row("g", &["a", "n", "b", "c"], &[])]), vec![vec![0, 3, 1, 2]]);
        // Removing `b` leaves the others where they were.
        assert_eq!(memo.columns(&[row("g", &["a", "n", "c"], &[])]), vec![vec![0, 3, 2]]);
    }

    #[test]
    fn a_round_trip_through_another_gutter_restores_the_column() {
        let mut memo = WiringMemo::default();
        memo.columns(&[row("g", &["a", "b"], &[]), row("h", &["x"], &[])]);
        assert_eq!(memo.columns(&[row("g", &["a"], &[]), row("h", &["x", "b"], &[])]), vec![vec![0], vec![0, 1]]);
        assert_eq!(memo.columns(&[row("g", &["a", "b"], &[]), row("h", &["x"], &[])]), vec![vec![0, 1], vec![0]]);
    }

    #[test]
    fn a_column_is_never_shared() {
        let mut memo = WiringMemo::default();
        memo.columns(&[row("g", &["a", "b"], &[])]);
        memo.columns(&[row("g", &["a"], &[]), row("h", &["b"], &[])]);
        // `c` is new while `b` is away, and must not take `b`'s column.
        memo.columns(&[row("g", &["a", "c"], &[]), row("h", &["b"], &[])]);
        let cols = memo.columns(&[row("g", &["a", "c", "b"], &[])]);
        let mut sorted = cols[0].clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), 3, "{cols:?}");
    }

    #[test]
    fn subscriptions_point_right() {
        let mut memo = WiringMemo::default();
        memo.columns(&[row("g", &["ctl", "ev"], &[])]);
        // Now `ev` (index 1) feeds `ctl` (index 0): `ctl` moves to the end.
        let cols = memo.columns(&[row("g", &["ctl", "ev"], &[(1, 0)])]);
        assert!(cols[0][1] < cols[0][0], "{cols:?}");
        assert_eq!(cols[0][1], 1, "the event keeps its column");
    }

    #[test]
    fn clearing_forgets_everything() {
        let mut memo = WiringMemo::default();
        memo.columns(&[row("g", &["a", "b"], &[])]);
        memo.clear();
        assert_eq!(memo.columns(&[row("g", &["b", "a"], &[])]), vec![vec![0, 1]]);
    }
}
