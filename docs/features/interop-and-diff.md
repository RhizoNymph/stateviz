# Interop and diff

## Scope

- Writing a definition back to YAML in the spec's compact style.
- Importing XState v5 machine configs (JSON) and SCXML documents into a
  definition that resolves and renders without hand edits.
- Exporting a model to SCXML, Mermaid (structure and causal graph) and a P
  language skeleton, including the semantics-phase features: guards,
  timers, subscriptions, `sets`, emit `with` and typed fields (see
  expressions.md and semantics-contracts.md).
- Comment-preserving YAML patches for every edit op, including the new
  keys (build-and-play.md, "Saving edits (YAML patch)").
- Comparing two versions of a definition (`diff_models`) and merging them
  into one displayable model with removed elements as ghosts
  (`merge_for_display`).
- Reading a definition file at a git revision (`read_at_rev`).
- The CLI commands `cascade export`, `cascade import` and `cascade diff`.

## Non-scope

- Drawing diff mode (green outlines, red ghosts): `cascade-scene` and the
  app consume `ModelDiff` and the merged model.
- SVG/PNG export (`cascade render`, see view-scenes.md).
- Extracting machines from code (a later experiment in the spec).
- Executing or verifying anything: guards and conditions are written as
  their canonical text (see expressions.md), the P output is a skeleton
  that was never compiled here.

## Public interface

```rust
// cascade_interop
pub fn import(format: ImportFormat, text: &str) -> Result<Imported, InteropError>;
pub fn import_with(format: ImportFormat, text: &str, options: &ImportOptions) -> Result<Imported, InteropError>;
pub fn export(format: ExportFormat, model: &Model) -> Result<String, InteropError>; // never fails today
pub fn to_yaml(definition: &Definition) -> String;
pub fn read_at_rev(file: &Path, rev: &str) -> Result<String, GitError>;

pub struct Imported { pub definition: Definition, pub warnings: Vec<ImportWarning> }
pub struct ImportOptions { emit_prefix, environment_source, clock_source, router_controller }
pub enum ImportFormat { XState, Scxml }                  // --from xstate|scxml
pub enum ExportFormat { Scxml, Mermaid, MermaidCausal, P, Yaml } // --to scxml|mermaid|mermaid-causal|p|yaml

// cascade_core::diff
pub fn diff_models(old: &Model, new: &Model) -> ModelDiff;
pub fn merge_for_display(old: &Model, new: &Model) -> Result<(Model, ModelDiff), LoadError>;
```

Errors are typed: `InteropError::{Syntax { line, col }, Invalid { location },
Unsupported { location, what }}`, `GitError::{Spawn, InvalidPath, InvalidRev,
NotARepository, OutsideRepository, Show, NotUtf8}`. Import warnings are
`ImportWarning { location, kind: Renamed | Ignored | Approximated }`.

## Data and control flow

### Import

```text
XState JSON ──serde_json──▶ Json (ordered) ──xstate::parse──┐
                                                           ├─▶ Vec<Chart> + Wiring ──lower──▶ Definition
SCXML text ──roxmltree──▶ Document ──scxml::read───────────┘        │
                                                          warnings ◀┘
```

1. The format parser builds one `Chart` per machine: a tree of nodes with
   raw names, kinds, explicit initial states, entry/exit emits and edges
   (trigger spec, resolved target node, guard, own emits, `reenter`). Targets
   and `initial` values are resolved after the whole tree (and every id) is
   known. Anything the IR cannot hold fails here with `Unsupported`.
2. `import::lower` names everything through `NameTable`s (sanitize invalid
   characters to `_`, prefix a leading digit, suffix collisions; each change
   is a `Renamed` warning), writes the `StateDef` tree, turns edges into
   `TransitionDef`s with full state paths, and attributes entry/exit actions
   to transitions: exits innermost first, then the transition's own actions,
   then entries outermost first down the target's default entry chain
   (`attributed_emits`).
3. `import::wiring` adds controllers and sources. For formats without
   Cascade wiring it synthesizes a routing controller and external sources
   (below). For Cascade-annotated SCXML it restores the annotated
   controllers, declared events and sources.
4. `cascade import` prints warnings to stderr, writes `to_yaml(definition)`,
   and validates the text with `load_str` before writing (exit 2 if the
   importer ever produced something that does not load).

Synthesized wiring:

