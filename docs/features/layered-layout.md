# Layered layout

`cascade_layout::layout(&LayoutGraph, &LayoutOptions, &LayoutHints) ->
Result<LayoutResult, LayoutError>` places sized boxes in layers, routes the
edges between them and draws groups as stacked lanes. It stands in for
ELK's layered algorithm with orthogonal routing, since there is no ELK for
native Rust. The crate is generic: it knows nothing about machines,
transitions or events.

## Scope

- Cycle breaking with reversed edges flagged (`EdgeRoute::reversed`).
- Layering that honours `LayerConstraint::{First, Exact, Free}`, compacted
  by network simplex.
- Crossing minimisation, port-aware.
- Coordinates that straighten long edges and line ports up.
- Orthogonal routing through channels with track assignment, and polyline
  routing through dummy points.
- Ports on any side, spread evenly. Unported edges attach on the
  flow-facing sides.
- Edge labels: room reserved in the layout and boxes returned.
- Groups (lanes) stacked top to bottom. Edges between them run through
  the gaps between groups and straight across the groups in between
  through free passages, with side corridors only as a last resort.
- Optional alignment of groups with each other
  (`LayoutOptions::align_across_groups`), so edges between stacked groups
  run straight.
- Label boxes clear of nodes, group headers and each other, with a count
  of any that could not be placed cleanly.
- `LeftToRight` and `TopToBottom` flow.
- Stability from the previous layout, and pins.
- Typed errors for contradictory constraints and invalid input.
- Public measurement helpers (`cascade_layout::metrics`), including the
  readability measures of a layout.

## Non-scope

- What the boxes mean, their styling and text measurement (see
  view-scenes.md).
- Nested groups: a group contains nodes, not other groups.
- Global optimality: every stage is a heuristic except layering, which is
  optimal for its objective.

## Public interface

| Item | Notes |
| --- | --- |
| `layout(graph, options, hints)` | The entry point. Deterministic: the same input gives the same output, bit for bit. |
| `LayoutError` | `Unsatisfiable { key, reason }`, `InvalidNodeSize { key }`, `InvalidEdgeLabel { edge }`, `InvalidGroup { key }`, `InvalidOption { name }`, `InvalidPin { key }`. |
| `metrics::count_crossings` | Proper crossings between the routes of different edges. |
| `metrics::count_order_crossings` | Crossings between neighbouring-layer edges, judged from `layer`/`order` alone. |
| `metrics::overlapping_nodes`, `metrics::segments_cross` | Test and diagnostic helpers. |
| `metrics::measure` → `RouteMetrics` | Crossings, corridor edges, total length, bends, label overlaps and unplaced labels of one layout. The single measures (`total_length`, `bends`, `corridor_edges`, `label_overlaps`, `group_headers`) are public too. |
| `LayoutOptions::align_across_groups` | Off by default. See step 6. |
| `LayoutResult::unplaced_labels` | Labels that could not be placed clear of every node, header and label (additive field; `with_unplaced_labels` sets it). |

The input and output types (`LayoutGraph`, `LayoutOptions`, `LayoutHints`,
`LayoutResult`) are the foundation's contracts; they only grew additively.

## Data and control flow

Everything runs in a canonical left-to-right frame: layers along `x`, and
order within a layer along `y`. `TopToBottom` transposes the input on the
way in and the output on the way out (`frame.rs`), which maps top-left
corners to top-left corners. The frame also maps port sides, and keeps a
group's header on the real top.

1. **Problem** (`problem.rs`). Validates sizes, spacings, group insets,
   labels and pins, and returns a typed error for any negative or
   non-finite value. It transposes everything into the canonical frame and
   resolves each edge end to a side. A port uses its own side. An unported
   end leaves East and enters West; both ends of an unported self-loop sit
   East. Nodes are split into bands: band 0 holds ungrouped nodes and band
   `g + 1` holds group `g`. Each edge gets an `EdgeKind`:
   - `Chain`: same band, not pinned.
   - `CrossBand`: different bands.
   - `SelfLoop`.
   - `Pinned`: touches a pinned node.

   The mode comes from `hints.previous`:
   - `Fresh`: no previous layout.
   - `Seeded`: fewer than half the nodes match; the previous layout only
     seeds the ordering.
   - `Stable`: half or more match. Every free node with a previous
     placement gets a soft layer fix at its previous layer.
