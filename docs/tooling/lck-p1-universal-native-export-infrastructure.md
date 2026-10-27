# Universal native export: common dependency infrastructure

Base: `f1fdfe6df65a01ea1143fc3c7a6dbacf7e727a70`. Existing uncommitted static-copy diagnostics, tests and report are retained. No commit, push, IMG generation or emulator operation was performed.

## Capability matrix

This distinguishes existing native serialization from new inventory support. An editor preview, successful resource discovery or a synthetic fixture is not proof of a playable transplant.

| World Objects content | Common category / native route | Native resource output available today | Blocking work |
| --- | --- | --- | --- |
| Static buildings/environmental objects | MapPlacement / existing MAP adapter locally; strict resident-static bridge for foreign placements | Constrained native MAP movement and independent descriptor/reference allocation, subject to existing collision, binding, transform, floor and spawn-anchor gates | Authored collision/surface acceptance; nonresident TMD/TIM packing; complete runtime spawn-window coverage |
| Decorative props | Same MAP route if a placement; TerrainTile if terrain-backed | Same constrained MAP capability for actual placed records | Decoration is not inferred from appearance; terrain-backed cases require a terrain writer |
| Terrain geometry/tiles | TerrainTile / unsupported adapter | None for edited/imported terrain | Native terrain representation, resource registration and preserved floor/binding semantics |
| Walkable surfaces | WalkableFloors requirement, independent of the visual category | Original floors preserved; no new surface compiler | Floor tiers/corners, LUT and kind-2 dependencies, collision consumers; stacked surfaces cannot be supplied by wall bits |
| NPCs/animated actors | Actor / existing local MAN adapter | Existing constrained local MAN spawn/model/animation edits and experimental copies, with existing limits | Foreign actor/ANM resource import, independent runtime state, general script/animation closure; arbitrary preview frames not serialized |
| Bound/interactive objects | MAP or actor category plus ScriptBindingsTriggers requirement | No general independent behavior transplantation | MAN/context/trigger relocation, shared state and motion/animation dependencies; existing copy adapter rejects script-bound static sources |
| Raw reusable TMD geometry | RawMesh / unsupported adapter | Exact source bytes can be inventoried; no native placement output | A valid native placement/collider/behavior template and destination resource registration; a mesh alone does not supply these |

There is no claim that all objects within a category export. No new category is unlocked in this increment. The existing native adapters remain the authority for their constrained supported cases.

## Shared paths reused

`ps1_export::plan_export` remains the only public native planning entry point. It calls the existing scene-document parser/preparation, strict donor-aware static reuse bridge, descriptor allocator, NativeBindings trigger protection, MAP/MAN serializers, story-map checks, complete source-archive validation and compiled-image readback. `export` consumes the private plan's already compiled bytes; it does not rediscover dependencies or allocate again. Existing allocation/copy limits, transforms, collision gates and sector/image checks remain enabled.

The previously implemented automatic collision recipes, additive WallAdditionPlan and independent allocation fixture are retained. Original native walls are never attributed using mesh bounds. Reviewed authored additions and original-retail-footprint removal remain distinct requirements. Mesh floor candidates do not become walkable geometry.

## New reusable infrastructure

`native_asset_plan::NativeAssetPlan` is a deterministic, editor-independent import inventory retained inside NativeExportPlan and included in preflight errors and successful-plan summaries.

- Per-instance requests use persistent created UIDs. Original draw addresses are explicitly plan-local adapters, not new saved identities. Global aliases/display names never select donors or resources. Duplicate/zero UIDs, missing graph edges, orphan foreign instances and changed foreign TMD bytes reject planning.
- Each request records its source/category and routes to NativeMap, ResidentStaticBridge, NativeMan or an explicitly unsupported adapter. MAP buildings/props are not given guessed semantic classifications. Bound behavior and walkability are explicit requirements rather than inferred visual categories.
- Requirements cover geometry, TIM/CLUT/page context, placement allocation, spawn windows, walls, walkable floors, actor/animation state, scripts/bindings/triggers, resource packing/memory and transforms. These are structured obligations, **not a claimed resolved dependency closure**. Existing adapters must still validate their supported subset; missing general writers remain blockers.
- A shared immutable resource registry retains exact TMD/TIM bytes, SHA-256 identities, source-scene/PROT-entry/resolved-offset provenance, and all origins when equal payloads are reused. Resource kinds remain distinct even for identical byte strings. Multiple requests sharing a payload retain independent instance identities.
- TIM inventory uses the existing native scene TIM scanner, retaining complete scene-local image/CLUT uploads conservatively. It does not guess a minimal texture subset. Whole assembled VRAM pixels plus upload coverage form a separate texture-context identity, so zero uploaded texture data differs from absent data. Shared-block/runtime loader closure is not inferred from the scene-local TIM list.
- Destination-discovered resources are inventoried first. Donor assembly and TIM inventory are cached once per source per plan. Equal discovered bytes can be shared in the inventory; **this does not allocate a native mesh selector or prove runtime pointer-table registration/memory capacity**. The existing static bridge may separately prove decoded equivalence for supported requests.
- Source offsets refer to resolved/decompressed resource bytes, not arbitrary patch addresses inside a compressed PROT payload. No metadata address is used to rewrite a scene or script.
- Unsupported terrain, raw geometry and foreign-actor routes do not yield writable plans. Existing authored-collision rejection retains priority and includes the new inventory where available. Unsupported instances are never filtered away to make another request succeed.

