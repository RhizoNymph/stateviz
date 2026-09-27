# Static analysis

## Scope

- `cascade_core::analyze(model, graph) -> Vec<Finding>`: the seven checks from
  the spec (invalid fire, nondeterminism, cascade cycle, unhandled event,
  orphan controller, unreachable state, race candidate) plus the
  state-dependent-fire note, on a well-formed `Model`.
- Dead commands: external sources exposing a trigger nothing accepts,
  reported under the invalid-fire check.
- The finding contract (`Check`, `Severity`, `FindingDetail`, `CycleStep`,
  `Finding`) that the CLI, scene builders, simulator and app consume.
- `cascade check`: text and JSON reports and the CI exit codes.
- The pinned examples: `examples/shop/` (the M1 acceptance design) and the
  spec's `examples/order-fulfillment/`.

## Non-scope

- Load errors (unknown names, bad YAML, shape errors): those are
  `Diagnostic`s from parsing and resolving (definition-format.md). Analysis
  only runs on a model that loaded.
- Evaluating guards or rule conditions. Guards are compared as text; nothing
  is assumed about their truth.
- Instance-level or interleaving semantics: races are candidates for the
  trace view (simulator.md), not proofs. Anything stronger goes through the
  P export.
- Drawing findings (badges, red back edges): view-scenes.md.

## Data and control flow

```text
Model ──CausalGraph::build──▶ CausalGraph
(Model, CausalGraph) ──analyze──▶ invalid_fire, nondeterminism, cycles, events,
                                  reachability, races, state_dependent
                                  (each returns unsorted Vec<Finding>)
                              ──sort_findings──▶ Vec<Finding>
```

`analyze` calls every check in turn; each builds findings with
`Finding::new(detail, message)`, which sets the check's default severity.
The result is sorted once by `(severity descending, Check order, source
position of the primary element, primary element, subjects)`, so errors come
first and the order is fully deterministic even for several findings on one
element or for models without source positions. `graph` must be built from
the same `model` (ids are shared).

Messages are one line and name elements with the model's labels
(`Order: pending → paid`, `Order.placed.paid`, `Shipment.start`).

### Invalid fire (error)

For every rule, in model order: when `trigger.accepted_by` is empty, the rule
fires a trigger no transition of the target machine takes, from any state →
`FindingDetail::InvalidFire { rule, trigger }`. For every external source and
each trigger it exposes, the same test gives
`FindingDetail::DeadExternalTrigger { source, trigger }` (a dead command),
whose `check()` is `InvalidFire`, `primary()` the external source and
`subjects()` the source and the trigger. A trigger accepted in only some
states is not invalid; see the state-dependent note.

### Nondeterminism (error)

Per machine, transitions are grouped by `(from, trigger)`, groups in order of
first appearance. A group of two or more is ambiguous unless its guards are
mutually exclusive, approximated syntactically:

1. every candidate has a non-blank guard,
2. at most one guard is `else`,
3. guards are pairwise distinct after whitespace normalization (runs of
   whitespace collapse to one space, ends trimmed; `x  >  0` equals `x > 0`,
   but `x>0` does not).

The first failing rule, in that order, is named in the message (`… has no
guard`, `2 of them are guarded by else`, `the guard x > 0 appears 2 times`).
`transitions` lists every candidate of the group. Only transitions declared
on the same state are compared: a state's own transitions shadow its
ancestors' (statechart priority, `Model::enabled_transitions`), so a child
and an inherited transition on the same trigger never conflict.

### Cascade cycle (warning)

1. Build the transition-level causal relation: for each transition T1,
   `CausalGraph::transition_successors(T1)` gives `(T2, rule)` pairs (T1
   emits an event, a handler of that event has `rule`, which fires a trigger
   T2 accepts). Transitions marked `bounded: true` are dropped (no edges in
   or out) and edges carrying a `bounded: true` rule are dropped. Each
   adjacency list is sorted by `(successor, rule)` and deduplicated.
2. Strongly connected components by Tarjan's algorithm, iterative (an
   explicit frame stack), so a chain of any length cannot overflow the call
   stack (`scc.rs`).
3. Each non-trivial component (more than one transition, or one transition
   with an edge to itself) gives one finding. Its representative cycle starts
   at the component's first transition in model order (lowest
   `TransitionId`) and is a shortest cycle through it: breadth-first search
   inside the component, stopping at the first edge back to the start; ties
   go to the lowest `(successor, rule)` edge. The BFS scratch arrays are
   allocated once and reset per component, so the total work is linear.
4. `CascadeCycle { first, rest }` lists `CycleStep { transition, rule }` in
   causal order: step i's rule fires step i+1's transition, and the last
   step's rule fires the first.

A cycle is silenced when any transition or rule on it is bounded, because
removing that node or edge breaks it; a parallel unbounded rule between the
same two transitions keeps the cycle.

