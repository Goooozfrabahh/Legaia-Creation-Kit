# Run 3 — imported collision serialization

Continues the uncommitted Run 2 tree. No IMG, emulator operation or commit.

## Native evidence versus authoring

| Source | Native record evidence | Automatic mesh recipe | Still unknown |
| --- | --- | --- | --- |
| Biron `bylon` #291/model 10 | cell (28,102), world (3648,13120), flags 0x16, collider-center offset (0,0), source floor tier 0 | 14 floor candidates, 28 steep faces; no explicit wall/open corrections | live actor class/extents/enabled state, object-owned walls, walkable-surface needs and script writers |
| Hunter's Spring `izumi` #36/model 2 | cell (29,8), world (3712,1088), flags 0x36, collider-center offset (0,0), source floor tier 0 | 31 floor candidates, 66 steep faces; no explicit wall/open corrections | same requirements |

The floor tier is native placement-height data. Mesh floor triangles do not
prove an engine walkable surface. Zero collider offset and absence of a direct
kind-1 binding do not prove a nonblocking object. Both default recipes remain
`MeshDerived`, not verified collision. No ownership was inferred from bounds.

## Implemented writing

`collision_recipes::native_import::compile` verifies original donor MAP, MAN,
descriptor and grid-reference provenance, associates each contribution with a
stable imported UID and its actually allocated destination descriptor, and
rebases explicit authoring into a single destination revision. Original saved
recipes and donor provenance remain intact; destination binding is a temporary
compiler context, not invented retail ownership metadata.

Only explicit `UserAuthored` intent is eligible for this writer. `MeshDerived`,
`NativeObserved`, missing recipes and empty automatic proposals do not establish
native nonblocking behavior. User-authored emptiness is also not proof of native
nonblocking behavior; full physical acceptance remains independently blocked.

Generated geometry uses its saved pivot and the final serialized instance pose;
manual corrections use their original donor-grid frame. This avoids double
translation and mismatched imported coordinate frames. Unsupported manual
translation/vertical changes, stale provenance and boundary crossings reject.

The adapter calls the existing `WallAdditionPlan` once for the complete union.
Overlapping blockers share bits; open requirements conflict with existing native
walls or another blocker rather than removing them. It never clears any native
wall or changes low floor nibbles, object references or bindings.

`NativeImportCandidate::compile_collision`, called by `compile_saved_imports`,
connects the compiled wall plan to the same resource/placement image. It patches
a private image, reparses the MAP, checks resource-sector/MAP EDC/ECC and commits
the candidate change only after successful verification. Failure retains the
resource/placement candidate and reports unresolved collision intent. The exact
compiled wall plan and donor receipts are retained for readback.

The native fixture round-trips both imports through saved scenes. Default
automatic recipes remain blocked. A separate **explicit fixture authoring**
scenario (not human approval and not recovered retail physics) compiles **70
additive wall bits across 38 bytes** for UIDs 101/102. `plan.apply(baseline)`
equals the final native MAP exactly; an independent `FloorGrid` reader confirms
all floor indexes and original grid cells remain unchanged. Reviewed fixture
recipes are also saved/restored before compilation.

## Runtime protection added

The saved-import compiler now protects verified field display buffers, capture
pages and the native transition clear strip; it also reserves the existing
boot-resident UI upload footprint through `ProtIndex::system_ui_bundle` and its
verified uploader. Zero-valued uploaded words remain occupied. Caller-provided
reservations extend these defaults. This is not a universal free-VRAM map.

Measured serialization demand for two imports: scene pool 116 + resident head 5;
decoded TMD pack 304116 -> 307008 bytes; bundle descriptor total 463285 bytes.
These are demands, not approved RAM capacities. Added image uploads are native
word rectangles (880,128,16,64), (880,64,16,64); palette envelopes are
(832,0,16,1), (848,0,16,1). The index-200 guard remains unchanged.

## Exact remaining acceptance conditions

- `NativeImportCandidate::compile_collision` rejects default proposals until
  every imported UID has explicit authored wall intent. It does not promote
  authored intent into verified actor collision, walkability or script behavior.
- `ps1_export::plan_export` still rejects recipe-bearing documents before a
  writable export plan is produced. Candidate-to-production promotion is not
  enabled: complete physical and runtime contracts are absent.
- **Walkable-floor writing remains unsupported.** No independently owned native
  floor/corner/kind-2 surface definition has been established for either donor.
  Copying mesh triangles, overwriting a destination tier or inventing a LUT
  slot would damage/guess native behavior. Existing floor data and verified
  placement-height correction remain preserved; no floor contribution is
  silently accepted or discarded in a successful IMG.
- RAM needs actual allocation/free ordering, largest contiguous free blocks
  and high-water state at model load and later actor/primitive allocations.
  Actor free-slot availability and registry alias/extent evidence are also
  required. The model ceiling alone is insufficient.
- VRAM needs the complete relevant boot/scene/actor/script/CLUT/menu/transition
  write lifetime at the proposed addresses, including STP state. Existing
  immutable upload footprints are only part of that contract.

Use the companion `lck-p1-run3-runtime-capture.md` for a read-only, address-based
DuckStation capture procedure on the original game. No GUI/camera changes or
model-registry expansion were made. **No playable or experimental IMG was saved.**

## Files changed this run

- `crates/studio/src/app/native_collision_import.rs` — new native adapter + four regressions.
- `crates/studio/src/app/collision_recipes.rs` — module registration.
- `crates/studio/src/app/ps1_export.rs` — combined candidate integration, diagnostics and transactional sector writing.
- `crates/studio/src/app/texture_relocation.rs` — verified field/shared VRAM protection.
- `crates/studio/src/app/texture_relocation_tests.rs` — reservation regression.
- `crates/studio/src/app/scene_registration_tests.rs` — real-donor authoring, persistence, preservation and demand evidence.
- This report and `docs/tooling/lck-p1-run3-runtime-capture.md`.

## Validation / Windows CMD

Studio library: **199 passed, 0 failed, 14 ignored**. Real-native import fixture:
passed separately. Studio build: passed, with existing/dead-code warnings.

```bat
cd /d D:\AstraRimElmProject\legaiasource\legend-of-legaia-re-main
cargo build -p legaia-studio --locked
cargo test -p legaia-studio --lib --locked
set "LCK_P0_SOURCE=D:\Emulators\Games\Legend of Legaia\LegendOfLegaia.img"
cargo test -p legaia-studio --lib --locked real_town01_pac_packaging_and_native_donor_gates -- --ignored --nocapture
```

These commands validate native buffers without writing a game image. There is
no accepted foreign-IMG export command until the remaining contracts pass.
