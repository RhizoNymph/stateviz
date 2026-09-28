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
| `crates/cascade-sim/src/session/mod.rs` | Interactive simulator | `PlaySession`, `PlayAction`, `PendingItem`, `InstanceState`, `AvailableFire`, `Timeline`, `Branch`, `scenario_to_yaml` |
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
