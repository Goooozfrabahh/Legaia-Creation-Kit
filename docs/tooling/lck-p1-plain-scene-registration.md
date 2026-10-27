# Verified plain-bundle resource registration

Working tree continues from `432cb3df`; all uncommitted single-/multi-TIM implementations remain preserved. No checkpoint or IMG output.

## Implemented

The reusable adapter rewrites a plain native scene asset table with one type-1 TIM-list and one type-2 TMD-pack descriptor. It uses the existing descriptor-walk `scene_asset_table::mesh_pool` lookup, not scanner order: `FUN_80020224` dispatches type 2 to `FUN_8001F05C`, registering pack members through `FUN_80026B4C` in pack order. The engine's `descriptor_walk_pool` implementation documents this mapping and the five-model field resident head. This provides a verified mapping for the restricted format, superseding earlier reports' broader unresolved scanner-based mapping statement.

New immutable TIM/TMD members append or reuse exact existing member bytes; old model selectors remain stable. The adapter maps donor ResourceIds to destination bundle selectors, checks source fingerprints, and can return a copied placed MAP record with only its mesh-selector halfword changed. It allocates no scene instances or grid anchors and does not claim to satisfy their collision/script contracts.

All descriptors are decoded within validated nonoverlapping source spans. The table count/types remain unchanged; type/size words, offsets and the decompressed-size total are updated. Only TIM/TMD streams are recompressed using `disc_patch::compress_within`. Unrelated compressed payload spans, their decoded bytes, and suffix/padding bytes are preserved. Full output is reparsed and decompressed; old/new model bytes are resolved via native descriptor lookup, and the resulting TIM-list must replay to the relocation plan's destination VRAM.

Caller-supplied fixture limits enforce model count, total decoded bytes, encoded bytes and native integer widths. In addition, the documented field-loader allocations cap encoded bundles at the `0x62c00` arena and decoded TIM lists at `0x70800`; callers cannot enlarge these caps. Other caller limits are not asserted retail RAM/table capacities. Duplicate/alternate registrations, invalid bounds, inconsistent totals, exhausted limits, active CLUT walkers, bare TMD2/PAC streaming and unsupported table variants reject explicitly. Selectors are bundle-relative; MAP use requires the single-mesh-bundle field context checked by the native fixture. Whole-CDNAME multi-bundle mappings remain unsupported.

Declared retail TIM members use the existing native `tim::parse` decoder and retain upper flag metadata verbatim; new generated TIMs remain strictly validated. The initial native test correctly exposed that strict scanning rules cannot be applied indiscriminately to already-declared retail members. No unknown flag bits were cleared or synthesized.

## Integration and exact source scope

- New `crates/studio/src/app/scene_registration.rs`: adapter, limits, readback and typed fixture result.
- New `crates/studio/src/app/scene_registration_tests.rs`: three standard regressions and one explicit read-only native acceptance fixture.
- `crates/studio/src/app/native_asset_plan.rs`: module registration.
- `crates/studio/src/app/ps1_export.rs`: `NativeExportPlan::register_resources_fixture`, compiled **only under cfg(test)**. Its result cannot be passed to the disc exporter. Live export planning/gates are unchanged.
- `crates/studio/src/app/multi_tim.rs`: immutable source-ID accessor and type visibility for the adapter.
- `crates/studio/src/app/texture_relocation.rs` and `texture_relocation_tests.rs`: module/test-helper visibility for reuse; previous relocation behavior retained.
- This report: `docs/tooling/lck-p1-plain-scene-registration.md`.

## Validation

- Full suite: **188 passed, 0 failed, 13 ignored**. Three new regressions cover selector/native lookup, MAP reference readback, exact preservation of unrelated streams/padding and original model IDs, repeated resource reuse, upper TIM metadata preservation, duplicate descriptors, malformed/truncated offsets, active CLUT walkers, stale resources and explicit capacity failures. All previous texture, collision, naming, restoration and editor regressions pass.
- Read-only actual-bundle fixture passed: Biron Monastery PROT 53 registered model count **115 -> 116**, selector **115**, single-bundle field pool index **120**; encoded bytes **335872 -> 333320**. Hunter's Spring PROT 31: **50 -> 51**, selector **50**, field pool index **55**; encoded bytes **159744 -> 158019**. Every preexisting model remains at its original index. Incoming texture/model resources are synthetic relocation fixtures registered into authentic destination bundles; no playable transplanted object is claimed.
- Rim Elm's PAC-streaming variant rejects explicitly. No scene-specific exception was added to the adapter.
- Studio build passed. Existing and expected unused fixture-API warnings remain. Whitespace checks pass. Retail hash unchanged: `750dbcbadb70362feb3468467bbe7efbf3d151a414caa428c66c368d4b0b73e6`.

## Remaining boundary

This produces actual native bundle bytes with validated model registration, but only in memory. Native IMG output still requires authoritative scene-wide upload/shared-memory and runtime capacity contracts, whole-CDNAME/global selector mapping where applicable, source/destination physics and behavior acceptance, and integration with allocation and archive/sector verification. No script, collision, floor, naming, camera or GUI behavior was changed. No disc, existing test image or backup was overwritten; no emulator, push, reset or commit occurred.

Smallest next increment: bind this adapter to an explicit destination scene context that validates all CDNAME registration contributors and native budgets, then test combined independent MAP allocation plus resource-bundle changes through an in-memory NativeExportPlan with unchanged-resource/sector readback. Keep writable IMG output blocked until the physics/dependency contract is independently satisfied.

## Known model-registration collision protection

`0x8007C018 + 200*4 = 0x8007C338`, the native four-word SFX countdown array (`FUN_80035B50`, store at `0x80035B9C`; delay setter store at `0x80035BCC`). Every resulting registration at global index 200 or higher is rejected. **This does not certify that 200 entries, or every index below 200, are safe.**

Diagnostics report existing global occupancy, additional registrations after exact-resource reuse, the first proposed index and the known conflicting index/address. The single-bundle field adapter includes the five resident models: 194 original scene models plus one new model reaches global index 199; 195 plus one reaches 200 and rejects. Reuse adds zero registrations. Future exporters must supply actual global occupancy including all other contributors to the shared diagnostic, never just a bundle-local count. All stricter model/encoded/decoded limits, runtime gates and fixture-only IMG restrictions remain enforced. Two targeted regressions cover these boundaries, stale-overfull occupancy, overflow, reuse and stricter limits; no production transplantation capability is added.
