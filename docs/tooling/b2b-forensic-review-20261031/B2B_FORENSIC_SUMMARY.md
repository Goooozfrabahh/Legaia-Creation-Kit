# B2B forensic review â€” A/B/C load; M freezes

This package contains source/evidence synchronization only. No exporter/runtime fix,
replacement IMG, GUI/emulator operation, production merge or Latest Build installation.
User confirms all combined-resource variants freeze, including M with no imported actors.
That rules out imported actor instantiation as a necessary trigger, not all existing actor or renderer activity.

Source checkpoint: `e59873c30dccc1d3d84be9a2f5b4ae76a6640799`.
History: `41ec9d8` -> `a29b784` -> `34bb350` -> `e5eefff` -> `e59873c`.
Full resource identities/decoded offsets/hashes are in the TSV and allocation JSON.

| Case | User result | Scene/global models | ANM clips | P1 actors | Bundle/PAC bytes | Persistent requests | Peak + fixed arena |
|---|---|---|---|---|---|---|---|
| A | LOADS | 114/119 | 69 | 53 | 227328/368640 | 458468 | 867064 |
| B | LOADS | 115/120 | 70 | 54 | 227328/370688 | 466516 | 875112 |
| C | LOADS | 115/120 | 70 | 54 | 229376/368640 | 469116 | 877712 |
| M | FREEZES | 116/121 | 71 | 53 | 231424/370688 | 477076 | 885672 |

The first differing native allocation in M is still the FIRST type-2 model-pack
allocation: 320284 bytes versus B310900/C313500. There is no new sixth malloc;
the five persistent dispatcher calls retain the same order. M's first additional
model registration is global slot120 (scene selector115), after Balden at119.
M retains retail MAN/P1 count53; B/C each have54. M has both added clips70/71
and both TIM packs, without either new actor record. The ANM allocation grows
inside the existing type-5 call. These are structural/instruction differences,
NOT identified runtime failure points.

M has 477076 accounted persistent request bytes, only 88 fewer than E because
its MAN is retail. M freezing materially strengthens investigation of resource
loading, cumulative registration/material handling, transition/CD and live heap
lifetimes. It does not establish which subsystem failed or an allocator failure.

The prior cumulative native simulation passed model registration, pointer fixups
and imported construction on one CPU, with substituted heap/LZS/GPU/GTE/RNG.
The saved E snapshot shows an earlier pending read, not a reproduced hang site.
No combined-resource runtime success or fixed-freeze claim exists.

## Reviewer entry points

- `crates/studio/src/app/native_animation.rs`: closure, pack merge, MAN/ANM writer.
- `crates/studio/src/app/scene_registration.rs`: native resource registration/lifetimes.
- `crates/studio/src/app/native_loader_trace_tests.rs`: original loader/registrar simulation.
- `crates/studio/src/app/native_animation_combined_tests.rs`: M/F/G/H/I construction and generation provenance.
- `crates/studio/src/app/native_load_budget.rs`: exact accounting rules and omissions.
- `scripts/replay-native-heap.rs`: original allocator on historical snapshots, not current live proof.
- `scripts/inspect-duckstation-state77.py`: offline capture decoder, explicitly v77 only.

Existing automated checks: 324 Studio tests, 89 code-hook tests, 28 opt-in regressions,
combined instruction matrix and five diagnostic decoder/sector checks passed.
Those results are preserved historical automated evidence; this packaging task
does not rerun gameplay or certify native playback. See runtime limitations.

## Additional observed runtime evidence: F log, not M

The readable DuckStation log identifies `F_REVERSE_MODELS.img`. It reports
`tmds 121`, `anm set`, `man set`, then `malloc err size 8368` at26.6251 seconds,
followed by instruction-read failures at `0x000000A0` and `0x80000080`.
This is a direct runtime allocation-error message from F, not an inference from
static totals. It does not identify the allocation caller, prove the same failed
request in M, or prove the complete causal chain to the application hang.
The full log remains local; its exact path, hash and scoped extract are in the
allocation JSON and runtime report. No fix was implemented in this task.
