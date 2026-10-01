//! Layout memoisation and stability bookkeeping.
//!
//! The key is the layout input itself: the [`LayoutGraph`] (node keys,
//! sizes, ports, groups, edges, label sizes), the options and the pins. Every
//! structural input (model, hidden machines, collapse, machine pair, the
//! visible set in hide mode) changes that graph, while emphasis (selection,
//! dimming, search, badges, diff outlines) never does, so a selection change
//! can never relayout. Comparing the graph itself rather than a hash rules
//! out stale layouts from collisions.
//!
//! A few recent layouts are kept per view so toggling a structural filter
//! off returns to exactly the picture before it. On a miss, the most recent
//! layout of the view made with the same options is fed back as
//! [`LayoutHints::previous`] (the causal view's lanes and its flat layout
//! do not seed each other).

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::Arc;

use cascade_layout::{LayoutError, LayoutGraph, LayoutHints, LayoutOptions, LayoutResult, Point, layout};

use crate::view_state::ViewKind;

/// Layouts remembered per view.
const MEMOS_PER_VIEW: usize = 4;

#[derive(Debug)]
struct Memo {
    graph: LayoutGraph,
    options: LayoutOptions,
    pins: BTreeMap<String, Point>,
    result: Arc<LayoutResult>,
}

#[derive(Debug, Default)]
pub(crate) struct LayoutCache {
    memos: HashMap<ViewKind, VecDeque<Memo>>,
    runs: u64,
}

impl LayoutCache {
    /// The layout of `graph`, from the cache when this exact input was laid
    /// out recently.
    pub(crate) fn layout(
        &mut self,
        view: ViewKind,
        graph: LayoutGraph,
        options: LayoutOptions,
        pins: BTreeMap<String, Point>,
    ) -> Result<Arc<LayoutResult>, LayoutError> {
        let memos = self.memos.entry(view).or_default();
        if let Some(pos) = memos.iter().position(|m| m.graph == graph && m.options == options && m.pins == pins)
            && let Some(memo) = memos.remove(pos)
        {
            let result = Arc::clone(&memo.result);
            memos.push_front(memo);
            return Ok(result);
        }
        // Only a layout made with the same options seeds this one: the
        // causal view's flat layout says nothing useful about its lanes.
        let previous = memos.iter().find(|m| m.options == options).map(|m| m.result.to_previous(&m.graph));
        let hints = LayoutHints { previous, pins: pins.clone() };
        let result = Arc::new(layout(&graph, &options, &hints)?);
        self.runs += 1;
        memos.push_front(Memo { graph, options, pins, result: Arc::clone(&result) });
        memos.truncate(MEMOS_PER_VIEW);
        Ok(result)
    }

    /// How many times the layout engine actually ran.
    pub(crate) fn runs(&self) -> u64 {
        self.runs
    }

    pub(crate) fn clear(&mut self) {
        self.memos.clear();
    }
}

#[cfg(test)]
mod tests {
    use cascade_layout::{LayoutNode, Size};

    use super::*;

    fn graph(keys: &[&str]) -> LayoutGraph {
        let mut g = LayoutGraph::new();
        for k in keys {
            g.add_node(LayoutNode::new(*k, Size::new(10.0, 10.0))).expect("node");
        }
        g
    }

    #[test]
    fn identical_inputs_hit_the_cache() {
        let mut cache = LayoutCache::default();
        let a = cache.layout(ViewKind::Causal, graph(&["a", "b"]), LayoutOptions::default(), BTreeMap::new());
        let b = cache.layout(ViewKind::Causal, graph(&["a", "b"]), LayoutOptions::default(), BTreeMap::new());
        assert_eq!(cache.runs(), 1);
        assert_eq!(a.expect("a"), b.expect("b"));
    }

    #[test]
    fn structural_changes_relayout_and_old_layouts_are_remembered() {
        let mut cache = LayoutCache::default();
        let full = cache.layout(ViewKind::Causal, graph(&["a", "b"]), LayoutOptions::default(), BTreeMap::new());
        let _ = cache.layout(ViewKind::Causal, graph(&["a"]), LayoutOptions::default(), BTreeMap::new());
        assert_eq!(cache.runs(), 2);
        let back = cache.layout(ViewKind::Causal, graph(&["a", "b"]), LayoutOptions::default(), BTreeMap::new());
        assert_eq!(cache.runs(), 2, "returning to an earlier input reuses its layout");
        assert_eq!(full.expect("full"), back.expect("back"));
        let pins: BTreeMap<String, Point> = [("a".to_owned(), Point::new(5.0, 5.0))].into_iter().collect();
        let _ = cache.layout(ViewKind::Causal, graph(&["a", "b"]), LayoutOptions::default(), pins);
        assert_eq!(cache.runs(), 3, "pins are part of the key");
        let _ = cache.layout(ViewKind::Structure, graph(&["a", "b"]), LayoutOptions::default(), BTreeMap::new());
        assert_eq!(cache.runs(), 4, "views are cached separately");
        cache.clear();
        let _ = cache.layout(ViewKind::Causal, graph(&["a", "b"]), LayoutOptions::default(), BTreeMap::new());
        assert_eq!(cache.runs(), 5);
    }
}
