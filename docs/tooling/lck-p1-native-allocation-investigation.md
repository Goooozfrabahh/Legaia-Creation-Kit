# P1 native allocation investigation — 2026-10-26

Production baseline: `008ccaf4e55f99627a5c77d2c49b32cc99a22ae2`. Changes described here are uncommitted. No emulator was launched, navigated or tested. No IMG was generated.

## Classification

The P0 test reported only the first failure for each of 35 town01 placements. Its mutually exclusive counts were 26 bound-source rejections, six shared-record rejections, and three unresolved-wall rejections. Those counts did not mean only three objects had collision uncertainty.

The P1 diagnostic independently checks record references, bind ownership, wall conflicts and capacity in MAP entries 1, 10 and 19. Exact per-placement results, including coordinates, mesh selector, flags, record IDs, wall counts and subsequent allocation outcomes, are retained at:

`D:\AstraRimElmProject\Backups\LCK_P1_20261026_allocation\placement-constraints.txt`

| Native constraint | town01 evidence |
|---|---|
| Bound spawn | 26 placements; cannot drop their MAN binding, animation or script identity |
| Shared descriptor | 15 placements share records 141 (4 cells), 230 (5), or 456 (6); nine of these placements are also bound |
| Unbound shared descriptor | Placements 1, 8, 15, 24, 28 (record 456), and 29 (record 230) |
| Unbound unique descriptor | Placements 7 (84), 16 (290), 33 (168) |
| Collision uncertainty | All 35 encounter the existing wall constraint when checked independently |
| Missing/oversized diagnostic bounds | 11 placements; conservative check sees all 4228 wall bits |
| Bounded wall conflicts | Remaining 24 placements encounter nearby walls; proximity is not ownership evidence |
| Record capacity | 87 blank, unreferenced descriptors in each principal MAP; not the immediate bottleneck |
| Copy spawn reach | Cave record 168 cannot allocate a copy within the supported nearby-anchor reach for +64 X; the other eight unbound candidates can allocate |

Vahn's House record 137, placement 12: cell (38,25), position (4864,-192,3208), mesh selector 36, flags 0x101e, one reference, bound spawn, corrected collider centre (4832,3168), 43 nearby wall bits. It remains blocked. Its correction flag is not a claim of ownership over those 43 walls.

## Implemented: independent native descriptors

`blank_object_record` is a shared deterministic allocator for copies and shared-reference movement. It accepts only entirely zeroed, unreferenced slots 4..511. It never reuses a donor cell or relocates an existing anchor.

An unbound shared placement now detaches on movement: clone all 32 descriptor bytes, apply the requested position and existing collider correction, and replace only the selected grid word's low nine record-index bits. Preserve its other flags. All other references retain the unchanged original descriptor. Copies use the same allocator and retain the original source descriptor even when their source is moved in the same plan.

The native basis is the grid's nine-bit descriptor index and descriptor +0x10 mesh selector, documented in `docs/formats/field-map.md` and `crates/scene-formats/src/field_objects.rs`. Record-local collider offsets follow the existing verified `collision_footprint_offset` implementation. No engine behavior, new resources, guessed geometry or texture substitution is introduced.

`map_patches` permits the allocated descriptor and selected reference bytes explicitly; all other byte restrictions and the sealed NativeExportPlan readback remain in force. Bound sources, unresolved walls, out-of-range anchors and exhausted allocation still reject the complete export.

## Evidence and limitations

Three new regression tests verify deterministic move+copy planning with native readback, independent movement of two shared references, unchanged original descriptor and unrelated placement, retained cell flags, identical wall/floor/trigger bytes, overlap at the original copy position, and rejection for wall ambiguity or allocation exhaustion.

The real-disc diagnostic succeeds across all three principal MAPs. In memory, all nine unbound placements can allocate/encode a +64 X move and eight can allocate a +64 X copy. Parsed unrelated placements remain identical. These checks are allocation evidence only: the full planner still rejects every tested move for unresolved walls. No diagnostic candidate was written to disc.

Studio regression suite: 133 passed, 0 failed, three opt-in real-disc tests ignored. The classification test passed separately. The first sandboxed full-suite attempt had four catalog filesystem Access denied failures; the permitted rerun passed all 133. Build status is reported separately in the development response. Whitespace validation passed.

Retail SHA-256 remains `750dbcbadb70362feb3468467bbe7efbf3d151a414caa428c66c368d4b0b73e6`.

## Remaining gate before a test IMG

This is an allocation increment, not completed native World Objects export. The engine has separate actor-local collision and a shared wall/floor grid. The grid contains no general per-instance wall owner IDs. The local documentation also establishes that MAN scripts can paint walls (for example the south gate), and bound actors carry interaction and animation state. Neither copying nearby wall bits nor clearing the bind bit is a valid solution.

The next required work is evidence-backed collision and script dependency resolution: identify authored grid writes and all shared consumers, preserve unknown/shared wall cells, and establish which instance resources can be transferred without losing collision or interaction. A bound copy additionally requires an independent MAN context and trigger linkage with validated cross-context references and compressed-resource limits. Existing documentation references disassembly dumps under ghidra/scripts/funcs, but that directory is absent from this checkout; documentation and live-parity tests are available, while direct instruction evidence must be recovered before widening semantics.

Do not interpret this report as approval to relax those gates. No safe environmental test IMG is ready yet, and no runtime claim is made.
