# Build and play

Turns the app from a viewer into a workbench: build a system of machines,
controllers and sources on the canvas, then play it: create instances,
fire triggers, step the queue, rewind and branch, and save what happened as
a scenario.

## Scope

- Structural edits with undo/redo, addressed by names so they survive
  reloads (`cascade_core::edit`).
- Saving edits to the YAML file in place, keeping comments and formatting
  (`cascade_interop::patch`). The file stays the source of truth: hand
  edits reload into the app as before.
- An interactive simulator session with a recorded, branchable timeline
  (`cascade_sim::session`), sharing the batch simulator's semantics.
- Build-mode and play-mode drawing (`cascade_scene::play`): wiring and
  connect handles in the structure view, instance markers and
  active/pending highlights in the causal and structure views.
- The app's build and play modes (`cascade-app`).

## Non-scope

- Free-form manual layout: layout stays automatic, dragging still pins.
- Evaluating guards or conditions during play (they remain free text).
- Collaborative or multi-file editing.

## Data and control flow

```text
Build mode
  gesture / inspector ──▶ EditOp ──▶ edit::apply(definition) ──▶ Applied { definition, inverse, touched }
                                 └──▶ patch::patch_text(file text) ──▶ write file ──▶ reload (own write recognised)
  undo = apply(inverse) the same way; redo = re-apply

Play mode
  palette / queue panel / timeline ──▶ PlayAction ──▶ PlaySession::apply ──▶ trace, instances, pending
  PlaySession ──▶ PlayOverlay { markers, active, pending } ──▶ SceneInput::play ──▶ scene
  after an edit: PlaySession::replay(new model) (stops at the first action that no longer applies)
  save: PlaySession::to_scenario ──▶ scenario_to_yaml ──▶ scenarios/<name>.yaml
```

## Contracts and ownership

| Workstream (branch) | Owns | Implements |
| --- | --- | --- |
| `feat/edit-ops` | `cascade-core/src/edit/` | `apply`, `locate_transition` (inverse ops, rename propagation, cascading removal, validity) |
| `feat/yaml-patch` | `cascade-interop/src/patch/` | `patch_text` (span-based surgical edits; full rewrite fallback flagged) |
| `feat/sim-session` | `cascade-sim/src/session/`, engine refactor | `PlaySession`, `scenario_to_yaml`; batch `simulate` built on the session; scenario format extended for manual queue choices |
| `feat/scene-edit-play` | `cascade-scene` (`play.rs` consumers, structure view wiring band, handles) | `SceneMode::Edit`, `PlayOverlay` drawing, `HitTarget::ConnectHandle` |
| `feat/app-build-play` | `cascade-app` | Build mode (toolbar, drag-to-connect, inspector, delete, undo/redo, save) and play mode (instances, trigger palette, queue, timeline, record) |

Every stub returns a typed `NotImplemented` error or an empty result
until its workstream lands, so the app can be built against the contracts
in parallel.

## Files

| File | Role | Key exports |
| --- | --- | --- |
| `crates/cascade-core/src/edit/mod.rs` | Edit ops on definitions | `EditOp`, `Applied`, `EditError`, `apply`, `locate_transition`, `fresh_name` |
| `crates/cascade-interop/src/patch/mod.rs` | Comment-preserving persistence | `patch_text`, `Patched`, `PatchError` |
| `crates/cascade-sim/src/session/mod.rs` | Interactive simulator | `PlaySession`, `PlayAction`, `PendingItem`, `InstanceState`, `AvailableFire`, `Timeline`, `Branch`, `scenario_to_yaml` |
| `crates/cascade-scene/src/play.rs` | Build/play drawing inputs | `SceneMode`, `PlayOverlay`, `PlayMarker` |
| `crates/cascade-scene/src/scene.rs` | New hit target | `HitTarget::ConnectHandle` |

## Invariants and constraints

- `edit::apply` never returns a definition that fails to resolve; a
  rejected op changes nothing.
- `apply(apply(d, op).definition, inverse) == d` (ignoring spans).
- `parse(patch_text(text, op).text) == apply(parse(text), op).definition`
  (ignoring spans); comments survive unless `rewritten` is set.
- Play actions and edit ops name elements, never typed ids, so both
  survive model reloads.
- A session's trace equals the batch simulator's for the same actions;
  `to_scenario` then `from_scenario` reproduces the trace exactly.

## Saving edits (YAML patch)

`cascade_interop::patch_text(text, op)` applies an `EditOp` to the
definition file's text and returns the new text. Everything the op does
not touch stays byte for byte: comments, blank lines, key order, quoting,
flow or block style and indentation. Only when a surgical patch is
impossible does it re-emit the file with `to_yaml` and set
`rewritten: true`, so the app can warn that comments were dropped.

