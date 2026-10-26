# LCK P0 and the native-reference milestone

Date: 2026-10-26. Preservation baseline: `e8a121ad0212539253b5a83f18c19edc66105938`.

## P0 implementation

- Global repository drops resolve as local only when the donor is the current scene. Geometry fingerprints never substitute a local material, palette, placement or collider for a foreign donor. Foreign assets retain the existing donor-aware rendering and provenance path.
- Export preparation has distinct local-export, cross-cell-export and collision-preview modes. Both export modes retain and validate original NPC edits. Preview-frame-only changes explicitly fail native export.
- `ps1_export::plan_export` binds the loaded resource index to the source archive by byte comparison, prepares all edits, orders allocations deterministically and compiles a `NativeExportPlan`. `compile_plan` executes MAP allocation, spawn/collider checks, story-variant checks, MAN serialization, bounded compression, descriptor updates, sector patching and readback/decompression verification in memory. Preflight and export invoke this same planner. The writer accepts only a plan and performs no second allocation or compilation.
- The plan owns its compiled image bytes. Output size must equal source size. Private fields prevent UI callers from manufacturing a partially validated plan. Preflight displays limitations for skipped auxiliary story states; selected-scene failure always blocks planning.
- Native wall bits have no recorded instance owner. Bounds are now only a conflict search area. Nearby unowned walls block affected edits; unknown bounds require a wall-free map. Foreign wall claims likewise block import. The wall/floor grid is preserved exactly. Proven record-local colliders remain supported; this is not a wall ownership solver.
- Script-bound MAP source moves/copies and movement outside supported spawn-anchor reach fail explicitly. No unrelated record or anchor is moved to create room.
- Full overlays and legacy offset layouts are applied to a cloned world and swapped only on success. Geometry, instances, donor references and donor VRAM survive failure. Immutable source bytes remain shared; decoded animation memoization may be shared, but document state is not published early.
- Cell loading retains the previous ready document until assembly and copied-cell overlay restoration both succeed. Camera reset, selection reset and history clearing occur only on successful cell replacement. Failed undo/redo restoration retains its history entries.

## Validation and practical limits

Studio unit suite: 130 passed. Tests include headless GUI/camera behavior, identical compiled bytes from repeated synthetic planning, allocated-record roundtrip, allocation exhaustion, stale resources, missing MAN rejection, overlapping/shared walls, late overlay failure, legacy layout rollback and failed scene/copy switching. Five older tests that expected inferred wall transplantation were replaced with ownership-protection assertions.

`cargo build -p legaia-studio --locked` passed. `cargo test -p legaia-studio --lib --locked` passed all 130 tests; `cargo test -p legaia-disc-patch --lib --locked` passed all 21 tests. `git diff --check` passed. Existing float-literal, deprecated API and dead-code warnings remain; sandbox builds also reported incremental-cache hard-link fallback warnings.

Tests use synthetic data and in-memory disc structures. No new disc file or emulator run was performed. Retail runtime correctness is unverified. Full-image export to disk and real-disc end-to-end preflight remain unexecuted. The preservation ZIP SHA-256 and CRC remain valid; all 69 excluded files remain hash-identical. No additional commit or staging occurred.

Initial development runs exposed a test-helper visibility error and an incorrectly placed synthetic binding that occupied the only allowed test anchor; both were corrected. Sandbox temporary-file restrictions also affected existing catalog tests; the complete Studio suite passed with normal temporary-file access.

Current limitations:

- Planning retains a complete image in memory and compares the full PROT archive. This prioritizes correctness over memory and latency; planning is currently invoked synchronously by the UI. A worker and compact verified patch representation are follow-up improvements.
- Wall conflict detection cannot prove that no wall outside a mesh's search area belongs to it. No new ownership claim is inferred. Explicit ownership evidence is required before wall relocation can be enabled.
- Cross-cell export still requires verified equivalent destination resources. General TMD/TIM/CLUT relocation is not implemented. NPC clones alongside foreign imports remain blocked.
- Native allocation retains the existing fixed MAP record/grid capacity and conservative spawn reach. No general rotation/scale serializer or walkable-ground compiler is added.
- MAN resolution retains the existing unique-nearby-bundle validation. Native script behavior and story-state overrides require dedicated evidence and emulator testing. NPC edits remain selected-story-state only, explicitly reported.
- Interactive GUI operation, real-disc preflight and emulator behavior need a separately approved validation session. Headless tests do not establish those results.

