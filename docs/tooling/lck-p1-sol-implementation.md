# LCK P1 implementation checkpoint

Date: 2026-10-26. Production baseline: `008ccaf4e55f99627a5c77d2c49b32cc99a22ae2`. All P1 changes remain uncommitted and unstaged. Astra's allocation and trigger-dependency improvements are retained.

## Implemented features

### Explicit recipe persistence and history

WorldModel now carries native collision source identities and per-reference recipes. Original identities use the source scene, MAP PROT entry, native cell and descriptor ID; recipes additionally preserve the complete original descriptor, MAP SHA-256 and optional decompressed MAN SHA-256. Source inventory verifies native mesh selector, position, orientation and placement before exposing that reference for authoring. Current raster indices are adapter keys only and are never persisted as collision owner identities.

Created references use the existing persisted editor UID. Each duplicate receives its own recipe value and retains the original source provenance. Ctrl+D from an edited copy inherits that copy's current recipe or deliberate absence, rather than reviving the original object's recipe. Repository drops inherit their actual base source, not another selected object. Deleting a copy removes only its recipe. Recipe updates and copies that would exceed the existing scene-file limit fail without changing the previously valid document.

`COLLISION` records extend the existing `.lckscene` overlay with one versioned JSON assignment per line. Existing overlays without recipes continue to load. Unknown recipe versions, invalid evidence, duplicate owners, missing targets, stale sources and invalid sub-cell coordinates reject restoration. Native observations remain distinct from explicit user authorship through save/load. A native observation does not authorize wall compilation.

Recipe edits use the same before/after scene-overlay history as the existing editor. Transactional restoration includes recipes, created instances and donor references. Undo/redo can restore deleted recipes and copies. The compiler does not infer any owner set from bounds, mesh faces or adjacent native walls.

### In-memory contribution compiler

The compiler accepts an immutable complete native MAP and explicit recipe contributions, ordered by stable identities. It recomputes the union of authored additions on every revision. It never uses the previous compiled map as an implicit source baseline.

- Native wall bits, floor nibbles, descriptors, object references and trigger bytes are preserved.
- Each authored sub-cell is an integer X/Z pair in 0..255. Each sub-cell represents the native 64-unit wall-grid resolution.
- Overlapping authored contributors union their solid bits. Moving or removing one contribution leaves other contributors and every native baseline bit intact.
- X/Z movement must be an exact multiple of 64. Boundary overflow, nonfinite values, Y movement, rotation and scale fail explicitly.
- Duplicate owners, stale MAP/descriptor/reference preconditions and native-observed ownership claims fail explicitly.
- It adds authored walls only. It does not transfer or remove existing native walls, author actor colliders, create walkable surfaces or interpret script-conditioned wall clearing.

This is not a native ownership solver. Existing native walls remain an unowned baseline. Native export explicitly rejects documents containing recipes, preventing successful export that silently omits collision intent. The new compiler is available only for in-memory validation and synthetic tests.

### Read-only selection dependency report

The report uses Astra's primary/fallback trigger resolver and the actual native load window, preserving first-match semantics. It exposes source resource fingerprints, descriptor/grid/anchor data, record-local collider offsets, relevant trigger rows, resolved flat MAN record, partition/header/animation information and bounded script instruction inventories.

Script scans report observed incoming/outgoing cross-context references and wall-write instruction sites. Every record stops at its first decode error; the reporter never resynchronizes through unknown bytes and claims a complete scan. Missing records and incomplete scans are explicit. A linear instruction inventory does not establish control-flow closure, resolve helper/dynamic targets or prove safe relocation. No MAN relocation is implemented.

## How to use in Studio

1. Select a World Object and open its existing Properties window.
2. Expand **Native dependencies (read only)** and press **Read native dependencies**. The report is computed on demand and cached per selected source revision.
3. Expand **Authored collision recipe (in memory)**. Enter explicitly chosen native sub-cell pairs, one `X,Z` pair per line, and a printable authorship/evidence note. Coordinates refer to the object's original source pose, not its current moved transform. No cells are automatically suggested from geometry.
4. Press **Save authored recipe**. This updates the editor document and its undo history. Use the existing Save LCK scene action for disk persistence. **Reload saved recipe** reloads the currently applied document recipe into the text fields; unsaved text fields are not document state.
5. **Validate saved contributions in memory** validates current applied recipes and transforms and reports changed wall-byte count. It writes no file and does not change viewport collision rendering or native export eligibility.

Unavailable/foreign/NPC/terrain sources do not receive fabricated native World Object provenance. General cross-cell recipe attachment and walkable-surface authoring remain unsupported.

## Exact files changed during this Sol run

