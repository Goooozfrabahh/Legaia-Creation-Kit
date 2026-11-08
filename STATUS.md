# LCK Development Status

## Current editor checkpoint - November 8, 2026

A verified local editor-interaction snapshot has been prepared
on a dedicated checkpoint branch based on lck/development.

The checkpoint includes the current Rust editor source,
persistent mixed-category groups, selection and marquee tools,
floor and collision authoring, group duplication, shared-center
rotation, and empty-space deselection.

## Verification

- Studio library regression tests: passed during checkpoint.
- Optimized studio build: passed during checkpoint.
- User-confirmed editor interactions: ongoing.
- Retail PS1 gameplay certification: not established.
- Original retail IMG: not modified by checkpoint.
- DuckStation: not launched by checkpoint.

## Git organization

- foundation-02 remains the historical default branch.
- lck/development remains the cumulative published baseline.
- The new checkpoint is separate until reviewed and merged.
- Historical branch references have not been removed.
- No force push or history rewrite was performed.

## Remaining work

- Universal native object/resource placement and export.
- Original retail collision and terrain ownership.
- Verified native floor walkability in all supported scenes.
- NPC, MAN, animation, trigger and script portability.
- Full GUI and DuckStation gameplay regression testing.

See docs/LCK_WORKSTREAMS.md and
docs/LCK_CHECKPOINT_2026-11-08.md.