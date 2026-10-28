# Observable Stage E floor test — replacement procedure

The previous invisible-floor-only gameplay procedure was insufficient. Preserve
that IMG as a regression artifact; use this separate observable build for the
next test. The user has reported opening Stage E town01 in LCK with geometry and
textures intact. No Stage E DuckStation gameplay result has been reported.

Artifacts: `D:\AstraRimElmProject\ManualTests\M2_StageE_Observable_20261027_01`.
The new IMG is `LCK_EXPERIMENTAL_FLOOR_OBSERVABLE_RimElm.img`, SHA256
`4baafe967a6f02ebb79567e9ce4dc44b7d18e0fba18fa973c91206ec606275ed`.
Its adjacent JSON is the code fingerprint and per-cell comparison oracle.

## 1. LCK debug visualization

Personally launch the separate `bin\legaia-studio-M2-observable.exe`, open the
new IMG (or the preserved Stage E IMG), and open town01. Enable **Native floor
comparison**, beside Show walk-ground / Collision wireframe.

The viewport draws original orange and Stage E cyan elevation wire surfaces.
The separate comparison window shows exact half-open 64-unit region boundaries,
original/relocated center heights, source/destination categories and stored wall
occupancy. Hover the plan for the exact original and Stage E integer height at
that X/Z. The wire display uses an eight-unit sample lattice and does not join
surfaces across discontinuities. Lines between samples are visualization, not
additional native collision records. Stored native floor data, not mesh bounds,
determines every sample. The model was compared against original and patched
MIPS execution at 512 positions; actual GUI drawing remains user-verification.

Source: X `[4736,4864)`, Z `[2304,2432)`.
Destination: X `[4416,4544)`, Z `[2368,2496)`.
Heights are native PSX Y-down; the viewport converts to editor Y-up.
Preview uses the stored MAN ladder; live game results come from telemetry.

Opening the retail image without an extension shows an explicit unavailable
message. Opening a different cell reloads its own data; no Rim Elm object-ID
or geometry-proximity rule is used. This debug view does not change selection,
camera input, mesh data or normal IMG export behavior.

## 2. Reachability findings

All three principal Rim Elm variants have the same stored wall findings here:

| Destination center X,Z | Stored native wall |
|---|---|
| 4448,2400 | clear |
| 4512,2400 | clear |
| 4448,2464 | blocked |
| 4512,2464 | blocked |

The lower half is blocked under the stored native wall grid. Clear upper cells
do **not** establish an ordinary gameplay route: actor colliders, scripted walls,
floor/ledge classification and the player's actual start/story state also matter.
Full natural reachability is not verified. No request to walk there is required
for the replacement test. The monitor also records whether Vahn is actually
observed inside the destination rectangle; it does not fabricate a route.

## 3. Controlled native test entry point

The new image validates and applies the explicit floor test independently to
town01, town0b and town0c, so the existing save near Vahn's house need not be
identified in advance. Native data tags report the loaded variant as profile
1, 2 or 3. Other scenes keep their normal floor behavior and run no controlled
probes. No actor teleport, noclip, wall deletion or input automation is used.

The test observer runs after ordinary native floor queries. Every 32 outer
queries, it makes one immediate original-retail / Stage E comparison on a
stack-local proxy actor. The eight slots rotate: one samples Vahn's live X/Z;
seven sample fixed test coordinates. They need no player access to those places.
The original retail calculation bypasses the sparse redirect for its query;
the Stage E calculation uses it. Both execute the actual game instructions on
the same loaded MAP and live floor ladder. A recursion guard prevents proxy
queries from generating further probes. The real actor X/Y/Z is not written.

| Slot | X,Z | Retail height | Stage E height | Purpose |
|---|---|---:|---:|---|
| 0 | captured live Vahn position | recorded | recorded | live position probe; coordinates/sequence are explicit |
| 1 | 4448,2367 | 0 | 0 | just outside north boundary |
| 2 | 4448,2368 | 0 | -64 | relocated left step |
| 3 | 4512,2368 | 0 | -112 | relocated right step |
| 4 | 4384,2464 | -48 | -48 | retained bilinear slope |
| 5 | 4448,2496 | -96 | -96 | just outside south boundary |
| 6 | 4768,2336 | -64 | 0 | vacated source left |
| 7 | 4832,2336 | -112 | 0 | vacated source right |

These expected values match the stored initial ladders in all three variants.
Unexpected live ladder/story effects must produce a captured mismatch for
analysis; the reader never changes expected results to force a pass.

## 4. Read-only DuckStation monitoring setup — user performs these steps

The assistant has not launched, attached to, or read a running DuckStation.
The monitoring integration is prepared from official source and tested offline;
its operation with the user's installed DuckStation is not yet verified.

1. Close DuckStation yourself and back up its own `settings.ini`.
2. In the existing `[Hacks]` section set `ExportSharedMemory = true`.
3. To obtain the mapping name, enable Information-level file logging in DuckStation's logging
   settings, or set `LogToFile = true` under its existing `[Logging]` section.
   Do not duplicate whole INI sections. No Windows/security change is required.