2. **Layering, per band** (`layering.rs`, `cycles.rs`,
   `network_simplex.rs`). Tarjan's algorithm finds which edges lie on
   cycles. An acyclic edge between two *hard*-fixed nodes that does not
   point forward returns `Unsatisfiable`. The Eades–Lin–Smyth greedy
   arrangement (per component, with components in topological order)
   orients every edge that has a free end. A relaxation pass in
   arrangement order computes earliest layers; a desired edge into a fixed
   node that cannot point forward is dropped rather than failing, so free
   nodes are relaxed. Network simplex then minimises total edge length:
   - Fixed nodes are merged into one super-node per fixed layer, tied to a
     root by heavy edges.
   - Labelled edges get a minimum length of 2 so the label gets a layer.
   - Fixed values are compressed first (gaps capped at what free nodes
     could use) and mapped back afterwards, so `Exact(4e9)` costs nothing.
3. **Band graph** (`layered.rs`). Columns are the distinct layers of the
   band's non-pinned nodes, plus the middle layer of long labelled edges.
   Channel `c` lies left of column `c`. Each end of a chain edge picks a
   channel from its side (East: right of the node; West: left;
   North/South: toward the other end). The edge then gets one dummy in
   every column between the two end channels. That one rule covers forward
   edges, reversed edges (they leave East, double back through their own
   column and enter West), flat edges and ports facing any way. The middle
   dummy of a labelled chain is a label item as large as the label.
4. **Slots** (`slots.rs`). A side's slots are its explicit ports in index
   order, then one slot per unported end. Slots are spread evenly along the
   side, and edges on one port share its slot. North/South ends run
   vertically to a stub *level* (a multiple of the edge spacing) before
   turning toward their channel. Levels nest so stubs heading the same way
   don't cross, and self-loops between different sides get their own
   levels. Levels and self-loop labels become node margins.
5. **Order and cross-axis position.**
   - Fresh bands (`ordering.rs`, `coordinates.rs`):
     - Barycentre sweeps alternate direction. A neighbour's position
       counts at its port's offset along its side, so ports keep their
       order.
     - A transpose pass follows each sweep. The best ordering by exact
       crossing count (accumulator tree) is kept.
     - After that, a priority-style L1 placement. Links pull their ends
       into line with weights dummy–dummy 8, node–dummy 2, node–node 1,
       and a slight bias to predecessors. Each column is solved exactly by
       pool-adjacent-violators with weighted medians (`packing.rs`),
       sweeping until stable.
     - A straightening pass then moves each dummy chain, whole, onto its
       source's or target's line where every column has room.
     - A greedy pass straightens any remaining bent link without bending
       another.
   - Bands with previous positions (`stability.rs`). A node in the same
     layer as before, and near its column, is *anchored*. Each such band
     is first laid out from scratch (a trial of the fresh pipeline).
     - **Reproduced.** If the trial puts every node at one translation of
       its previous position, the band did not change. The trial is moved
       back into place: nodes onto their exact previous floats, and
       dummies, labels and routes by the same translation, so the band and
       its rect come back as before.
     - **Inserted.** Otherwise the band changed. Anchored nodes keep their
       previous top-left exactly, and only move apart where they would
       really collide: their stub margins plus half an edge spacing, and
       at least half the node spacing between them. New nodes go into the
       nearest free space to the average height of their placed
       neighbours. A new node with no placed neighbour (and no previous
       position) takes its column's first free slot from the band's top:
       the top of an empty column, or a hole between placed nodes, before
       the space below them. So it never opens a new row below the band
       when its column has room. Each chain's dummies take one line clear
       through all their columns near the source's (or target's) line, or
       else the nearest free spot per column. The order within a column
       follows position.
   - Between the two phases, unported slots are sorted by where their
     edges head: by neighbour order in fresh and reproduced bands, and by
     neighbour height in inserted ones.
6. **Main axis** (`coordinates.rs`, `stability.rs`, `context.rs`).
   Provisional channel tracks size the channels. An inner channel is at
   least `layer_spacing` wide and holds `(tracks + 1) × edge_spacing`.
   Columns are as wide as their widest item; nodes, dummies and labels are
   centred. Reproduced bands are placed the fresh way and moved back:
   by one translation, or by one per column when the previous layout
   aligned the columns separately (every column with nodes agrees on one
   translation, node-less columns move with the column before them, and
   no channel ends up narrower). Otherwise they fall back to the
   inserted rule. In inserted bands, anchored nodes keep their `x`, new
   columns sit next to their neighbours, and a column moves right only as
   far as its channel needs.
   - **Alignment** (`align.rs`, with `align_across_groups`). Each fresh
     band shifts its columns along the main axis toward the other ends of
     its cross-band edges. Per band this is an exact L1 compaction
     (`place_l1`):
     - units are columns; a column without nodes (dummies and labels
       only) is welded to the column before it;
     - units keep their order and current distances as minimums, and the
       first stays at or right of its fresh position, so columns only
       spread to the right;
     - every cross-band end pulls its unit so its leg lines up with the
       other end's leg (the attachment point of a direct leg, the
       channel's middle otherwise), weight 1, with a feeble pull to stay.

     Bands are solved in stacking order, down and back up, until nothing
     moves more than half a unit (at most 8 rounds). Kept bands and pins
     never move, but new bands still align with them.
