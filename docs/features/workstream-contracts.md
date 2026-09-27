# Workstream contracts

The foundation (`feat/core-foundation`) fixes the types every crate exposes
and ships a working stub behind each function a later workstream
implements. Workstreams build against these signatures in parallel. A
workstream may extend a contract (new fields, new functions) but must not
break a signature another workstream uses without noting it in its PR.

## Scope

- Which workstream owns which crate, file and stub.
- The cross-crate interfaces each workstream relies on.

## Non-scope

- How each workstream implements its part (see each feature doc).

## Ownership

| Workstream (branch) | Owns | Replaces stub(s) | Relies on |
| --- | --- | --- | --- |
| `feat/static-analysis` | `cascade-core/src/analysis/`, `cascade-cli/src/commands/check.rs`, `examples/shop/` | `cascade_core::analyze` | `Model`, `CausalGraph` |
| `feat/layered-layout` | `cascade-layout/src/engine/`, `cascade-layout/src/metrics.rs` | `cascade_layout::layout` | nothing |
| `feat/simulator` | `cascade-sim/`, `cascade-cli/src/commands/simulate.rs` | `parse_scenario`, `simulate`, `race_orderings` | `Model`, `FindingDetail::RaceCandidate` |
| `feat/interop-and-diff` | `cascade-interop/`, `cascade-core/src/diff.rs`, CLI `export`/`import`/`diff` | `import`, `export`, `read_at_rev`, `diff_models`, `merge_for_display` | `Definition`, `Model`, `ElementKey` |
| `feat/view-scenes` | `cascade-scene/` (except `pins.rs`/`view_state.rs` shapes), CLI `render` | `SceneBuilder::build` for all views, `machine_styles` domains, `to_svg`, `to_png` | `layout`, `Trace`, `Finding`, `ModelDiff` |
| `feat/gpui-app` | `cascade-app/` | the placeholder `main` | `Scene`, `SceneBuilder`, `ViewState`, `LayoutSidecar`, `analyze`, `search`, `simulate`, `read_at_rev`, `merge_for_display` |

## Fixed interfaces

- **Element identity:** `ElementKey` (string form in `key.rs`) is the only
  identity that crosses a model reload, a crate boundary into the UI, the
  view link, the pins sidecar or a diff.
- **Findings:** `Finding { severity, detail: FindingDetail, message }`.
  `FindingDetail::subjects()` gives the elements to badge and `primary()`
  the element to focus. Match on `FindingDetail` through these methods (or
  with a wildcard arm): `feat/static-analysis` adds
  `DeadExternalTrigger { source, trigger }` (reported under
  `Check::InvalidFire`).
- **Layout:** `LayoutGraph` (validated on insert) + `LayoutOptions` +
  `LayoutHints { previous, pins }` → `LayoutResult` (node rects with
  layer/order, edge polylines with `reversed`, group rects, bounds);
  `LayoutResult::to_previous` feeds the next run.
- **Traces:** `Trace { lifelines, steps (with cause links), final_states }`;
  `RaceTraces { as_queued, swapped }`.
- **Scenes:** `Scene { lanes, edges, nodes, overlays, notes, bounds }` with
  every visual property baked in; `Scene::hit_test` and `Scene::locate`.
  Paint order: lanes, `Layer::Under` overlays, edges, nodes, `Layer::Over`
  overlays.
- **View state:** `ViewState` and its `cascade://` link
  (`to_link`/`from_link`).
- **Pins:** `LayoutSidecar` JSON at `sidecar_path(definition)`.
- **Text:** views size text with `TextMeasure`; hosts draw with a monospace
  font so `MonoMeasure` sizes match.

## Integration

`test/integration` merges every workstream and holds the glue that needs
more than one of them: `cascade render` reads the `diff=` link parameter
through `cascade_interop::read_at_rev` and `merge_for_display`, and an
end-to-end test checks the M3 guarantee that an edit to one transition
moves no unrelated node (`crates/cascade-scene/tests/edit_stability.rs`).
Every stub listed above has since been replaced; the remaining
`NotImplemented`-style errors were removed with them.

## Expected sibling conflicts

Every workstream adds dependencies, so `Cargo.lock` and crate `Cargo.toml`
files conflict trivially. `docs/OVERVIEW.md` conflicts where two branches
edit the same feature entry. The CLI command modules are split per command
so branches do not share a file.

## Invariants and constraints

- Stubs never panicked; they returned empty results or a typed
  `NotImplemented` error until their workstream replaced them.
- Headless crates never depend on `gpui`; only `cascade-app` does, and it is
  excluded from the workspace's default members.
