//! The interaction model: what is selected, which part of the causal graph
//! is in focus (a cone or a path query), and what matches the search.
//!
//! Pure: [`Interaction::new`] reads the [`ViewState`], the model and the
//! causal graph, and views ask it how to emphasise each item. Items name the
//! elements they stand for and an [`Anchor`] that ties them to the causal
//! graph; the interaction answers with an [`Emphasis`], or whether the item
//! is hidden.
//!
//! Precedence: selected, then dimmed (outside the focus), then search match,
//! then focused (inside the focus), then normal. Selected items always
//! count as inside, so hiding never removes the selection.

mod seeds;

use std::collections::BTreeSet;

use cascade_core::search::search;
use cascade_core::{CausalGraph, Cone, EdgeIx, ElementRef, Model, NodeIx};

pub use seeds::causal_seeds;

use crate::scene::Emphasis;
use crate::view_state::{ConeFocus, OutsideFocus, ViewState};

/// How a scene item relates to the causal graph, for cone and path focus.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Anchor {
    /// Not part of any focus (headers, notes): never dimmed or hidden.
    #[default]
    Free,
    /// Inside when any of these causal nodes is inside. An empty list is
    /// always outside an active focus.
    Nodes(Vec<NodeIx>),
    /// Inside when every edge of at least one chain is inside (a direct
    /// edge is a chain of one; a transition-to-transition link is
    /// emit, subscribe, fire).
    Chains(Vec<Vec<EdgeIx>>),
}

/// Which kind of focus is active.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FocusKind {
    /// Forward or backward cone from the one selected element.
    Cone(ConeFocus),
    /// Every causal path between the two selected elements.
    Path,
}

/// An active cone or path query.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FocusRegion {
    pub kind: FocusKind,
    /// The causal nodes and edges in focus.
    pub region: Cone,
    /// What happens to everything else.
    pub outside: OutsideFocus,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Interaction {
    selected: Vec<ElementRef>,
    focus: Option<FocusRegion>,
    search: BTreeSet<ElementRef>,
    notes: Vec<String>,
}

impl Interaction {
    /// Derive the interaction from the view state. Selection keys that do
    /// not name an element of `model` are ignored; at most two selections
    /// count.
    pub fn new(model: &Model, graph: &CausalGraph, view: &ViewState) -> Self {
        let selected: Vec<ElementRef> = view.selection.iter().filter_map(|k| model.resolve_key(k)).take(2).collect();
        let mut notes = Vec::new();
        let focus = match (selected.as_slice(), view.cone) {
            ([a, b], _) => {
                let region = path_region(model, graph, *a, *b);
                if region.is_empty() {
                    notes.push(format!("No causal path between {} and {}.", model.label_of(*a), model.label_of(*b)));
                }
                Some(FocusRegion { kind: FocusKind::Path, region, outside: view.outside })
            }
            ([one], Some(cone)) => {
                let seeds = causal_seeds(model, graph, *one);
                let region = graph.cone(&seeds, cone.direction, cone.depth);
                Some(FocusRegion { kind: FocusKind::Cone(cone), region, outside: view.outside })
            }
            _ => None,
        };
        let search = match view.search.as_deref().map(str::trim) {
            Some(q) if !q.is_empty() => {
                search(model, q, usize::MAX).into_iter().map(|hit| hit.element).collect::<BTreeSet<_>>()
            }
            _ => BTreeSet::new(),
        };
        Self { selected, focus, search, notes }
    }

    /// An interaction with nothing selected, focused or searched.
    pub fn none() -> Self {
        Self { selected: Vec::new(), focus: None, search: BTreeSet::new(), notes: Vec::new() }
    }

    /// The resolved selection, in selection order.
    pub fn selected(&self) -> &[ElementRef] {
        &self.selected
    }

    pub fn focus(&self) -> Option<&FocusRegion> {
        self.focus.as_ref()
    }

    /// Messages for the host, e.g. an empty path query.
    pub fn notes(&self) -> &[String] {
        &self.notes
    }

    /// Whether items outside the focus are removed rather than dimmed.
    pub fn hides_outside(&self) -> bool {
        self.focus.as_ref().is_some_and(|f| f.outside == OutsideFocus::Hide)
    }

    pub fn is_selected(&self, elements: &[ElementRef]) -> bool {
        elements.iter().any(|e| self.selected.contains(e))
    }

    pub fn is_search_match(&self, elements: &[ElementRef]) -> bool {
        elements.iter().any(|e| self.search.contains(e))
    }

