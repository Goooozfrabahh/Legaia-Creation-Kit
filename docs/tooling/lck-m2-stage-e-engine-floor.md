# M2 Stage E: experimental native hybrid floor engine

This milestone implements executable native engine support, not just a codec.
It remains isolated from the production IMG planner and installed M1.1 build.
The output scope is one explicit native floor component, not a moved mesh or the
six-object benchmark. No wall or script ownership is inferred from floor data.

## Engine changes

`legaia-code-hooks::extended_floor` generates 480 bytes of R3000 instructions in
the guarded existing SCUS arena `0x8007AB80..0x8007AD60` (capacity ends at
`0x8007AF00`). It rejects occupied storage and overlaps with known live tables.
The studio adapter requires exact NTSC-U retail image, SCUS and field-overlay
profiles. Other mods, revisions and already-patched inputs fail the fingerprint
guards. No original executable is edited in place.

Three two-instruction hooks modify `FUN_80019278`:

| Site | Function |
|---|---|
| `0x80019278` | Replay the retail prologue; initialize frame-local actor pointer and query mode |
| `0x800192FC` | Replay grid load/sign branch; optionally bypass the kind-2 path for a bilinear-base query |
| `0x80019588` | Common return path, including the flat-floor shortcut; resolve sparse overrides then restore the retail frame |

Seven direct SCUS callers were found: `80025068`, `800394E4`, `8003BA1C`,
`8003BA64`, `8003BAC0`, `8003BC98`, `8003BCBC`. Their call sites are unchanged.
Indirect calls to the original entry also enter the hook. The retail floor
routine and its native primary/fallback lookups still perform all height math,
integer rounding and live floor-ladder reads.

For an override, a stack-local actor proxy carries translated X/Z coordinates
and the actor's flags. A guarded nested query bypasses the extension resolver,
preventing recursion. Its result and native surface flags propagate back; the
real actor's coordinates never change. Original callee-saved registers, return
address and stack pointer are restored. Bilinear-base mode masks special floor
bits only in the loaded register; it never temporarily modifies the shared MAP.

Unchanged query locations return retail results. A scene with no valid extension
header follows the retail path. The `0x1000` actor-surface flag is distinct from
the `0x0800` kind-2 height selector; retained flags and surfaces are not erased.

## Representation and allocation

The extension occupies checked zero padding immediately after the aligned end
of a MAP's parsed native trigger tables. Its location is computed at runtime
from that native end offset. Native headers, table counts, strides, row order,
fallback, descriptors, wall/floor tiers and object-grid words remain unchanged.
It does not overwrite arbitrary zero runs or append into neighboring resources.

The existing loader already reads this storage within its `0x14000`-byte scene
window. No loader sector count changes, heap allocation or new resource pointer
are required. Normal queries use the existing 40-byte retail frame. Redirected
queries require 72 additional stack bytes over the corresponding retail query
path: a 32-byte proxy plus the outer retained 40-byte frame. Existing lookup
stack usage remains in addition. Instructions/data and accesses are aligned.

The little-endian header is `LXF1`, u16 version 1, u16 row count. Each 12-byte row
contains destination 64-unit subcell X/Z, mode, reserved byte, signed i16 source
X/Z offsets, signed i16 height delta, and two reserved bytes. Mode 1 delegates
to the original source surface; mode 2 restores the original local bilinear
base. Counts, version, bounds, native coordinate range and serialized records
are checked. The current bounded budget is 84 rows, further limited by actual
available MAP padding. Runtime bounds also keep the table inside its block.

The original kind-2 row remains immutable sampling data. For a move, four sparse
rows restore the vacated source base and four rows redirect the destination to
the original staircase. Retained destination subcells still use the original
continuous bilinear function, not approximated constant steps. Live source-tier
changes continue to affect the moved surface. There is no destination 48-unit
shared-coarse-byte restriction because neighboring subcells can use different
sampling modes. This directly handles the requested 64-unit discontinuity.

