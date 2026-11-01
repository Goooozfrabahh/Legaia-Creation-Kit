# B2B combined donor interaction investigation

Parent: `e5eefff0434d1856c443788c76224444ea9504f3`.
Branch: `lck/stage-b2b-loading-repair`.

## User evidence and current conclusion

The user reports A (retail), B (Balden) and C (Bubu1) LOAD; D (three
References) and E (two donors without duplicate) FREEZE. The duplicate is
not necessary. The user subsequently clarified that DuckStation itself becomes
unresponsive, preventing another saved-state capture. These are user reports,
not assistant-operated tests. Loading alone does not verify animation playback.

No exact cause or writer defect has been established. No production writer
correction or corrected-gameplay claim is made. This remains an investigation
checkpoint. No donor-specific compatibility rule or animation removal was added.

## Exact B/C/E comparison

| Property | B | C | E |
|---|---:|---:|---:|
| Native scene models / including five residents | 115 / 120 | 115 / 120 | 116 / 121 |
| Added scene model selector | 114 | 114 | 114, 115 |
| Added global model slot | 119 | 119 | 119, 120 |
| Added ANM ID | 70 | 70 | 70, 71 |
| P1 count | 54 | 54 | 55 |
| New global MAN context | 89 | 89 | 89, 90 |
| Decoded model pack bytes | 310900 | 313500 | 320284 |
| Decoded MAN bytes | 45383 | 45383 | 45428 |
| Decoded ANM bytes | 97668 | 97668 | 98888 |
| Encoded bundle footprint | 227328 | 229376 | 231424 |
| Encoded PAC footprint | 370688 | 368640 | 370688 |
| Persistent dispatcher request bytes | 466516 | 469116 | 477164 |

Introducing Bubu1 after Balden adds 9,384 decoded model-pack bytes (9,380
model bytes plus one offset), one 45-byte MAN record/table contribution, and
1,220 ANM bytes (1,216 clip bytes plus an offset). Aligned persistent requests
rise by 10,648 bytes. This is not a live-memory failure proof.

The original 114 model members, all 69 original clips, MAP/collision and all
six MAN section payloads are identical in B/C/E. Balden's added model/clip
remain exact. Bubu1's added clip remains exact. C-to-E Bubu1 model differences
are precisely 16 bytes, each changing a CBA low byte from `0x34` to `0x36`:
the 16-color palette moves from `(832,0)` to `(864,0)`. Its palette words are
identical (SHA256 `1106d36fe8901cda8184d1f1c17a586677ccb9f98942d76a034f688258ed0a33`).
Its two image uploads retain identical bytes and coordinates `(864,192,8,31)`
and `(864,224,8,15)`. Balden's five added uploads are retained; the three Bubu1
uploads follow them. The added rectangles do not overlap. Original descriptor
type order is unchanged: `2,3,5,6,7,20`. Source offsets change with recompression.

Model pointer-table stores are `0x8007C1F4` (global119) and `0x8007C1F8`
(global120). Original instruction registration and cumulative construction
retain both pointers; no conflict at slot120 was reproduced. This is not proof
that every later engine consumer supports every registered slot.

## Extended original-instruction verification

The new `execute_original_machine` test helper retains the exact CPU from the
existing original descriptor/PAC walker, dispatcher, registrar and pointer-fixup
execution. `combined_construct` continues on that same machine through:

- Original 143-node free-stack initialization and seven list-sentinel allocations.
- Real native free-stack/list allocation for each imported actor, without
  returning a manufactured actor node.
- Original installer, initializer, model-object table construction, native ANM
  binding, the real field-overlay spawn prologue and installer return.

B/C/E and every diagnostic variant pass. E's pool top changes 135 -> 133,
with distinct actor addresses and 156-byte per-actor construction allocations.
Each actor's object pointers address its registered TMD objects, and its ANM
pointer addresses the exact selected bank record. The model pointer table and
earlier actor state survive subsequent construction unchanged. Native list links
are allowed to change when another actor joins the list.
The checked per-actor state range is `+0x10..+0xBF`; pool links are excluded.