| Emitted by | Routed by `EventRouter` to |
| --- | --- |
| XState `raise` | the emitting machine |
| XState `sendTo` with `to: "Id"`, SCXML `<send target="#_Id">` | machine `Id` (a warning if it is not imported) |
| XState `sendParent`, `emit`; SCXML `target="#_parent"` or external targets | every other machine |
| `emit:Name` actions; SCXML `<raise>` and untargeted `<send>` | every machine, the emitter included |

A rule `fire: M.E` is added for each machine `M` reached that handles `E`
(by the raw event name). Then external sources give every trigger a cause:
`Environment` fires every named-event trigger that no routed emit covers
(plus approximated completion triggers), `Clock` fires every `after_*`
trigger, and each invoked actor is a source firing its `<actor>_done` and
`<actor>_error` triggers. Empty sources are omitted. Names are configurable
through `ImportOptions`.

### Export

`export` dispatches on the format; every exporter reads the resolved
`Model` (the YAML writer reads `model.definition()`), so exports see
resolved paths, expanded multi-source transitions and derived indexes.

### Diff

`diff_models` builds a map `ElementKey → attributes` for both models
(`diff/compare.rs`) and classifies every key: new only → Added, old only →
Removed, both with different attributes → Changed. Unchanged keys are
omitted from `ModelDiff`.

### Merge for display

`merge_for_display` (`diff/merge.rs`):

1. Clone the new definition, so new elements keep their spans.
2. Rewrite its state references to full paths and make every initial state
   explicit (a ghost could make a unique local name ambiguous, or become a
   default initial by being inserted first).
3. Rebuild each removed element from the old model (`diff/rebuild.rs`: full
   paths, unknown spans) and insert it next to its old neighbours: whole
   machines, state subtrees, transitions (appended, so duplicate-transition
   ordinals line up), controllers, handlers, rules, sources.
4. Keep the union resolvable: drop ghosts that cannot exist in the new
   structure (children or outgoing transitions of a state that became final
   or history); in strict-events mode declare removed events; widen field
   and payload lists that ghost selectors use.
5. Resolve (returning the `LoadError` if the union still fails), then give
   every element the span it has in the new file; ghosts have none.

Diff mode in the app and `cascade diff` load the old text with
`read_at_rev`: `git -C <dir> rev-parse --show-toplevel`, the file's path
relative to that (both canonicalized), then `git show <rev>:<relpath>`.

## Mapping tables

### XState v5 → Cascade

Input: one config, an array of configs, or `{ "machines": { "Name": config } }`.
A machine is named by its `id`, else its key, else `Machine`.

