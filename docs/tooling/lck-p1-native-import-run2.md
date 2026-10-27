# Native foreign import — Run 2 results

Continues `432cb3df` plus all retained uncommitted Run 1 work. This report
supersedes Run 1's TIM metadata and archive-growth blockers. No IMG was saved.

Files changed in this run (all previous uncommitted files retained):

- `crates/tim/src/lib.rs`: native-upload parse policy, metadata regression and pre-allocation input check.
- `crates/disc-patch/src/disc.rs`: bounded transactional growth and raw-prefix regression.
- `crates/studio/src/app/native_asset_plan.rs`: expose existing resource registration to the native compiler.
- `crates/studio/src/app/texture_relocation.rs`: app-internal planner/rectangle visibility.
- `crates/studio/src/app/multi_tim.rs`: explicit native traces and upper-flag-preserving upload envelopes.
- `crates/studio/src/app/scene_registration.rs`: verified native upload sequence and package metadata acceptance.
- `crates/studio/src/app/ps1_export.rs`: saved-instance compiler, combined candidate, cross-cell floor correction, archive growth and resource-sector integrity checks.
- `crates/studio/src/app/scene_registration_tests.rs`: real native donors, combined MAP/PAC serialization, saved-scene restoration and growth coverage.
- This report, `docs/tooling/lck-p1-native-import-run2.md`.

## Implemented and proven

### Retail TIM metadata

Read-only cross-check of `SCUS_942.54`, `FUN_800198E0`:

| Address | Instruction / meaning |
| --- | --- |
| `0x80019948` | `lw s3,4(s2)` — original TIM flags |
| `0x80019950` | `andi v0,s3,8` — CLUT presence |
| `0x80019974..0x8001999C` | Flatten CLUT width × height to one upload row |
| `0x80019A28` | `andi v1,s3,3` — pixel-mode selection |
| `0x80019998..0x800199EC` | Separate global `DAT_8007B998` can force STP on nonzero palette words |

The upload routine does not consume upper flag bits. This establishes their
upload interpretation, not their meaning in every other native subsystem.
`legaia_tim::parse_native_upload` preserves the full flags word and validates
strict block extents/dimensions; `parse_strict` remains unchanged for scanning
and ordinary strict TIM validation. Unsupported mode bit 2 remains rejected.
No source bytes are masked or normalized.

Ordered native traces fingerprint original TIM bytes. Repacked image/palette
upload envelopes retain source upper flags, exact pixel/palette words and
provenance. Primitive-reference rewrite, UV/blend preservation and sampled
texture readback use the existing multi-TIM machinery. The runtime STP latch
and complete shared/dynamic VRAM state remain acceptance requirements.

### PROT growth

The old TOC tail loop stopped at the first **extraction entry**, which is after
raw resident entries 0/1. It therefore treated resident resource bytes as sector
offsets; a large word overflowed at the old `disc.rs:260` addition. The writer
now validates the declared header extent, dense entry starts and final end
bound, shifts only live starts and that bound, and preserves opaque header
padding and the raw resident prefix. Arithmetic is checked. Reopening and
comparing every entry must succeed before replacing the caller's patcher.
Expanding a final entry across an opaque archive trailer is explicitly rejected.
Resource compilation now uses this writer when growth is necessary and the
existing in-place sector writer otherwise. EDC/ECC of affected resource sectors
is checked after packaging.

### Saved World Objects to native resource + placement candidate

`NativeExportPlan::compile_saved_imports` reads the same serialized scene
document as save/load, resolves donor identities and cached native descriptor
provenance, reconstructs verified descriptor/PAC upload order, relocates actual
resources, registers destination selectors and allocates independent MAP records.
Multiple donors use one deterministic evolving texture allocator. Unsupported
categories or additional transform/actor/visibility edits reject explicitly.

`compile_registered_copy` reuses the existing allocator against the **real donor
MAP**, checks donor and destination native bindings, resolves both native floor
ladders, rewrites only the new descriptor's selector, and verifies all unrelated
MAP bytes and donor records remain unchanged. Equal tier numbers in different
maps no longer imply equal floor heights. Visual overlap is not rejected; native
grid references remain independently allocated without moving original anchors.

