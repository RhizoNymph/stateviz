//! Watching and stopping a running exploration from another thread (the
//! app runs explorations off its UI thread).

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

/// Shared between an exploration and its host: the search publishes how
/// many distinct states it has visited, and stops with
/// [`SimError::ExploreCancelled`](crate::SimError::ExploreCancelled) once
/// [`cancel`](Self::cancel) is called.
#[derive(Debug, Default)]
pub struct ExploreProgress {
    states: AtomicUsize,
    cancelled: AtomicBool,
}

impl ExploreProgress {
    pub fn new() -> Self {
        Self::default()
    }

    /// Distinct states visited so far.
    pub fn states(&self) -> usize {
        self.states.load(Ordering::Relaxed)
    }

    /// Ask the search to stop at its next move.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed)
    }

    pub(crate) fn set_states(&self, states: usize) {
        self.states.store(states, Ordering::Relaxed);
    }
}
