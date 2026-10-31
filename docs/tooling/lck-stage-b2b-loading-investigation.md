# B2B loading failure investigation

Starting checkpoint: `34bb3502e37dc7f92a273ee2e2d8910f10cf539f`.
Investigation branch: `lck/stage-b2b-loading-repair`.

## Verification status

The user reports that the original B2B experimental IMG froze loading Rim Elm
in DuckStation. This is **failed user-reported native gameplay verification**.
Prior automated export/readback and partial original-instruction tests do not
override that result. No B2B image is runtime verified. No assistant-operated
GUI, emulator or sandbox test has occurred.

Original failure image SHA256:
`443bde32cbab09c8d53567c15112bf00e248250ec0f9eb917249ee3afab58703`.
Its project, recovery bundle and old development branch remain preserved.

## Additional instruction tests

The earlier actor test stopped at `0x8003A474`, before the spawn-script pre-run.
The new test continues the original SCUS instructions into the actual field
overlay (retail PROT 897, loaded at `0x801CE818`), through the authored prologue,
until `0x8003A4FC`. All three imported records complete, with script PC 41.
This includes real opcode dispatch and immediate transform writes. Actor/node
allocation, the initial GTE helper and RNG are still substituted. The full
scene initializer, allocator pressure and rendering are not established by this.

The original SCUS LZS decoder also reproduces all five compressed descriptors
from the failed image exactly, with zero bytes written into output canaries.
This rejects a decoder-output-overrun explanation for those payloads; it does
not certify the rest of the loader.

## Isolated images

The create-only `stage_b2b_loading_isolation_matrix` library fixture uses the
untouched retail image and a read-only copy of the original failed project.
It uses the same destination, donors, transforms and export pipeline:

| Case | Active imported References | Purpose |
|---|---|---|
| A_CONTROL | none | Distinguish the common export pipeline from animated imports |
| B_FIRST_BALDEN | first Balden only | Isolate donor model/ANM family one |
| C_SECOND_BUBU1 | Bubu1 only | Isolate donor model/ANM family two |
| D_EXACT_THREE | Balden, Bubu1, Balden duplicate | Preserve the exact failed arrangement |
| E_TWO_UNIQUE | Balden and Bubu1 | Separate combined resources from the shared-resource duplicate |

The fixture records exact IMG hashes, distinct P1/global context identities,
original native registration traces and allocation requests. It checks retail
MAP/collision bytes, original model prefix and original ANM record preservation,
exact donor clip readback and saved project reopen. These remain structural and
bounded instruction tests, not gameplay tests.

Completed output directory:
`D:\AstraRimElmProject\ManualTests\LCK_B2B_Loading_20261031_01`.
`ISOLATION_REPORT.json` records the exact paths and hashes. A is byte-for-byte
retail (`750dbcb...`); D reproduces the entire failed IMG exactly (`443bde32...`),
not just its MAN. The other hashes are B `dc84ddf7...`, C `9b13a75a...`,
E `7b6f40ef...`. These are diagnostic images, not corrected-image claims.

Independent retail/failed/R8 structural comparison is retained under
`D:\AstraRimElmProject\ManualTests\LCK_B2B_Heap_20261031_01`.
Retail/failed MAN counts are `[36,53,39]` / `[36,56,39]`. All six original
section payloads are unchanged. Five original script operands changed by +3
under the P2 remap; complete byte differences are in `MAN_ORIGINAL_RECORD_DIFFS.json`.
R8 retains the retail MAN counts and ANM bank, and uses its earlier static
resource/TIM packaging. Its successful historical gameplay does not test the
new animated-reference startup path.

