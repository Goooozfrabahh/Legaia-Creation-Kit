# Run 4: captured evidence and experimental native export

2026-10-27. No production resource acceptance or playable-object claim.

## Evidence and provenance

Input: `D:\AstraRimElmProject\Backups\LCK_Run3_Runtime_Updated.zip`.
Offline report: `D:\AstraRimElmProject\Backups\LCK_Run4_capture_analysis_20261027_v2.json`.
Exact fixture copies: `D:\AstraRimElmProject\Backups\LCK_Run4_CaptureFixtures_20261027`.

| RAM observation | Scene | Models | Free payload bytes | Largest free block |
| --- | --- | ---: | ---: | ---: |
| Early arena-return dump | 0x02EC | 0 | 832432 | 832432 |
| Pre-load | 0x0003 | 45 | 104924 | 103892 |
| Post-load | 0x0003 | 119 | 17780 | 17780 |
| Settled | 0x0003 | 119 | 17780 | 17780 |

The early arena-return dump is **not town01 evidence**. Separately, the user observed a later return at `0x8001E294`, town01 ID 3, `v0=0x8015CBD0`, for request `0x62C00`, in debugger screenshots. No RAM dump accompanies that later observation. The post-load/settled dumps independently contain town01 ID 3 and 119 registrations.

Post-load/settled contain the arena at `0x8015CBD0`, allocation header `0x8015CBC4`, size `0x62C00`. Heap descriptor global `0x8007BB58` resolves root `0x80091800`; linked-list sentinels and block backlinks/extents validate. There are 110 allocated blocks, 1244460 allocated payload bytes and one 17780-byte free block. These are snapshots, **not allocation-time headroom or a high-water certificate**. Model payload allocations include 46236 resident bytes and 304116 field-pack bytes. All 119 model pointers remain identical post-load to settled; the resident first five also survive pre-load to post-load.

Capture save-state headers name `dupvahnshouseandsavetest.img`, whose SHA-256 differs from retail. The live town01 descriptor array and wall/floor region match that modified image; 52 grid bytes differ. Against retail there are 83 differing MAP bytes, including descriptor 509 and wall/floor changes. Thus these are valid town01 runtime observations in a **modified-image context**, not clean-retail capacity certification. The analysis report retains hashes and detailed comparisons. Do not interpret these differences as native ownership.

## VRAM observations and implemented exclusion

| Word rectangle (x,y,w,h) | Post-load nonzero | Settled nonzero |
| --- | ---: | ---: |
| (880,64,16,64) | 248/1024 | 0/1024 |
| (832,0,16,1) | 12/16 | 0/16 |

The two captures contain 408232 union activity/hazard words and 177694 changed words. Nonzero words may be stale; zeros do not establish available lifetime. New `runtime_evidence.rs` records hashes and excludes nonzero **or changed** words. Its mask supplements existing reservations, never grants production approval.

The real donor package fixture now relocates to image rectangles `(880,128,16,64)` and `(1008,64,16,64)`, palettes `(848,0,16,1)` and `(864,0,16,1)`. All avoid captured activity. This removes the two observed conflicts without claiming the remaining space is universally free.

## Actual native artifact

`D:\AstraRimElmProject\Backups\LCK_Run4_Visual_20261027\LCK_EXPERIMENTAL_VISUAL_ONLY_town01_two_foreign_props_20261027.img`

466719120 bytes; SHA-256 `b621855d954912945005ca5b6c45b70bcf1f27ebcbc01c7c4c97ec4d1913557f`.

| Donor | Destination descriptor / selector | Spawn cell | PSX world position |
| --- | --- | --- | --- |
| bylon descriptor 291, model 10 | 509 / 114 | (92,78) | (12096,0,10048) |
| izumi descriptor 36, model 2 | 463 / 115 | (95,80) | (12224,0,10048) |

This is an explicitly acknowledged **visual-only experiment**, produced through a separate create-only path, not an accepted NativeExportPlan or normal-export fallback. It uses real relocated TIM/TMD, PAC registration, independent native MAP allocation and PROT/LZS/ISO rebuilding. Existing originals, wall/floor grids and bindings remain intact. Resource readback, primitive references, MAP records, output hash and native sector integrity passed. `SCUS_942.54` is byte-identical to retail (SHA-256 `292256e2e66db42727f613406785e444254d3f699569e611f65fcf1c6d2f3482`).

Two automatically generated collision recipes remain pending, preserved in the `.lckscene` sidecar and **not compiled**. Donor record-local collision bytes are retained, but actor extents/class, walkability, script/animation behavior, spawn-window coverage and allocation/VRAM lifetime remain unverified. The JSON manifest explicitly sets production approval and playability verification false. Structural hardware ceilings in this path are not certified free RAM or registry capacity; the existing index-200 conflict guard remains active.

Manual test: cold boot the new IMG; enter the selected town01 through its loader. Do not restore full captured save states (they restore old resources). Inspect the two positions above for independent models, authentic textures, original-object preservation, and appearance after walking away/back. Record model count, allocation failures and texture changes during camera/movement transitions. Other town01 story variants are not changed. This is not a physics or normal-play/save-progression test. Open the sidecar against the original source image for recipe review, not against the changed native layout.

## Changes and validation

Run-4 source: `native_asset_plan.rs`, `ps1_export.rs`, `texture_relocation.rs`, `texture_relocation_tests.rs`, `scene_registration_tests.rs`; new `runtime_evidence.rs` and `experimental_native_export.rs` under `crates/studio/src/app/`. New tools: `scripts/lck_runtime_capture.py`, `scripts/test_lck_runtime_capture.py`. All earlier uncommitted work remains preserved. No GUI/camera changes.

- Studio build passed, with existing/dead-code warnings and an incremental-cache hard-link fallback warning.
- Studio library: **202 passed, 0 failed, 14 ignored**.
- A default-TEMP rerun initially produced 8 access-denied failures in existing persistence tests (194 passed). Redirecting TEMP/TMP into workspace `target/lck-test-temp` resolved them without source changes; the full rerun passed as above.
- Read-only retail/capture fixture plus explicitly requested visual output: **1 passed** separately.
- Python mock/offline tests: **6 passed**. No live GDB or emulator connection by the agent.
- `git diff --check` passed; nothing staged or committed.
- Retail image unchanged, SHA-256 `750dbcbadb70362feb3468467bbe7efbf3d151a414caa428c66c368d4b0b73e6`.

Production remains blocked by temporal heap/registry/VRAM proof and complete physical/dependency contracts. Next evidence should come from one clean-retail town01 load using the bounded workflow below, followed by the user's separate visual experiment. No whitelist or production exception was added.
