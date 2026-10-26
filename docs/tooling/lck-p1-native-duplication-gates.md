# P1 native duplication: narrowed export gate

2026-10-26; incremental work after `8257656f`. No export permission is added by this change. No IMG, emulator run or commit.

## Concrete progress

`collision_recipes::WallAdditionPlan` compiles explicit contributions into deterministic `(MAP byte offset, expected original byte, replacement byte)` writes. Its fields are private. Compilation rejects writes outside `0x4000..0x8000`, low-nibble changes and removal of any original bit; application requires the exact original MAP SHA-256 and verifies the output SHA-256. Compilation also applies its patches and compares the complete output with the compiler result. The existing in-memory editor validation now uses this path.

Tests check overlap/order independence, stale descriptors/grid/tail/walls, rejection of accidental accumulation on a previous compiled revision, unchanged native walls, and floor/object-cell readback using the independent engine-field `FloorGrid` reader. This establishes an additive native-byte serialization primitive, not complete object collision or export eligibility. `NativeExportPlan` still rejects recipe-bearing documents and unresolved ownership.

The dependency report previously returned before scanning MAN when the selected descriptor was unbound. It now scans scene scripts for unbound selections too, exposes each observed wall writer's MAN address and operands, and reports individual decode gaps. Paint ranges use the verified VM consumer's asymmetric row bias and wrapping: X `[x0,x1+1)`, Z `[z0+1,z1+2)`. Sub-0 clears all wall bits, sub-1 sets all, sub-2 clears a mask, sub-3 sets a mask. All retain floor nibbles. Conditions and reachability are not inferred from linear decoding.

## First candidate, not yet an approved collision reconstruction

Prefer town01 native descriptor **230**, placement cell **(94,78)**, mesh selector **15**, original position **(12096,0,10048)**. This identifies the unbound use; other placements of descriptor 230 are bound. Existing allocation evidence establishes a +64 X independent copy without moving unrelated references. Nine nearby wall bits are diagnostic conflicts, never an ownership assignment. Resident immutable model/texture resources do not require relocation for this same-cell test.

Descriptor 84 at (41,7), mesh 11, was also directly confirmed unbound, but missing diagnostic bounds make it a less focused first collision-authoring case. Neither candidate is claimed fully exportable.

Direct retail reporting found town01 MAN partition counts `[36,53,39]`, with no errors in the 128 bounded linear scans. Nine observed wall-paint instructions occur in P0[20] at MAN bytes `0x763, 0x769, 0x76f, 0x783, 0x789, 0x78f, 0x79e, 0x7a4, 0x7aa`. Their encoded ranges are near tile columns 21..29, rows 43..46 before the native row bias. These observed writes are spatially separate from candidate (94,78). This is useful evidence, not proof that helpers, overlay code or other story variants cannot write its collision.

MAP PROT 1 SHA-256: `60ecaa14978708f8696d60c12f6e98a88cdb19c2e63c7cae8f1c967a1214eb3b`.
Decompressed MAN SHA-256: `85a1ed2fdf79e7bf2e2daa4bf100740a1e18c9f92bc9f82fa4b72c296a3af5b3`.

## Shortest safe path

1. Keep the source unchanged. Start with a copy-only, same-resource, X/Z-translation capability; no deletion or source relocation is needed for its proof.
2. Review explicit wall sub-cells for the compact candidate and its destination, with source fingerprints and evidence notes. Do not seed ownership from bounds or automatically copy the nine conflicts. User-authored collision is a deliberate reconstruction, not recovered retail attribution. An empty recipe cannot certify that the object has no collision.
3. Establish actor collider and ground requirements separately. Walls are 64-unit sub-cells, but their world probe has a Z bias of two sub-cells; floor queries use a different indexing scheme. Preserve floor corners, the MAN LUT, kind-2 rows and grid markers. A wall stencil cannot supply a platform, stair or second walkable height. Reject candidates requiring unsupported surface transfer.
4. Close writers/consumers for the selected source/destination and each affected story MAP. The complete decode inventory is necessary but not sufficient: validate control-flow/helper/overlay effects, and treat any unresolved clear/set interaction as a blocker.
5. Only after that contract is satisfied, attach a sealed wall plan to the same NativeExportPlan as descriptor/reference allocation. Compile against each original MAP independently. Preserve native baseline walls; recompute authored contribution union rather than accumulating prior output. Verify parsed placement identity, original references, triggers, mesh selector, floor data, every unchanged PROT resource and sector integrity before writing a new-path IMG.
6. User verifies that the copy renders, blocks correctly and leaves original interactions intact in the emulator. A byte-correct image alone is not a playable-behavior claim.

