//! Keyboard commands: the key map and how each command changes the view.
//!
//! Pure: no GPUI. `workspace::actions` turns [`KEYMAP`] into GPUI key
//! bindings and routes each action back to [`reduce`]. Commands that need
//! the host (clipboard, viewport, editor) come back as [`HostEffect`]s.

use cascade_core::{Direction, ElementKey, ElementKind};
use cascade_scene::{ConeFocus, OutsideFocus, ViewKind, ViewState};

/// Deepest finite cone depth the stepper offers; one more step is ∞.
pub const MAX_CONE_DEPTH: u32 = 12;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Command {
    /// Esc: clear the selection and cone, then the machine pair, then the
    /// search, one layer per press.
    ClearSelection,
    ConeForward,
    ConeBackward,
    /// Dim or hide what is outside the cone or path query.
    ToggleOutside,
    DepthLess,
    DepthMore,
    ShowView(ViewKind),
    ZoomIn,
    ZoomOut,
    FitView,
    PanLeft,
    PanRight,
    PanUp,
    PanDown,
    FocusSearch,
    CopyLink,
    PasteLink,
    ToggleTheme,
    OpenSource,
    UnpinSelection,
    Reload,
    Quit,
}

/// Where a binding applies.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Scope {
    /// Plain keys: only while no text field has focus.
    Canvas,
    /// Chords with a modifier: everywhere in the window.
    Global,
}

impl Scope {
    /// The GPUI key context predicate for this scope.
    pub const fn context(self) -> &'static str {
        match self {
            Scope::Canvas => "Workspace && !TextInput",
            Scope::Global => "Workspace",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Binding {
    /// GPUI keystroke syntax.
    pub keys: &'static str,
    pub command: Command,
    pub scope: Scope,
}

const fn canvas(keys: &'static str, command: Command) -> Binding {
    Binding { keys, command, scope: Scope::Canvas }
}

const fn global(keys: &'static str, command: Command) -> Binding {
    Binding { keys, command, scope: Scope::Global }
}

/// Every key binding of the workspace. Text fields have their own bindings
/// in `input.rs`.
pub const KEYMAP: &[Binding] = &[
    canvas("escape", Command::ClearSelection),
    canvas("f", Command::ConeForward),
    canvas("b", Command::ConeBackward),
    canvas("h", Command::ToggleOutside),
    canvas("[", Command::DepthLess),
    canvas("]", Command::DepthMore),
    canvas("1", Command::ShowView(ViewKind::Causal)),
    canvas("2", Command::ShowView(ViewKind::Structure)),
    canvas("3", Command::ShowView(ViewKind::Trace)),
    canvas("4", Command::ShowView(ViewKind::Matrix)),
    canvas("+", Command::ZoomIn),
    canvas("=", Command::ZoomIn),
    canvas("-", Command::ZoomOut),
    canvas("0", Command::FitView),
    canvas("left", Command::PanLeft),
    canvas("right", Command::PanRight),
    canvas("up", Command::PanUp),
    canvas("down", Command::PanDown),
    canvas("/", Command::FocusSearch),
    canvas("o", Command::OpenSource),
    canvas("u", Command::UnpinSelection),
    global("ctrl-f", Command::FocusSearch),
    global("cmd-f", Command::FocusSearch),
    global("ctrl-shift-c", Command::CopyLink),
    global("cmd-shift-c", Command::CopyLink),
    global("ctrl-shift-v", Command::PasteLink),
    global("cmd-shift-v", Command::PasteLink),
    global("ctrl-shift-t", Command::ToggleTheme),
    global("cmd-shift-t", Command::ToggleTheme),
    global("ctrl-r", Command::Reload),
    global("cmd-r", Command::Reload),
    global("ctrl-q", Command::Quit),
    global("cmd-q", Command::Quit),
];

/// Every command, for exhaustiveness checks.
#[cfg(test)]
pub const ALL_COMMANDS: &[Command] = &[
    Command::ClearSelection,
    Command::ConeForward,
    Command::ConeBackward,
    Command::ToggleOutside,
    Command::DepthLess,
    Command::DepthMore,
    Command::ShowView(ViewKind::Causal),
    Command::ShowView(ViewKind::Structure),
    Command::ShowView(ViewKind::Trace),
    Command::ShowView(ViewKind::Matrix),
    Command::ZoomIn,
    Command::ZoomOut,
    Command::FitView,
    Command::PanLeft,
    Command::PanRight,
    Command::PanUp,
    Command::PanDown,
    Command::FocusSearch,
    Command::CopyLink,
    Command::PasteLink,
    Command::ToggleTheme,
    Command::OpenSource,
    Command::UnpinSelection,
    Command::Reload,
    Command::Quit,
];

/// The first key bound to `command`, for button hints.
pub fn key_hint(command: Command) -> Option<&'static str> {
    KEYMAP.iter().find(|b| b.command == command).map(|b| b.keys)
}

/// Work only the host can do after a command.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum HostEffect {
    /// Multiply the zoom, anchored at the canvas centre.
    Zoom(f32),
    Fit,
    /// Pan by a screen-space delta.
    Pan(f32, f32),
    FocusSearch,
    CopyLink,
    PasteLink,
    ToggleTheme,
    OpenSource,
    UnpinSelection,
    Reload,
    Quit,
}