### Unhandled event and orphan controller (warnings)

- An event with at least one emitting transition and no handler is
  unhandled (one finding per event). Events declared but neither emitted nor
  handled are not reported.
- A handler (one controller's subscription to one event) whose event no
  transition emits is an orphan, one finding per handler, so two controllers
  waiting for the same dead event give two findings.

### Unreachable state (warning)

A least fixpoint over all machines at once (`reachability.rs`), with a work
queue so each state, trigger, transition and event is processed once:

- Every machine's initial state is entered.
- Entering a state marks it, its ancestors, and its default-entry chain
  (`Model::default_entry`: initial children down to a leaf) reachable.
- Entering a history pseudo-state marks it and enters its parent (the
  fallback when no history was recorded yet, as in XState and SCXML), so the
  parent's default entry is marked too. A top-level history state falls back
  to the machine's initial state.
- A transition is *enabled* when its source state is reachable. Marking a
  state marks its ancestors, so this covers transitions declared on compound
  states, which apply to their descendants.
- A trigger is *triggerable* when an external source exposes it, or a rule
  fires it whose event is emitted by a *live* transition.
- A transition is *live* when it is enabled and its trigger triggerable. A
  live transition enters its target and emits its events, which arms the
  triggers of every rule handling them.

Guards and rule conditions are assumed satisfiable. States never marked are
reported outermost only: a state is reported when its parent is reachable
(or it is top-level), since an unmarked compound state implies unmarked
descendants; the message counts the nested states it stands for. History
pseudo-states are never reported.

### Race candidate (info)

1. *Racers*: rules whose trigger is accepted somewhere (invalid fires are
   errors already, and a fire nothing accepts cannot race) and whose target
   is not a spawn.
2. *Aliasing*: `new …` (spawn) never aliases, since no one else can hold the
   new instance; a singleton target (`Machine`, no predicates) aliases any
   other target on its machine; selector targets (`where` / `all … where`)
   are assumed to alias, since predicates are not compared.
3. *Causal order*: for every racer, the forward cone of the transitions it
   fires (all transitions accepting its trigger) gives the set of handlers
   downstream of it. A pair is excluded when either rule's handler is
   downstream of the other rule, because then one fire causes the other.
4. For every event E in model order: the forward cone of E gives each
   handler's hop distance (transitions entered). Racers of reached handlers
   are grouped by target machine, and every pair from different controllers
   that may alias and is not causally ordered is a candidate.
5. Each pair is reported once, with the origin event closest to both: the
   smallest sum of the two hop distances, ties to the event first in model
   order. `first` < `second` by rule id; `machine` is the machine both fire
   into.

### State-dependent fire (info)

For each rule whose trigger is accepted somewhere: the candidate states are
the target machine's leaf states (atomic and final, in model order; compound
states are never current alone and history pseudo-states never current).
A leaf drops the fire when `Model::enabled_transitions(leaf, trigger)` is
empty, so transitions declared on an ancestor count. The note is emitted
only when some leaf drops it, with `dropped_in` listing those leaves.

Refinement for spawning rules: a `new Machine …` fire always lands in the
new instance, which is in its initial state's default entry, so only that
leaf is considered. A spawning rule gets a note only when its trigger is
not accepted there, in which case the fire is always dropped.

In a lifecycle machine most rules get this note (final states accept
nothing). That is by design: the note answers "where would this fire be
silently dropped?".

## `cascade check`

```text
cascade check FILE [--format text|json] [--deny-warnings]
```

Text (standard output): one line per finding, in analysis order, then a
summary.

```text
examples/shop/cascade.yaml:207:10: error[invalid-fire]: Fulfillment fires Shipment.cancel on OrderCancelled, but no Shipment transition accepts `cancel`
…
summary: 3 errors, 4 warnings, 12 info
```

The location is the primary element's span start; the summary is
`no findings` when there are none.

JSON (standard output, one line):

```json
{"file": "…", "diagnostics": [],
 "findings": [{"check": "invalid-fire", "severity": "error", "message": "…",
               "line": 207, "col": 10,
               "primary": "rule:Fulfillment/OrderCancelled#0",
               "subjects": ["rule:Fulfillment/OrderCancelled#0", "trigger:Shipment.cancel"]}],
 "summary": {"errors": 3, "warnings": 4, "info": 12}}
```

`primary` and `subjects` are `ElementKey` strings. Key order is fixed by
the report structs, and findings follow analysis order, so the output is
byte-stable for a given file.

| Exit code | When |
| --- | --- |
| 0 | No errors, and no warnings under `--deny-warnings` (info never fails) |
| 1 | Errors, or warnings under `--deny-warnings` |
| 2 | The definition is invalid (text: `path:line:col: error: …` lines and a summary on standard error; JSON: `diagnostics` filled, `findings` empty) or cannot be read (`error: cannot read …` on standard error) |