7. **Stacking and pins** (`bands.rs`). Bands stack top to bottom: the
   ungrouped band first (when it has nodes), then groups in insertion
   order.
   - A fresh band sits right after a gap of `group_spacing` or, if that
     is taller, room for its cross-band tracks plus one spare, so a later
     edit adding a track usually fits.
   - A band with previous positions stays put. It moves down only when
     the gap above would drop below half `group_spacing`, or its tracks
     no longer fit even closed up to half the edge spacing. So a change
     inside one band, or a new track in a gap, does not move other bands.
   - Each band's items are pushed off pinned nodes (per column, keeping
     order and gaps, up if that is closer and clear, otherwise down)
     before the band is measured, so everything below makes room.
   - Group rects cover content, pinned members, padding and header. All
     groups share one width.
8. **Routing** (`routing/`).
   - Chains: each channel's vertical segments get tracks (`tracks.rs`).
     For every overlapping pair, the order causing fewer crossings is
     preferred, cycles are broken greedily, and a segment's track is its
     longest-path depth. Segments on one port share a track. Tracks are
     spread evenly across the channel.
   - Cross-band edges (`routing/cross/`):
     - **Legs.** An end on a North or South side facing the other band
       leaves straight out of its port (a *direct* leg) when no other node
       or label of its column lies between it and the band's boundary.
       Any other end runs from its stub to a channel track and along it to
       the boundary (a *channel* leg). Direct legs take no channel track.
     - **Adjacent bands.** One run in the gap between them. When the legs
       line up the whole route is one vertical line; otherwise it has one
       jog. Adjacent bands never use a corridor.
     - **Bands in between.** Each band's *passages* (`passages.rs`) are
       the stretches of the lanes' shared width clear of every column
       extent, channel track and pinned node, each widened by the edge
       spacing. The strip left of the first column is never a passage;
       the lane's title sits there. The planner (`planner.rs`) runs a
       small dynamic program over the bands in between. For each band it
       picks a passage with room left, or the left or right corridor. It
       minimises horizontal travel, plus 24 per jog, plus 300 per in-band
       link a passage crosses, plus 10⁶ per band passed in a corridor. So
       a corridor is used only when a band has no passage with room, and
       then on the shorter side. A vertical lines up with the previous one
       wherever its passage allows. Routes are planned with the fewest
       bands in between first (then by edge), each taking one place in
       every passage it uses.
     - **Resolution** (`resolve.rs`). Verticals sharing a passage are
       ordered by the midpoint of where they come from and go to, then
       placed exactly (L1) as near their planned line as the edge spacing
       allows. Lined-up verticals weigh more. Every gap's runs and every
       corridor's verticals get tracks from `tracks.rs`. For each
       overlapping pair that picks the order with fewer crossings, judged
       from where the verticals join. Corridor tracks therefore nest by
       span.
   - Self-loops (`paths.rs`): East/East and West/West loops are a small C
     on a channel track. Other side pairs go around the node's corners on
     their levels. A North/South loop goes around the nearer of the East
     and West sides. A loop from a port back into itself is a small lasso.
   - Pinned edges: the obstacle router (`astar.rs`). This is weighted A*
     over a sparse grid of obstacle-side lines (with midlines on small
     grids), with bend penalties. Crossing an obstacle is allowed but
     costs a penalty per unit of length, and overlapping obstacles add
     up, so a route always exists:
     - a node: 16 000;
     - running along inside a foreign group: 1 000;
     - crossing a foreign group straight along the stacking axis, the way
       a passage does: 1.

     Cheap straight crossings keep the search from flooding the grid
     before a long detour. Step penalties come from prefix sums over the
     grid, so each step costs O(1). Search buffers are reused across
     searches (reset by a stamp).
   - Repair: any other route that crosses a node, or enters a foreign
     group other than straight across it, is rerouted by the same router.
     This can only happen around pins.