Native walls have no general owner IDs. LCK's stable owner IDs remain editor metadata; compiled bits are simply the native union. Overlapping authored solids union safely. Moving an authored contribution recomputes that union, but moving a retail object cannot clear its original walls without exclusive/shared ownership evidence. Keeping the old wall in that case would leave ghost collision; therefore native movement remains blocked rather than reported as preservation.

## Vahn's House: concrete dependency boundary

The retail report still resolves descriptor 137 to P0[0], animation 1. It also observes incoming extended-context references from **P0[35]**, **P2[3]** (flat 92), **P2[6]** (flat 95), and **P2[15]** (flat 104). Thus cloning just the house's locally simple script cannot establish independent behavior. Observed operands are not a complete reachable graph.

Appending P0 shifts later partition flat IDs. A supported allocator must account for every affected trigger and context reference, motion-VM section-1 actor bindings, helper/dynamic dispatch, and partition-specific headers; appending the same bytes to another partition is not a substitute. Preserve original story-script targets by default; deciding which incoming events should also address a new instance is explicit behavior authoring. Animation ID 1 can only be shared after verifying mutable animation/actor state isolation. CFlag bit 0 changes actor collision, so a static wall recipe cannot replace this behavior. Native actor/context, offset-table and compressed-resource limits still require validation. No speculative MAN rewriting was added.

## Bounded follow-ups

- Sol Medium: expose wall-grid selection and source/destination ground samples for candidate 230; persist explicit reviewed classifications without claiming retail ownership. Show writers and unresolved coverage alongside selections.
- Sol Medium: extract a structured scene wall-write inventory from the existing report, add control-flow coverage diagnostics, and fixtures for overlapping conditional clear/set ranges. Keep export permission closed.
- Sol Light: tests for sealed wall-plan stale hashes, multi-contributor removal and story-MAP mismatch; capture reproducible read-only candidate reports and maintain the acceptance checklist.
- Astra: close candidate actor/surface/writer semantics and approve the narrow export contract; investigate the house's four incoming context sources and motion bindings before any MAN allocator/rewriter.

## Validation

- `cargo test -p legaia-studio --lib --locked`: 150 passed, 0 failed, 6 ignored with the filesystem access required by catalog tests. The sandboxed attempt had 146 passed and four catalog Access denied failures; all four passed in the permitted rerun.
- Read-only `selected_world_object_report_preserves_native_provenance`: passed, including the house's incoming-context inventory.
- Read-only `unbound_candidate_reports_scene_wall_dependencies`: passed for descriptor 230 at (94,78). Full NativeExportPlan rejects its +64 X copy with exactly nine unresolved native wall bits. The first sandboxed planner attempt could not canonicalize the retail path; the permitted read-only rerun reached and verified the collision rejection. No image was generated.
- `cargo test -p legaia-engine-core --lib --locked field_tile_is_wall_matches_retail_subcell_derivation`: 1 passed. This validates the existing port's wall query, not emulator runtime behavior of a new copy.
- Existing GUI/camera/rendering code and native exporter gates are unchanged. Four other opt-in disc tests and emulator/manual GUI tests were not run.
- Retail image SHA-256 remains `750dbcbadb70362feb3468467bbe7efbf3d151a414caa428c66c368d4b0b73e6`.

Changed files: `collision_recipes.rs`, `native_dependency_report.rs`, `native_export_verification.rs`, and this document. Build completion and final whitespace verification are recorded in the delivery response. All changes remain uncommitted.
