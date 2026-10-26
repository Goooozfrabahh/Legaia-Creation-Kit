# P1 collision and binding provenance

2026-10-26. Baseline remains `008ccaf4`; allocation and dependency changes are uncommitted. No emulator use or IMG generation.

## Conclusions and evidence limits

The examined native formats do **not** contain a general wall-bit-to-object ownership relation. MAP wall bits are shared scene data. Object descriptors and bind scripts define a separate actor collider. Geometry bounds cannot reconstruct which walls an artist intended to associate with a building. Script writes recover a writer/dependency relationship, not exclusive ownership, and absence of a local write is not proof that no other script affects that area.

Therefore automatic complete collision transfer is not currently justified for Vahn's House or the other blocked placements. LCK needs explicit, source-bound authorship for ambiguous static wall/surface data, supplemented by verified script dependencies. This is a representational gap, not descriptor exhaustion. Existing allocation work is retained.

### Three native systems

| System | Verified native representation | What transfers with a descriptor |
|---|---|---|
| Actor collision | Actor position plus descriptor signed tile/sub-anchor offsets; flag 0x8 applies offset correction; scripts can change actor class/collision flags | Static offset parameters transfer; bound runtime state needs its script context |
| Wall collision | High nibble of each byte at MAP +0x4000, four sub-cells per tile; runtime wall-paint opcodes can modify it | Nothing automatically: no descriptor owner ID is stored in those bits |
| Walkable height | Low nibble indexes a scene height LUT, sampled using four neighboring corners; grid bit 0x800 selects kind-2 elevation override instead | Neither the LUT, corner samples, nor override record transfers merely by cloning a mesh |

Supporting code: `scene-formats/src/field_objects.rs::collision_footprint_offset`, `engine-field/src/field_elevation.rs`, `engine-field/src/field_regions.rs`. Supporting native consumer documentation: `docs/subsystems/field-locomotion.md`, `docs/formats/field-map.md`, and the south-gate case in `docs/reference/re-settled-threads/field.md`.

The documented south-gate script paints wall rectangles conditionally; town0c carries that paint sequence in more than one script. This demonstrates why even a known writer cannot automatically become a sole owner. The floor sampler is a single-valued scene grid, so overlapping geometry does not imply support for two independent walkable heights at the same X/Z.

## Vahn's House read-only case study

New direct retail reads inspected MAP entries 1, 10 and 19 and their actual 0x28-sector load windows. Native MAN payloads were decompressed through the existing reader. Results:

`D:\AstraRimElmProject\Backups\LCK_P1_20261026_dependencies_final\binding-dependencies.txt`

Record 137 has placement cell (38,25), but its **bind lookup anchor is (37,24)** after applying signed descriptor deltas. Earlier summaries loosely called the placement cell an anchor; the raw bind lookup must use the latter coordinate. The selected kind-1 row is at primary trigger-block offset 0xcc (absolute MAP byte 0x100cc), naming flat MAN record 0, gate 0, animation 1. In town01 its bounded MAN record is 0x1ab..0x1d9, with script PC 12 relative to the record.

The complete bounded P0[0] linear decode has no decode error. It includes animation initialization, rate/hold control, sound/effect, animation flag changes, CFlag.Set bit 0 and a local loop. The bit-0 change is the documented actor collision/touch disable during opening. It has no 0x4C outer-nibble-7 wall paint and no extended cross-context target in this bounded stream. This establishes local script content, not absence of incoming references or indirect behavior elsewhere in the scene.

The prior 43 wall bits are **43 diagnostic conflicts**, not 43 owned collision bits. The door/house animation script does not provide a stencil attributing them. Adjacent native record 315 is another bound placement, using anchor (38,25), MAN record 29; it must not be conflated with record 137. The relationship between all house-area wall cells, doorway transitions, adjacent actors, and walkable surface samples still needs an explicit dependency/authoring model.

