# Stage B1.1 resource closure investigation

Initial package coverage: **8501/8546**. Final: **8501/8546**. Baseline regressions: **0**.
All 45 source identities retain exact TMD hash, page/palette references, ordered carriers and missing-word spans in ALL_45_DEPENDENCY_FINDINGS.json.

## Missing static inputs

There are 19 groups by first missing address; this groups a shared symptom, not proof of one runtime cause.
All 5,410 distinct missing word coordinates have coordinate candidates elsewhere among the retail TIM sources searched. None has a witness in its own donor scene's inventoried static TIMs. An unrelated scene's bytes at the same address are not the requested texture. No such substitution was made.
The search examined 3290 unique TIM-tag/scanner payload hashes. 519 candidates failed native TIM extraction/validation (including false typed-stream interpretations); these are retained explicitly. Opaque resources and runtime-produced data are not exhausted by a TIM search.
Known framebuffer/capture/clear addresses, CLUT-walk MoveImage behavior and ideal UV support identify follow-up domains. They do not establish the producer or the intended words for these models. Every unresolved model has some missing words intersecting ideal UV support; dropping bounding-box corners alone does not prove complete material closure.
No supported evidence justifies changing these 45 material packages yet. This is not a proven PS1 hardware limitation.

## Raw payload investigation

All 360 original payload hashes and 680 source occurrences were recovered from their exact original PROT, descriptor, LZS, pack or stream coordinates and hash-verified.
Final structural counts: **{'ModelPack': 103, 'Unresolved': 80, 'BareModel': 177}**. See RAW_360_CLASSIFIED.json for every source, member, geometry extent and provisional Base Form classification.
The old audit counted type-2 dispatch payloads as TMD-family entries even when they were containers or lacked valid geometry. The generic model-payload classifier now records bare model versus complete model pack versus unresolved tag. Whole-pack hashes and padding differences are preserved, not treated as new semantic object identities.
All 103 model packs have every geometry member matched to established scene registrations after separately retaining stored-member padding and geometry extents. These whole-pack hashes do not add new bare models. Native audio/ANM consumer findings for the other 80 tags: {'Component of native VAB-prefixed audio carrier; not validated TMD': 78, 'Native ANM-format animation data': 1, 'Native consumer unresolved; not validated TMD': 1}.
Bare-model families include battle-stage shells, summon components, effect geometry and field character meshes. Battle shells require the native object mask and second draw transform; character/effect/summon geometry requires its appropriate animation, texture and actor/draw dependencies. Native type 9 and type 2 already provide bare/pack registration formats; this is not full cross-cell registration or gameplay verification.
All candidates remain in the universal investigation inventory. Unknown candidates are not removed or offered as falsely proven independent placeable objects. The current Object Window still needs a general unregistered-resource adapter to expose these source identities; this change does not claim that adapter is complete.

## Changes and Stage B2

Implemented reusable payload classification, geometry-extent versus stored-payload evidence, source readback and cross-disc missing-word investigation. No unsupported pixel repair, sampler bypass or new asset restriction was added. Unrestricted experimental export is preserved.
Stage B2 must bind animation/VDF, native actor/draw class, palette lifetime, scripts and resource budgets to each Base Form separately from static geometry. Static detachment cannot certify behavior or freeze an unknown runtime texture producer.

## Verification boundaries

These are automated structural/package tests. Original-engine rendering, visibility, collision behavior and gameplay compatibility have not been tested in this run. Latest Build and protected IMGs are not installation targets. Regression results and GitHub refs are recorded separately in the delivery manifest.
The encoded fixed asset arena remains 404,480 bytes; package closure does not establish decoded heap capacity, staging or actor/GPU allocations. The complete eight-TIM resident-character source was checked separately and supplies none of the missing words. Character and alternate-face protections remain unchanged.

## Final regression and checkpoint record

- 316 regular Studio tests passed; 61 opt-in diagnostics are ignored by default.
- All 22 prior Stage B1 regression fixtures passed again, including Stage A Base
  Form/Reference routing, Bylon textures, native uploader simulation, R8 collision
  lifecycle, the five-object IMG fixtures and the exact six-request dependency graph.
- The two new source/consumer investigations, full 8,546-model matrix, per-scene
  memory profiles and report integrity verification passed. The verified report
  has 249,344 identity rows, 302,633 graph edges and zero dangling edges.
- Editor compile check passed without launching it. Latest Build SHA256 remains
  `c3ef8524f780bd720b92e567fca774ad0c7bb68ffad9843fe63ec1d6c55f453c`.
- Native TIM suffix regression initially lacked its required source environment
  variable; its original log is retained and the configured rerun passed. Audit
  rendering required the separate memory-profile pass; final report integrity passed.

Before any development, GitHub branch `lck/stage-b1-static-resource-resolution`
and annotated tag `lck-stage-b1-audit-3ab839b` were pushed and independently read
back. Both resolve to `3ab839b5ae72a7119e24eb94978984e766fc52bf`; the annotated
tag object is `980ce51129ceef96e3ac1bde52dece02d2aa6df4`. Its message explicitly
identifies automated verification and excludes gameplay certification. No production
merge, rebase, force push or history rewrite occurred.

Implementation branch: `lck/stage-b1-1-static-resource-closure`, created directly
from `3ab839b`. The final implementation SHA and remote verification are recorded
in the local results' `DELIVERY.json` after committing. The underlying census still
contains an inherited legacy checkpoint label; use this report's baseline and the
delivery manifest for actual implementation provenance.

Complete local reports: `D:\AstraRimElmProject\ManualTests\LCK_StageB1_1_20261031_Results`.
Source readback: adjacent `LCK_StageB1_1_20261031_Investigation_Final`; matrix:
`LCK_StageB1_1_20261031_Matrix`; regression logs:
`LCK_StageB1_1_20261031_Regression\FINAL_REGRESSION_RESULTS.json`.
Compact complete source-identity indexes are committed in `stage-b1-1-evidence`;
the large dependency graphs, test IMGs and extracted data remain local.
