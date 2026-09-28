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

## Play session

Owned by `feat/sim-session`; details and the full error list are in
simulator.md ("Play session").

- `PlaySession::apply(model, PlayAction)` performs one action by name:
  `AddInstance`, `RemoveInstance`, `Fire` (an external source's trigger,
  delivered at once), `Step { choice }` (the queue head, or any pending item
  by position, to explore orderings by hand) and `RunUntilQuiet`. A
  rejected action changes nothing and returns `SimError::Action(kind)`.
  `ActionOutcome::steps` is the range of trace steps the action appended,
  which is what the overlay's `active` elements come from.
- Semantics are the batch simulator's, through one engine: the batch
  `simulate` turns a scenario into play actions, and
  `PlaySession::from_scenario` records those same actions, so a session's
  trace equals the batch trace.
- For the host: `trace()` + `payloads()`, `instances()` (live instances in
  lifeline order, with leaf state and fields), `pending()` (queue head
  first, stable `PendingId`s, labels like `Fulfillment → s1: start`),
  `available_fires(model)` (every source × trigger × instance with an
  `accepted` flag for the palette) and `timeline()`.
- Rewind and branch: `seek(model, p)` replays from the start when going
  back; acting before the end saves the whole old line as a `Branch` (the
  same action as the next one just moves forward); `switch_branch` swaps
  the current line with a saved one, so switching twice returns.
- After an edit: `replay(new_model)` re-runs the timeline up to its
  position and stops at the first action that no longer applies, returning
  its index and error; the rest of the line stays as the future.
- Record: `to_scenario(name)` then `scenario_to_yaml` writes a scenario that
  keeps manual choices (`- { step: n }`), runs (`- run`), mid-session
  instance changes (`- { create: … }`, `- { remove: … }`), immediate fires
  and `end: pause` for a session stopped mid-cascade. It parses back and
  replays to the same trace, in a session or in `cascade simulate`.
- Removing an instance discards the fires queued for it; its lifeline and
  last state stay in the trace and its name is never reused.
- `cascade simulate <file> [scenario] --interactive` is the same session at
  a text prompt.

## Build and play drawing

Implemented in `cascade-scene` (`views/structure/wiring.rs`,
`views/structure/edit.rs`, `views/overlays/`); view-scenes.md has the full
encoding. The app paints the scene and dispatches clicks through
`Scene::hit_test` as for every view.

### Edit mode (`SceneMode::Edit`, structure view only)

```text
Drafter (lanes, pills with a second South port) ──▶ wiring band ──▶ realize (LayoutCache)
     ──▶ Decor (emphasis, badges, diff) ──▶ band lanes, empty hints, connect handles ──▶ play overlay
```

- **Lanes** are the view mode's, except that each pill has an extra
  South port (index 4) where emits leave.
- **Wiring band:** two more layout groups below the machine lanes, first
  "External sources" (one box per source), then "Events and controllers"
  (one tag per event, including events nothing emits or handles, and one
  hexagon per controller listing its handlers as "on Event"). The
  events-and-controllers band grows most while building, so it goes last,
  where its growth moves nothing else.
- **Edges** are the causal graph's with handlers folded into their
  controller, aggregated per pair of drawn ends and kind: emit (pill
  South-out → event North, dashed gray, target the transition), subscribe
  (event East → controller West, solid gray, target the handler), fire
  (controller North → pill South-in, dashed in the target hue, labelled
  with the selector without the machine, e.g. `where orderId ==
  event.orderId`, `all`, `new with orderId = event.orderId`, then
  `[when]`; merged rules list each label once; target the first rule),
  trigger (source North → pill South-in, solid external neutral, target
  the trigger). They replace the view mode's pill-to-pill links. Ends on a
  collapsed state or machine are unported; ends on a hidden machine's stub
  become dotted `StubLink`s counted in its label.
- **Connect handles:** a circle (radius 4.5) centred on the east edge of
  every state, pill, controller and source, as an `Overlay::Rect` with
  `HitTarget::ConnectHandle { element }`, background fill and a muted
  outline, following the node's opacity. `Scene::hit_test` checks handles
  before anything else, since they straddle their node's edge. SVG and PNG
  exports leave handles out: they are an editing affordance, not part of
  the picture.
- **Empty affordances:** a machine lane with no transitions gets a muted
  hint after its title ("no transitions yet: drag between state handles");
  a definition without machines gets the note "Empty system: add a machine
  to start building." (in `Scene::notes` and as an overlay text).
- Causal, trace and matrix views ignore the mode.

