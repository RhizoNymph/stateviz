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
`views/structure/gutters/`, `views/structure/selector.rs`,
`views/structure/edit.rs`, `views/overlays/`); view-scenes.md has the full
encoding and readability.md the placement's rationale and measurements. The app paints the scene and dispatches clicks through
`Scene::hit_test` as for every view.

### Edit mode (`SceneMode::Edit`, structure view only)

```text
Drafter (a gutter group above each machine, lanes, pills with extra ports) ──▶ last gutter
     ──▶ wiring: nodes, merged edges ──▶ gutters::assign ──▶ gutters::order ──▶ WiringMemo columns
     ──▶ ports facing the gutter, fire labels ──▶ realize (LayoutCache)
     ──▶ Decor (emphasis, badges, diff) ──▶ gutter lanes, empty hints, connect handles ──▶ play overlay
```

- **Lanes** are the view mode's, except that each pill has two extra
  ports where emits leave: South-out (4) toward a gutter below, North-out
  (5) toward a gutter above.
- **Gutters:** thin layout groups without a header, one right above each
  machine's groups (its lane and nested bands, or its stub or collapsed
  lane) and one below the last; only gutters holding nodes are laid out.
  Each is drawn as an untitled neutral lane (pale fill, dashed rule
  outline). They hold one tag per event (including events nothing emits
  or handles), one hexagon per controller listing its handlers as
  "on Event", and one box per external source.
- **Which gutter** (`gutters::assign`): the layout routes a wire straight
  through the gap between two neighbouring groups and sends any other
  wire around through a side corridor. So each node goes where the fewest
  of its wires need a corridor, then where they cross the fewest lane
  groups:
  - an event by its emits plus one stand-in wire per handling controller
    (to the nearest pill that controller's rules for the event fire
    into); ties go nearest its placed controllers, then downward;
  - a controller by its fires plus its subscriptions to the placed
    events; ties go upward (above the lane it fires into);
  - a source by its triggers; ties go upward;
  - a node with no wires at all goes to the last gutter.
  Each rule looks only at the node's own wiring (events also at the rules
  handling them), so an edit only moves the wiring nodes whose wiring
  changed.
- **Row order** (`gutters::order`): by the median estimated column of the
  pills a node wires (a pill's column is estimated from breadth-first
  depth in its band); nodes without pills last; ties sources, events,
  controllers, then definition order. A controller that handles an event
  in the same gutter goes right after its last such event.
- **Columns** (`gutters::memo`): gutter nodes get fixed layout columns
  (`LayerConstraint::Exact`) so the row stays one row in that order. The
  `SceneBuilder` remembers each node's column per gutter: a node keeps
  its column, a node new to a gutter gets the next unused one (the right
  end of the row), and a controller that would sit left of an event it
  handles in the same gutter moves to the end (the engine rejects a fixed
  edge pointing backwards). `SceneBuilder::reset` forgets the columns, so
  a fresh build is the tidy order again. Returning to an earlier input
  (undo) restores its columns, hence its cached layout.
- **Edges** are the causal graph's with handlers folded into their
  controller, aggregated per pair of drawn ends and kind. Pill ends face
  the gutter: emit (pill South-out → event North when the event is below,
  pill North-out → event South when above; dashed gray, target the
  transition), subscribe (event East → controller West, solid gray, target
  the handler), fire (controller → pill, South → North when the pill is
  below, North → South when above; dashed in the target hue, target the
  first rule), trigger (source → pill likewise, solid external neutral,
  target the trigger). They replace the view mode's pill-to-pill links.
  Ends on a collapsed state or machine are unported; ends on a hidden
  machine's stub become dotted `StubLink`s counted in its label.
