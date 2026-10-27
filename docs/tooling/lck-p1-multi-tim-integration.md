# Multi-TIM closure and native package fixtures

Baseline `432cb3df`; all preceding uncommitted single-TIM work is retained. No commit, IMG output or live scene registration.

## Working increment

- `multi_tim.rs`: explicit ordered upload replay with per-word provenance/overwrites, partial image/CLUT operations and merge-zero handling. Sequence gaps/unknown order reject. Full pixel **and upload-coverage** replay must match supplied reference VRAM. A successful fixture replay is not proof of an arbitrary retail scene's chronological uploads; TIM scanner order is never promoted to native evidence.
- Primitive closure resolves every texture word in each UV bounding rectangle and the complete 16/256-word palette. Separate/shared palettes, complete overwrites and palettes supplied through ordinary image uploads are supported. Missing external data, image depth mismatches and fragmented/composite dependencies reject explicitly. Complete-block transplantation remains the bounded contract.
- The existing allocator relocates all required blocks on staged state. Source hashes, all primitive attributes/UVs, palette/STP values and sample equivalence are checked. Failure leaves prior allocations unchanged. Equal immutable blocks reuse destinations without changing object UIDs; every source resource retains provenance. Identical emitted native uploads are deduplicated while provenance records remain separate.
- `MultiPlan::package` now writes real native resource **buffers**: valid TIM image uploads and native count/word-offset TIM/TMD packs. Palette runs use a 16-bit TIM image envelope containing their exact original palette words at the relocated CBA address; this derived envelope is not claimed byte-identical to the donor TIM header. Original image depth and primitive blend flags remain preserved. Pack offsets/bytes are read back with the existing native container parser, TIMs are reparsed/replayed, and resulting VRAM must match the relocated plan. Generated packs can be decoded and fed through closure again.
- Package model slots are local immutable-pack indexes, **not assigned runtime mesh selectors**. There is no connection to writable NativeExportPlan or disc output.

## Single-TIM preservation correction

The existing `Vram::upload_tim_partial_opts` consumer documents and implements native CLUT `w*h` entries as one linear row, not a rectangular w-by-h upload. The single-TIM planner now uses that same footprint while preserving donor TIM dimensions/bytes. A new 8x2 stored CLUT / 16x1 consumed-run regression covers this correction. Out-of-row/clipped palettes remain unsupported. All six prior single-TIM regressions remain green.

## Exact scope

This increment modifies `crates/studio/src/app/texture_relocation.rs` (linear CLUT footprint and submodule), `texture_relocation_tests.rs` (test helper visibility only), and adds `multi_tim.rs`, `multi_tim_tests.rs`, and this report under `docs/tooling/`. All Rust files are in `crates/studio/src/app/`.

Previously uncommitted `native_asset_plan.rs` module registration, `ps1_export.rs` sampler visibility, the single-TIM implementation/tests and `docs/tooling/lck-p1-native-texture-relocation.md` are retained. No unrelated tracked source, GUI, naming, collision, scene-save, camera or export-gate edits.

## Validation

- Full library suite: **185 passed, 0 failed, 12 ignored**, with required filesystem access for existing metadata tests. Five new standard regressions cover multiple images, shared/separate/equal palettes, overwritten sources and order failures, missing/fragmented dependencies, linear CLUTs/merge-zero rejection, stale resources/destinations, reuse, rollback, native package parse/replay and re-import.
- Explicit read-only cross-town fixture passed. Authentic Biron Monastery TIM PROT 53 +`0x519c4` produced a 1,108-byte TIM pack and 172-byte TMD pack; Hunter's Spring TIM PROT 33 +`0x4528` produced a 33,332-byte TIM pack and 172-byte TMD pack. Both use synthetic primitive references to authentic texture bytes and a single-TIM consumer replay per source; this does not claim full retail-model portability or observed multi-upload scene chronology.
- Studio build passed. Existing and expected unused isolated-planner API warnings remain. An initial ordering test incorrectly expected identical writes to reveal chronology; the test was corrected to use noncommuting writes. Reuse tests now use distinct image data where separate-image allocations are expected. No failing regression remains.
- Retail SHA-256 unchanged: `750dbcbadb70362feb3468467bbe7efbf3d151a414caa428c66c368d4b0b73e6`. No emulator, image output/overwrite, push, reset or checkpoint.

## Immediate integration boundary

Verified existing components: `container_formats::pack::{parse_pack,extract_pack}` defines the word-offset asset pack; `scene_asset_table::{resolve,decode_slot_by_type,encode_size_word}` exposes native descriptors and decompressed payloads; `scene_bundle::BundleSource` handles plain/scripted/embedded tables; `disc_patch::compress_within` and the existing native plan provide constrained compression/sector/resource readback. These can be reused rather than replaced.

However, `scene_assets::SceneTmd` explicitly describes scanner hits pending runtime registration-order resolution; their discovery index is not proof of the pointer-table slot installed through `FUN_80026B4C` at `0x8007C018`. The asset loader also distinguishes field/battle, targeted partial uploads, shared blocks and CLUT merge policy. No existing scanner reconstructs complete verified native upload chronology. This run does not invent either fact.

Thus verified **fixture packages** can be constructed, but a playable IMG still requires: (1) actual source upload/registration trace or an authoritative order/policy decoder; (2) destination model-selector assignments tied to MAP references; (3) validated descriptor/payload repacking with native RAM/compressed-footprint limits; and (4) the unchanged collision/walkability/script gates.

**Smallest next step:** for one verified plain bundle format, implement an in-memory descriptor adapter that decodes its existing TIM/TMD slots, preserves all unrelated descriptor payloads, appends a fixture package, and roundtrips the native loader's actual slot-registration mapping with explicit size budgets. If that mapping cannot be verified, stop at an exact trace request rather than write a disc. Composite palettes/fragmented texture regions require a separate provenance-preserving block materializer before they can enter that adapter.
