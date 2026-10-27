# Native texture relocation: bounded fixture milestone

Baseline: `432cb3df928da51bcf42045846af58058d5a91fc`. No partial source changes existed after the interrupted request. Changes here remain uncommitted. No foreign-resource IMG transplantation is enabled.

## Implemented contract

The reusable planner lives beneath `native_asset_plan`, consumes its exact ResourceId/payload/provenance registry, and produces an isolated in-memory RelocationPlan. It accepts one complete 4-bit, 8-bit or 16-bit TIM plus one or more complete native TMDs whose textured primitives are fully supplied by that TIM's image and, for indexed modes, its own CLUT. A multi-palette CLUT rectangle is retained in full. Arbitrary source/destination scene names and resource IDs are supported; no town/object exception selects allocation or rewriting behavior.

This is deliberately narrower than a scene importer. TMDs needing external images/palettes fail with an explicit dependency-closure error. A scene-local resource inventory is not sufficient proof of the effective runtime upload order or free VRAM.

### Allocation and preservation

- A caller supplies destination VRAM and additional protected rectangles for framebuffer, shared resources and runtime use. All uploaded destination words are protected, including transparent zero words. The planner does not infer that an unuploaded word is safe for a live game's framebuffer or future uploads; those regions must be reserved by the caller before a future scene packer uses this API.
- Automatic placement is deterministic first fit, with exact compatible blocks reused first. Explicit image/CLUT destinations are also supported. Both allocation modes reject overlap, invalid rectangles, incompatible resources and exhaustion.
- Images translate by multiples of 64 VRAM words horizontally and 256 rows vertically, preserving UVs and the source origin's page-relative coordinates. Every referenced page must remain encodable; only the low five TSB location bits change. Texture depth, ABR and all other TSB bits remain unchanged.
- CLUT rectangles translate independently with 16-word alignment. Each primitive's palette offset within the rectangle is retained, including CBA's non-location bit. Image/palette self-overlap, wrapped palettes and references to incomplete/external palettes are rejected rather than reinterpreted.
- The planner shares only its own allocated blocks with equal dimensions, payload bytes, role/depth and compatible alignment. It never treats arbitrary protected destination words as available merely because their current pixel values match. Equal byte counts with different rectangle shapes do not alias. Equal indexed pixels with different palettes can reuse the image while retaining separate palette allocations and CBA values.
- Entire image and palette payloads are unchanged. TIM flags, dimensions and other metadata remain unchanged; only the image/CLUT load coordinates are patched. No RGBA conversion, palette quantization, transparency approximation or pixel re-encoding occurs.
- Planning stages allocations on a clone. Failure commits no allocations or VRAM changes. A sealed plan applies only to the exact input VRAM pixel/coverage fingerprint, rejecting stale destination state. It returns an in-memory VRAM result, not native scene/image bytes.

### Native primitive rewriting and verification

- TMDs use the existing native parser, strict Legaia primitive group iterator and verified Descriptor table. Texture offsets come from Descriptor.texture_block_offset, covering the distinct lit/baked layouts rather than the previously invalid vertex-offset subtraction heuristic.
- Unknown descriptors, truncated texture blocks, invalid vertex references, incomplete primitive counts, aliased texture fields or fields overlapping geometry/header data reject planning. The implementation never silently switches to the lenient walk or drops unknown primitive groups.
- Raw TMD bytes are copied and only the CBA/TSB halfwords are patched. Geometry, normals, colors, group flags/modes, topology, UVs, padding, transparency-enable attributes and every unrelated byte remain unchanged.
- Rewritten TMDs are reparsed. Every UV bounding-rectangle sample is compared through the existing PS1 VRAM texel reader, retaining raw BGR555/STP results. Readback has an explicit eight-million-sample ceiling; exceeding it rejects the transaction rather than skipping validation.
- Plans retain the donor TIM identity/provenance, and rewritten TMD payloads remain keyed by their original ResourceIds. Sharing resource payloads does not change the existing per-instance UIDs or merge scene references.