4. Personally cold-boot the **new observable IMG**. Use a memory-card save or
   start a new game and enter the normal Rim Elm field. Do not load an old
   emulator save-state: it can restore old executable RAM and remove the patch.
5. Find the log line `Shared memory object name is "duckstation_<PID>".` in
   DuckStation's `duckstation.log`. If your build lacks this setting/name, stop
   and report the version/log rather than enabling an untested debugger route.
6. Leave ordinary gameplay unpaused in any of the three Rim Elm variants.
   Remain wherever Vahn already is; no trip to the staircase is needed.
7. Personally run **RUN_MONITOR.cmd** in this artifact directory. It opens only
   the Python reader; it does not start or control an emulator. Enter the exact
   exported mapping name. It records a new uniquely named JSONL for 60 seconds.
8. If counters do not advance while stationary, unpause/close game menus and
   take one ordinary step anywhere. Do not alter the test surfaces or walls.

The reader calls only `OpenFileMappingW(FILE_MAP_READ)` and a read-only mapped
view of the already exported PS1 RAM. It never creates that mapping, opens a
process handle, writes RAM, pauses/resumes execution, sends GDB commands or
injects mouse/keyboard input. The assistant has not run live mode.

Official implementation references:
[exported RAM and RAM offset](https://github.com/stenzek/duckstation/blob/master/src/core/bus.cpp),
[Windows mapping names](https://github.com/stenzek/duckstation/blob/master/src/common/memmap.cpp),
[configuration keys](https://github.com/stenzek/duckstation/blob/master/src/core/settings.cpp).
The setting can be reverted by closing DuckStation and restoring its backed-up
configuration. Neither retail IMG nor M1.1 needs modification.

## 5. Execution evidence versus correctness

The console and JSONL distinguish these cases:

- **LOADED_NOT_EXECUTED / NOT_EXECUTED:** a debug signature or displayed geometry
  alone is not execution evidence.
- **NATIVE_HOOK_EXECUTED_WAITING_FOR_PAIR:** native callback counters advanced;
  complete controlled coverage is still missing.
- **CONTROLLED_STAGE_E_EXECUTED:** fresh controlled queries ran in this session.
- **WAIT_FOR_EIGHT_FRESH_NATIVE_PAIRS:** stale or partial rows are not accepted.
- **PASS_FIXED_NATIVE_PAIRS:** expected runtime code hash, active cell tag, eight
  fresh samples and every fixed coordinate/retail/Stage E height match the oracle.
- **FAIL_HEIGHT_COMPARISON_CAPTURE_LOG:** actual returned values differ. Preserve
  the log and report them; do not call the test successful.
- **NO_ACTIVE_EXTENSION_IN_CURRENT_CELL / WRONG_OR_UNVERIFIED_CODE:** wrong cell,
  image, old save-state or build; not a floor-height pass.

The reader resets freshness after counter rollback or profile change. Busy/torn
snapshots are retried. Offline RAM dumps are explicitly labeled non-live and
cannot become a live pass. The packaged `OFFLINE_*` files are CPU-tool fixtures,
not DuckStation captures.

`live_vahn_xyz` reads the actual player pointer at `0x8007C364`, then actor
X/Y/Z at `+0x14/+0x16/+0x18`. Current actor elevation is separate from the sampled
ground height. Slot 0 includes its own sampled X/Z and sequence, so an older
ground probe is not confused with a newer moving player position. The most
recent natural floor query also records its actor pointer, position, retail
and Stage E return, and whether its argument is actually the live player.
An NPC query is never labeled Vahn's natural query.

## 6. What to return

Return the new JSONL capture, actual IMG path/hash, DuckStation version, and the
console verdict. If useful, include a screenshot of LCK's debug comparison.
A screenshot of unchanged geometry is not floor-physics evidence. A probe pass
verifies native sampler execution/results under DuckStation; it does not prove
ordinary walking accessibility, complete object/collision ownership, all story
behavior, actual console hardware or the six-object export.

## 7. Implementation and automated checks

The observable routine is a separately generated 884-byte profile inside the
existing guarded SCUS arena. A 256-byte trace/config area uses the guarded
`0x80077728..0x80077828` gap. Original Stage E generation remains byte-for-byte
compatible with the preserved image. Each scoped MAP adds a four-byte profile
tag after its sparse records; counts and native trigger structures are unchanged.
Only seven raw sectors differ in the new multi-variant IMG; all have valid
EDC/ECC and unrelated resources are preserved.

The native CPU checks validate all three written MAP variants, original Stage E
preservation, natural return/flag/register preservation, actual actor X/Y/Z
preservation, controlled pairs, tags, recursion guard and reader fixtures.
The old 3,840-query floor regression, changed ladders, R3000 load delays, strict
alignment and unchanged Retona/Vell behavior remain checked. Python tests cover
loaded-versus-executed, wrong heights/code, stale counters, cell switches, busy
snapshots and rejection of offline evidence as live proof.

M1.1 stays Latest Build. The prior Stage E IMG and recovery remain untouched.
New GUI and live DuckStation monitoring are **not verified** until the user
reports results. No GUI/emulator/desktop-control application was launched by
the assistant, and no settings on the user's emulator were edited.
