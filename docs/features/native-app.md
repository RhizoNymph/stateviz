# Native app (`cascade-app`)

The GPUI desktop app. It replaces the spec's web app: it opens one
definition, paints the four views from `cascade-scene` scenes, and keeps
up with the file, its pins sidecar and its scenarios as they change on disk.
It is also a workbench: Build mode edits the definition (saved back to the
YAML file in place) and Play mode drives an interactive simulator session
over the canvas. Build and play are documented in
[build-and-play.md](build-and-play.md#app-build-and-play-modes).

```text
cascade-app [--new] <file> [--view <cascade:// link>] [--mode view|build|play]
```

`--new` creates `<file>` with a starter definition (one machine, one state)
and opens it in Build mode; it refuses to overwrite an existing file.

From the CLI, `cascade new [file]` (default `cascade.yaml`) creates a
**blank** definition (`machines: {}`, no machines yet) and launches the app
on it in Build mode (`--mode build`); `--no-open` only creates the file. It
never overwrites an existing file (exit code 2). The canvas shows the
empty-system note until the first machine is added, and that first edit
patches the blank file in place, keeping its header comment.

`CASCADE_EDITOR` sets the click-to-source command (see below);
`XDG_CONFIG_HOME` (else `HOME`) locates the settings file (see "Settings");
`RUST_LOG`
sets log filters (default `cascade_app=info`). Exit code 2 means a bad
argument (missing file, invalid `--view` link).

## Scope

- Bootstrap: arguments, logging, the GPUI application and its one window.
- The document: loading, analysis, live reload with last-good retention and
  diagnostics, pins sidecar and scenario discovery.
- The canvas: painting every `Scene` field, pan/zoom/fit, hit testing,
  hover, click, shift-click, double/secondary-click, alt-click, node drag.
- Interaction: selection shared across views, cone tracing with a depth
  stepper, dim/hide, path queries, legend entity filter, findings panel,
  fuzzy search, trace view scenario/race picker, matrix cell drill-down,
  click-to-source, pins, diff mode, shareable links, light/dark theme,
  status bar, the structure view's pills/arrows toggle.
- App settings remembered between runs (`settings.rs`).
- Modes: View, Build (toolbar adds and deletes, drag-to-connect, inspector,
  undo/redo, saving edits to the file, new files) and Play (instances,
  trigger palette, queue stepping, timeline with branches, scenarios).

## Non-scope

- What a view draws, emphasis, badges, diff decorations and hidden-machine
  stubs: the scene builders decide (view-scenes.md). The app paints what
  the scene says and never recolours it.
- Checks, simulation, git access and model merging: the app calls
  `analyze`, `simulate`/`race_orderings`, `read_at_rev` and
  `merge_for_display` and shows their results or errors.
- How edits are applied to the definition and patched into the text
  (`cascade_core::edit`, `cascade_interop::patch`) and how a play session
  simulates (`cascade_sim::session`): the app calls them and shows their
  results or errors.
- Scenario discovery beyond the local fallback described below.

## Data and control flow

### Startup

1. `main` parses `Args` (clap). `--view` goes through
   `ViewState::from_link`; the file is canonicalized so watch events match.
   The settings file is read (`settings::load_or_default`); without a
   `--view` link its `transition_pills` seeds the initial view state (a
   link carries its own).
2. `application().run`: text-input and workspace key bindings are
   installed, a monospace family is picked from the installed fonts
   (`canvas::font::pick_mono`), and one window opens with a `Workspace`.
3. `Workspace::new` loads the definition synchronously (so the first frame
   has content), loads the sidecar, discovers scenarios, starts the file
   watcher, and focuses itself so keys work without a click.

### Load, watch, reload

```text
notify thread ──classify──▶ Changes ──unbounded channel──▶ GPUI task
     (dir of the definition,            │  first change, wait DEBOUNCE (120 ms),
      and scenarios/ if present)        │  drain the rest, merge
                                        ▼
                         Workspace::on_files_changed(Changes)
       definition → start_reload: background_spawn(reread) ─▶ own write? ignore
                                                            ─▶ else Document::apply, clear undo
       sidecar    → reload_sidecar (sync; unchanged content is ignored)
       scenarios  → rediscover, watch scenarios/ if it appeared, rerun traces
```

- The watcher watches directories, not files, because editors often save by
  writing a new file and renaming it over the old one.
- `document::analyze_text` is `load_str` + `CausalGraph::build` + `analyze`;
  the loaded text is kept (`Loaded::text`) for build mode to patch.
- `document::reread` skips analysis when the file's text hashes the same as
  what the app last read or wrote (`build::disk::DiskSync`), so the app's own
  saves do not reload or clear the undo history. See build-and-play.md.
- `Document::apply` is the reload state machine:
  `Empty → Ready | Failed`, `Ready → Ready | Stale`, `Stale → Ready | Stale`,
  `Failed → Ready | Failed`. `Stale` keeps the last good `Loaded` on screen
  while the banner lists the new diagnostics (`line:col: message`).
- Each successful load gets a new `generation`. Traces and diffs record the
  generation they were computed from.
- A new reload replaces (cancels) one still in flight.

### Input → ViewState → SceneBuilder → paint

```text
keys ── GPUI actions ──▶ Workspace::run_command ──▶ commands::reduce(ViewState)
mouse ── canvas listeners ──▶ gesture::Gesture ──▶ classify_click / pan / drop
panels ── on_click ──▶ Workspace methods
                                   │
                          Workspace::changed: start trace / diff work if the
                          new state needs it; scene_dirty = true; notify
                                   │
render ──▶ rebuild_scene (once per frame at most):
           SceneInput { model, graph, findings, view, theme, MonoMeasure,
                        sidecar, traces, diff } ──SceneBuilder::build──▶ Rc<Scene>
       ──▶ canvas(prepaint: record bounds; paint: canvas::paint::paint_scene)
```

- All app state lives in the `Workspace` entity; the scene is derived and
  rebuilt on the main thread (`SceneBuilder` keeps previous layouts per
  view). Builds are timed at `debug` level.
- Painting: background, lanes, `Layer::Under` overlays, edges, nodes,
  `Layer::Over` overlays, then the dragged node. Every item is transformed
  through the viewport (`viewport.rs`) and culled against the canvas.
  Shapes: pill (fully rounded quad), rect, rounded rect, stub (rounded,
  dashed by its stroke), hexagon and tag (`PathBuilder` polygons).
  `Border::Double` draws a second inset outline; `Border::ThickLeft` a bar.
  Dashed strokes use `PathBuilder::dash_array` (edges, polygons) or
  `BorderStyle::Dashed` (quads). Arrowheads are filled triangles on the last
  segment. Badges are outlined circles with the count. Every color is
  multiplied by its item's opacity.
- Labels are shaped with the monospace family and a forced per-glyph
  advance of `0.6 × font size`, so text occupies exactly the width
  `MonoMeasure` gave the builder. Screen font sizes are rounded to 0.25 px so
  zooming reuses GPUI's line cache; labels under 3.5 px are skipped.
- Hover thickens the outline (weight, never hue).
- The viewport lives in `ViewState::viewport`. `None` means "fit": each
  frame fits the scene to the canvas. Pan/zoom store an explicit viewport;
  `0` returns to fit. Each view keeps its own viewport while another view
  is shown.

### Selection, cones, findings, search

- Click selects (`commands::select`); shift-click toggles a second
  selection (`add_to_selection`), and two transitions form a path query,
  which drops any cone. Esc peels one layer: selection and cone, then the
  matrix machine pair, then the search highlight (clearing the search box).
- `F`/`B` set or toggle the forward/backward cone of the selection with the
  current depth; `[`/`]` step the depth `0 … 12, ∞` (it is remembered while
  no cone is active); `H` toggles dim/hide.
- **Group by machine** (toolbar toggle, shown in the causal view, or `G`)
  flips `ViewState::group_by_machine`: the causal view is drawn with one
  lane per machine on shared causal columns (view-scenes.md, "Causal
  lanes"). The toggle clears the stored viewport so the new picture is
  fitted; selection, cone and search carry over. `G` was free (no other
  binding uses it), so it needed no substitute.
- **Pills** (toolbar toggle, shown in the structure view in every mode, or
  `P`) flips `ViewState::transition_pills`: transitions are drawn as pills
  or as one labelled state → state arrow each (view-scenes.md, "Arrow
  mode"; build-and-play.md for building with arrows). The causal view
  keeps its pills. Like the lanes toggle it clears the stored viewport so
  the new picture is fitted, and carries over selection, cone and search.
  The toggle (button or key) also saves the choice to the settings file;
  a pasted link changes the view but not the setting. `P` was free.
- Clicking a stub, trace step or lifeline selects the element behind it
  (`locate::target_key`), so selection is shared across all views.
- A finding click selects `detail.primary()`, switches to the causal view
  and centres the element, falling back to the finding's other subjects and
  then related elements (a state's transitions, a rule's handler) when the
  primary is not drawn. A race candidate instead opens the trace view with
  `race` set, when a scenario exists.
- Search runs `cascade_core::search::search` on every edit, shows up to 12
  hits, and sets `ViewState::search` for highlighting. Enter (or a click)
  selects the highlighted hit, centres it (switching to the causal view if
  the current view does not draw it) and returns the keyboard to the canvas.

### Trace view

- Scenario files: `scenarios/*.yaml|yml` and `*.scenario.yaml|yml` next to
  the definition (`document::scenarios`, a local fallback until the
  simulator ships discovery). The id is the file name without extensions;
  `ViewState::scenario` stores it. Entering the trace view picks the first
  scenario when none is selected.
- `TraceRequest { scenario, race, generation }` runs on the background
  executor: parse, then `simulate` or `race_orderings` (for `race`, the
  n-th race-candidate finding). Results are used only when their request
  (including the generation) matches the current one, because traces hold
  ids of one model. Errors show in the trace panel.

### Click-to-source

Double-click, cmd/ctrl-click, or `O` on the selection resolves the key in
the working-tree model, takes `span_of`, and builds a command
(`editor::build_command`, pure): `CASCADE_EDITOR` as a template with
`{file}`, `{line}`, `{col}` (the file is appended when absent; quotes group
words), else `zed file:line`, else `code -g file:line`, else `xdg-open file`
(`open` on macOS). The child is spawned with null stdio and reaped by a
timer task.

### Pins

Dragging a node in the causal or structure view previews it under the
pointer; on release its new top-left corner is pinned
(`LayoutSidecar::pin`) and saved atomically (`save_sidecar`), and the scene
is rebuilt. Alt-click, or `U` on the selection, unpins. Pins are per view.
The resulting sidecar change on disk is read back and ignored when equal.

### Diff mode

The toolbar takes a base ref and an optional head (blank = working tree);
Enter or Compare sets `ViewState::diff`. A `DiffRequest` (refs plus the
working-tree generation when the head is the working tree) runs on the
background executor: `read_at_rev` + `load_str` for each ref, then
`merge_for_display` and `CausalGraph::build` on the merged model. The causal
and structure views draw the merged model with `SceneInput::diff` and no
findings (finding ids belong to the head model); the trace and matrix views
keep showing the working tree. While a reload recomputes the diff the
previous display stays up. Errors (stubs today) appear in the banner.

### Links and theme

- Copy link writes `ViewState::to_link()` with the on-screen viewport to the
  clipboard. Paste link finds the first `cascade://` link in the clipboard
  text and replaces the whole view state (search box and diff fields
  included); unknown keys are kept and counted in the status bar. Links
  carry the lanes toggle as `lanes=1` and arrow mode as `pills=0`.
- The theme follows the window appearance; the toolbar button or
  ctrl/cmd-shift-T cycles system → the opposite → system. The chrome
  (`theme::Chrome`, a GPUI global) derives from the scene theme.

### Settings

`settings.rs` keeps what the app remembers between runs in
`$XDG_CONFIG_HOME/cascade/settings.json`, or
`~/.config/cascade/settings.json` when `XDG_CONFIG_HOME` is unset or not an
absolute path:

```json
{
  "transition_pills": false
}
```

- `AppSettings` is a typed serde struct (`#[serde(default)]`): missing keys
  take their defaults (`transition_pills: true`) and unknown keys are
  ignored, so files written by older or newer versions load.
- Reading happens once at startup, synchronously. A missing or invalid
  file, or no config directory (neither variable set), logs a warning and
  falls back to the defaults; it is never fatal.
- Writing happens when the Pills toggle is used: `Workspace::save_settings`
  serialises the settings and writes them on the background executor
  (directory created, atomic temp file + rename through
  `build::disk::write_atomic`). A failure is logged and shown in the
  status bar.
- Errors are `SettingsError` (`NoConfigDir`, `Read`, `Parse`, `Encode`,
  `Write`, each with the path where there is one).

## Key bindings

Plain keys are bound in `Workspace && !TextInput`, so they are inert while
a text field has focus; chords are bound in `Workspace`.

| Keys | Action |
| --- | --- |
| `Esc` | Clear selection and cone, then the machine pair, then the search |
| `F` / `B` | Forward / backward cone of the selection (again to turn off) |
| `[` / `]` | Cone depth down / up (`0 … 12, ∞`) |
| `H` | Dim or hide what is outside the cone or path query |
| `G` | Causal view: group by machine (lanes) on or off |
| `P` | Structure view: transitions as pills or as arrows (remembered) |
| `1` `2` `3` `4` | Causal, structure, trace, matrix view |
| `+` (`=`) / `-` | Zoom in / out |
| `0` | Fit to view |
| arrow keys | Pan |
| `/`, ctrl/cmd-F | Focus search |
| `O` | Open the selection's source |
| `U` | Unpin the selection |
| ctrl/cmd-shift-C | Copy view link |
| ctrl/cmd-shift-V | Paste view link |
| ctrl/cmd-shift-T | Toggle theme |
| ctrl/cmd-R | Reload the definition, pins and scenarios |
| ctrl/cmd-Q | Quit |
| ctrl/cmd-1 / 2 / 3 | View / Build / Play mode |
| ctrl/cmd-Z | Undo the last edit |
| ctrl/cmd-shift-Z, ctrl-Y | Redo |
| `Delete`, `Backspace` | Build mode: delete the selection |
| ctrl/cmd-N | New definition file |
| `Space` | Play mode: deliver the queue's head |
| `R` | Play mode: run until the queue is quiet |

Undo and redo are chords, so they also work (on the definition) while a
text field has focus; the text fields have no undo of their own.

In a text field: Enter submits, Esc returns to the canvas, ↑/↓ move through
search results, plus the usual editing keys.

| Pointer | Action |
| --- | --- |
| click | Select |
| shift-click | Add or remove a second selection (path query) |
| double-click, ctrl/cmd-click | Open source |
| alt-click | Unpin |
| drag a node (causal, structure) | Pin it where it is dropped |
| drag the background | Pan |
| drag a connect handle (Build) | Connect: transition, emit, fire or source trigger (see build-and-play.md) |
| drag from within 8 px of a transition arrow (Build, pills off) | Connect from that transition, as from its handle; a click selects it |
| scroll / shift-scroll | Pan / pan horizontally |
| ctrl/cmd-scroll, pinch | Zoom about the pointer |
| matrix cell click | Causal view restricted to that machine pair |

## Files

| File | Role | Key exports |
| --- | --- | --- |
| `crates/cascade-app/src/main.rs` | Arguments, logging, GPUI bootstrap, window | `main` |
| `crates/cascade-app/src/args.rs` | CLI arguments | `Args`, `Args::initial_view`, `Args::initial_mode` |
| `crates/cascade-app/src/document/mod.rs` | Load + analyze; own-write-aware re-read; reload state machine | `analyze_text`, `load_definition`, `reread`, `Reread`, `Analyzed`, `Loaded`, `LoadFailure`, `DocState`, `Document` |
| `crates/cascade-app/src/document/scenarios.rs` | Scenario discovery fallback | `ScenarioFile`, `scenario_id`, `is_scenario_path`, `discover` |
| `crates/cascade-app/src/watch.rs` | notify watcher, path classification | `WatchTargets`, `Changes`, `classify_event`, `is_relevant`, `FileWatcher`, `DEBOUNCE` |
| `crates/cascade-app/src/commands.rs` | Key map and view-state reducer (pure) | `Command`, `Scope`, `KEYMAP`, `reduce`, `Outcome`, `HostEffect`, `select`, `add_to_selection`, `depth_more`, `depth_less` |
| `crates/cascade-app/src/gesture.rs` | Pointer state machine (click, pan, pin drag, connect drag), click classification (pure) | `Gesture`, `Pick`, `Draggable`, `Mods`, `classify_click`, `ClickAction` |
| `crates/cascade-app/src/viewport.rs` | Scene ↔ screen transforms, fit, zoom, pan (pure) | `ScreenPoint`, `ScreenRect`, `to_screen`, `to_scene`, `fit`, `zoom_about`, `pan_by`, `center_on` |
| `crates/cascade-app/src/settings.rs` | Settings file: path, typed load/save, defaults on failure | `AppSettings`, `SettingsError`, `default_path`, `path_from`, `load`, `load_or_default`, `save` |
| `crates/cascade-app/src/build/pick.rs` | What a press or drop lands on: pick tolerances, arrow picking on the build canvas (pure) | `HIT_TOLERANCE_PX`, `ARROW_PICK_PX`, `scene_units`, `Picking`, `hit`, `connect_source` |
| `crates/cascade-app/src/locate.rs` | Hit target → key; where a key is drawn; drop targets; race indices | `target_key`, `drop_key`, `drop_targets`, `locate_key`, `locate_with_fallback`, `locate_finding`, `race_finding`, `race_index` |
| `crates/cascade-app/src/editor.rs` | Click-to-source command builder (pure) and spawn | `build_command`, `command_for`, `split_words`, `find_on_path`, `EditorCommand`, `Location` |
| `crates/cascade-app/src/link.rs` | Link copy/paste flow (pure) | `link_for`, `parse_pasted`, `unknown_keys` |
| `crates/cascade-app/src/trace.rs` | Trace requests and runs | `TraceRequest`, `TraceRun`, `run`, `TraceError` |
| `crates/cascade-app/src/diffmode.rs` | Diff requests, runs and merge | `DiffRequest`, `DiffRun`, `DiffDisplay`, `compute`, `applies_to`, `refs_from_fields` |
| `crates/cascade-app/src/theme.rs` | Theme choice, color conversion, chrome palette | `ThemeChoice`, `scene_theme`, `hsla`, `Chrome`, `ActiveChrome`, `chrome` |
| `crates/cascade-app/src/input.rs` | Single-line text input (from GPUI's example) | `TextInput`, `InputEvent`, `bind_keys`, `CONTEXT` |
| `crates/cascade-app/src/canvas/paint.rs` | Scene painter, connect rubber band and drop highlights | `paint_scene`, `PaintInput`, `ConnectPaint` |
| `crates/cascade-app/src/canvas/shapes.rs` | Shape geometry, dashes, arrowheads, culling (pure) | `hexagon`, `tag`, `arrowhead`, `dash_pattern`, `quantize_font`, `visible` |
| `crates/cascade-app/src/canvas/font.rs` | Monospace family choice | `pick_mono` |
| `crates/cascade-app/src/workspace/mod.rs` | Root view: state, layout, theme sync, status expiry | `Workspace`, `CanvasState`, `SearchState`, `Status` |
| `crates/cascade-app/src/workspace/actions.rs` | GPUI actions for commands, key binding install | `bind_keys`, `register` |
| `crates/cascade-app/src/workspace/scene.rs` | Scene rebuild, viewport helpers, trace and diff orchestration | `Workspace::changed`, `rebuild_scene`, `switch_view`, `trace_request`, `diff_request` |
| `crates/cascade-app/src/workspace/operations.rs` | Files, commands, selection, findings, pins, source, links, search, diff | `Workspace::run_command`, `open_finding`, `pin`, `unpin`, `open_source`, `pins_supported` |
| `crates/cascade-app/src/workspace/canvas_events.rs` | Canvas element and pointer handlers | `Workspace::render_canvas_area` |
| `crates/cascade-app/src/panels/*.rs` | Toolbar (mode control, the causal view's group-by-machine toggle, the structure view's pills toggle), build bar, inspector, play panel, sidebar (legend, findings), overlays (search, notes, trace picker), banner and status bar | `render_*` methods on `Workspace` |
| `crates/cascade-app/src/mode.rs` | View/Build/Play modes (pure) | `AppMode`, `view_for` |
| `crates/cascade-app/src/build/*` | Build mode logic: ops, connect, inspector, pipeline, undo, disk (pure) | see build-and-play.md |
| `crates/cascade-app/src/play/*` | Play mode logic: session state, overlay, timeline, forms (pure) | see build-and-play.md |
| `crates/cascade-app/src/workspace/building.rs`, `playing.rs` | Build and play glue | `BuildUi`, `PlayUi` |

## Invariants and constraints

- The scene is derived: only `Workspace` state changes, through methods
  that end in `changed` (or `notify` for viewport-only changes).
- Hue means machine. The painter never changes a scene color; hover and
  pressed chrome use weight and neutral backgrounds. Chrome uses hue only
  for severity.
- Traces are paired only with the model generation they were computed
  from; stale results are discarded. Diff displays are self-consistent
  (merged model, graph and statuses together) and are never mixed with the
  working-tree findings.
- The last good model stays on screen after a failed reload; the banner
  names the failure time and the version shown.
- Plain-key bindings never fire while a text field has focus.
- A settings file can never stop the app from starting: every read or
  write failure falls back to defaults or a status message.
- Blocking work (file reads for reloads, git, simulation) runs on GPUI's
  background executor; results come back through the entity. The notify
  thread only classifies and sends; there is no other shared state.
- No `unwrap`/`expect` outside tests; failures become status messages,
  banner entries or `tracing` warnings.
- `cascade-app` is excluded from the workspace's default members; build it
  with `cargo build -p cascade-app`.

## Known gaps

- GPUI's `TestAppContext` needs `gpui/test-support`, which pulls a git fork
  of `proptest` not in the offline cache, so the GPUI glue is covered by
  launching the app rather than by UI tests; all pure logic is unit tested.
- Structure, trace and matrix scenes, SVG export, analysis, the simulator,
  git reading and model diffing are stubs on this branch; the app shows
  their notes or errors until the sibling workstreams land.
- On `feat/app-build-play`, `edit::apply`, `locate_transition`,
  `patch_text`, the play session, edit-mode wiring/handles and play overlays
  are stubs: edits and play actions report "not implemented" in the UI, and
  connecting works through shift-click + **Connect selection** until the
  scene draws connect handles.