    /// Whether an item is inside the focus; `None` when no focus is active
    /// or the item is [`Anchor::Free`].
    pub fn inside(&self, anchor: &Anchor) -> Option<bool> {
        let focus = self.focus.as_ref()?;
        match anchor {
            Anchor::Free => None,
            Anchor::Nodes(nodes) => Some(nodes.iter().any(|&n| focus.region.contains(n))),
            Anchor::Chains(chains) => Some(
                chains.iter().any(|chain| !chain.is_empty() && chain.iter().all(|&e| focus.region.contains_edge(e))),
            ),
        }
    }

    /// The emphasis of an item standing for `elements`.
    pub fn emphasis(&self, elements: &[ElementRef], anchor: &Anchor) -> Emphasis {
        if self.is_selected(elements) {
            return Emphasis::Selected;
        }
        let inside = self.inside(anchor);
        if inside == Some(false) {
            Emphasis::Dimmed
        } else if self.is_search_match(elements) {
            Emphasis::SearchMatch
        } else if inside == Some(true) {
            Emphasis::Focused
        } else {
            Emphasis::Normal
        }
    }

    /// Whether an item is removed: outside the focus in hide mode, and not
    /// selected.
    pub fn is_hidden(&self, elements: &[ElementRef], anchor: &Anchor) -> bool {
        self.hides_outside() && !self.is_selected(elements) && self.inside(anchor) == Some(false)
    }
}

/// Every causal path between any seed of `a` and any seed of `b`.
fn path_region(model: &Model, graph: &CausalGraph, a: ElementRef, b: ElementRef) -> Cone {
    let seeds_a = causal_seeds(model, graph, a);
    let seeds_b = causal_seeds(model, graph, b);
    // An empty cone of the right size: a cone from no seeds.
    let mut region = graph.cone(&[], cascade_core::Direction::Forward, Some(0));
    for &x in &seeds_a {
        for &y in &seeds_b {
            if x != y {
                region = region.union(&graph.paths_between(x, y));
            }
        }
    }
    region
}

#[cfg(test)]
pub(crate) mod tests {
    use std::collections::BTreeSet;

    use cascade_core::{CausalGraph, Direction, ElementKey, Model, NodeIx, load_str};

    use super::*;

    /// The causal-graph test chain:
    ///
    /// ```text
    /// User ─▶ A:a0→a1 ─(Go)─▶ C1 ─▶ B:b0→b1 ─(Done)─▶ C2 ─▶ A:a1→a0 ─(Back)─▶ C3 ─▶ B:b1→b0
    ///                                                     └──────────────▶ D:d0→d1
    /// ```
    pub(crate) const CHAIN: &str = r#"
machines:
  A:
    states: [a0, a1]
    transitions:
      - { from: a0, to: a1, on: go, emits: [Go] }
      - { from: a1, to: a0, on: reset, emits: [Back] }
  B:
    states: [b0, b1]
    transitions:
      - { from: b0, to: b1, on: start, emits: [Done] }
      - { from: b1, to: b0, on: rewind }
  D:
    states: [d0, d1]
    transitions:
      - { from: d0, to: d1, on: note }
controllers:
  C1:
    on:
      Go: [{ fire: B.start }]
  C2:
    on:
      Done: [{ fire: A.reset }, { fire: D.note }]
  C3:
    on:
      Back: [{ fire: B.rewind }]
external:
  User: [A.go]
"#;

    pub(crate) fn load(text: &str) -> Model {
        match load_str(text) {
            Ok(m) => m,
            Err(err) => panic!("{err}"),
        }
    }

    pub(crate) fn node_labels(
        model: &Model,
        graph: &CausalGraph,
        nodes: impl IntoIterator<Item = NodeIx>,
    ) -> Vec<String> {
        let mut out: Vec<String> = nodes.into_iter().map(|n| model.label_of(graph.node(n).element())).collect();
        out.sort();
        out
    }

    fn key(s: &str) -> ElementKey {
        s.parse().expect("key")
    }

    fn view(selection: &[&str], cone: Option<ConeFocus>) -> ViewState {
        ViewState { selection: selection.iter().map(|s| key(s)).collect(), cone, ..ViewState::default() }
    }

    /// Labels of the causal nodes inside the focus.
    fn inside_labels(model: &Model, graph: &CausalGraph, interaction: &Interaction) -> Vec<String> {
        let nodes =
            graph.nodes().map(|(ix, _)| ix).filter(|&ix| interaction.inside(&Anchor::Nodes(vec![ix])) == Some(true));
        node_labels(model, graph, nodes)
    }

