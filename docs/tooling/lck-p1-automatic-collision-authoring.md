# P1 automatic collision authoring

2026-10-26. Incremental implementation on `8257656f`, retaining Astra's uncommitted wall-addition planner, native reports and duplication investigation. No commit, disc export or emulator run.

## Default behavior

New native World Object copies and repository drops receive independent collision proposals automatically. An existing recipe wins over generation. Ctrl+D inherits the selected copy's edited recipe; deliberate removal on a selected copy stays removed. Restoring a scene uses its exact saved definitions rather than regenerating defaults. Defaults are suppressed only on the transactional restoration clone, so a failed restore cannot affect the live document.

Generation first inspects verified native descriptor flags and the record-derived collider offset. Existing data does not establish the complete live actor class, extents and enabled state for arbitrary objects. The implementation therefore does not promote that offset into a guessed native shape. No verified complete native-shape resolver is added in this increment; native collider recovery remains an acceptance dependency. The current fallback uses actual independent draw triangles.

The fallback retains rest-space triangle geometry, its SHA-256, pivot, generation policy version, filtering parameters and inspection evidence. It never samples nearby retail walls to assign ownership. Repeated copies reuse matching immutable definitions; equality includes geometry, pivot, policy and inspection. This cache is editor data, not proof of native behavior. Each persisted instance retains its own definition and corrections.

Native-placement donors retain actual MAP/MAN/descriptor provenance. Geometry-only donors instead use a separate `MESH_COLLISION` record with the real source scene, category, resource index and original TMD SHA-256. This avoids fabricating MAP provenance for raw meshes. Foreign placement and mesh-only recipes remain blocked from destination MAP compilation until their destination resource/collision contract exists.

## Geometry filtering and transforms

- Only triangles whose three vertices belong to the selected instance are used. No object AABB, convex-hull fill or connected-component ownership inference is substituted.
- Nonfinite/degenerate geometry is rejected or excluded. The default minimum face area is 16 square PS1 units; smaller faces are treated as decorative candidates. This is a heuristic, not semantic recognition of leaves, glass or materials.
- Faces with normalized absolute Y normal at least 0.70 are retained as floor/surface candidates, not converted to wall bits or declared walkable. Classification happens after the current transform when compiling walls.
- Steep faces are clipped to an explicit review band above the transformed mesh's lowest point. Default band height is 128 PS1 units, scaled on Y. This is an authoring heuristic, not a recovered player height or verified native floor. Overhead lintels outside the band do not close a doorway; large decorative faces and unusual pivots/ground shapes may still need review.
- Blocking sub-cells are computed by actual clipped polygon/cell intersection. Iterating a triangle's bounds only narrows the search; cells inside those bounds are not automatically filled. No flood fill closes holes between separate faces.
- The renderer's exact X/Y/Z rotation, scale and translation function also transforms collision geometry. Definitions remain immutable, avoiding cumulative drift and regeneration over user edits.
- Wall-grid projection is conservative at the native 64-unit resolution, including the wall-query Z bias. Boundaries and computation budgets fail explicitly. Coarse wall bits cannot encode arbitrary 3D height or multiple floors.

All generated results are labelled `MeshDerived`; native observations and manual `UserAuthored` edits remain distinct. A generated empty wall set does not certify that an object has no collision. Generated floor triangles do not authorize walkability or native heightfield changes.

## Optional visual review and correction

Select an object and expand **Authored collision recipe (in memory)** in Properties. Automatic proposals show native inspection, geometry hash and surface counts. Existing objects can explicitly request a default proposal using **Generate default collision from selected object**.

The optional grid shows a movable 16-by-16 window over the original native MAP's 256-by-256 wall sub-cells. Source and translated destination views expose sub-cell coordinates, byte offset, bit index, biased world center and original floor tier. Source-cell centering is spatial display only, never ownership attribution.

Colors distinguish protected original wall bits, selected automatic/manual blocks, explicit nonblocking requirements and other saved contributions. Original bits retain a yellow marker even when overlaid by an authored contribution. Destination view includes supported generated contributors. Drag a rectangle or enter exact inclusive X0/Z0/X1/Z1 bounds; select Blocking, Nonblocking or Unspecified. Unspecified removes manual corrections and reveals the automatic default.

**Save authored recipe** applies changes through existing scene undo/redo; normal Save LCK scene persists them. Text/grid edits remain drafts until saved. Raw mesh-only donors expose automatic geometry and TMD provenance, but native-grid painting remains unavailable without a verified MAP source. Existing mesh collision visualization remains available.

Nonblocking removes this owner's generated block at that cell and asserts that the compiled cell must be open. It cannot erase an original wall or another owner's contribution. These conflicts reject compilation with exact sub-cell coordinates, byte/mask and contributing owner IDs. Manual grid corrections currently require exact X/Z translation with unchanged rotation/scale/Y; unsupported transforms are reported rather than silently approximated. Generated triangle-only definitions support transformed in-memory projection.

## Native compilation and preflight

Native-source proposals feed the retained `WallAdditionPlan`: immutable baseline, deterministic contributor union, source fingerprint validation and byte-exact readback. It preserves every original wall bit, all floor nibbles, native descriptors, object grid and triggers. Open-space conflicts fail before an output is returned. Mesh-only/foreign contributions reject whole-document native compilation rather than disappearing from a partial result.

Recipe-bearing native export remains blocked. Check IMG now includes saved owner/source identities, compilation success/failure, native conflict counts and the read-only binding/script report. Passing additive compilation is not behavior acceptance. Native collider state, floor corners/LUT/kind-2 compatibility, script writers/consumers, story variants, native transforms/allocation and destination resources remain required checks. No MAN rewriting or terrain/actor-collider serializer is introduced.

## Validation

- Studio full library suite: 158 passed, 0 failed, 6 ignored, with filesystem access required by existing catalog tests.
- Focused app suite: 125 passed, 0 failed, 6 ignored before the final read-only fixture expansion.
- Read-only retail #230 test verifies automatic geometry attachment, exact generated scene roundtrip, visual Properties rendering without document mutation, and continued preflight rejection with nine unresolved native wall bits.
- The same fixture verifies native-placement and raw-mesh repository defaults, donor recipe roundtrip and transactional rejection after an original TMD fingerprint mismatch. Source is read-only; no IMG/project file is generated.
- Synthetic tests cover actual openings, floor/overhead filtering, exact renderer transforms, independent edited copies, cache reuse, unchanged originals, nonblocking/native/shared conflicts, legacy recipe compatibility, stale generated hashes, save/load/history and preservation of native floor/wall bytes.
- An existing persistence regression caught default generation occurring during restoration. Defaults are now suppressed on the staged restoration clone, and the regression passes.
- Build completion, final read-only fixture result, whitespace verification and retail image hash are recorded in the delivery response. Manual mouse interaction and emulator behavior remain untested.

## File scope

New: `crates/studio/src/app/collision_authoring.rs`, `crates/studio/src/app/collision_generation.rs`, this report.

Extended: `app.rs` (module registration only), `collision_recipes.rs`, `scene_instances.rs`, `world_view.rs` (creation/deletion hooks and shared transform visibility), `ps1_export.rs` (diagnostics and retained rejection), `native_dependency_report.rs`, `native_export_verification.rs`.

Astra's `lck-p1-native-duplication-gates.md` remains intact. Camera, viewport, rasterization, textures and retail resources were not edited. All work remains uncommitted for review.
