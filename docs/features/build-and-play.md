# Build and play

Turns the app from a viewer into a workbench: build a system of machines,
controllers and sources on the canvas, then play it: create instances,
fire triggers, step the queue, rewind and branch, and save what happened as
a scenario.

## Scope

- Structural edits with undo/redo, addressed by names so they survive
  reloads (`cascade_core::edit`).
- Saving edits to the YAML file in place, keeping comments and formatting
  (`cascade_interop::patch`). The file stays the source of truth: hand
  edits reload into the app as before.
- An interactive simulator session with a recorded, branchable timeline
  (`cascade_sim::session`), sharing the batch simulator's semantics.
- Build-mode and play-mode drawing (`cascade_scene::play`): wiring and
  connect handles in the structure view, instance markers and
  active/pending highlights in the causal and structure views.
- The app's build and play modes (`cascade-app`).

## Non-scope

- Free-form manual layout: layout stays automatic, dragging still pins.
- Evaluating guards or conditions during play (they remain free text).
- Collaborative or multi-file editing.

## Data and control flow

```text
Build mode
  gesture / inspector ──▶ EditOp ──▶ edit::apply(definition) ──▶ Applied { definition, inverse, touched }
                                 └──▶ patch::patch_text(file text) ──▶ write file ──▶ reload (own write recognised)
  undo = apply(inverse) the same way; redo = re-apply

Play mode
  palette / queue panel / timeline ──▶ PlayAction ──▶ PlaySession::apply ──▶ trace, instances, pending
  PlaySession ──▶ PlayOverlay { markers, active, pending } ──▶ SceneInput::play ──▶ scene
  after an edit: PlaySession::replay(new model) (stops at the first action that no longer applies)
  save: PlaySession::to_scenario ──▶ scenario_to_yaml ──▶ scenarios/<name>.yaml
```

## Contracts and ownership

| Workstream (branch) | Owns | Implements |
| --- | --- | --- |
| `feat/edit-ops` | `cascade-core/src/edit/` | `apply`, `locate_transition` (inverse ops, rename propagation, cascading removal, validity) |
| `feat/yaml-patch` | `cascade-interop/src/patch/` | `patch_text` (span-based surgical edits; full rewrite fallback flagged) |
| `feat/sim-session` | `cascade-sim/src/session/`, engine refactor | `PlaySession`, `scenario_to_yaml`; batch `simulate` built on the session; scenario format extended for manual queue choices |
| `feat/scene-edit-play` | `cascade-scene` (`play.rs` consumers, structure view wiring band, handles) | `SceneMode::Edit`, `PlayOverlay` drawing, `HitTarget::ConnectHandle` |
| `feat/app-build-play` | `cascade-app` | Build mode (toolbar, drag-to-connect, inspector, delete, undo/redo, save) and play mode (instances, trigger palette, queue, timeline, record) |

Every stub returns a typed `NotImplemented` error or an empty result
until its workstream lands, so the app can be built against the contracts
in parallel.

## Files

| File | Role | Key exports |
| --- | --- | --- |
| `crates/cascade-core/src/edit/mod.rs` | Edit ops on definitions | `EditOp`, `Applied`, `EditError`, `apply`, `locate_transition`, `fresh_name` |
| `crates/cascade-interop/src/patch/mod.rs` | Comment-preserving persistence | `patch_text`, `Patched`, `PatchError` |
| `crates/cascade-sim/src/session/mod.rs` | Interactive simulator | `PlaySession`, `PlayAction`, `PendingItem`, `InstanceState`, `AvailableFire`, `Timeline`, `Branch`, `scenario_to_yaml` |
| `crates/cascade-scene/src/play.rs` | Build/play drawing inputs | `SceneMode`, `PlayOverlay`, `PlayMarker` |
| `crates/cascade-scene/src/scene.rs` | New hit target | `HitTarget::ConnectHandle` |

## Invariants and constraints

- `edit::apply` never returns a definition that fails to resolve; a
  rejected op changes nothing.
- `apply(apply(d, op).definition, inverse) == d` (ignoring spans).
- `parse(patch_text(text, op).text) == apply(parse(text), op).definition`
  (ignoring spans); comments survive unless `rewritten` is set.
- Play actions and edit ops name elements, never typed ids, so both
  survive model reloads.
- A session's trace equals the batch simulator's for the same actions;
  `to_scenario` then `from_scenario` reproduces the trace exactly.

## Edit operations