### Flow

```text
text ──parse_definition──▶ Definition ──edit::apply──▶ expected definition (or NotImplemented)
text ──ops::apply(op)──▶ patched text        (steps: Doc::parse ─▶ splices ─▶ apply, repeated)
patched text ──parse──▶ compare with expected, ignoring spans
   equal     ─▶ Patched { text, rewritten: false }
   different ─▶ Patched { to_yaml(expected), rewritten: true }
```

1. The original text must parse (`PatchError::Unparseable` otherwise), and
   `edit::apply` decides whether the op is valid (`PatchError::Edit`).
2. The op is applied to the text as one or more *steps*. Each step parses
   the current text into a span index (`Doc`), computes non-overlapping
   splices (byte range → replacement) and applies them. Ops with dependent
   edits (cascading removals, form conversions, field-by-field updates)
   run several steps, re-parsing in between. A `Batch` applies its ops in
   order without validating in between.
3. The patched text is parsed and compared with `apply`'s definition,
   ignoring spans. Any difference, or any step that cannot be done in
   place, falls back to the rewrite.

**Before `edit::apply` lands** (it returns `NotImplemented` on this
branch), the comparison is skipped: the patched text must resolve, else
the op is reported as `Edit(Invalid)`, and an op that cannot be patched in
place returns `PatchError::NotImplemented`, since there is nothing to
rewrite from. No code change is needed when `apply` lands; the check turns
on by itself. The tests carry their own span-free expectation for every
case (the parsed original with the edit applied by hand, compared through
`to_yaml`), and were also run against the `feat/edit-ops` implementation:
every patch agreed with `apply`.

### Span strategy

`Doc` is a private index built from saphyr's marked nodes
(`MarkedYamlOwned`): every node with exact byte offsets (saphyr counts
characters, so offsets are converted), scalar quoting style, collection
style, and the offset of each block sequence item's `-`. saphyr's end
markers are unreliable, so ends are recomputed:

- quoted scalars end at their closing quote (saphyr includes trailing
  blanks and comments);
- flow collections end after their closing bracket, found by scanning
  from the last child over blanks, commas and comments;
- block collections end where their last descendant ends;
- an implicit null (`key:`) sits just past its colon.

A line table answers the rest. A block entry *owns*:

