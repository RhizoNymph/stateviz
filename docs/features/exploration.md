# Exploration

Every delivery ordering a scenario (or a play position) allows, run to
completion and deduplicated into distinct **outcomes**, each with a
**witness**: the play actions that reproduce it. Optionally every option at
every choice point, and timers expiring between any two deliveries. This
answers "can this system end up somewhere else?" (handler-order dependence,
#30; races; timeouts racing cascades) without writing one scenario per
ordering.

## Scope

- `cascade_sim::explore(model, scenario, options)` and
  `explore_from(session, model, options)`; `replay_witness`.
- Two ordering models: per-consumer FIFO (default) and unordered.
- Branching on choice options (`choices`), timers at any time
  (`timers_anytime`), a state bound (`max_states`), and partial-order
  reduction that can be switched off (`reduction`).
- Outcomes: final instances, timers left armed, anomalies (no target,
  ambiguous selector, dropped trigger, skipped input, unused decision, step
  limit), the shortest witness found, and a path count.
- `cascade explore` (text and JSON, exit codes, `--save-witness`).

## Non-scope

- The app's Explore panel (a later step; the API is shaped for it: explore
  from the session's position, open a witness as a branch).
- Livelock detection: paths that never settle (a cascade cycling forever)
  produce no outcome; they are cut by the visited set, not reported. Only
  when no path settles at all (complete, zero outcomes) is there a note.
- Scenario expectations (#31), an `ordered: true` controller flag,
  arithmetic, wall-clock time.

## Options

```rust
pub enum OrderingModel { PerConsumer /* default */, Unordered }
pub struct ExploreOptions {
    pub ordering: OrderingModel,
    pub choices: bool,          // false
    pub timers_anytime: bool,   // false
    pub max_states: usize,      // DEFAULT_MAX_STATES = 100_000
    pub reduction: bool,        // true
}
```

- **PerConsumer**: each consumer (a controller, or a subscribing instance)
  receives events in emission order; the fires one controller queues at one
  instance arrive in order; everything else interleaves, including the
  consumers of one event and expired timers. An event can be delivered to
  its consumers one at a time.
- **Unordered**: any queued item, and any remaining consumer of a queued
  event, may go next.

## Data and control flow

```text
scenario ──validate──▶ inputs::from_scenario
   start: AddInstance per declared instance, then directives before the first input
   inputs: each fire / wait with its timing, and the choose/create/remove after it (chained)
   notes: ignored step/run directives, ignored choose (with choices), end: pause

root SearchState { core (trace forgotten), next input, anomalies }
   │ search::search: iterative DFS
   ▼
frame(state, sleep set Z)
   moves::enabled ── deliver item i to consumer c   (eligible under the ordering model)
                  ├─ input k                          (immediate: any time; else when the queue is empty)
                  └─ advance                          (timers_anytime and a timer armed)
   × decisions: moves::perform runs the base with pushed decisions; with `choices`,
     the first choice point that took its default branches, once per option
   for each move t not asleep:
     child = t(state)  (exec on a cloned Core, anomalies read off the new raw steps,
                         then Recorder::forget)
     Z' = { u ∈ Z | independent(t, u) };  Z += t
     key::search_key(child) seen? ── no ─▶ new node; terminal? record outcome : frame(child, Z')
                                  └─ yes ▶ edge; revisit: explore what slept there before
                                           but is awake now (stored set := stored ∩ Z')
graph (nodes, edges labelled with MoveDescs)
   ├─ shortest(): BFS from the root ─▶ per outcome the nearest terminal node
   │      └─ rebuild(): replay the MoveDescs from the root, turning each into
   │         the actions it takes in the state actually reached ─▶ witness
   └─ path_counts(): paths from the root in the graph minus DFS back edges
Exploration { outcomes sorted by (witness length, sort_text), states, complete, notes }
```

### Inputs

- An `after-quiescence` fire or wait (the default) is a move only when the
  queue is empty; an `immediate` one is a move at any point after the
  previous input, so it interleaves with the running cascade.
- A targetless fire names the fired machine's only live instance when it
  is applied. An input that cannot be applied in some ordering (it targets
  an instance a spawn has not created yet, `wait` with nothing armed, …) is
  skipped there and recorded as `Anomaly::InputFailed`; later inputs still
  run.
- `choose`, `create` and `remove` are chained to the input before them
  (applied in the same move, like the batch run, which never waits for
  them); those before the first input happen at the start.
- `step`, `{ step: n }`, `run` choose an ordering, which exploration
  enumerates: ignored with a note. `end: pause` is ignored with a note.
- With `choices`, `choose` directives are ignored with a note (every
  option is explored). `explore_from` honours the session's pending
  decisions: a choice point they decide does not branch.

### Moves and decisions

A move is a base (`Deliver { position, consumer }`, `Input(k)`, `Advance`)
plus the decisions pushed right before it. Delivering an event with one
consumer left is `Step { choice, consumer: None }` (the head as `choice:
None`); otherwise `Step { choice: Some(pos), consumer: Some(c) }`. Expiring
timers is `Advance { by: None }` (to the next deadline). Identical queue
items lead to identical states, so only one of them is offered.

With `choices`, decisions are found by running the base: if a choice
point took its default, the base is run again once per option `o` with
`Choose { o }` pushed first (so the point consumes it), recursively for the
next defaulted point. Every free choice point gets an explicit `Choose`, so
pushed decisions always line up with the points (a point decided by an
older pending decision is not free and does not branch).

### Terminal states and outcomes

A state is terminal when every input is used and the queue is empty, and,
with `timers_anytime`, no timer is armed. (Without it, timers left armed
are part of the outcome.) Two terminal states are the same outcome when
their instances (name → leaf and fields, removed ones included), timers
(instance, trigger, time left) and anomalies are equal; history and the
absolute clock do not count. Decisions still pending become
`UnusedChoice` anomalies. A move that would exceed `STEP_LIMIT` deliveries
ends the path with `Anomaly::StepLimit` (its witness reaches the state
before that move).

### Search key

`key::search_key`: instances (creation order; name, machine, leaf, fields,
sorted history; removed by name), the queue, armed timers relative to now,
pending decisions, the next input and the anomalies met so far (two paths to
one simulator state with different anomalies are different search states,
so no outcome loses an anomaly).

With reduction, queue orders the ordering model cannot tell apart share a
key. Each item is in channels (an event: one per remaining consumer; a
fire: its (controller, target); a timer: none); only the relative order of
items sharing a channel matters, and the queue is written as the canonical
linearization of that partial order: repeatedly the smallest item text
heading all its channels (Unordered: the sorted multiset). Armed timers are
written sorted. Without reduction the key keeps the exact order.

Equivalent states have different positions and pending ids, so graph edges
and stored sleep sets use `MoveDesc` (item text, consumer, decisions), and
witnesses are rebuilt by replaying descriptions in the actual states.

### Independence (sleep sets)

Each enabled move gets a conservative `Footprint`:

- global (dependent with everything): inputs, `Advance`, a controller
  delivery whose handler has a spawn rule, and every move while decisions
  are pending;
- a delivery to an instance (fire, timer, subscription) reads and writes
  that instance, and may emit the events of any transition enabled from its
  leaf, to those events' consumers;
- a delivery to a controller reads every live instance of the machines its
  rules target (selectors, `NoTarget`/`Ambiguous`).

Two moves are independent when neither is global, they have different
consumers, neither writes what the other reads or writes, and (per-consumer
only) the events they may emit have no consumer in common (otherwise the
two emissions' order in that consumer's channel would differ). Sleep sets
skip transitions only; with the revisit rule every reachable state is still
visited, so reduction never loses an outcome (tested against `reduction:
false` on the examples, the fixtures and ~1200 random explorations).

### Witnesses and path counts

- The witness is the shortest path in the explored graph (BFS) to any
  terminal node of the outcome, rebuilt as play actions. For `explore` it
  starts from `PlaySession::new` (the declared instances are its first
  actions); for `explore_from` it is appended to the session's position.
  Applying it reaches exactly the outcome (tested over every example and
  fixture and every option combination, with and without reduction).
- `paths`: the number of paths from the root to the outcome's terminal
  nodes in the explored graph minus the back edges of a DFS from the root
  (so cycles count once), saturating at `u64::MAX`. Reduction lowers it.

## CLI

```text
cascade explore <definition> <scenario> [--unordered] [--choices] [--timers]
                [--max-states N] [--no-reduction] [--format text|json]
                [--save-witness <dir>]
```

Text output: the scenario, the options, `N outcomes, M states (complete)`
or `(INCOMPLETE: bound of N hit)`, notes, then each outcome with its path
count, its instances (all for outcome 1; for the others only those that
differ from outcome 1), timers left armed, anomalies, and the witness as
`steps:` lines of the scenario it saves as (`scenario_to_yaml` of the
replayed session's `to_scenario`). JSON: `{ scenario, options, states,
complete, notes, outcomes: [{ index, paths, instances: [{ name, machine,
state, fields }], timers, anomalies, witness: [step text] }] }`.
`--save-witness <dir>` writes `<scenario-stem>-outcome-<n>.yaml` per
outcome, each a scenario that `cascade simulate` runs to that outcome.

Exit codes: 0 one outcome and no anomalies; 1 several outcomes or any
anomaly; 2 errors (definition, scenario, I/O); 3 incomplete (takes
precedence over 0/1).

## Files

| File | Role | Key exports |
| --- | --- | --- |
| `crates/cascade-sim/src/explore/mod.rs` | Entry points, building the result, rebuilding witnesses | `explore`, `explore_from`, `replay_witness` |
| `crates/cascade-sim/src/explore/options.rs` | Ordering models and options | `OrderingModel`, `ExploreOptions`, `DEFAULT_MAX_STATES` |
| `crates/cascade-sim/src/explore/outcome.rs` | Result types | `Exploration`, `Outcome`, `OutcomeTimer`, `Anomaly` |
| `crates/cascade-sim/src/explore/inputs.rs` | A resolved scenario as start actions, inputs and notes | crate-private `Inputs`, `Input`, `from_scenario` |
| `crates/cascade-sim/src/explore/moves.rs` | Enabled moves under an ordering model, footprints and independence, performing a move (anomalies, defaulted choices) | crate-private `SearchState`, `Base`, `LineId`, `MoveDesc`, `Footprint`, `enabled`, `independent`, `perform` |
| `crates/cascade-sim/src/explore/key.rs` | Search keys, canonical queue linearization | crate-private `search_key`, `item_text` |
| `crates/cascade-sim/src/explore/search.rs` | DFS with visited set and sleep sets; outcome grouping | crate-private `search` |
| `crates/cascade-sim/src/explore/graph.rs` | Explored graph: BFS shortest paths, path counts | crate-private `Graph` |
| `crates/cascade-sim/src/explore/report.rs` | Text helpers for hosts | `instance_text`, `timer_text`, `differing_instances` |
| `crates/cascade-sim/src/engine/record.rs` | `Recorder::forget`/`kinds_since` (cheap core clones) | crate-private |
| `crates/cascade-sim/tests/explore.rs` | Race fixture, handler order (#30), timers, choices, bounds, notes, skipped inputs, unordered, `explore_from`, saved witnesses | — |
| `crates/cascade-sim/tests/explore_properties.rs` | Witness replay and reduction-equivalence over the corpus and random models | — |
| `crates/cascade-sim/tests/fixtures/explore/shop-order.yaml` | A shop order from checkout to delivery | — |
| `crates/cascade-cli/src/commands/explore.rs` | `cascade explore` | `run`, `ExploreArgs` |
| `crates/cascade-cli/tests/explore.rs` | CLI end to end | — |

## Invariants and constraints

- Exploration is deterministic: the same model, scenario and options give
  the same `Exploration` (no output depends on hash order).
- Every witness replays (from the exploration's start) to exactly its
  outcome: empty queue, the outcome's instances, timers and traceable
  anomalies.
- Reduction never changes the set of outcomes; it may change `states`,
  `paths` and (between equally short ones) which witness is kept.
- Outcomes are distinct and sorted by (witness length, `sort_text`).
- `states <= max_states`; `complete` is false exactly when the bound
  stopped the search.
- The public behaviour of the simulator and play session is unchanged;
  exploration uses the same `exec` and only adds `Recorder::forget`.
