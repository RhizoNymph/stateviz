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
      typed ids), the expression language (guards, conditions, filters,
      field types, durations, eval, solve), stable ElementKeys, the derived CausalGraph with cone and
      path queries, static analysis finding types and checks, fuzzy search,
      model diffing. Pure; no UI, the only I/O is load_file.
    cascade-layout: >
      Generic layered (Sugiyama-style) graph layout: sized nodes, ports,
      groups (lanes) stacked with edges routed through the gaps and free
      passages between them, orthogonal edge routing, collision-free
      labels, stability from the previous layout, pins. Knows nothing
      about Cascade.
    cascade-sim: >
      Scenario files and the simulator: instances, one global FIFO queue of
      events, controller fires and expired timers, guards and choice points,
      sets/with, subscriptions, a virtual clock, target selectors, traces
      with cause links; replays race candidates between two controller
      rules with the contested fires swapped. One
      steppable engine serves batch runs and the interactive PlaySession
      (rewind, branches, replay after edits, save as a scenario).
      Exploration enumerates every delivery ordering (and optionally every
      choice and timer expiry) into distinct outcomes with replayable
      witnesses.
    cascade-scene: >
      Model + view state → Scene, a backend-neutral display list with hit
      targets, for all four views. Owns the visual encoding (Okabe-Ito hues,
      shapes, dashes, emphasis/dimming, badges, diff decorations, edit-mode
      wiring, connect and subscribe handles, play markers, timers and
      highlights; trigger kinds, choices in italics, Notify/Delay edges), the
      cascade:// view link format, the pins sidecar, and SVG/PNG export.
    cascade-interop: >
      XState v5 and SCXML import through a shared statechart IR (names
      sanitized, entry/exit emits attributed, a routing controller and
      external sources synthesized so imports resolve; delays become
      timers, assigns `sets`); SCXML (timers as delayed sends, subscriptions,
      assign/param), Mermaid (structure and causal, notify/delay edges), P
      skeleton (typed fields, translated guards) and YAML export;
      comment-preserving YAML patches; reading a file at a git revision for
      diff mode.
    cascade-cli: >
      The `cascade` binary: check (CI exit codes), render, export, import,
      diff, simulate, explore (every ordering of a scenario as outcomes),
      open, new (blank definition opened in Build mode).
    cascade-app: >
      The `cascade-app` GPUI binary: paints scenes, pan/zoom, selection, cone
      tracing, search, legend/entity filter, findings panel, trace and matrix
      interaction, the causal view's group-by-machine toggle (G), the
      structure view's pills toggle (P, remembered in a settings file), live
      reload, click-to-source, diff mode, pin dragging; a
      workbench with Build mode (edit ops with undo saved to the file in
      place, drag-to-connect including subscribe offers, inspector with typed
      fields, trigger kinds, guards/conditions with live parse and type
      feedback, sets and emit with rows, new files) and Play mode
      (interactive session with instances, trigger palette, clock with armed
      timers and Advance, choice prompt, per-consumer queue delivery,
      timeline, scenarios).

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
    Play mode drives a cascade_sim::PlaySession and passes its PlayOverlay
    to the scene; the Explore panel runs cascade_sim::explore_from_watched
    on a copy of the session on the background executor and applies a
    chosen outcome's witness back to the session as a timeline branch.
    Element identity crosses every boundary as ElementKey (stable) rather
    than typed ids (valid for one Model only).

