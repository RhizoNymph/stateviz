# Semantics phase: contracts

The shared design for three changes that land together:

1. **Expressions.** Guards, rule conditions and selector filters become typed
   expressions over instance fields and event payloads, or explicit named
   choices. Fields and payloads can declare types. Transitions can set
   fields (`sets:`, #28) and shape emitted payloads (`with:`, #35); selectors
   can test the event (`event.x == lit`, #32); scenarios can pick a choice
   (#26).
2. **Exploration.** Every delivery ordering a scenario allows is explored,
   with partial-order reduction and state deduplication. The result is the
   set of distinct outcomes, each with a witness that replays in Play mode.
   Two ordering models are available: per-consumer FIFO (the default) and
   fully unordered. This covers handler-order dependence (#30).
3. **Machines that subscribe to events, and timers.** A transition can be
   triggered by an event directly (`on: { event: E, where: … }`), so a saga
   is just a machine. A transition can fire after a delay (`after: 30s`),
   with a virtual clock in the simulator and in Play mode.

This document is the contract between the workstreams. Each workstream also
writes or updates its own feature doc (listed under *Workstreams*).

## Non-scope

- Parallel regions (#33), history re-rooting (#34), find-or-create (#27),
  scenario expectations (#31), holds (#36), payload provenance (#29),
  conformance to recorded traffic (#37).
- Arithmetic in expressions; string functions; wall-clock time.
- A `ordered: true` flag on controllers (a follow-up once exploration
  exists).

## 1. Expressions

### Value types

```yaml
machines:
  Order:
    fields:                        # a list: untyped (as before)
      - orderId
    # or a mapping: name → type
    fields:
      orderId: string
      amount: number
      gift: bool
      tier: enum [basic, gold]
      note: any                    # same as untyped
events:
  OrderPaid:
    payload: { orderId: string, amount: number }   # or a list (untyped)
```

```rust
// cascade_core::expr
pub enum FieldType { Any, String, Number, Bool, Enum(Vec<String>) }
pub struct FieldDecl { pub name: String, pub ty: FieldType }
```

Runtime values stay strings: scenario payloads, instance fields and
`PlayAction` maps are unchanged (`BTreeMap<String, String>`). A type says how
a string is read when an expression uses it:

| Type | Accepted text | Compared as |
| --- | --- | --- |
| `any` | anything | text, or numbers when both sides parse as numbers (ordering ops only) |
| `string` | anything | text |
| `number` | a decimal number (`42`, `-1.5`) | `f64`, total order |
| `bool` | `true` / `false` | bool |
| `enum [a, b]` | one of the members | equality only; ordering ops are a type error |

Scenario validation checks declared instance fields and step payloads against
their types (`ScenarioErrorKind::InvalidValue { expected }`).

### Grammar

```text
expr     := or
or       := and ("or" and)*
and      := unary ("and" unary)*
unary    := "not" unary | "(" expr ")" | test
test     := operand cmp operand
          | operand ["not"] "in" "[" literal ("," literal)* "]"
cmp      := "==" | "!=" | "<" | "<=" | ">" | ">="
operand  := "event." NAME | "self." NAME | NAME | literal
literal  := NUMBER | QUOTED | WORD | "true" | "false"
```

- `event.x` is a field of the payload that came with the trigger (or the
  event being handled). `self.x` is a field of the instance in scope (see
  contexts).
- A bare `NAME` means a different thing on each side. On the **left** of a
  test in a selector or subscription filter it is `self.NAME` (the
  candidate), as today (`orderId == event.orderId`). Anywhere else it is a
  literal word (an enum member or a string). Guards must write `self.x`.
- Keywords `and`, `or`, `not`, `in`, `true`, `false` are reserved; quote
  them to use them as literals.

```rust
pub enum Operand { Event(String), SelfField(String), Lit(Literal) }
pub struct Literal { pub text: String, pub quoted: bool }
pub enum CmpOp { Eq, Ne, Lt, Le, Gt, Ge }
pub enum Expr {
    Cmp { left: Operand, op: CmpOp, right: Operand },
    In { operand: Operand, set: Vec<Literal>, negated: bool },
    Not(Box<Expr>),
    And(Vec<Expr>),   // ≥ 2 items
    Or(Vec<Expr>),    // ≥ 2 items
}
```

`Display` for `Expr` prints canonical text that parses back to an equal
`Expr` (minimal parentheses, single spaces, literals quoted only when
needed). Edit ops and the YAML patcher write this canonical form.

### Guards and conditions

```rust
pub enum Guard { Expr(Expr), Choice(String), Else }   // transitions
pub enum Condition { Expr(Expr), Choice(String) }      // rule `when:`
```

```yaml
guard: "event.amount > 0 and self.tier == gold"   # Expr
guard: "?card declined"                            # Choice, label "card declined"
guard: else                                        # Else
when:  "event.policy == sanctioned"                # Expr
when:  "?not a gift card"                          # Choice
```

- A guard or condition that is neither a valid expression, `?label` nor
  `else` is a load error (`DiagnosticKind::InvalidExpression`). There is no
  free text any more.
- A choice label is trimmed text after `?`, non-empty, any characters.

### Contexts: what `self` and `event` mean

| Where | `self.x` | `event.x` | bare left NAME |
| --- | --- | --- | --- |
| transition `guard` | the instance taking it | the trigger's payload | literal |
| subscription `where` | the candidate instance | the event's payload | `self.NAME` |
| rule `when` | not allowed (controllers are stateless) | the handled event | literal |
| rule target filter (`Machine where …`) | the candidate instance | the handled event | `self.NAME` |
| `new Machine with f = v` | not allowed | the handled event | (assignment target) |
| transition `sets: { f: v }` | the instance (before the transition) | the trigger's payload | (assignment target) |
| emit `with: { k: v }` | the instance (after `sets`) | the trigger's payload | (assignment target) |

The trigger's payload is the fire's event payload (controller fire), the
step payload (external fire), the event payload (subscription), or empty
(timer).

### Assignments

```rust
pub struct Assignment { pub field: String, pub value: Operand }
```

```yaml
- from: awaiting_content
  to: confirmed
  on: match
  sets: { fromAgent: event.originAgent, status: open }
  emits:
    - TransmissionConfirmed                              # plain
    - event: ChannelSuperseded
      with: { channelId: self.channelId, by: event.channelId }
```

- `sets` runs after the guard is chosen and before the emits. An assignment
  from a missing `event.x` or `self.x` leaves the field unchanged.
- An emitted payload is still the instance's fields (after `sets`) overlaid
  with the trigger's payload (today's rule), then each `with` entry is
  applied on top. A `with` from a missing value removes nothing and adds
  nothing.
- Spawn assignments (`new M with f = v`) use the same `Assignment` type.

### Typing (resolver)

The resolver type-checks every expression and assignment and reports
`Diagnostic`s, with the span of the containing string:

- `self.x` must be a declared field when the machine declares fields;
  `event.x` must be a declared payload field when the event declares a
  payload. For a transition's guard, the "event" is every event whose
  handlers can fire the trigger plus the step payloads of external sources;
  since that set is open, `event.x` in a guard is only checked when the
  trigger is a subscription (its event is known). Undeclared means `any`.
- Both sides of a comparison must have compatible types (`any` is
  compatible with all). Enum literals must be members. Ordering ops need
  `number` or `any` on both sides. `in` lists follow the same rules.
- An assignment's value type must be compatible with the field's type.
- New `DiagnosticKind`s: `InvalidExpression { message }`,
  `UnknownField { name, owner }` (reused if it exists),
  `TypeMismatch { left, right }`, `NotAMember { value, ty }`,
  `SelfNotAllowed`, `InvalidFieldType`, `InvalidDuration`,
  `TriggerKindConflict` (see §3).

### Evaluation (core)

```rust
// cascade_core::expr::eval
pub trait Scope {
    fn event(&self, field: &str) -> Option<&str>;
    fn self_field(&self, field: &str) -> Option<&str>;
}
pub fn eval(expr: &Expr, scope: &dyn Scope, types: &Types) -> bool;
pub fn value<'a>(operand: &'a Operand, scope: &'a dyn Scope) -> Option<&'a str>;
```

`Types` tells the evaluator how to read each `self.x` / `event.x` (from the
model's declarations; `any` when undeclared). Semantics:

- A test with a missing operand is **false** (`!=` included), so
  `not (event.x == a)` is true when `x` is missing. This matches today's
  selectors, where a missing side never matches.
- A value that does not parse as its declared type makes the test false and
  is reported by scenario validation beforehand.

### Static reasoning (core)

```rust
// cascade_core::expr::solve
pub enum Overlap { Exclusive, MayOverlap }
pub fn overlap(a: &Expr, b: &Expr, types: &Types) -> Overlap;
pub enum Coverage { Complete, Gaps(Vec<String>) /* readable examples */, Unknown }
pub fn coverage(guards: &[&Expr], types: &Types) -> Coverage;
```

A sound but incomplete decision procedure: normalize to DNF (bounded; give
up as `MayOverlap`/`Unknown` past a size limit), and decide each conjunction
over atoms on the same operand: equality/inequality and membership for
strings and enums (finite domains for enums), intervals for numbers, value
for bools. Two expressions are exclusive when every pair of their
conjunctions is unsatisfiable together. Atoms comparing two operands
(`self.a == event.b`) are treated as unknown (satisfiable).

## 2. Machines that subscribe to events, and timers

```yaml
Checkout:
  fields: [orderId]
  states: [waiting, paid, done, expired]
  transitions:
    - from: waiting
      to: paid
      on: { event: PaymentCaptured, where: "orderId == event.orderId" }
    - from: paid
      to: done
      on: { event: StockReserved, where: "orderId == event.orderId" }
      emits: [ReadyToShip]
    - { from: [waiting, paid], to: expired, after: 30m }
```

### Trigger kinds

```rust
pub enum TriggerKind {
    /// Fired by controller rules and external sources (today's triggers).
    Command,
    /// A subscription: delivering `event` offers this trigger to every live
    /// instance of the machine.
    Event(EventId),
    /// Fires `delay` after the instance enters the transition's source state.
    Timer(Duration),
}
// Trigger gains `kind: TriggerKind`; Event gains `subscribers: Vec<TriggerId>`.
```

- A subscription trigger is named after its event (`PaymentCaptured`); a
  timer trigger is named `after-<duration>` (`after-30m`). Triggers are still
  unique per `(machine, name)`. A name used with two kinds (a command
  `after-30m`, or a command named like a subscribed event) is
  `TriggerKindConflict`.
- Only command triggers may be fired by rules or listed by external sources
  (resolve error otherwise).
- `on:` and `after:` are mutually exclusive; exactly one is required.
- `where:` is only allowed on subscriptions. It is part of the transition,
  next to its guard: a transition is enabled for a delivery when its
  `where` holds (and then its guard decides as usual).
- When `events:` is strict, subscribed events must be declared, as handlers'
  events are.

```rust
pub struct Duration { millis: u64 }   // cascade_core::expr
// text: <integer><unit>, unit ∈ ms | s | m | h | d; Display picks the
// largest unit that divides exactly (90s → "90s", 120s → "2m").
```

### Delivering an event

When an event is delivered, its **consumers** are, in this order:

1. subscriptions: for each subscribing machine in model order, each live
   instance of it (creation order);
2. controller handlers in definition order (as today).

An instance consumes the event by offering it the subscription trigger:
`enabled_transitions(leaf, trigger)` filtered by `where` (and then the
guard); none means **ignored**, which is not recorded (an instance that is
not interested in an event is the normal case). In the trace a subscription
that is taken records `Notify { instance, event }` (cause: the `Emit`), then
the `Transition` (cause: the `Notify`).

### Timers

- Entering a state arms one timer per timer trigger accepted from that state
  (its own transitions, not its ancestors'; ancestors already armed theirs
  when they were entered). Leaving a state cancels its timers. A self
  transition leaves and re-enters (re-arms). An instance created in a state
  arms the timers of every state entered by default entry.
- An armed timer has a deadline `now + delay`. The clock only moves by an
  explicit advance: scenario `- { wait: 30s }` / `- wait` (to the next
  deadline) and `PlayAction::Advance`. Timers due at the new time are queued
  in deadline order (ties: arming order) as `Timer` queue items. A queued
  timer whose state has since been left is delivered anyway and is dropped
  if not accepted (the event already happened).
- Trace: `Wait { by, now }` (scenario step, no cause), then
  `TimerExpired { instance, trigger }` (cause: the `Wait`) when queued, and
  the `Transition`/`Dropped` when delivered (cause: the `TimerExpired`).
- `end: drain` drains the queue; it never advances time.

### Causal graph

New edge kinds:

```rust
CausalEdgeKind::Notify { trigger: TriggerId }  // Event → Transition (subscription)
CausalEdgeKind::Delay { trigger: TriggerId }   // Transition → Transition (timer)
```

- `Notify`: event → every transition accepting the subscription trigger.
- `Delay`: from each transition whose target is the timer transition's
  source state or inside it, to the timer transition. (Entering the state
  arms the timer.) The machine's initial entry has no node; a timer on the
  initial state has no incoming `Delay` edge.
- `transition_successors(t)` returns `(successor, Option<RuleId>)`: `None`
  for `Notify` links. `Delay` links are not successors (a timer is not a
  cascade).
- `CycleStep { transition, rule: Option<RuleId> }`.

## 3. Choices (decisions the data doesn't carry)

A **choice point** happens during a delivery when:

- a trigger has several enabled candidates after evaluating `Expr` guards
  and `where` filters, and some remaining candidates have `Choice` (or
  `Else`) guards: the options are the remaining candidates' labels in
  definition order (`else` is the label `else`), or
- a rule with a `Choice` condition is applied: the options are the label and
  `not <label>`.

```rust
// cascade_sim
pub struct ChoicePoint { pub at: String /* instance or controller name */, pub options: Vec<String> }
```

Decisions:

- The session keeps a FIFO of **pending decisions** (option strings). At a
  choice point the first pending decision that is one of the options is
  consumed and taken. Without one, the **first option** is taken (today's
  behaviour) and the step is marked `defaulted`.
- Scenario directive `- { choose: "card declined" }` and
  `PlayAction::Choose { option }` push a pending decision. Decisions still
  pending when the run ends are a scenario diagnostic
  (`UnusedChoice`) in batch runs and are dropped in a session.
- Trace: `Choice { at: LifelineIx, options: Vec<String>, chosen: String, defaulted: bool }`,
  cause the delivering step; the transition or rule outcome that follows
  has the `Choice` as its cause.
- Mixed groups (an `Expr`-guarded candidate that holds next to `Choice`
  ones) are a choice point among all of them, the expression's label being
  its canonical text. The nondeterminism check flags these.

## 4. Per-consumer delivery

An event in the queue is delivered to its consumers. Today the whole
delivery is one step. Now:

- `PlayAction::Step { choice, consumer: None }` delivers the item to all of
  its remaining consumers in order (today's behaviour, unchanged traces).
- `Step { choice, consumer: Some(name) }` delivers an event only to the named
  consumer (a controller name, or an instance name for a subscription). The
  item stays queued until every consumer has had it.
  `PendingItem::consumers_left: Vec<String>` lists them.
- Fires and timers have one consumer (their target); `consumer` must be
  `None` or that instance.
- Scenario directive: `- { step: n, consumer: Fulfillment }`.

## 5. Session and scenario additions

```rust
pub enum PlayAction {
    AddInstance { .. }, RemoveInstance { .. }, Fire { .. }, RunUntilQuiet,  // unchanged
    Step { choice: Option<u32>, consumer: Option<String> },
    /// Advance the clock: by a duration, or to the next deadline (None).
    /// Due timers join the queue. Nothing armed and `by: None` is an error
    /// (`NoTimerArmed`).
    Advance { by: Option<Duration> },
    /// Push a pending decision (§3).
    Choose { option: String },
}

pub enum PendingKind {
    Event { event: EventId },
    Fire { rule: RuleId, target: LifelineIx },
    Timer { trigger: TriggerId, target: LifelineIx },
}

impl PlaySession {
    pub fn now(&self) -> Duration;
    pub fn timers(&self) -> Vec<ArmedTimer>;          // instance, trigger, deadline, in deadline order
    pub fn pending_decisions(&self) -> &[String];
    /// The choice points `Step { choice, consumer }` would hit with the
    /// current pending decisions, without changing anything.
    pub fn choice_points(&self, model: &Model, choice: Option<u32>, consumer: Option<&str>)
        -> Result<Vec<ChoicePoint>, SimError>;
    /// A canonical key of everything that affects the future (instances,
    /// fields, leaves, history, queue contents in order with consumers
    /// left, armed timers relative to now, pending decisions, removed
    /// names). Ids, labels and the trace are excluded. Two sessions with
    /// equal keys behave the same under the same future actions.
    pub fn state_key(&self) -> StateKey;              // Eq + Hash + Clone
}
```

Scenario format additions:

```yaml
steps:
  - { wait: 30s }                       # advance the clock (after-quiescence by default)
  - { wait: 30s, timing: immediate }
  - wait                                # to the next deadline
  - { choose: "card declined" }
  - { step: 0, consumer: Fulfillment }
```

`to_scenario` writes `Advance` as `wait`, `Choose` as `choose`, and
`Step { consumer }` with `consumer:`.

New `TraceStepKind` variants (the trace view draws them; `describe` gives
them text):

```rust
Notify { instance: LifelineIx, event: EventId },
Wait { by: Duration, now: Duration },
TimerExpired { instance: LifelineIx, trigger: TriggerId },
Choice { at: LifelineIx, options: Vec<String>, chosen: String, defaulted: bool },
SetFields { instance: LifelineIx, fields: BTreeMap<String, String> },   // after a Transition with `sets`
```

## 6. Exploration

```rust
// cascade_sim::explore
pub enum OrderingModel {
    /// Each consumer sees events in emission order; each instance receives
    /// one controller's fires in order; everything else interleaves,
    /// including the consumers of one event.
    PerConsumer,
    /// Any queued item (and any consumer of a queued event) may go next.
    Unordered,
}
pub struct ExploreOptions {
    pub ordering: OrderingModel,
    /// Branch on every option at choice points (otherwise use defaults and
    /// pending decisions only).
    pub choices: bool,
    /// Let armed timers expire between any two deliveries (otherwise only
    /// when the scenario waits).
    pub timers_anytime: bool,
    pub max_states: usize,     // default 100_000
}
pub fn explore(model: &Model, scenario: &Scenario, options: &ExploreOptions) -> Result<Exploration, SimError>;
pub fn explore_from(session: &PlaySession, model: &Model, options: &ExploreOptions) -> Result<Exploration, SimError>;

pub struct Exploration {
    pub outcomes: Vec<Outcome>,       // distinct, sorted by witness length then text
    pub states: usize,                // distinct states visited
    pub complete: bool,               // false when max_states was hit
}
pub struct Outcome {
    /// Every instance (by name, removed included) with its final leaf and fields.
    pub instances: BTreeMap<String, (StateId, BTreeMap<String, String>)>,
    /// What went wrong on the way: NoTarget, Ambiguous, Dropped (with labels).
    pub anomalies: BTreeSet<Anomaly>,
    /// Actions that reproduce it from the exploration's start, for a
    /// PlaySession (appended to `explore_from`'s timeline).
    pub witness: Vec<PlayAction>,
    pub paths: u64,                   // how many explored paths ended here (saturating)
}
pub enum Anomaly {
    NoTarget { rule: String },
    Ambiguous { rule: String, candidates: Vec<String> },
    Dropped { instance: String, trigger: String, state: String },
    StepLimit,
}
```

- The scenario's external fires, waits and choices are the inputs, applied
  in file order at the scenario's timing (`after-quiescence` waits for
  quiet). Manual queue directives (`step`, `{ step: n }`, `run`) are
  ignored by exploration (they choose orderings, which exploration
  enumerates) and reported as a note.
- An outcome is a terminal state: inputs used up and the queue empty (and,
  with `timers_anytime`, no timer armed, or the remaining timers left
  armed; they are part of the outcome key).
- Moves: deliver an eligible item (or one consumer of an event) under the
  ordering model; at choice points, one move per option when `choices`;
  with `timers_anytime`, expiring the earliest armed timer.
- Search: DFS over `PlaySession` clones with a visited set of `StateKey`s,
  plus sleep-set partial-order reduction (two moves are independent when
  they touch different instances and neither spawns, removes, or changes a
  field a selector of the other reads; when unsure, dependent). Reduction
  must never lose an outcome: tests compare with reduction disabled.
- The witness is the shortest path found to the outcome (BFS order is not
  required; keep the shortest seen).

CLI:

```text
cascade explore <definition> <scenario> [--unordered] [--choices] [--timers]
                [--max-states N] [--format text|json]
```

Exit codes: 0 one outcome and no anomalies; 1 several outcomes or any
anomaly; 2 errors; 3 incomplete (bound hit). Text output lists each outcome
(the instances that differ from the first outcome, anomalies, witness as
scenario lines).

The app's Play mode gets an **Explore** panel: explore from the current
position, list outcomes, and open one as a branch of the timeline (the
witness applied from the current position).

## Workstreams

| Branch | Owns | Delivers | Feature doc |
| --- | --- | --- | --- |
| `feat/lang-contracts` (wave 1) | cascade-core: `expr` (types, parse, Display, eval, solve), definition/model/parse/resolve changes, `TriggerKind`, causal edge kinds, edit ops and diff follow-through; every other crate updated just enough to compile with today's behaviour | the contract types and the language | `expressions.md` (new), `definition-format.md`, `causal-graph.md` |
| `feat/guard-analysis` (wave 2) | cascade-core `analysis` | nondeterminism via `overlap`, new `uncovered-guard` info check via `coverage`, races excluding exclusive conditions, reachability and cycles for subscriptions and timers | `static-analysis.md` |
| `feat/sim-semantics` (wave 2) | cascade-sim (except `explore`), `cascade simulate` | evaluation, `sets`, `with`, choices, subscriptions, timers and clock, per-consumer steps, scenario additions, `state_key`, `choice_points`, trace kinds | `simulator.md`, `timers-and-subscriptions.md` (new) |
| `feat/interop-semantics` (wave 2) | cascade-interop | YAML patch/render and `to_yaml` for every new key; SCXML (`cond`, `<send delay>` for timers), Mermaid labels, P export; XState import (`after`, guards as choices) | `interop-and-diff.md` |
| `feat/scene-semantics` (wave 2) | cascade-scene | labels for guards/timers/subscriptions/sets, `Notify`/`Delay` edges in causal and structure views, new trace step kinds | `view-scenes.md` |
| `feat/app-semantics` (wave 2) | cascade-app | inspector editing (typed fields, guard/condition expressions with live errors, sets, emit with, `on` event / `after`), Play mode clock (now, timers, advance), choice prompt, per-consumer delivery in the queue | `native-app.md`, `build-and-play.md` |
| `feat/explore` (wave 3) | cascade-sim `explore`, `cascade explore`, the app's Explore panel | §6 | `exploration.md` (new) |

Wave 1 ships the types (this document's Rust blocks), the parser, resolver,
evaluator and solver, with `todo!()`-free stubs elsewhere so the workspace
builds and every existing test passes (existing guards in examples and
fixtures are rewritten to expressions or `?choices`). Wave 2 branches stack
on wave 1 and run in parallel; wave 3 stacks on the wave 2 integration.

## Interop as landed (wave 2, `feat/interop-semantics`)

- **SCXML.** Timers: `<onentry><send event="M.after-30m" delay="1800s"
  id="<state id>.after-30m"/>` + `<onexit><cancel sendid/>` in the source
  state and a transition on `M.after-30m`. Subscriptions: a transition on
  the bare event name; `cond` = filter (plain context) `and` an expression
  guard, for engines; `cascade:where`/`cascade:guard` carry them apart.
  Choices stay `cond="?label"` with an XML comment before the transition.
  `sets` → `<assign location expr>`, emit `with` → `<param name expr>`,
  typed fields/payloads → `<cascade:field name type/>` children. Import
  reads all of these back; in foreign documents a delayed `<send>` in
  `<onentry>` whose event a transition of the same state takes is a timer,
  and `<assign>`/`<param>` are kept only when `expr` is `event.x`/`self.x`.
- **XState.** Numeric `after` delays (> 0) are Cascade timers (no `Clock`
  source); named delays stay `after_NAME` commands fired by `Clock`.
  `guard: { type, params }` → `?type` (parameters dropped). `assign` with a
  static `assignment` copying `event.x` → `sets`; other assignments and
  `assign` in entry/exit are ignored with a warning.
- **Mermaid.** Structure labels: `after 30s`, `on Event where …`, guards as
  canonical text. Causal: notify edges `e ==>|where …| t`, delay edges
  `t -.->|after 30s| t` with a dashed stroke, from `CausalGraph::build`.
- **P.** `number` → `float`; enums → P `enum`s; instance fields → machine
  variables; expression guards over typed instance fields and rule
  conditions over a typed payload → P conditions, else `$`; `else` → the
  `else` branch; `sets` → assignments; emits also go to subscribers;
  timers → nondeterministic self-send on entry (TODO comment for a timer
  machine).

## Wave 1 as landed

Names and details the Rust blocks above leave open, as implemented on
`feat/lang-contracts` (wave 2 builds on these):

- **Parsing.** `ExprContext::{Plain, Filter}`; `parse_expr(text, ctx)`,
  `parse_guard`, `parse_condition`, `parse_operand` (assignment values),
  `parse_field_type`, `parse_duration` all return `Result<_, ExprError>`
  with `ExprError { message, column }` (1-based character column).
  `Expr::display_filter()` prints filter-context canonical text (left
  `self.x` as `x`); `TargetSpec`'s `Display` uses it.
- **Literals.** `Literal::new(text)` quotes only when needed;
  `Literal::quoted`. A quoted literal is always a string for typing
  (`"42"` is not a number). `true`/`false` unquoted are bool literals.
- **Types.** `Types::new(self_fields, event_payload)`, `Types::untyped()`,
  `Model::types_for(machine, Option<EventId>)`, `Model::event_types(event)`;
  `MapScope { event, self_fields }` implements `Scope` over the simulator's
  maps. `compare(left, op, right, ty)` and `literal_fits` are public.
- **Definition.** `TransitionDef { from, to, trigger: TriggerSpec, guard:
  Option<Spanned<Guard>>, sets: Vec<Spanned<Assignment>>, emits:
  Vec<EmitDef>, bounded, span }` with `TriggerSpec::{Command(name),
  Event { event, filter: Option<Spanned<Expr>> }, After(Spanned<Duration>)}`
  and `TriggerSpec::name()`; `EmitDef { event, with }`; `FieldDef { name, ty:
  Option<Spanned<FieldType>> }` (`None` = list form; `fields_are_typed`);
  `RuleDef.when: Option<Spanned<Condition>>`; `TargetSpec::{One { machine,
  filter }, All { machine, filter }, Spawn { machine, assignments }}` with
  `mode()`, `machine()`, `machine_mut()`, `filter()` (`TargetMode` stays as
  the mode tag). `EditOp::SetMachineFields` carries `Vec<FieldDef>`.
- **Model.** `Transition { guard, filter, sets, emits: Vec<Emit>, .. }`
  (`filter` only on subscriptions; an event emitted twice by one transition
  is kept once); `TriggerKind::{Command, Event(EventId), Timer(Duration)}`
  with `describe()`; `Event.subscribers`; `Model::subscription_trigger`.
- **Diagnostics.** `InvalidExpression { text, message }`, `InvalidFieldType
  { text, message }`, `InvalidDuration { text, message }` (also a zero timer
  delay), `TypeMismatch { text, left, right }`, `NotAMember { value, ty }`,
  `SelfNotAllowed { text, context }`, `TriggerKindConflict { machine,
  trigger, first, second }` (also when a rule or external source names a
  subscription or timer trigger), and one addition: `ConflictingKeys {
  context, first, second }` for `on:` with `after:` (neither is `MissingKey
  on`). `UnknownField` / `UnknownPayloadField` are reused, also for `sets`
  targets and `with` keys; duplicate field names are `Duplicate { what:
  "field" }`.
- **Causal graph.** `Delay` edges come from transitions that *enter* the
  timer's source state: the contract's rule (target is the state or inside
  it) plus default entry below the target and history targets whose parent
  contains the state, all within the timer's machine.
- **Coverage.** `Coverage::Gaps` examples read `self.tier = vip, event.amount
  is missing`; no expressions at all is `Gaps(["any input"])`.
- **Simulator (wave 1 stub).** A trigger takes the first candidate in
  definition order whose `Expr` guard is not false (`Choice`/`Else` stay
  candidates); command guards read `event.x` as `any`. A rule whose `Expr`
  condition is false does not apply (no trace step); a `Choice` condition
  holds. Subscriptions and timers never fire; `sets`/`with` are not applied.
- **Interop, scene, app (wave 1).** Canonical text everywhere a guard or
  condition used to be shown (so choices show their `?`). Importers produce
  `?label` choices, except that text already in Cascade syntax (`else`,
  `?label`, or an expression reading `event.x`/`self.x`) is kept, so a
  Cascade SCXML export imports back unchanged. The YAML writer quotes guard,
  `when:` and `where:` text only when YAML needs it (a `?choice` always
  does). The structure view does not draw `Notify`/`Delay` wires yet; the
  causal view draws them like subscribe/trigger links. The app inspector
  parses guard/`when` text on commit and shows parse errors inline; field
  and payload lists keep the types of names that stay.

### Simulator contract types (wave 1 follow-up)

Added on `feat/lang-contracts` so wave 2 branches compile against them; no
semantics yet:

- `cascade_sim::{ChoicePoint, ArmedTimer, StateKey}` (lib root).
  `ChoicePoint { at: String, options: Vec<String> }`; `ArmedTimer {
  instance: String, trigger: TriggerId, deadline: Duration }`; `StateKey` is
  an opaque newtype (`Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash`)
  over a canonical string (instances, fields, leaves, sorted history,
  removed names, queue in order with consumers left). Wave 2 extends it
  with timers relative to now and pending decisions.
- `PlayAction::Step { choice, consumer }`, `Advance { by }`, `Choose {
  option }`; `exec` rejects `Advance`, `Choose` and `consumer: Some(_)` with
  `SimError::Action(ScenarioErrorKind::Unsupported { what })`.
- New `ScenarioErrorKind`s: `Unsupported { what }`, `NoTimerArmed`,
  `InvalidDuration { text, message }` (scenario `wait:` text).
- `PendingKind::Timer { trigger, target }`; `PendingItem::consumers_left`
  is filled (event: controller names in handler order, deduplicated; fire:
  the target's name).
- `Directive::Deliver { choice, consumer: Option<Spanned<String>>, span }`,
  `Directive::Wait { by: Option<Spanned<Duration>>, timing: StepTiming,
  span }` (`- wait`, `- { wait: 30s }`, `- { wait: ~, timing: immediate }`),
  `Directive::Choose { option: Spanned<String>, span }`. The parser accepts
  them; `validate` rejects them as `Unsupported` at their span.
  `scenario_to_yaml` writes them; there is no file spelling for "the head,
  one consumer", so `Step { choice: None, consumer: Some(c) }` saves as `{
  step: 0, consumer: c }` (wave 2 may record it as `choice: Some(0)` so the
  round trip is exact).
- `TraceStepKind::{Notify, Wait, TimerExpired, Choice, SetFields}` as in §5;
  `step_text` covers them. The trace view draws nothing for them yet (an
  empty row); the app's `step_element` became `Option<ElementRef>` (`None`
  for `Wait`, `Choice`, `SetFields`); `cascade simulate --format json`
  writes them as `notify`, `wait`, `timer-expired`, `choice`, `set-fields`.
- `PlaySession::{now, timers, pending_decisions, choice_points}` return
  zero / empty / `Ok(vec![])`; `state_key` is real. (Superseded by wave 2,
  below.)

### Simulator semantics (wave 2, `feat/sim-semantics`)

As landed in cascade-sim; the details the sections above leave open:

- **Picking a transition.** Candidates are `enabled_transitions` whose
  `where` holds. A choice point happens only when `?choice` candidates
  remain *and* at least two candidates remain; its options are the
  remaining candidates' labels in definition order: a choice's label, a
  holding expression's canonical text, `unguarded` for a candidate with no
  guard, `else`. A single remaining candidate (even a `?choice`) is taken
  without a choice point. Without choices, several holding expressions
  take the first in definition order (no choice point; the nondeterminism
  check flags them) and `else` is taken only when nothing else holds.
- **Choice steps.** `Choice.at` is the instance (guards) or the controller
  (conditions). Options for a condition are `label` and `not label`.
- **Event consumers are fixed when the event is queued** (the subscribing
  instances alive at emit time, then controllers); a removed instance drops
  out, an instance created later is not added. A whole delivery equals
  per-consumer deliveries in order. Each per-consumer delivery counts as
  one delivery for the step limit.
- **Per-consumer steps are recorded as `Step { choice: Some(position),
  consumer }`** (the head as `Some(0)`), so saving round-trips exactly. A
  consumer not among `consumers_left` is the new
  `ScenarioErrorKind::UnknownConsumer { consumer, item, consumers }`; a
  fire or timer with `consumer: Some(target)` is delivered whole.
- **Timers.** Entering and leaving are computed from the transition's
  domain (the nearest state properly containing source and target); timers
  of a state are its own timer transitions' triggers, deduplicated, in
  definition order; ties between equal deadlines go by arming order (outer
  state first on creation). `Advance { by: Some(_) }` with nothing armed is
  allowed (the clock moves); `Advance { by: None }` is `NoTimerArmed`.
  Removal cancels armed timers and discards queued timer items.
- **Unused decisions.** Batch runs report `ScenarioErrorKind::UnusedChoice
  { option }` at each leftover `choose` directive (the run fails with
  `SimError::Scenario`); `PlaySession::from_scenario` keeps them pending.
- **Value typing.** `ScenarioErrorKind::InvalidValue { field, value,
  expected }` (expected from `FieldType::expected`): instance fields
  against the machine's declarations; step payload entries against the
  fired machine's field of that name and the declared payload field of
  that name of every event emitted by a transition accepting the trigger.
  `AddInstance` and `Fire` actions are checked the same way.
- **`choice_points`** runs the step on a copy of the session; it returns
  the same errors the step would (e.g. `QueueEmpty`).
- **`state_key`** adds timer queue items, armed timers (instance, trigger,
  arming state, `deadline - now`) and pending decisions; the absolute clock
  is excluded.
- **Race replays** treat a `Choice` caused by a contested fire as that
  fire's delivery; findings other than `RaceCandidate` (including the
  analysis branch's subscription races) return `SimError::NotARace`.
- `ScenarioErrorKind::Unsupported` is kept but no longer returned.

## Wave 2 guard analysis as landed

On `feat/guard-analysis` (static-analysis.md has the details):

- **New check** `Check::UncoveredGuard` (`uncovered-guard`, info, last in
  `Check::ALL`, which now has 9 entries) with `FindingDetail::UncoveredGuard
  { state, trigger, transitions, gaps }`. Gap examples made only of missing
  fields are dropped.
- **Subscription races**: a new `FindingDetail::SubscriptionRace { origin,
  machine, first: RaceParty, second: RaceParty }` (check `RaceCandidate`)
  with `pub enum RaceParty { Rule(RuleId), Subscription(TriggerId) }`
  (`RaceParty::element()`), re-exported from the crate root. Rule-only pairs
  stay `RaceCandidate` with `RuleId`s, so the simulator's race replay is
  unchanged; it returns `SimError::NotARace` for a `SubscriptionRace`
  (cascade-sim may add replay for it later). Code that indexes race findings
  by `check() == RaceCandidate` sees both variants.
- **Constant tests**: `DiagnosticKind::ConstantTest { test, what:
  &'static str, suggestion: Option<String> }`, a load error for a test that
  reads no field (`x > 0` with a bare `x`, `1 == 1`, a literal `in`). Fixtures
  and tests with such guards now use `self.x` / `event.x`.
- Unchanged messages: unhandled-event still says "no controller subscribes
  to it" (it is only reported when no machine subscribes either), and
  unreachable-state still says "through external triggers or controller
  fires". The shop and order-fulfillment findings are unchanged.

### Scene semantics as landed (wave 2, `feat/scene-semantics`)

- **Scene types.** `Label::style: FontStyle { Normal, Italic }` (choices
  and `else` italic; a label carrying a choice is italic as a whole);
  `EdgeKind::{Notify, Delay}`; `CLOCK_GLYPH` = `◷` (U+25F7: DejaVu Sans
  Mono and the other monospace faces carry it at one advance; `⏱` does
  not). Backends must render `FontStyle::Italic` (SVG writes
  `font-style="italic"`; the GPUI painter needs `.italic()`).
- **Labels.** Pills: `from → to` over `capture` / `on Event` (+ a `where
  …` line in filter-context canonical text) / `◷ 30s`; the pill's way out
  carries `sets a, b`. Arrow mode: `trigger [where …] [guard] / sets a, b`.
  Causal `Delay` edges: `◷ 30s [guard]`.
- **Links.** Structure view (view mode): subscription links emitter pill →
  subscriber pill as `Notify` edges labelled with the event; edit mode:
  `Notify` wires event tag → subscribing pill. Timers get no wire. The
  matrix counts subscription links like rule links (timers never).
- **Cycles.** Cycles through `Notify`/`Delay` edges are drawn as back edges
  (red) like cascade cycles; a timer that re-arms itself is a red
  self-loop. Causal lanes keep "no leftward arrows" except back edges.
- **Subscribe affordance.** A new `HitTarget::SubscribeHandle { event:
  String }` (not `ConnectHandle { element: Event }`, because existing
  scene tests pin connect handles to states, transitions, controllers and
  sources): an `Overlay::Rect` circle (radius 4.5) on every event tag's
  point in `SceneMode::Edit`; `Scene::hit_test`/`hit_test_arrows` return it
  before nodes (`HitTarget::is_handle`). Drop on a transition pill
  (`Element(Transition)`) or, in arrow mode, the arrow
  (`ConnectHandle { element: Transition }`). The app maps it to the drag
  source `ElementKey::Event { event }` in `build::pick::connect_source`
  and handles `(Event, Transition)` drops; on this branch `locate.rs` only
  got the match arms needed to compile (selects the event, not a drop
  key). Subscribe handles are exported (connect handles are not).
- **Play.** `PlayOverlay::timers: Vec<PlayTimer { instance, trigger:
  ElementKey, remaining: Duration }>`, drawn as `◷ 25s` / `◷ due` chips
  after the instance marker; the app fills it from `PlaySession::timers`
  minus `now` (this branch sets `timers: Vec::new()` in the app).
- **Trace.** `Notify` delivery arrow, `Wait` full-width divider
  `◷ +30s · now 1m`, `TimerExpired` clock self-message, `Choice` note
  `◇ chosen (default)` over `of a | b`, `SetFields` note `set f = v`.

### App (`feat/app-semantics`, wave 2)

As landed in `cascade-app` (details in build-and-play.md):

- **Typed fields in the inspector** are one text field in YAML terms
  (`id, amount: number, tier: enum [a, b]`); no `:` anywhere means the list
  form. This replaces wave 1's "keep the types of names that stay": the
  text shows every type, so it is the whole truth.
- **Live type feedback** plans the edit and runs `edit::apply` on a copy;
  the diagnostics' kinds (not spans) are shown under the field.
- **Choice prompt** relies on `PlaySession: Clone`, `apply(Choose)` and
  `choice_points` returning every point the step hits (decided or not): the
  app works out which are undecided from `pending_decisions()` (first
  matching decision, oldest first, one per point). If `choice_points`
  returns only undecided points instead, the prompt still works. Run until
  quiet never prompts (defaults).
- **Per-consumer delivery** buttons appear for `PendingKind::Event` items
  with more than one `consumers_left`; the head is sent as `choice: None`.
- **Drag-to-connect** onto a transition from an event (or from another
  machine's transition) offers subscribing, plus rules of controllers
  handling the event. The app reads emit handles as
  `HitTarget::ConnectHandle { element: ElementKey::Event { .. } }`; a
  different hit target shape needs mapping in `build::pick::connect_source`
  and `locate::drop_key` only.
- Keys: `T` advances the clock to the next timer; with a prompt open,
  Space takes the first option.

## Invariants

- No free text in guards or conditions: every one parses as `Guard` /
  `Condition`.
- `Expr` display → parse is the identity; canonical text is what the
  patcher writes.
- With no choices, timers or subscriptions in a model, and no new scenario
  directives, simulation traces are identical to today's except where a
  guard expression now evaluates (the first *enabled* candidate, not the
  first declared, is taken).
- Exploration never reports an outcome no `PlaySession` can reach: every
  witness replays to exactly its outcome (tested).
- Partial-order reduction never loses an outcome (tested against no
  reduction).
