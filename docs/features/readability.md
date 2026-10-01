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

Report: `cargo test -p cascade-scene --test readability -- --nocapture`
(structure view and build modes, the causal view, and the causal lanes).

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

## Placement

Owned by `feat/readable-placement` (`cascade-scene`,
`views/structure/{gutters/,selector.rs,wiring.rs,edit.rs}`).
build-and-play.md ("Build and play drawing") has the full drawing rules;
this section says why they are what they are.

### Wiring in gutters

The layout engine stacks groups top to bottom, all one width, and routes
an edge between two groups straight through the gap between them only
when the groups are neighbours. Any other edge detours through a side
corridor. The old build canvas put all events and controllers in one band
below every lane and the sources in another, so almost every wire crossed
several lanes and ran up the left corridor.

Now the wiring sits in **gutters**: a thin, untitled group above each
machine's groups and one below the last. Each wiring node goes to the
gutter where the fewest of its wires need a corridor, then where they
cross the fewest lane groups (a nested band counts as a lane group, so a
pill in `Order`'s `placed` band is reached from the gutter below `Order`):

| Node | Wires counted | Ties |
| --- | --- | --- |
| Event | its emits, plus one stand-in wire per handling controller to the nearest pill that controller's rules for the event fire into | nearest its placed controllers, then downward |
| Controller | its fires, plus its subscriptions to the placed events | upward (above the lane it fires into) |
| Source | its triggers | upward (right above the lane it triggers) |
| Nothing wired | — | the last gutter |

Each rule looks only at the node's own wiring (events also at the rules
handling them), so an edit moves only the wiring nodes whose wiring
changed. A global search over gutter assignments (coordinate descent
from 200 starts, total over all wires) finds 10 corridor wires for the
shop, one fewer than these local rules (11), so the local rules keep
their stability for a cost of one wire.

**Row order.** Inside a gutter, nodes run left to right by the median
estimated column of the pills they wire. A pill's column is estimated
before layout from breadth-first depth in its band (`2 × depth + 1`),
which is how the layered layout places it. Ties go sources, events,
controllers, then definition order. A controller that handles an event
in the same gutter goes right after its last such event, so the
subscription is a short hop right. Gutter nodes get fixed columns
(`LayerConstraint::Exact`), which keeps each gutter one row in that
order. `LayoutOptions::align_across_groups` is on for the structure view.
Once the engine implements it, the row slides under its pills. Until
then the row starts at the lane's left edge, and the order alone keeps
wires from crossing each other.

**Columns across edits.** Ranks would shift every node right of an
insertion, so the `SceneBuilder` keeps a `WiringMemo`: a node keeps its
column per gutter, a node new to a gutter takes the next unused column
(the right end), and a controller that would sit left of an event it
handles in the same gutter moves to the end (both ends are fixed, and
the engine rejects a fixed edge pointing backwards). Undo restores the
earlier columns, so the cached layout is reused. `SceneBuilder::reset`
forgets the columns, and a fresh build is the tidy order again.

**Ports face the gutter.** Pills gained a North-out port (5) next to the
South-out one (4). An emit leaves toward its event's gutter, and fires
and triggers enter from their controller's or source's side, so no wire
wraps around its pill.

### Label policy

