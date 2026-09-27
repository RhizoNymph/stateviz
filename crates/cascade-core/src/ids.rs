//! Typed indices into a [`Model`](crate::Model)'s arenas.
//!
//! Ids are only constructed by the model that owns them, so an id obtained
//! from a model is always valid for that model. Using an id with a different
//! model is a logic error; accessors index directly and will panic on an
//! out-of-range id.

use std::fmt;

macro_rules! define_id {
    ($(#[$meta:meta])* $name:ident, $prefix:literal) => {
        $(#[$meta])*
        #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(u32);

        impl $name {
            pub(crate) fn new(index: usize) -> Self {
                // Arenas are bounded far below u32::MAX by the definition
                // file's size; saturating keeps this total.
                Self(u32::try_from(index).unwrap_or(u32::MAX))
            }

            /// Position in the owning arena, in definition order.
            pub const fn index(self) -> usize {
                self.0 as usize
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, concat!($prefix, "#{}"), self.0)
            }
        }
    };
}

define_id!(
    /// A state machine (one entity type).
    MachineId,
    "machine"
);
define_id!(
    /// A state, possibly nested, belonging to exactly one machine.
    StateId,
    "state"
);
define_id!(
    /// A transition `Machine: from → to` on a trigger.
    TransitionId,
    "transition"
);
define_id!(
    /// A named trigger accepted (or expected to be accepted) by one machine.
    TriggerId,
    "trigger"
);
define_id!(
    /// A named event emitted by transitions.
    EventId,
    "event"
);
define_id!(
    /// A controller: a named set of reaction rules.
    ControllerId,
    "controller"
);
define_id!(
    /// One controller's subscription to one event, holding its rules.
    HandlerId,
    "handler"
);
define_id!(
    /// One reaction rule: fire a trigger on a target selector.
    RuleId,
    "rule"
);
define_id!(
    /// An external source of triggers: users, timers, webhooks.
    ExternalId,
    "external"
);