Limits: heap addresses and LZS/GPU/GTE/RNG services remain substituted. This is
not the complete scene initializer, original live allocator, CD hardware,
rendering, or gameplay. Current failures were not reproduced by this bounded
construction path. Existing ownership/deletion/retirement fixtures remain required.

## User-created E snapshot

The screenshot's selected address zero and zero-valued visible register panel
did not establish the running PC. The saved `SCUS-94254_1.sav` was copied
read-only into the investigation directory and decoded offline with Zstandard.
No emulator was launched, attached to, paused or otherwise controlled.

State version77, SHA256:
`4e58c04fbda95bac7b7498bf2bb0a2ee652903eb6bf2d237642f846a67b79513`.
The embedded media path names E. Parsing follows DuckStation's documented
serialization in [save_state_version.h](https://github.com/stenzek/duckstation/blob/master/src/core/save_state_version.h),
[CPU::DoState](https://github.com/stenzek/duckstation/blob/master/src/core/cpu_core.cpp),
[Bus::DoState](https://github.com/stenzek/duckstation/blob/master/src/core/bus.cpp)
and [CDROM::DoState](https://github.com/stenzek/duckstation/blob/master/src/core/cdrom.cpp).
The decoder explicitly accepts v77 only; it does not guess future layouts.

Observed snapshot state:

- PC `0x8005FD40`, RA `0x8005FC1C`, inside the original native wait/VSync path.
- Cached first4KiB of PROT TOC match E exactly; stale retail/B/C TOC is not supported.
- Resident MAN display name `Mt. Dhini`, partitions `[3,14,19]`, model count77.
- Transition actor `0x8008077C`, original handler `0x80021934`, state3.
- Pending raw TOC6 / extraction4 read, relative LBA243, 113 sectors requested
  and 113 remaining, destination `0x8015CBD0`. Buffer head does not yet match E.
- CD drive state `SeekingLogical`; source internal LBA28890, target635,
  setloc binary `[0,8,35]`. Raw sector485 on E has the matching BCD header
  `00:08:35`; B/C/retail have the same header at that entry start.
- Failed actor-spawn counter0; pool top82 describes the resident old scene.
- The real pending-read arena header declares `0x62C00` (404,480) bytes at
  `0x8015CBD0`, ending `0x801BF7D0`. The 231,424-byte pending bundle fits that
  actual allocation. The old scene has 214,920 free heap bytes in two blocks;
  that pre-cleanup state cannot certify or condemn later target allocations.

In the original SCUS, transition state3 polls `FUN_8003DE7C(1)`; that enters
the retail `FUN_8003F2B8` / `FUN_8005BE8C` CD-sync path. This snapshot has not
reached E's new model registration or imported actor installation. However,
because the application itself subsequently became unresponsive and a second
capture is unavailable, this snapshot may precede the actual hang. It is not
proof that a logical seek remains stuck forever, or that CD seeking caused it.
It neither establishes a heap failure nor a native model/actor ownership defect.

## Final diagnostic images and manual procedure

Use only `D:\AstraRimElmProject\ManualTests\LCK_B2B_Combined_20261031_01\Variants_Final`.
The earlier `Variants` directory is preserved intermediate evidence: its H
incorrectly retained C's model selector. Final H explicitly asserts global120
and clip71 in the original constructor. This was a diagnostic-fixture error,
not a proven production writer defect.

All final variants retain E's exact ANM bank, ordered PAC/TIM bytes and retail
MAP/collision. They are IMG-only hypothesis tests, not edited production projects
or corrected exports. All native actor identities remain independent.

| Case | Controlled change / purpose |
|---|---|
| M_BOTH_RESOURCES_NO_IMPORT_ACTORS | E's model/ANM/TIM resources, retail MAN; separates resources/transition from imported actor construction |
| G_BOTH_RESOURCES_BALDEN_ONLY | Both resource sets, only Balden actor instantiated |
| H_BOTH_RESOURCES_BUBU_ONLY | Both resource sets, only Bubu1 actor, using E's global120 and clip71 |
| I_TWO_BALDEN_E_RESOURCES | E resources and two live independent actors; deliberately both use Balden model/clip. This appearance substitution is diagnostic and explicit |
| F_REVERSE_MODELS | Reverse the two imported model members and matching actor selectors; ANM order unchanged. Recompression also changes encoded layout/footprint, so a result cannot uniquely distinguish order from layout |

After personally recovering/restarting DuckStation, use a fresh boot for each
case, the same in-game save and route, and preserve existing states/memory cards.
Recommended order: M, G, H, I, F. Do not load the newly saved freeze state as a
test starting point. Record separately: application responsive/unresponsive,
scene loads/freezes, actor appearance and visible animation. Loading is not
animation verification. If the application becomes unresponsive, no further
debugger operation is expected or required during that hang.

Interpret results cautiously:
M freezing supports a resource/transition issue without imported actor
construction. M loading with H failing narrows toward the second model/clip
binding or its material/render path. G/H loading while I and E fail supports a
two-live-actor/cumulative-state issue. I loading while E fails supports a distinct
model/clip interaction. F differing from E supports registration-order/encoded-
layout sensitivity. Small MAN/allocation size changes remain confounders.

## Evidence locations

Root: `D:\AstraRimElmProject\ManualTests\LCK_B2B_Combined_20261031_01`.
`B_C_E_NATIVE_COMPARE.json`, `EXACT_SHARED_RESOURCE_COMPARE.json`,
`ORDERED_TIM_TAILS.json`, `UserCapture\STATE_REPORT.json`,
`Variants_Final\COMBINED_REPORT.json` preserve the data behind this report.
The existing A-E IMGs, retail, Latest Build4476a72, projects, old checkpoints and
B2B recovery bundle remain protected. No production installation is authorized.

Reproduce the native matrix with `LCK_COMBINED_INPUT` naming the preserved A-E
directory (containing `ISOLATION_REPORT.json`), `LCK_COMBINED_OUTPUT` naming a new
create-only directory, and `LCK_AUDIT_SOURCE` naming retail:
`cargo test -p legaia-studio --release --lib --locked stage_b2b_combined_interaction_matrix -- --ignored --nocapture`.
The capture decoder is `scripts/inspect-duckstation-state77.py`; its Zstandard
dependency was installed only in the investigation's `Tools\python` directory,
not globally or into LCK/DuckStation. Existing capture copies are never replaced.

## Completed automated verification

- Final Studio library harness: **324 passed, 0 failed, 68 ignored** by default.
- Code hooks: **89 passed, 0 failed, 1 ignored**.
- Full B2B runner: all **28 opt-in regressions passed**, including Stage A,
  B1/B1.1, Bylon, five-object, R8/owned collision, project reopening, native
  resource closure and shared-resource retirement. Editor source check passed.
- Final combined original-instruction matrix: passed for B/C/E and F/G/H/I/M.
  H's native global model120 and ANM71 are explicitly asserted.
- Original SCUS decompressor: all five final diagnostic images passed.
- Independent structural and sector checks: all five passed. Each has 125 new
  payload sectors with valid EDC/ECC; moved payload/parity bytes match retail.
- A-E hashes unchanged; protected retail, Latest Build, project, original failed
  B2B image and recovery bundle hashes unchanged.

Regression manifest:
`D:\AstraRimElmProject\ManualTests\LCK_B2B_Combined_20261031_Regression_01\REGRESSION_RESULTS.json`.
Final image checks are `FINAL_SECTOR_VALIDATION.json` and
`FINAL_STRUCTURAL_COMPARE.json` in the investigation root. The runner's separate
regression `Candidate` is the historical three-reference fixture, not a correction.

No assistant-operated GUI/gameplay verification occurred. User-reported A/B/C
loading successes and D/E failures remain distinct from these automated results.
The new diagnostic images await user-operated testing after the user recovers
the unresponsive application. No successful combined-donor runtime result exists.
