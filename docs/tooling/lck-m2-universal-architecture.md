# M2 native export architecture checkpoint

This checkpoint establishes a game-wide read-only native record/dependency
inventory, operation assessments and an exact kind-2 elevation codec. It does
not claim universal export support or unlock unresolved production export gates.
The installed, user-verified M1.1 release remains Latest Build.

## Scope and identities

The original retail disc census enumerates all 124 distinct CDNAME blocks,
including their separately named story variants. All 1,233 PROT entries are
accounted for by the named-block ranges. There are 102 recognized field MAPs;
the other layouts remain entry inventories with explicit gaps. Two entries
participate in overlapping named ranges; these contexts are retained separately.
Unrecognized layouts are not treated as empty cells or successfully exported.

The native inventory contains these reference/record counts (not a promise
that every record spawns a distinct editable or renderable GUI object):

| Record category | Count |
|---|---:|
| MAP placement references | 3,607 |
| Terrain/decor references | 27,217 |
| MAP descriptors | 52,224 |
| MAN actor spawn records | 2,708 |
| MAN script records | 6,488 |
| Native model registrations | 8,546 |
| Raw scene TIM uploads | 1,274 |
| Kind-2 elevation rows, including fallback/shadowed rows | 30,162 |
| Other tile trigger rows | 9,029 |
| Region trigger rows | 1,217 |
| Bundle descriptors / exactly decoded LZS assets | 618 / 618 |

Native object identities use the scene context, MAP entry and grid reference,
not viewport index or geometry hash. All 16,384 grid references per MAP are
counted, including invisible/ground-only and descriptor-zero references. Shared
descriptors do not merge object identities. Model selectors resolve through
`SceneModelBank`, the existing native loader registration order, including
compressed and streaming carriers. The editor's raw mesh scanner order is not
used as the native model ID space.

Source addresses distinguish physical MAP/load windows, PROT entry offsets and
decoded MAN/TMD/asset offsets. Decoded offsets are never claimed to be directly
writable IMG locations. The original-disc SHA-256 is
`750dbcbadb70362feb3468467bbe7efbf3d151a414caa428c66c368d4b0b73e6`.

## Evidence and ownership semantics

`native_cell_inventory.rs` records typed dependencies with native source spans
and explanations:

- Exclusive/shared reference: counted in the explicitly stated reference
  domain. A descriptor used once by the MAP grid is not thereby free of script,
  ground-texture or runtime dependencies.
- References: native selector, floor sample, trigger or binding relationships.
  Sampling a floor does not own the floor. Content equality does not own a TMD.
- Affects: decoded script context, wall-grid, floor-ladder and CLUT effects.
  Instruction boundaries and bytes are retained; execution is not assumed.
- Unrelated: byte-disjoint storage within one native container only. Disjoint
  records can still reference or affect each other at runtime.
- Unresolved: missing model/bank/context data, primitive texture/CLUT closure,
  animation lifetime, or wall/floor ownership without native evidence.

Native context selectors `0xf8` and `0xfb` follow `FUN_8003C83C`'s player and
system-channel special cases. They are shared runtime contexts, not nonexistent
MAN records 248/251. The system channel references the scene's partition-1
record 0. Other unresolved selectors remain explicit graph nodes.

Kind-0/1/2 and kind-3 region trigger rows retain physical primary/fallback
provenance, including duplicates. The effective kind-1 binding uses the native
first-match lookup and the descriptor's footprint anchor. Binding provenance
points to the trigger bytes, rather than merely the selected grid cell.

Dependency traversal retains incoming script effects. Generic assessments for
move, rotate, scale, duplicate, delete and cross-cell import report dependency
closure, shared descriptors, unresolved relationships and required capabilities.
They never serialize edits, authorize exports, or silently omit selected IDs.
Scene-global dependencies can conservatively enlarge a closure; inclusion does
not mean the corresponding record should be edited.

The wall grid has no object-owner IDs. No geometry-proximity ownership rule
was added. The existing reviewed contribution reconstruction remains separate
from export permission. Cross-cell packing, resource aliasing, independent actor
contexts, primitive texture closure and runtime memory budgets remain additional
adapter obligations. Bundle decoded sizes/storage spans are inventoried; fitting
a compressed payload is not proof of available PS1 runtime RAM or VRAM.

## Exact elevation representation and the six-request benchmark

`native_elevation_codec.rs` encodes and decodes the genuine kind-2 record:

```text
[tile_x, tile_z, signed_coarse, four_packed_2_bit_steps]
height = native_floor_mean - 32*coarse - 16*selected_step
```

