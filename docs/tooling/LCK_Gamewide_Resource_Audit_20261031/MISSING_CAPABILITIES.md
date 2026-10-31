# Missing reusable capabilities

Counts below are concrete census model-check failures where that check applies. They do not count all affected behavior assets: an unknown dependency is not proof it is irrelevant. Inspect ALL_ASSETS.tsv and cell graphs for exact affected identities.

## 1. Complete ordered texture contexts and composite VRAM reads

- Measured check failures in this bucket: **11**. Other affected assets remain unresolved; no invented count.
- Families: TIM, raw palettes, streamed resources, TMD primitives.
- Root cause: Unresolved texture/CLUT; inspect precise per-model reason in EXPORT_COMPATIBILITY.tsv.
- Modules: native_asset_plan.rs, scene_registration.rs, multi_tim.rs, texture_relocation.rs.
- Universal fix: Resolve missing source banks and native upload chronology; implement lossless composite word regions when an image/palette spans multiple last-writer uploads. Retain character/runtime reservations.
- Prerequisites: Proven loader routes and source-bank identities.

## 2. Native carrier and model registration families

- Measured check failures in this bucket: **1278**. Other affected assets remain unresolved; no invented count.
- Families: Descriptor bundles, raw/embedded packs, non-field/cutscene/battle/global resources.
- Root cause: Native carrier/loader adapter; inspect precise per-model reason in EXPORT_COMPATIBILITY.tsv.
- Modules: native_loader_trace.rs; engine-scene/model_bank.rs, scene_bundle.rs; container-formats.
- Universal fix: Connect every detected container family to a verified native registrar, including non-field routes; test cumulative imports and retained selectors against independent loader contracts.
- Prerequisites: Descriptor and streaming provenance; destination allocation.

## 3. Animation, MAN, scripts and interaction closure

- Measured check failures in this bucket: **0**. Other affected assets remain unresolved; no invented count.
- Families: NPCs, MAN, MOVE, ANM/CLUT-walk, VDF, event stagers, MES.
- Root cause: Model materialization; inspect precise per-model reason in EXPORT_COMPATIBILITY.tsv.
- Modules: native_script_dependencies.rs; engine-scene/scene_resources; scene-formats/man_section.rs.
- Universal fix: Resolve shared/special model banks, animation identity/clip/skeleton lifetimes and dynamic script targets; relocate incoming references and triggers with explicit geometry-only versus full-behavior semantics.
- Prerequisites: Model registration; instruction/control-flow evidence.

## 4. Collision and walkable-floor ownership

- Measured check failures in this bucket: **0**. Other affected assets remain unresolved; no invented count.
- Families: Retail wall grid, floor tiers/LUT, elevation overrides, actor colliders.
- Root cause: Collision writer limitation; inspect precise per-model reason in EXPORT_COMPATIBILITY.tsv.
- Modules: native_wall_journal.rs, native_wall_export.rs, native_elevation_codec.rs, collision_generation.rs.
- Universal fix: Keep editor-owned wall layer recomposition; establish original native ownership from actual scripts/records. Add exact walkable-surface encoding where original representation supports it. Mesh walls are not floor surfaces.
- Prerequisites: Complete physical dependencies and runtime sampling evidence.

## 5. Resource allocation, staging and runtime budgets

- Measured check failures in this bucket: **0**. Other affected assets remain unresolved; no invented count.
- Families: All import families and destinations.
- Root cause: Native VRAM allocation; inspect precise per-model reason in EXPORT_COMPATIBILITY.tsv.
- Modules: native_load_budget.rs, native_loader_trace.rs, scene_registration.rs, texture_relocation.rs.
- Universal fix: Bounded mixed-import fixture matrix by loader family and destination context; resource sharing/retirement and independent references; compare allocator traces when user supplies runtime logs.
- Prerequisites: All resource lifetimes and verified original-engine behavior.

## 6. Primitive and opaque format decoding

- Measured check failures in this bucket: **1569**. Other affected assets remain unresolved; no invented count.
- Families: Unrecognized data, unsupported primitives, multimedia/item/effect/audio containers.
- Root cause: Primitive/upload decoder; inspect precise per-model reason in EXPORT_COMPATIBILITY.tsv.
- Modules: container-formats, actor-formats, scene-formats, disc-audit.
- Universal fix: Investigate opaque range hashes/offsets and parser failures, create format-level adapters and representative regressions; retain unknown assets until decoded.
- Prerequisites: Native format evidence; no heuristic validity restrictions.

## Measured behavior/physics closure scope

- MAN script records: 6488; actor headers: 2708; animation-selector operands: 2708. Compatible animation clip/lifetime and dynamic script closure are not proven by those counts.
- Explicit unresolved wall-ownership edges: 30824; retail floor grids: 102; retained elevation-override rows: 30162. Rows are stored native records, not exclusive object ownership or runtime-executed surfaces.


## Precise measured failure mechanisms


- **Native carrier/upload context: 1344 model identities.** First witness `scene:balden2/model:0`: `Ordered native uploads: No native field descriptor table`.

- **Untextured geometry import route: 1162 model identities.** First witness `scene:balden/model:32`: `No textured dependencies`.

- **Mixed-depth native image interpretation: 234 model identities.** First witness `scene:conc/model:14`: `Native image depth mismatch`.

- **Raw palette upload alignment: 107 model identities.** First witness `scene:edbalden/model:0`: `Ordered native uploads: Unaligned native palette`.

- **Composite VRAM image/palette writer: 91 model identities.** First witness `scene:conc2/model:122`: `Fragmented image/composite palette requires a native composite-block writer`.

- **Unresolved source VRAM words: 11 model identities.** First witness `scene:conc2/model:53`: `Unresolved external image/palette data at Rect { x: 560, y: 128, w: 1, h: 1 }; palette=false`.


Untextured TMDs are legitimate geometry: the current foreign-import material planner demands textured dependencies, so these are a pipeline route gap, not invalid models. Native image-depth mismatch is a resolver assumption about upload metadata versus sampled GPU page depth; raw VRAM words are not intrinsically typed by their uploader. Unaligned palette upload rejection is an ordered-context validator constraint; distinguish upload-strip coordinates from the actual CBA palette start before proposing a fix. Fragmented regions require lossless composite last-writer word assembly. Missing source words remain unresolved until an actual native source bank/upload is identified.

## Engine extensions

No new original-engine limitation is proven by these host-side failures. Dynamic registry capacity, scene heaps, floor span limits and VRAM residency require specific data/instruction evidence plus runtime validation. Do not install an unverified engine patch or interpret missing adapters as hardware impossibility. Unrestricted export remains unchanged.
