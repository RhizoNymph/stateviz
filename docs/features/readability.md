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

## Invariants and constraints

- Stability guarantees still hold (an edit moves no unrelated node).
- View mode must not get worse on any metric.
- Determinism: the same input gives the same scene.
