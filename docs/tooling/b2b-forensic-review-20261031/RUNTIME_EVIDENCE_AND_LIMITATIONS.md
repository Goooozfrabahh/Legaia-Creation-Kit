# Runtime evidence and limitations

## User-reported results

A retail control LOADS; B Balden alone LOADS; C Bubu1 alone LOADS.
M both resource sets with no imported actors FREEZES. User also confirms every
other combined-resource variant freezes. DuckStation itself becomes unresponsive;
another capture could not be made. Loading alone is not verified animation,
dialogue, behavior preservation, or broad gameplay compatibility.

## Saved-state observation (E, not M)

The preserved user-created v77 state identifies E as its media, SHA256
`4e58c04fbda95bac7b7498bf2bb0a2ee652903eb6bf2d237642f846a67b79513`.
Decoded offline: PC `0x8005FD40`, RA `0x8005FC1C` in the wait/VSync path;
resident MAN `Mt. Dhini`, counts `[3,14,19]`, 77 models, failed-spawn counter0;
transition actor `0x8008077C`, handler `0x80021934`, state3. It polls CD-sync.
Cached first4KiB of PROT TOC match E. Pending raw TOC6/extraction4 read requests
113 sectors with113 remaining; CD state `SeekingLogical`, source internal LBA28890,
target635, setloc binary `[0,8,35]`. The target buffer is not yet the new bundle.
The actual arena is404480 bytes and the pending bundle231424, so that read fits.
Old-scene free heap214920 bytes in two blocks is PRE-CLEANUP, not a target heap test.

This snapshot may precede the actual application hang. It cannot prove a persistent
seek failure, a native allocation failure, or the final frozen instruction. There
is no M runtime capture. No new capture was requested after the user reported
that the application itself was unresponsive. No assistant host interaction occurred.

## Allocation and lifetime boundaries

The JSON retains the original-instruction request chronology (synthetic pointers
are NOT actual PS1 addresses), plus independent descriptor accounting. Five
persistent calls load type2/3/5/6/7. Each compressed destination additionally uses
4116 bytes of temporary decoder scratch, freed on decoder return; that scratch
is omitted by the simulator's substituted decompressor and therefore added only
in the accounting model. Copy-only PAC TIM lists use the source arena; their
zero destination-allocation entry does not certify GPU completion or residency.
The fixed encoded arena is404480 bytes, retail heap1263616. Native heap headers,
fragmentation, existing actors, cleanup, resident resources, sidecars, rendering,
audio, DMA/GPU packets and future allocations are not fully accounted.
Do not label remaining static arithmetic as available live memory.

Historical heap replay succeeded for A/B/C/E on one older arena-return snapshot;
all failed the first model allocation on another older preload snapshot, INCLUDING
retail A. M has not been replayed on a live M heap. This neither identifies nor
rules out a current cumulative-memory/lifetime defect. Source provenance remains
distinct from user gameplay results and from bounded original-instruction tests.

## Retained detailed evidence

No retail binary, IMG, save state, RAM, local settings or full logs are committed.
The source files below remain unchanged; JSON contains compact native identities,
allocation traces and the saved-state extract needed to interpret this package.

- `D:\AstraRimElmProject\ManualTests\LCK_B2B_Loading_20261031_01\ISOLATION_REPORT.json` â€” 35014 bytes; SHA256 `66312dd9bcc5b1c38dd9cdeb7be3b3b2c318df9aed3487181fc6be3808558fb1`.
- `D:\AstraRimElmProject\ManualTests\LCK_B2B_Combined_20261031_01\Variants_Final\COMBINED_REPORT.json` â€” 129121 bytes; SHA256 `2a34786584040b53e9dfe2ccbad14c032680fdf0441f15bf10493c418d279f14`.
- `D:\AstraRimElmProject\ManualTests\LCK_B2B_Combined_20261031_01\UserCapture\STATE_REPORT.json` â€” 2339 bytes; SHA256 `20671ab6a6d0c173ced27a2b7e7148747539c7105d458101da311b0286857998`.
- `D:\AstraRimElmProject\ManualTests\LCK_B2B_Combined_20261031_01\CAPTURE_HEAP_LAYOUT.json` â€” 430 bytes; SHA256 `086f89d38604f6f2bcde9b8cb12c3ec620491ea1bca8de35ebde7779b5a6ee76`.
- `D:\AstraRimElmProject\ManualTests\LCK_B2B_Combined_20261031_01\EXACT_SHARED_RESOURCE_COMPARE.json` â€” 1484 bytes; SHA256 `f53d47a2a9e14e5acf553439145392ffd1d990e1cbda3f8b92cc25180c4f7c0d`.
- `D:\AstraRimElmProject\ManualTests\LCK_B2B_Combined_20261031_01\ORDERED_TIM_TAILS.json` â€” 6374 bytes; SHA256 `0849e2fe4f4bd332f9f02053522471b95ad2539b24fb61c2b915353c7fe9eb5b`.
- `D:\AstraRimElmProject\ManualTests\LCK_B2B_Combined_20261031_01\FINAL_SECTOR_VALIDATION.json` â€” 11150 bytes; SHA256 `9117865d65c173d2f0b899e628614fe65a81c3e3421044c67de0ec5cf0e616d5`.
- `D:\AstraRimElmProject\ManualTests\LCK_B2B_Heap_20261031_01\HISTORICAL_HEAP_SUMMARY.json` â€” 3687 bytes; SHA256 `2beddea2f09e3862affc95a39db2231bd55fd2c05fdc0ddc0f61cae324e3af90`.
- `D:\AstraRimElmProject\ManualTests\LCK_B2B_Combined_20261031_Regression_01\REGRESSION_RESULTS.json` â€” 7344 bytes; SHA256 `9c1b0887f7cd37f08829a96b24b103363a5bb5c106baa6afd2ea0782d2c7142c`.
- `D:\AstraRimElmProject\b2b-combined-final-matrix.log` â€” 15919 bytes; SHA256 `1b6cf711833e228f752e2f00c0fc1ac0eb91a1c5ebd7af7578b8eb72026ec249`.