- **Fire labels** (`selector.rs`): the selector without the machine,
  shortened: `by orderId` for `where orderId == event.orderId` (a clause
  comparing different names or a literal reads `f=event.g`, `f=lit`),
  `all`, `all by f`, `new`, `new with f`; nothing for a singleton; then
  `[when]`. A fire standing for several rules lists each distinct
  selector and condition once. Among one controller's fires into one
  machine, a label identical to one already shown is left off. The full
  selector is in the inspector (the edge's hit target is the rule). The
  layout reserves room for each label.
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
| `crates/cascade-scene/src/views/structure/wiring.rs` | Edit mode's wiring: nodes, aggregated edges, placement glue, ports facing the gutter, fire labels | crate-private |
| `crates/cascade-scene/src/views/structure/gutters/` | Gutter assignment (`mod.rs`), row order (`order.rs`), columns kept across edits (`memo.rs`) | `assign`, `order`, `WiringMemo` (crate) |
| `crates/cascade-scene/src/views/structure/selector.rs` | Fire label policy | crate-private |
| `crates/cascade-scene/src/views/structure/edit.rs` | Gutter lanes, empty-machine hint, empty-definition note | crate-private |
| `crates/cascade-scene/src/views/overlays/mod.rs` | Play decoration entry point | `PlayDecor`, `Placement`, `add_handles` (crate) |
| `crates/cascade-scene/src/views/overlays/handles.rs` | Connect handles | `add_handles` (crate) |
| `crates/cascade-scene/src/views/overlays/markers.rs` | Instance markers | crate-private |
| `crates/cascade-scene/src/views/overlays/queue.rs` | Active and pending | crate-private |
| `crates/cascade-scene/src/views/overlays/chip.rs` | Chip drawing | crate-private |
| `crates/cascade-scene/tests/{edit_mode,edit_mode_stability,edit_mode_performance,gutters,play_overlay,view_mode_golden}.rs` | Build and play drawing tests; view-mode fingerprints in `tests/golden/view_mode.txt` | — |

## App build and play modes

Owned by `feat/app-build-play` (`crates/cascade-app`). The app is a
workbench with three modes, chosen by the segmented control at the left of
the toolbar or by keys:

| Keys | Mode | Canvas |
| --- | --- | --- |
| ctrl/cmd-1 | View | the four views as before (diff mode only here) |
| ctrl/cmd-2 | Build | structure view with `SceneMode::Edit`; inspector on the right |
| ctrl/cmd-3 | Play | structure or causal view with `SceneInput::play`; play panel on the right |

Asking for a view the mode cannot show (e.g. `3` for the trace view in
Build mode) drops back to View mode (`mode::view_for`). `cascade-app --new
<file>` creates `<file>` with a starter definition (one machine `Machine`
with one state `idle`, `build::disk::NEW_FILE_TEXT`) and opens it in Build
mode; it refuses to overwrite an existing file. `--mode view|build|play`
picks the starting mode. "New file…" (ctrl/cmd-N) asks for a path, creates
the starter there and switches the window to it.

### Build mode

The build bar (a second toolbar row) has **+ Machine**, **+ State**,
**+ Controller**, **+ Source**, **Connect selection**, **Delete**, **Undo**,
**Redo** and **New file…**. Default names come from `edit::fresh_name`
(`Machine`, `Machine2`, …; `state`, `state2`, … fresh among every state
name of the machine; `Controller`; `Source`). **+ State** adds into the
selected machine or compound state, next to a selected atomic state, into
the machine of a selected transition or trigger, or into the only machine
when there is one (`build::ops::state_target`); the inspector's
**+ Child state** nests into the selected state. **Delete** (also `Delete`
or `Backspace` on the canvas) maps each element kind to its remove op
(`build::ops::delete`); deleting an event is one `Batch` that drops it from
every `emits:`, removes every handler of it and its declaration; triggers
cannot be deleted directly.

Gestures (`gesture::Gesture::press_handle`, `build::connect`):

| Drag from | Drop on | Edit |
| --- | --- | --- |
| state handle | state of the same machine | `AddTransition`, trigger `go`, `go2`, … (rename it in the inspector) |
| transition handle | controller | `Batch`: declare `<Trigger>Done` (fresh; only when the file declares events), add it to the transition's `emits`, `AddHandler` on the controller |
| transition handle | event | the transition also emits the event |
| event handle | controller | `AddHandler` |
| controller handle | transition or trigger | `AddRule` on the controller's latest handler firing that trigger at the one instance (no `target:`) |
| handler handle | transition or trigger | `AddRule` on that handler |
| source handle | transition or trigger | `SetExternalTriggers` with the trigger added |

While dragging, a dashed rubber band follows the pointer, every valid drop
target gets a dashed outline and the one under the pointer a heavy one
(neutral text color, never hue). Dropping on empty canvas or an invalid
target cancels with a status note. Dragging a node body still pins it.
Without handles (the scene builder draws them), shift-click the source then
the target and press **Connect selection**: the same mapping applies.

The inspector (`build::inspector`) shows the selected element's fields.
Text fields validate live with the core grammar (names, state paths,
`Machine.trigger`, target selectors via `parse_target`); Enter commits one
field as one `EditOp`; invalid input shows the error under the field and
changes nothing. Choices (machine color from the Okabe-Ito palette plus
"auto", initial state, state kind, initial child) and toggles (`bounded`)
commit on click.

| Element | Fields → op |
| --- | --- |
| machine | name → `RenameMachine`; color → `SetMachineColor`; initial → `SetMachineInitial`; fields → `SetMachineFields`; domain → `SetMachineDomain` |
| state | name → `RenameState`; kind → `SetStateKind`; initial child → `SetStateInitial` |
| transition | from, to, trigger, guard, emits, bounded → `UpdateTransition` (entry found by `locate_transition`) |
| event | name → `RenameEvent`; payload → `Batch[RemoveEventDeclaration, DeclareEvent at the same index]` (or `DeclareEvent` when undeclared) |
| controller | name → `RenameController`; "handle event" → `AddHandler`; handlers listed with select and remove |
| handler | rules listed with select and remove |
| rule | fire, target, when, bounded → `UpdateRule` |
| source | name → `RenameExternal`; triggers → `SetExternalTriggers` |

### Committing, undo and the file

```text
Planned { op, label } ──build::pipeline::commit(file text, definition, op)
   edit::apply ──▶ Applied { inverse, touched }        (EditError: nothing written)
   patch_text  ──▶ Patched { text, rewritten }         (PatchError: nothing written)
   analyze_text(text) ──▶ Analyzed                     (does not load: nothing written)
──▶ build::disk::write_atomic (hidden temp file + rename)
──▶ DiskSync::note(text) ──▶ Document::apply(new model) ──▶ PlayState::rebase
──▶ History::record(inverse, label) ──▶ select the first touched key that exists
```

- Undo applies the top undo entry through the same pipeline; its inverse
  goes onto the redo stack (`build::undo::History`, two-phase
  `peek`/`complete`, so a failed undo leaves both stacks as they were).
  Keys: ctrl/cmd-Z undo, ctrl/cmd-shift-Z and ctrl-Y redo. The history keeps
  200 entries.
- **Own writes.** `DiskSync` holds an FNV-1a hash of the text the app last
  read or wrote. A watcher-triggered reload reads the file on the
  background executor (`document::reread`); text that hashes the same is the
  app's own save echoing back and is ignored (the undo history survives).
  The foreground re-checks against the current hash, since another save may
  have happened meanwhile. Any other text is an external edit: it reloads as
  before, and a non-empty undo history is cleared with a status note. An
  unreadable file (e.g. mid-rename) forgets the hash, so whatever comes back
  reloads.
- If `Patched.rewritten`, a one-time warning says comments and formatting
  were not preserved.
- Build mode edits only a `Ready` document: while the file does not load
  (a hand edit in progress, `Stale` or `Failed`), the build bar says it is
  read-only and the banner lists the diagnostics.
- Every stage's error (including today's `NotImplemented` from `apply`,
  `patch_text` and `locate_transition`) shows in the build bar, the status
  bar and, for inspector fields, under the field.

