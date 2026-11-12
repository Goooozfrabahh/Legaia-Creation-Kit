# Native 256-grid implementation checkpoint — 2026-11-11

Status: **implementation building blocks tested; no installed runtime expansion**.
The pinned LCK executable and retail/test disc images were not changed by this
work. The existing extended-grid manifest remains truthful: native runtime
extension is not enabled. No installer may describe this checkpoint as playable.

## Established from files

The authoritative source inspected was
`Backups/LCK_0436H_FAST_20261106_203504/src` in the user's project. Binary evidence
comes from the fingerprinted retail SCUS and field overlay 897, not assumptions
about GECK or DuckStation. See `lck-256-native-audit-20261111.json` for hashes,
saved-project measurements, historical heap evidence and pointer candidates.

* Native MAP has 128x128 collision bytes and 128x128 object words. A 256 editor
  grid currently changes authoring bounds and companion metadata, not those
  native tables. `ground_edit_ops::w2b_cells` still clips to 0..128.
* `0x801D699C..0x801D69A0` supplies 0x14000 bytes to the allocator call at
  `0x801D69EC`; `0x801D6A00` publishes the result to scratchpad MAP pointer
  `0x1F8003EC`.
* `0x8001F9B0` requests 0x28 sectors from the CD reader; the map is followed
  by the fallback/PCH and sound window. The sound pointer is established at
  `0x8001F848..0x8001F864` with offset 0x12800.
* The floor sampler masks the object-grid coordinate (`0x800192D4`,
  `0x800192E4`), but calculates its collision pointer separately
  (`0x80019364..0x8001938C`). Out-of-range access is therefore not a reliable
  empty-ground result. A global constant replacement would not fix it.
* Native wall probes also mask coordinates to the old grid. The standalone
  probe at `0x801D56C4` uses a 64-unit quadrant mapping and a +2 half-tile Z
  bias. Its mask sites are `0x801D570C` and `0x801D571C`. There are additional
  inlined probes in movement and ledge handling. The new resolver does not
  pretend that ordinary floor coordinates replace these wall semantics.
* `0x801D7B50` rebuilds the visible static-object list. It reads the latched
  region rectangle as unsigned bytes (`0x801D7C04..0x801D7C38`) and indexes
  the grid at `0x801D7C48..0x801D7C60`. This is a second visibility constraint
  in addition to storage bounds. Merely enlarging the MAP cannot display a
  room excluded by that rectangle.
* MAN instructions write the collision grid directly; examples are
  `0x801E1CE4..0x801E1D00`, with other arms at `0x801E1D58`, `0x801E1DD0`,
  and `0x801E1E54`. Read hooks alone would leave writes using the old stride.
* Native object descriptor indices are still nine bits. Expanded spatial
  coordinates do not create additional descriptor slots or texture memory.
* The existing `DiscPatcher::grow_prot_entries` can relocate larger entries
  and update the disc structures. It does not change native allocation or
  loading. Preserve its complete ISO/raw-sector relayout verification.

The audit's 60 pointer-offset candidates include false positives (for example
stack offsets); they are an investigation index, **not proof of exhaustive
consumer coverage**. Aliased pointers require additional tracing. The SCUS floor
sampler is also used outside the field scene: legacy/world-map paths need to
retain their existing layout.

## Memory result and implementation choice

The historical settled Rim Elm capture's allocator free list has one 17,780-byte
free block. The historical arena-return capture has 832,432 free bytes. These
are distinct lifecycle snapshots, not current gameplay captures. A large
allocation may fit during loading and still starve the settled scene.

A dense 256 window would require 0x38000 bytes rather than 0x14000, an additional
147,456 bytes. Do not enable that allocation globally.

Use the original MAP for original cells and a versioned sparse extension for
external cells. The implemented LKG1 format has a 16-byte header, two-byte page
directory entries, and 768 bytes per occupied 16x16 page. Pages contain 256
collision bytes and 256 object words. Empty pages are absent; unassigned cells
inside an allocated page default to collision 0xF7 and object word zero.

At side 256, the signed tile domain is [-64,192), world domain
[-8192,24576). Authored coordinates remain unchanged. Storage indices are
translated; the side room is not moved into a different part of town.

DOORROOM's saved ground requests occupy 2,169 distinct external tiles in 12
pages: 9,744 encoded bytes, with no request crossing the 256 boundary. This
counts grid occupancy only, not geometry, texture residency, code, allocator
headers, new descriptors or slope overrides. Overlapping floor requests do not
each require their own page. The project itself was not modified.

## Code delivered in this checkpoint

`crates/code-hooks/src/field_grid.rs` implements:

1. Checked signed coordinate conversion and layouts for 128/160/192/224/256.
2. Dense window conversion as a reference/storage utility. Preserves descriptor,
   grid, primary-trigger, fallback and sound bytes; not a runtime allocation plan.
3. Sparse page encoding/decoding with version, length, origin, directory and
   canonical page-reference validation. Original-grid writes are rejected by
   this extension API; they belong to the original MAP compiler.
4. Relocatable R3000 dense and sparse address routines. The sparse routine
   returns original MAP addresses for original cells; external cells resolve
   through allocated pages. Absent/outside cells return zero addresses. Callers
   must handle that result and must never dereference it.
5. Tests under the existing R3000 subset interpreter with load delays and
   alignment checking. The routines allocate no memory, use no guessed RAM
   cave and preserve argument/callee-saved registers.

`scripts/audit-field-grid-runtime.py` reproduces the binary-profile checks,
historical free-list measurements and saved-project page count without running
LCK or any emulator. It refuses to overwrite earlier output evidence.

## Still required before a playable installer

* Select, allocate, load, validate and release each cell's sparse payload.
  Establish a valid extension pointer throughout initialization, transitions,
  battle return and repeated cell entry. Do not borrow an unproven RAM cave.
* Compile real external floor/object data into those pages, including source
  donor identity, descriptor sharing, height/slope semantics, textures and
  deletions. Grid occupancy is not an encoded visible floor.
* Adapt all affected readers and writers. Integrate the existing Stage E /
  extended-floor hooks rather than installing mutually overwriting jumps.
* Replace or extend region bounds and spawning for signed external coordinates;
  retain native indoor-room isolation and camera behavior. An unsigned byte
  rectangle cannot represent negative coordinates or exclusive endpoint 256.
* Cover external triggers/warps and MAN coordinate operations without narrowing
  signed coordinates into bytes. Keep untouched scenes/world-map behavior.
* Verify disc readback, runtime allocation lifecycle, native functions and
  scene initialization. Obtain user gameplay results in DuckStation separately.

No change to `native_runtime_extended` or export-success wording is justified
until this integration is complete. User acceptance of permissive export is
not evidence that missing runtime consumers have been implemented.

## Automated validation

Command: `cargo test -p legaia-code-hooks field_grid:: --lib --offline`.
Four tests pass. Address tests execute 583,680 ordinary tile queries across
all five sizes, plus out-of-range/header/legacy checks. Codec tests cover native
byte preservation, malformed/truncated payloads and original-grid isolation.
This is component verification, **not DuckStation gameplay verification**.

The complete `legaia-code-hooks` library test run also passed: 93 passed,
0 failed, 1 existing ignored test. No optimized editor build was published,
because the native integration listed above is still incomplete.
