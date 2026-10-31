# Stage B retail resource audit — 2026-10-31

Source: `D:\Emulators\Games\Legend of Legaia\LegendOfLegaia.img`  
SHA256: `750dbcbadb70362feb3468467bbe7efbf3d151a414caa428c66c368d4b0b73e6`; unchanged after audit. Baseline `fec6025`.

Attempted **124 distinct CDNAME blocks/variants**, **102 with recognized field MAP layouts**; **0 failed cell inventories**. A CDNAME block is not necessarily a playable cell. No variants merged by town prefix.

## Counts and denominators

| Family | Source identity rows | Distinct payload hashes within that family |
|---|---:|---:|
| ANM | 101 | 4 |
| AnimationSelector | 2708 | — |
| BundleDescriptor | 618 | — |
| DecodedAsset | 618 | 383 |
| ElevationOverride | 30162 | — |
| EventStagerRecord | 2245 | 1740 |
| FloorHeightLut | 101 | — |
| FloorTierGrid | 102 | — |
| ISO file | 45 | 45 |
| LZS section | 1730 | 516 |
| LoaderFlag | 32 | — |
| MAN | 114 | 113 |
| MES | 97 | 1 |
| MOVE | 105 | 98 |
| MOVE2 | 14 | 14 |
| ManActor | 2708 | — |
| ManRecord | 6488 | — |
| Map | 102 | 95 |
| MapDescriptor | 52224 | — |
| MapPlacement | 3607 | — |
| NativeModel | 8546 | 4937 |
| OrderedNativeUpload | 5403 | 2380 |
| PROT entry | 1233 | 979 |
| PackMember | 20805 | 6835 |
| ProtEntry | 1235 | 979 |
| RegionTrigger | 1217 | — |
| SharedRuntimeContext | 202 | — |
| TIM | 16261 | 3290 |
| TIM_LIST | 663 | 494 |
| TMD | 31404 | 5284 |
| TMD2 | 19 | 17 |
| TerrainDecoration | 27217 | — |
| TextureContext | 124 | — |
| TileTrigger | 9029 | — |
| TimUpload | 1274 | 587 |
| Unknown / Requires Investigation | 3057 | 1594 |
| UnresolvedDependency | 432 | — |
| UnresolvedModel | 176 | — |
| VDF | 104 | 54 |
| VisualPackageVerification | 5494 | — |
| WallBitfield | 102 | — |

Container/member, scan-hit and native scene identities are separate *views*: do not sum these rows to claim unique semantic assets. Exact hashes identify shared bytes only. Native models can be repeated between blocks or registration paths. Primary scene record categories and MAP definition/occurrence distinctions remain in ALL_ASSETS.tsv.

## Original archive container families

| Existing native parser classification | PROT entries |
|---|---:|
| AllZeros | 4 |
| BattleDataPack | 4 |
| BseBank | 2 |
| DataFieldStreaming | 49 |
| EfectPack | 1 |
| FieldMap | 101 |
| InitPak | 1 |
| LzsContainer | 18 |
| MipsOverlay | 22 |
| OverlayDataBlob | 25 |
| OverlayPtrTable | 42 |
| Pack | 7 |
| PochiFiller | 266 |
| SceneAssetTable | 90 |
| SceneEventScripts | 101 |
| SceneTmdStream | 182 |
| SceneV12Table | 97 |
| SceneVabStream | 218 |
| SummonReadef | 2 |
| VabMultiBank | 1 |

These counts retain character, battle, audio, effects and other non-field carriers even when no clip/frame/item-level semantic decoder is established. A recognized outer container does not prove its inner behavior closure.

## Capability measurements

- Additional raw TMD source identities without an exact named-scene bank payload hash: **680**, representing **360 additional payload hashes**. These remain registration-unknown in the compatibility matrix; byte-footprint differences can include padding, so they are not asserted to be distinct semantic models.
- Union of raw structural TMD and native scene-model payload hashes: **5297**. Content hashes do not define Base Forms.
- Native scene model registrations attempted: **8546**, including repeated source identities.
- Complete **static visual TIM/TMD package closure plus encoding/readback**: **5597/8546 (65.49%)**. This is a bounded individual-model test into an empty texture context with field runtime regions reserved, **not** arbitrary destination-cell fit.
- Native TIM/TMD package readback: **5597/8546 (65.49%)**; this is not an IMG export rate.
- Full behavior dependency closure proven: **0/8546**. Unknown animation/script/physics closure is retained, not failed assets or invalid game data.
- New census-wide IMG readback: **0/8546**. Separate exact scene fixtures are recorded in VALIDATION_RESULTS.md.
- Gameplay verification in this audit: **0**. Historical user-reported Bylon rendering/collision and R8 lifecycle evidence is retained separately and is not applied to every asset.
- Opaque/unresolved identity rows: **3665** (overlapping container views; not distinct missing files).

Positive animation/script evidence appears in SCRIPTED_ANIMATED_ASSETS.tsv. Other assets remain Unknown / Insufficient Evidence; absence of a parsed binding does not establish Static. No asset is removed from the inventory or editor on that basis.

## Behavior evidence counts

Positive record-structure evidence (not active playback proof): {'Animated + Scripted': 2708, 'Animated evidence; scripting unknown': 2708, 'Scripted evidence; animation unknown': 8838}. Unknown remains the conservative result for unresolved dimensions; absence does not establish Static, Animated Only or Scripted Only. CLUT-bearing ordered uploads: 4857 occurrences; raw palette-word images and repeated boot uploads are separately retained.

## Registration and memory

`SCENE_MEMORY_PROFILES.json` retains one native request profile or explicit unresolved reason for every named block: **90 profiles**, **34 unresolved contexts**. `audit-totals.json` includes one bounded real resource-registration probe per discovered source-route family, with independent static loader SHA comparison and native allocation-request profiles. Those are static contracts, not original-engine execution. Every model's package result is recorded separately. Empty-destination package success does not establish persistent transforms, independent reference allocation or collision in a destination.

The encoded fixed asset arena is 404,480 bytes. Decoded persistent destinations consume separate heap blocks. Neither arena accounting nor package parse proves live heap capacity, fragmentation, actor/render/effect allocations, GPU packet capacity, or runtime residency. Read per-family profiles and warnings.

## Animated & Scripted Object Window design

Retain each asset's primary category (Architecture, Terrain, NPC, Effects, etc.). Add orthogonal filters: Animated Only, Scripted Only, Animated + Scripted, Unknown Dependencies. “Only” requires evidence of absence in the other dimension; current positive evidence with unresolved other behavior remains Unknown Dependencies. Preserve visible geometry and preserve behavior are distinct future placement modes; never label static detachment a complete behavior copy. No production UI changes made.

See MISSING_CAPABILITIES.md and AUDIT_INTEGRITY.md before using these counts.
