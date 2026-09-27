# Cascade codebase overview

```yaml
Overview:
  description: >
    Cascade renders a system of communicating state machines from one YAML
    definition file, so a designer can trace how a transition in one machine
    causes transitions in others. A pure Rust core parses and resolves the
    definition, derives the causal graph and runs static checks; a layout
    engine, a FIFO simulator and scene builders turn that into four views
    (causal, structure, trace, matrix) drawn by a native GPUI app and
    exported as SVG/PNG. A CLI runs the same checks in CI. Product spec:
    docs/spec.md.

  subsystems:
    cascade-core: >
      Definition format (YAML → spanned Definition → resolved Model with
      typed ids), stable ElementKeys, the derived CausalGraph with cone and
      path queries, static analysis finding types and checks, fuzzy search,
      model diffing. Pure; no UI, the only I/O is load_file.
    cascade-layout: >
      Generic layered (Sugiyama-style) graph layout: sized nodes, ports,
      groups (lanes) stacked and routed around, orthogonal edge routing,
      stability from the previous layout, pins. Knows nothing about Cascade.
    cascade-sim: >
      Scenario files and the simulator: instances, queued FIFO event
      semantics, target selectors, traces; replays race candidates in both
      orders.
    cascade-scene: >
      Model + view state → Scene, a backend-neutral display list with hit
      targets, for all four views. Owns the visual encoding (Okabe-Ito hues,
      shapes, dashes, emphasis/dimming, badges, diff decorations), the
      cascade:// view link format, the pins sidecar, and SVG/PNG export.
    cascade-interop: >
      XState v5 and SCXML import; SCXML, Mermaid, P and YAML export; reading a
      file at a git revision for diff mode.
    cascade-cli: >
      The `cascade` binary: check (CI exit codes), render, export, import,
      diff, simulate, open.
    cascade-app: >
      The `cascade-app` GPUI binary: paints scenes, pan/zoom, selection, cone
      tracing, search, legend/entity filter, findings panel, trace and matrix
      interaction, live reload, click-to-source, diff mode, pin dragging.

  data_flow: >
    Text → cascade_core::parse_definition → Definition → resolve → Model.
    CausalGraph::build(Model); analyze(Model, CausalGraph) → Vec<Finding>.
    The app and CLI hold (Model, CausalGraph, findings) per load and a
    ViewState per window. SceneBuilder::build(SceneInput) → Scene, calling
    cascade_layout::layout with the previous layout and pins as hints; the
    trace view also takes cascade_sim::simulate output. The app paints the
    Scene and maps clicks through Scene::hit_test back to ElementKeys, which
    update the ViewState; the CLI writes the Scene with to_svg/to_png.
    Diff mode loads the old text via cascade_interop::read_at_rev and merges
    it with core::diff::merge_for_display before building scenes.
    Element identity crosses every boundary as ElementKey (stable) rather
    than typed ids (valid for one Model only).

Features Index:
  definition_format:
    description: YAML schema, parser with source spans, name resolution, the resolved Model and stable element keys.
    entry_points: [cascade_core::load_str, cascade_core::load_file, cascade_core::parse_definition, cascade_core::resolve]
    depends_on: []
    doc: docs/features/definition-format.md
  causal_graph:
    description: The derived causal graph, causal depth, forward/backward cones with hop limits, and path queries.
    entry_points: [cascade_core::CausalGraph::build, CausalGraph::cone, CausalGraph::paths_between, CausalGraph::depths]
    depends_on: [definition_format]
    doc: docs/features/causal-graph.md
  static_analysis:
    description: The seven checks plus state-dependent fire notes; `cascade check` for CI.
    entry_points: [cascade_core::analyze, cascade check]
    depends_on: [definition_format, causal_graph]
    doc: docs/features/static-analysis.md
  layered_layout:
    description: >
      Sugiyama-style layered layout: constrained network-simplex layering
      with cycle breaking, port-aware crossing minimisation, L1 coordinate
      placement, orthogonal channel routing with track assignment, lanes
      stacked and routed around, stability from the previous layout, pins
      with an obstacle router.
    entry_points: [cascade_layout::layout, cascade_layout::metrics]
    depends_on: []
    doc: docs/features/layered-layout.md
  simulator:
    description: Scenario files, FIFO simulation, traces, race orderings.
    entry_points: [cascade_sim::parse_scenario, cascade_sim::simulate, cascade_sim::race_orderings, cascade simulate]
    depends_on: [definition_format, static_analysis]
    doc: docs/features/simulator.md
  view_scenes:
    description: Scene builders for the causal, structure, trace and matrix views; emphasis; view links; pins sidecar; SVG/PNG export.
    entry_points: [cascade_scene::SceneBuilder::build, cascade_scene::ViewState::to_link, cascade_scene::to_svg, cascade render]
    depends_on: [definition_format, causal_graph, static_analysis, layered_layout, simulator]
    doc: docs/features/view-scenes.md
  interop_and_diff:
    description: XState/SCXML import, SCXML/Mermaid/P/YAML export, model diff and git revision loading.
    entry_points: [cascade_interop::import, cascade_interop::export, cascade_interop::read_at_rev, cascade_core::diff::diff_models, cascade export, cascade import, cascade diff]
    depends_on: [definition_format]
    doc: docs/features/interop-and-diff.md
  native_app:
    description: The GPUI desktop app.
    entry_points: [cascade-app, cascade open]
    depends_on: [view_scenes, static_analysis, simulator, interop_and_diff]
    doc: docs/features/native-app.md
  workstream_contracts:
    description: The interface types each crate exposes and which workstream implements each stub.
    entry_points: []
    depends_on: []
    doc: docs/features/workstream-contracts.md
```

## Repository layout

```text
Cargo.toml                 workspace; default members exclude cascade-app
crates/cascade-core/       model, parser, causal graph, analysis, search, diff
crates/cascade-layout/     layered layout engine
crates/cascade-sim/        scenarios, simulator, traces
crates/cascade-scene/      scenes, visual encoding, view links, pins, export
crates/cascade-interop/    import/export formats, git
crates/cascade-cli/        `cascade` binary
crates/cascade-app/        `cascade-app` GPUI binary
examples/                  example definitions (and scenarios)
docs/spec.md               product spec
docs/features/             one doc per feature
```

## Building

- `cargo test` / `cargo build` at the root cover the headless crates.
- `cargo run -p cascade-app -- examples/order-fulfillment/cascade.yaml` builds
  and runs the GPUI app (long first build).
