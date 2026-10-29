# World Objects runtime correctness checkpoint

The user's `ec2a039` gameplay test FAILED: the tower is absent, visible appearance
differs from the editor, and Vahn's face is corrupted. Earlier serialization and
reparse checks did not certify runtime behavior. This checkpoint fixes a proven
resident-texture overwrite and expands independent diagnostics; it does **not**
declare the tower or the entire repository gameplay compatible.

## Proven texture overwrite and generic correction

The mixed export's tower image at `(832,256,40,96)` overwrites Vahn/Noa's resident
atlas. Other imported image regions also intersect the party texture footprint.
The exporter reserved framebuffers, boot UI and mesh-targeted scene uploads but
omitted the complete `FUN_8001E890` field-character texture upload: PROT 874,
descriptor 2, eight TIMs. Native face-frame `MoveImage` operations subsequently
read and write this same atlas. The exact old IMG overwrites **5,940 resident
words**; the corrected candidate overwrites **zero**, compared with the original
scene's existing writes. This is automated byte/coverage verification, not a
gameplay retest of Vahn's face.

The allocator now reserves all decoded field-character image/flattened-CLUT
footprints, including zero pixels and alternate face source frames. Protection
uses the native resource data, not a town-specific rectangle list.

Preserving UV numbers forced images into occupied page-congruent positions.
The generic material allocator now groups disjoint sampled native-word regions
of an upload, retains overlapping primitive regions together, and packs images
before palettes. When needed it rebases each primitive's UVs and texture page
by the same integer translation for all vertices. It preserves geometry,
topology, colors, depth/blend flags and sampled texels; it does not subdivide a
primitive, resample an image or wrap UV bytes. Every UV-bounding-box texel is
compared against the ordered original donor uploads. All allocation policy is
shared across donors and destinations. Unrestricted mode remains enabled.

## Independent original-loader investigation

`native_loader_trace.rs` independently walks descriptor counts, type dispatch,
word-offset packs and the mode-selected PAC. It does not call the exporter's
`mesh_pool`, registration planner or `SceneModelBank`.

The source-gated CPU harness executes original SCUS instructions for
`FUN_80020224`, the relevant `FUN_8001F05C` arms, `FUN_8002541C`,
`FUN_80026B4C` and `FUN_800268DC`. It enables integer load delays and alignment
checks. OS, heap allocation, LZS decompression, GPU operations and non-model
dependency installers are substituted. Thus it verifies bounded registration
logic under those services, **not live heap validity, DMA completion, rendering
or gameplay**. Original overlay 897 window-spawn gate instructions are tested
separately with a full map region and substituted actor construction.

| Scene selector | Global slot / pointer address | Exact selected donor |
|---|---|---|
| 114 | 119 / `0x8007C1F4` | Rayman placement 12, walked descriptor 1, member 3 |
| 115 | 120 / `0x8007C1F8` | Rayman placement 6, walked descriptor 1, member 82 |
| 116 | 121 / `0x8007C1FC` | Retockin placement 2, streamed chunk 0, member 3 |

All three old exported TMD hashes match their logged relocated donors under
original registration instructions. The new candidate also matches its new
material-rebased hashes. Original window gates reach the actor constructor for
all three when the region is `[0,128)`. This does not prove the user's live
region includes the tower or that its actor renders. Static native vertices
match the requested editor preview within the benchmark's 1.5-unit combined
translation/yaw rounding bound. Donor material comparison against editor assembly
finds zero differing samples (322,303 / 515,072 / 157,915 samples respectively).

**Still missing:** the tower's donor MAN object context 7 and ANM clip 3 are not
transplanted. Its destination binding is absent. The native window sweep gives
it an unbound actor; this does not preserve its original animation/script
behavior. This is a confirmed exporter dependency gap, **not yet a proven sole
cause of invisibility**. A live capture is still required to distinguish active
region/culling, actor state, heap/resource lifetime and rendering. Do not add a
dummy script, strip the requested behavior or bake a replacement pose and call
that a complete animated import.

Expected changes to destination MAP/bundle/PAC during cumulative import no
longer produce stale-source warnings. Donor and unrelated source entries are
still checked against the opened source; unexpected differences remain errors.
Earlier registered model hashes are rechecked after all subsequent imports and
transforms. No original engine patch was made.

## Disc-wide evidence and remaining adapters

Audit output: `D:\AstraRimElmProject\ManualTests\Runtime_Compatibility_20261029_02`.