| XState | Cascade |
| --- | --- |
| `states` (nested) | states, in document order |
| `initial` (`"key"` or `{ target }`) | machine: `initial` path; compound state: `initial` child |
| `type: "final"` | `kind: final` |
| `type: "history"`, `history: "deep"` | `kind: history` / `deep-history`; a history `target` is ignored |
| `on: { E: target \| {target, guard, actions, reenter} \| [...] }` | one transition per alternative, in order, trigger `E` |
| `"sibling.child"`, `".child"`, `"#id.path"`, `"#machineId.path"` | full state paths (a plain `"child"` is also accepted at the root) |
| targetless transition | self-transition |
| root-level `on` | one transition whose `from` lists every top-level normal state |
| transition to the machine itself | its initial state (warning) |
| `guard`/`cond`: `"name"`, `{ type, params }`, `and`/`or`/`not` | a `?choice` guard: `?name`, `?type` (parameters dropped, so the label reads like the guard's name), combinators `?(a) && (!(b))`; text that is already a Cascade expression reading `event.x`/`self.x` (or `else`, `?label`) is kept as such |
| `after: { 1000: … }` | a Cascade timer `after: 1s` (no source fires it) |
| `after: { NAMED: … }`, `after: { 0: … }` | trigger `after_NAMED` / `after_0ms`, fired by `Clock` (the delay is only known at run time) |
| `assign` in transition `actions` with `assignment: { f: "event.x" }` (or `"({ event }) => event.x"`, also under `params`) | `sets: { f: event.x }` |
| any other `assign` (computed values, no static assignment, `assign` in `entry`/`exit`) | ignored with a warning |
| `invoke: { id \| src, onDone, onError }` | triggers `<actor>_done` / `<actor>_error`, fired by a source named after the actor |
| `onDone` on a compound state | trigger `<state>_done`, fired by `Environment` (warning: approximation) |
| emitting actions (see the routing table) in `actions`, `entry`, `exit` | `emits` on the transitions that run them |
| entry actions of the initial configuration | dropped (warning: nothing to attach them to) |
| `always`, `type: "parallel"`, multiple targets, initial history state, machine without states | `InteropError::Unsupported` |
| `*` wildcard transitions, `in` guards, `forwardTo`, `onSnapshot`, unknown keys | ignored with a warning |
| `context`, `meta`, `tags`, `description`, `output`, `types`, … | ignored silently |

Emitting actions (the `xstate.` prefix is optional; `event` may be a string
or `{ type }`, directly or under `params`): `{ type: "xstate.raise", event }`,
`{ type: "xstate.sendTo", to, event }`, `{ type: "xstate.sendParent", event }`,
`{ type: "xstate.emit", event }`, and the Cascade convention
`"emit:Name"` / `{ type: "emit:Name" }` (prefix: `ImportOptions::emit_prefix`).
Any other action emits nothing.

### SCXML → Cascade

| SCXML | Cascade |
| --- | --- |
| `<scxml name initial>` | one machine named `name` (default `Machine`) |
| a sole top-level `<parallel>` | a system: each region `<state id>` is a machine; `<scxml name>` is the system name |
| `<state id initial>`, `<initial><transition target/></initial>` | a state and its initial child; a deeper initial is lifted to the child containing it (warning); the machine's may be any descendant |
| `<final>`, `<history type="shallow\|deep">` | `kind: final` / `history` / `deep-history`; a history's default transition is ignored |
| ids | local names with the parent's id prefix removed (`Order.draft` under `Order` → `draft`); missing ids are generated (warning) |
| `<transition event="a b" target cond type>` | one transition per event descriptor; `.*`/`*` suffixes and a `Machine.` prefix removed; targetless → self-transition; `cond` → guard (Cascade syntax kept, anything else a `?choice`); `type="internal"` → no re-entry |
| `<send event delay="2.5s" id/>` in a state's `<onentry>` (no `target`), with a `<transition>` of the same state on that event | a timer `after: 2500ms` on that transition (the matching `<cancel>` in `<onexit>` is the pattern's other half and is not needed); delays are `s`/`ms` CSS2 times, whole milliseconds; a delayed send nothing in its state takes stays an entry emit |
| `<assign location expr>` in a transition | `sets`, when `expr` is a Cascade operand (in a foreign document only `event.x` / `self.x`); else ignored with a warning |
| `<param name expr>` in a `<send>` | the emit's `with:`, read like `<assign>` |
| `<raise>`, `<send event target>` in transitions, `<onentry>`, `<onexit>`, also inside `<if>`/`<elseif>`/`<else>`/`<foreach>` | `emits`, routed as in the routing table |
| `<parallel>` anywhere else, eventless transitions, multiple targets or initial states, transitions into another region | `InteropError::Unsupported` |
| `<invoke>` | ignored with a warning (its done/error events are ordinary events) |
| `<datamodel>`, `<script>`, `<log>`, `<assign>` outside transitions, … | ignored |

A document that declares the `urn:x-cascade:scxml` namespace is Cascade's
own export: `cascade:controller` regions, `<cascade:event>`,
`<cascade:source>`, `<cascade:field name type/>` (typed fields of a machine
or of a `<cascade:event>` payload), and the
`cascade:color|domain|fields|system|bounded|target|where|guard` attributes
are read back and no wiring is synthesized. In such a document a
transition whose event lacks the `Machine.` prefix is a subscription; its
filter is `cascade:where`, its guard `cascade:guard` when there is a
filter and `cond` otherwise.

### Cascade → SCXML

```xml
<scxml xmlns="http://www.w3.org/2005/07/scxml" xmlns:cascade="urn:x-cascade:scxml" version="1.0" name="System">
  <cascade:event name="OrderPaid" payload="orderId amount"/>   <!-- only in strict-events mode -->
  <cascade:source name="Customer" fires="Order.submit"/>
  <parallel id="system">
    <state id="Order" initial="Order.draft" cascade:color="blue" cascade:fields="orderId">
      <state id="Order.pending" initial="Order.pending.waiting">
        <transition event="Order.capture_ok" target="Order.paid" cond="guard text" cascade:bounded="true">
          <send event="OrderPaid"/>
        </transition>
        …
      </state>
      <final id="Order.paid"/>
      <history id="Order.pending.h" type="deep"/>
    </state>
    <state id="Fulfillment" cascade:controller="Fulfillment">
      <transition event="OrderPaid">
        <send event="Shipment.start" cascade:target="Shipment where orderId == event.orderId"/>
        <if cond="when text"><send event="Shipment.start"/></if>
      </transition>
    </state>
  </parallel>
</scxml>
```