### Play mode

The play panel (`panels/play.rs`, `workspace/playing.rs`) drives one
`play::PlayState`: a `PlaySession` tied to the model generation its ids
belong to, the step range of the last action, and the last error and replay
note.

- **Session:** New session; start from any discovered scenario
  (`load_scenario_file` → `PlaySession::from_scenario`).
- **Instances:** each with its current state and Remove; add one with a
  machine picker, optional name, `key=value` fields and optional start
  state (`play::forms::parse_assignments`).
- **Triggers:** `available_fires` grouped by source (`group_fires`); a
  trigger the target's state does not accept is drawn dashed and muted but
  still fires (and records a drop). An optional payload field applies to the
  next fire.
- **Queue:** `pending()` in order, head marked; **Step** (Space) delivers
  the head, clicking an item delivers that one (`Step { choice }`), **Run
  until quiet** (R) drains the queue.
- **Last action:** its trace steps as `cascade_sim::step_text` lines.
- **Timeline:** a start chip plus one chip per action
  (`play::timeline::chips`); clicking seeks; actions past the position are
  muted and acting then forks (the note says so); branches are listed with
  Switch (`switch_branch`).
- **Save as scenario:** a name (a valid name) → `to_scenario` →
  `scenario_to_yaml` → `scenarios/<name>.yaml` (atomic write), then the
  scenario list is rediscovered so the trace view's picker shows it. An
  empty YAML result is reported as not implemented.