The result is a **NativeImportCandidate**, not a writable NativeExportPlan.
Collision recipes remain pending and counted; normal export cannot accept or
write this candidate. The real `plan_export` safety path remains enforced.

## Real-native evidence

The opt-in test reads the retail image only and builds private image buffers:

| Genuine source | Source record/model | Standalone town01 allocation |
| --- | --- | --- |
| Biron `bylon` | #291 / model 10 | descriptor 509, grid (92,78), selector 114 |
| Hunter's Spring `izumi` | #36 / model 2 | descriptor 509, grid (92,78), selector 114 |

Each requests native world X/Z `(12096,10048)`, preserves native orientation
and scale, and carries actual source TMD/TIM/CLUT data. PAC logical size grows
from 368640 to 370776 bytes (then sector padding); native archive growth,
decompression, model lookup, texture samples, EDC/ECC and unchanged-entry checks
pass. Neither test claims gameplay-collision or RAM-headroom validation.

The combined test creates both imports through `WorldModel::insert_foreign`,
serializes, restores into a fresh world, and runs `compile_saved_imports`:

- UID 101 -> descriptor 509, grid (92,78), selector 114.
- UID 102 -> descriptor 463, grid (93,79), selector 115.
- Two independently retained collision recipes are pending.
- Normal `plan_export` rejects unresolved authored collision as intended.

No donor/object whitelist is used in production code. Named towns and records
are fixture results, not routing rules.

## Remaining rejection / unfinished integration

1. `ps1_export::plan_export`: the condition requiring both
   `world.collision.recipes` and `world.collision.mesh_recipes` to be empty still
   rejects these imported scenes. Required next native work: establish the
   object's actor-collider, MAP-wall and walkable-surface contract and compile
   reviewed contributions without attributing original walls from geometry.
2. No authoritative complete destination RAM headroom, global contributor
   census or shared/dynamic VRAM lifetime contract is supplied. Fixture budgets
   and protected rectangles are inputs, **not retail certification**. Preserve
   the index-200 rejection; passing it proves only absence of that known overlap.
3. Candidate promotion into the normal preflight/execution plan remains
   unimplemented until those contracts can be validated. Without authored
   recipes, the legacy `prepare_cross_cell` route still rejects a foreign model
   without an equivalent resident model. Do not remove that guard alone: route
   to the new compiler only through complete resource/physics/runtime acceptance.

| Category | Current result |
| --- | --- |
| Unbound static MAP objects/buildings/props | Genuine cross-cell resource + placement candidates serialize; playable IMG acceptance remains blocked |
| Terrain/raw meshes/walkable ground | Resource tools reusable; terrain/floor native writer and physical contract required |
| NPCs/animated actors | Existing native actor edits retained; foreign actor/animation adapter required |
| Bound/interactive objects | Explicit rejection pending verified MAN/animation/shared-state handling |
| Other bundle/PAC layouts | Fail closed until native registration/contributor adapter exists |

No experimental IMG is emitted: remaining uncertainty includes **required
physical behavior**, not just whether a validated visual object appears.

## Validation and reproduction (Windows CMD)

```bat
cd /d D:\AstraRimElmProject\legaiasource\legend-of-legaia-re-main
cargo build -p legaia-studio --locked
cargo test -p legaia-studio --lib --locked
cargo test -p legaia-tim -p legaia-disc-patch --lib --locked
set "LCK_P0_SOURCE=D:\Emulators\Games\Legend of Legaia\LegendOfLegaia.img"
cargo test -p legaia-studio --lib --locked real_town01_pac_packaging_and_native_donor_gates -- --ignored --nocapture
```

Studio: 194 passed, 0 failed, 14 ignored. TIM: 59 passed. Disc patch: 22 passed.
The real-native fixture also passed separately, including multi-source scene
restoration. An initial full TIM suite aborted because an existing malformed-
dimension test reserved ~17 GB before checking source length; the narrow
pre-allocation input-size check fixes it, and the full suite now passes.
Other initial compilation/fixture failures were corrected and rerun.

No safe foreign-IMG export command exists yet. No original or previous test
image was changed; no DuckStation operation, Git staging, commit or push.
