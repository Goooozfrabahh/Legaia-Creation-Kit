# Legaia Creation Kit v2.2.0

Baseline checkpoint for the R648 Universal Native Builder development phase.

Checkpoint date: 2026-11-14 02:48:41
Git branch: lck/v2.2
Based on: lck/v2.1
Local source snapshot: D:\AstraRimElmProject\LCK\source\R637_Merged_20261113_215415_892

## Current working foundation

- LCK 2.1 native IMG export foundation retained.
- World Repository improvements and visually unique asset browsing.
- Cross-cell asset drag/drop and independently tracked instances.
- Native MAP edits, floor edits, collisions, doors and warp workflows.
- Existing native resource registration and IMG export staging.
- Native 128-grid expansion experiments and 256-grid authoring interface.
- Original editor data and donor provenance retained.

## Known limitations at this baseline

- The 256x256 authoring grid is not yet a complete native PS1 runtime expansion.
- Native MAP reference allocation can fail despite unused descriptor IDs.
- Multiple objects may need an independent runtime instance system.
- Arbitrary positive object scaling still needs a native transform adapter.
- Imported terrain, visual floors and walkable surfaces do not yet share
  a fully universal native resource/instance export contract.
- Cross-cell assets may require TMD/TIM/CLUT relocation and runtime verification.
- IMG readback does not itself verify in-game rendering or interaction.
- No claim is made that universal native exporting is complete.

## Next milestone: R648

1. Identify native instance and runtime loader capacity.
2. Resolve object placement independently of an empty MAP grid reference.
3. Implement additional native instance storage or supported runtime hooks.
4. Support native 256x256 authoring-space activation where feasible.
5. Unify static resource registration across World Objects, Terrain and Floors.
6. Implement category-specific floor/collision/transform serialization.
7. Verify model registration, MAP changes, IMG publication and runtime behavior.
8. Preserve original game content and all existing exportable editor changes.

## Build

Package: legaia-studio
Package version: 2.2.0
Executable: releases/LCK_2_2_0/Legaia_Creation_Kit_2.2.0.exe
SHA256: E1AA61EB748021EBF6F27FFF3E6CFD971C602ABFA67BF9128EE2FCF0B280BF16

Cargo check: PASS
Release build: PASS
Full test suite: NOT RUN as part of this checkpoint
DuckStation gameplay: NOT RUN as part of this checkpoint

This is a development baseline, not a universal-export completion release.