- **Overlay:** `play::overlay::build_overlay` turns `instances()` into
  markers (instance, machine, `ElementKey::State`), the last action's
  transition/emit/deliver/fire/spawn steps into `active` keys (once each,
  in step order), and `pending()` into event and rule keys in queue order.
  It is passed as `SceneInput::play` only in Play mode, without diff mode,
  and only when the session's generation matches the model on screen.
- **After an edit or reload,** `PlayState::rebase` replays the timeline on
  the new model; where it stopped is shown in the panel and the status bar.
  An empty timeline just starts over.

### App files

| File | Role | Key exports |
| --- | --- | --- |
| `crates/cascade-app/src/mode.rs` | Modes, views per mode, scene mode (pure) | `AppMode`, `view_for` |
| `crates/cascade-app/src/build/defs.rs` | Definition lookups, names in use, synthetic elements (pure) | `machine`, `state`, `trigger_names`, `event_names`, `pascal_case`, `split_list` |
| `crates/cascade-app/src/build/ops.rs` | Toolbar adds and deletes as ops (pure) | `Planned`, `PlanError`, `add_machine`, `state_target`, `add_state`, `add_controller`, `add_external`, `delete`, `transition_entry` |
| `crates/cascade-app/src/build/connect.rs` | Drag-to-connect mapping (pure) | `is_source`, `can_connect`, `connect` |
| `crates/cascade-app/src/build/inspector/` | Inspector fields, live validation, field → op (pure) | `inspect`, `validate`, `field_op`, `Inspection`, `FieldId`, `FieldInput`, `FieldValue`, `FieldError` |
| `crates/cascade-app/src/build/pipeline.rs` | apply → patch → load (pure) | `commit`, `commit_with`, `Commit`, `CommitError` |
| `crates/cascade-app/src/build/undo.rs` | Undo/redo stacks (pure) | `History`, `Entry`, `Direction` |
| `crates/cascade-app/src/build/disk.rs` | Atomic writes, own-write detection, new files | `write_atomic`, `DiskSync`, `DiskChange`, `ContentHash`, `create_new`, `NEW_FILE_TEXT` |
| `crates/cascade-app/src/play/mod.rs` | Session state for the panels (pure) | `PlayState` |
| `crates/cascade-app/src/play/overlay.rs` | Session → `PlayOverlay` (pure) | `build_overlay` |
| `crates/cascade-app/src/play/timeline.rs` | Timeline chips and labels (pure) | `chips`, `action_label`, `branch_label`, `acting_forks` |
| `crates/cascade-app/src/play/forms.rs` | Assignments, palette grouping, scenario saving | `parse_assignments`, `group_fires`, `scenario_path`, `scenario_text`, `write_scenario` |
| `crates/cascade-app/src/workspace/building.rs` | Build glue: commit, undo/redo, toolbar, connect, inspector inputs, new file | `BuildUi`, `InspectorUi`, `Workspace::commit`, `undo_redo`, `sync_inspector`, `commit_field` |
| `crates/cascade-app/src/workspace/playing.rs` | Play glue: actions, seek, branches, scenarios | `PlayUi`, `Workspace::play`, `seek`, `save_scenario`, `load_play_scenario` |
| `crates/cascade-app/src/panels/build.rs` | Build bar and inspector panel | `render_build_bar`, `render_inspector` |
| `crates/cascade-app/src/panels/play.rs` | Play panel | `render_play_panel` |

