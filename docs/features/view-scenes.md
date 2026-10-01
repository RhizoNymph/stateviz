# View scenes

`cascade-scene` turns a model plus a view state into a `Scene`: a
backend-neutral display list with every visual decision baked in. The GPUI
app only paints and hit-tests it; SVG and PNG export draw the same scene.

## Scope

- Scene builders for the four views: causal flow, structure, trace, matrix.
- The visual encoding: machine hues (Okabe-Ito, domain grouping past eight
  machines), shapes, borders, dashes, arrow styles, badges, diff
  decorations.
- The interaction model (`emphasis`): selection, cone tracing, path
  queries, search, and how each scene item responds (outline weight,
  dimming, hiding with stubs).
- Structural filters: entity filter (hidden machines as stubs), machine
  pair, collapse, hide mode.
- Layout caching and stability across rebuilds, pins from the sidecar.
- SVG and PNG export, and `cascade render`.
- Build and play drawing: `SceneMode::Edit` (the structure view's wiring
  in gutters between the lanes, and connect handles) and the `PlayOverlay` (instance markers,
  active and pending items) over the causal and structure views.

## Non-scope

- Layout itself (`cascade-layout`; see layered-layout.md). Views code
  against its API and do not depend on the stub engine's behaviour.
- Painting, pan/zoom and hit-test dispatch (the GPUI app).
- Producing traces (`cascade-sim`) and findings (`cascade_core::analyze`).
- The `cascade://` link format and the pins sidecar file format
  (`view_state.rs`, `pins.rs`). The format is unchanged apart from the
  `lanes=1` parameter (`ViewState::group_by_machine`, left out when off).

## Data and control flow

```text
SceneInput { model, graph, findings, view, theme, measure, sidecar, traces, diff, mode, play }
      │
      ├─ Interaction::new(model, graph, view)          selection, focus region, search hits
      │
      ├─ causal / structure:
      │     view builder ──▶ DraftGraph (nodes + edges + groups, each with a NodeLook and a Meta)
      │        filters: machine pair → hidden-machine stubs → collapse (structure)
      │        apply_hide: hide mode removes outside items, records cut links
      │     realize(draft, cuts) ──LayoutCache──▶ Scene items (+ metas aligned with them)
      │     lanes from group rects (structure)
      │     edit mode (structure): gutter lanes, hints, connect handles
      │
      ├─ trace / matrix: placed directly, metas kept alongside
      │
      ├─ Decor::apply(scene, metas)                    emphasis, badges, diff
      │
      └─ PlayDecor::apply (causal, structure)          active, pending, markers; bounds; notes
```

Every scene item carries a `Meta` while it is built: the model elements it
stands for (selection, search and diff match on these; the first is
primary), extra elements whose findings badge it (a handler's rules), and
an `Anchor` tying it to the causal graph (`Nodes`: inside when any node is
in focus; `Chains`: inside when every edge of some chain is; `Free`: never
dimmed). Decoration runs after layout on every build, so it cannot move
anything.

### Interaction model (`emphasis/`)

- **Selection:** `ViewState::selection` keys resolved in the model
  (unknown keys ignored, at most two count). Selected items get
  `theme.selected_stroke_width` and `Emphasis::Selected`, and always count
  as inside the focus, so hiding never removes them.
- **Seeds:** `causal_seeds` maps any element onto causal nodes: sources,
  transitions, events and handlers map to themselves; a machine to its
  transitions; a state to the transitions leaving or entering it or any
  state nested in it; a controller to its handlers; a rule to its handler;
  a trigger to the transitions accepting it (or, for an invalid fire, the
  handlers firing it and the sources exposing it).
- **Cone:** one selection plus `cone` gives
  `CausalGraph::cone(seeds, direction, depth)`.
- **Path query:** two selections give the union of
  `paths_between(x, y)` over their seeds; the cone setting is ignored. An
  empty result leaves a note ("No causal path between …").
- **Emphasis precedence:** selected, then dimmed (outside an active focus:
  `theme.dim_opacity`, 15%), then search match (a dotted halo overlay, no
  hue change), then focused (inside: an outline between normal and
  selected width), then normal.
- **Hide mode** (`outside == Hide`): graph views remove items outside the
  focus before layout. Each removed link with one remaining endpoint
  becomes a short dotted stub leaving that node on the link's side (fanned
  when several share a side) whose hit target is the hidden endpoint.
  Links outside the focus between two remaining nodes are removed. The
  trace view dims instead, since removing steps would break the time axis.

