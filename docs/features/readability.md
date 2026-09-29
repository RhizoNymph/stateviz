# Readability of layouts and routing

## Scope

- A measurable definition of "readable" for any scene
  (`cascade_scene::metrics`), and a report over the examples.
- Placement of the build canvas's wiring (events, controllers, sources)
  and routing of edges between machine lanes and the wiring.
- Label placement for guards and selectors.

## Non-scope

- Changing the visual encoding (shapes, hues, dashes).
- Manual layout: layout stays automatic, pins still override.

## Measure

`cascade_scene::metrics::measure(&Scene) -> SceneMetrics` counts, over
visible items: edge crossings (pairs of edges whose segments properly
cross), total edge length, bends, edges passing through a node that is not
their endpoint, overlapping labels (edge labels with each other, with node
labels and with foreign nodes), edges routed through a side corridor
(points outside every lane's horizontal extent), and bounding area.

Report: `cargo test -p cascade-scene --test readability -- --nocapture`.

### Baseline (before this work)

```text
order-fulfillment  structure/view   edges  11  crossings   1  length   1541  bends  10  label-overlaps 0  corridor  0
order-fulfillment  structure/build  edges  18  crossings  23  length   9745  bends  60  label-overlaps 0  corridor  7
order-fulfillment  causal           edges   8  crossings   0  length    896  bends   0  label-overlaps 0  corridor  0
shop               structure/view   edges  70  crossings  38  length  31165  bends 151  label-overlaps 5  corridor  9
shop               structure/build  edges 108  crossings 728  length 139922  bends 376  label-overlaps 9  corridor 38
shop               causal           edges  57  crossings  17  length  16518  bends  72  label-overlaps 0  corridor  0
```

### Targets

| Scene | Crossings | Length | Label overlaps | Corridor edges | Through nodes |
| --- | --- | --- | --- | --- | --- |
| shop structure/build | ≤ 150 | ≤ 70 000 | 0 | ≤ 5 | 0 |
| order-fulfillment structure/build | ≤ 5 | ≤ 5 000 | 0 | 0 | 0 |
| shop structure/view | ≤ 38 (no regression) | ≤ 31 165 | 0 | ≤ 9 | 0 |
| every causal view | no regression | no regression | 0 | 0 | 0 |

Beyond the numbers, renders are checked by eye: a reader should be able to
follow a transition → event → controller → transition chain without
tracing wires across the canvas.

## Workstreams

| Branch | Owns | Work |
| --- | --- | --- |
| `feat/readable-placement` | `cascade-scene` | Where wiring nodes go (gutters between the lanes they connect, sources beside their lanes), band ordering, label policy (dedupe/shorten selector labels), turning on `LayoutOptions::align_across_groups`, readability assertions |
| `feat/readable-routing` | `cascade-layout` | `align_across_groups`, direct routing between adjacent groups, corridor use only when unavoidable (left/right split), crossing-aware track assignment, collision-free label placement, faster obstacle routing |

## Routing

Owned by `feat/readable-routing` (`cascade-layout`); the details are in
layered-layout.md, steps 6, 8 and 9.

- **Adjacent groups.** An edge between neighbouring groups runs straight
  through the gap between them. An end on a North or South port facing the
  other group leaves straight out of the port when nothing of its column is
  in the way (a direct leg), so lined-up ends make one vertical line and
  others one jog. Such edges never use a corridor.
- **Groups in between.** An edge between groups that are not neighbours
  alternates runs in the gaps with verticals straight across each group
  in between, through a *passage*: a strip of the lanes' width clear of
  that group's columns, channel tracks and pins. A small dynamic program
  picks one passage per group, trading horizontal travel, jogs (24) and
  in-group links crossed (300 each). A side corridor is used only for a
  group with no passage left, on whichever side is shorter.
- **Tracks.** Verticals sharing a passage are ordered by where they come
  from and go to and packed an edge spacing apart. Runs in each gap and
  verticals in each corridor get crossing-aware tracks, which nests
  corridor tracks by span.
- **`align_across_groups`.** Each freshly placed group shifts its layers
  right toward the other ends of its cross-group edges. This is an exact
  L1 compaction that keeps order and spacing. Kept groups and pins never
  move, and unchanged groups come back exactly.
- **Labels.** A label goes in its reserved room when that is clear.
  Otherwise it slides along its route's segments, longer first, from the
  middle outward on either side, until it clears nodes, group header
  strips and other labels (and preferably other routes).
  `LayoutResult::unplaced_labels` counts any it could not place cleanly.
- **Obstacle routing.** Crossing a foreign group straight along the
  stacking axis is cheap for the A* router; running along inside one is
  not. That stops searches flooding the grid. Steps cost O(1) through
  prefix sums, and search buffers are reused.

## Results (routing)

Layout level (`cargo test -p cascade-layout --test readability --
--nocapture`): build-canvas-shaped graphs with 6 lanes, summed over 4
seeds, `align_across_groups` on. The option had no effect before this
work.

| Canvas | Crossings | Corridor edges | Length | Bends | Label overlaps |
| --- | --- | --- | --- | --- | --- |
| one wiring band, before | 2801 | 111 | 419 600 | 1442 | 7 |
| one wiring band, after | 1913 | 0 | 368 789 | 1582 | 0 |
| gutters, before | 699 | 40 | 271 588 | 1316 | 3 |
| gutters, after | 649 | 0 | 245 586 | 1250 | 0 |
| gutters, after, alignment off | 806 | 0 | 259 280 | 1276 | 0 |

Scene level (`cargo test -p cascade-scene --test readability --
--nocapture`), with this branch's routing and the scene code as it was
before placement work (wiring in one band below the lanes, alignment
off):

| Scene | Crossings | Length | Bends | Label overlaps | Corridor |
| --- | --- | --- | --- | --- | --- |
| shop structure/view | 38 → 31 | 31 165 → 24 212 | 151 → 117 | 5 → 2 | 9 → 0 |
| shop structure/build | 728 → 469 | 139 922 → 139 976 | 376 → 484 | 9 → 0 | 38 → 0 |
| order-fulfillment structure/view | 1 → 1 | 1541 → 1354 | 10 → 8 | 0 → 0 | 0 → 0 |
| order-fulfillment structure/build | 23 → 10 | 9745 → 7265 | 60 → 40 | 0 → 0 | 7 → 0 |
| causal views | unchanged | unchanged | unchanged | 0 | 0 |

The two view-mode label overlaps left are the hand-drawn self-link label
the scene places itself; `feat/readable-placement` places it clear.

Combined with `feat/readable-placement` (gutters, alignment on), every
target in `tests/readability_targets.rs` passes, including the ones it
ignores until routing lands:

| Scene | Crossings | Length | Bends | Label overlaps | Corridor |
| --- | --- | --- | --- | --- | --- |
| shop structure/build | 84 | 39 541 | 196 | 0 | 0 |
| shop structure/view | 33 | 24 789 | 111 | 0 | 0 |
| order-fulfillment structure/build | 4 | 2607 | 16 | 0 | 0 |
| order-fulfillment structure/view | 1 | 1354 | 6 | 0 | 0 |

Timings, release. The 500-node / 800-edge layout is unchanged within
noise: fresh 24.6 ms and stable 21.1 ms without groups (26.0 and 22.5 ms
before), and 8.9 and 8.5 ms with 8 groups (8.4 and 9.2 ms before).
Pinning the busiest controller of a 96-node canvas costs 1.7–3.7 ms
(10–170 ms before). A pinned shop build, scene included, takes 14–18 ms
(up to 142 ms before).

## Invariants and constraints

- Stability guarantees still hold (an edit moves no unrelated node).
- View mode must not get worse on any metric.
- Determinism: the same input gives the same scene.