    #[test]
    fn no_focus_without_cone_or_second_selection() {
        let model = load(CHAIN);
        let graph = CausalGraph::build(&model);
        let i = Interaction::new(&model, &graph, &view(&["transition:A:a0->a1@go"], None));
        assert!(i.focus().is_none());
        assert_eq!(i.inside(&Anchor::Nodes(vec![])), None);
        let a = model.resolve_key(&key("transition:A:a0->a1@go")).expect("t");
        assert_eq!(i.emphasis(&[a], &Anchor::Free), Emphasis::Selected);
        let go = model.resolve_key(&key("event:Go")).expect("e");
        assert_eq!(i.emphasis(&[go], &Anchor::Nodes(vec![])), Emphasis::Normal);
    }

    #[test]
    fn forward_cone_with_depth_limits_the_focus() {
        let model = load(CHAIN);
        let graph = CausalGraph::build(&model);
        let cone = ConeFocus { direction: Direction::Forward, depth: Some(1) };
        let i = Interaction::new(&model, &graph, &view(&["transition:A:a0->a1@go"], Some(cone)));
        assert_eq!(i.focus().map(|f| f.kind), Some(FocusKind::Cone(cone)));
        assert_eq!(
            inside_labels(&model, &graph, &i),
            ["A: a0 → a1", "B: b0 → b1", "C1 on Go", "C2 on Done", "Done", "Go"]
        );
        let user = model.resolve_key(&key("external:User")).expect("user");
        let user_ix = graph.ix_of_element(user).expect("ix");
        assert_eq!(i.emphasis(&[user], &Anchor::Nodes(vec![user_ix])), Emphasis::Dimmed);
        let go = model.resolve_key(&key("event:Go")).expect("go");
        let go_ix = graph.ix_of_element(go).expect("ix");
        assert_eq!(i.emphasis(&[go], &Anchor::Nodes(vec![go_ix])), Emphasis::Focused);
    }

    #[test]
    fn unlimited_backward_cone_reaches_sources() {
        let model = load(CHAIN);
        let graph = CausalGraph::build(&model);
        let cone = ConeFocus { direction: Direction::Backward, depth: None };
        let i = Interaction::new(&model, &graph, &view(&["transition:D:d0->d1@note"], Some(cone)));
        assert_eq!(
            inside_labels(&model, &graph, &i),
            ["A: a0 → a1", "B: b0 → b1", "C1 on Go", "C2 on Done", "D: d0 → d1", "Done", "Go", "User"]
        );
        let limited = Interaction::new(
            &model,
            &graph,
            &view(&["transition:D:d0->d1@note"], Some(ConeFocus { direction: Direction::Backward, depth: Some(1) })),
        );
        assert_eq!(
            inside_labels(&model, &graph, &limited),
            // One hop back reaches B:b0→b1 plus the event and handler that
            // would set it off.
            ["B: b0 → b1", "C1 on Go", "C2 on Done", "D: d0 → d1", "Done", "Go"]
        );
    }

    #[test]
    fn two_selections_make_a_path_query() {
        let model = load(CHAIN);
        let graph = CausalGraph::build(&model);
        // The cone setting is ignored when two elements are selected.
        let cone = ConeFocus { direction: Direction::Backward, depth: Some(0) };
        let i = Interaction::new(
            &model,
            &graph,
            &view(&["transition:D:d0->d1@note", "transition:A:a0->a1@go"], Some(cone)),
        );
        assert_eq!(i.focus().map(|f| f.kind), Some(FocusKind::Path));
        assert_eq!(
            inside_labels(&model, &graph, &i),
            ["A: a0 → a1", "B: b0 → b1", "C1 on Go", "C2 on Done", "D: d0 → d1", "Done", "Go"]
        );
        assert!(i.notes().is_empty());

        let none =
            Interaction::new(&model, &graph, &view(&["transition:D:d0->d1@note", "transition:B:b1->b0@rewind"], None));
        assert!(inside_labels(&model, &graph, &none).is_empty());
        assert_eq!(none.notes().len(), 1);
        // Both endpoints stay selected even with no path between them.
        let d = model.resolve_key(&key("transition:D:d0->d1@note")).expect("d");
        let d_ix = graph.ix_of_element(d).expect("ix");
        assert_eq!(none.emphasis(&[d], &Anchor::Nodes(vec![d_ix])), Emphasis::Selected);
    }