## Existing emulator log

{
  "path": "D:\\Emulators\\Ps1\\Duckstation\\duckstation.log",
  "bytes": 15886,
  "sha256": "00036fa5b2d19b6e1101833ce1ad4129277d20f5f90c7bb009c94ae6e41d7766",
  "scope": "read-only file snapshot; not emulator interaction",
  "extract": [
    "[    0.1701] I/System: Version: 0.1-8153-g31c1cfa65 [dev]",
    "[    5.8553] I/System: Boot Filename: D:\\AstraRimElmProject\\ManualTests\\LCK_B2B_Combined_20261031_01\\Variants_Final\\LCK_EXPERIMENTAL_F_REVERSE_MODELS.img",
    "[    5.8554] I/System: Loading CD image 'LCK_EXPERIMENTAL_F_REVERSE_MODELS.img'...",
    "[   22.1507] I/TTY: \u001b[1;34mmap read\u001b[0m",
    "[   22.1510] I/TTY: \u001b[1;34mdecode\u001b[0m",
    "[   22.3070] I/TTY: \u001b[1;34mmap patch\u001b[0m",
    "[   22.3183] I/TTY: \u001b[1;34mtmds 77\u001b[0m",
    "[   22.3183] I/TTY: \u001b[1;34mcamera set \u001b[0m",
    "[   22.3184] I/TTY: \u001b[1;34manm set \u001b[0m",
    "[   22.3185] I/TTY: \u001b[1;34mman set \u001b[0m",
    "[   22.3228] I/TTY: \u001b[1;34m    town mode 14\u001b[0m",
    "[   23.3489] I/TTY: \u001b[1;34mgame mode\u001b[0m",
    "[   24.8814] I/TTY: \u001b[1;34mmap name town01\u001b[0m",
    "[   24.8815] I/TTY: \u001b[1;34mMAP NAME town01\u001b[0m",
    "[   25.1991] I/TTY: \u001b[1;34mmap read\u001b[0m",
    "[   25.1993] I/TTY: \u001b[1;34mdecode\u001b[0m",
    "[   25.4381] I/TTY: \u001b[1;34mmap patch\u001b[0m",
    "[   25.4492] I/TTY: \u001b[1;34mout read 20\u001b[0m",
    "[   25.8912] I/TTY: \u001b[1;34mtmds 121\u001b[0m",
    "[   25.8913] I/TTY: \u001b[1;34mcamera set \u001b[0m",
    "[   25.8913] I/TTY: \u001b[1;34manm set \u001b[0m",
    "[   25.8913] I/TTY: \u001b[1;34mman set \u001b[0m",
    "[   25.8956] I/TTY: \u001b[1;34m    town mode 55\u001b[0m",
    "[   26.6250] I/TTY: \u001b[1;34mgame mode\u001b[0m",
    "[   26.6251] I/TTY: \u001b[1;34mmalloc err size 8368\u001b[0m",
    "[   26.6252] E(ReadBlockInstructions): Instruction read failed at PC=0x000000A0, truncating block.",
    "[   26.6252] E(CompileOrRevalidateBlock): Failed to read block at 0x000000A0, falling back to uncached interpreter",
    "[   26.6252] E(ReadBlockInstructions): Instruction read failed at PC=0x80000080, truncating block.",
    "[   26.6252] E(CompileOrRevalidateBlock): Failed to read block at 0x80000080, falling back to uncached interpreter"
  ],
  "snapshot_path": "D:\\AstraRimElmProject\\ManualTests\\LCK_B2B_Forensic_Review_20261031\\duckstation-log-readonly-snapshot.log"
}

This log is explicitly F, not M or the earlier E saved state. The logged 8368-byte allocation error is observed runtime evidence; its caller and M equivalence remain unproven. It supersedes treating allocation failure as purely hypothetical for F, but does not certify a fix or explain every combined-resource failure.