## Exact changed files

1. `crates/studio/src/app/native_asset_plan.rs`: registers the isolated relocation submodule only; existing resource inventory and category gates unchanged.
2. `crates/studio/src/app/texture_relocation.rs` (new): deterministic allocation, TIM/TMD patch planning, provenance and readback.
3. `crates/studio/src/app/texture_relocation_tests.rs` (new): six synthetic regressions and one explicit read-only native fixture.
4. `crates/studio/src/app/ps1_export.rs`: widens visibility of the existing vram_texel helper to its parent app module for reuse; its implementation and all export safety gates are unchanged.
5. `docs/tooling/lck-p1-native-texture-relocation.md`: this report.

No new dependencies, GUI work, naming/persistence/collision changes, native investigations, MAN/NPC changes or directory reorganization. No NativeExportPlan writable-image integration was added.

## Validation

- Focused relocation tests: **6 passed, 0 failed, 1 opt-in ignored**.
- Full library suite: `cargo test -p legaia-studio --lib --locked --message-format short`: **180 passed, 0 failed, 11 ignored**, with filesystem access required by existing persistence tests.
- Synthetic cases cover 4/8/16-bit images, complete palettes, texture and palette relocation, preserved raw pixels/STP/metadata, all baked FT3/FT4/GT3/GT4 shapes, untextured primitives, exact-resource reuse, same geometry/different palettes, shape-sensitive allocations, protected uploaded zero words, reserved regions, image/palette conflicts, exhaustion, transactional failures, stale hashes, unknown GPU depth/primitive formats, external texture/CLUT dependencies, trailing/truncated TIM data, incomplete TMD counts and geometry aliases.
- Explicit read-only native fixture: **1 passed**. Uses Biron Monastery TIM at PROT 55 resolved offset `0x3e18`, a real 4-bit image of 64x256 VRAM words. The native TIM relocated to image (0,0), CLUT (64,0) in empty fixture VRAM, preserving all image/palette bytes. It uses a synthetic native-format TMD referencing that real TIM; this is not a claimed complete retail-model transplant. Separately, strict texture-reference walks accepted all 16 sampled complete native TMDs; none was silently truncated or rejected in that sample.
- `cargo build -p legaia-studio --locked --message-format short`: passed. Existing compiler warnings remain; the new isolated planner APIs also produce expected unused-code warnings because live export intentionally does not call them.
- Whitespace/diff review passed. Retail disc SHA-256 remains `750dbcbadb70362feb3468467bbe7efbf3d151a414caa428c66c368d4b0b73e6`. No IMG was generated, existing image overwritten, emulator operated, branch changed, commit created or push performed.

## Remaining limitations and next milestone

Unsupported: 24-bit/mixed TIMs, reserved GPU depth, wrapped/out-of-bounds CLUT uploads, partial-image/partial-palette transplantation, multi-TIM primitive dependencies, UV adjustment/atlas repacking, unsupported native primitive descriptors and live scene upload-order changes. Complete blocks may conservatively fail to fit even if a more sophisticated packer could use fragmented VRAM.

The planner does not discover framebuffers/reserved runtime memory, close shared/animated upload dependencies, register new native model selectors, repack/compress scene resources or rewrite scripts. Collision, walkability, actor state, native scene packing and existing export gates remain separate unresolved requirements. No new real-disc export capability or emulator behavior is claimed.

**Smallest next Sol Medium milestone:** fixture-only multi-TIM dependency closure: resolve each native primitive's image words and full palette against verified ordered donor uploads, build a deterministic region/reference map, explicitly detect overwritten or shared/animated uploads, and extend relocation/readback across multiple source blocks. Preserve exact primitive bytes and reject unresolved upload order. Only after that and native resource registration/packing are validated should this layer be considered for NativeExportPlan integration.
