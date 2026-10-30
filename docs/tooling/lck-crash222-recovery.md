# 222.img crash investigation and reversible recovery

## Manual results received after e082867

The user reports that **E also crashes while loading Rim Elm**. Subsequently
the user cold-boot tested **A and reports that A also crashes during loading**.
Neither result establishes the crash instruction or heap state. Later results:

| Case | User-reported native gameplay result |
| --- | --- |
| A | Freezes loading Rim Elm. |
| B | Loads; walking and duplicated/rotated geometry work. Vahn face textures are corrupted; original import collision persists after move/deletion. |
| C | Freezes loading Rim Elm, without seven collision recipes or duplicate/move/delete edits. |
| D | Not tested. |
| E | Freezes loading Rim Elm despite corrected character-atlas destinations and small texture staging. |
| R1 (E resources, exact corrected-parent MAP) | Freezes DuckStation; new placements and collision recipes are not necessary to trigger the failure. |
| R2 (first Geremi import on corrected parent) | Freezes loading Rim Elm. This does not prove Geremi alone is the cause: the corrected parent has not independently passed gameplay. |
| R3 (Koin4 only), R4 (two Geremi copies) | Not tested. |

R1–R4 are under `ManualTests\RuntimeRecovery_20261029_03`; `isolation.json`
records their exact scope. Do not infer success or failure for untested cases.
The previous procedure below is retained as historical artifact documentation.
E must not be described as gameplay fixed. The reduced staging allocation and
resident-atlas correction are proven changes, not a demonstrated crash remedy.

The next diagnostic profile is under `ManualTests\RuntimeRecovery_20261030_01`.
It changes only guarded SCUS call sites/caves to log BIOS TTY resource and TIM
upload begin/end events; **all PROT entries, full scene edits, collision and
Stage E floor data are unchanged**. It is instrumentation, not a crash fix.
The original loader instruction simulation verifies identical model identities
and allocation chronology with and without these logging shims. The shims are
also tested for argument/return/stack/callee-saved preservation with load delays.
The floor-observer trace arena and other occupied engine patches are rejected
by byte guards; no existing code is overwritten by assuming zero padding is free.

Original SCUS LZS instructions decoded all E descriptors with matching bytes
and zero output-canary overruns. The original decoder additionally allocates
4,116 scratch bytes per compressed payload; the older unlimited-loader harness
substitutes that routine and does not include this nested allocation. Historical
allocator replays pass the dispatcher requests on the arena-return snapshot;
both retail and failing images fail on the later preload snapshot. Neither is
the live failing load state, and neither establishes current heap safety or OOM.
The independent trace also incorrectly reported single-TIM status as 1; real
SCUS type 0 returns 0, while type 1 returns 1. That static contract is corrected.
Their high stream mode is unchanged; this correction is not a demonstrated
explanation of the observed freeze.

The current one-attempt manual procedure is
`ManualTests\RuntimeRecovery_20261030_01\READ_ME_FIRST.md`, superseding the
historical matrix order below. R5 SHA256 is
`c8e794c0a3ac737212e1c25e431c4663f7bf20159b485f40c5c01068010213d4`.
`scripts/analyze-native-loader-log.py` pairs nested begin/end observations from
user-supplied BIOS TTY logs; it never connects to an emulator and never infers a
proven crash mechanism from a truncated log. Unrestricted export now records
static native allocation requests, including nested LZS scratch, and publishes
`scene-verification.json` with live loader/GPU/render/gameplay explicitly false.
Allocation accounting is not available-memory proof and never blocks export.

## Status and scope

The user reported a Rim Elm loading crash in `D:\AstraRimElmProject\222.img`.
This work preserves that IMG, its parent, the retail disc, all seven collision
recipes and the exact editor manifest. No LCK or DuckStation process was launched
and no host desktop/input was controlled. No graphical sandbox is established.
All new verification described below is automated; native gameplay and GUI
behavior remain unverified. The user's fresh-retail 1111.img screenshot is a
separate, accurately reported zero-import failure, not proof of inherited damage.

## Proven findings