/// What a command did.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Outcome {
    /// Nothing to do (e.g. a cone with nothing selected).
    Unchanged,
    /// The view state changed; the scene must be rebuilt.
    ViewChanged,
    Host(HostEffect),
}

/// Apply `command` to the view state. `depth` is the cone depth setting,
/// which survives turning the cone off and on.
pub fn reduce(command: Command, state: &mut ViewState, depth: &mut Option<u32>) -> Outcome {
    use crate::viewport::{KEY_PAN_STEP, KEY_ZOOM_STEP};
    match command {
        Command::ClearSelection => changed(clear_layer(state)),
        Command::ConeForward => changed(toggle_cone(state, Direction::Forward, *depth)),
        Command::ConeBackward => changed(toggle_cone(state, Direction::Backward, *depth)),
        Command::ToggleOutside => {
            state.outside = match state.outside {
                OutsideFocus::Dim => OutsideFocus::Hide,
                OutsideFocus::Hide => OutsideFocus::Dim,
            };
            Outcome::ViewChanged
        }
        Command::DepthLess => changed(set_depth(state, depth, depth_less(*depth))),
        Command::DepthMore => changed(set_depth(state, depth, depth_more(*depth))),
        Command::ShowView(view) => changed(show_view(state, view)),
        Command::ZoomIn => Outcome::Host(HostEffect::Zoom(KEY_ZOOM_STEP)),
        Command::ZoomOut => Outcome::Host(HostEffect::Zoom(1.0 / KEY_ZOOM_STEP)),
        Command::FitView => Outcome::Host(HostEffect::Fit),
        Command::PanLeft => Outcome::Host(HostEffect::Pan(KEY_PAN_STEP, 0.0)),
        Command::PanRight => Outcome::Host(HostEffect::Pan(-KEY_PAN_STEP, 0.0)),
        Command::PanUp => Outcome::Host(HostEffect::Pan(0.0, KEY_PAN_STEP)),
        Command::PanDown => Outcome::Host(HostEffect::Pan(0.0, -KEY_PAN_STEP)),
        Command::FocusSearch => Outcome::Host(HostEffect::FocusSearch),
        Command::CopyLink => Outcome::Host(HostEffect::CopyLink),
        Command::PasteLink => Outcome::Host(HostEffect::PasteLink),
        Command::ToggleTheme => Outcome::Host(HostEffect::ToggleTheme),
        Command::OpenSource => Outcome::Host(HostEffect::OpenSource),
        Command::UnpinSelection => Outcome::Host(HostEffect::UnpinSelection),
        Command::Reload => Outcome::Host(HostEffect::Reload),
        Command::Quit => Outcome::Host(HostEffect::Quit),
    }
}

fn changed(did_change: bool) -> Outcome {
    if did_change { Outcome::ViewChanged } else { Outcome::Unchanged }
}

/// Switch views. The selection is shared, so only the view changes.
pub fn show_view(state: &mut ViewState, view: ViewKind) -> bool {
    let did_change = state.view != view;
    state.view = view;
    did_change
}