## Examples

`examples/shop/cascade.yaml` is a five-machine shop (Order, Payment,
Inventory, Shipment, Notification; ten controllers; six external sources)
where each check fires on purpose, marked `PLANTED <check>` in the file:

| Check | Planted problem |
| --- | --- |
| invalid-fire | Fulfillment fires `Shipment.cancel`, which Shipment lacks; Customer exposes `Order.request_return`, which no transition accepts |
| nondeterminism | `Inventory.requested` has an unguarded `reserve` next to a guarded backorder branch |
| cascade-cycle | Order cancel → Refunds → Payment refund → Orders → Order cancel |
| unhandled-event | `ShipmentLost` has no subscriber |
| orphan-controller | Returns waits for `ReturnRequested`, which nothing emits |
| unreachable-state | `Order.returned`, entered only by the orphaned Returns rule |
| race-candidate | Billing (capture) and FraudCheck (void) both react to `PaymentAuthorized` on the same Payment |
| state-dependent-fire | Every non-spawning rule into a lifecycle state (11 notes) |

It also shows what does *not* fire: guards made exclusive with `else`, a
polling self-loop silenced by `bounded: true`, a transition on a compound
state, and spawning rules. `examples/shop/expected-findings.txt` is the
exact text output of `cascade check examples/shop/cascade.yaml` run from the
repository root; after an intended change, regenerate it with
`cargo run -q -p cascade-cli -- check examples/shop/cascade.yaml > examples/shop/expected-findings.txt`
and review the diff.

## Files

| File | Role | Key exports |
| --- | --- | --- |
| `crates/cascade-core/src/analysis/mod.rs` | Contract types, orchestration, sorting | `Check`, `Severity`, `CycleStep`, `FindingDetail`, `Finding` (`Finding::new`), `analyze`, `has_errors` |
| `crates/cascade-core/src/analysis/invalid_fire.rs` | Invalid fires and dead external triggers | `check` (module-private) |
| `crates/cascade-core/src/analysis/nondeterminism.rs` | Ambiguous transitions per state and trigger | `check`, `normalize_guard` |
| `crates/cascade-core/src/analysis/cycles.rs` | Transition-level causal cycles | `check` |
| `crates/cascade-core/src/analysis/scc.rs` | Iterative Tarjan SCC | `strongly_connected` |
| `crates/cascade-core/src/analysis/events.rs` | Unhandled events and orphan handlers | `check` |
| `crates/cascade-core/src/analysis/reachability.rs` | Cross-machine reachability fixpoint | `check`, `Reachability` |
| `crates/cascade-core/src/analysis/races.rs` | Race candidates | `check` |
| `crates/cascade-core/src/analysis/state_dependent.rs` | State-dependent fire notes | `check` |
| `crates/cascade-core/src/analysis/describe.rs` | Shared message wording | `rule`, `trigger`, `state`, `list`, `count` |
| `crates/cascade-core/src/analysis/tests/` | Per-check behaviour, pinned examples, ordering, the 20-machine performance bound | — |
| `crates/cascade-cli/src/commands/check.rs` | `cascade check`: text and JSON reports, exit codes | `run` |
| `crates/cascade-cli/tests/check.rs` | The binary end to end: golden text, JSON, exit codes | — |
| `examples/shop/cascade.yaml`, `examples/shop/expected-findings.txt` | M1 acceptance example and its exact output | — |

## Invariants and constraints

- `analyze` is a pure function of `(Model, CausalGraph)`; the same model
  gives the same findings in the same order (tested).
- Every finding's severity is its check's default severity, and
  `detail.check()` agrees with the variant; `primary()` is always one of
  `subjects()`.
- `CascadeCycle` steps are in causal order and start at the cycle's first
  transition in model order; a `RaceCandidate` has `first < second` and
  both rules fire into `machine`.
- No recursion proportional to model size: SCC and cycle search are
  iterative; the fixpoint uses a work queue. Recursion depth is bounded by
  state nesting depth only.
- The whole analysis of a generated 20-machine, 200-state, 50-controller
  model runs in well under 500 ms in a debug build (about 40 ms measured);
  a test enforces the bound.
- Adding a `FindingDetail` variant must keep `check`, `primary` and
  `subjects` exhaustive; consumers matching exhaustively on it must be
  updated.

## Known limitations

- Guards are compared as normalized text: `x > 0` and `x >= 1` count as
  exclusive, and `x>0` differs from `x > 0`.
- Race aliasing does not compare selector predicates, so two rules selecting
  by different fields are still candidates.
- Source columns come from the parser's spans, which currently carry the
  YAML library's 0-based columns (lines are 1-based) although `Pos`
  documents both as 1-based. Once the parser adds 1, the columns in
  `expected-findings.txt` shift by one and the file must be regenerated.
