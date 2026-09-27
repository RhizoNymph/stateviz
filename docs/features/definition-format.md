# Definition format and model

## Scope

- The YAML schema of a Cascade definition file.
- Parsing YAML into a spanned `Definition` with every shape error reported.
- Resolving a `Definition` into a `Model`: typed ids, statechart structure,
  reverse indexes, and every reference error reported.
- Stable element identity (`ElementKey`) and id-based references
  (`ElementRef`).
- Statechart helpers: ancestors, default entry, enabled transitions.

## Non-scope

- Evaluating guards or conditions (free text, displayed only).
- Static analysis of well-formed models (see static-analysis.md).
- Writing YAML back out (see interop-and-diff.md).
- Scenario files (see simulator.md).

## Schema

```yaml
system: Shop                      # optional display name
machines:                         # required, mapping name → machine
  Order:
    color: blue                   # optional; orange, sky-blue, green, yellow, blue,
                                  #   vermillion, purple, black (+ aliases red, grey, …)
    domain: sales                 # optional; groups hues past 8 machines
    initial: draft                # optional; defaults to the first state
    fields: [orderId]             # optional; selectors are checked against it
    states:                       # required; list or mapping
      - draft                     #   plain name
      - pending:                  #   single-key mapping for a body
          initial: waiting        #   compound: initial child (default: first)
          states: [waiting, authorizing]
      - paid: { kind: final }     #   kind: normal | final | history | deep-history
    transitions:
      - from: draft               # name, dotted path, or a list (one transition each)
        to: pending
        on: submit                # trigger name
        guard: "amount > 0"       # optional free text
        emits: [OrderSubmitted]   # optional string or list
        bounded: true             # optional; silences cascade cycles through it
events:                           # optional; when present, every event must be declared
  OrderPaid: { payload: [orderId, amount] }
controllers:
  Fulfillment:
    on:
      OrderPaid:                  # a list of rules, or a single rule mapping
        - fire: Shipment.start    # Machine.trigger
          target: Shipment where orderId == event.orderId
          when: "not a gift card" # optional free text
          bounded: false
external:
  Customer: [Order.submit]        # a list or a single trigger ref
```

Target selectors:

```text
Machine                                   the one instance (singleton)
Machine where f == event.g and h == lit   exactly one matching instance
all Machine where f == event.g            every matching instance (fan-out)
new Machine with f = event.g, h = lit     spawn an instance, then fire on it
```

Names (machines, states, triggers, events, controllers, sources, fields)
match `[A-Za-z_][A-Za-z0-9_-]*`. State references may be dotted paths
(`running.fetching`) or a unique local name.

## Data and control flow

```text
text ──saphyr MarkedYamlOwned──▶ parse::yaml (Diags) ──▶ Definition
Definition ──resolve::Resolver──▶ Model | LoadError{diagnostics}
```

1. `parse::yaml::parse` loads marked YAML (1-based line/col spans), rejects
   empty and multi-document files, then walks the schema. `parse::node::Diags`
   wraps every typed access (mapping, string, name, path, bool, lists) and
   records a `Diagnostic` instead of failing. Unknown keys are reported, so
   typos like `colour:` are caught. Embedded grammars (`parse::grammar`) parse
   trigger refs and target selectors in place.
2. If any diagnostic was recorded, parsing returns them all, sorted by span.
3. `resolve::resolve` builds the model in phases: machines and their states
   (pre-order, paths, compound initial children), declared events,
   transitions (creating triggers and implicit events on first mention,
   expanding `from` lists, computing ordinals), controllers (handlers, rules,
   selectors checked against declared fields and payloads), external sources.
   Every problem becomes a diagnostic. A machine that fails to resolve is
   remembered so references to it are not reported twice.
4. `load_str` / `load_file` chain both steps.

## Files

| File | Role | Key exports |
| --- | --- | --- |
| `crates/cascade-core/src/span.rs` | Source positions | `Pos`, `SourceSpan`, `Spanned<T>` |
| `crates/cascade-core/src/ids.rs` | Typed arena indices, constructible only inside core | `MachineId`, `StateId`, `TransitionId`, `TriggerId`, `EventId`, `ControllerId`, `HandlerId`, `RuleId`, `ExternalId` |
| `crates/cascade-core/src/color.rs` | Color names a definition may use | `PaletteColor`, `UnknownColor` |
| `crates/cascade-core/src/definition.rs` | Spanned, unresolved document | `Definition`, `MachineDef`, `StateDef`, `TransitionDef`, `EventDef`, `ControllerDef`, `HandlerDef`, `RuleDef`, `ExternalDef`, `TriggerRef`, `TargetSpec`, `TargetMode`, `FieldClause`, `ValueExpr` |
| `crates/cascade-core/src/error.rs` | Load diagnostics | `LoadError`, `Diagnostic`, `DiagnosticKind`, `Expected` |
| `crates/cascade-core/src/parse/mod.rs` | Parser entry | `parse_definition` |
| `crates/cascade-core/src/parse/node.rs` | Diagnostic-recording YAML accessors | `Diags`, `Fields` (crate-private) |
| `crates/cascade-core/src/parse/yaml.rs` | Schema walk | `parse` (crate-private) |
| `crates/cascade-core/src/parse/grammar.rs` | Names, paths, trigger refs, selectors | `is_valid_name`, `is_valid_path`, `parse_trigger_ref`, `parse_target` |
| `crates/cascade-core/src/resolve.rs` | Definition → Model | `resolve` |
| `crates/cascade-core/src/model/mod.rs` | The model and its accessors | `Model` |
| `crates/cascade-core/src/model/elements.rs` | Element structs | `Machine`, `State`, `StateKind`, `Transition`, `Trigger`, `Event`, `Controller`, `Handler`, `Rule`, `Target`, `ExternalSource` |
| `crates/cascade-core/src/key.rs` | Element identity | `ElementRef`, `ElementKey`, `ElementKind`, `InvalidElementKey` |
| `crates/cascade-core/src/load.rs` | Convenience entry points | `load_str`, `load_file`, `LoadFileError` |
| `crates/cascade-core/src/search.rs` | Fuzzy search over labels | `search`, `SearchHit` |
| `examples/order-fulfillment/cascade.yaml` | The spec's example | — |

## Invariants and constraints

- Every id stored in a `Model` indexes a valid element of that model;
  accessors index directly. Ids from one model must not be used with another.
- Reverse indexes agree with forward references: `Trigger::accepted_by`,
  `fired_by`, `sources`; `Event::emitted_by`, `handlers`;
  `Machine::states` (pre-order), `top_states`, `transitions`, `triggers`;
  `Controller::handlers`; `Handler::rules`.
- A compound state always has an initial child (`StateKind::Compound`).
  Final and history states have no children and no outgoing transitions.
  Neither a machine's nor a compound state's initial state is a history state.
- State paths are unique per machine; sibling names are unique.
- A trigger exists for every `(machine, name)` mentioned by a transition,
  rule or external source, even if nothing accepts it.
- `Transition::ordinal` distinguishes transitions sharing
  `(from, to, trigger)`; `Rule::ordinal` is the rule's position in its
  handler. Both are part of the element key.
- `ElementKey` string forms round-trip through `Display`/`FromStr` and serde,
  and survive unrelated edits (tested).
- Parsing and resolving never stop at the first error; diagnostics come back
  sorted by source position.
- When `events:` is non-empty, every emitted or subscribed event must be
  declared (strict mode).
- A rule's target selector names the same machine as its `fire`.