### App invariants

- Nothing is written unless apply, patch and load all succeed; a rejected
  edit changes neither the file nor the history.
- The app's own saves never clear the undo history; external changes always
  do (when there is history to clear).
- Play overlays are only drawn over the model generation the session was
  built from.
- Pure logic (ops, connect mapping, inspector, pipeline, history, own-write
  detection, overlay, timeline, forms) has no GPUI dependency and is unit
  tested; `workspace/*` and `panels/*` only wire it to the UI.

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
  controller it joins; a node joining a gutter moves nothing already in
  it (it takes the next column); appending a source moves nothing above
  or in its gutter, and can shift everything below down, but only as one
  piece (the engine gives a new node without neighbours in its band a
  new row). Entering edit mode opens the gutters, so lanes shift down,
  but states keep their place within their lane. Undoing an edit returns
  to the earlier picture from the layout cache.
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

## Saving edits (YAML patch)

`cascade_interop::patch_text(text, op)` applies an `EditOp` to the
definition file's text and returns the new text. Everything the op does
not touch stays byte for byte: comments, blank lines, key order, quoting,
flow or block style and indentation. Only when a surgical patch is
impossible does it re-emit the file with `to_yaml` and set
`rewritten: true`, so the app can warn that comments were dropped.

### Flow

```text
text ──parse_definition──▶ Definition ──edit::apply──▶ expected definition (or NotImplemented)
text ──ops::apply(op)──▶ patched text        (steps: Doc::parse ─▶ splices ─▶ apply, repeated)
patched text ──parse──▶ compare with expected, ignoring spans
   equal     ─▶ Patched { text, rewritten: false }
   different ─▶ Patched { to_yaml(expected), rewritten: true }
```

1. The original text must parse (`PatchError::Unparseable` otherwise), and
   `edit::apply` decides whether the op is valid (`PatchError::Edit`).
2. The op is applied to the text as one or more *steps*. Each step parses
   the current text into a span index (`Doc`), computes non-overlapping
   splices (byte range → replacement) and applies them. Ops with dependent
   edits (cascading removals, form conversions, field-by-field updates)
   run several steps, re-parsing in between. A `Batch` applies its ops in
   order without validating in between.
3. The patched text is parsed and compared with `apply`'s definition,
   ignoring spans. Any difference, or any step that cannot be done in
   place, falls back to the rewrite.

**Before `edit::apply` lands** (it returns `NotImplemented` on this
branch), the comparison is skipped: the patched text must resolve, else
the op is reported as `Edit(Invalid)`, and an op that cannot be patched in
place returns `PatchError::NotImplemented`, since there is nothing to
rewrite from. No code change is needed when `apply` lands; the check turns
on by itself. The tests carry their own span-free expectation for every
case (the parsed original with the edit applied by hand, compared through
`to_yaml`), and were also run against the `feat/edit-ops` implementation:
every patch agreed with `apply`.

### Span strategy

