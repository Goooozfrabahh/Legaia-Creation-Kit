# M2 native export development

## Preserved acceptance baseline

On October 27, 2026, the user reported all corrected M1.1 manual GUI tests
passed: viewport/hierarchy selection parity, group movement, rotation, scaling,
duplication, deletion, undo/redo and persistence. The tested release checkpoint
is `7dc88047c43127e9fa2c3a474f097e3bd459b0e2`, executable SHA-256
`ce3dc080a0a7427310176be1bf0c7be33af4e569451bf2b596ea040392612b32`.
This is user-reported manual GUI evidence. Native gameplay export acceptance
has not been reported for M2.

## Exact immediate reproduction

Input image:
`D:\AstraRimElmProject\Backups\LCK_Outdoor_Visual_20261027\LCK_EXPERIMENTAL_VISUAL_ONLY_town01_outdoor_foreign_props_20261027.img`.

User export log:
`D:\AstraRimElmProject\legaiasource\legend-of-legaia-re-main\export integrationV1_logs\town01_1793142914866312400.txt`.

The log contains exactly six original-object translations, no rotation, scale,
copies or deletions:

```text
LCK_SCENE_V1	town01
MOVE	terrain	36	-320	0	64
MOVE	terrain	37	-320	0	64
MOVE	terrain	43	-320	0	64
MOVE	placement	12	-320	0	64
MOVE	placement	13	-320	0	64
MOVE	placement	14	-320	0	64
```

The initial category inventory routes the three terrain draws to Unsupported;
the placement writer has additional binding, wall-ownership and anchor checks.
Removing the category error alone does not satisfy this reproduction.

## Native representation and implemented writer

`TerrainTile` here is a non-placed MAP decoration, not an independently movable
walkable floor tile. The field decoration emitter consumes a grid reference
and its 32-byte descriptor. X is additive, Z subtractive, and PSX Y has the
opposite sign to editor Y. Its floor height follows the mean of four unchanged
floor tiers. Ground rendering separately consumes the grid's visibility and
the descriptor's terrain texture fields; walkability additionally uses the
collision grid and kind-2 elevation records.

The new `NativeTerrain` adapter accepts native decoration translations through
the normal inventory, preparation, MAP planning, sector patching and readback
pipeline. Shared descriptors detach into a completely blank, unreferenced slot;
only the selected reference changes. It preserves selector, rotation, texture,
CLUT, flags, cull radius, floor tiers, walls and trigger bytes. It checks integer
coordinates, signed offsets, native category, authored cull reach, bindings,
triggers and unresolved walls. It does not claim floor/collision ownership.
Terrain copies, deletion, arbitrary rotation/scale and foreign terrain packing
remain unsupported. Terrain edits apply to the selected scene only: matching
coordinates in other story states do not establish shared identity.

The collision diagnostic now uses `(terrain, index)` correctly. Previously it
looked up placement index 36/37/43 for terrain draws and could fall back to a
whole-map wall count. Bounds remain diagnostic areas, never ownership claims.
Native preflight now reports all observed moving-object dependency blockers
together rather than revealing them one export attempt at a time.

## Exact native findings and remaining acceptance blockers

| Editor object | Grid cell | Descriptor | TMD selector | References | Nearby wall bits |
|---|---|---|---|---|---|
| terrain 36 | 37,18 | 82 | 70 | 1 | 30 |
| terrain 37 | 41,18 | 180 | 6 | 1 | 16 |
| terrain 43 | 41,22 | 162 | 10 | 2 | 8 |
| placement 12 | 31,20 | 509 | 114 | 1 | 0 |
| placement 13 | 32,20 | 463 | 115 | 1 | 24 |
| placement 14 | 38,25 | 137 | 36 | 1 | 43 |

Counts are diagnostic and can overlap; they cannot be summed into an owned
collision stencil. Existing insertion provenance proves placement 13 did not
contribute its nearby baseline walls, so its established continuation exemption
remains valid. No equivalent ownership receipt exists for the original objects.

Terrain 36's cell contains grid flag `0x0800` and a kind-2 floor override at MAP
trigger-block byte `0x2aa`: `[37,18,02,cc]`. This is genuine physics data. Its
coarse step is -64 native Y units, with packed -16-unit sub-cell steps. Moving
only the visible mesh would leave this stair surface at its old location.
Moving the entire grid word would also move or overwrite unrelated ground,
texture, visibility and collision data. Neither is a valid solution.

Placement 14 is Vahn's animated house, not another imported static prop. It has
kind-1 binding at anchor `(37,24)`, MAP trigger byte `0xcc`, flat MAN record 0,
gate 0, animation 1 and a two-object/166-vertex TMD. The bounded P0[0] script
does not paint walls, but references into its context originate in P0[35],
P2[3], P2[6] and P2[15]. A complete linear scan is not proof of control flow,
helper effects or spatial script closure.

The static MAP wall bitfield does not encode an object-owner ID. The missing
input is an evidence-backed contribution map distinguishing each object's
exclusive walls/floors from retained terrain and shared contributions, plus
closure of the house's script/interaction dependencies. Inferring that map
from mesh bounds would violate the required ownership rule. Supporting the
half-tile `(-320,+64)` floor move also needs sub-cell elevation redistribution
and destination overlap/packing checks, not simply changing trigger tile X/Z.
If such surfaces cannot be represented in the existing four packed two-bit
steps per tile, additional floor-sampler/scene format engine work is needed.

The exact-source automated regression proves all six requests survive inventory
and preparation and all three terrain translations reparse correctly at the
descriptor layer, including the retained shared reference. It then proves
production planning rejects the unresolved physics/script dependencies. It is
NOT a passing six-request export acceptance test, a valid exported retail IMG,
an LCK GUI reopening test or a gameplay verification.

## Verification and release policy

Automated tests cover descriptor detachment, stable resource fields, integer
and cull constraints, native category checks, descriptor exhaustion, wall and
elevation rejection, and a complete synthetic raw IMG export/reparse with
unchanged source, unrelated entry and overwrite protection. Synthetic artifacts
under `target/m2-terrain-synthetic-*` are test fixtures, not game-ready images.

Logs under the worktree's `target` directory:

- `m2-all-targets.log`: complete automated studio regressions.
- `m2-six-request-regression.log`: exact-source dependency rejection and descriptor readback.
- `m2-six-request-inventory.log`: detailed object and script inventory.
- `m2-m11-native-continuation.log`: existing genuine two-insertion move/delete/mixed regression.
- `m2-release.log`: release candidate compilation.

Until the full six-request acceptance succeeds, retain the manually verified
M1.1 executable at the Latest Build target. Preserve the compiled M2 candidate
separately; do not label it Universal Native Export or replace the user's known
working release with an incomplete milestone. No M2 GUI or gameplay check has
been performed. No host LCK/DuckStation or desktop-control tool was launched.

Final automated suite: 239 library tests and 9 integration tests passed; 18
explicit external/controlled tests were ignored by the default run. The genuine
M1.1 native continuation test was run explicitly and passed all three cases
(move two, delete two, move one/delete one). The exact-source six-request test
is a successful regression of rejection, not successful export acceptance.
Existing compiler warnings remain (85 in the release build).
