# Simulator

Scenario files, the FIFO simulator that steps through them, the traces it
records for the trace view, and replaying race candidates in both orders.

## Scope

- The scenario file format: parsing with source spans, and validation
  against a model.
- Finding the scenario files that belong to a definition.
- Simulating a scenario with queued FIFO event semantics: instances, target
  selectors (one, all, spawn), history, payloads, step timing.
- Recording a `Trace`: lifelines in a stable order, steps with cause links,
  final states.
- Replaying a race candidate with its two contested fires in both orders.
- `cascade simulate` (text and JSON output, `--race`).

## Non-scope

- Evaluating guards or `when:` conditions (free text; see below).
- Drawing the sequence diagram (view-scenes.md) or the app's trace panel
  (native-app.md).
- Finding race candidates (static-analysis.md); the simulator only replays
  them.
- Timers or delayed transitions: time is an external source such as `Clock`.
- Recorded runtime logs (a later version of the trace view).
- Exploring every interleaving; that is what the P export is for.

## Scenario format

```yaml
scenario: happy path              # required display name
instances:                        # optional; name → instance, in declaration order
  o1:
    machine: Order                # required
    fields: { orderId: "1" }      # optional scalar values; checked against the
                                  #   machine's `fields:` when it declares any
    state: pending                # optional; path or unique local name, default
                                  #   the machine's initial state; compound states
                                  #   are entered by default entry
  log: Log                        # shorthand for { machine: Log }
steps:                            # required; the external triggers, in order
  - source: PaymentGateway        # an external source from the definition
    fire: Order.capture_ok        # Machine.trigger; the source must list it
    target: o1                    # optional; default: the fired machine's only instance
    payload: { amount: "42" }     # optional scalar values
    timing: immediate             # optional; after-quiescence (default) | immediate
```

- Scalars are read as text: `orderId: 1` and `orderId: "1"` are the same.
- Names (instances, machines, sources, fields, payload keys) follow the
  definition format's name rule; `state:` may be a dotted path.
- Unknown keys are errors, so typos are caught. YAML anchors and aliases are
  expanded by the YAML loader.
- A step may target an instance a controller will spawn, by the name the
  simulator will give it (`shipment1`, see below). Whether it exists is
  checked when the step runs.
- Scenario files live in a `scenarios/` directory next to the definition
  (any `*.yaml`/`*.yml`), or next to it as `*.scenario.yaml`/`*.scenario.yml`.

Every problem is reported with its source span, sorted by position, and
parsing and validation keep going after the first one:

| Stage | Diagnostics (`ScenarioErrorKind`) |
| --- | --- |
| Parse | `YamlSyntax`, `EmptyDocument`, `MultipleDocuments`, `WrongType`, `UnknownKey`, `MissingKey`, `InvalidName`, `InvalidTriggerRef`, `UnknownTiming` |
| Validate | `DuplicateInstance`, `UnknownMachine`, `UnknownField`, `UnknownState`, `AmbiguousState`, `HistoryStart`, `UnknownSource`, `UnknownTrigger`, `SourceCannotFire`, `UnknownInstance`, `TargetMachineMismatch`, `NoInstance`, `AmbiguousInstance` |
| Run (step targets only) | `UnknownInstance`, `TargetMachineMismatch`, `NoInstance`, `AmbiguousInstance` |

## Semantics

### The queue

There is one global FIFO queue of two kinds of item: emitted events and
controller fires.

```text
scenario step ─▶ ExternalFire ─▶ Transition | Dropped ─▶ Emit ─▶ queue
queue head = event ─▶ Deliver (each handler, definition order)
                        └─▶ each rule: Fire | Spawn + Fire | NoTarget | Ambiguous ─▶ fires join the queue
queue head = fire  ─▶ Transition | Dropped ─▶ Emit ─▶ queue
```

1. Taking the head **event**, the simulator delivers it to every handler
   subscribed to it, in controller definition order (`Event::handlers`).
   Each handler's rules run in order; each picks its target(s) and queues a
   fire. Because fires are queue items, fires from two controllers
   interleave with each other and with the events their transitions emit.
2. Taking the head **fire**, the simulator delivers its trigger to the
   target instance.

### Scenario steps and timing

- `after-quiescence` (default): the queue is drained completely, then the
  step runs.
- `immediate`: the step runs right after the previous one, before any of
  the cascade that step queued is processed. This is how an external
  trigger interleaves with a running cascade (a timeout arriving while
  fulfillment is still in flight).

A step's trigger is delivered to its target at once (recorded as
`ExternalFire` followed by the delivery); the events the resulting
transition emits join the back of the queue. After the last step the queue
is drained.

### Delivering a trigger

- The instance's current state is always a leaf (atomic or final).
- `Model::enabled_transitions(leaf, trigger)` gives the candidates from the
  innermost state that has any. None means the trigger is **dropped**
  (`Dropped`). Several (guarded alternatives) means the **first in
  definition order** is taken: guards are free text and never evaluated, and
  neither are rule `when:` conditions (rules always fire).
