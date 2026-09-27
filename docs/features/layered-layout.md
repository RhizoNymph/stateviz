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
- Groups (lanes) stacked top to bottom, with edges between them routed
  around foreign groups.
- `LeftToRight` and `TopToBottom` flow.
- Stability from the previous layout, and pins.
- Typed errors for contradictory constraints and invalid input.
- Public measurement helpers (`cascade_layout::metrics`).

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

The input and output types (`LayoutGraph`, `LayoutOptions`, `LayoutHints`,
`LayoutResult`) are the foundation's contracts and are unchanged.

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
   - Stable bands (`stability.rs`). A node in the same layer as before,
     and near its column, is *anchored* and keeps its previous top-left
     exactly. New nodes go into the nearest free space to the average
     height of their placed neighbours. Each chain's dummies take one line
     clear through all their columns near the source's (or target's)
     line, or else the nearest free spot per column. The order within a
     column follows position.
   - Between the two phases, unported slots are sorted by where their
     edges head: by neighbour order in fresh bands and by neighbour height
     in stable ones.
6. **Main axis** (`coordinates.rs`, `stability.rs`, `context.rs`).
   Provisional channel tracks size the channels. An inner channel is at
   least `layer_spacing` wide and holds `(tracks + 1) × edge_spacing`.
   Columns are as wide as their widest item; nodes, dummies and labels are
   centred. In stable bands, anchored nodes keep their `x`, new columns sit
   next to their neighbours, and a column moves right only as far as its
   channel needs.
7. **Stacking and pins** (`bands.rs`). Bands stack top to bottom: the
   ungrouped band first (when it has nodes), then groups in insertion
   order. Each gap is at least `group_spacing` and fits its cross-band
   tracks.
   - A fresh band sits right after the previous gap. A stable band stays
     put unless the band above now reaches into it, in which case it moves
     down.
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
   - Cross-band edges (`cross.rs`): leave the source band through its top
     or bottom boundary from a channel track, run in the gap next to it,
     and drop into the target band if it borders that gap. Otherwise they
     take the nearer side corridor to the gap bordering the target. Gap
     runs and corridor runs get tracks too.
   - Self-loops (`paths.rs`): East/East and West/West loops are a small C
     on a channel track. Other side pairs go around the node's corners on
     their levels; a loop from a port back into itself is a small lasso.
   - Pinned edges: the obstacle router (`astar.rs`). This is weighted A*
     over a sparse grid of obstacle-side lines, with bend penalties.
     Crossing an obstacle is allowed but costly (nodes cost 16× groups,
     and overlapping obstacles add up), so a route always exists.
   - Repair: any other route crossing a node or a foreign group's interior
     (possible only around pins) is rerouted by the same router.
9. **Labels and output** (`labels.rs`, `mod.rs`).
   - Label boxes use the reserved room: the label item's column, or above
     the node for self-loops. Otherwise a label goes beside a segment,
     preferring spots clear of nodes.
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
| `engine/packing.rs` | 1-D placement: L1 PAV, free space, push-off | `place_l1`, `Occupancy`, `push_off` |
| `engine/coordinates.rs` | Fresh-mode coordinates, straightening, channel widths | `assign_y`, `assign_x`, `channel_width` |
| `engine/stability.rs` | Stable-mode placement | `anchors`, `place_nodes`, `place_dummies`, `place_x` |
| `engine/context.rs` | Shared read-only views | `Ctx`, `Columns` |
| `engine/bands.rs` | Stacking, pins, group rects | `stack`, `Placement` |
| `engine/tracks.rs` | Track assignment | `assign`, `TrackSeg`, `Toward` |
| `engine/routing/mod.rs` | Routing driver and repair | `route_all`, `Routes` |
| `engine/routing/channels.rs` | Channel segments and track positions | `collect`, `SegKey` |
| `engine/routing/cross.rs` | Gaps and corridors | `plan`, `resolve`, `CrossPlan` |
| `engine/routing/paths.rs` | Polylines for chains, loops, cross-band edges | `chain_orthogonal`, `chain_polyline`, `self_loop`, `simplify` |
| `engine/routing/astar.rs` | Obstacle router | `route`, `Obstacle`, `Ends` |
| `engine/routing/check.rs` | Route validation, rect index | `is_clear`, `RectIndex` |
| `engine/labels.rs` | Label boxes | `boxes` |
| `src/metrics.rs` | Public measurements | `count_crossings`, `count_order_crossings`, `overlapping_nodes` |
| `tests/*.rs` | Behaviour: basics, cycles, constraints, ordering, ports, labels, groups, stability, pins, determinism, direction, routing, fuzzed invariants, performance | `tests/common` holds the invariant checker |

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
  apart while the channel has room. No route crosses a node's or a
  foreign group's interior, except where pins make that unavoidable (a
  pinned node overlapping an end, or a group dragged over another band).
- **Groups.** Group rects contain their nodes plus padding and header, and
  stack in insertion order with at least `group_spacing` between them.
  They never overlap unless a pin drags a member over another band; pins
  win.
- **Stability.** With `hints.previous` covering at least half the nodes,
  a node keeps its previous rect exactly (bit for bit) if both hold:
  - its layer is unchanged (true unless a hard constraint changed);
  - it isn't forced to move.

  A node is forced to move only when:
  - a node above it in its column grew into it (new nodes never push:
    they go into free space);
  - a new or widened column leaves its channel too narrow, which pushes
    the column right;
  - the band above grows into its band, or a gap needs more tracks than
    it has room for;
  - a newly dragged pin lands on it.

  Adding or removing a node or edge therefore leaves unrelated nodes
  where they were: new nodes keep previous layers fixed, so a new node
  between two adjacent layers shares a layer instead of shifting
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
  and 5 ms stable in a release build (`tests/performance.rs`). Random
  graphs with long edges, 1000 nodes or pins stay under about 0.35 s.