9. **Labels and output** (`labels.rs`, `mod.rs`).
   - Label boxes first use the reserved room: the label item's column, or
     above the node for self-loops. They keep it if it is clear.
   - Every other label (cross-band, pinned, rerouted, or a reserved box
     that collides) is placed in edge order. It goes beside one of its
     segments: longer segments first, starting mid-segment and sliding
     toward the ends in steps, on either side.
   - The first candidate wins if it keeps 0.5 clear of every node, group
     header strip (padding and header on the real top) and placed label,
     and covers no other edge's route. Failing that, the first clear of
     the hard obstacles wins. Failing that, the one overlapping least
     wins, and it is counted in `unplaced_labels`. Route segments are
     indexed in a grid for these queries.
   - `reversed` = the edge lies on a cycle within one band and does not
     point forward.
   - `order` ranks nodes within (band, layer) by position.
   - Everything is transposed back, and bounds cover nodes, groups, routes
     and labels.

## Files

| File | Role | Key items |
| --- | --- | --- |
| `crates/cascade-layout/src/engine/mod.rs` | Pipeline, errors, output assembly | `layout`, `LayoutError` |
| `engine/problem.rs` | Validation, canonical input, bands, modes, edge kinds | `Problem`, `Fix`, `Mode`, `EdgeKind`, `Spacing` |
| `engine/frame.rs` | Canonical frame and sides | `Frame`, `Side` |
| `engine/cycles.rs` | SCCs and greedy arrangement | `strongly_connected`, `greedy_arrangement` |
| `engine/network_simplex.rs` | Layering LP solver | `solve`, `NsEdge` |
| `engine/layering.rs` | Constrained layer assignment per band | `assign`, `LayerEdge`, `Layering` |
| `engine/layered.rs` | Columns, items, dummies, chains | `BandGraph`, `Item`, `ItemKind`, `Chain`, `end_channel` |
| `engine/slots.rs` | Port and slot positions, stub levels, margins | `Slots`, `attach_point` |
| `engine/ordering.rs` | Crossing minimisation | `minimize`, `count_inversions` |
| `engine/packing.rs` | 1-D placement: L1 PAV, free space, push-off | `place_l1`, `Occupancy` (`nearest_free`, `first_free`), `push_off` |
| `engine/coordinates.rs` | Fresh-mode coordinates, straightening, channel widths | `assign_y`, `assign_x`, `channel_width` |
| `engine/stability.rs` | Placement with previous positions: reproducing unchanged bands, fitting changes into changed ones | `anchors`, `reproduce_y`, `reproduce_x`, `place_nodes`, `place_dummies`, `place_x` |
| `engine/context.rs` | Shared read-only views | `Ctx`, `Columns` |
| `engine/align.rs` | Cross-group alignment of fresh bands | `shifts` |
| `engine/bands.rs` | Stacking, pins, group rects | `stack`, `Placement` |
| `engine/tracks.rs` | Track assignment | `assign`, `TrackSeg`, `Toward` |
| `engine/routing/mod.rs` | Routing driver and repair | `route_all`, `Routes` |
| `engine/routing/channels.rs` | Channel segments and track positions, leg kinds | `collect`, `SegKey`, `Leg`, `leg`, `leg_offset` |
| `engine/routing/cross/mod.rs` | Cross-band route types | `CrossPlan`, `Via`, `Corridor`, `Stack` |
| `engine/routing/cross/passages.rs` | Free passages through a band | `Passage`, `of_band` |
| `engine/routing/cross/planner.rs` | Choosing passages or corridors per band | `Request`, `plan_all` |
| `engine/routing/cross/resolve.rs` | Passage, gap and corridor tracks → coordinates | `resolve`, `gap_track_counts`, `CrossGeometry` |
| `engine/routing/paths.rs` | Polylines for chains, loops, cross-band edges | `chain_orthogonal`, `chain_polyline`, `self_loop`, `simplify` |
| `engine/routing/astar.rs` | Obstacle router | `Router`, `Obstacle`, `Ends` |
| `engine/routing/check.rs` | Route validation, rect index | `is_clear`, `passes_across`, `RectIndex` |
| `engine/labels.rs` | Collision-free label boxes | `boxes` |
| `src/metrics.rs` | Public measurements | `measure`, `RouteMetrics`, `count_crossings`, `corridor_edges`, `label_overlaps`, `count_order_crossings`, `overlapping_nodes` |
| `tests/*.rs` | Behaviour: basics, cycles, constraints, ordering, ports, labels, groups, stability, pins, determinism, direction, routing, readability, fuzzed invariants, performance, pinned-hub performance | `tests/common` holds the invariant checker; `tests/canvas` builds build-canvas-shaped benchmark graphs (lanes with gutters, or one wiring band) and draws them as SVG |

