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
                                                      └──Notify{trigger}──────────────────────────▶ Transition
Transition ──Delay{trigger}──▶ Transition (timer)
```

- `Trigger`, `Fire` and `Notify` edges go to every transition that accepts
  the trigger, since which one is taken depends on the target instance's
  current state.
- `Notify`: event → every transition accepting a subscription trigger on it
  (`Event::subscribers`).
- `Delay`: from each transition of the same machine that enters the timer
  transition's source state to the timer transition. A transition enters a
  state when its target is the state or inside it (the contract's rule), when
  default entry from its target passes through the state, or when its target
  is a history pseudo-state whose parent contains the state. The machine's
  initial entry has no node, so a timer on the initial state has no incoming
  `Delay` edge.
- `transition_successors(t)` returns `(successor, Option<RuleId>)`: through
  a handler's rule (`Some`) or a subscription (`None`). `Delay` links are not
  successors (a timer is not a cascade), so cascade cycles ignore them;
  `CycleStep::rule` is `None` for a subscription link.
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
| `crates/cascade-core/tests/causal_semantics.rs` | `Notify` and `Delay` edges, successors, subscription cycles | — |

## Invariants and constraints

- The graph is a pure function of the model; it is rebuilt on every load,
  never edited.
- `NodeIx`/`EdgeIx` are valid only for the graph that produced them.
- `CausalNode::from_element` maps only externals, transitions, events and
  handlers; other element kinds are not in the graph.
- Cones terminate on cyclic graphs and include edges back into the cone
  (a self-triggering retry loop's fire edge is in its own forward cone).
- Seed nodes always have hop 0; hop counts are shortest distances.
