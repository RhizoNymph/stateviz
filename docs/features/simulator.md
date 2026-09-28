# Simulator

Scenario files, the FIFO simulator that steps through them, the traces it
records for the trace view, replaying race candidates in both orders, and
the interactive play session built on the same engine.

## Scope

- The scenario file format: parsing with source spans, and validation
  against a model.
- Finding the scenario files that belong to a definition.
- Simulating a scenario with queued FIFO event semantics: instances, target
  selectors (one, all, spawn), history, payloads, step timing.
- Recording a `Trace`: lifelines in a stable order, steps with cause links,
  final states.
- Replaying a race candidate with its two contested fires in both orders.
- Manual scenario steps: delivering a chosen queue item, running until
  quiet, creating and removing instances mid-run, ending without draining.
- `PlaySession`: the simulator driven one action at a time, with a
  rewindable, branchable timeline, replay against an edited model, and
  saving as a scenario (`scenario_to_yaml`).
- `cascade simulate` (text and JSON output, `--race`, `--interactive`).

## Non-scope

- Evaluating guards or `when:` conditions (free text; see below).
- Drawing the sequence diagram (view-scenes.md) or the app's trace panel
  (native-app.md).
- Finding race candidates (static-analysis.md); the simulator only replays
  them.
- Timers or delayed transitions: time is an external source such as `Clock`.
- Drawing play state (`cascade-scene` `PlayOverlay`) and the app's play
  mode (build-and-play.md).
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
  - step                          # deliver the queue's head
  - { step: 1 }                   # deliver the item at position 1 (0 is the head)
  - run                           # deliver until the queue is empty
  - { create: o2, machine: Order, fields: { orderId: "2" }, state: pending }
  - { remove: o2 }                # take an instance out of play
end: pause                        # optional; drain (default) | pause
```

`steps:` entries other than external fires are *directives*; saved play
sessions use them to reproduce exactly what the player did:

| Entry | Meaning |
| --- | --- |
| `- step` | Deliver the queue's head (one event or fire). |
| `- { step: n }` | Deliver the item at position `n` of the queue (0-based, 0 is the head), ahead of the ones before it. |
| `- run` | Deliver queue heads until the queue is empty. |
| `- { create: name, machine: M, fields: {…}, state: s }` | An instance appears here, as in `instances:` (same checks); it takes part from this point on. |
| `- { remove: name }` | The instance leaves play (see below). |
| `end: pause` (top level) | After the last entry, stop with whatever is still queued instead of draining. |

- Directives interleave with external fires in file order. A fire's
  `timing` applies when the fire is reached, after the directives before
  it.
- In the parsed `Scenario`, external fires stay in `steps` (each with the
  directives written before it in `Step::before`), directives after the last
  fire are in `trailing`, and `Scenario::entries()` yields everything in
  file order as `ScenarioEntry::Fire` / `ScenarioEntry::Directive`.
  Existing scenario files parse exactly as before.
- Validation walks the entries in order, tracking which instances exist at
  each point: a fire may only target an instance declared, or created
  before it and not removed since; a targetless fire counts the instances
  alive at that point. Instance names are never reused: creating a name
  that was declared or created before, even if since removed, is a
  `DuplicateInstance`.

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
| Parse | `YamlSyntax`, `EmptyDocument`, `MultipleDocuments`, `WrongType`, `UnknownKey`, `MissingKey`, `InvalidName`, `InvalidTriggerRef`, `UnknownTiming`, `InvalidQueuePosition`, `UnknownEnd` |
| Validate | `DuplicateInstance`, `UnknownMachine`, `UnknownField`, `UnknownState`, `AmbiguousState`, `HistoryStart`, `UnknownSource`, `UnknownTrigger`, `SourceCannotFire`, `UnknownInstance`, `TargetMachineMismatch`, `NoInstance`, `AmbiguousInstance` |
| Run (step targets and directives) | `UnknownInstance`, `TargetMachineMismatch`, `NoInstance`, `AmbiguousInstance`, `NameTaken` (a spawn took the name), `QueueEmpty`, `NoPendingItem` |

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
transition emits join the back of the queue. After the last entry the queue
is drained, unless the scenario says `end: pause`.

### Removing instances

A removed instance leaves play: selectors, step targets and the session's
instance list no longer see it; fires already queued for it are
**discarded** (nothing delivers them, so their `Fire` steps have no
delivery); events it emitted stay queued, since they were already sent. Its
lifeline and last state stay in the trace (`final_states` keeps it), and its
name is never given out again, by `create`, a player or a spawn.

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

A run (or a play session, in total) delivers at most `STEP_LIMIT` (10 000)
queue items; past that it fails with `SimError::StepLimit`, which almost
always means an unbounded cascade cycle. In a session the failing action is
rejected and changes nothing.

## Play session

`PlaySession` (`cascade_sim::session`) is the simulator driven one
`PlayAction` at a time. Actions name things (machine, instance, source and
`Machine.trigger` names), never typed ids, so a session survives model
reloads:

| Action | Effect |
| --- | --- |
| `AddInstance { name, machine, fields, state }` | New instance. `name: None` picks `<machine-lowercase><n>` with the smallest `n` never used; the timeline records the name it got. `state: None` is the initial state. |
| `RemoveInstance { name }` | See *Removing instances*. |
| `Fire { source, trigger, target, payload }` | An external fire, delivered at once; emitted events join the queue. Firing a trigger the state does not accept records a drop. |
| `Step { choice }` | Deliver the head (`None`) or the item at position `choice` of `pending()`. |
| `RunUntilQuiet` | Deliver heads until the queue is empty (recorded even when it was already empty). |

```text
host ──PlayAction──▶ PlaySession::apply
                      │ exec: look names up in the Model and the core; any problem
                      │       → SimError::Action(kind), nothing changed
                      ▼
                     Core (instances, queue, recorder) ──▶ trace(), payloads(),
                                                           instances(), pending()
