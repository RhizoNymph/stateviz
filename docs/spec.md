# Spec: Cascade — a visualizer for interacting state machines

Sep 25, 2026 · @Nymph

## Purpose

Cascade renders a system of communicating state machines from one definition file, so a designer can trace how a transition in one machine causes transitions in others. It is a design-time tool: used while deciding how entities and controllers should behave, before and alongside the code.

The test it must pass: pick any transition and answer "what can this set off, across every machine?" and "what can cause this?" in one action.

**Goals**

- Generate every view from a single definition, with no hand-drawn diagrams to drift out of date.
- Stay readable at roughly 20 machines, 200 states and 50 controllers, through filtering rather than zooming out.
- Catch design bugs statically: cascade cycles, unhandled events, dead commands, unreachable states.
- Fit a normal repo workflow: the definition lives in git, the checks run in CI.

**Non-goals**

- A runtime state machine library or execution engine.
- A general-purpose diagram editor. Layout is automatic; people nudge it, they don't draw it.
- Full formal verification. Cascade exports to tools that do this (P, TLA+) instead.

## Core model

Transitions are first-class objects, not edge labels, because they are what emit events and what controllers fire. Every view and check is derived from the six concepts below.

| Concept | What it is | Key fields |
| --- | --- | --- |
| Machine | The state machine for one entity type (Order, Shipment, Job) | id, color, initial state, states, transitions |
| State | A phase of one machine; may nest inside a parent state | id, parent, kind (normal, final, history) |
| Transition | A move from one state to another, identified as `Machine: before → after` | from, to, trigger, guard, emits |
| Event | A named signal emitted by a transition | name, payload fields |
| Controller | A set of reaction rules: on an event, if a condition holds, fire a trigger on a machine | subscriptions, rules, target selector |
| External source | Anything outside the system that fires triggers: users, timers, webhooks | name, triggers it can fire |

The causal graph is derived, never authored. Transition T1 causes T2 when T1 emits an event, a controller handles that event, and one of its rules fires T2's trigger. For example: `Order: pending → paid` emits `OrderPaid`, which `Fulfillment` handles by firing `start` on the matching Shipment, which takes `Shipment: idle → picking`.

The target selector says which instance a controller acts on ("the Shipment whose orderId matches the event"). It matters for race detection and for the trace view, which runs over instances rather than types.

## Source format

One YAML definition file per system, checked into the repo next to the code it describes. Cascade watches the file and re-renders on save.

```yaml
machines:
  Order:
    color: blue
    initial: draft
    states: [draft, pending, paid, cancelled]
    transitions:
      - { from: draft,   to: pending,   on: submit }
      - { from: pending, to: paid,      on: capture_ok, emits: [OrderPaid] }
      - { from: pending, to: cancelled, on: timeout,    emits: [OrderCancelled] }

  Shipment:
    color: green
    initial: idle
    states: [idle, picking, shipped]
    transitions:
      - { from: idle,    to: picking, on: start }
      - { from: picking, to: shipped, on: handoff, emits: [Shipped] }

controllers:
  Fulfillment:
    on:
      OrderPaid:
        - fire: Shipment.start
          target: Shipment where orderId == event.orderId

external:
  Customer: [Order.submit]
  PaymentGateway: [Order.capture_ok]
  Clock: [Order.timeout]
```

Nested states use a `states:` map inside a state, as in statecharts. Guards are free-text strings in v1; Cascade displays them but does not evaluate them.

**Import:** XState v5 machine configs and SCXML files, mapped onto the same model. Extracting machines from code (Rust enums with match arms, TypeScript discriminated unions) is a later experiment.

**Export:** SCXML, Mermaid (for READMEs), SVG and PNG of any view, and a P language skeleton for model checking.

## Views

Four views of the same model, with the causal flow view as the default because cross-machine causality is what existing tools show worst. Selection is shared, so picking a transition in one view highlights it in all four.