* All 124 named sources and all 1,233 PROT entry classifications were attempted.
* The actual World Objects catalog contains 40,671 original-source entries:
  4,092 MAP objects, 25,815 terrain/decor entries, 8,599 meshes and 2,165 MAN NPCs.
  The user's edited IMG adds three placements and three mesh registrations.
* Independent loader traces cover 101 field-map blocks and 13 layout families.
  Original instruction registration simulations match one representative of
  every family. Shared head defines are explicitly excluded from field-registry
  certification; their legacy extraction aliases are retained as audit gaps.
* The evidence inventory includes 8,517 native models, 88 unresolved models,
  6,488 MAN records, 29,927 elevation rows, 1,272 TIM uploads and 430 unresolved
  dependency records. These are observations, not portability certificates.
* Memory-only cross-cell fixtures test two different donors, cumulative registry
  preservation, independently allocated duplicate MAP identities and native yaw
  for each family. Four family cases complete the resource/MAP test; nine expose
  explicit gaps: external image/palette closure, noncanonical TIM member tails,
  textureless/shared-resource routes, and a PAC carrier fallback reached through
  a strict provisional-budget failure. These are exporter gaps, not established
  PS1 engine limitations. No arranged gameplay scene is written by these tests.
* Count-driven bare type-9 model registration now has a reusable `WalkedSingle`
  source, including materialization and a regression fixture. This does not
  establish that every catalog model's gameplay dependencies can be exported.

`compatibility.json`, `cell-*.json`, `world-objects-catalog.json` and
`cross-cell-families.json` retain identities, categories, layouts, dependencies,
reference evidence and unresolved cases. Full MAN/ANM/VDF/NPC/script closure,
runtime allocations and category-specific transforms remain unfinished.

## Per-object verification and observable runtime evidence

Every unrestricted export creates `object-verification.json` beside `debug.txt`.
It separates encoding, committed native reference, IMG write/readback,
independent static loader identity, actual runtime identity, render visibility,
collision/interaction and missing/unverified dependencies. It retains requested
editor centers, native offsets/yaw, source and relocated TMD hashes, record/grid
addresses, source binding bytes, animation selector and known incoming contexts.
Missing behavior produces an **INCOMPLETE NATIVE BEHAVIOR** GUI warning even when
all three models were encoded. It does not prevent experimental IMG output.

No live engine/visibility/interaction flag is set true by serialization tests.
The corrected candidate preserves the old mixed IMG's collision/floor/binding
regions and all unrelated entries. The Bylon replay preserves its entire MAP
and unrelated entries; all 29,164 material samples match, and framebuffer
overwrite simulation corrupts zero corrected samples. Bylon's previously
user-verified rendering/collision is historical evidence; its interaction script
is still an independently missing dependency, now honestly reported.

An offline tool is ready to inspect a **user-supplied raw 2 MiB main-RAM dump**:

```powershell
python scripts/verify-native-runtime-dump.py <raw-main-ram.bin> <export-log-folder>/object-verification.json <NEW-runtime-identity.json>
```

It never launches, attaches to or controls an emulator. It reads the live scene
name, model counter and scene-bank base, checks each actual pointer-table slot,
reverses the documented TMD absolute-pointer fixups, and compares the normalized
live TMD hash with the exported witness. A mismatch is reported as wrong/stale
data, mutation or corruption requiring investigation. A hash match certifies
only sampled live registry identity; rendering and interaction remain unverified.
Synthetic tests cover valid pointer fixups, corrupt model bytes and unregistered
slots. No live RAM dump has been supplied or analyzed at this checkpoint.

For any future manual comparison, record whether the IMG was cold-booted with an
in-game memory-card save or resumed from a savestate. A restored state can retain
old scene resources; the current user's launch method remains unknown. Do not
attribute this failure to a savestate without evidence.

## Preservation and verification status

`Backups\LCK_RuntimeIdentity_20261029_01` preserves the `ec2a039` executable and
pre-change Git bundle. M1.1, the original retail IMG, successful Bylon artifacts,
old mixed IMG, and earlier recovery/rollback copies are retained. All new IMGs
use new filenames.

Automated verification is recorded in the associated logs. User-reported manual
GUI/native gameplay evidence remains separate. No graphical sandbox exists;
sandbox GUI, new candidate GUI and new native gameplay are **not verified**.
No LCK/DuckStation process, physical input or host desktop was launched or
controlled. Universal native compatibility is **not achieved** at this checkpoint.