`cascade_core::edit::apply(definition, op)` clones the definition, applies
the op to the clone, then resolves the result once. Any error leaves the
caller's definition as it was. A `Batch` applies its ops in order and is
validated only at the end, so intermediate steps may pass through
definitions that do not resolve, such as swapping two names through a
temporary or removing every declaration of a strict `events:` list.

### Checks, in order

1. Names the op introduces must match the name grammar
   (`parse::grammar::is_valid_name`; state references use
   `is_valid_path`). This covers nested content too: child states,
   transitions, rules, target selectors and trigger refs. Otherwise the op
   fails with `InvalidName`. A transition with an empty `from` is
   `Invalid(MissingKey from)`, because the file format cannot express it.
2. Every element the op addresses must exist, or it fails with `NotFound`.
   The name has the form `Machine`, `Machine.state.path`,
   `Controller/Event` (for handlers) or the event name.
3. New names must not collide, or it fails with `NameTaken`. Machines,
   controllers, external sources and events (declared or used) are
   checked globally. States are checked among their siblings. Handlers are
   checked by event within their controller. Renaming something to its own
   name is a no-op that succeeds.
4. Insert positions must be `<= len` and existing positions `< len`, or the
   op fails with `IndexOutOfRange`.
5. The result must resolve, or it fails with `Invalid(LoadError)`. Every
   semantic rule is left to the resolver: unknown or ambiguous states,
   final or history states with children or outgoing transitions, history
   initials, machines without states, selector fields, target and fire
   mismatches, and undeclared events in strict mode.

### State references

Transition `from`/`to` and a machine's `initial` refer to states by full
path or by bare local name. The resolver tries a full path first, then a
local name that is unique in the machine. After an edit changes a
machine's state tree (add, rename), every reference is retargeted so it
still points at the same state, and keeps how it was written:

- A reference written as the full path becomes the state's new full path.
  A top-level state's name is its full path.
- A bare local name stays bare, with the state's new local name, if it
  still resolves to the same state. Otherwise it is written as the full
  path. That happens when another state now has the same local name, or
  when a top-level state of that name would capture it.

Compound `initial:` keys name a direct child, never a path, so they only
change when that child is renamed or removed. References that did not
resolve before the edit (possible only inside a batch) are left alone.

### Rules

| Op | Cascade and propagation | Inverse | `touched` |
| --- | --- | --- | --- |
| `AddMachine` | none | `RemoveMachine` | machine, its states and transitions |
| `RemoveMachine` | removes every rule whose `fire:` names the machine (handlers stay, possibly empty) and every external trigger on it (sources stay) | `Batch[AddMachine at index, AddRule at original index (ascending, per handler), SetExternalTriggers(original)]` | machine, states, transitions, removed rules, changed sources (keys as before) |
| `RenameMachine` | `fire:` refs, target selector machine names, external triggers | `RenameMachine` back | machine, states and transitions (new keys), changed rules and sources |
| `SetMachineColor` / `Domain` / `Initial` / `Fields` | none (the resolver checks initials and selector fields) | the same setter with the old value | machine |
| `AddState` | retargets references it would make ambiguous or capture | `RemoveState`, plus fix-ups restoring qualified references | new state and descendants, retargeted transitions, machine if its initial was retargeted |
| `RemoveState` | removes entries whose `to` is in the subtree and entries whose every `from` is. Drops subtree sources from multi-source `from` lists. Resets the machine initial if it pointed into the subtree, and the parent's initial if it named the state. A machine left without states is rejected. | `Batch[AddState at index, AddTransition at original index (ascending), fix-ups: UpdateTransition for trimmed lists, SetMachineInitial, SetStateInitial]` | state and descendants, every removed expansion (keys as before), machine and parent if their initials were reset |
| `RenameState` | paths of the state and its descendants, every reference (retargeted, see above), the parent's initial if it named the state | `RenameState` back, plus fix-ups restoring references that had to be written in full | state and descendants (new keys), retargeted transitions, parent and machine if their initials changed |
| `SetStateKind` / `SetStateInitial` | none | the same setter with the old value | state |
| `AddTransition` | none | `RemoveTransition` | one key per `from` source |
| `UpdateTransition` | none | `UpdateTransition` with the old entry | the entry's keys after the update |
| `RemoveTransition` | none | `AddTransition` at index | the entry's keys before removal |
| `DeclareEvent` | in a file without `events:` (lenient), also declares every event in use, in order of first mention (emits by machine, then subscriptions by controller), and places the new one at `index` among them | `RemoveEventDeclaration` (strict), or a batch removing everything it declared (lenient) | every event it declared |
| `RemoveEventDeclaration` | none. The resolver rejects it while a strict file still uses the event, unless it is the last declaration, which makes the file lenient. | `DeclareEvent` at index, plus removals of anything that re-declaration would add beyond the original list (only inside batches) | event |
| `RenameEvent` | the declaration, every `emits:`, every subscription | `RenameEvent` back | event, handlers (new keys), emitting transitions |
| `AddController` / `RemoveController` | handlers and rules travel with it | `RemoveController` / `AddController` at index | controller, handlers, rules |
| `RenameController` | none (nothing refers to controllers) | `RenameController` back | controller, handlers, rules (new keys) |
| `AddHandler` / `RemoveHandler` | rules travel with it | `RemoveHandler` / `AddHandler` at index | handler and its rules |
| `AddRule` / `UpdateRule` / `RemoveRule` | none | `RemoveRule` / `UpdateRule` with the old rule / `AddRule` at index | the rule at its position |
| `AddExternal` / `RemoveExternal` / `RenameExternal` / `SetExternalTriggers` | none | the opposite op, or the setter with the old list | source |
| `SetSystemName` | none | the setter with the old value | nothing |
| `Batch` | each op in order; validated once at the end | `Batch` of the inverses, reversed | union, in first-touched order |

