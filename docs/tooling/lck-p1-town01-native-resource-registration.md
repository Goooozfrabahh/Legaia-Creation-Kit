# Town01 native resource registration — Run 1

Baseline: `432cb3df`, retaining all pre-existing uncommitted texture, multi-TIM,
registration and index-200 protection work. No executable patch or registry
expansion is involved.

## Implemented

`scene_registration::register_scene` selects the native route from the
destination descriptor structure, without a town or donor whitelist. The plain
route remains available. The new streamed route supports a descriptor bundle
with an existing TMD pack and Flag(0x14), paired with a texture-only PAC.

- Preserve opaque Flag payload, declared size and nonzero padding; do not try
  to LZS-decode a loader-mode descriptor.
- Preserve existing model selectors, deduplicate exact immutable model bytes,
  append new models, and retain source fingerprints and model-count diagnostics.
- Replay PAC TIM/TIM_LIST chunks in native order. Append required generated
  texture uploads after all original uploads; preserve original chunk bytes,
  terminator and nonzero trailer data. Consume only verified zero padding where
  necessary to fit the existing footprint.
- Reject PAC model/script contributors: inserting bundle models would shift
  their selectors without a complete global mapping. Reject malformed chunks,
  missing terminators, unsupported modes, active CLUT walkers and unsafe budgets.
- Reparse bundle payloads and model lookup; compare final texture replay with
  the relocation plan. Preserve the >=200 global-index rejection and stricter
  caller limits. Neither the five-resident prefix nor fixture budgets prove a
  complete scene-wide runtime contract.

`NativeExportPlan::compile_scene_resources` is now a non-test-only resource-byte
compiler. It resolves CDNAME-relative MAP+3 bundle and MAP+4 PAC entries, uses
the existing native sector writer on a private image, verifies all PROT entries,
and reparses model lookup from actual sector-readback bytes. Failure leaves the
input image unchanged. It returns in-memory resource results, not an accepted
writable NativeExportPlan. **The GUI's `plan_export` still rejects unresolved
foreign resources and authored collision. This is not end-to-end playable import.**

No placement, naming, camera, GUI, collision-authoring or scene-persistence
behavior was changed. Independent MAP allocation remains the existing subsystem;
it is not yet combined with these packages in an accepted foreign-object plan.

## Evidence

- Studio build: passed (existing compiler warnings remain).
- Studio library suite: **193 passed, 0 failed, 14 ignored** with normal filesystem
  access. The initial sandbox run had eight existing metadata atomic-write test
  failures; the normal-access rerun passed them.
- Three new standard regressions cover streamed registration/reuse, malformed
  or unresolved PAC contributors and budgets, and deterministic transactional
  native sector compilation with unchanged neighboring entries and growth refusal.
- Opt-in retail test `real_town01_pac_packaging_and_native_donor_gates`: passed.
  It reads the original disc only; no output image is saved.
- Actual town01 bundle: 227328 -> 224298 logical bytes, padded back to the original
  footprint. PAC remains 368640 bytes. A **synthetic** donor package registers at
  scene selector 114 / global index 119; global occupancy 119 -> 120. Native model
  lookup and every unrelated PROT entry pass readback. This proves serialization,
  not retail donor behavior, memory headroom or runtime spawn coverage.
- Real donor dependency replay correctly refuses Biron (`bylon`) flags
  `0x00010008` and Hunter's Spring (`izumi`) flags `0x80000008` in
  `multi_tim::UploadTrace::verify_replay` -> `legaia_tim::parse_strict`.
  Those authentic bytes were not normalized to force success. An initial direct
  donor acceptance attempt failed at that gate; the final regression explicitly
  checks the refusal rather than claiming a successful import.

## Exact remaining gates / Run 2

1. **Native TIM upload contract:** add a separately verified native-source path
   for retail metadata-bearing TIMs, preserving source identity and validating
   uploader semantics and block extents. The strict generated-TIM path must stay
   strict. Do not simply mask reserved flags. Complete source/destination upload
   and dynamic VRAM-write closure is still required.
2. **Archive growth:** an attempted retail in-memory call to the existing
   `DiscPatcher::grow_prot_entries` overflowed at `disc.rs:260`, adding growth to
   a word in its TOC-tail loop. That loop runs through the header-sector tail;
   the valid tail extent/non-offset words need independent validation. This run
   does not change that writer. The new compiler now rejects footprint growth
   before calling it. Fitting packages use the existing in-place writer.
3. **Runtime/behavior acceptance:** establish destination-wide RAM, reserved and
   dynamic VRAM usage, original-object physics, walkability and script contracts.
   Mesh proposals do not establish native ownership. Keep the current collision
   and dependency gates, including the model-index guard.
4. **Final export assembly:** once those contracts are established, connect the
   verified package and independent MAP allocations to the same deterministic
   `plan_export` result used by preflight and output. Resolve each destination
   story-state's resources independently; do not reuse a selector blindly across
   Rim Elm variants. Terrain/floor, actors/animation and interactive script
   writers remain separate required category adapters.

This increment does not enable new foreign-resource playable IMG support in any
category. Existing supported native exports retain their original gates.
Resource-level serialization now supports compatible plain and streamed
texture-PAC destinations; there is no permission to omit category behavior.
**No IMG was generated.**

## Windows CMD reproduction

```bat
cd /d D:\AstraRimElmProject\legaiasource\legend-of-legaia-re-main
cargo build -p legaia-studio --locked
cargo test -p legaia-studio --lib --locked
set "LCK_P0_SOURCE=D:\Emulators\Games\Legend of Legaia\LegendOfLegaia.img"
cargo test -p legaia-studio --lib --locked real_town01_pac_packaging_and_native_donor_gates -- --ignored --nocapture
```

There is no safe foreign-IMG export command for this increment. Use the existing
Studio executable for existing supported exports; these tests never call its
output writer. No original image, previous test image or backup is overwritten;
no emulator, commit or push is involved.
