# Outdoor foreign-object visibility experiment

2026-10-27. Test-fixture positioning only; production export protections unchanged.

The user reports that the preceding experimental IMG boots with 121 models instead of 119, but neither import was visible. This proves additional registration, not successful actor creation or on-screen rendering. The previous world positions around `(12096,10048)` were far from the outdoor village and the documented Vahn-house placement `(4864,-192,3208)`. Their displaced spawn anchors passed the existing reach gate, which is not a complete visibility proof. Inaccessible physical placement and window coverage are plausible causes. A scene/story variant mismatch remains possible; the model count alone cannot rule it out. No emulator was operated to establish the actual cause.

## New artifact and test route

`D:\AstraRimElmProject\Backups\LCK_Outdoor_Visual_20261027\LCK_EXPERIMENTAL_VISUAL_ONLY_town01_outdoor_foreign_props_20261027.img`

Size 466719120 bytes; SHA-256 `3d2f0fd87192d3e24eedd51deb6da19051d30519ece3297d1de0948ab403f82f`.

| Genuine donor | Destination descriptor / selector | Spawn cell | PSX world origin |
| --- | --- | --- | --- |
| Biron `bylon` #291 / model 10 | 509 / 114 | (31,20) | (4032,-240,2624) |
| Hunter's Spring `izumi` #36 / model 2 | 463 / 115 | (32,20) | (4160,-240,2624) |

Both tiles were zero object references and wall-free in the retail MAP. Their floor tier is 6, native ground Y -192. Origins are 48 units above that floor; the lowest preview vertex clears it by 48.75 and 48 units respectively. The fractional preview lift is rounded upward to a whole native translation; native quantization checks remain enforced. Models retain original orientation and scale. No unrelated reference is replaced and no original anchor is moved.

Cold boot this new IMG, leave Vahn's house, and move toward the village center, approximately **five native tiles west and three north of the doorstep**. The documented exit is tile `(36,23)`, world `(4672,3008)`. An all-wall-free MAP route goes west along row 23 to columns 31/32, then north to row 20. The imports are one tile apart. Screen directions depend on the camera. The location is supported as an outdoor approach near the house; the exact item-shop frontage is not asserted.

Do not restore a full save state from another image. This experiment changes the selected `town01` MAP, not all story variants. If nothing appears, provide the live scene ID, player X/Y/Z, registry count and a view of this outdoor area. Registration count alone is insufficient. These props are small; inspect near their stated tile centers and walk away/back to exercise window rebuilding.

## Structural checks and limits

The fixture asserts the same exact genuine donor identities, zero/wall-free target cells, unchanged collision proposals, exact zero-displacement spawn cells, and presence of both actors in the existing ported retail window sweep over `(29,18)..(35,24)`. Both have drawable template kind 5 and footprint anchors equal to their spawn cells. This is stronger static evidence than model registration alone, but not a retail runtime guarantee.

The existing exporter reparses native MAP/TMD/TIM/PAC resources and checks output hashes and original wall/floor preservation. Relocated textures use the prior captured-VRAM exclusions. Collision recipes stay pending in the sidecar and are not compiled or approved. RAM timing, VRAM lifetime, physical behavior and actual runtime visibility remain unproven. The index-200 conflict guard and stricter normal-export gates are unchanged.

Only `crates/studio/src/app/scene_registration_tests.rs` and this report change in this run. The fixture's previous placement remains available by omitting `LCK_VISUAL_EXPERIMENT_PLACEMENT`. The new explicit selection is:

```bat
set "LCK_VISUAL_EXPERIMENT_PLACEMENT=town01-outdoor-v1"
```

Use this with the existing source/capture/acknowledgement variables and a new output directory as documented in `lck-p1-run4-bounded-capture.md`. The two donors and positions are test cases, not importer whitelists.

## Validation result

- Studio build passed (warnings remain).
- Studio library suite: **202 passed, 0 failed, 14 ignored**.
- Explicit native fixture with outdoor output: **1 passed**, including exact-anchor/drawn-window checks, resource/placement serialization and on-disk native readback.
- Independent readback confirms both resource bundle and PAC are byte-identical to the previous experiment: this test changes placement, not the imported resources. Retail executable is unchanged; all original descriptors other than the two previously empty allocated slots, all original nonempty grid references, walls/floors and binding tail remain intact.
- Source retail and previous experimental image hashes remain unchanged. A new `.verification.json` receipt accompanies the new `.json` manifest and `.lckscene` sidecar.
- Initial fixture attempts stopped before producing an IMG: first on fractional Y translation (corrected by upward integer rounding), then on Windows access denial. The authorized filesystem-access rerun passed. No native checks were disabled.
- No staging, commit, push, retail modification or DuckStation operation. Previous uncommitted development preserved.