The failed image adds 16,168 decoded model-pack bytes, 135 MAN bytes and 2,440
ANM bytes. Persistent dispatcher requests rise from 458,468 to 477,212 bytes.
This 18,744-byte increase excludes actor/render costs and fragmentation.
Original allocator replay succeeds for all five cases on the historical
arena-return RAM fixture (D ends with 355,160 free bytes); **all five, including
retail A**, fail the first allocation on the historical preload fixture. Neither
snapshot therefore identifies this freeze. Exact request/return ledgers and
`HISTORICAL_HEAP_SUMMARY.json` preserve the evidence and its limitations.

Reproduce the new create-only fixture with `LCK_AUDIT_SOURCE` set to retail,
`LCK_FAILED_IMG` / `LCK_FAILED_PROJECT` set to the preserved original candidate
and project, and `LCK_ISOLATION_OUTPUT` set to a **new** directory:
`cargo test -p legaia-studio --release --lib --locked stage_b2b_loading_isolation_matrix -- --ignored --nocapture`.
The `stage_b2b_failed_image_startup_probe` fixture needs `LCK_FAILED_IMG` only.
`scripts/audit-native-scene.py` and `scripts/replay-native-heap.rs` provide reusable
read-only scene comparison and bounded historical allocator replay.

## Completed automated regression verification

Studio: 324 passed, 0 failed, 67 ignored by default. Code hooks: 89 passed,
0 failed, 1 ignored. All 28 configured opt-in regressions passed, including B2B,
Stage A/Base Form/Reference, B1/B1.1, Bylon, five-object and owned-collision/R8
coverage. Editor source check passed. New isolation and failed-image startup
probes passed; the original decompressor also passed on each of A/B/C/D/E.
Independent sector verification passed for all five outputs: new sector payloads
have valid EDC/P-Q parity (or valid/optional XA Form-2 EDC); relocated payloads
and stored parity match original sectors exactly, excluding the MSF address.
`SECTOR_VALIDATION.json` records the checked counts. The initial verifier's
Form-1-only assumption rejected an unchanged relocated XA sector; supporting
the actual Form-2 format resolved that diagnostic-tool limitation. It was not
evidence of an IMG writer defect.

Logs and statuses:
`D:\AstraRimElmProject\ManualTests\LCK_B2B_Loading_20261031_Regression_01\REGRESSION_RESULTS.json`.
No new manual GUI or successful native gameplay verification occurred.

## User-operated test procedure

Use a fresh DuckStation boot for **each** image, with the same in-game memory-card
save and route into Rim Elm. Do not resume a save state: it can retain an earlier
image's TOC, resource pointers or heap. Preserve existing saves and use a copy of
the memory card for experiments. Confirm the loaded disc filename for each case.

Test retail first as a baseline, then A, B, C, E, D. Record whether the loading
transition completes, whether walking and existing actors work, whether each
imported actor appears and animates, and whether entering/leaving the scene works.
Do not infer animation success merely from a completed loading screen.

If a case freezes, retain its DuckStation log. If available, pause it manually in
the debugger and record the CPU PC and RA, nearby instructions, free-stack top at
`0x8007C348`, failed-spawn counter at `0x8007B8A0`, model count at `0x8007B774`,
ANM pointer at `0x8007B888` and MAN pointer at `0x8007B898`. An optional RAM dump
allows original-allocator and pointer analysis without an assistant host launch.

## Remaining causal boundary

No exact failing native instruction or live allocator result has been supplied
for the B2B freeze. Allocation chronology and static remaining-byte estimates
cannot prove a live heap fits. Historical R6/R7 allocator evidence belongs to
different images/control flow and must not be relabelled as this failure.
Actor-pool exhaustion, heap fragmentation, resource lifetime, full scene-script
initialization and render-time pointer behavior remain open until isolated tests
or a failure capture distinguish them. A guessed writer change is not a repair.

No writer defect or gameplay fix is claimed by this investigation checkpoint.
The intended repair objective remains incomplete. There is no defensible exact
or most-likely native cause yet; a fresh-boot isolation result or frozen PC/RA
is needed to narrow it further. The user's answer "yes" to the earlier combined
boot/save-state question did not disambiguate which boot method was used.
