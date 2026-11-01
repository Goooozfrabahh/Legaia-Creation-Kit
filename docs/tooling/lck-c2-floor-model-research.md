# LCK Stage C2 - Original PS1 Floor Asset Research

## Baseline
Parent checkpoint: 731469341052a5a1c51e0c1a014e9db7e3029190
Research only. No engine, editor, or IMG modifications.

## Native MAP ground inventory
- 5,299 scene-specific flat tile texture references.
- 79 non-world-map source scenes.
- 177,756 existing flat textured ground cell placements.
- 229 original texture previews previously decoded.
- Flatness checked by matching four native corner-height tiers.
- Native kind-2 staircase overrides excluded.

## Native TMD model-floor inventory
- 5,713 horizontal textured square polygons found.
- 2,247 measured exactly 128 x 128 native units.
- 81 source scenes contain candidates.
- 2,887 source/texture/UV signatures were identified.

These are geometry/texture candidates, not 5,713 verified
unique textures or gameplay-tested walkable floor objects.

## Selected original source scenes

| Cell | Location | Square TMD polygons | Exact 128x128 |
|------|----------|--------------------:|-------------:|
| geremi | Jeremi | 300 | 235 |
| rayman | Octam Underground | 256 | 165 |
| bylon | Biron Monastery | 108 | 68 |
| doman | Usha Research Center | 156 | 26 |
| retockin | Ratayu Interior | 35 | 18 |
| koin1 | Sol Tower Rooms A | 169 | 13 |
| koin2 | Sol Tower Rooms B | 80 | 45 |
| koin3 | Sol Tower Dance Floor | 65 | 18 |

## Native asset identities
Track each candidate's:
- Original source scene and model registration.
- TMD object and primitive identity.
- TMD geometry fingerprint.
- Original UV coordinates.
- TPAGE and CLUT.
- Original TIM/VRAM resource dependencies.
- Measured geometry dimensions and orientation.

## Existing source implementations
- crates/scene-formats/src/field_objects.rs
- crates/tmd/src/legaia_prims.rs
- crates/tmd/src/descriptor.rs
- crates/tmd/src/mesh/vram.rs
- crates/studio/src/app/native_floor_patch.rs
- crates/studio/src/app/native_elevation_codec.rs
- crates/studio/src/app/native_extended_floor.rs
- crates/studio/src/app/native_wall_journal.rs

## Research still required
1. Confirm upward-facing polygons rather than ceilings.
2. Verify complete and correctly oriented original texture pixels.
3. Remove duplicate texture/material identities.
4. Identify square polygons with repeatable tiling.
5. Determine portable TIM/CLUT/VRAM dependencies per donor.
6. Link selected floor geometry to native walkable height records.
7. Test resulting floor tiles in user-run DuckStation.

## Planned Stage C2
Create native, independently editable floor References carrying
authentic original-game textures, geometry provenance, native
walkable height data and independent ownership.

Start with flat indoor floors. Then extend to slopes, stairs,
other-cell imports and extended native floor sampling.

## Preservation policy
Keep C1 unchanged and available for rollback.
Do not merge C2 without user approval.
Do not launch DuckStation automatically.
Do not implement code as part of this research checkpoint.
