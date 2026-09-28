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