This establishes the common importer front end and adapter boundary. It does not implement new native TMD/TIM repacking, VRAM relocation, primitive rewriting, ANM or MAN relocation, terrain/floor generation, or executable changes. No requirement-list success or inventory hash is treated as export permission.

## Source scope

Changes in this run:

1. `crates/studio/src/app/native_asset_plan.rs` (new): typed instance/resource inventory, exact immutable resource reuse, provenance and category routing; three focused synthetic regressions.
2. `crates/studio/src/app/ps1_export.rs`: collect and retain the inventory in the existing NativeExportPlan; include it in failure/success diagnostics; reject unsupported routes. Allocation/serialization algorithms unchanged.
3. `crates/studio/src/app.rs`: module registration only; no GUI or input changes.
4. `crates/studio/src/app/native_export_verification.rs`: new read-only three-scene inventory/adapter regression, alongside the retained static-copy test.
5. `docs/tooling/lck-p1-universal-native-export-infrastructure.md`: this report.

Retained prior uncommitted work also includes `crates/studio/src/app/collision_recipes.rs` and `docs/tooling/lck-p1-first-static-copy-contract.md`. No renaming, camera/viewport/rendering, scene persistence or GUI implementation changes. No Cargo manifest/dependency changes. User alias catalog, historical files and existing image outputs are untouched. Everything remains unstaged/uncommitted for review.

## Validation

- `cargo test -p legaia-studio --lib --locked --message-format short`: **174 passed, 0 failed, 10 ignored**, with required filesystem access for existing metadata tests. Includes resource reuse versus independent instances, texture/palette byte differences, upload coverage versus missing data, category routing, allocation/native readback, collision union, scene restoration and naming regressions.
- New read-only `multiple_towns_and_categories_share_one_native_dependency_inventory`: **passed**. Uses `town01` (Rim Elm), `izumi` (Hunter's Spring), `bylon` (Biron Monastery), selected by existing verified catalog IDs. Eight requests span native copy, local actor movement and foreign placement/terrain/raw mesh imports from both donors. Two inventories compare equal, 237 resource nodes retain donor bytes, stale donor geometry rejects, and both unsupported categories appear in the planner rejection. No object-specific export exception was added.
- Retained read-only `rim_elm_static_copy_additive_contract_stays_blocked`: **passed again**. Descriptor allocation/additive readback evidence remains intact and unresolved export stays blocked. It does not run an emulator.
- `cargo build -p legaia-studio --locked --message-format short`: passed; existing compiler/filesystem hard-link warnings remain.
- Final whitespace review passed. The initial full-suite run exposed a changed error priority in an existing collision safety regression; the collision gate's priority was restored and the unchanged regression passes. No failures remain.
- No successful new real-disc export or gameplay test is claimed. Texture relocation and general native packing remain unimplemented, so these have no relocation roundtrip pass to report. Existing synthetic MAP/MAN plan/resource and collision readback tests remain green.
- Retail source SHA-256 remains `750dbcbadb70362feb3468467bbe7efbf3d151a414caa428c66c368d4b0b73e6`. Only explicitly selected read-only retail tests ran; no image-producing ignored test ran.

## Native boundaries and next increment

New content must be registered in the engine's native mesh/actor tables; a scanned TMD's presence in a PROT entry is insufficient. Scene-bundle layout, compressed footprints, loader registration order, native selector widths and RAM budgets must be validated before allocating new native resource selectors. TIM uploads must respect shared/reserved VRAM, upload order, CLUT depth, texture-page addressing and all primitive references. MAN/context tables and independent mutable actor state require separate writers and proofs. Current native MAP limits and the allocator's 16-copy/4-actor experimental caps are retained; inventory counts do not expand them. Whether specific cases need executable changes remains unresolved rather than assumed.

**Next Sol Medium milestone:** a fixture-only TIM/CLUT footprint and relocation planner behind this resource registry. Enumerate every supported native textured primitive's CBA/TSB/UV reference using authoritative TMD decoders; retain/explicitly reject unsupported native primitive variants. Reserve destination/shared upload regions, place a complete image/palette without overlap, emit deterministic TIM + native primitive patch records, and independently parse/read back UV/page/palette equivalence. Test 4-bit, 8-bit, direct-colour, shared palettes, upload-order collisions and exhausted VRAM. Keep real-disc transplantation blocked until native scene packing and registration consume this validated plan.

Sol Light can review the exact diff and add deterministic/stale-source inventory regressions. Native resource registration/packing and actor/surface/script closure require targeted engine evidence before authorizing their adapters. This is a shared architectural boundary, not a list of object-specific exemptions.