`Doc` is a private index built from saphyr's marked nodes
(`MarkedYamlOwned`): every node with exact byte offsets (saphyr counts
characters, so offsets are converted), scalar quoting style, collection
style, and the offset of each block sequence item's `-`. saphyr's end
markers are unreliable, so ends are recomputed:

- quoted scalars end at their closing quote (saphyr includes trailing
  blanks and comments);
- flow collections end after their closing bracket, found by scanning
  from the last child over blanks, commas and comments;
- block collections end where their last descendant ends;
- an implicit null (`key:`) sits just past its colon.

A line table answers the rest. A block entry *owns*:

- the comment lines directly above it at its own column (never the
  file's header comment block);
- its own lines up to where its content ends;
- comment lines below that are indented deeper than the entry, up to the
  next line at its column or shallower.

Removing an entry deletes exactly the lines it owns plus one side of its
blank-line separation. Inserting mirrors that, so adding then removing an
entry restores the text byte for byte. Core's public API is unchanged; the
only change outside `patch/` makes `yaml::quote` visible within the crate.

### Style rules

- **Scalars** that change keep their quoting style (`'…'` stays single
  quoted, `"…"` double quoted, plain stays plain when it can). New names
  are plain unless YAML would read them differently. New free text
  (`guard:`, `when:`) is double-quoted when most of the file's free text
  is.
- **Transitions** are written like their neighbours. Among one-line flow
  mappings a new row is `{ from: …, to: …, on: … }`, padded to the
  neighbours' `to:`, `on:` and extra-key columns when every row shares
  them (the order-fulfillment example), single-spaced otherwise (the shop).
  Among block mappings (the majority style) it is a block mapping with
  the neighbours' key indentation. In a flow list it joins the list.
- **Aligned rows stay aligned**: when a value inside a padded flow row
  changes (a rename), the padding after it shrinks or grows so the next
  key keeps its column.
- **States** go into the collection they live in. A plain state joins a
  flow list `states: [a, b]`. A state with a body turns the flow list into
  a block list, only then. Inside a flow mapping, where block form is
  impossible, it is written inline as `{ name: { kind: final } }`. Block
  lists get `- name`, `- name: { kind: final }` or a compound block body;
  mapping-form lists get `name: {}` entries. A plain state that gets a
  child becomes `- name:` with a body; a body that loses its last key
  collapses back to a plain name.
- **Keys** that appear go before the first present key that follows them
  in canonical order (`to_yaml`'s order), else after the last one that
  precedes them. This keeps an author's own order (the shop puts
  `fields:` before `initial:`).
- **Sections** (`events:`, `controllers:`, `external:`) are created in
  canonical order (system, machines, events, controllers, external),
  separated by a blank line when the file's sections are. An empty
  section left by a removal is removed. `controllers: {}` becomes a block
  section when a controller is added. A new `system:` line opens the file
  after its header comments.
- **Indentation** follows the surrounding block: the column of the
  siblings for new entries, and for new nested blocks the file's
  indentation unit and sequence offset (detected from the first nested
  mapping and the first block sequence under a key).
- **Siblings** separated by blank lines (machines, controllers) get a
  blank line around a new entry.
- **Lists of names** (`fields`, `emits`, `from`, `payload`, external
  triggers) change item by item, keeping unchanged items' text. A single
  name becomes a flow list when a second one is added.
- **Rules** follow their neighbours (block or flow mappings). A handler
  written as one flow rule mapping becomes a one-item list when a rule is
  added. A handler that loses its last rule is `Event: []`.
- **Renames** replace every reference occurrence. Machines: the key,
  `fire:` refs, target selector machine words (keeping the selector's
  spacing) and external triggers. States: the name, the parent's
  `initial:`, and every state reference (machine `initial:`, transition
  `from`/`to`), keeping bare names bare unless they would become
  ambiguous or captured, then written as full paths. Events: the
  declaration, `emits:` and handler keys. Adding a state rewrites bare
  references it would capture as full paths.
- **A payload change** (`Batch[RemoveEventDeclaration(e),
  DeclareEvent(e', same index)]`) is patched in place, keeping the
  declaration's comments, when other declarations remain. Removing the
  only declaration makes the file lenient, so re-declaring then declares
  every event in use, as `apply` does.

### Fallback cases

These are left to the rewrite:

- nodes with YAML tags (`!tag value`);
- editing a block scalar (`|`, `>`) or a multi-line plain scalar;
- restructuring a flow collection that contains comments (turning it
  into a block list, removing an item across a comment);
- adding a rule to a handler written as a single block mapping;
- declaring an event with a payload in a list-form `events:` whose items
  are not indented under the key.

On every example and fixture, a matrix of about 670 valid ops of every
kind (`tests/patch_invariants.rs`) never falls back. A broader
cross-check of 2164 valid candidate ops from `feat/edit-ops` (including
batches) also agreed with `apply` in every case, with no rewrite. Undoing
each of them by patching `apply`'s inverse op restored the original text
byte for byte in about 89% of cases. The rest differ only in style that
the inverse cannot know: comments of removed entries, a flow list that
became a block list, removed elements re-rendered in canonical style.

### Invariants

- `parse(patch_text(text, op).text) == apply(parse(text), op).definition`,
  ignoring spans. It is checked on every call once `apply` exists, and a
  mismatch rewrites.
- Unless `rewritten` is set, every comment survives except those owned by
  removed entries, and text outside the edited entries is unchanged.
- Adding then removing an entry restores the text byte for byte (tested
  for machines, states, transitions, events, controllers, handlers, rules,
  external sources, renames and the system name on every fixture). One
  exception: `controllers: {}` stays a block section.
- Patching never parses or validates intermediate batch steps.

### Files

| File | Role | Key exports |
| --- | --- | --- |
| `crates/cascade-interop/src/patch/mod.rs` | Entry point, verification against `edit::apply`, rewrite fallback | `patch_text`, `Patched`, `PatchError` |
| `crates/cascade-interop/src/patch/doc/` | Span index over the text (`build.rs`: saphyr nodes → byte ranges; `lines.rs`: line table) | `Doc`, `Node`, `Kind`, `Scalar`, `ScalarStyle`, `Lines` (crate) |
| `crates/cascade-interop/src/patch/block.rs` | Block entry regions (comments, blank-line separation), removal and insertion | `region`, `remove`, `insert`, `separated` (crate) |
| `crates/cascade-interop/src/patch/collection.rs` | Generic edits: flow insert/remove, set or remove a key, reconcile a list of names, flow → block conversion, alignment-keeping scalar replacement | `set_key`, `set_scalar_key`, `set_text_key`, `set_names`, `replace_scalar`, `replace_value`, `remove_child` (crate) |
| `crates/cascade-interop/src/patch/render/` | New text in the file's style: detected `Style`, flow rows with `RowLayout`, states, machines, controllers, rules; style-keeping scalars | `Style`, `Block`, `RowLayout`, `restyle`, `free_text` (crate) |
| `crates/cascade-interop/src/patch/ops/` | One module per op group (`machines`, `states`, `transitions`, `events` with external sources, `controllers`), navigation (`nav`), state references (`refs`) | `apply` (crate) |
| `crates/cascade-interop/src/patch/splice.rs` | Byte-range splices applied in one pass | `Splice`, `apply` (crate) |
| `crates/cascade-interop/src/patch/verify.rs` | Span-free definition comparison | `same_definition` (crate) |
| `crates/cascade-interop/src/patch/error.rs` | Why a surgical patch failed | `SurgeryError`, `Unsupported` (crate) |
| `crates/cascade-interop/tests/patch_*.rs`, `tests/patch_common/`, `tests/fixtures/patch/` | Golden line diffs per op on the examples and nested, flow and block (4-space) fixtures; invariants matrix; fallback cases | — |