| View | Question it answers | What's drawn | Layout |
| --- | --- | --- | --- |
| Causal flow (default) | What does this set off, and what can cause it? | Transition pills (`Order: pending → paid`), event tags, controller hexagons; states are hidden | Left to right by causal depth from external sources |
| Structure | What phases does each machine have, and where do the cross-machine links attach? | Each machine as a colored lane with its states; transition pills sit on the edges; emit and fire edges cross between lanes | Lanes stacked, layered layout inside each |
| Trace | In a concrete scenario, what happens in what order, and where can things interleave? | A lifeline per machine instance and controller, messages between them | Sequence diagram, time running down |
| Matrix | Which parts of the system are coupled, and how tightly? | Machines × machines grid; each cell counts the causal links from row to column | Grid, sorted to cluster coupled machines |

The trace view runs from scenario files: an ordered list of external triggers, stepped through by a built-in simulator. A later version also accepts recorded runtime logs, so real behavior can be compared against the design.

The matrix is the entry point for large systems: a dense cell is a click into the causal view filtered to those two machines.

## Interaction

Filtering dims rather than hides by default, so the layout never jumps and the reader keeps their bearings. Hiding is available but always leaves a stub.

- **Cone tracing:** Select a transition and press F for its forward cone (everything it can set off) or B for its backward cone (everything that can cause it). A depth slider limits the hops. Everything outside the cone fades to 15% opacity.
- **Path query:** Select two transitions to show every causal path between them, and nothing else.
- **Entity filter:** Toggle machines on and off from a legend. A hidden machine collapses to a stub node that keeps its cross-links, labeled with the count ("Payment, 4 links").
- **Collapse:** Composite states collapse to one node; whole machines collapse to one node in the structure view.
- **Search:** Fuzzy search over machines, states, transitions, events and controllers. Enter focuses the match and centers it.
- **Click to source:** Any element opens its line in the definition file in your editor.
- **Diff mode:** Compare the definition at two git refs. Added elements show with a green outline, removed ones as red ghosts. This is the design review mode.

Every filter state is encoded in the URL, so a specific view of the design can be shared as a link.

## Visual encoding

Hue means entity and nothing else; everything else is carried by shape, line style and weight. Selection and focus use outline weight, never color, so the entity coding stays intact.

| Element | Shape | Color and line |
| --- | --- | --- |
| Machine | Lane (structure view), legend chip | Its own hue from the Okabe-Ito colorblind-safe palette (8 hues); past 8, machines are grouped by domain and share a hue at different lightness |
| State | Rounded rectangle | Machine hue, pale fill; initial state gets a thick left border, final states a double border |
| Transition | Pill labeled `before → after`, trigger name underneath | Machine hue, full saturation |
| Event | Tag shape | Neutral gray |
| Controller | Hexagon | Dark neutral outline, no fill |
| Transition within a machine | Solid arrow | Machine hue |
| Emit (transition to event) | Dashed arrow | Gray |
| Fire (controller to transition) | Dashed arrow | Hue of the target machine |
| Guard | Bracketed label on the arrow, `[amount > 0]` | Default text color |
| Analysis finding | Badge with a count | Red outline; back edges from cascade cycles drawn red |

Pills carry both endpoints so a transition reads correctly even when its machine's states are hidden, which is most of the time in the causal view.

## Layout

All graph views use ELK's layered algorithm with orthogonal edge routing, run in a web worker. Target: under 1 second for 500 nodes.

- **Ports:** Transition pills get an input port (fires arrive) and an output port (emits leave), so cross-machine edges attach cleanly instead of converging on a point.
- **Structure view:** Each machine is a compound node, stacked vertically in definition order. Cross-lane edges route around lanes, not through them.
- **Causal view:** Layers are causal depth from external sources. Cycles are broken by ELK's cycle-breaking step, and the resulting back edges are drawn red.
- **Stability:** On every edit, previous positions are fed back to ELK as ordering constraints, so adding one transition doesn't reshuffle the whole diagram.
- **Pins:** Dragging a node pins it. Pins are saved to a sidecar file, `cascade.layout.json`, next to the definition.
- **Trace view:** Lifelines follow definition order, so the same scenario always lays out the same way.