- Machines are regions of one top-level `<parallel>`; controllers are
  regions marked `cascade:controller` (one transition per subscribed event,
  one `<send>` per rule, inside `<if cond>` when the rule has a condition).
  Executed by an SCXML engine, the controller regions route events the way
  Cascade's controllers do.
- State ids are `Machine.path`; triggers are events named
  `Machine.trigger` (SCXML events are global to the document); emits are
  untargeted `<send>`s.
- A model with one machine and no controllers is written flat (states
  directly under `<scxml name="Machine">`, `cascade:system` for the system
  name), which single-machine SCXML tools handle best.
- Text is escaped for XML attributes (newlines as `&#10;`); characters XML
  cannot carry become U+FFFD.

Semantics-phase features:

```xml
<state id="Checkout.waiting">
  <onentry><send event="Checkout.after-30m" delay="1800s" id="Checkout.waiting.after-30m"/></onentry>
  <onexit><cancel sendid="Checkout.waiting.after-30m"/></onexit>
  <transition event="OrderPaid" target="Checkout.paid"
              cond="self.orderId == event.orderId and event.amount &gt;= 10"
              cascade:where="orderId == event.orderId" cascade:guard="event.amount &gt;= 10"/>
  <transition event="Checkout.after-30m" target="Checkout.expired"/>
</state>
<!-- ?card declined: a choice the data does not decide (a scenario picks it); … -->
<transition event="Order.pay" target="Order.done" cond="?card declined"/>
<transition event="Order.pay" target="Order.paid" cond="event.amount &gt; 0">
  <assign location="amount" expr="event.amount"/>
  <send event="Receipt"><param name="total" expr="self.amount"/></send>
</transition>
<cascade:field name="tier" type="enum [basic, gold]"/>   <!-- typed fields, inside the machine element -->
```

| Cascade | SCXML |
| --- | --- |
| guard (expression, `else`) | `cond` with the canonical text |
| `?choice` guard | `cond="?label"` preceded by an XML comment saying it is a choice: readable, and opaque to engines either way |
| timer `after: 30m` on state S | in S: `<onentry><send event="M.after-30m" delay="1800s" id="<S id>.after-30m"/>`, `<onexit><cancel sendid=…/>`, and a transition on `M.after-30m` (delays in `s`, or `ms` when not whole seconds; one send per timer trigger and state) |
| subscription `on: { event: E, where: f }` | a transition on `E` itself (not `M.E`); `cond` is the filter in plain context (`self.x`) joined with an expression guard by `and`, for engines; `cascade:where`/`cascade:guard` keep them apart |
| `sets: { f: v }` | `<assign location="f" expr="v"/>` before the sends |
| emit `with: { k: v }` | `<send event="E"><param name="k" expr="v"/></send>` |
| typed fields / payload | `<cascade:field name type/>` children of the machine element / `<cascade:event>`; untyped ones stay the `cascade:fields`/`payload` attribute |

### Cascade → Mermaid

- `--to mermaid` (`stateDiagram-v2`): each machine is a composite
  `state "Order" as Order {`; compound states nest; history states are
  labelled `H` / `H*`; `[*] -->` marks each composite's initial child and
  `--> [*]` each final state; transitions are labelled `trigger [guard]`.
  Mermaid cannot draw transitions between internal states of different
  composites, so each transition is drawn in the innermost composite holding
  both endpoints, between their ancestors there, with the real endpoints
  appended to the label (`finish (running.computing → done)`). Labels
  name the trigger by kind: a command by name, a timer `after 30s`, a
  subscription `on OrderPaid` (plus `where <filter>`); guards are their
  canonical text (`capture [event.amount #62; 0]` once escaped). Ids are
  `[A-Za-z0-9_]`, globally unique, never a keyword. A system name becomes
  front-matter `title`.