The format-aware backend accepts explicit 64-unit translations and copies within
one MAP. Overlap with the selected source is composed deterministically; overlap
with unrelated kind-2 contributions, occupied storage and stale source data is
rejected. This is not arbitrary-angle terrain editing, cross-cell floor-resource
allocation, independent actor allocation, or universal export completion.

## Actual native evidence

The controlled regression extracts the real retail executable, field overlay and
town01 MAP. It compares 3,840 queries across step boundaries, retained slopes,
vacated source areas, four actor-flag states, the original and changed live floor
ladders, with integer load delays enabled and disabled. Strict halfword/word
alignment checks are enabled. The maximum observed execution is 1,449 modeled
instructions per query, not a measured PS1 cycle/frame budget.

Additional queries on unchanged Retona and Vell MAPs compare 96 samples each
against unpatched retail code, including native floor results and actor flags.
The actual exported IMG was reparsed and its scene reassembled; the same CPU
tests then ran its extracted patched SCUS/MAP bytes. The test also checks that
the sampler never changes MAP bytes at runtime or actor X/Z coordinates, and
preserves caller register/stack state.

The CPU toolkit now has optional R3000 integer load-delay and strict-alignment
checks. It remains a bounded functional subset with sparse memory, not a complete
or cycle-accurate PS1. GTE/whole-game, cache, interrupt, CD streaming lifetime,
performance, actual hardware and native gameplay compatibility are not verified
by these tests. No desktop application or emulator was launched.

## Experimental IMG and command

Artifact directory:
`D:\AstraRimElmProject\ManualTests\M2_StageE_20261027_01`.

`LCK_EXPERIMENTAL_STAGE_E_town01_floor_x37_z18.img` contains the explicitly
selected floor row `[37,18,2,CC]` moved by X=-320, Z=+64, Y=0. The table is at
MAP `0x104F8`, 104 bytes. Only raw sectors 43, 238 and 477 changed; their Mode2
Form1 EDC/ECC validates. Every unrelated PROT resource and every byte outside the
allowed sector set is unchanged; expected SCUS and MAP bytes are read back.
Output SHA256: `7b62310192a1b61b03991b5bcc0670d201be01be7de5521197aa30e95b88e1da`.

The separate `lck-native-floor` command never creates a GUI or launches an
emulator. Arguments are:

```text
lck-native-floor SOURCE_IMG ABSOLUTE_EXPERIMENTAL_OUTPUT_IMG CELL TILE_X TILE_Z DX DZ DY [--copy]
```

It uses create-only staging/publication, emits a scoped JSON receipt, validates
native bytes and raw sectors, and protects retail inputs. The normal GUI does
not yet author or display this extended walkability representation. The floor
test IMG is ready for user-run experimental DuckStation testing; its behavior
has not been gameplay-verified. Latest Build remains M1.1.

## Wall/script dependency work and remaining integration

`native_script_relocation` now relocates explicitly selected MAN wall-paint
instructions in place. It handles both native six-byte clear/set and seven-byte
masked forms, preserves instruction size, mask, context references, other bytes
and script layout, and compares exact decoded subcell effects including native
Z-row bias/wrapping. Bounds, stale sources and non-instruction selections fail.
Nine actual town01 wall instructions (all in flat context 20) passed native MAN
relocation/readback. This provides a reusable dependency serializer, not proof
that the script or its wall cells exclusively belong to a particular object.
No such MAN edits were added to the floor-only IMG.

The existing house 38-operand reference writer and controlled real-house
regression are retained. Independent MAN context allocation, MOVE/animation
closure, door/teleport behavior and static wall contribution ownership remain
unresolved. The six-request GUI pipeline still retains all six edits and rejects
these dependencies. It has not been permitted to substitute this standalone
floor patch or omit requested objects. No complete six-object IMG is claimed.

Automated results: 265 studio library tests, 9 integration tests and 85 CPU-toolkit
tests pass; 25 studio controlled cases and one CPU controlled case are excluded
by default. Real-data CPU/IMG, wall-script, house-reference and six-request
rejection cases were run explicitly. Release compilation produces a separate
M2 GUI candidate and headless writer; neither replaces installed M1.1.