- Before entering the target, the leaf being left is recorded as the last
  active leaf under each of its ancestors (and under the machine's top
  level).
- The target is entered by default entry (`Model::default_entry`). A
  history target restores from that record: **deep** history restores the
  last leaf itself; **shallow** history re-enters the parent's child on the
  way to that leaf, by default entry. With nothing recorded, the history's
  parent is entered by default. History is per instance.
- `Transition { from, to }` records the actual leaves before and after,
  which may be nested inside the transition's declared states.

### Payloads

Events carry string key/value payloads. An emitted event's payload is the
emitting instance's fields overlaid with (overridden by) the payload that
came with the trigger:

- a scenario step's trigger comes with the step's `payload:`;
- a controller fire comes with the payload of the event being handled.

So correlation keys such as `orderId` flow down a cascade. Payloads are not
filtered to an event's declared payload fields. `SimRun::payloads` keeps the
payload of every `Emit` step and of every `ExternalFire` step that has one;
`Trace` itself carries none.

### Target selectors

- `field == event.x` compares the instance's `field` with the payload's
  `x`; `field == literal` compares with the literal. A predicate holds only
  when both sides are present and equal. No predicates match every instance
  of the machine.
- `Machine [where …]` (one): exactly one match fires; none records
  `NoTarget`; several record `Ambiguous` with the candidates, and nothing is
  fired.
- `all Machine [where …]`: fires at every match in instance order (creation
  order); none records `NoTarget`.
- `new Machine [with …]`: creates an instance in the machine's initial state
  (default entry) with the assigned fields (`event.x` assignments whose
  payload lacks `x` leave the field unset), records `Spawn`, then fires at
  it. It is named `<machine-lowercase><n>` with the smallest `n ≥ 1`, counting
  up per machine, that is not taken (declared `shipment1` makes the first
  spawn `shipment2`). Rules after it in the same delivery can already select
  it.

### Cause links

Every step names the step that directly caused it:

| Step | Cause |
| --- | --- |
| `ExternalFire` | none (scenario step) |
| `Transition` / `Dropped` | the `ExternalFire` or `Fire` it delivers |
| `Emit` | its `Transition` |
| `Deliver` | the `Emit` of the event |
| `Fire` / `NoTarget` / `Ambiguous` / `Spawn` | the `Deliver` |
| `Fire` after a spawn | the `Spawn` |

Causes always point to earlier steps.

### Lifelines

Lifelines are ordered by definition, never by first appearance, so the same
scenario always lays out the same way:

1. external sources that fire at least one step, in model order;
2. every instance (declared and spawned), grouped by machine in model
   order; within a machine, declared instances in declaration order, then
   spawned ones in spawn order;
3. controllers that receive at least one event, in model order.

`final_states` maps every instance lifeline to its leaf state after the run.

### Step limit

A run delivers at most `STEP_LIMIT` (10 000) queue items; past that it fails
with `SimError::StepLimit`, which almost always means an unbounded cascade
cycle.

## Race orderings

`race_orderings(model, scenario, &FindingDetail::RaceCandidate { origin, first, second, .. })`:

1. Run the scenario in plain FIFO order (`as_queued`).
2. Find the contested pair in that trace: a `Fire` of each of the two rules,
   at the same instance, both descended (through cause links) from the same
   `Emit` of `origin`. When several pairs qualify, the one whose first
   delivery is earliest wins. None: `SimError::RaceNotReached`.
3. Run again with a swap. The fire FIFO delivered first (the *yielder*,
   identified by rule, instance name and occurrence count, which are stable
   because runs are deterministic) lets the other rule's next fire at that
   instance (the *overtaker*) go first:
   - if an overtaker is queued when the yielder reaches the head, the
     overtaker jumps ahead of it and the yielder follows immediately;
   - otherwise the yielder is held back while the queue keeps running and is
     delivered right after the next overtaker (this covers fires at
     different cascade depths);
   - if the queue drains while the yielder is held, the overtaker only
     exists because of the yielder: `SimError::RaceNotSwappable`.
   Everything else keeps its FIFO position.
4. `Trace::ordering` names the controller whose fire goes first, e.g.
   `Fulfillment first` / `Billing first` (full rule labels when both rules
   belong to one controller).

Other findings give `SimError::NotARace`.

## CLI

```text
cascade simulate <definition> <scenario> [--format text|json] [--race <n>]
```

Text output, one line per step, indented by causal depth, with `← n` naming
the cause when it is not the line above:

```text
Scenario: timeout races the payment
Lifelines: Customer, PaymentGateway, Clock, Order o1, Shipment s1, Fulfillment
   0  Customer fires submit at o1
   1    Order o1: draft → pending (submit)
   2  PaymentGateway fires capture_ok at o1
   3    Order o1: pending → paid (capture_ok)
   4      o1 emits OrderPaid {orderId: 1}
   5  Clock fires timeout at o1
   6    Order o1: timeout dropped in paid
   7        Fulfillment receives OrderPaid  ← 4
   8          Fulfillment fires start at s1
   9            Shipment s1: idle → picking (start)
Final states:
  Order o1: paid
  Shipment s1: picking
```

`--format json` prints `{ scenario, ordering, lifelines, steps, final_states }`;
each step has `index`, `cause`, `depth`, `kind` (kebab-case), `text`, its
lifeline indices, element keys (`transition:…`, `rule:…`) and `payload` when
it has one. `--race <n>` runs static analysis and replays the n-th race
candidate (from 0, in `cascade check` order), printing both orderings (JSON:
`{ race, as_queued, swapped }`). Scenario problems print as
`path:line:col: error: …`; every failure exits with code 2.

## Files

| File | Role | Key exports |
| --- | --- | --- |
| `crates/cascade-sim/src/lib.rs` | Entry points and semantics summary | `simulate`, `simulate_run`, `race_orderings`, `race_runs`, `SimRun`, `RaceTraces`, `RaceRuns` |
| `crates/cascade-sim/src/error.rs` | Typed, spanned errors | `ScenarioError`, `ScenarioDiagnostic`, `ScenarioErrorKind`, `SimError` |
| `crates/cascade-sim/src/scenario/mod.rs` | Scenario types | `Scenario`, `InstanceDecl`, `Step`, `StepTiming`, `ValueMap`, `ValueEntry`, `Payload` |
| `crates/cascade-sim/src/scenario/node.rs` | Diagnostic-recording YAML accessors (saphyr) | crate-private |
| `crates/cascade-sim/src/scenario/parse.rs` | YAML → `Scenario` | `parse_scenario` |
| `crates/cascade-sim/src/scenario/validate.rs` | `Scenario` × `Model` → typed ids | `validate`, `ResolvedScenario` |
| `crates/cascade-sim/src/scenario/file.rs` | Discovery and loading | `discover_scenarios`, `load_scenario_file`, `DiscoverError`, `ScenarioFileError` |
| `crates/cascade-sim/src/engine/mod.rs` | The run loop: queue, delivery, rules, spawn naming | `STEP_LIMIT`; crate-private `run`, `Swap`, `FireKey` |
| `crates/cascade-sim/src/engine/instance.rs` | Instance state, taking transitions, history | crate-private `Instance`, `InstanceIx` |
| `crates/cascade-sim/src/engine/select.rs` | Selector predicates and spawn assignments | crate-private |
| `crates/cascade-sim/src/engine/record.rs` | Raw steps → `Trace` with ordered lifelines | crate-private `Recorder` |
| `crates/cascade-sim/src/engine/schedule.rs` | FIFO with one swap (pull ahead / hold / release) | crate-private `next_swapped` |
| `crates/cascade-sim/src/race.rs` | Contested pair detection, swapped replay, labels | crate-private |
| `crates/cascade-sim/src/describe.rs` | Plain-text labels for hosts that list traces | `step_text`, `lifeline_label`, `payload_text`, `selector_text`, `causal_depths` |
| `crates/cascade-sim/src/trace.rs` | The trace contract (unchanged shapes) | `Trace`, `TraceStep`, `TraceStepKind`, `Lifeline`, `LifelineIx`, `StepIx` |
| `crates/cascade-sim/tests/` | Parsing, validation, FIFO/timing, selectors, history, lifelines, races, discovery, examples; `fixtures/race/` holds a two-controller race | — |
| `crates/cascade-cli/src/commands/simulate.rs` | `cascade simulate` | `run` |
| `crates/cascade-cli/tests/simulate.rs` | CLI output end to end | — |
| `examples/order-fulfillment/scenarios/` | `happy-path`, `timeout-race` (immediate clock), `timeout-first` | — |

## Invariants and constraints

- Runs are deterministic: the same model and scenario give the same trace,
  lifelines included. No iteration order depends on hashing.
- A `ResolvedScenario` only comes from `validate` and is valid for the model
  it was validated against.
- An instance's state is always a leaf: atomic or final, never compound or
  history.
- Every `LifelineIx` in a trace indexes `Trace::lifelines`; every step's
  cause is an earlier step; every instance has a lifeline and a final
  state; external and controller lifelines exist only if a step uses them.
- `Trace` and `TraceStepKind` shapes are a contract with the trace view;
  payloads travel beside the trace in `SimRun`, not inside it.
- Guards and conditions are never evaluated: the first enabled transition in
  definition order is taken and every rule fires.
- The queue is bounded by `STEP_LIMIT` delivered items per run.
- Race replays change only the order of the two contested fires; every other
  item keeps its FIFO position.