/// Esc peels one layer: selection and cone, then the machine pair filter,
/// then the search highlight.
fn clear_layer(state: &mut ViewState) -> bool {
    if !state.selection.is_empty() || state.cone.is_some() {
        state.selection.clear();
        state.cone = None;
        true
    } else if state.machine_pair.is_some() {
        state.machine_pair = None;
        true
    } else if state.search.is_some() {
        state.search = None;
        true
    } else {
        false
    }
}

/// Set the cone in `direction`, or turn it off when it is already set in
/// that direction. Needs a selection.
fn toggle_cone(state: &mut ViewState, direction: Direction, depth: Option<u32>) -> bool {
    if state.selection.is_empty() {
        return false;
    }
    state.cone = match state.cone {
        Some(cone) if cone.direction == direction => None,
        _ => Some(ConeFocus { direction, depth }),
    };
    true
}

fn set_depth(state: &mut ViewState, depth: &mut Option<u32>, next: Option<u32>) -> bool {
    if *depth == next {
        return false;
    }
    *depth = next;
    if let Some(cone) = &mut state.cone {
        cone.depth = next;
    }
    true
}

/// One step deeper; past [`MAX_CONE_DEPTH`] is unlimited (`None`).
pub fn depth_more(depth: Option<u32>) -> Option<u32> {
    match depth {
        None => None,
        Some(n) if n >= MAX_CONE_DEPTH => None,
        Some(n) => Some(n + 1),
    }
}

/// One step shallower; unlimited steps down to [`MAX_CONE_DEPTH`].
pub fn depth_less(depth: Option<u32>) -> Option<u32> {
    match depth {
        None => Some(MAX_CONE_DEPTH),
        Some(n) => Some(n.saturating_sub(1)),
    }
}

/// `∞` or the hop count.
pub fn depth_label(depth: Option<u32>) -> String {
    depth.map_or_else(|| "∞".to_owned(), |d| d.to_string())
}

/// Plain click: select only `key`. The cone, if any, follows the new
/// selection.
pub fn select(state: &mut ViewState, key: ElementKey) -> bool {
    if state.selection.len() == 1 && state.selection[0] == key {
        return false;
    }
    state.selection = vec![key];
    true
}

/// Shift-click: toggle `key` in a selection of at most two. Two selected
/// elements form a path query, which replaces any cone.
pub fn add_to_selection(state: &mut ViewState, key: ElementKey) {
    if let Some(pos) = state.selection.iter().position(|k| *k == key) {
        state.selection.remove(pos);
    } else if state.selection.len() < 2 {
        state.selection.push(key);
    } else {
        state.selection.truncate(1);
        state.selection.push(key);
    }
    if state.selection.len() != 1 {
        state.cone = None;
    }
}

/// Whether the selection is a path query (two transitions).
pub fn is_path_query(state: &ViewState) -> bool {
    state.selection.len() == 2 && state.selection.iter().all(|k| k.kind() == ElementKind::Transition)
}

/// Toggle a machine in the legend.
pub fn toggle_machine(state: &mut ViewState, machine: &str) {
    if !state.hidden_machines.remove(machine) {
        state.hidden_machines.insert(machine.to_owned());
    }
}