### Causal flow view (`views/causal.rs`)

1. Machine pair (`filters::pair_nodes`): keep both machines' transitions,
   the sources that trigger them, and the events and handlers on links
   between them (either direction, or within one of them).
2. One draft node per kept causal node, in graph order: sources
   (`Rect`, `LayerConstraint::First`), transition pills (`Machine: from →
   to` over the trigger; ports West = in, East = out), event tags, one
   hexagon per handler (labelled with the controller).
3. Transitions of hidden machines collapse onto one stub per machine (at
   the machine's first transition's position). Links touching a stub are
   aggregated per (endpoints, kind) into `EdgeKind::StubLink` edges
   (dotted, keeping the original color; `×n` when several links merge). The
   stub is labelled "Payment, 4 links" with the number of stub links drawn
   and targets `HitTarget::MachineStub`.
4. Edges: trigger (solid neutral, `[guard]` of the target), emit (dashed
   gray, from the East port), subscribe (solid gray), fire (dashed in the
   target machine's hue, `[when] [guard]`, into the West port).
5. Hide mode, layout, decoration. An edge the layout reports `reversed` is
   drawn red with `back_edge = true` only when it lies on a causal cycle
   (both ends in one strongly connected component), so stub links and
   reversals made for other reasons are never mistaken for cascade cycles.

### Causal lanes (`views/causal/lanes.rs`)

`ViewState::group_by_machine` (link parameter `lanes=1`, the app's **Group
by machine** toggle and `G`) draws the same causal graph with one lane per
machine. Everything above still applies (machine pair, hidden-machine
stubs, hide mode, emphasis, cones, path queries, search, badges, diff, play
overlay, red back edges); only these change:

- **Lanes:** one layout group per machine in definition order, drawn like
  the structure view's machine lanes (`structure::machine_lane`: pale hue
  fill, header with the machine name in its hue, bold; a selected machine
  outlines its lane at the selected width), then an **Unattached** lane
  (neutral fill, dashed rule outline, muted title, no hit target). Lanes
  without nodes (hide mode, pair filter) are not drawn.
- **Placement** (looked up in the model, so filters never move a node to
  another lane):

  | Node | Lane |
  | --- | --- |
  | Transition pill | its machine |
  | Hidden machine's stub | that machine (the lane is marked `collapsed`) |
  | Event | the machine of the first transition in definition order that emits it |
  | Handler | the machine its first rule fires into |
  | External source | the machine of its first trigger |
  | An event nothing emits, a handler without rules, a source without triggers | Unattached |

  Alternatives were measured (readability.md, "Causal lanes"); these rules
  are kept.
- **Pills** read `from → to` over the trigger, without the machine name
  (the lane says it). States are still not drawn.
- **Layout:** `LayoutOptions::shared_layers` (layered-layout.md). A node's
  column is its causal layer, shared by every lane; columns are as wide as
  their widest item in any lane.

Guarantees (tested in `tests/causal_lanes.rs`, measured in the readability
report):

- Every edge that is not a cascade-cycle back edge points right: it ends
  right of where it starts and no horizontal segment of it runs right to
  left, within a lane or across lanes (`metrics::leftward_edges` is 0 for
  both examples, in hide mode and with hidden machines). Back edges of
  genuine cascade cycles stay red and are the only edges running left.
- No side corridors, no edge through a node, no label overlaps in either
  example.
- Emphasis never relayouts; an edit in one machine leaves the other lanes'
  nodes in place; toggling lanes off returns exactly the flat layout (the
  cache seeds a layout only from one made with the same options, so the
  lanes are never seeded by the flat layout or the other way round).
- With the toggle off the causal scene is exactly as before
  (`tests/view_mode_golden.rs` unchanged).

Known gap: the pins sidecar keys pins by view, so the flat causal view and
its lanes share one set of pins; a node pinned in one is placed at the
same point in the other.

### Structure view (`views/structure/`)

- **Lanes:** one layout group per machine in definition order, drawn as a
  `Lane` whose rect is the union of the machine's groups, header with the
  machine name in its hue.
- **States:** rounded rects with the machine's pale fill, laid out in
  layers with a pill per transition between them (state → pill West,
  pill East → state; the first arrow has no head and carries `[guard]`, the
  second has the arrowhead). Pills are labelled `from → to` (full paths)
  over the trigger.