### Play overlay (`SceneInput::play`, causal and structure views)

Drawn after layout and emphasis, so it never relayouts (the layout cache
key never sees it) and never changes a hue. Keys that name nothing are
ignored; `PlayOverlay::default()` draws nothing.

- **Markers:** a chip per instance (`o1`) in its machine's hue (on-hue
  text, bold), sitting on the node's top edge, left to right in marker
  order.
  - Structure view: on the instance's current state; if that is not drawn,
    on its nearest drawn ancestor (a collapsed compound state), else on the
    collapsed machine's node or the hidden machine's stub.
  - Causal view (no states): on every pill leaving the current state or one
    of its ancestors, i.e. what the instance can do next, which reads better
    than marking every pill of the machine. In a dead end (nothing leaves),
    a hollow chip sits on the pills entering the state instead. A hidden
    machine's instances sit on its stub.
- **Active:** the theme's selected outline width plus a glow ring (a
  6-unit stroke in the item's own outline color at 35% alpha, 4 units out,
  `Layer::Under`) on nodes; the width alone on edges. An edge is active
  when it stands for an active element or joins two active nodes (the emit
  from a taken transition to the event it emitted).
- **Pending:** a dotted outline, and a chip with the entry's queue
  position: `1` is the head (filled with the text color), later positions
  hollow; an item queued more than once lists every position ("1, 3").
  Node chips sit on the top-left corner (finding badges use the top
  right), edge chips 18 units before the arrowhead.
- **Matching:** an item stands for an element its meta lists (pills,
  tags, hexagons, fire edges, transition arrows, links). When nothing drawn
  does, items related through a rule stand in, so a pending event in the
  view mode's structure view marks the links whose rules handle it.
- Chips are ordinary overlays, so exports include them.

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
| `crates/cascade-sim/src/session/mod.rs` | Interactive simulator (`view.rs`: host queries; `save.rs`: timeline → scenario) | `PlaySession`, `PlayAction`, `PendingItem`, `InstanceState`, `AvailableFire`, `Timeline`, `Branch`, `scenario_to_yaml` |
| `crates/cascade-sim/src/engine/` | The steppable core shared by play and batch runs (`core.rs`, `exec.rs`, `drive.rs`) | crate-private |
| `crates/cascade-scene/src/play.rs` | Build/play drawing inputs | `SceneMode`, `PlayOverlay`, `PlayMarker` |
| `crates/cascade-scene/src/scene.rs` | New hit target; handles win hit tests | `HitTarget::ConnectHandle`, `Scene::hit_test` |
| `crates/cascade-scene/src/views/structure/wiring.rs` | Edit mode's wiring band: groups, band nodes, aggregated edges, fire labels | crate-private |
| `crates/cascade-scene/src/views/structure/edit.rs` | Band lanes, empty-machine hint, empty-definition note | crate-private |
| `crates/cascade-scene/src/views/overlays/mod.rs` | Play decoration entry point | `PlayDecor`, `Placement`, `add_handles` (crate) |
| `crates/cascade-scene/src/views/overlays/handles.rs` | Connect handles | `add_handles` (crate) |
| `crates/cascade-scene/src/views/overlays/markers.rs` | Instance markers | crate-private |
| `crates/cascade-scene/src/views/overlays/queue.rs` | Active and pending | crate-private |
| `crates/cascade-scene/src/views/overlays/chip.rs` | Chip drawing | crate-private |
| `crates/cascade-scene/tests/{edit_mode,edit_mode_stability,edit_mode_performance,play_overlay,view_mode_golden}.rs` | Build and play drawing tests; view-mode fingerprints in `tests/golden/view_mode.txt` | — |

## Invariants and constraints

- `edit::apply` never returns a definition that fails to resolve; a
  rejected op changes nothing.
- `apply(apply(d, op).definition, inverse) == d` (ignoring spans).
- `parse(patch_text(text, op).text) == apply(parse(text), op).definition`
  (ignoring spans); comments survive unless `rewritten` is set.
- Play actions and edit ops name elements, never typed ids, so both
  survive model reloads.
- Drawing: `SceneMode::View` scenes are unchanged, down to the `Debug` form
  and the SVG (fingerprinted in `tests/view_mode_golden.rs`). A play
  overlay never relayouts, never moves or recolors a node, and trace and
  matrix views ignore it. In edit mode, adding a transition moves nothing
  outside its machine; appending an event or a controller moves no
  existing node; wiring a new handler moves at most the event and
  controller it joins; appending a source can shift the events-and-controllers
  band down, but only as one piece.
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
