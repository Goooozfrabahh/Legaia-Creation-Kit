# Native descriptor, resource and model adapters — 2026-10-28

Based on installed checkpoint `083b99a` and the user's actual
`D:\AstraRimElmProject\testtesttest_export_1793234633983423100\debug.txt`.
Unrestricted export stays enabled. No LCK/DuckStation process was launched.

## Actual failures and generic resolution

The exporter incorrectly required a strict plain descriptor table at offset zero
and assumed every MAP selector indexed only the `MAP+3` bundle's TMD pack. Those
are exporter gaps, not engine limits.

* Descriptor resolution now follows the native count/type/offset walk for plain,
  prescript-prefixed, embedded and small-count tables. Repacking preserves the
  prefix, rewrites table-relative offsets and preserves unrelated decoded slots.
* Model resolution uses `SceneModelBank` registration order: packed and bare
  descriptor models, walked small-table packs, and packed/bare DATA_FIELD model
  chunks. The selected model must match the editor's actual donor TMD bytes.
  Donor bank selection is never guessed from another destination object.
* Native textures follow descriptor order and the mode-selected streamed route:
  mode `0x14` typed chunks and mode `0x0A` bare packs. Model and script chunks
  remain distinct from texture uploads. Raw CLUT records are decoded with the
  existing native park-strip reader; their exact palette words are serialized as
  16-bit native LoadImage TIMs. No replacement colours or fabricated pixels.
* Shared resource sessions retain valid allocations across independent imports,
  committing session state only after successful serialization. Allocation uses
  referenced native image words and complete used CLUT windows. Original UVs,
  depth/blend bits and sampled colours are preserved. Images are allocated before
  small palettes to avoid unnecessary page fragmentation. Framebuffer, existing
  texture, boot and CLUT-walker reservations remain in force.
* If uncompressed added textures would overflow the PAC staging arena, the writer
  can add/reuse a compressed type-1 TIM_LIST descriptor. Original PAC bytes and
  the mode flag remain intact. New texture loads occur before the original PAC;
  disjoint protected allocation makes the final upload words identical. Model
  registration IDs do not shift. The actual native walker reads its count word;
  reader storage now supports up to 16 descriptors and the scene API returns the
  actual count rather than a fixed seven-slot array. This is a bounded reader,
  not an assertion that the retail engine has a sixteen-descriptor limit.

Packed, bare and streamed model carriers are also supported by the destination
registration/readback path. New models append in their actual registration bank;
existing IDs and resources are checked. No new PS1 executable patch was added.
The existing source image's engine bytes and Stage E work are preserved.

## Mixed benchmark

The implementation contains no Rayman/Retockin/Bylon scene-ID branches. The user's
benchmark provides these representative native sources:

| Requested object | Native model source |
|---|---|
| Rayman placement 12 | Four-descriptor walked bundle, PROT 200, descriptor 1, member 3 |
| Rayman placement 6 | Same format, member 82 |
| Retockin placement 2 | Streamed TMD pack, PROT 283, chunk header 0, member 3 |

Rayman's TIM list also contains raw 256-word CLUT uploads at native rows 503/504.
These caused the next failure after the descriptor restriction was removed;
they are now decoded by format. Importing whole carrier atlases and treating
their small palettes as independent allocations wasted VRAM; exact material
regions, shared sessions and image-first placement resolve that packing problem.

All three requested objects now serialize their real native TMD/material data and
receive independent MAP descriptors/references. The requested Retockin yaw is
encoded at native precision, with requested/encoded values logged. Editor
coordinates, rotation and scale remain unchanged by export.

## Partial-export reporting

Native import coverage is structured separately from successful writer reports:
requested imports, written native objects and failed imports. A collision write
does not count as an imported object. `debug.txt`, the integration outcome and GUI
completion show those counts. A partial result gets a persistent red failed-import
message. Zero successful imports is labelled **PARTIAL IMG FILE ONLY — NO REQUESTED
OBJECTS IMPORTED**, never a completed scene. Other failed edits still mark the
result incomplete even if all base model imports succeeded.

Every output still verifies file existence, size and disk-readback hash. The
output path and Open IMG output folder control remain available.

## Evidence and limits

Automated suites: 280 studio unit tests + 9 integration tests; 635 core native
format/scene/model-bank tests. Total 924 passed. Twenty-seven optional studio tests
were ignored in that run. Tests cover small/embedded/extended tables, raw CLUTs,
bare/streamed models, mode-0A packs, exact texture footprints and zero-import
reporting. Core-reader regressions and existing editing/native-floor tests pass.

The exact user-authored mixed manifest was replayed without creating a new scene
arrangement. Three requests, three native objects written, zero failed imports,
zero writer failures. The output reparsed through native MAP/resource readers and
reopened through LCK's scene loader with all new references resolving to TMDs.
Final on-disc descriptor/PAC upload replay matches the intended native texture
words. Original observed collision grid, floors, bindings and unrelated PROT
entries remain unchanged. Committed carriers: bundle 278,528 bytes and original
PAC 368,640 bytes, both within the documented 404,480-byte staging arena. Raw PAC
candidate warnings are explicitly labelled provisional; those candidates are not
the written carrier.

Output:
`D:\AstraRimElmProject\ManualTests\Native_Format_Adapters_20261028_01\LCK_EXPERIMENTAL_MIXED_FORMATS_FIXED.img`

SHA256 `b2e438fc23c411907eb92d0d3d97ee6aba36e1ea9932a06fc9017d8f8438a780`.
Its sibling export directory contains the complete debug.txt.

Bylon protection: its three-object MAP remains byte-identical to the user's
gameplay-tested IMG. All 29,164 texture samples match, simulated known framebuffer
writes change zero corrected samples, and unrelated entries remain unchanged.
Its cropped PAC uses 378,880 bytes. This is automated simulation/readback, not a
new gameplay result.

The new GUI and mixed-scene DuckStation behavior are **not verified**. Native
handler formats and buffer extents are verified; actual load-time heap behavior,
story-specific runtime writes and gameplay still require the user's test. This
does not claim universal actor/animation/script/walkability transplantation or
every operation on every category. Unsupported writers remain explicit failures;
no validation was disabled to make this benchmark pass.

Rollback and recovery:
`D:\AstraRimElmProject\Backups\LCK_NativeFormats_20261028_01` preserves the installed
083b99a executable, Git bundles and installation receipt. Earlier Bylon builds,
M1.1, retail IMG, user IMGs and checkpoints remain preserved.

For the next manual check, boot the new fixed IMG using the same save and loaded
Rim Elm variant as the failed mixed export. Verify the two Rayman objects, the
Retockin object and its rotation, textures and collision. Test a newly made mixed
edit through Latest Build separately; provide its debug.txt if anything fails.
