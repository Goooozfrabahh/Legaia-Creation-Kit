# Native foreign texture corruption — 2026-10-28

Input evidence: the user's screenshot and
`ManualTests/M2_StageE_20261027_01/teeeeeest_export_1793232207220742600/debug.txt`.
The user verified three independent foreign Bylon objects rendered in Rim Elm
with working collision in the preceding build. Preserve that result; the current
correction's gameplay appearance is not yet verified.

## Causes

Unrestricted mode incorrectly disabled three allocator inputs: the field runtime
reservations, written destination TIM coverage, and boot-resident coverage. The
first free-looking locations were therefore live GPU render targets. These are
16-bit VRAM word coordinates, not editor-world coordinates:

| Upload | Old destination and extent | Corrected destination |
|---|---|---|
| Main indexed image | (0,0), 64x256 | (832,0), 64x256 |
| Main CLUT words | (64,0), 256x1 | (0,482), 256x1 |
| Secondary image | (320,0), 16x64 | (960,0), 16x64 |
| Secondary CLUT words | (336,0), 16x1 | (976,0), 16x1 |

All four old allocations intersect the reservations already defined in
`texture_relocation.rs`. The main image crosses both left framebuffer regions;
the other allocations touch the top framebuffer regions. Subsequent drawing or
clearing can replace texture indices and palette words with framebuffer pixels.
The screenshot is consistent with that failure; no live GPU trace was taken.

There was also a real RAM packing defect. The old PAC footprint was 405,504 bytes,
exceeding the documented `0x62C00` (404,480-byte) field staging arena by 1,024 bytes.
Obsolete all-zero padding was retained after inserted uploads. Keeping one native
terminator and only the required new sector padding reduces the footprint to
403,456 bytes without removing any resource or changing original chunk bytes.
The old file could overrun that staging arena; no live heap capture was taken.

## Generic correction

The allocator now respects runtime framebuffer/clear regions, destination TIM
coverage (including uploaded zero words), full assembled destination resources,
boot resources, explicit observations, and native CLUT-walker source/destination
cells. Actual VRAM bounds and explicit destination fitting remain encoder checks.
Unrestricted object editing and per-operation export attempts stay enabled; no
world object is snapped, moved or rejected to satisfy inferred collision rules.
The corrected coordinates above are allocation results, not hardcoded locations.

Exact repeated donor identities reuse their registered TMD/TIM closure. Each copy
still gets an independent native descriptor/grid reference. Native translation
rounds each requested placement from its original donor, avoiding compounded
relative rounding. No unrelated destination object serves as an import source.

The comparison also exposed an existing persistence issue: FOREIGN records save
absolute centers, while saved generated collision keeps its original rest pivot.
Generated collision translation now uses `draw.center - saved_pivot`; this equals
the original offset in a live scene and restores the tested walls after loading
the logged manifest. Manual grid corrections are unchanged.

## Upload order, texture pages and palettes

The existing native path remains descriptor TIM uploads first, then the selected
mode-`0x14` PAC chunk walk. `FUN_8002541C` dispatches each chunk in stored order
through `FUN_8001F05C`; TIM_LIST members reach `FUN_800198E0` in pack order. The
original PAC prefix through `0x59B88` is preserved verbatim. Four new native TIM
members follow it before the zero-size terminator. CLUT runs are raw 16-bit TIM
image uploads, so their exact words land at the CBA destinations. Original image
and palette upload words remain unchanged.

The relocated TMD's TSB and CBA words are rewritten to those destinations while
UVs, texture depth/blend bits and other model bytes remain intact. New debug logs
record image versus CLUT roles, each primitive's old/new TSB/CBA, decoded page and
palette coordinates, UVs, sample count, upload order and sector-rounded PAC size.
Matching texel sampling is checked against ordered native upload results. Known
CLUT-walker regions are excluded from new allocations; this is not a certificate
that every possible future script/menu write is known.

Primary repository evidence: `docs/formats/field-pack.md`,
`docs/subsystems/asset-loader.md`, `docs/formats/asset-type.md`,
`docs/subsystems/field-ambient-fx.md`, `crates/tim/src/vram.rs`, and the native
descriptor/PAC/TMD parsers used by the exporter. No original-engine patch added.

## Verification and preservation

Automated: 274 unit tests and 9 integration tests passed, including unrestricted
framebuffer/boot protection, PAC padding, independent duplicate precision, and
existing native object/floor/selection tests. 27 optional tests were ignored.

The user's exact three-object manifest was replayed without authoring a new
arrangement. Its complete native MAP is byte-identical to `teeeeeest.img`:
placements, independent records, walls, floors, bindings and script bytes are
preserved. Every unrelated PROT entry is byte-identical as well. Only the resource
bundle and PAC payloads change, plus required archive addressing/sector metadata.

29,164 imported native TMD UV-bounding-box samples match between old and corrected
initial uploads. An in-memory overwrite of the known runtime framebuffer regions
corrupts 29,164 old samples and zero corrected samples. Original native upload
words remain unchanged. This is an automated framebuffer-write simulation, not
DuckStation execution or GUI verification.

New output, native readback and LCK scene/resource reparse passed:
`D:\AstraRimElmProject\ManualTests\VRAM_Fix_20261028_01\LCK_EXPERIMENTAL_BYLON_VRAM_FIXED.img`

SHA256: `cadcb94ac11a27a8abe8b973093f85cde68610561485b960be273bac80082267`.
Per-export debug.txt is in its sibling export directory. No writer failures.

Rollback executable and Git recovery:
`D:\AstraRimElmProject\Backups\LCK_VRAMFix_20261028_01`.
The previous installed `fda11a8` executable has SHA256
`a972fba793ef0ce46c3fa4b6f6ed190b91b97229a324c99cf4a906029149891e`.
Retail, M1.1, earlier checkpoints and the user's working IMG remain untouched.

## Manual confirmation

Boot the new corrected IMG yourself using the same save/story state and route
as the successful three-object test. Compare the formerly streaked panels, move
around the objects and recheck their collision. Watch the surfaces across several
frames and a menu open/close or scene re-entry to expose later overwrites. Provide
the new debug.txt and screenshots if corruption persists. New build GUI and
native gameplay remain unverified until the user reports them. No LCK or
DuckStation process was launched or controlled by the assistant.
