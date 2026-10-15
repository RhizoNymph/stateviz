# Timers and subscriptions

How the simulator runs machines that subscribe to events (`on: { event: E,
where: … }`) and transitions that fire after a delay (`after: 30m`), with a
virtual clock. The definition format and the model (`TriggerKind`,
`Event.subscribers`, causal `Notify`/`Delay` edges) are wave 1's
([definition-format.md](definition-format.md),
[semantics-contracts.md](semantics-contracts.md) §2); this document covers
the run-time half in `cascade-sim`. The rest of the simulator (queue,
choices, scenarios, sessions) is [simulator.md](simulator.md).

## Scope

- Delivering an event to subscribing instances: consumer order, `where`
  filters, guards, `Notify` steps, ignored deliveries.
- `PendingItem::consumers_left` for events, and per-consumer delivery of an
  event.
- Arming timers on entry (creation, transitions, history restores),
  cancelling them on exit, deadlines, the clock, `Advance` / scenario
  `wait`, `Wait` and `TimerExpired` steps, timer queue items.
- `PlaySession::{now, timers}`; timers and partial deliveries in the state
  key.
- `cascade simulate --interactive` commands `wait`, `timers`, `now`,
  `step n to <consumer>`.

## Non-scope

- Static analysis of subscriptions and timers (reachability, cycles, races;
  static-analysis.md).
- Drawing them (view-scenes.md) or the app's play clock (build-and-play.md).
- Timers expiring "at any time" between deliveries: that is an exploration
  option (`timers_anytime`, wave 3). In a run the clock only moves by an
  explicit `wait` / `Advance`.
- Wall-clock time.

## Subscriptions

```text
Emit E ──queue──▶ consumers (fixed at emit time):
   for each machine subscribing to E (model order):
      for each live instance (creation order):
         candidates = enabled_transitions(leaf, trigger E) whose `where` holds
         guards / choice point (simulator.md)
         nothing ──▶ ignored (no step)
         taken   ──▶ Notify(cause Emit) ──▶ [Choice] ──▶ Transition ──▶ [SetFields] ──▶ Emit…
   then each controller with a handler for E: Deliver ──▶ rules
```

- The subscription trigger of machine `M` for event `E` is
  `Model::subscription_trigger(M, E)`; it is named after the event.
- `where` and guards read `self` = the instance and `event` = the delivered
  event's payload, typed by `Model::types_for(M, Some(E))` (the machine's
  fields and the event's declared payload).
- **Ignored** deliveries record nothing: an instance not interested in an
  event (wrong `where`, no candidate in its state, every guard false) is the
  normal case.
- The consumers of a queued event are computed when it is emitted:
  subscribing instances alive then, then controllers. A removed instance
  drops out of every queued event's consumers; an instance created later
  does not receive events emitted before it existed. An instance can
  receive its own event.
- `PendingItem::consumers_left` lists the remaining consumers by name:
  instances first, then controllers. `Step { consumer: Some(name) }`
  delivers to that one consumer and removes it from the list; the item is
  removed when the list empties. A whole `Step` delivers to every remaining
  consumer in order, which is the same as stepping them one by one.

## Timers

```text
enter state S (creation, transition, history restore)
   └─▶ for each timer trigger of S's own transitions (definition order):
          arm { instance, trigger, state: S, deadline: now + delay, seq }
leave state S ─▶ cancel the instance's timers armed by S
Advance { by } / wait ─▶ now += by ─▶ Wait { by, now }
   └─▶ every armed timer with deadline ≤ now, by (deadline, seq):
          TimerExpired { instance, trigger } (cause: Wait) ─▶ queue item Timer
queue item Timer ─▶ offer the timer trigger (empty payload) ─▶ [Choice] ─▶ Transition | Dropped
```

- **Which states are left and entered.** Taking a transition leaves the
  states from the old leaf up to (not including) the transition's domain,
  the nearest state that properly contains both its source and its target
  (the machine's top level when none does), and enters the states from the
  domain down to the new leaf. So a self transition re-arms its state's
  timers, and a transition between two children of `P` keeps `P`'s timers.
  An instance created in a state (or added mid-run, or spawned) arms the
  timers of every state it starts in, outermost first.
- **Deadlines** are absolute clock times; `timers()` lists them in deadline
  order, ties in arming order (outer state before inner, earlier instance
  before later).
- **Advancing.** `Advance { by: Some(d) }` (`- { wait: 30s }`) moves the
  clock by `d`; `Advance { by: None }` (`- wait`) moves it to the earliest
  deadline, and is `NoTimerArmed` when nothing is armed. A fixed advance
  with nothing armed is fine (the clock moves, `Wait` is recorded). All
  timers due by the new time are queued at once, in deadline order; the
  clock does not stop at intermediate deadlines.
- **Late timers.** A queued timer whose state has since been left is still
  delivered, and is dropped (`Dropped`, cause the `TimerExpired`) when the
  instance no longer accepts it.
- **Scenario timing.** `- wait` waits for quiet first, like an external
  fire, unless `timing: immediate`. `end: drain` drains the queue but never
  advances time, so a scenario can end with timers armed.
- **Removal** cancels the instance's armed timers and discards its queued
  timer items.
- The **state key** records each armed timer's remaining time (`deadline -
  now`), so sessions that differ only in elapsed time share a key.

## Files

| File | Role | Key exports |
| --- | --- | --- |
| `crates/cascade-sim/src/engine/clock.rs` | Timers of a state, arming, cancelling, advancing | crate-private `Core::{arm, cancel, advance, next_deadline}` |
| `crates/cascade-sim/src/engine/deliver.rs` | Offering triggers (subscriptions included), taking transitions (cancel/arm), events to consumers | crate-private `Core::{offer, deliver_event_to}` |
| `crates/cascade-sim/src/engine/core.rs` | Queue items (`Timer`), consumers (`Consumer`, `ConsumerRef`), per-consumer delivery, removal | crate-private `Core::{deliver_to_consumer, event_consumers}`, `Armed` |
| `crates/cascade-sim/src/engine/instance.rs` | States left and entered by a transition | crate-private `Change`, `Instance::{take, active_states}` |
| `crates/cascade-sim/src/engine/exec.rs` | `Advance`, `Step { consumer }` checks | crate-private `exec` |
| `crates/cascade-sim/src/session/clock.rs` | `now`, `timers`, state key | `PlaySession::{now, timers, state_key}`, `ArmedTimer` |
| `crates/cascade-sim/src/session/view.rs` | `PendingKind::Timer`, consumer names | `PlaySession::pending` |
| `crates/cascade-cli/src/commands/simulate/play.rs` | `wait`, `timers`, `now`, `step n to c` at the prompt | — |
| `crates/cascade-sim/tests/subscriptions.rs`, `timers.rs`, `consumers.rs`, `state_key.rs` | The behaviour above | — |
| `examples/checkout-saga/` | A saga machine subscribing to payment and stock events with a 30m timeout | — |

## Invariants and constraints

- The clock never moves except by `Advance`; it never goes backwards
  (seeking back replays from the start).
- An instance's armed timers belong to states it is in: entering arms,
  leaving cancels, so `timers()` never lists a timer of a state the
  instance has left.
- Every `TimerExpired` is caused by a `Wait`, and every timer queue item by
  a `TimerExpired`.
- Ignored subscription deliveries leave no trace step; a `Notify` is always
  followed (possibly after a `Choice`) by a `Transition` it causes.
- An event's consumers are fixed when it is queued and only shrink.
- Whole delivery of an event equals delivering to each remaining consumer
  in order.