- `--to mermaid-causal` (`flowchart LR`): sources `s0[/"Customer"/]`,
  transition pills `t0(["Order: draft → pending"])` filled with the machine
  hue, event tags `e0>"OrderPaid"]`, controller hexagons `c0{{"Fulfillment"}}`,
  and `u0["M.t (no transition)"]` for fired triggers nothing accepts. Edges:
  source → transition (solid, trigger label), emit (dashed, gray),
  subscribe (solid), fire (dashed, `trigger [when]`, target machine hue via
  `linkStyle`), notify (event → subscribing transition, thick `==>`,
  labelled `where <filter>`, machine hue) and delay (transition → timer
  transition it arms, dotted, labelled `after 30s`, machine hue with
  `stroke-dasharray:2 4`); notify and delay edges come from
  `CausalGraph::build`. Hues follow the app's rule (declared color, else
  the first unused Okabe-Ito hue).
- Labels are escaped with Mermaid entity codes (`#35;` for `#`, `#59;`,
  `#quot;`, `#60;`/`#62;`, `#123;`/`#125;`, `#37;`, `#124;`, `#96;`, `#38;`,
  `#58;` in transition labels). Output was checked against the Mermaid 11
  parser during development.

### Cascade → P

- `type tRegistry = map[string, machine]; event eWire: tRegistry;` wire all
  instances together; the `TestDriver` creates one instance of every machine
  and controller, sends each the registry, then for 10 steps lets every
  external source fire each of its triggers nondeterministically (`if ($)`).
- Events: each Cascade event keeps its name (`type t<Event> = (f: T, …)`
  when a payload is declared); each trigger is `e<Machine>_<trigger>`.
- Field types (`p_lang/fields.rs`): `any` → `any`, `string` → `string`,
  `number` → `float` (Cascade numbers are decimals, so `float` keeps
  `1.5`; numeric literals are written `42.0`), `bool` → `bool`, `enum [a,
  b]` → `enum t<Owner>_<field> { <Owner>_<field>_a, … }`. Declared
  instance fields are machine variables (`var amount: float;`).
- Guards that are expressions over typed instance fields become P
  conditions (`if ((amount > 0.0) && (tier == Order_tier_gold))`); rule
  conditions over the handled event's typed payload read `payload.f`.
  Anything else (a `?choice`, `event.x` in a guard — trigger events carry
  no payload — an `any` field, a subscription filter) is `$` with the text
  as a comment. `else` is the `else` branch. `sets` assign the variables
  (an `event.x` source is a TODO comment).
- Subscriptions: an emitted event is also sent, as the subscription
  trigger event, to each subscribing machine.
- Timers: a state whose own or ancestors' transitions have a timer gets
  `entry { if ($) { send this, e<M>_after_30s; } }`: the timer may fire
  while the state is current, or never. A stale timer is ignored unless
  the new state takes the same trigger, which is Cascade's rule. Real
  cancellation and deadline order need a timer machine (TODO comment);
  moving between children of a compound state re-arms its timers.
- Each machine: `start state Wiring` defers its triggers until `eWire`, then
  enters the default entry of the initial state. Only atomic and final
  states become P states (paths with `.` → `_`); handlers follow
  `Model::enabled_transitions`, so compound-state transitions are inherited
  and the innermost wins. One unguarded transition → `on e goto S [with { sends }]`;
  otherwise `on e do { if (<cond> | $) { … goto A; } else … }` with guards as comments.
  Emits are `send registry["Controller"], Event[, default(tEvent)];` to each
  subscribed controller. Unaccepted triggers are `ignore`d (Cascade drops
  them), with a comment to remove the line to have P report them.
- Each controller: `state Running` with `on Event do { … }`; per rule a
  comment (fire, selector, when), `if ($)` for a `when`, and
  `send registry["Machine"], eMachine_trigger;`; fan-out and spawn selectors
  get TODO comments.
- `test tcSystem [main = TestDriver]: { TestDriver, … };`
- Identifiers are `[A-Za-z_][A-Za-z0-9_]*`, never P keywords, unique per
  namespace.

### Diff attributes

| Element | Compared |
| --- | --- |
| Machine | color, domain, initial state path, fields |
| State | kind (atomic, compound with initial child path, final, history deep/shallow) |
| Transition | guard, emits (ordered), bounded |
| Event | payload (ordered), declared |
| Rule | fired trigger, target (mode and clauses), condition, bounded |
| External source | fired triggers (sorted) |
| Trigger, controller, handler | nothing: added or removed only (trigger acceptance is derived) |