    #[test]
    fn chains_need_every_edge_inside() {
        let model = load(CHAIN);
        let graph = CausalGraph::build(&model);
        let cone = ConeFocus { direction: Direction::Forward, depth: Some(1) };
        let i = Interaction::new(&model, &graph, &view(&["transition:A:a0->a1@go"], Some(cone)));
        let region = &i.focus().expect("focus").region;
        let inside: Vec<EdgeIx> = region.edges().collect();
        let outside: Vec<EdgeIx> = graph.edges().map(|(e, _)| e).filter(|e| !region.contains_edge(*e)).collect();
        assert!(!inside.is_empty() && !outside.is_empty());
        assert_eq!(i.inside(&Anchor::Chains(vec![inside.clone()])), Some(true));
        assert_eq!(i.inside(&Anchor::Chains(vec![vec![inside[0], outside[0]]])), Some(false));
        assert_eq!(i.inside(&Anchor::Chains(vec![vec![outside[0]], vec![inside[0]]])), Some(true));
        assert_eq!(i.inside(&Anchor::Chains(vec![vec![]])), Some(false));
        assert_eq!(i.inside(&Anchor::Free), None);
    }

    #[test]
    fn hide_mode_hides_outside_items_but_never_the_selection() {
        let model = load(CHAIN);
        let graph = CausalGraph::build(&model);
        let mut state =
            view(&["transition:A:a0->a1@go"], Some(ConeFocus { direction: Direction::Forward, depth: Some(0) }));
        state.outside = OutsideFocus::Hide;
        let i = Interaction::new(&model, &graph, &state);
        assert!(i.hides_outside());
        let user = model.resolve_key(&key("external:User")).expect("user");
        let user_ix = graph.ix_of_element(user).expect("ix");
        assert!(i.is_hidden(&[user], &Anchor::Nodes(vec![user_ix])));
        assert!(!i.is_hidden(&[user], &Anchor::Free));
        let a = model.resolve_key(&key("transition:A:a0->a1@go")).expect("a");
        assert!(!i.is_hidden(&[a], &Anchor::Nodes(vec![])));
    }

    #[test]
    fn search_matches_are_emphasised_but_selection_and_dimming_win() {
        let model = load(CHAIN);
        let graph = CausalGraph::build(&model);
        let state = ViewState { search: Some("done".into()), ..ViewState::default() };
        let i = Interaction::new(&model, &graph, &state);
        let done = model.resolve_key(&key("event:Done")).expect("done");
        let go = model.resolve_key(&key("event:Go")).expect("go");
        assert_eq!(i.emphasis(&[done], &Anchor::Free), Emphasis::SearchMatch);
        assert_eq!(i.emphasis(&[go], &Anchor::Free), Emphasis::Normal);

        let mut focused = state.clone();
        focused.selection = vec![key("external:User")];
        focused.cone = Some(ConeFocus { direction: Direction::Forward, depth: Some(1) });
        let i = Interaction::new(&model, &graph, &focused);
        let done_ix = graph.ix_of_element(done).expect("ix");
        assert_eq!(i.emphasis(&[done], &Anchor::Nodes(vec![done_ix])), Emphasis::Dimmed);
        let blank = Interaction::new(&model, &graph, &ViewState { search: Some("  ".into()), ..ViewState::default() });
        assert_eq!(blank.emphasis(&[done], &Anchor::Free), Emphasis::Normal);
    }

    #[test]
    fn unknown_selection_keys_are_ignored_and_at_most_two_count() {
        let model = load(CHAIN);
        let graph = CausalGraph::build(&model);
        let i = Interaction::new(
            &model,
            &graph,
            &view(
                &["event:Nope", "event:Go", "event:Done", "event:Back"],
                Some(ConeFocus { direction: Direction::Forward, depth: None }),
            ),
        );
        let selected: BTreeSet<String> = i.selected().iter().map(|e| model.label_of(*e)).collect();
        assert_eq!(selected, ["Done".to_owned(), "Go".to_owned()].into_iter().collect());
        assert_eq!(i.focus().map(|f| f.kind), Some(FocusKind::Path));
    }

    #[test]
    fn selecting_a_controller_traces_all_its_handlers() {
        let model = load(CHAIN);
        let graph = CausalGraph::build(&model);
        let i = Interaction::new(
            &model,
            &graph,
            &view(&["controller:C3"], Some(ConeFocus { direction: Direction::Forward, depth: None })),
        );
        assert_eq!(inside_labels(&model, &graph, &i), ["B: b1 → b0", "C3 on Back"]);
    }
}