/// A matrix cell click: the causal view restricted to the two machines.
pub fn open_pair(state: &mut ViewState, row: String, column: String) {
    state.view = ViewKind::Causal;
    state.machine_pair = Some((row, column));
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    fn key(s: &str) -> ElementKey {
        s.parse().expect("valid key")
    }

    fn t1() -> ElementKey {
        key("transition:Order:pending->paid@capture_ok")
    }

    fn t2() -> ElementKey {
        key("transition:Shipment:idle->picking@start")
    }

    #[test]
    fn keymap_has_no_duplicate_keys_per_scope() {
        let mut seen = HashSet::new();
        for b in KEYMAP {
            assert!(seen.insert((b.keys, b.scope)), "duplicate binding {}", b.keys);
        }
    }

    #[test]
    fn every_command_is_bound() {
        for command in ALL_COMMANDS {
            assert!(key_hint(*command).is_some(), "{command:?} has no key");
        }
    }

    #[test]
    fn plain_keys_are_canvas_scoped_and_chords_are_global() {
        for b in KEYMAP {
            let chord = b.keys.contains("ctrl-") || b.keys.contains("cmd-");
            assert_eq!(chord, b.scope == Scope::Global, "{}", b.keys);
        }
    }

    #[test]
    fn spec_keys_are_bound() {
        let find = |keys: &str| KEYMAP.iter().find(|b| b.keys == keys).map(|b| b.command);
        assert_eq!(find("f"), Some(Command::ConeForward));
        assert_eq!(find("b"), Some(Command::ConeBackward));
        assert_eq!(find("h"), Some(Command::ToggleOutside));
        assert_eq!(find("escape"), Some(Command::ClearSelection));
        assert_eq!(find("0"), Some(Command::FitView));
        assert_eq!(find("/"), Some(Command::FocusSearch));
        assert_eq!(find("3"), Some(Command::ShowView(ViewKind::Trace)));
    }

    #[test]
    fn cone_needs_a_selection() {
        let mut state = ViewState::default();
        let mut depth = None;
        assert_eq!(reduce(Command::ConeForward, &mut state, &mut depth), Outcome::Unchanged);
        assert_eq!(state.cone, None);
    }

    #[test]
    fn cone_keys_set_switch_and_toggle() {
        let mut state = ViewState { selection: vec![t1()], ..ViewState::default() };
        let mut depth = Some(2);
        assert_eq!(reduce(Command::ConeForward, &mut state, &mut depth), Outcome::ViewChanged);
        assert_eq!(state.cone, Some(ConeFocus { direction: Direction::Forward, depth: Some(2) }));
        reduce(Command::ConeBackward, &mut state, &mut depth);
        assert_eq!(state.cone, Some(ConeFocus { direction: Direction::Backward, depth: Some(2) }));
        reduce(Command::ConeBackward, &mut state, &mut depth);
        assert_eq!(state.cone, None);
    }

    #[test]
    fn depth_steps_through_infinity() {
        assert_eq!(depth_more(Some(0)), Some(1));
        assert_eq!(depth_more(Some(MAX_CONE_DEPTH)), None);
        assert_eq!(depth_more(None), None);
        assert_eq!(depth_less(None), Some(MAX_CONE_DEPTH));
        assert_eq!(depth_less(Some(1)), Some(0));
        assert_eq!(depth_less(Some(0)), Some(0));
        assert_eq!(depth_label(None), "∞");
        assert_eq!(depth_label(Some(3)), "3");
    }

    #[test]
    fn depth_keys_update_an_active_cone() {
        let mut state = ViewState { selection: vec![t1()], ..ViewState::default() };
        let mut depth = None;
        reduce(Command::ConeForward, &mut state, &mut depth);
        assert_eq!(reduce(Command::DepthLess, &mut state, &mut depth), Outcome::ViewChanged);
        assert_eq!(depth, Some(MAX_CONE_DEPTH));
        assert_eq!(state.cone.map(|c| c.depth), Some(Some(MAX_CONE_DEPTH)));
        assert_eq!(reduce(Command::DepthMore, &mut state, &mut depth), Outcome::ViewChanged);
        assert_eq!(state.cone.map(|c| c.depth), Some(None));
        assert_eq!(reduce(Command::DepthMore, &mut state, &mut depth), Outcome::Unchanged);
    }

    #[test]
    fn depth_is_remembered_without_a_cone() {
        let mut state = ViewState::default();
        let mut depth = None;
        reduce(Command::DepthLess, &mut state, &mut depth);
        reduce(Command::DepthLess, &mut state, &mut depth);
        assert_eq!(depth, Some(MAX_CONE_DEPTH - 1));
        assert_eq!(state.cone, None);
        state.selection = vec![t1()];
        reduce(Command::ConeForward, &mut state, &mut depth);
        assert_eq!(state.cone.and_then(|c| c.depth), Some(MAX_CONE_DEPTH - 1));
    }

    #[test]
    fn outside_toggles_between_dim_and_hide() {
        let mut state = ViewState::default();
        let mut depth = None;
        reduce(Command::ToggleOutside, &mut state, &mut depth);
        assert_eq!(state.outside, OutsideFocus::Hide);
        reduce(Command::ToggleOutside, &mut state, &mut depth);
        assert_eq!(state.outside, OutsideFocus::Dim);
    }

    #[test]
    fn escape_peels_one_layer_at_a_time() {
        let mut state = ViewState {
            selection: vec![t1()],
            cone: Some(ConeFocus { direction: Direction::Forward, depth: None }),
            machine_pair: Some(("Order".into(), "Shipment".into())),
            search: Some("paid".into()),
            ..ViewState::default()
        };
        let mut depth = None;
        assert_eq!(reduce(Command::ClearSelection, &mut state, &mut depth), Outcome::ViewChanged);
        assert!(state.selection.is_empty() && state.cone.is_none());
        assert!(state.machine_pair.is_some());
        reduce(Command::ClearSelection, &mut state, &mut depth);
        assert!(state.machine_pair.is_none());
        assert!(state.search.is_some());
        reduce(Command::ClearSelection, &mut state, &mut depth);
        assert!(state.search.is_none());
        assert_eq!(reduce(Command::ClearSelection, &mut state, &mut depth), Outcome::Unchanged);
    }

    #[test]
    fn view_keys_keep_the_selection() {
        let mut state = ViewState { selection: vec![t1()], ..ViewState::default() };
        let mut depth = None;
        assert_eq!(reduce(Command::ShowView(ViewKind::Matrix), &mut state, &mut depth), Outcome::ViewChanged);
        assert_eq!(state.view, ViewKind::Matrix);
        assert_eq!(state.selection, vec![t1()]);
        assert_eq!(reduce(Command::ShowView(ViewKind::Matrix), &mut state, &mut depth), Outcome::Unchanged);
    }

    #[test]
    fn host_commands_do_not_touch_the_view_state() {
        let mut state = ViewState::default();
        let mut depth = None;
        assert!(
            matches!(reduce(Command::ZoomIn, &mut state, &mut depth), Outcome::Host(HostEffect::Zoom(f)) if f > 1.0)
        );
        assert!(
            matches!(reduce(Command::ZoomOut, &mut state, &mut depth), Outcome::Host(HostEffect::Zoom(f)) if f < 1.0)
        );
        assert_eq!(reduce(Command::FitView, &mut state, &mut depth), Outcome::Host(HostEffect::Fit));
        assert_eq!(reduce(Command::CopyLink, &mut state, &mut depth), Outcome::Host(HostEffect::CopyLink));
        assert!(
            matches!(reduce(Command::PanLeft, &mut state, &mut depth), Outcome::Host(HostEffect::Pan(dx, _)) if dx > 0.0)
        );
        assert_eq!(state, ViewState::default());
    }

    #[test]
    fn select_replaces_and_keeps_the_cone() {
        let mut state = ViewState {
            selection: vec![t1()],
            cone: Some(ConeFocus { direction: Direction::Forward, depth: None }),
            ..ViewState::default()
        };
        assert!(select(&mut state, t2()));
        assert_eq!(state.selection, vec![t2()]);
        assert!(state.cone.is_some());
        assert!(!select(&mut state, t2()));
    }

    #[test]
    fn shift_click_builds_a_path_query_and_drops_the_cone() {
        let mut state = ViewState {
            selection: vec![t1()],
            cone: Some(ConeFocus { direction: Direction::Forward, depth: None }),
            ..ViewState::default()
        };
        add_to_selection(&mut state, t2());
        assert_eq!(state.selection, vec![t1(), t2()]);
        assert!(state.cone.is_none());
        assert!(is_path_query(&state));
        let event = key("event:OrderPaid");
        add_to_selection(&mut state, event.clone());
        assert_eq!(state.selection, vec![t1(), event.clone()]);
        assert!(!is_path_query(&state));
        add_to_selection(&mut state, event);
        assert_eq!(state.selection, vec![t1()]);
    }

    #[test]
    fn legend_toggle_and_matrix_pair() {
        let mut state = ViewState { view: ViewKind::Matrix, ..ViewState::default() };
        toggle_machine(&mut state, "Order");
        assert!(state.hidden_machines.contains("Order"));
        toggle_machine(&mut state, "Order");
        assert!(state.hidden_machines.is_empty());
        open_pair(&mut state, "Order".into(), "Shipment".into());
        assert_eq!(state.view, ViewKind::Causal);
        assert_eq!(state.machine_pair, Some(("Order".into(), "Shipment".into())));
    }
}