| File | Change |
|---|---|
| `Cargo.lock` | Studio references existing workspace engine-field and SHA-256 dependencies; no package version changes |
| `crates/studio/Cargo.toml` | Add direct workspace `legaia-engine-field` and `sha2` dependencies |
| `crates/studio/src/app.rs` | Register modules; add Properties sections; route recipe edits through history and selected-copy inheritance |
| `crates/studio/src/app/collision_recipes.rs` | New schema, source inventory, editing helpers, compiler, authoring panel and synthetic tests |
| `crates/studio/src/app/native_dependency_report.rs` | New read-only report, scan coverage and Properties display |
| `crates/studio/src/app/native_dependencies.rs` | Retain Astra's resolver; expose it within app for shared report use |
| `crates/studio/src/app/native_export_verification.rs` | Retain Astra's opt-in harness; add read-only selected-object report validation |
| `crates/studio/src/app/ps1_export.rs` | Retain Astra's allocation changes; reject recipe-bearing native export explicitly; add omission regression and fixture field |
| `crates/studio/src/app/scene_instances.rs` | Recipe serialization/restoration, transactional validation and persistence/history regressions |
| `crates/studio/src/app/world_view.rs` | Source inventory, independent recipe inheritance, deletion and reset lifecycle; fixture initialization |
| `docs/tooling/lck-p1-sol-implementation.md` | This implementation/validation/checkpoint report |

Astra's existing untracked documents were read and retained unchanged:

- `docs/tooling/lck-p1-native-allocation-investigation.md`
- `docs/tooling/lck-p1-collision-and-binding-provenance.md`

## Validation evidence

- `cargo test -p legaia-studio --lib --locked`: **148 passed, 0 failed, 5 ignored**. Catalog filesystem tests require normal filesystem access. An initial sandboxed full-suite attempt had four Access denied catalog failures; the permitted full run passed.
- Focused collision and persistence tests passed while developing the change. Final full-suite coverage includes independent descriptor allocation, exact sealed-plan readback, trigger fallback protection, GUI/camera regression tests, scene rollback, recipe identity persistence across adapter-index changes, selected-copy inheritance, save/load, history restoration, stale provenance, unknown native ownership, overlap/movement/boundaries, native bit preservation and transactional scene-size-limit rejection.
- Read-only opt-in `selected_world_object_report_preserves_native_provenance`: **passed** against existing retail input. It resolved 35 native source identities and validated Vahn's House's record-0 binding, animation 1, unknown wall ownership and unchanged scene overlay. No IMG or report file was generated by this test.
- The remaining four real-disc opt-in tests were not rerun during this implementation. Their previous evidence remains in Astra's reports.
- Retail input SHA-256 remains `750dbcbadb70362feb3468467bbe7efbf3d151a414caa428c66c368d4b0b73e6`.
- No DuckStation activity, new test IMG, branch change, staging, commit or push. Manual interaction with the new Properties controls remains unexecuted; headless regressions and compilation do not establish emulator runtime behavior.

Build completion and final whitespace verification are recorded in the delivery response. Existing compiler warnings remain.

## Remaining blockers and Astra work

The immediate goal of a fully functional native duplicate is still blocked. Authored collision is additive and in-memory; it cannot identify or relocate the house area's unknown native walls. Complete bound-object duplication still needs a proven independent MAN context, trigger linkage, flat-reference rebasing and closure over incoming/outgoing scripts, coordinates, animation/resources and story-state behavior.

Tasks requiring Astra's native analysis:

1. Validate and close one minimal bound-object dependency graph, including helper/dynamic references and original instance interactions.
2. Establish a verified MAN allocation/rebasing strategy and native runtime resource/actor limits.
3. Define explicit native wall ownership/authoring acceptance and script-clear/set semantics; resolve shared consumers before permitting any baseline wall removal.
4. Specify a separate walkable-surface recipe/serializer for LUT corners, kind-2 records, height conflicts and scene variants.
5. Determine the first narrow real-disc capability that can safely consume a validated recipe. Add native readback and untouched-resource checks before a user-run emulator test.

Routine follow-ups suitable for Sol include richer grid authoring and visualization, structured dependency-report export, and additional schema/union compiler fixtures after Astra defines those native contracts. Current reports intentionally do not promote observed script writes into exclusive ownership.

## Recommended checkpoint

Review and checkpoint the complete P1 working increment together: the eleven files changed in this Sol run plus Astra's two investigation documents, **13 files total**. This retains Astra's allocator and dependency resolver with the persistence/compiler/report consumers and regressions. Exclude all historical backups, build outputs, retail images and unrelated untracked artifacts.

Suggested commit message: `LCK P1 - persist collision recipes and report native dependencies`

No commit is made. Native export remains guarded until Astra's remaining native contracts are proven and separately implemented.
