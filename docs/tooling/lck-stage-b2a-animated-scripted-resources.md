# Stage B2A — native animated and scripted resource architecture

Baseline: `41ec9d8fe7b7b3bbd3ea032eaa9bf4b1f7ce01cf`.
Branch: `lck/stage-b2a-animated-scripted-resources`.

This stage adds dependency authoring and reporting architecture. It does not implement a universal native animation/script transplant writer.

## Persistent evidence and identity

Base Forms now have optional `native_behavior` evidence. Legacy projects and master snapshots without this field still deserialize. Existing asset categories, FormKeys, Reference keys, static resource closure, collision ownership and native export allocation remain separate.

Typed relationships cover MAN records, ANM resources, actor bindings, motion streams, model selection, triggers, dialogue, conditions and cross-script contexts. Each edge retains a logical decoded source, exact carrier SHA256, byte offset/length and native consumer. Optional FormKey targets participate in database dependency validation and use indexes. Missing or malformed evidence cannot establish a classification; missing FormKey targets fail the existing transactional validation.

Native NPC scene migration inventories the original donor MAN partition-1 record. The partition-relative actor index is kept separate from flattened script context IDs and model-bank selectors. The native placement header provides the initial scene-bank ANM selector (`anim_id - 1`), resolved through the existing scene ANM resolver. ANM evidence identifies the source PROT entry, exact decoded bundle hash and record span. Special/global actor banks remain unresolved. Initial animation binding does not establish compatible bones, future script-selected clips, equipment, textures or portable playback.

Field instruction observations retain dialogue/text/picker sites, BBOX trigger tests, flag/conditional sites, spawned contexts and cross-context references. MAN motion-table bindings use the native actor selector and variant flag rules. These are source observations, not proof that a branch executes or that a BBOX test is an independently portable trigger. The linear decoder is not a complete control-flow analyzer. Decode failures remain explicit; no scan is used to certify absence of behavior.

## Classification and copy reporting

The supplemental labels are Static, Animated Only, Scripted Only, Animated + Scripted, and Unknown Dependencies. They never filter the universal Object Window or change placeability categories. Both animation and scripting scope reviews are required before assigning one of the four known labels. Any unresolved gap yields Unknown Dependencies. MAN inventories intentionally retain unresolved closure gaps, so their known ANM/script observations do not falsely become a complete classification.

The Object Window displays the supplemental label. Selected Base Forms expose dependency evidence and separate static visual, animated and behavior-preserving copy assessments. Native project export includes both the assessment and dependency inventory in each Reference report/debug log. The existing visual writer still determines material, registration and operation success. B2A reports no fully functional copy: animation relocation and behavior relocation/independent state have not been certified. No animation or behavior operation is silently replaced by a purported successful visual operation.

Old already-catalogued master records are not silently rewritten; records without the optional inventory remain Unknown Dependencies. This stage does not add the outstanding unregistered raw-resource Object Window adapter from B1.1. All existing catalog candidates remain retained.

## Boundaries preserved

No new Latest Build installed; no LCK or DuckStation launched; no desktop control or VM work. No retail modifications, replacement test IMGs, budget relaxation, exporter whitelists, unrelated pixel substitution or collision ownership inference. B1.1 static resolver and all 45 unresolved material identities remain unchanged. The full static matrix is not rerun because its resolver/writer was not changed; current coverage remains historical B1.1 evidence, not new B2A gameplay evidence.

Verification results and the final Git checkpoint are recorded in the B2A local delivery report. Automated tests, manual GUI evidence and native gameplay evidence are separate. No B2A manual GUI or native gameplay verification has occurred.

## Automated verification

- Regular Studio library suite: 321 passed, 0 failed, 62 ignored by default (five new regular tests and one new opt-in census).
- Editor binary compile check: passed; no execution of the editor.
- Read-only native dependency census: passed in 264.83 seconds; 101 readable MAN scenes, 2,708 actors, 1,562 initial scene-bank ANM bindings, 76,821 observed edges and 10,033 explicit gaps. These are observations/relationships, not unique portable assets or behavior-complete copies. Unreadable/non-MAN named blocks are outside this census, and special/global bindings remain unresolved.
- Native Base Form/Reference acceptance and reordered draws: passed.
- Exact legacy scene migration/idempotence/project roundtrip: passed.

The initial restricted runs retained 13 Windows access-denied failures in existing atomic-save/path tests. The normal-file-access rerun passed all 321 regular tests; the failed runs remain as diagnostic evidence. No functionality was weakened to pass them. All headless test logs are retained under the workspace's `b2a-*.log` paths.

Manual GUI verification: none for B2A. Native gameplay verification: none for B2A. Historical user reports remain historical and case-specific. No sandbox gameplay evidence.

## Reproduce without launching applications

```powershell
cargo test -p legaia-studio --lib --locked
cargo check -p legaia-studio --bin legaia-studio --locked
$env:LCK_AUDIT_SOURCE='D:\Emulators\Games\Legend of Legaia\LegendOfLegaia.img'
cargo test -p legaia-studio --lib --locked stage_b2a_native_dependency_census -- --ignored --nocapture
```

The census reads the source and reports totals; it does not create or replace an IMG. Existing opt-in acceptance/legacy/export fixtures are those named above and in the local B2A report. Run their library harness, never the editor's main executable.

The R8 project/export/portable collision regression also passed (529.41 seconds in the current debug harness). It confirmed byte-equivalent output from the existing/project writers, exact project save/reopen collision witnesses, relocated-source output equality and rejection of stale collision evidence. Retained fixture output: `D:\AstraRimElmProject\ManualTests\lck-stage-a4-lck-0c2a6efc9c799eee00a9e3a361f14a7ab6be2a81363e97d837c75f26abcf987b`.

Final protected-file SHA256 values match the B1.1 handoff: retail IMG `750dbcbadb70362feb3468467bbe7efbf3d151a414caa428c66c368d4b0b73e6`; Latest Build `c3ef8524f780bd720b92e567fca774ad0c7bb68ffad9843fe63ec1d6c55f453c`.
