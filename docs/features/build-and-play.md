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
- A session's trace equals the batch simulator's for the same actions;
  `to_scenario` then `from_scenario` reproduces the trace exactly.