- A fire's label is its selector without the machine, shortened: `where
  orderId == event.orderId` → `by orderId`, `all … where` → `all by …`,
  `new … with orderId = event.orderId` → `new with orderId`. A clause
  comparing different names or a literal keeps both sides (`by
  orderId=event.id`, `tier=gold`). The singleton selector (the default,
  no `where`) gets no label. `[when]` follows.
- A fire standing for several rules lists each distinct selector and
  condition once (Tracking → `poll_tracking`: `by orderId [parcel not
  yet delivered]`, not the selector twice).
- Among one controller's fires into one machine (parallel wires leaving
  it together), a label identical to one already shown is left off:
  Orders' four fires into `Order` carry `by orderId` once.
- The full selector is one click away: the edge's hit target is the
  rule, and the inspector shows it. (A hover tooltip needs the app. The
  scene has no field for one, and `SceneEdge` is a shared contract.)
- Labels still go to the engine as label boxes (`LayoutEdge::with_label`),
  so it reserves their room. Guard labels are unchanged.

### View mode

A self-link (a pill whose event's controller fires back into the same
pill, e.g. `in_transit → in_transit` in the shop) is drawn by hand as a
small loop. Its label used to sit at the loop's midpoint, on top of the
neighbouring nodes. It now goes beside the loop's outer corner, else
above the loop, else left of it, whichever first clears every node. Two
further changes were tried and dropped because they regressed a metric
(see Results).

## Results (placement)

`cargo test -p cascade-scene --test readability -- --nocapture`, on
this branch alone (no routing changes; `align_across_groups` is still a
no-op):

| Scene | Crossings | Length | Bends | Label overlaps | Corridor | Through nodes | Area |
| --- | --- | --- | --- | --- | --- | --- | --- |
| shop structure/build, before | 728 | 139 922 | 376 | 9 | 38 | 0 | 7 162 608 |
| shop structure/build, after | **83** | **55 488** | 328 | **0** | **11** | 0 | 4 451 794 |
| order-fulfillment structure/build, before | 23 | 9 745 | 60 | 0 | 7 | 0 | 756 856 |
| order-fulfillment structure/build, after | **4** | **4 572** | 46 | 0 | **0** | 0 | 503 733 |
| shop structure/view, before | 38 | 31 165 | 151 | 5 | 9 | 0 | 2 827 336 |
| shop structure/view, after | 38 | 31 165 | 151 | **3** | 9 | 0 | 2 827 336 |
| order-fulfillment structure/view | 1 | 1 541 | 10 | 0 | 0 | 0 | 239 832 (unchanged) |
| shop causal | 17 | 16 518 | 72 | 0 | 0 | 0 | unchanged |
| order-fulfillment causal | 0 | 896 | 0 | 0 | 0 | 0 | unchanged |

Targets (asserted in `tests/readability_targets.rs`):

| Target | Placement alone |
| --- | --- |
| shop build: crossings ≤ 150, length ≤ 70 000, 0 label overlaps, 0 through nodes | met |
| shop build: corridor edges ≤ 5 | not met (11), `#[ignore]`d. No gutter assignment found does better than 10: with this routing, every wire between non-neighbouring groups needs a corridor. Reaching 5 needs routing that can run a wire between a lane's nodes. |
| order-fulfillment build: every target | met |
| shop view: no regression, corridor ≤ 9, 0 through nodes | met |
| shop view: 0 label overlaps | not met (3), `#[ignore]`d. The overlaps are labels of different links placed in the same spot of a shared gap, which collision-free label placement in the engine fixes. |
| every causal view: no regression | met (unchanged) |

### Arrangements tried

| Arrangement | shop build: crossings / length / overlaps / corridor | order-fulfillment build |
| --- | --- | --- |
| One band below the lanes (before) | 728 / 139 922 / 9 / 38 | 23 / 9 745 / 0 / 7 |
| **Gutters (kept)** | **83 / 55 488 / 0 / 11** | **4 / 4 572 / 0 / 0** |
| Gutters, first cut (one stand-in wire per fire, event ties always downward) | 94 / 58 157 / 0 / 12 | 4 / 4 572 / 0 / 0 |
| Gutters, but every source in the top gutter | 202 / 77 936 / 0 / 19 | 4 / 4 572 / 0 / 0 |
| Wiring column at the right end of each lane (the nearest the engine gets to a column right of the lanes) | 138 / 63 292 / 1 / 10 | 5 / 4 590 / 0 / 0 |

A separate wiring column to the right of the lanes cannot be expressed:
groups only stack vertically and share one width. Its closest stand-in
puts each wiring node in a lane's own group, in columns after the states
(fixed layer 1000 + rank). The engine compresses those fixed layers in
among the states, so the wiring mixes into the lanes and the lanes
reshuffle. That also breaks the lane-stays-put property between view and
edit mode. It saves one corridor wire, but gutters win on everything
else.

View mode, tried and dropped:

| Change | shop view: crossings / length / overlaps | Why dropped |
| --- | --- | --- |
| Self-links routed by the engine (reserves label room above the node) | 38 / 31 507 / 3 | longer loops: length regresses |
| Parallel link labels said once (one pill into two `reserve` pills: `OrderPaid › Fulfillment` twice) | 39 / 31 151 / 2 | removing a label box re-routes a link: one more crossing |
| **Self-link label beside the loop (kept)** | **38 / 31 165 / 3** | — |

### What a reader sees now