A rename is a removal plus an addition, since elements match by key.

## Files

| File | Role | Key exports |
| --- | --- | --- |
| `crates/cascade-interop/src/lib.rs` | Facade: dispatch by format | `import`, `import_with`, `export`, re-exports |
| `crates/cascade-interop/src/format.rs` | Format names for the CLI | `ImportFormat`, `ExportFormat`, `UnknownFormat` |
| `crates/cascade-interop/src/error.rs` | Import errors | `InteropError` |
| `crates/cascade-interop/src/git.rs` | File contents at a revision | `read_at_rev`, `GitError` |
| `crates/cascade-interop/src/yaml.rs` | Definition → compact YAML | `to_yaml`, `selector` (crate) |
| `crates/cascade-interop/src/yaml/quote.rs` | Plain vs quoted scalars | `scalar`, `Ctx` (crate) |
| `crates/cascade-interop/src/import/mod.rs` | Import report types | `ImportOptions`, `Imported`, `ImportWarning`, `WarningKind`, `NameKind` |
| `crates/cascade-interop/src/import/chart.rs` | Statechart IR | `Chart`, `Node`, `Edge`, `TriggerSpec`, `Emit`, `Route` (crate) |
| `crates/cascade-interop/src/import/names.rs` | Name sanitizing | `sanitize`, `NameTable`, `KeyedNames` (crate) |
| `crates/cascade-interop/src/import/lower.rs` | IR → Definition, emit attribution | `lower`, `attributed_emits` (crate) |
| `crates/cascade-interop/src/import/wiring.rs` | Router, sources, annotations | `Wiring`, `Annotations`, `wire` (crate) |
| `crates/cascade-interop/src/xstate/mod.rs` | XState entry and input shapes | `import` (crate) |
| `crates/cascade-interop/src/xstate/json.rs` | Order-preserving JSON value | `Json` (crate) |
| `crates/cascade-interop/src/xstate/parse.rs` | Config → chart, target resolution | `machine` (crate) |
| `crates/cascade-interop/src/xstate/actions.rs` | Emitting actions, guard text | `emits`, `guard_text` (crate) |
| `crates/cascade-interop/src/scxml/mod.rs` | SCXML entry, namespaces | `import`, `export`, `SCXML_NS`, `CASCADE_NS` (crate) |
| `crates/cascade-interop/src/scxml/read.rs` | Document → charts and annotations | `read` (crate) |
| `crates/cascade-interop/src/scxml/write.rs` | Model → SCXML | `export` (crate) |
| `crates/cascade-interop/src/scxml/xml.rs` | XML escaping and writer | `attr`, `comment`, `XmlWriter` (crate) |
| `crates/cascade-interop/src/mermaid/` | `structure.rs`, `causal.rs`, `escape.rs`, `ids.rs`, `palette.rs` | `structure`, `causal` (crate) |
| `crates/cascade-interop/src/p_lang.rs`, `p_lang/` | `names.rs`, `writer.rs`, `fields.rs` (P types, guard/condition translation), `machines.rs`, `controllers.rs`, `driver.rs` | `export` (crate) |
| `crates/cascade-interop/src/patch/ops/keyed.rs` | Entry-by-entry edits of `sets:`, `with:`, typed `fields:`/`payload:`, list ↔ mapping switch | `reconcile_map`, `set_field_decls` (crate) |
| `crates/cascade-interop/src/patch/ops/emits.rs` | Item-by-item edits of `emits:` with `with:` | `reconcile` (crate) |
| `crates/cascade-core/src/diff.rs` | Diff types and entry points | `DiffStatus`, `ModelDiff`, `diff_models`, `merge_for_display` |
| `crates/cascade-core/src/diff/compare.rs` | Attributes per element | `attributes_by_key` (super) |
| `crates/cascade-core/src/diff/merge.rs` | Union definition with ghosts | (super) |
| `crates/cascade-core/src/diff/rebuild.rs` | Removed elements rebuilt from a model | (super) |
| `crates/cascade-cli/src/commands/{export,import,diff}.rs` | CLI commands | `run` |
| `examples/xstate/*.json` | XState fixtures: traffic light (after, onDone, root `on`, emits), fetch (invoke, retry, deep history, raise, `#id`), checkout (three machines, `sendTo`/`sendParent`) | — |
| `examples/scxml/*.scxml` | SCXML fixtures: microwave (generic, onentry/onexit, history, internal), vending (parallel regions, `#_Id`), order-fulfillment (the golden export) | — |