## Invariants and constraints

- **Determinism.** Indices are processed in order, ties break by index,
  floats compare with `total_cmp`, and hash maps are only used for
  lookups.
- **No overlaps.** Node rects never overlap. The one exception: pinned
  nodes may overlap each other.
- **Layers.** Forward edges point left to right (top to bottom). An edge
  is `reversed` exactly when it lies on a cycle within its band and does
  not point forward, so every cycle shows at least one reversed edge and
  no acyclic edge is flagged. Self-loops and cross-band edges are never
  reversed.
- **Constraints.** `First` and `Exact` layers are always honoured.
  `Unsatisfiable` means an acyclic edge joins two hard-fixed nodes and
  points backwards or sideways. When a free node cannot fit between its
  fixed neighbours, its edge runs against the flow, unflagged, instead of
  failing. A view that passes BFS depth as `Exact` for every node will hit
  `Unsatisfiable` on acyclic edges that point to a shallower node. Such a
  view should use `First` for sources and leave the rest `Free`.
- **Routes.** Every route starts exactly on its source attachment point
  and ends exactly on its target attachment point, leaving and entering
  perpendicular to the side. Orthogonal routes are axis-aligned.
  Overlapping vertical segments in a channel are at least `edge_spacing`
  apart while the channel has room, and so are verticals sharing a
  passage. No route crosses a node's interior. A route enters a foreign
  group only straight across it along the stacking axis, from one side to
  the other, through a passage. Pins are the exception where they make
  this unavoidable (a pinned node overlapping an end, or a group dragged
  over another band). Edges between adjacent groups never use a corridor.
- **Labels.** Label boxes do not overlap nodes, group header strips or
  each other. Where that is impossible, the result counts the label in
  `unplaced_labels`.
- **Groups.** Group rects contain their nodes plus padding and header, and
  stack in insertion order. A fresh layout keeps at least `group_spacing`
  between them. A relayout lets a gap close to half of that before a
  group moves. Groups never overlap unless a pin drags a member over
  another band; pins win.
- **Stability.** With `hints.previous` covering at least half the nodes,
  a node keeps its previous rect exactly (bit for bit) if both hold:
  - its layer is unchanged (true unless a hard constraint changed);
  - it isn't forced to move.

  A group or band whose contents did not change is reproduced exactly:
  every node, dummy and route inside it, and its rect. A node is forced to
  move only when:
  - a node above it in its column grew into it, beyond the relaxed
    clearance (new nodes never push: they go into free space, and new
    stubs don't push a neighbour they clear);
  - a new or widened column leaves its channel too narrow, which pushes
    the column right;
  - the band above grows so far that the gap between them would drop
    below half `group_spacing`, or that gap's tracks no longer fit at
    half the edge spacing;
  - a newly dragged pin lands on it.

  Adding or removing a node or edge therefore leaves unrelated nodes
  where they were. That includes a transition added inside one lane of
  the structure view: every other lane keeps every node
  (`tests/stability.rs`). In the shop example, even the edited lane's
  existing nodes stay put. With `align_across_groups`, an unchanged
  layout comes back exactly (nodes, groups and routes), and an edit
  inside one lane still moves no node in another (`tests/stability.rs`). New nodes keep previous layers fixed, so a
  new node between two adjacent layers shares a layer instead of shifting
  everything after it. Existing nodes keep their relative order within a
  layer. `SceneBuilder::reset` (a fresh layout) tidies up after many
  edits.
- **Pins.** A pinned node's top-left is exactly its pin. Other nodes (and
  dummies) in columns under the pin are pushed off it by the node or edge
  separation. Edges to pinned nodes attach to their ports and are routed
  around obstacles. Pinned nodes still get a `layer` and an `order`.
- **Spacing floors.** Edge spacing below 2 and layer spacing below twice
  the edge spacing are raised internally, so routes can leave their nodes.
- **Performance.** 500 nodes and 800 edges lay out in about 25 ms fresh
  and 21 ms stable without groups, and about 9 ms either way with 8
  groups, in a release build (`tests/performance.rs`; unchanged by the
  routing work within noise). Pinning the busiest controller of a
  96-node build canvas anywhere costs 2–4 ms, down from up to 170 ms
  (`tests/pins_performance.rs`).