1. The old mixed-formats parent predates resident-character texture protection.
   Both that parent and 222 contain scene uploads whose final values conflict
   with **5,794 resident-character VRAM words**. Retail and Stage E have zero
   conflicts by this comparison. PROT 874 itself is unchanged: unsafe scene
   upload destinations overwrite the atlas at runtime. Protecting only new
   allocations cannot repair an already damaged input scene. An earlier 5,940
   count measured all new writes into the resident footprint, including words
   whose value was unchanged; these are different metrics.
2. The original SCUS compressed type-1 TIM_LIST dispatcher calls malloc with
   **0x70800 (460,800) bytes at 0x8001F15C**, regardless of the small encoded
   carrier size, and frees it at 0x8001F200. The type-0 single-TIM path requests
   **0x11800 (71,680) bytes at 0x8001F0EC**, uploads and frees each member.
   This transient request was omitted from previous encoded-resource budget
   checks. It is not the persistent TMD allocation and is freed before those
   model allocations. No live 222 heap snapshot/exception PC exists, so an
   actual malloc failure and the exact crash instruction are **not proven**.
3. Executing original model-registration instructions in the bounded headless
   harness finds 119 scene models / 124 global including five resident models
   in 222. The cumulative identities match an independent native reader.
   Selectors 114, 115 and 116 are not shifted in this instruction simulation.
   The simulation substitutes allocator, decompression and GPU services; it
   does not prove live visibility, heap capacity or animation behavior.
4. In this user's edited parent, placement ordinal 13 is the previously
   imported Retockin object (selector 116), not a retail-native Rim Elm object.
   Ordinals 12 and 16 are the old imported Rayman objects. All three already
   have unbound native anchors. The newly requested static imports also have
   unbound references, no active CELL_BIND_OWNED anchor and no primary/fallback
   trigger binding. Story dependence is not established as this crash's cause.
5. Between Stage E, the old mixed parent and 222 only MAP PROT 1 and resource
   PROT 4 change. MAN bytes and trigger region are unchanged. The Stage E SCUS
   engine patch is inherited unchanged; this work adds no PS1 engine patch.
   Original model allocations total 304,116 bytes (retail), 321,144 (parent)
   and 322,740 (222). Their live memory headroom remains unverified.

## Generic correction

The writer lowers representable compressed TIM_LIST members into ordered native
type-0 single-TIM descriptors at the same point in the descriptor walk. Each
member must fit the original single-TIM staging buffer. It preserves native
model registration order, upload order/words, member suffixes, other stored
payloads and mode flags. Lists that cannot be represented this way retain their
existing encoding. The bounded reader now has 256 descriptor storage slots;
that is an editor reader bound, not a claimed retail engine limit. Original
engine instructions verify the resulting repeated type-0 descriptor walk.

Unrestricted export also checks inherited character-upload conflicts before
new edits, emits a prominent dependency warning, and normalizes representable
inherited carriers transactionally. It does not reject the export or silently
relocate existing requested objects. A bad parent's old VRAM uploads remain a
warning: recovery below uses an explicitly verified corrected parent instead.
No asset-specific scene/ID branches or new compatibility gates were added.

## Full scene recovery and diagnostic cases

Folder: `D:\AstraRimElmProject\ManualTests\Crash222_Matrix_20261029_02`.

**Start with E_FULL_USER_SCENE_RECOVERED_SOURCE.img.** It contains all three
foreign imports, four duplicates, original requested moves/rotations, two hidden
placements and seven collision recipes. Its complete MAP payload is byte-for-byte
identical to 222. Recovery uses the prior corrected mixed parent only after
proving identical parent MAP, identical model geometry/primitive bytes, unchanged
story data and 995,290 identical intended material samples. In the final E
candidate, 1,066,614 relocated intended samples match, all 119 native model
identities pass the original-instruction trace, LCK's native assembler reparses
the scene/materials, all placement selectors are in range, and resident-character
overwrites are zero. These are automated results, not gameplay confirmation.

Other files isolate variables without replacing or rearranging the user's scene:

| Case | Contents / diagnostic purpose |
| --- | --- |
| A_222_EXACT_MAP_SMALL_STAGING.img | Exact 222 MAP, model geometry and GPU words; only carrier staging changed. Inherited bad atlas destinations deliberately retained. |
| B_NATIVE_EDITS_ONLY.img | Four duplicates, moves/rotations and two hides; excludes three foreign requests and all seven collision recipes explicitly. Old parent's original imports remain. |
| C_FOREIGN_IMPORTS_ONLY.img | Three new foreign imports and their rotation; excludes duplicate/move/hide and collision requests explicitly. Old parent's original imports remain. |
| D_ALL_GEOMETRY_NO_COLLISION.img | All geometry requests; excludes seven collision recipes explicitly. |
| E_FULL_USER_SCENE_RECOVERED_SOURCE.img | Full requested scene, corrected parent material relocation, small staging; no requested edits omitted. |

All images have disk readback checks and separate debug/report folders.
`matrix.json` records exact coverage. The preserved original scene is
`D:\AstraRimElmProject\ManualTests\Crash222_20261029_01\222-full-scene-preserved.lckscene`.
Do not apply that overlay again on E: E already contains the complete edits.

## Manual procedure (user only)

1. Keep 222 and its old parent untouched. Select **E_FULL_USER_SCENE_RECOVERED_SOURCE.img**
   as a new disc in DuckStation. Cold boot it and load the same in-game memory-card
   save used for the crash. Do not restore a savestate for this comparison.
2. Record whether loading reaches Rim Elm, which story/scene variant loads if
   known, and whether the scene remains stable while walking and changing camera
   views. A town01-only test does not verify town0b/town0c.
3. Inspect Vahn's face, all imported materials, the two Geremi copies and Koin4
   object, four Retockin copies and their rotation, and the two intentionally
   hidden parent imports. Check collision and the previously functional floors
   separately from rendering. Report wrong geometry, invisibility, bad textures
   or missing behavior independently.
4. If E still crashes, report the exact load stage/message/exception address and
   IMG filename. Test **A** with the same cold boot/save. A preserves the old
   unsafe texture destinations intentionally: compare load success separately
   from texture correctness. A versus original 222 isolates carrier staging.
5. If necessary, cold boot B, C and D with the same save to identify whether new
   imports, MAP edits or collision recipes change the failure. These are partial
   diagnostics, not completed scene exports. Record each result by filename.
6. Optionally open E in LCK to compare the full scene and transforms. Native
   automated reparse already passed; user GUI observations remain separate.

No successful encoding or screenshot of intact geometry proves native collision,
floor sampling, live model registration or gameplay. A crash-free manual run is
still needed before calling this recovery gameplay verified.

## Evidence and regressions

`ManualTests\Crash222_20261029_01\inheritance.json` compares all PROT entries,
placements and native model pools across retail, Stage E, parent, corrected parent
and 222. `allocation-chronology.json` captures original instruction malloc/free
calls. `small-staging-original-instructions.json` verifies old/new identities,
20 actual single-TIM uploader bodies and identical GPU words. The original
allocator and GPU completion remain substituted.

Source-gated tests: `crash_222_inheritance_audit`,
`crash_222_allocation_chronology`, `exact_222_small_buffer_original_instructions`,
`exact_222_incremental_scene_regression`, `exact_222_recovered_candidate_identity`.
They use preserved user artifacts and create-only output paths; none invokes an
emulator. Regular carrier regressions verify suffix and unrelated payload
preservation, idempotence and cumulative model identities. The Bylon replay
separately guards the user's previously successful duplicate/collision/floor path.

## Remaining separate gaps

The fresh-retail 1111 export requested conc3 mesh 21, two dolk2 mesh 4 imports,
and ropeway placement 3. The first three need a native raw-mesh MAP-instance
adapter; the fourth lacks complete native image/palette dependency resolution.
All four failed, and the red 0/4 message accurately reports incomplete output.
These exporter capabilities are not fixed by the 222 carrier correction and are
not demonstrated PS1 engine limitations. Universal World Objects support is not
claimed. Live culling, active MAN/animation/script closure and actual heap capacity
also remain unverified where their required native dependencies are absent.
