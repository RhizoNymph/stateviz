//! Which queue item is delivered next.
//!
//! Plain runs are FIFO. A race replay swaps two contested fires: the fire
//! that FIFO delivers first (the *yielder*) lets a fire of the other rule at
//! the same instance (the *overtaker*) go first.
//!
//! - If an overtaker is already queued when the yielder reaches the head,
//!   the overtaker jumps ahead of it and the yielder follows immediately.
//! - Otherwise the yielder is held back while the queue keeps running, and
//!   is delivered right after the next overtaker.
//! - If the queue drains while the yielder is held, nothing else can happen
//!   before it, so it is released and the swap has failed: the overtaker
//!   only exists because of the yielder.
//!
//! Everything else keeps its FIFO position, and the whole mechanism is
//! deterministic.

use std::collections::VecDeque;

/// Progress of one swap.
#[derive(Debug)]
pub(crate) enum SwapState<T> {
    /// The yielder has not reached the head of the queue yet.
    Armed,
    /// The yielder is waiting for an overtaker.
    Holding(T),
    /// An overtaker was delivered, and the yielder right after it.
    Swapped,
    /// The queue drained while the yielder was held; it was delivered
    /// without being overtaken.
    Released,
}

/// Take the next item under a swap. `is_yielder` and `is_overtaker` pick out
/// the contested items.
pub(crate) fn next_swapped<T>(
    queue: &mut VecDeque<T>,
    state: &mut SwapState<T>,
    is_yielder: impl Fn(&T) -> bool,
    is_overtaker: impl Fn(&T) -> bool,
) -> Option<T> {
    loop {
        match state {
            SwapState::Swapped | SwapState::Released => return queue.pop_front(),
            SwapState::Armed => {
                let item = queue.pop_front()?;
                if !is_yielder(&item) {
                    return Some(item);
                }
                if let Some(pos) = queue.iter().position(&is_overtaker)
                    && let Some(overtaker) = queue.remove(pos)
                {
                    queue.push_front(item);
                    *state = SwapState::Swapped;
                    return Some(overtaker);
                }
                *state = SwapState::Holding(item);
            }
            SwapState::Holding(_) => {
                let Some(item) = queue.pop_front() else {
                    return match std::mem::replace(state, SwapState::Released) {
                        SwapState::Holding(held) => Some(held),
                        SwapState::Armed | SwapState::Swapped | SwapState::Released => None,
                    };
                };
                if is_overtaker(&item)
                    && let SwapState::Holding(held) = std::mem::replace(state, SwapState::Swapped)
                {
                    queue.push_front(held);
                }
                return Some(item);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Drain `items` under a swap where `y` yields to `o`; returns the
    /// delivery order and the final state's name.
    fn drain(items: &[&'static str], y: &'static str, o: &'static str) -> (Vec<&'static str>, &'static str) {
        let mut queue: VecDeque<&'static str> = items.iter().copied().collect();
        let mut state = SwapState::Armed;
        let mut out = Vec::new();
        while let Some(item) = next_swapped(&mut queue, &mut state, |i| *i == y, |i| *i == o) {
            out.push(item);
        }
        let name = match state {
            SwapState::Armed => "armed",
            SwapState::Holding(_) => "holding",
            SwapState::Swapped => "swapped",
            SwapState::Released => "released",
        };
        (out, name)
    }

    #[test]
    fn a_queued_overtaker_jumps_ahead_of_the_yielder() {
        assert_eq!(drain(&["a", "y", "b", "o", "c"], "y", "o"), (vec!["a", "o", "y", "b", "c"], "swapped"));
    }

    #[test]
    fn a_held_yielder_follows_the_overtaker() {
        // `o` is not queued when `y` reaches the head, so `y` waits.
        let mut queue: VecDeque<&'static str> = ["y", "a"].into_iter().collect();
        let mut state = SwapState::Armed;
        let is_y = |i: &&str| *i == "y";
        let is_o = |i: &&str| *i == "o";
        assert_eq!(next_swapped(&mut queue, &mut state, is_y, is_o), Some("a"));
        assert!(matches!(state, SwapState::Holding("y")));
        queue.push_back("b");
        queue.push_back("o");
        queue.push_back("c");
        assert_eq!(next_swapped(&mut queue, &mut state, is_y, is_o), Some("b"));
        assert_eq!(next_swapped(&mut queue, &mut state, is_y, is_o), Some("o"));
        assert_eq!(next_swapped(&mut queue, &mut state, is_y, is_o), Some("y"));
        assert_eq!(next_swapped(&mut queue, &mut state, is_y, is_o), Some("c"));
        assert!(matches!(state, SwapState::Swapped));
    }

    #[test]
    fn a_yielder_is_released_when_nothing_overtakes_it() {
        assert_eq!(drain(&["y", "a", "b"], "y", "o"), (vec!["a", "b", "y"], "released"));
    }

    #[test]
    fn without_a_yielder_the_order_is_fifo() {
        assert_eq!(drain(&["a", "o", "b"], "y", "o"), (vec!["a", "o", "b"], "armed"));
    }
}
