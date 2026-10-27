# Bounded town01 runtime capture (user operated)

The agent has not operated DuckStation or changed its installation/settings. The utility is a user-run debugger observer. It reads RAM/registers and installs temporary execute breakpoints; it does not write guest RAM/registers, patch the executable, or supply controller input.

## Interface evidence

Exact [DuckStation revision 31c1cfa65 GDB server](https://github.com/stenzek/duckstation/blob/31c1cfa65/src/core/gdb_server.cpp) supports register/memory reads, execute breakpoints, continue and interrupt. It does **not** implement single-step. The utility uses verified straight-line prologue gateways instead. The old stub does not automatically remove breakpoints on disconnect; cleanup is explicit. The [settings implementation](https://github.com/stenzek/duckstation/blob/31c1cfa65/src/core/settings.cpp) exposes `Debug/EnableGDBServer` and `GDBServerPort` (default 2345). The chosen workflow below uses 1234 explicitly.

Read-only installed-EXE metadata reported version 0.1.9787. This does not establish which executable produced the supplied older captures. Record the actual About version when capturing; compatibility has source/mock coverage, not live validation.

## One controlled clean-retail load

1. With DuckStation closed, manually enable its GDB server through available settings, or set these entries in the configuration used by your installation:

   ```ini
   [Debug]
   EnableGDBServer = true
   GDBServerPort = 1234
   ```

2. Boot the **original retail** image. Do not use full save states from modified images. Pause before the town01 loading sequence. Use a dedicated debugger session with no existing breakpoints (the old stub cannot enumerate breakpoint ownership).
3. In Windows CMD, run the following. Replace the version label with the actual About version. The output folder must not already exist.

   ```bat
   cd /d D:\AstraRimElmProject\legaiasource\legend-of-legaia-re-main
   python scripts\lck_runtime_capture.py trace --source-image "D:\Emulators\Games\Legend of Legaia\LegendOfLegaia.img" --out-dir "D:\AstraRimElmProject\Backups\LCK_Run4_Town01_Trace_01" --port 1234 --seconds 120 --max-events 4000 --max-mib 16 --emulator-version "YOUR-ACTUAL-ABOUT-VERSION"
   ```

4. The utility resumes execution. Complete the one loading transition with your normal controls, then let town01 settle. It stops at the time/event/byte bound and attempts to leave the emulator paused after clearing its breakpoints. Initial/final dump overhead is outside the observation interval. Inspect the final JSONL record for failures, unmatched returns and remaining breakpoints before resuming manually. If cleanup fails, manually clear the listed breakpoints or restart the game; disconnect alone does not clear them in the older stub.

Hooks: allocator `0x80017888`, free `0x80017B94`, LoadImage wrapper `0x800583C8`, MoveImage wrapper `0x80058490`, model registration `0x80026B4C`, stage initialization `0x8001E1B4`. Hook instruction profiles are compared against your read-only SCUS before execution. Paired entry/return events use RA/SP and include scene, model/arena state and before/after heap samples. Upload records preserve submitted bytes/hash and the STP latch. Initial/final RAM dumps and bounded JSONL/blob output are included.

This records observed heap extrema and ordered wrapper submissions, not every possible allocator/GPU path or DMA completion. A bound/error can truncate the transition. Direct GPU writers, later script-driven activity, actor/primitive allocations and non-observed lifetimes may still require separate evidence. Do not turn observations into an unconditional production budget.

## Offline reproduction and tests

```bat
cd /d D:\AstraRimElmProject\legaiasource\legend-of-legaia-re-main
python -m unittest discover -s scripts -p test_lck_runtime_capture.py -v
if not exist target\lck-test-temp mkdir target\lck-test-temp
set "TEMP=D:\AstraRimElmProject\legaiasource\legend-of-legaia-re-main\target\lck-test-temp"
set "TMP=%TEMP%"
cargo build -p legaia-studio --locked
cargo test -p legaia-studio --lib --locked
set "LCK_P0_SOURCE=D:\Emulators\Games\Legend of Legaia\LegendOfLegaia.img"
set "LCK_RUNTIME_CAPTURE_DIR=D:\AstraRimElmProject\Backups\LCK_Run4_CaptureFixtures_20261027"
cargo test -p legaia-studio --lib --locked real_town01_pac_packaging_and_native_donor_gates -- --ignored --nocapture
```

These tests do not operate DuckStation. The retail fixture reads the source image; no image is emitted unless both explicit experimental output and acknowledgement variables are provided. To reproduce the visual artifact, choose a **new directory** and run the same ignored test with:

```bat
set "LCK_VISUAL_EXPERIMENT_OUT=D:\AstraRimElmProject\Backups\LCK_Run4_Visual_20261027_02\LCK_EXPERIMENTAL_VISUAL_ONLY_town01_two_foreign_props_20261027.img"
set "LCK_VISUAL_EXPERIMENT_ACK=VISUAL_ONLY_UNVERIFIED_PHYSICS_AND_RUNTIME"
cargo test -p legaia-studio --lib --locked real_town01_pac_packaging_and_native_donor_gates -- --ignored --nocapture
set "LCK_VISUAL_EXPERIMENT_OUT="
set "LCK_VISUAL_EXPERIMENT_ACK="
```

This reproduces an explicitly incomplete visual experiment, never a production-approved object export. Existing output directories/images cannot be overwritten.