- the comment lines directly above it at its own column (never the
  file's header comment block);
- its own lines up to where its content ends;
- comment lines below that are indented deeper than the entry, up to the
  next line at its column or shallower.

Removing an entry deletes exactly the lines it owns plus one side of its
blank-line separation. Inserting mirrors that, so adding then removing an
entry restores the text byte for byte. Core's public API is unchanged; the
only change outside `patch/` makes `yaml::quote` visible within the crate.

### Style rules

- **Scalars** that change keep their quoting style (`'…'` stays single
  quoted, `"…"` double quoted, plain stays plain when it can). New names
  are plain unless YAML would read them differently. New free text
  (`guard:`, `when:`) is double-quoted when most of the file's free text
  is.
- **Transitions** are written like their neighbours. Among one-line flow
  mappings a new row is `{ from: …, to: …, on: … }`, padded to the
  neighbours' `to:`, `on:` and extra-key columns when every row shares
  them (the order-fulfillment example), single-spaced otherwise (the shop).
  Among block mappings (the majority style) it is a block mapping with
  the neighbours' key indentation. In a flow list it joins the list.
- **Aligned rows stay aligned**: when a value inside a padded flow row
  changes (a rename), the padding after it shrinks or grows so the next
  key keeps its column.
- **States** go into the collection they live in. A plain state joins a
  flow list `states: [a, b]`. A state with a body turns the flow list into
  a block list, only then. Inside a flow mapping, where block form is
  impossible, it is written inline as `{ name: { kind: final } }`. Block
  lists get `- name`, `- name: { kind: final }` or a compound block body;
  mapping-form lists get `name: {}` entries. A plain state that gets a
  child becomes `- name:` with a body; a body that loses its last key
  collapses back to a plain name.
- **Keys** that appear go before the first present key that follows them
  in canonical order (`to_yaml`'s order), else after the last one that
  precedes them. This keeps an author's own order (the shop puts
  `fields:` before `initial:`).
- **Sections** (`events:`, `controllers:`, `external:`) are created in
  canonical order (system, machines, events, controllers, external),
  separated by a blank line when the file's sections are. An empty
  section left by a removal is removed. `controllers: {}` becomes a block
  section when a controller is added. A new `system:` line opens the file
  after its header comments.
- **Indentation** follows the surrounding block: the column of the
  siblings for new entries, and for new nested blocks the file's
  indentation unit and sequence offset (detected from the first nested
  mapping and the first block sequence under a key).
- **Siblings** separated by blank lines (machines, controllers) get a
  blank line around a new entry.
- **Lists of names** (`fields`, `emits`, `from`, `payload`, external
  triggers) change item by item, keeping unchanged items' text. A single
  name becomes a flow list when a second one is added.
- **Rules** follow their neighbours (block or flow mappings). A handler
  written as one flow rule mapping becomes a one-item list when a rule is
  added. A handler that loses its last rule is `Event: []`.
- **Renames** replace every reference occurrence. Machines: the key,
  `fire:` refs, target selector machine words (keeping the selector's
  spacing) and external triggers. States: the name, the parent's
  `initial:`, and every state reference (machine `initial:`, transition
  `from`/`to`), keeping bare names bare unless they would become
  ambiguous or captured, then written as full paths. Events: the
  declaration, `emits:` and handler keys. Adding a state rewrites bare
  references it would capture as full paths.
- **A payload change** (`Batch[RemoveEventDeclaration(e),
  DeclareEvent(e', same index)]`) is patched in place, keeping the
  declaration's comments, when other declarations remain. Removing the
  only declaration makes the file lenient, so re-declaring then declares
  every event in use, as `apply` does.

### Fallback cases

These are left to the rewrite:

- nodes with YAML tags (`!tag value`);
- editing a block scalar (`|`, `>`) or a multi-line plain scalar;
- restructuring a flow collection that contains comments (turning it
  into a block list, removing an item across a comment);
- adding a rule to a handler written as a single block mapping;
- declaring an event with a payload in a list-form `events:` whose items
  are not indented under the key.

On every example and fixture, a matrix of about 670 valid ops of every
kind (`tests/patch_invariants.rs`) never falls back. A broader
cross-check of 2164 valid candidate ops from `feat/edit-ops` (including
batches) also agreed with `apply` in every case, with no rewrite. Undoing
each of them by patching `apply`'s inverse op restored the original text
byte for byte in about 89% of cases. The rest differ only in style that
the inverse cannot know: comments of removed entries, a flow list that
became a block list, removed elements re-rendered in canonical style.

### Invariants

- `parse(patch_text(text, op).text) == apply(parse(text), op).definition`,
  ignoring spans. It is checked on every call once `apply` exists, and a
  mismatch rewrites.
- Unless `rewritten` is set, every comment survives except those owned by
  removed entries, and text outside the edited entries is unchanged.
- Adding then removing an entry restores the text byte for byte (tested
  for machines, states, transitions, events, controllers, handlers, rules,
  external sources, renames and the system name on every fixture). One
  exception: `controllers: {}` stays a block section.
- Patching never parses or validates intermediate batch steps.

### Files

| File | Role | Key exports |
| --- | --- | --- |
| `crates/cascade-interop/src/patch/mod.rs` | Entry point, verification against `edit::apply`, rewrite fallback | `patch_text`, `Patched`, `PatchError` |
| `crates/cascade-interop/src/patch/doc/` | Span index over the text (`build.rs`: saphyr nodes → byte ranges; `lines.rs`: line table) | `Doc`, `Node`, `Kind`, `Scalar`, `ScalarStyle`, `Lines` (crate) |
| `crates/cascade-interop/src/patch/block.rs` | Block entry regions (comments, blank-line separation), removal and insertion | `region`, `remove`, `insert`, `separated` (crate) |
| `crates/cascade-interop/src/patch/collection.rs` | Generic edits: flow insert/remove, set or remove a key, reconcile a list of names, flow → block conversion, alignment-keeping scalar replacement | `set_key`, `set_scalar_key`, `set_text_key`, `set_names`, `replace_scalar`, `replace_value`, `remove_child` (crate) |
| `crates/cascade-interop/src/patch/render/` | New text in the file's style: detected `Style`, flow rows with `RowLayout`, states, machines, controllers, rules; style-keeping scalars | `Style`, `Block`, `RowLayout`, `restyle`, `free_text` (crate) |
| `crates/cascade-interop/src/patch/ops/` | One module per op group (`machines`, `states`, `transitions`, `events` with external sources, `controllers`), navigation (`nav`), state references (`refs`) | `apply` (crate) |
| `crates/cascade-interop/src/patch/splice.rs` | Byte-range splices applied in one pass | `Splice`, `apply` (crate) |
| `crates/cascade-interop/src/patch/verify.rs` | Span-free definition comparison | `same_definition` (crate) |
| `crates/cascade-interop/src/patch/error.rs` | Why a surgical patch failed | `SurgeryError`, `Unsupported` (crate) |
| `crates/cascade-interop/tests/patch_*.rs`, `tests/patch_common/`, `tests/fixtures/patch/` | Golden line diffs per op on the examples and nested, flow and block (4-space) fixtures; invariants matrix; fallback cases | — |