Graphviz is ruled out: it handles edges between clusters poorly and has no port-level control over where edges attach.

## Static analysis

Seven checks run on every save and in CI. Findings show as badges on the affected elements and as a list panel; clicking one focuses it in the causal view.

| Check | Flags when | Default severity |
| --- | --- | --- |
| Invalid fire | A controller fires a trigger that no transition in the target machine accepts | Error |
| Nondeterminism | One state has two transitions on the same trigger without mutually exclusive guards | Error |
| Cascade cycle | The causal graph has a cycle, so a transition can eventually re-trigger itself | Warning; silenced by marking the cycle `bounded: true` (retry loops) |
| Unhandled event | An event is emitted but no controller subscribes to it | Warning |
| Orphan controller | A controller subscribes to an event nothing emits | Warning |
| Unreachable state | No path from the initial state reaches it, counting cross-machine fires and external sources | Warning |
| Race candidate | One originating event leads, through different controllers, to two fires on the same machine instance | Info; opens the trace view to inspect ordering |

A related info note lists state-dependent fires: triggers the target accepts only from some states, with the states where the fire would be dropped. Anything beyond these checks, such as proving a property over all interleavings, goes through the P export.

## Architecture

A pure core library does all modeling and analysis; the CLI, web app and editor extension are thin shells around it. That keeps checks identical in CI and in the UI.

| Component | Responsibility | Stack |
| --- | --- | --- |
| Core | Parse and validate the definition, derive the structure and causal graphs, run checks, step the simulator | TypeScript, Zod for the schema |
| CLI | `cascade check` (non-zero exit on errors), `cascade render --view causal`, `cascade export --to scxml`, `cascade serve` | Node |
| Web app | The four views, filtering, diff mode | React, React Flow for rendering, elkjs in a web worker for layout |
| Live link | `cascade serve` watches the definition and pushes the new model to the browser over a WebSocket | Node, chokidar |
| Editor extension | The web app in a side panel, click-to-source, findings as squiggles on the YAML | VS Code webview |

React Flow is chosen over Cytoscape.js because the pills, lanes and hexagons are easier as custom components. Cytoscape is the fallback if views need to exceed about 2,000 nodes.

## Milestones

The causal view ships before the structure view, since it is the part no existing tool covers. Each milestone is usable on its own.

| Milestone | Scope | Done when |
| --- | --- | --- |
| M1: Model and checks | YAML schema, validation, causal graph derivation, `cascade check` with all seven checks | A 5-machine example design produces exactly the expected findings |
| M2: Causal view | Web app with the causal view, entity filter, cone tracing, search, live reload | The one-click question from Purpose is answerable on the example |
| M3: Structure view | Lanes, states, transition pills, cross-lane edges, collapse, layout stability and pins | An edit to one transition moves no unrelated node |
| M4: Trace view | Scenario files, simulator, sequence diagram, race candidates linked to traces | Each race candidate in the example opens a trace showing both orderings |
| M5: Interop and diff | XState and SCXML import; SCXML, Mermaid and P export; git diff mode | An existing XState project renders without hand edits |
| M6: Editor integration | VS Code extension, click-to-source, first code-extraction experiment | Checks show inline while editing the YAML |

## Open questions

- [ ] **Event semantics:** Does a controller's fire run in the same step as the emit (run-to-completion, as in statecharts) or go on a queue? This decides what the simulator does and what counts as a race.
- [ ] **Stateful controllers:** If controllers need their own state, should they just be machines with a controller flag? That would simplify the model to one concept.
- [ ] **Instances:** How do controllers spawn new instances, or fan out to many? The target selector needs syntax for both.
- [ ] **Time:** Are timeouts external triggers from a Clock source, or delayed transitions (`after: 30s`) as in statecharts?
- [ ] **Source of truth:** Hand-written YAML is simple but can drift from the code; extraction from code stays honest but is language-specific. Is drift acceptable if CI can detect it?
- [ ] **Core language:** TypeScript keeps one language end to end; Rust compiled to WebAssembly would allow a native CLI and faster analysis on large systems.