Features Index:
  definition_format:
    description: YAML schema (typed fields and payloads, command/subscription/timer triggers, guards, conditions, sets, emit with), parser with source spans, name resolution and type checking, the resolved Model and stable element keys.
    entry_points: [cascade_core::load_str, cascade_core::load_file, cascade_core::parse_definition, cascade_core::resolve]
    depends_on: []
    doc: docs/features/definition-format.md
  expressions:
    description: >
      The typed expression language of guards (expression, `?choice`,
      `else`), rule conditions, selector and subscription filters and
      assignments; field types; durations; canonical text that round-trips;
      evaluation over string values; sound exclusivity (`overlap`) and
      coverage reasoning; resolver type checking.
    entry_points: [cascade_core::expr::parse_expr, cascade_core::expr::parse_guard, cascade_core::expr::parse_condition, cascade_core::expr::eval, cascade_core::expr::overlap, cascade_core::expr::coverage, cascade_core::expr::parse_duration, cascade_core::Model::types_for]
    depends_on: [definition_format]
    doc: docs/features/expressions.md
  semantics_contracts:
    description: >
      The shared design of the semantics phase (expressions, exploration,
      subscriptions and timers) and its workstreams; wave 1 (the core
      language) and wave 2 (simulator, analysis, interop, scenes, app) have
      landed and been integrated, with the glue between them recorded in
      the contract's as-landed sections. Exploration (wave 3) has landed in
      the simulator, the CLI and the app's Explore panel.
    entry_points: []
    depends_on: [expressions, definition_format, causal_graph, simulator]
    doc: docs/features/semantics-contracts.md
  causal_graph:
    description: The derived causal graph (including subscription Notify and timer Delay edges), causal depth, forward/backward cones with hop limits, and path queries.
    entry_points: [cascade_core::CausalGraph::build, CausalGraph::cone, CausalGraph::paths_between, CausalGraph::depths]
    depends_on: [definition_format]
    doc: docs/features/causal-graph.md
  static_analysis:
    description: >
      The seven checks (dead external triggers count as invalid fires) plus
      state-dependent fire and uncovered-guard notes, sorted
      deterministically. Guards, filters and rule conditions are compared
      with the expression solver (overlap/coverage); subscriptions and timers
      take part in races, reachability and cycles. `cascade check` for CI
      with text/JSON reports and exit codes; the pinned shop example.
    entry_points: [cascade_core::analyze, cascade_core::analysis::has_errors, cascade check]
    depends_on: [definition_format, causal_graph, expressions]
    doc: docs/features/static-analysis.md
  layered_layout:
    description: >
      Sugiyama-style layered layout: constrained network-simplex layering
      with cycle breaking, port-aware crossing minimisation, L1 coordinate
      placement, orthogonal channel routing with track assignment, lanes
      stacked with edges between them run through gaps and free passages
      (side corridors only as a last resort), optional cross-lane
      alignment, optional shared layers (one layering and one column
      position per layer across every lane, edges between lanes always
      heading with the flow through zoned channels and reserved
      verticals), collision-free labels, stability from the previous
      layout, pins with an obstacle router.
    entry_points: [cascade_layout::layout, cascade_layout::metrics, cascade_layout::LayoutOptions::shared_layers]
    depends_on: []
    doc: docs/features/layered-layout.md
  simulator:
    description: >
      Scenario files (parse, validate, discover, write; manual queue steps,
      mid-run create/remove, end: pause), FIFO simulation with selectors,
      spawn, history and payloads, traces with cause links and stable
      lifelines, race candidates replayed in both orders, and the play
      session (actions, pending queue, available fires, seek, branches,
      replay, to_scenario) on the same engine. Guards, `where` filters and
      `when:` conditions are evaluated with declared types; `?choice`
      guards and conditions are choice points decided by pending decisions
      (`choose`, `PlayAction::Choose`) or the first option; `sets` and emit
      `with` shape fields and payloads; events can be delivered to one
      consumer at a time; scenario values are checked against declared
      types; `state_key`, `choice_points` and `choice_points_of` (any
      action, e.g. an external fire) support exploration and the app's
      choice prompt. Subscription races are not replayed:
      `race_replay_note` says why and callers fall back to the as-queued
      run.
    entry_points: [cascade_sim::parse_scenario, cascade_sim::validate, cascade_sim::discover_scenarios, cascade_sim::simulate, cascade_sim::simulate_run, cascade_sim::race_orderings, cascade_sim::race_replay_note, cascade_sim::PlaySession, cascade_sim::PlaySession::choice_points_of, cascade_sim::scenario_to_yaml, cascade simulate, cascade simulate --interactive]
    depends_on: [definition_format, expressions, static_analysis]
    doc: docs/features/simulator.md
  timers_and_subscriptions:
    description: >
      Run-time subscriptions and timers in the simulator: events delivered
      to subscribing instances first (Notify steps, ignored when nothing is
      enabled), per-consumer delivery with consumers_left, timers armed on
      entry and cancelled on exit, a virtual clock advanced by `wait` /
      `PlayAction::Advance` (Wait and TimerExpired steps, Timer queue
      items), `now()` and `timers()`.
    entry_points: [cascade_sim::PlaySession::now, cascade_sim::PlaySession::timers, cascade_sim::PlayAction::Advance, "cascade_sim::PlayAction::Step { consumer }", cascade simulate --interactive (wait, timers, now)]
    depends_on: [simulator, definition_format, expressions]
    doc: docs/features/timers-and-subscriptions.md
  exploration:
    description: >
      Every delivery ordering a scenario (or a play position) allows, under a
      per-consumer or unordered model, optionally branching on every choice
      option and expiring timers at any time: DFS over simulator states with
      a visited set, canonical queue keys and sleep-set partial-order
      reduction; distinct outcomes (instances, timers left, anomalies) with
      the shortest witness found, which replays in a PlaySession. The CLI
      (`cascade explore`) and the app's Explore panel (watched, cancellable
      runs; outcomes opened as timeline branches) host it.
    entry_points: [cascade_sim::explore, cascade_sim::explore_from, cascade_sim::explore_from_watched, cascade_sim::replay_witness, cascade explore, cascade-app Explore panel (E)]
    depends_on: [simulator, timers_and_subscriptions, build_and_play]
    doc: docs/features/exploration.md
  view_scenes:
    description: Scene builders for the causal, structure, trace and matrix views; causal lanes (group by machine, `lanes=1`: one lane per machine on shared causal columns, every forward arrow pointing right); the interaction model (selection, cones, path queries, search, hide stubs); layout caching; view links; pins sidecar; SVG/PNG export; build-mode drawing (the structure view's wiring in gutters between the lanes, and connect handles); arrow mode (`pills=0`: the structure view draws each transition as one labelled state → state arrow through an invisible layout junction, wires attaching to the arrow with a dot) and play overlays (instance markers, armed timers, active and pending items). Subscriptions, timers, choices and `sets` are drawn in every view: pills/arrows say what triggers them (`capture`, `on Event` + `where`, `◷ 30s`), choice guards are italic, `Notify` (event → subscribed transition) and `Delay` (transition → timer it arms) edges, subscription links across lanes and as build-canvas wires with a subscribe handle on every event tag, the five new trace step kinds, and the matrix counting subscriptions as links.
    entry_points: [cascade_scene::SceneBuilder::build, cascade_scene::HitTarget::SubscribeHandle, cascade_scene::PlayTimer, cascade_scene::emphasis::Interaction, cascade_scene::ViewState::to_link, cascade_scene::ViewState::group_by_machine, cascade_scene::ViewState::transition_pills, cascade_scene::Scene::hit_test_arrows, cascade_scene::to_svg, cascade_scene::to_png, cascade render]
    depends_on: [definition_format, causal_graph, static_analysis, layered_layout, simulator]
    doc: docs/features/view-scenes.md
  interop_and_diff:
    description: >
      XState/SCXML import with warnings, SCXML/Mermaid (structure and causal)/P/YAML export,
      including guards, timers, subscriptions, sets, emit payloads and typed
      fields; comment-preserving YAML patches for edit ops (span index over
      the file, entry-by-entry edits of keyed mappings and emits, rewrite
      fallback flagged), model diff, ghost merge for diff mode, and git
      revision loading.
    entry_points: [cascade_interop::import, cascade_interop::import_with, cascade_interop::export, cascade_interop::to_yaml, cascade_interop::patch_text, cascade_interop::read_at_rev, cascade_core::diff::diff_models, cascade_core::diff::merge_for_display, cascade export, cascade import, cascade diff]
    depends_on: [definition_format, expressions, causal_graph]
    doc: docs/features/interop-and-diff.md
  native_app:
    description: >
      The GPUI desktop app: paints scenes with pan/zoom/fit and culling, shared
      selection, cone tracing with a depth stepper, path queries, legend
      entity filter, findings panel, fuzzy search, trace scenario/race picker,
      matrix drill-down, click-to-source, pin dragging, diff mode, view links
      via the clipboard, light/dark theme, the structure view's pills/arrows
      toggle (P, remembered in $XDG_CONFIG_HOME/cascade/settings.json), and
      live reload that keeps the last good model on screen. View/Build/Play modes: Build edits the definition
      (toolbar, drag-to-connect from handles or, with pills off, from
      anywhere within 8 px of a transition arrow, inspector, undo/redo,
      own-write-aware saves, `--new`; event tags' subscribe handles drag onto
      transitions to subscribe them); Play drives an interactive simulator session with an overlay
      (instance markers, armed timer countdowns), a clock (T advances to the next timer), a choice
      prompt before steps and external fires that need decisions, and per-consumer delivery from
      the queue. Choice labels paint in italics; subscription races open as the as-queued trace
      with a note. The Explore panel (E, Play mode) explores every ordering from the session's
      position off the UI thread, lists outcomes, previews one on the canvas and opens it as a
      timeline branch.
    entry_points: [cascade-app, cascade-app --new, cascade new, cascade open, cascade_app::workspace::Workspace, cascade_app::settings, cascade_app::build::pick]
    depends_on: [definition_format, causal_graph, view_scenes, static_analysis, simulator, interop_and_diff]
    doc: docs/features/native-app.md
  build_and_play:
    description: >
      Build systems in the app (typed edit ops with exact inverses for undo,
      rename propagation and cascading removal, saved to the YAML file in
      place keeping comments) and play them (interactive simulator
      session with instances, trigger palette, queue stepping, branchable
      timeline, record as scenario). The app's inspector edits typed fields,
      trigger kinds (command, subscription with `where`, timer), guards and
      conditions with live type feedback, `sets:` and emit `with:` rows;
      drag-to-connect (from event tags' subscribe handles too) offers
      subscriptions; play shows the clock, armed timers (also on the
      canvas), a choice prompt (steps and external fires), per-consumer
      delivery and the Explore panel (outcomes of every ordering, opened as
      branches).
    entry_points: [cascade_core::edit::apply, cascade_core::edit::locate_transition, cascade_interop::patch_text, cascade_sim::PlaySession, cascade_scene::PlayOverlay, cascade-app build/play modes, cascade_app::build::inspector::check, cascade_app::build::subscribe, cascade_app::play::choices]
    depends_on: [definition_format, simulator, view_scenes, interop_and_diff, native_app]
    doc: docs/features/build-and-play.md
  readability:
    description: >
      Scene readability metrics (crossings, length, bends, label overlaps,
      corridor use, leftward forward edges) with a report over the examples
      (causal lanes included) and asserted targets,
      and the placement (wiring in gutters between the lanes, short fire
      labels said once) and routing work that improves them; arrow mode
      measured against pill mode.
    entry_points: [cascade_scene::metrics::measure, cascade-scene tests/readability.rs, cascade-scene tests/readability_targets.rs, cascade_scene::views::structure::gutters::assign (crate), cascade_layout::LayoutOptions::align_across_groups]
    depends_on: [view_scenes, layered_layout, build_and_play]
    doc: docs/features/readability.md
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
