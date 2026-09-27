# Causal graph

## Scope

- Deriving the causal graph from a `Model`.
- Causal depth (the causal view's layer order).
- Forward and backward cones with a hop limit (cone tracing, F/B keys).
- Path queries between two elements.
- Direct transition successors (`T1 causes T2`).

## Non-scope

- Cycle detection and other checks (static-analysis.md), though they run on
  this graph.
- Instance-level causality (the simulator's traces).
- Drawing (view-scenes.md).

## Data and control flow

Nodes are created in a fixed order (external sources, transitions, events,
handlers, each in model order) so `NodeIx` values are deterministic for a
given model. Edges:

```text
External ──Trigger{trigger}──▶ Transition ──Emit──▶ Event ──Subscribe──▶ Handler ──Fire{rule}──▶ Transition
```

- `Trigger` and `Fire` edges go to every transition that accepts the trigger,
  since which one is taken depends on the target instance's current state.
- A handler is one controller's subscription to one event. Drawing handlers
  (rather than whole controllers) keeps paths honest: a controller that
  handles two events does not create a false path from one to the other.

`CausalGraph::depths` runs a BFS from every external source; unreachable
nodes get `None`.

`CausalGraph::cone(seeds, direction, max_hops)` is a 0-1 BFS. Stepping onto
a transition or external source costs one hop, while events and handlers
cost nothing. A node's hop count is its distance from the seeds. An edge
is in the cone when it can be traversed from inside the cone without
exceeding the limit. So depth N shows the transitions within N hops plus
the events and handlers the frontier transitions set off (what fires next).

`CausalGraph::paths_between(a, b)` intersects forward(a) with backward(b)
(nodes on some path a ⇝ b) and does the same for b ⇝ a. The union answers
the spec's path query regardless of selection order.

## Files

| File | Role | Key exports |
| --- | --- | --- |
| `crates/cascade-core/src/causal/mod.rs` | Graph types, derivation, depths, successors | `CausalGraph`, `CausalNode`, `CausalEdge`, `CausalEdgeKind`, `NodeIx`, `EdgeIx` |
| `crates/cascade-core/src/causal/cone.rs` | Cones and path queries | `Cone`, `Direction`, `CausalGraph::cone`, `CausalGraph::paths_between` |
| `crates/cascade-core/tests/causal.rs` | Behavioural tests on the spec example and a chain with fan-out | — |

## Invariants and constraints

- The graph is a pure function of the model; it is rebuilt on every load,
  never edited.
- `NodeIx`/`EdgeIx` are valid only for the graph that produced them.
- `CausalNode::from_element` maps only externals, transitions, events and
  handlers; other element kinds are not in the graph.
- Cones terminate on cyclic graphs and include edges back into the cone
  (a self-triggering retry loop's fire edge is in its own forward cone).
- Seed nodes always have hop 0; hop counts are shortest distances.