scenario ──validate──▶ drive ──PlayActions──▶ the same exec ──▶ simulate / from_scenario
```

- **One engine.** The batch simulator is a scenario turned into play
  actions (`engine::drive`): declared instances become `AddInstance`, each
  fire becomes `Fire` (preceded by `RunUntilQuiet` when it is
  after-quiescence and something is queued; a targetless fire names the one
  instance it finds), directives map one to one, and the final drain is a
  `RunUntilQuiet`. `PlaySession::from_scenario` records exactly those
  actions, so its trace and payloads equal `simulate_run`'s.
- **Queries.** `trace()` and `payloads()` (as `SimRun`), `instances()` (live
  instances in lifeline order with their leaf state and fields),
  `pending()` (the queue, head first: a stable `PendingId`, the kind, the
  step that queued it, and a label such as `OrderPaid from o1` or
  `Fulfillment → s1: start`), `available_fires(model)` (every external
  source × each trigger it lists × each live instance of the trigger's
  machine, with `accepted` from `Model::enabled_transitions` on the current
  leaf), `timeline()`.
- **Pending ids** are given out in queueing order and never reused within
  a line of play, so an id names the same item for as long as it is
  queued. Replaying the same actions gives the same ids.
- **Lifeline indices** (`InstanceState::lifeline`, `PendingKind::Fire`) are
  valid for the current `trace()` only: lifelines are ordered by
  definition, so a new participant can shift them.
- **Errors.** A rejected action changes nothing (the session applies it to
  a copy). Name and queue problems are `SimError::Action(kind)` with the
  same `ScenarioErrorKind`s scenarios use (`UnknownMachine`, `NameTaken`,
  `InvalidName`, `UnknownField`, `UnknownState`, `UnknownSource`,
  `UnknownTrigger`, `SourceCannotFire`, `UnknownInstance`,
  `TargetMachineMismatch`, `QueueEmpty`, `NoPendingItem`, …);
  `StepLimit` as above. Problems in a scenario given to `from_scenario` or
  `simulate` carry the entry's source span (`SimError::Scenario`).

### Timeline, rewind and branches

- `Timeline { actions, position, branches }`: `actions[..position]` are
  applied. The session keeps only the state at `position`.
- `seek(model, p)`: forward applies the next actions to the current state;
  backward replays `actions[..p]` from the start (deterministic, and cheap
  for designer-sized systems; no snapshots). `p > len` is
  `SimError::NoSuchPosition`.
- `apply` at a position before the end forks: the whole old line is saved
  as `Branch { fork: position, actions }` (full line, not just the tail, so
  a branch stays meaningful whatever happens to the current line), then the
  line is truncated and the action appended. If the action equals the next
  one on the line, the session just moves forward instead.
- `switch_branch(model, i)`: branch `i` becomes the current line,
  positioned at its end; the current line takes its place in the list
  (`fork` = the actions they share), so switching to `i` again goes back.
  `SimError::NoSuchBranch` for a missing index.
- `replay(new_model)`: after an edit, re-run `actions[..position]` against
  the new model. It stops at the first action that fails, returning the
  session positioned before it (the rest of the line kept as its future,
  branches kept) and `Some((index, error))`: e.g. a removed transition
  leaves nothing queued, so the next `Step` fails with `QueueEmpty`.

### Saving

`to_scenario(name)` writes `actions[..position]` as a scenario: leading
`AddInstance`s become `instances:`; later ones `create`; `RemoveInstance`
`remove`; a `Fire` into an empty queue a plain step and one while items are
queued `timing: immediate`; `Step` `- step` / `- { step: n }`;
`RunUntilQuiet` `- run`; and a non-empty queue at the end `end: pause`.
`scenario_to_yaml` writes one flow-style line per entry (values always
double-quoted, names quoted only when YAML would misread them), and parses
back to the same scenario. `from_scenario(parse_scenario(scenario_to_yaml(
to_scenario(s))))` has the same trace, payloads, pending items and actions
as `s`.

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
cascade simulate <definition> [scenario] --interactive
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

`--interactive` plays the system at a prompt, starting from the scenario
when one is given: `add <Machine> [name] [@state] [field=value …]`,
`remove <name>`, `fire <Source> <Machine.trigger> <target> [key=value …]`,
`step [n]`, `run`, `instances`, `pending`, `fires`, `trace`, `timeline`,
`branches`, `seek <n>`, `branch <i>`, `save <path>`, `quit`. Each action
prints the trace lines it added; mistakes print `error: …` and play goes
on.

## Files

| File | Role | Key exports |
| --- | --- | --- |
| `crates/cascade-sim/src/lib.rs` | Entry points and semantics summary | `simulate`, `simulate_run`, `race_orderings`, `race_runs`, `SimRun`, `RaceTraces`, `RaceRuns` |
| `crates/cascade-sim/src/error.rs` | Typed, spanned errors | `ScenarioError`, `ScenarioDiagnostic`, `ScenarioErrorKind`, `SimError` |
| `crates/cascade-sim/src/scenario/mod.rs` | Scenario types | `Scenario` (`entries()`), `ScenarioEntry`, `Directive`, `ScenarioEnd`, `InstanceDecl`, `Step`, `StepTiming`, `ValueMap`, `ValueEntry`, `Payload` |
| `crates/cascade-sim/src/scenario/node.rs` | Diagnostic-recording YAML accessors (saphyr) | crate-private |
| `crates/cascade-sim/src/scenario/parse.rs` | YAML → `Scenario` (fires and directives) | `parse_scenario` |
| `crates/cascade-sim/src/scenario/validate.rs` | `Scenario` × `Model` → typed ids, entries checked in order | `validate`, `ResolvedScenario`; crate-private `ResolvedEntry`, `lookup_state` |
| `crates/cascade-sim/src/scenario/write.rs` | `Scenario` → YAML | `scenario_to_yaml` |
| `crates/cascade-sim/src/scenario/file.rs` | Discovery and loading | `discover_scenarios`, `load_scenario_file`, `DiscoverError`, `ScenarioFileError` |
| `crates/cascade-sim/src/engine/mod.rs` | The batch run (a core driven by a scenario, FIFO or with a swap) | `STEP_LIMIT`; crate-private `run`, `Swap`, `FireKey` |
| `crates/cascade-sim/src/engine/core.rs` | The steppable core: instances, queue with ids and labels, delivery, rules, spawn naming, removal | crate-private `Core`, `Queued`, `QueueItem` |
| `crates/cascade-sim/src/engine/exec.rs` | One `PlayAction` → checked core operations | crate-private `exec`, `Schedule` |
| `crates/cascade-sim/src/engine/drive.rs` | A resolved scenario → play actions, errors given the entry's span | crate-private `drive`, `Player` |
| `crates/cascade-sim/src/engine/instance.rs` | Instance state, taking transitions, history, removed flag | crate-private `Instance`, `InstanceIx` |
| `crates/cascade-sim/src/engine/select.rs` | Selector predicates and spawn assignments | crate-private |
| `crates/cascade-sim/src/engine/record.rs` | Raw steps → `Trace` with ordered lifelines (and each instance's lifeline) | crate-private `Recorder`, `Finished` |
| `crates/cascade-sim/src/engine/schedule.rs` | FIFO with one swap (pull ahead / hold / release) | crate-private `next_swapped` |
| `crates/cascade-sim/src/session/mod.rs` | The play session: apply, seek, branches, replay, from a scenario | `PlaySession`, `PlayAction`, `ActionOutcome`, `Timeline`, `Branch`, `PendingId`, `PendingItem`, `PendingKind`, `InstanceState`, `AvailableFire`, `scenario_to_yaml` |
| `crates/cascade-sim/src/session/view.rs` | Instances, pending items, available fires | `PlaySession::{instances, pending, is_pending, available_fires}` |
| `crates/cascade-sim/src/session/save.rs` | Timeline → scenario | `PlaySession::to_scenario` |
| `crates/cascade-sim/src/race.rs` | Contested pair detection, swapped replay, labels | crate-private |
| `crates/cascade-sim/src/describe.rs` | Plain-text labels for hosts that list traces | `step_text`, `lifeline_label`, `payload_text`, `selector_text`, `causal_depths` |
| `crates/cascade-sim/src/trace.rs` | The trace contract (unchanged shapes) | `Trace`, `TraceStep`, `TraceStepKind`, `Lifeline`, `LifelineIx`, `StepIx` |
| `crates/cascade-sim/tests/` | Parsing, validation, FIFO/timing, selectors, history, lifelines, races, discovery, examples; `scenario_steps` (directives), `session` (actions, queue, removal, limit), `session_timeline` (seek, branches, replay), `session_scenario` (batch equivalence, saving round trips); `fixtures/race/` holds a two-controller race | — |
| `crates/cascade-cli/src/commands/simulate.rs` | `cascade simulate` | `run` |
| `crates/cascade-cli/src/commands/simulate/play.rs` | `cascade simulate --interactive` | `run` |
| `crates/cascade-cli/tests/simulate.rs`, `simulate_play.rs` | CLI output end to end; manual steps and the prompt | — |
| `examples/order-fulfillment/scenarios/` | `happy-path`, `timeout-race` (immediate clock), `timeout-first`, `second-order-first` (a manual `{ step: 1 }`) | — |

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
  item keeps its FIFO position. (A race replay of a scenario with manual
  `step` entries applies the swap only to the queue items the simulator
  schedules itself.)
- A session driven by a scenario's actions has exactly the batch trace and
  payloads; `to_scenario` then `from_scenario` reproduces the trace, pending
  items and actions exactly.
- A rejected play action (or seek or branch switch) changes nothing.
- `Timeline::position <= Timeline::actions.len()`; every recorded
  `AddInstance` has a name.
- Instance names are unique across a run, removed instances included.
- `PendingId`s are unique within a line of play and stable while queued.
- Every `&Model` given to a session is the model it was built or last
  replayed with (ids are not checked against another model).