- **Build canvas, shop.** Above `Order`: Customer and Clock, and
  ReturnRequested with Returns, whose fire drops into `Order`. Between
  `Order` and `Payment`: OrderPlaced → Checkout (dropping `new with
  orderId` into Payment's first pill), PaymentGateway,
  OrderCancelled → Refunds, and PaymentCaptured / PaymentRefunded →
  Orders. Between `Payment` and `Inventory`: PaymentAuthorized → Billing
  and FraudCheck, both firing straight back up into `Payment`, plus
  OrderPaid and Warehouse. Between `Inventory` and `Shipment`:
  StockReserved → Fulfillment, ShipmentDispatched → Shipping and Tracking,
  Carrier, TrackingPolled and ShipmentDelivered. Between `Shipment` and `Notification`: OrderDelivered →
  Notifier, MailProvider, ShipmentLost. A transition → event →
  controller → transition chain is mostly two short hops through one
  gap. The 11 wires that still take a side corridor are the fan-ins of
  Orders (events from three machines) and Fulfillment (events from two),
  and wires between `Order`'s top lane and the gutter below its nested
  `placed` band, which sits in between. Fire labels read `by
  orderId` once per fan instead of the full selector on every wire, and
  no label overlaps anything.
- **Build canvas, order-fulfillment.** The three sources sit right above
  `Order` and drop into their pills. OrderPaid → Fulfillment and
  OrderCancelled sit between the lanes, and Fulfillment's `by orderId`
  fire drops into `Shipment`. Shipped hangs below `Shipment`. No
  corridors.
- **Still to come from routing.** Gutter rows start at the left edge
  rather than under their pills, so wires to pills far right run
  horizontally along the gap. `align_across_groups` will slide the rows
  under their pills.
- **View mode.** Unchanged except that the `TrackingPolled › Tracking`
  self-link label sits clear, right of its loop.

### Engine limitations met

1. Groups stack vertically and share one width, so there is no wiring
   column beside the lanes.
2. A wire between groups that are not neighbours always takes a side
   corridor. With lanes between wiring and its targets, that bounds the
   shop's corridor wires at about 10.
3. In a stable relayout, a new node with no neighbour in its band goes
   below every placed node of the band (`stability::place_nodes`, the
   `(None, None)` case), even when its own column is empty. So appending
   a source or a lone event to a middle gutter opens a new row and
   shifts every lane below down (rigidly).
   `appending_a_source_moves_at_most_what_lies_below_its_gutter_as_a_whole`
   pins that behaviour. Placing such a node at the top of its empty
   column would keep the lanes still.
4. Gutter rows are packed from the left edge until `align_across_groups`
   exists.
5. Engine-routed self-loops are longer than the hand-drawn loop, so
   self-links stay hand-drawn.

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

## Causal lanes

`ViewState::group_by_machine` draws the causal view with one lane per
machine on shared causal columns (view-scenes.md, "Causal lanes";
`LayoutOptions::shared_layers` in layered-layout.md). The report prints it
as `causal/lanes`, with one more measure: `leftward`
(`metrics::leftward_edges`), the edges that are not cascade-cycle back
edges yet end left of their start or have a horizontal segment heading
left. `tests/causal_lanes.rs` asserts it is 0, with 0 label overlaps, 0
corridor edges and 0 edges through nodes, at most 0 crossings and 1 200
length for order-fulfillment, and at most 40 crossings and 21 000 length
for the shop.

| Scene | Crossings | Length | Bends | Label overlaps | Corridor | Through nodes | Leftward | Area |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| order-fulfillment causal (flat) | 0 | 896 | 0 | 0 | 0 | 0 | — | 291 808 |
| order-fulfillment causal/lanes | 0 | 941 | 4 | 0 | 0 | 0 | **0** | 363 684 |
| shop causal (flat) | 17 | 16 518 | 72 | 0 | 0 | 0 | — | 3 332 486 |
| shop causal/lanes | 38 | 19 174 | 94 | 0 | 0 | 0 | **0** | 5 135 480 |

Lanes cost crossings and length: a machine's transitions no longer sit
next to the events and controllers of other machines they cause or are
caused by, so those links cross lanes (every one heading right). The flat
view is unchanged.

### Placements tried

Rules as in view-scenes.md, with one changed at a time:

| Placement | order-fulfillment: crossings / length / bends | shop: crossings / length / bends / area |
| --- | --- | --- |
| **Event with its first emitter, handler in the lane it fires into, source with its first trigger (kept)** | **0 / 941 / 4** | **38 / 19 174 / 94 / 5 135 480** |
| Event in the lane its first handler fires into | 1 / 941 / 4 | 31 / 18 198 / 104 / 4 761 408 |
| Handler with its event (the event's emitter lane) | 0 / 941 / 4 | 32 / 18 560 / 92 / 5 143 760 |
| Sources in the Unattached lane | 3 / 2 132 / 22 | 49 / 30 938 / 144 / 6 932 016 |

None regresses the guarantees (0 leftward, overlaps, corridors). Putting
handlers with their event saves 6 crossings in the shop and costs nothing
in order-fulfillment; putting events with their consumer saves 7 but
adds a crossing to order-fulfillment and 10 bends to the shop. The
specified rules keep a machine's causes and effects in its own lane
(a handler sits right before the transition it fires, an event right
after the transition emitting it), measure within a few crossings of the
best, and were kept. Sources in a lane of their own are clearly worse:
every trigger crosses lanes.

## Invariants and constraints

- Stability guarantees still hold (an edit moves no unrelated node).
- View mode must not get worse on any metric.
- Determinism: the same input gives the same scene.
