# First static native copy: bounded collision-contract verification

Base: `f1fdfe6df65a01ea1143fc3c7a6dbacf7e727a70`. This increment remains uncommitted. No IMG was generated and no emulator was operated.

## Result

Rim Elm descriptor 230 at grid (94,78), mesh 15, remains a suitable allocation/resource candidate. The existing native dependency reader confirms no kind-1 binding for this particular placement. Other uses of descriptor 230 are not interchangeable with it. The unchanged same-cell model uses the already resident scene resources; no texture or script transplantation was attempted.

A new read-only retail fixture runs the existing allocator and collision compiler separately in memory, composes their outputs only in the fixture, and confirms the complete NativeExportPlan still rejects the unresolved edit. It does not create an alternate export path or change an evidence classification to fabricate review.

Measured on the preserved retail source:

| Item | Verified value |
| --- | --- |
| Original world position | (12096, 0, 10048) |
| Proposed copy world position | (12160, 0, 10048), +64 X |
| Allocated descriptor | 509, distinct from original 230 |
| Copy spawn grid cell | (92,78), previously blank |
| Copied record collider center | (12160,10048); center only, not a complete shape |
| Model/orientation | Mesh 15, source native orientation retained, editor scale unchanged |
| Generated collision proposal | 41 floor candidates, 56 steep faces, 0 filtered faces |
| Vertical review band | 128 PS1 units; heuristic, not established player/collider height |
| Additive MAP wall changes | 5 bytes, 8 newly set bits |

The allocator uses a free descriptor and blank reference, preserves every original parsed placement, and changes only the new descriptor and new grid reference before collision composition. Both the chosen spawn cell and binding anchor pass existing binding/trigger checks. The X displacement between the copy world position and spawn-cell center is 320 units; Z displacement is zero. This passes the existing 384-unit anchor-reach gate. **This is not a full runtime spawn-window coverage proof.** The test does not relax that gate or move unrelated anchors.

The fixture reads back all floor tiers and preserved wall nibbles using the independent engine-field FloorGrid reader. Original descriptor 230, all original placements, and MAP binding/tail bytes remain intact. Mesh identity and native orientation roundtrip through the native placement parser. The read-only source archive/resources are never modified. These checks establish byte serialization properties, not playable behavior.

## Exact remaining blocker

The generated recipe is still MeshDerived, without an accepted native collision/surface contract. A descriptor's collider offset identifies a center; it does not establish complete actor collider class, enabled state, extents or vertical behavior. The 41 floor faces are candidates only; a high-nibble MAP wall addition cannot express a platform, stair or second walkable level. The heuristic review band cannot be promoted to native actor height. The bounded MAN inventory is not complete control-flow/helper/overlay/story-variant coverage.

The nine spatial native wall conflicts previously reported at the source are not ownership metadata. A new additive copy can preserve them all without needing permission to clear them. Conversely, moving/hiding the original would require evidence for removing its retail footprint; retaining a ghost footprint would not meet preservation requirements. Neither the mesh nor the eight new bits proves equivalence to retail collision.

For these reasons NativeExportPlan retains its existing recipe export rejection. No image path or playable-collision claim is provided. Changing the rejection to accept a successful additive compile alone would omit unresolved behavior and is outside this increment.

## Focused code changes

- `crates/studio/src/app/collision_recipes.rs`: preflight diagnostics distinguish additive copy intent from original-footprint removal, expose heuristic mesh surface counts, and report newly added wall-bit counts separately from byte counts. Diagnostics remain read-only. A synthetic regression verifies the distinction, unverified-floor warning and unchanged document state.
- `crates/studio/src/app/native_export_verification.rs`: new opt-in `rim_elm_static_copy_additive_contract_stays_blocked` fixture verifies actual native allocation, placement/model/orientation readback, protected original bytes, additive collision and independent grid readback, then requires the complete planner to reject unresolved collision. It writes neither scene metadata nor IMG files.
- `docs/tooling/lck-p1-first-static-copy-contract.md`: this report.

No changes to allocation algorithms, export serialization or permission gates, GUI, naming, NPC handling, MAN scripts, textures, rotation/scale support, or directory layout.

## Validation

- `cargo test -p legaia-studio --lib --locked --message-format short`: **171 passed, 0 failed, 9 ignored** with filesystem access needed by the existing persistence tests.
- Explicit read-only retail test `rim_elm_static_copy_additive_contract_stays_blocked`: **1 passed**, producing the measurements above. The remaining eight opt-in tests were not rerun in this increment; no ignored test that creates an image was run.
- `cargo build -p legaia-studio --locked --message-format short`: passed. Existing float/deprecation/dead-code and filesystem hard-link warnings remain.
- Two initial fixture compilation type errors were corrected before successful validation; no failing test remains.
- `git diff --check`: passed. Nothing staged, committed or pushed; HEAD remains the approved baseline.
- Retail SHA-256 unchanged: `750dbcbadb70362feb3468467bbe7efbf3d151a414caa428c66c368d4b0b73e6`. Existing images, user alias catalog and historical files untouched.

## Smallest next step

Close and document a candidate-specific collision/surface contract, without writing MAN: identify the static actor collider consumer/class and whether this exact object requires walkability; establish relevant floor/LUT/kind-2 and wall-writer behavior for its source/destination and supported story maps. Explicit reconstruction review must retain its authored provenance and state what behavior it replaces; ownership of retail walls must never be inferred. If actor/surface behavior cannot be closed for this object, use a different candidate only after those same requirements are demonstrated.

After that evidence exists, the next Sol increment can attach the sealed additive wall plan to the same NativeExportPlan as descriptor/reference allocation, compile independently against each supported story MAP, and retain full unchanged-resource, native readback and sector-integrity checks before producing a fresh IMG. The current authoring UI or a generic review checkbox is not sufficient evidence to unlock export. Runtime spawn-window and gameplay checks remain for the user; DuckStation was not launched or controlled.
