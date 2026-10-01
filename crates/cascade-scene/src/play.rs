//! Inputs for the app's build and play modes.
//!
//! Owner: the `feat/scene-edit-play` workstream draws these; the types are
//! the contract. Everything is keyed by [`ElementKey`] so the overlay does
//! not depend on which `Model` instance the host holds.

use cascade_core::ElementKey;

/// What the canvas is for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum SceneMode {
    /// Reading and analysing (the four views as before).
    #[default]
    View,
    /// Building: the structure view also shows the wiring (event tags and
    /// controller hexagons in a band below the machine lanes, and external
    /// sources), and every connectable element gets a
    /// [`crate::HitTarget::ConnectHandle`] to drag from.
    Edit,
}

/// One instance's position, for markers on its current state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlayMarker {
    /// Instance name, e.g. `o1`: the marker's label.
    pub instance: String,
    pub machine: String,
    /// `ElementKey::State` of the current leaf state.
    pub state: ElementKey,
}

/// A live play session drawn over the causal and structure views.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PlayOverlay {
    pub markers: Vec<PlayMarker>,
    /// Elements the last action went through (transitions taken, events
    /// emitted, handlers run, rules fired): drawn emphasised.
    pub active: Vec<ElementKey>,
    /// Elements waiting in the queue (events and the rules of queued fires),
    /// in queue order: drawn as pending; the head is marked.
    pub pending: Vec<ElementKey>,
}
