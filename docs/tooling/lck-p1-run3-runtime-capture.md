# Run 3: minimal read-only retail runtime capture

Use your unmodified US `SCUS_942.54` disc. Do not load a generated image or apply
cheats/patches for this capture. These are CPU addresses and debugger operations,
not version-specific DuckStation hotkeys/menu names. If your build cannot log
breakpoints or export memory/VRAM, provide its version and an unmodified save
state instead of guessing commands. No emulator was operated by LCK development.

## Files and identity

Record DuckStation version, disc SHA-256, BIOS, hardware-renderer/enhancement
settings and the save state's scene/story state. Use a fresh capture directory:

```bat
mkdir D:\AstraRimElmProject\Backups\LCK_Run3_Runtime
```

Export **main RAM** `0x80000000`, length `0x200000`, and **scratchpad**
`0x1F800000`, length `0x400`, at the checkpoints below. Export raw 16-bit VRAM
1024x512 (1,048,576 bytes, record byte order) if supported, plus a save state.
A screenshot/PNG alone cannot establish raw palette/STP words or write lifetimes.

## Session A — town01 allocation and texture lifetimes

1. Break on `0x8001E1B4` (stage init). Continue other scene stops until the raw
   CDNAME word at `0x80084540` is `0x0003` (`town01`). Dump the pre-load state.
   Re-enter through the normal loader if using a state already settled in town.
2. Log `0x80017888` allocations: at entry record PC, `a0` (heap index), `a1`
   (requested bytes), `ra`, `sp`, `gp`; at its caller return address record `v0`
   (returned pointer), paired with that request. Log every heap index, not just
   zero. Log frees at `0x80017B94` with PC, all `a0..a3`, `ra`, `sp`, `gp`.
   Do not infer the free wrapper's argument layout from the malloc wrapper.
3. Log `0x800583C8` (native LoadImage wrapper): `a0` rect pointer, four little-
   endian halfwords `(x,y,w,h)` at that pointer, and `a1` data pointer. Log
   `0x80058490` (MoveImage wrapper) with the input rect and all argument registers.
   Record any additional GPU upload/copy paths your debugger trace exposes.
   Preserve upload order, repeated/conditional writes and DMA data; a final
   texture scan is insufficient. Record `0x8007B998` (STP-OR latch).
4. At settled, controllable town01, dump RAM/scratchpad/VRAM. Repeat after idle,
   a walk around the intended import area `(X,Z)=(12096,10048)`, opening/closing
   the menu, and a natural scene return. Capture a normal battle transition and
   return when available on that save. Record CDNAME at every snapshot; town0b
   or town0c is a separate destination contract, not a town01 substitute.
5. Record these words at every checkpoint:
   `0x8007B774` next model index, `0x8007BB38` last registered index,
   `0x8007B6F8` scene prefix, `0x8007B824` persistent watermark,
   `0x8007B85C` bundle arena pointer, `0x8007C348` actor free-stack counter,
   and `gp+0x840` heap descriptor reference.
   Log writes to `0x8007C1F4` / `0x8007C1F8` (planned global model indices
   119/120). Stale pointer contents beyond the live count are not occupancy or
   proof of ownership. Passing the index-200 guard is not capacity certification.

Needed result: paired allocation/free history and heap state sufficient to
recover largest contiguous free blocks and allocation high-water marks **at the
model-pack allocation and later actor/primitive allocation points**, including
alignment/header overhead and fragmentation. Compare/replay proposed larger
requests in that order; total free bytes alone are insufficient. Model registry
alias/extent evidence must also support the proposed indices for this route.

Needed VRAM result: ordered upload/copy rectangles and raw words covering the
new package rectangles printed by the native fixture, boot/shared resources,
CLUT cycles, menu and transition writes. Current code already reserves display
buffers, capture pages, the native transition clear strip and verified boot UI
upload footprints (including zero words), but these are not
a complete free-VRAM map. Unvisited script branches remain unproven; snapshots
alone cannot prove that future writes never touch an allocation.

Current two-object fixture demand: 116 scene models + five residents = 121 live
registrations if there are no additional contributors. TMD decoded pack grows
304116 -> 307008 bytes (+2892); the descriptor's total 463285 is **not** a total
RAM budget. New image-word uploads are `(880,128,16,64)` and `(880,64,16,64)`;
palette upload envelopes are `(832,0,16,1)` and `(848,0,16,1)`. Required allocator
evidence includes later per-actor/primitive allocations and frees, not just the
2892-byte model-pack increase.

## Session B — original donor physical behavior

Use existing unmodified saves in these cells, without patching player position:

| Cell | Descriptor/model | Native source grid | Native world X/Z | Expected pool index if prefix=5 |
| --- | --- | --- | --- | --- |
| Biron `bylon` | 291 / 10 | (28,102) | (3648,13120) | 15 |
| Hunter's Spring `izumi` | 36 / 2 | (29,8) | (3712,1088) | 7 |

Approach until the original object is actually spawned in the sub-area window.
Dump RAM/scratchpad/VRAM; identify the live actor by **both** native position
and `actor+0x64` signed model index (many actors may share a model). Record its
pointer, `+0x10` live flags, `+0x14..+0x18` position and `+0x48` model pointer;
the full RAM dump preserves the rest without inventing collider-field offsets.
MAP flags `0x16` / `0x36` and cull radius alone do not establish its collider.

In the **field overlay only**, capture a stop/trace at `0x801CF9F4` during
normal movement collision near the prop, with all argument/result registers,
PC/RA and RAM. Disable that breakpoint while another overlay occupies the
same addresses. Also capture natural floor-query calls at `0x80019278` around
the footprint, with arguments and returned `v0`. Describe whether the player
is blocked, slides past, passes through or stands on a native surface; include
source actor script/context state and any observed collider/wall changes.

Needed result: independently identifiable collider class/extents/enabled state,
whether a walkable surface/kind-2 context is required, and which scripts alter
it. Zero mesh wall candidates, no direct MAN binding or zero collider offset
is **not** evidence that the native object is nonblocking.

These captures supply missing evidence for a narrowly scoped experiment; they
do not certify every scene, every story branch or a universal RAM/VRAM budget.