The diagnostic walks native placements, including non-rendered placements absent from the 35-draw editor list. A rendered-object list alone is insufficient to enumerate script consumers.

## Implemented increment

`native_dependencies.rs` adds a conservative native trigger resolver used by `NativeExportPlan` compilation:

- Read the actual 40-sector MAP load at its disc LBA, including the physically adjacent fallback window. Do not infer fallback provenance from an assumed PROT entry number.
- Validate signed table offsets/counts and complete row bounds. Malformed nonempty tables fail planning; they do not become empty bindings. Native nonpositive counts perform no iterations.
- Resolve kind-1 rows primary-first and first-match-first, retaining MAN record ID, gate, block and byte offset. Do not filter nonzero gates or discard an undecodable MAN record and accidentally select a fallback.
- Reject edits with an unresolved source bind even when the stored grid bind bit is clear.
- Avoid candidate copy anchors with native kind-1 bindings. Reserve kind-0, kind-1 and kind-2 trigger cells against spawn allocation, including fallback-only cells whose derived bits are not yet in the on-disc grid.
- Preserve existing bind-bit checks, wall guards, floor checks, allocation constraints and sealed-plan serialization/readback. No scripts, triggers or walls are rewritten by this increment.

The physical fallback read exposed incomplete synthetic fixtures: they contained only 36 MAP sectors. Tests now provide a separately bounded MAP plus the four-sector fallback window, matching the verified native load contract. The production bounds checks were not weakened to accommodate fixtures.

New tests cover primary shadowing, duplicate-row precedence, unknown record IDs, nonzero gates, truncated tables, teleport/elevation reservation, fallback-only source binds, and avoiding a copy anchor's unrelated fallback script. The prior independent allocation/readback regressions remain.

## Required dependency closure for a bound duplicate

An independently allocated descriptor is necessary but insufficient. A future supported bound-instance recipe must enumerate:

1. **Spawn reference:** independent descriptor and valid grid cell, retained placement flags, representable position/collider offsets, window reach and actor capacity. No unrelated anchor relocation.
2. **Binding:** independent trigger key and kind-1 row, correct primary/fallback precedence and derived grid markers. Check all consumers of the source binding.
3. **MAN context:** independent flat record identity, partition-specific header, script body and local state. Appending to P0 changes subsequent partition bases; update every affected flat reference or reject. A copied row pointing to the old context is not independence.
4. **Script graph:** outgoing and incoming cross-context targets, helper calls, motion streams, absolute coordinates, teleports, story/global flags, item rewards and wall writes. Shared flags/rewards require an explicit reuse policy; they must not be silently duplicated as independent state. Stop on unknown opcodes or unresolved dynamic targets. A linear decoder is an inventory tool, not proof of control-flow closure.
5. **Visual resources:** exact TMD selector, ANM clip and mesh hierarchy, TIM/CLUT/VRAM dependencies. Same-scene immutable resources may remain shared; mutable actor state must not. Cross-cell relocation still requires its own resource resolution.
6. **Collision and floors:** actor collider plus explicitly classified wall cells, shared users, writers, height LUT/corner inputs and elevation records. Door motion and arrival/teleport exemptions are behavior dependencies, not merely mesh transforms.
7. **Native budgets:** MAP descriptor/grid/trigger capacity, byte-sized trigger/context references, MAN offset encoding, partition rebasing, decompressed memory and compressed slot budget. Validate complete native readback before writing an image.

## Proposed ownership authoring/reconstruction contract

This is a design, **not an implemented export permission**. Unknown remains blocked.

Persist a versioned collision recipe in the scene document, attached to a stable SceneReference/BaseObject ID and exact source identity: disc fingerprint, MAP/MAN resource hashes, native cell/descriptor, descriptor bytes and relevant script ranges. Retain all baseline bytes separately. Reject stale recipes after any dependency changes; never silently reuse ownership across story variants.

Represent each relevant collision contribution as one of:

- `Unknown`: default for legacy walls/surfaces; cannot be removed, relocated or claimed.
- `SceneTerrain`: explicitly reviewed retained scene contribution.
- `ExclusiveInstance`: explicitly authored contribution with source-bit preconditions and evidence.
- `SharedInstances`: set of contributing references; removing one cannot remove another's contribution.
- `ScriptWritten`: exact writer sites, operation/mask, coordinates, control-flow conditions and shared consumers. A write is not exclusive ownership.

Maintain actor collider data separately. Surface recipes include the necessary neighboring tier samples, LUT values, grid flags and kind-2 primary/fallback rows; they cannot be represented as a wall stencil.

Authoring should display the exact grid cells and all known script writers/adjacent contributions alongside the native actor collider. Bounds may choose the initial viewing region, never the owner set. Require an explicit per-bit/per-surface decision and evidence notes. Label authored ownership as authored, not recovered retail metadata. Record external runtime acceptance separately; user-provided emulator observations can validate behavior but do not prove absence of all hidden script consumers.

For reconstruction, accept authoritative upstream source data if available, or a reviewed native script/resource dependency closure. Do not infer ownership from connected components, adjacency, mesh bounds, similar objects, or a single runtime state. If evidence is incomplete, preserve the unknown baseline contribution and reject movement that requires removing it.

Compilation should recompose the union of retained baseline terrain, remaining owners and transformed explicit contributions. Moving an exclusive owner clears only its proven contribution; shared and unknown bits remain. A copy keeps the original owner's contribution and adds a new owner. Require exact native grid quantization for wall/surface changes; no guessed rounding. Resolve overlaps by contributor union for solid bits, but reject incompatible surface heights or scripted clear/set precedence until semantics are explicitly defined. Recompose each supported story state independently and test against source preconditions.

Required acceptance tests include stale fingerprints, partial-bit ownership, overlapping/shared owners, unknown neighbors, multiple script writers and flag branches, floor-corner dependencies, elevation overrides, boundary/quantization failures, undo/redo and failed restoration, deterministic compilation, original-resource preservation, and native readback. Only a complete recipe may replace the current unknown-ownership rejection.

## Suitable bounded tasks for Sol

These are proposed tasks, not delegated work:

1. Add recipe persistence and transactional editing with no export unlock; test versioning, stale source identities and undo/redo.
2. Implement contribution-union compilation against synthetic maps, preserving unknown/shared bits. Keep it disconnected from retail export until explicit validated recipes exist.
3. Build a read-only dependency report using the existing decoder: instruction boundaries, incoming/outgoing IDs, wall writes, coordinate operands and unknown coverage. Stop on decode errors; do not resynchronize and claim completeness.
4. Add byte-exact fixtures for physical fallback precedence and original-reference preservation; no engine modifications or disc export required.

Higher-uncertainty work requiring further native analysis: proving full MAN context relocation/rebasing, closing Vahn's House behavioral dependencies, authentic wall/surface reconstruction, and runtime actor/spawn-budget validation. Those should not be assigned as routine serialization tasks.

## Current boundary

Final validation: `cargo build -p legaia-studio` passed; `cargo test -p legaia-studio --lib` passed with **139 passed, 0 failed, 4 ignored**. The opt-in read-only retail binding investigation passed separately. Existing compiler warnings remain. The physical-fallback integration test verifies that the complete planner rejects a hidden fallback binding. No production image was written. Retail SHA-256 remains `750dbcbadb70362feb3468467bbe7efbf3d151a414caa428c66c368d4b0b73e6`.

Native binding provenance and allocation safeguards are improved. Complete collision ownership, independently relocated bound contexts and a fully functional retail duplicate remain unproven. No test IMG is ready, and no collision-preservation claim is made for unknown ownership. GUI, camera, rendering and controls were not edited. Retail input remains read-only; no commit, branch change or push was performed.