- **Nesting (design choice):** the layout has one level of groups, so each
  expanded compound state gets a band of its own: a further group placed
  right after its machine's lane group, drawn as a nested `Lane` (inset,
  dashed outline, titled with the state's path) inside the machine lane.
  The compound state stays a node in its parent's band, marked
  "▾ n states", because transitions can leave or enter it as a whole. A
  pill goes in the band of the deepest compound state containing both of
  its (visible) endpoints.
- **Collapse:** a collapsed compound state keeps its node ("▸ n states");
  its descendants and the transitions inside it disappear, and transitions
  crossing its boundary attach to it (pills keep their real endpoints in
  the label). A collapsed machine becomes one node (double border, "n
  states · m transitions") in a lane with `collapsed = true`.
- **Cross-lane links (design choice):** direct pill-to-pill edges, one per
  pair of endpoints, dashed in the target machine's hue, labelled
  "Event › Controller" (merged links list each once). Pills in different
  groups connect South → North (downward) or North → South (upward); pills
  in the same group East → West. This keeps lanes free of event and
  controller nodes and lets the layout route links between lanes. Events
  that nothing handles and external sources are not drawn here (the causal
  view shows them).
- **Hidden machines:** a stub in a thin group of its own (no lane), with
  the links attached to it as `StubLink`s.
- Hide mode drops lanes whose nodes are all hidden. A selected machine or
  compound state outlines its lane at the selected width. Reversed edges
  are never marked red here: the graph includes ordinary state cycles.

### Edit mode (`views/structure/wiring.rs`, `gutters/`, `selector.rs`, `edit.rs`)

`SceneMode::Edit` changes only the structure view: pills get second South
(4) and North (5) ports for emits, and the cross-lane links are replaced
by the wiring (event tags, controller hexagons, source boxes) in
*gutters*: thin untitled layout groups, one above each machine's groups
and one below the last, drawn as quiet neutral lanes (pale fill, dashed
rule-colored outline). Each wiring node goes into the gutter next to what
it wires, in a row ordered by the pills it wires; pill ends face their
gutter. Edges are the real emit, subscribe, fire and trigger edges; fire
labels are short (`by orderId`, `new with orderId`, `[when]`) and said
once. Every state, pill, controller and source gets a connect handle.
Details, including the placement rules, column memory, empty-machine hint
and empty-definition note, are in build-and-play.md ("Build and play
drawing") and readability.md ("Placement"). The layout cache is per view,
so switching modes feeds one mode's layout to the other as the previous
layout (lanes open up for the gutters, and states keep their place within
their lane) and switching back hits the cache.

### Play overlay (`views/overlays/`)

Drawn after decoration on the causal and structure views: instance
marker chips (on states in the structure view; on pills leaving the
current state in the causal view, hollow on the pills entering a dead
end), active items (selected width plus a translucent glow ring in their
own color), pending items (dotted outline, queue-position chips with the
head `1` filled). It never enters the layout input, so it never
relayouts. See build-and-play.md for the rules.

### Trace view (`views/trace.rs`)

No layered layout. Each trace is a block titled with its `ordering` label
(or the scenario name); a race's two orderings sit side by side. Each
lifeline gets a column as wide as its header and the boxes on it; columns
then move apart until every message label fits between its two lifelines.
Headers: instances in their machine's full hue, sources as neutral
rectangles, controllers as hexagons; spawned instances' headers sit at the
creating row. Rows run down in step order:

| Step | Drawn as |
| --- | --- |
| `ExternalFire` | message source → instance, trigger name, solid neutral |
| `Transition` | `from → to` box on the instance, pale machine fill |
| `Emit` | event tag on the instance |
| `Deliver` | message from the emitting instance (via the step's cause) to the controller, event name, dashed gray |
| `Fire` | message controller → instance, trigger name, dashed in the target's hue |
| `Spawn` | "new s2" message to the created header |
| `Dropped` | red-outlined "✕ trigger dropped in state" note |
| `NoTarget` / `Ambiguous` | dashed neutral note on the controller |

Final states sit at the bottom of each instance's lifeline. Lifelines are
dashed `Overlay::Line`s under everything. Steps are `HitTarget::TraceStep`,
headers and final states `HitTarget::Lifeline`. With no traces the scene is
empty with the note "Pick a scenario to trace.".

### Matrix view (`views/matrix.rs`)

Cells count derived links `T1 → event → handler → T2`
(`links::causal_links`, identical to `transition_successors` and tested
against it) from a transition of the row machine to one of the column
machine. Rows and columns share one order from `seriate`: coupling is
symmetrised (`w[i][j] = c[i][j] + c[j][i]`, diagonal ignored); start from
the most coupled machine, then repeatedly take, among machines coupled to
those placed, the one most coupled to the last placed (ties: to all
placed, then total coupling); when none is coupled, start a new cluster
from the most coupled remaining machine; remaining ties go to definition
order. Headers carry hue chips (`Overlay::Rect` targeting the machine);
rows show "n Name", columns "n". Cells are shaded neutral gray by count
(background mixed toward a gray of the text's lightness), with the count
as text; empty cells stay blank. `HitTarget::MatrixCell` on every cell.
The machine pair's cell and the headers of the selection's machines get
the selected outline; cells outside a cone dim. Hidden machines leave the
matrix.

### Caching and stability (`views/cache.rs`)

`SceneBuilder` keeps a `LayoutCache`: per view, the four most recent
layouts keyed by the full layout input (the `LayoutGraph` itself, the
options and the pins, compared with `==`, so there are no hash
collisions). The graph encodes every structural input (model, hidden
machines, collapse, machine pair, the visible set in hide mode), while
emphasis never changes it, so selection, cone, search, badges and diff can
never relayout (`SceneBuilder::layouts_run` counts actual layouts). On a
miss, the view's most recent layout is passed as `LayoutHints::previous`
(`LayoutResult::to_previous`), and pins come from
`sidecar.pins_for(view)` keyed by element-key strings (filtered to nodes in
the graph). Toggling a filter back returns exactly the earlier picture.
Only a layout made with the same options seeds a miss, so the causal
view's lanes and its flat layout never seed each other.
Layout node keys are element-key strings; a hidden machine's stub uses the
machine's key, so it can be pinned too.

### Export (`export/`)

`to_svg` validates that all geometry is finite (`ExportError::NonFinite`),
then writes a standalone SVG: background rect, lanes, under-overlays,
edges, nodes, over-overlays, each item a `<g class="lane|overlay|edge|node">`
carrying its opacity. Shapes: `Rect`/`RoundedRect`/`Pill`/`Stub` as rects
with the right radius, `Tag` and `Hexagon` as paths. `Border::Double` adds
an inner outline, `Border::ThickLeft` a bar clipped to the shape. Dashes
map to `stroke-dasharray`. Connect handles (`HitTarget::ConnectHandle`
overlays) are left out; every other overlay, including play chips and
glow rings, is written. Arrowheads are markers, one per stroke color.
Labels use a monospace font family with the baseline 0.95 em below the
origin (the line box is 1.3 em), text escaped; edge labels get a halo in
the background color. Badges are red-outlined circles with the count,
filled for errors, dashed for info. `to_png` rasterises that SVG with
resvg 0.46 (system fonts loaded once, the monospace family set to an
installed monospaced face) at `scale` pixels per unit and encodes PNG;
non-positive scales and images over 16384 px a side or 100 Mpx are errors.

### CLI (`cascade render`)

`cascade render <file> --view <v> [--state <link>] [--scenario <file>]
[--dark] [--scale s] --out x.svg|png`: loads the definition (diagnostics
exit 2), applies the link (its view is overridden by `--view`), runs the
checks for badges, loads the pins sidecar, simulates the scenario (or, with
`race=N` in the link, replays the N-th race candidate in both orders),
builds the scene, prints its notes to stderr and writes SVG or PNG by
extension. `--view causal --state 'cascade://causal?lanes=1'` renders the
causal lanes.

With `diff=<base>,<head>` in the link, `render` reads the definition at
`base` (and at `head`, or the working tree when `head` is empty) with
`cascade_interop::read_at_rev`, merges them with
`cascade_core::diff::merge_for_display`, and builds the scene from the
merged model with `SceneInput::diff` set: added elements get green outlines
and removed ones become red ghosts. Findings are not computed in diff mode,
because the merged model's ghosts would distort them. An unreadable
revision or an invalid version exits with code 2.

## Visual encoding as implemented

| Element | Shape | Color and line |
| --- | --- | --- |
| Machine | Lane (structure), header chip (matrix), header box (trace) | Okabe-Ito hue; declared colors win; undeclared take the least-used hue; past 8 machines grouped by `domain`, sharing the domain's hue; machines sharing a hue get successive lightness steps |
| State | `RoundedRect` | Pale machine fill, hue outline; initial (machine or compound) `ThickLeft`, final `Double`, history a circled `H` / `H*` |
| Transition | `Pill`, `before → after` over the trigger | Full hue fill, outline a darker (lighter in dark mode) hue so weight stays visible |
| Event | `Tag` | Neutral gray fill and outline |
| Controller | `Hexagon` (one per handler) | Dark neutral outline, no fill |
| External source | `Rect` | Neutral outline, no fill |
| Hidden machine | `Stub`, "Name, n links" | Pale fill, dashed hue outline |
| Transition within a machine | Solid arrow | Machine hue |
| Emit | Dashed arrow | Gray |
| Subscribe | Solid arrow | Gray |
| Fire | Dashed arrow | Target machine's hue |
| Trigger (source → transition) | Solid arrow | External neutral |
| Stub link | Dotted arrow | The original link's color |
| Guard | `[guard]` label on the arrow into the transition | Default text color |
| Cascade-cycle back edge | `back_edge = true` | `theme.finding` red |
| Finding | `Badge` (count, highest severity) | Red outline on every subject |
| Diff added / removed | — | Green outline / red dashed ghost at 45% opacity |
| Selected / focused / dimmed / search | — | Selected width / middle width / 15% opacity / dotted halo; never a hue change |
| Matrix cell | `Rect` with count | Neutral gray by count |
| Controller (edit mode) | `Hexagon` per controller, name in bold over "on Event" lines | Dark neutral outline, no fill |
| Gutter (edit mode) | Untitled lane between machine lanes holding wiring | Neutral pale fill, dashed rule-colored outline |
| Fire label (edit mode) | Short selector then `[when]`: `by orderId`, `all by f`, `new with f`; none for a singleton | Default text color; once per parallel fires |
| Connect handle (edit mode) | Circle on the east edge | Background fill, muted outline; not exported |
| Instance marker | Chip on the top edge, `o1` | Machine hue fill, on-hue bold text; hollow (hue outline) on a dead end's way in (causal) |
| Active (play) | — | Selected width; nodes also a glow ring in their own outline color at 35% alpha |
| Pending (play) | Queue-position chip | Dotted outline; head chip filled with the text color, others hollow |

## Files

| File | Role | Key exports |
| --- | --- | --- |
| `crates/cascade-scene/src/lib.rs` | Crate root and re-exports | — |
| `crates/cascade-scene/src/scene.rs` | The display-list contract (unchanged) | `Scene`, `SceneNode`, `SceneEdge`, `Lane`, `Overlay`, `HitTarget`, `Shape`, `Border`, `Stroke`, `Badge`, `Emphasis` |
| `crates/cascade-scene/src/color.rs` | Palette, themes, machine hues with domain grouping | `machine_styles`, `machine_colors`, `style_for`, `Theme`, `Rgba` |
| `crates/cascade-scene/src/emphasis/mod.rs` | Interaction model | `Interaction`, `Anchor`, `FocusRegion`, `FocusKind` |
| `crates/cascade-scene/src/emphasis/seeds.rs` | Element → causal nodes | `causal_seeds` |
| `crates/cascade-scene/src/views/mod.rs` | Builder and dispatch | `SceneBuilder` (`build`, `reset`, `layouts_run`), `SceneInput`, `SceneError` |
| `crates/cascade-scene/src/views/cache.rs` | Layout memoisation and previous-layout hints | `LayoutCache` (crate) |
| `crates/cascade-scene/src/views/draft.rs` | Draft graph, hide cuts, realize through layout | `DraftGraph`, `Meta`, `realize` (crate) |
| `crates/cascade-scene/src/views/decorate.rs` | Emphasis, badges, diff, bounds | `Decor`, `FindingIndex`, `scene_bounds` (crate) |
| `crates/cascade-scene/src/views/style.rs` | Node looks and link strokes per element kind | `Painter` (crate) |
| `crates/cascade-scene/src/views/filters.rs` | Hidden machines, machine pair, hide mode | crate-private |
| `crates/cascade-scene/src/views/links.rs` | Derived transition links, cycle edges | `causal_links`, `cycle_edges` (crate) |
| `crates/cascade-scene/src/views/causal/mod.rs` | Causal flow view | crate-private |
| `crates/cascade-scene/src/views/causal/lanes.rs` | Causal lanes: lane rules, lane groups and drawing | `LaneRules`, `LaneOf`, `LaneGroups` (crate) |
| `crates/cascade-scene/src/views/structure/mod.rs` | Structure view orchestration and lanes | `machine_lane` (crate) |
| `crates/cascade-scene/src/views/structure/machines.rs` | Per-machine drafting, nesting, collapse | crate-private |
| `crates/cascade-scene/src/views/structure/links.rs` | Cross-lane links, stub counts | crate-private |
| `crates/cascade-scene/src/views/structure/wiring.rs` | Edit mode's wiring: nodes, merged edges, gutter placement glue, ports facing the gutter | crate-private |
| `crates/cascade-scene/src/views/structure/gutters/mod.rs` | Which gutter each wiring node goes to | `Stack`, `Gutter`, `Wires`, `assign` (crate) |
| `crates/cascade-scene/src/views/structure/gutters/order.rs` | Row order inside a gutter | `order`, `WiringNode` (crate) |
| `crates/cascade-scene/src/views/structure/gutters/memo.rs` | Gutter columns kept across edits | `WiringMemo` (crate, owned by `SceneBuilder`) |
| `crates/cascade-scene/src/views/structure/selector.rs` | Fire label policy (short selectors, said once) | crate-private |
| `crates/cascade-scene/src/views/structure/edit.rs` | Gutter lanes, empty hints | crate-private |
| `crates/cascade-scene/src/views/overlays/*.rs` | Connect handles, markers, active/pending, chips | `PlayDecor`, `Placement`, `add_handles` (crate) |
| `crates/cascade-scene/src/play.rs` | Build/play inputs (contract) | `SceneMode`, `PlayOverlay`, `PlayMarker` |
| `crates/cascade-scene/src/views/trace.rs` | Trace view | crate-private |
| `crates/cascade-scene/src/views/matrix.rs` | Matrix view and seriation | `seriate` (crate) |
| `crates/cascade-scene/src/export/mod.rs` | Export entry points and errors | `to_svg`, `to_png`, `ExportError` |
| `crates/cascade-scene/src/export/svg.rs` | SVG writer | crate-private |
| `crates/cascade-scene/src/export/png.rs` | resvg rasteriser and font loading | crate-private |
| `crates/cascade-scene/src/metrics.rs` | Readability measures | `measure`, `SceneMetrics`, `leftward_edges` |
| `crates/cascade-scene/tests/*.rs` | Per-view, export and performance tests (`causal_lanes.rs`, `view_link_lanes.rs` for causal lanes) | — |
| `crates/cascade-cli/src/commands/render.rs` | `cascade render` | `run`, `RenderArgs` |
| `crates/cascade-cli/tests/render.rs` | CLI render tests | — |
| `crates/cascade-cli/tests/render_lanes.rs` | `render --state 'cascade://causal?lanes=1'` | — |

## Invariants and constraints

- Hue means entity: only machine-owned things take a hue (instance marker
  chips included); emphasis, active and pending change outline weight,
  dash and opacity only. The exceptions are the finding red
  (badged outlines, cycle back edges) and diff outlines, which never fill.
- Emphasis never relayouts: the layout cache key is the layout input, and
  decoration runs after layout. Tested by comparing node rects and
  `layouts_run`.
- Builds are deterministic: the same inputs give the same scene (graph
  order, definition order, deterministic seriation and tie-breaks).
- Selected items are never hidden or dimmed.
- Every `SceneEdge` has at least two points; self-links are drawn as small
  loops rather than passed to the layout. A self-link's label goes beside
  the loop's outer corner, else above the loop, else left of it, whichever
  first clears every node.
- Scene bounds cover every node, badge, edge point, label, lane and
  overlay.
- Non-test code never panics on user input: unknown keys, names and
  malformed traces are ignored or become notes; export rejects non-finite
  geometry and oversized images with typed errors.
- Performance: at about 20 machines, 200 states and 50 controllers each
  view builds well under a second in a debug build (tested, including the
  current layout engine), edit mode included; a play overlay change costs
  no layout. Pinning a wiring node with many lane-crossing edges (a busy
  controller) goes through the engine's obstacle router: 14–18 ms release
  for the pinned shop build canvas, scene included.
- Play overlays never relayout, and `SceneMode::View` scenes are
  unchanged by build and play drawing (fingerprinted in
  `tests/view_mode_golden.rs`; re-blessed only for intended view-mode
  changes, last for the self-link label placement).
- `Scene::hit_test` checks connect handles before nodes; nothing else
  in its order changed.
- `Scene` and its item types are unchanged; additions are
  `SceneBuilder::layouts_run`, `cascade_scene::emphasis`,
  `machine_colors`, and a reworked `ExportError` (no `NotImplemented`).