All 65,536 coarse/step byte combinations round-trip to exactly the same four
heights. The repartitioner supports explicit 64-unit sample translations against
explicit native retained destination samples. It rejects missing context,
duplicate destination contributions, coordinate/height overflow, retained
bilinear surfaces and unrepresentable height sets. It does not infer ownership,
allocate trigger rows or create an export plan.

The actual benchmark source gives the selected staircase samples `-64,-112,
-64,-112` at source sub-cells `(74,36),(75,36),(74,37),(75,37)`. Translation
`(-320,+64)` is `(-5,+1)` sub-cells and crosses four destination tiles:
`(34,18),(35,18),(34,19),(35,19)`.

Two independent obstacles are demonstrated using native destination data:

1. The lower destination tiles retain bilinear slopes (mean -96). A kind-2
   constant-step replacement changes their continuous floor surface.
2. A top destination mixes retained height 0 with moved height -64. The required
   64-unit span exceeds the format's maximum 48-unit span across four 2-bit
   steps. Changing the common floor mean cannot reduce that span. The -112
   sample creates an even wider discontinuity in the adjacent destination.

Thus moving a trigger row's X/Z or allocating more ordinary rows is insufficient
to preserve these exact surfaces. Full-tile translation remains representable
in the codec; arbitrary half-tile placement can exceed this native encoding.
This is independent of the unresolved wall ownership and house behavior.

A possible native extension is a sparse exception-height table with independently
encoded sub-cell heights and an updated floor sampler. That requires identifying
and patching the appropriate native sampler/lookup path, adding a genuine loaded
scene resource, budgeting its RAM and packed storage, and preserving fallback
lookup behavior and script-driven floor ladders. Enlarging kind-2 rows also
requires the native stride/lookup change and new table-allocation proofs; the
existing 0x2000-byte primary/fallback blocks cannot simply be overwritten or
assumed to have spare space. These are investigated requirements, not an
implemented or verified engine patch. An alternative surface representation must
demonstrate exact sampling and preservation of unrelated floors before use.

The house benchmark retains MAN 0 and incoming references from MAN 35, 92, 95
and 104. The shared terrain descriptor remains shared. All six requests survive
assessment. This checkpoint neither proves those scripts independent nor exports
the six-request IMG.

## Reproduction and verification

Run the dedicated headless inventory harness, never the LCK GUI:

```powershell
.\scripts\inventory-native-disc.ps1 `
  -Source 'D:\Emulators\Games\Legend of Legaia\LegendOfLegaia.img' `
  -Output 'D:\AstraRimElmProject\ManualTests\NEW_inventory_directory'
```

The output directory must not exist. `inventory.json` enumerates all blocks,
PROT coverage, summaries and content-sharing contexts; numbered cell JSON files
contain the record graphs and explicit gaps. An interrupted run retains partial
evidence and cannot be mistaken for a completed global report. The source image
is hashed before and after the scan.

Final enriched census: `D:\AstraRimElmProject\ManualTests\M2_GlobalInventory_20261027_02`.
The `_01` census is preserved as the initial reader snapshot. Representative
tests cover ordinary field (`town01`), compressed bank (`koin3`), streaming MAN
(`rikuroa`) and kingdom (`map01`) layouts. Scene names/IDs occur only in test
fixtures; production readers and operation assessments contain no scene-specific
export exceptions.

Verification is automated only. The new architecture, height codec and
assessments have not been verified through GUI interaction, PS1 execution or
gameplay. Existing export safeguards remain, and Latest Build stays M1.1 until
the new candidate meets native acceptance. No desktop input/control tool, host
LCK or DuckStation was launched.

Final automated verification: 250 library tests and 9 integration tests passed;
20 controlled/external tests are excluded by the default run. The original-disc
census and representative/six-request graph benchmark were run explicitly and
passed. Release compilation passed with 106 warnings, including unused backend
entry points in this architecture checkpoint. Those entry points are exercised
by the dedicated headless harness and are not yet connected to permissive export
adapters. This is not a passing six-request export or native gameplay test.

## Next adapter work

1. Bind field-level contribution proofs and runtime script effects to the
   existing reviewed wall reconstruction without converting proximity into
   ownership; preserve retained and shared contributors.
2. Establish the additional native floor representation/sampler path required
   by exact half-tile staircase placement, including live LUT effects.
3. Close bound-object script control flow, helper calls, actor collider state,
   animation and trigger behavior before rebasing or cloning contexts.
4. Connect proven operation-specific plans to native adapters. Preserve untouched
   MAP variants and donor cells, independently allocate new identities, and
   validate resource registration, packing and hardware budgets through readback.
5. Demonstrate representative native exports across layout families before any
   claim of universal support or replacement of the user-verified release.