## Next milestone: base objects and scene references

Introduce an editor document model incrementally behind the existing WorldModel/DrawInfo interface. Keep the rasterizer, camera and viewport interactions intact. Native serialization must consume document intent and resource evidence, never infer identity from draw indices.

### Identity and document types

`BaseObjectId` identifies immutable content and provenance: source-disc identity, scene/resource coordinates, kind, original bytes and a content digest. Include the material dependency identity; equal TMD geometry is not equal asset identity. Distinguish a source definition from placement-authored state. Preserve names as user metadata.

`BaseObject` owns or references geometry, primitive/material attributes, TIM/CLUT dependencies, animation compatibility and collision evidence. It must not contain a mutable instance transform. A resource resolver may deduplicate proven equivalent resources without merging reference identities.

`SceneReferenceId` is a stable persisted UID. Every original and duplicate reference has its own base-object link, transform, visibility, overrides and optional native binding. Duplicating a reference creates a new UID, even if immutable resource storage is shared. Selection, history and save/load use this UID rather than raster offsets or allocation results.

`NativeBinding` distinguishes MAP descriptor plus grid anchor, MAN actor record, terrain draw and unresolved reference. Original bindings remain reserved. New bindings exist only in an export plan until a successful explicit rebase/import of that output; preview edits never mutate retail bindings.

### Native allocation and dependencies

1. Capture an immutable document revision and source fingerprint. Collect all requested physical edits; classify unsupported edits as errors before generating output.
2. Resolve each base object's complete geometry/material/animation dependencies against the destination cell. Compare decoded semantics and sampled texture data where reuse is claimed. Return explicit reuse/import decisions and proofs. Missing evidence is an error, never guessed resource data.
3. Reserve all original native descriptors and anchors. Sort new references by stable UID. Allocate distinct free descriptors and valid free spawn anchors within proven engine limits. Occupied visual space does not prohibit overlap; occupied native reference slots do. Exhaustion returns a diagnostic without moving unrelated objects.
4. Validate each planned reference through the native parser and spawn rules. Grid anchors are storage/dispatch references, not an excuse to relocate the visible object. MAN allocation requires proven script/binding semantics rather than a fallback for exhausted MAP slots.
5. Resolve texture pages and CLUT placement jointly with all destination consumers. Introduce new native resource import only after PROT/scene-table footprint, compression, VRAM overlap and loader lifetime constraints are represented and tested.
6. Serialize from one canonical plan. Retain preconditions, allocations, resource decisions, patches, required byte counts, diagnostics and postconditions. A compact plan may replace P0's full image only if execution cannot reallocate or reinterpret it and verifies its source preconditions.

### Transforms and collision

Represent authored native placement transforms separately from reference overrides. Encode rotation through verified MAP/native transform conventions with a documented quantization policy. Reject unsupported transform classes. Scaling must select a proven engine mechanism or an independently owned transformed native resource; never scale a shared base implicitly.

Represent collision as `Unknown`, `IndependentRecordCollider`, `OwnedWallSet`, `SharedWallSet`, or a separately proven walkable surface. Wall ownership evidence must record its source and affected references. Removing one owner cannot remove another owner's wall. Unknown ownership remains a blocking diagnostic. An empty candidate stencil alone is not a universal ownership proof.

Walkable ground is a separate compilation domain: native floor tiers, height lookup tables, slopes, seams, boundaries and scripts require explicit validation. Preserve current floor nibbles until this compiler exists. Do not derive walkable surfaces solely from render-mesh normals.

### Incremental delivery gates

1. Add stable base/reference identities with a backward-compatible overlay migration and a WorldModel adapter. Prove unchanged render geometry/materials and selection/history behavior with synthetic fixtures.
2. Move duplication and transform intent into references. Test independent editing, shared immutable resources, UID persistence and failed-load rollback.
3. Introduce a deterministic native allocation ledger and resource dependency graph. Test exhausted slots, shared descriptors, overlapping references, invalid anchors, missing palettes and source-revision mismatch.
4. Add one proven resource/transform/collision capability at a time. Each capability needs parser/serializer roundtrip and untouched-byte checks before separately authorized emulator validation.

Recommended current commit boundary: all P0 corrections and their tests as one reviewable checkpoint, plus this design document. Suggested message: `LCK P0 - validate native export plans and restore scenes transactionally`. Do not mix the base/reference migration into this checkpoint. No commit or push is performed by this report.