Tests: `crates/cascade-interop/tests/{yaml,xstate,scxml,mermaid,p_lang,git}.rs`,
the semantics-phase suites `{semantics_yaml,patch_semantics,scxml_semantics,xstate_semantics,mermaid_semantics,p_semantics}.rs`
(the patch suite uses `tests/fixtures/patch/semantics.yaml`, block style with comments everywhere)
(with `tests/common/mod.rs`, a full element/attribute signature of a model),
`crates/cascade-core/tests/diff.rs`, `crates/cascade-cli/tests/interop.rs`,
plus unit tests in the modules.

## Invariants and constraints

- **YAML round trip:** for every example, every import fixture and a
  kitchen-sink definition, `load_str(to_yaml(def))` resolves to the same
  design (every element key and attribute, in order), and emitting again is
  a fixed point. Choice labels and quoted literals with any characters
  survive (tested over YAML 1.1/1.2 lookalikes, indicators, quotes, control
  characters); typed fields, subscriptions, timers, `sets` and emit `with`
  round-trip too (`tests/semantics_yaml.rs`).
- **SCXML round trip:** export then import preserves every element key and
  attribute (machines, states and kinds, initials, transitions with guards,
  emits and `bounded`, declared events, controllers, handlers, rules with
  selectors and conditions, sources, colors, domains, fields, system name),
  and the semantics-phase ones (trigger kinds, timer delays, subscription
  filters, `sets`, emit `with`, field and payload types), with no
  warnings, for every example, fixture, the kitchen sink and a saga using
  every new feature (`tests/scxml_semantics.rs`).
- **Patches stay in place for the new keys:** every edit of a guard,
  trigger (command, timer, subscription and filter), `sets`, emit, `with`,
  field or payload type is patched without the rewrite fallback and parses
  to `edit::apply`'s definition; untouched comments survive; a chain of
  edits that returns to the original definition returns the original text
  byte for byte (`tests/patch_semantics.rs`).
- **Imports resolve:** every successful import resolves; fixtures also run
  through the causal graph and the checks without panicking.
- **Exports are total:** every resolved model exports in every format.
- **Deterministic output:** exports and imports depend only on input order.
- **Diff:** `diff_models(m, m)` is empty; statuses never include
  `Unchanged`; elements match by `ElementKey` only.
- **Merge:** new elements keep their spans; ghosts have unknown spans;
  merging a model with itself yields the same keys.
- **Git:** revisions starting with `-` are rejected before git runs; the
  file's directory must exist; output must be UTF-8.

Documented losses:

- XState/SCXML import: guards and conditions become `?choices` (unless
  already Cascade expressions; XState guard parameters are dropped from the
  label); XState assignments other than copies of an event field, and
  SCXML `<assign>`/`<param>` that are not Cascade operands, are dropped
  with a warning; named XState delays stay clock-fired commands;
  `context`/datamodel, non-emitting actions, invoked actors' behavior,
  history default targets and wildcard transitions are dropped; completion
  (`onDone`) is approximated by an environment trigger; exit actions of
  active descendants are not attributed (only the source's chain); root-level
  transitions attribute no exit actions; routing is by event name, so one
  event emitted by several actions routes to the union of their receivers;
  a deeper SCXML initial of a compound state is lifted to its child.
- SCXML export: other tools see guards and conditions as `cond` text they
  cannot evaluate (subscription `cond`s and `<assign>`/`<param>` `expr`s
  use Cascade operands, `event.x`/`self.x`, not a datamodel), and ignore the `cascade:` namespace (sources, declared
  events, selectors, colors, fields, bounded); an event named like a machine
  would prefix-match that machine's trigger events in an SCXML engine.
  Multi-source `from` lists become one transition per source and implicit
  initial states become explicit (same model).
- Mermaid: structure shows no emits, controllers or sources; lifted
  transitions are drawn between ancestors; the causal flowchart merges a
  controller's handlers into one node and omits selectors; layout is
  Mermaid's.
- P: one instance per machine (selectors as comments, fan-out/spawn TODO);
  guards other than expressions over typed fields are nondeterministic
  choices; subscription filters are not checked; timers are
  nondeterministic self-sends without cancellation; history enters the
  parent's default; payload values are `default(...)`; dropped triggers
  are `ignore`d; no specification monitors; never compiled here.