### Exact inverses

State edits and removing the last event declaration build their inverse in
two steps. The first is the primary inverse (rename back, remove the
added state, re-add the removed one). It is simulated on the edited
definition, and fix-up ops are appended for every difference that is left:
transition entries the edit qualified or trimmed, machine and compound
initials it reset, and events a re-declaration would add. Simulation runs
ops in a mode that skips fix-ups, so it never recurses.

### Spans

Values edited in place, such as a renamed name or a rewritten reference,
keep their span, because they sit at the same place in the file. Elements
carried by an op are inserted as given, so an undo restores the original
spans. Values the edit synthesizes get `SourceSpan::unknown()`: events
declared by a lenient-to-strict switch, lists written by
`SetMachineFields` and `SetExternalTriggers`, and optional values that did
not exist before. The laws hold ignoring spans; compare with
`edit::without_spans`.

### Files

| File | Role | Key exports |
| --- | --- | --- |
| `crates/cascade-core/src/edit/mod.rs` | Contract types, `apply` (clone, apply, resolve), `locate_transition`, `fresh_name` | `EditOp`, `Applied`, `EditError`, `apply`, `locate_transition`, `fresh_name`, `without_spans` |
| `crates/cascade-core/src/edit/engine.rs` | Dispatch per op, batches, simulation for exact inverses | `Effect`, `InverseMode`, `apply_op`, `simulate` (module-private) |
| `crates/cascade-core/src/edit/machines.rs` | Machine add, remove (cascade), rename (propagation), setters | module-private |
| `crates/cascade-core/src/edit/states.rs` | State add, remove (cascade), rename (propagation), kind, initial | module-private |
| `crates/cascade-core/src/edit/transitions.rs` | Transition entries | module-private |
| `crates/cascade-core/src/edit/events.rs` | Declarations (lenient-to-strict switch), renames | module-private |
| `crates/cascade-core/src/edit/controllers.rs` | Controllers, handlers, rules | module-private |
| `crates/cascade-core/src/edit/externals.rs` | External sources | module-private |
| `crates/cascade-core/src/edit/refs.rs` | State trees, reference resolution and retargeting | `StateIndex`, `retarget` (module-private) |
| `crates/cascade-core/src/edit/restore.rs` | Fix-ups that make state-edit inverses exact | module-private |
| `crates/cascade-core/src/edit/keys.rs` | Element keys from the definition alone (expansions, ordinals) | module-private |
| `crates/cascade-core/src/edit/validate.rs` | Name checks for introduced content | module-private |
| `crates/cascade-core/src/edit/lookup.rs` | Lookups, index checks, span-preserving setters | module-private |
| `crates/cascade-core/src/edit/spans.rs` | Span-insensitive comparison | `without_spans` |
| `crates/cascade-core/tests/edit.rs`, `edit_states.rs`, `edit_props.rs` | Ops on the examples, rename propagation, cascades, rejections, batches, `locate_transition`; the inverse law over every candidate op on every fixture and over random op sequences (proptest) | — |
