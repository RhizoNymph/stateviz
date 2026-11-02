//! What an exploration enumerates: the ordering model, choices, timers, and
//! its bounds.

/// Which delivery orders are possible.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum OrderingModel {
    /// Each consumer sees events in emission order; each instance receives
    /// one controller's fires in order; everything else interleaves,
    /// including the consumers of one event (and expired timers).
    #[default]
    PerConsumer,
    /// Any queued item (and any consumer of a queued event) may go next.
    Unordered,
}

impl OrderingModel {
    /// `per-consumer` or `unordered`.
    pub const fn name(self) -> &'static str {
        match self {
            OrderingModel::PerConsumer => "per-consumer",
            OrderingModel::Unordered => "unordered",
        }
    }
}

/// The default bound on distinct states.
pub const DEFAULT_MAX_STATES: usize = 100_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExploreOptions {
    pub ordering: OrderingModel,
    /// Branch on every option at choice points (otherwise use defaults and
    /// pending decisions only).
    pub choices: bool,
    /// Let armed timers expire between any two deliveries (otherwise only
    /// when the scenario waits). An outcome then has no timer armed.
    pub timers_anytime: bool,
    /// Stop after this many distinct states; the result is then
    /// incomplete.
    pub max_states: usize,
    /// Partial-order reduction: queue orders that the ordering model cannot
    /// tell apart count as one state, and sleep sets skip interleavings of
    /// independent moves. Never changes the outcomes found, only the
    /// states, path counts and (rarely) which equally short witness is
    /// kept; off is for tests and debugging.
    pub reduction: bool,
}

impl Default for ExploreOptions {
    fn default() -> Self {
        Self {
            ordering: OrderingModel::PerConsumer,
            choices: false,
            timers_anytime: false,
            max_states: DEFAULT_MAX_STATES,
            reduction: true,
        }
    }
}
