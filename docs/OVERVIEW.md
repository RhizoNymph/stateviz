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
      Scenario files and the simulator: instances, one global FIFO queue of
      events and controller fires, target selectors, traces with cause
      links; replays race candidates with the contested fires swapped.
    cascade-scene: >
      Model + view state → Scene, a backend-neutral display list with hit
      targets, for all four views. Owns the visual encoding (Okabe-Ito hues,
      shapes, dashes, emphasis/dimming, badges, diff decorations), the
      cascade:// view link format, the pins sidecar, and SVG/PNG export.
    cascade-interop: >
      XState v5 and SCXML import through a shared statechart IR (names
      sanitized, entry/exit emits attributed, a routing controller and
      external sources synthesized so imports resolve); SCXML, Mermaid
      (structure and causal), P skeleton and YAML export; reading a file at a
      git revision for diff mode.
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
    description: >
      The seven checks (dead external triggers count as invalid fires) plus
      state-dependent fire notes, sorted deterministically; `cascade check`
      for CI with text/JSON reports and exit codes; the pinned shop example.
    entry_points: [cascade_core::analyze, cascade_core::analysis::has_errors, cascade check]
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
    description: >
      Scenario files (parse, validate, discover), FIFO simulation with
      selectors, spawn, history and payloads, traces with cause links and
      stable lifelines, race candidates replayed in both orders.
    entry_points: [cascade_sim::parse_scenario, cascade_sim::validate, cascade_sim::discover_scenarios, cascade_sim::simulate, cascade_sim::simulate_run, cascade_sim::race_orderings, cascade simulate]
    depends_on: [definition_format, static_analysis]
    doc: docs/features/simulator.md
  view_scenes:
    description: Scene builders for the causal, structure, trace and matrix views; the interaction model (selection, cones, path queries, search, hide stubs); layout caching; view links; pins sidecar; SVG/PNG export.
    entry_points: [cascade_scene::SceneBuilder::build, cascade_scene::emphasis::Interaction, cascade_scene::ViewState::to_link, cascade_scene::to_svg, cascade_scene::to_png, cascade render]
    depends_on: [definition_format, causal_graph, static_analysis, layered_layout, simulator]
    doc: docs/features/view-scenes.md
  interop_and_diff:
    description: >
      XState/SCXML import with warnings, SCXML/Mermaid (structure and causal)/P/YAML export,
      comment-preserving YAML patches for edit ops (span index over the file,
      rewrite fallback flagged), model diff, ghost merge for diff mode, and
      git revision loading.
    entry_points: [cascade_interop::import, cascade_interop::import_with, cascade_interop::export, cascade_interop::to_yaml, cascade_interop::patch_text, cascade_interop::read_at_rev, cascade_core::diff::diff_models, cascade_core::diff::merge_for_display, cascade export, cascade import, cascade diff]
    depends_on: [definition_format]
    doc: docs/features/interop-and-diff.md
  native_app:
    description: >
      The GPUI desktop app: paints scenes with pan/zoom/fit and culling, shared
      selection, cone tracing with a depth stepper, path queries, legend
      entity filter, findings panel, fuzzy search, trace scenario/race picker,
      matrix drill-down, click-to-source, pin dragging, diff mode, view links
      via the clipboard, light/dark theme, and live reload that keeps the last
      good model on screen.
    entry_points: [cascade-app, cascade open, cascade_app::workspace::Workspace]
    depends_on: [definition_format, causal_graph, view_scenes, static_analysis, simulator, interop_and_diff]
    doc: docs/features/native-app.md
  build_and_play:
    description: >
      Build systems in the app (typed edit ops with undo, saved to the YAML
      file in place keeping comments) and play them (interactive simulator
      session with instances, trigger palette, queue stepping, branchable
      timeline, record as scenario).
    entry_points: [cascade_core::edit::apply, cascade_interop::patch_text, cascade_sim::PlaySession, cascade_scene::PlayOverlay, cascade-app build/play modes]
    depends_on: [definition_format, simulator, view_scenes, interop_and_diff, native_app]
    doc: docs/features/build-and-play.md
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
- `cargo build -p cascade-app` builds the GPUI app; `cargo run -p cascade-app
  -- examples/order-fulfillment/cascade.yaml` runs it (needs a Wayland display
  on Linux). GPUI comes from one pinned zed git rev (`gpui`, `gpui_platform`)
  plus the `[patch.crates-io]` block in the root `Cargo.toml`; the first build
  clones zed into cargo's git cache and takes a few minutes.
- `cargo test -p cascade-app` runs the app's unit tests (pure logic; no
  display needed).
