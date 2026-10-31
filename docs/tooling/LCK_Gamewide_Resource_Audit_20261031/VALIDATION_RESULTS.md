# Automated validation evidence — Stage B audit

No editor, renderer, DuckStation, physical input or VM was launched. No new GUI or gameplay evidence was produced.

- Final regular Studio unit suite: **310 passed, 0 failed, 59 ignored by default**. Includes new audit leaf/opaque-range regression and stable source-identity escaping test.
- Exact five-object fixture (jiji terrain1 three instances, bubu1 terrain22, conc2 terrain107): **passed**. All 109,860 donor material samples unchanged; 5/5 independent native references and owned vertical-wall policies; source/resident protection; repeated move/copy/delete/reopen readback. jiji horizontal faces still do not become new walkable floors.
- Existing Bylon/Izumi native resource/material/registration test: **passed**.
- Existing preserved R8 repeated move/rotate/delete/export/reopen and multi-cell ownership fixture: **passed**.
- Stage A Base Form/Reference and reordered draw acceptance: **passed**.
- Exact user legacy scene migration/project roundtrip: **passed**.
- Record-workspace group duplication/undo/redo and collision associations: **passed** (headless egui simulation, not GUI verification).

The first restricted-filesystem unit run had 13 path-resolution/publication permission errors; native-path rerun passed all tests. The first five-object attempt hit the same path-resolution error before publication; its new-directory rerun passed. A subsequent intermediate link attempt encountered a running headless test executable; it made no editor/source changes. These logs remain preserved, rather than being reported as successful runs.

The original unoptimized census was intentionally stopped after 20 cell reports, with partial outputs retained. An initial completed optimized census was retained separately while two audit-only issues were corrected: native leaf data must not be reinterpreted as stream headers, and registration probes must use the destination's actual ordered VRAM baseline. Consult the final census reports, not those intermediate folders.

## Exact regression outputs

New files are under `D:\AstraRimElmProject\ManualTests\StageB_FiveObject_Regression_20261031_Approved`. Their IMG hashes equal the preserved Stage A repair artifacts:

- `TEST12345_FIVE_TERRAIN_REPAIRED.img`: `5dde582a3c01076a990bed89a332e55750579c4bccd964a1888e2e76cfb5c1ff`
- `TEST12345_REFERENCE_MOVE_COPY_DELETE.img`: `327040b1e81a012269487c450622c13c0fa2d324ef177fc05ca6ad0bf3905065`
- `TEST12345_REOPEN_UNCHANGED.img`: `327040b1e81a012269487c450622c13c0fa2d324ef177fc05ca6ad0bf3905065`

R8 output directory: `D:\AstraRimElmProject\ManualTests\StageB_R8_Regression_20261031`.

## Verification boundaries

Native TIM/TMD package encoding/readback is tested across the model census. Bounded source-family registration uses actual destination VRAM plus boot/character reservations and compares the registered model bytes using the independent static native-loader contract. Neither equals executing the retail engine. Additional raw TMD identities without established native registration remain explicitly unknown in the matrix. There is no all-pairs cell/donor or all-behavior certification.

Historical user-reported Bylon rendering/collision and R8 owned-collision lifecycle successes remain historical, object-specific gameplay evidence. They are not propagated to every asset. This audit adds zero gameplay-verified objects.

Latest Build remains `4476a72`, SHA256 `c3ef8524f780bd720b92e567fca774ad0c7bb68ffad9843fe63ec1d6c55f453c`. Protected original disc SHA256 `750dbcbadb70362feb3468467bbe7efbf3d151a414caa428c66c368d4b0b73e6`.

Final supplemental per-scene native request census: 124 blocks attempted; 90 static profiles, 34 explicit unresolved contexts; source hash unchanged.
