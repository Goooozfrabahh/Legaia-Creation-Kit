# LCK: first retail-engine cross-cell rendering milestone

This checkpoint preserves the existing working implementation. It adds no
exporter feature or GUI behavior. Dates and validation counts below are a
historical milestone record requested by the project owner, not promises of
general game-wide native export support.

## Foundation and first native edit

Legaia Studio / LCK is a Windows desktop, GECK-inspired editor for authentic
Legend of Legaia PS1 scenes. The source supports native scene/cell browsing,
original geometry and texture rendering, scene hierarchy and selection, asset
preview, orbit/pan/zoom/Frame Selected, transform and duplicate/delete controls,
undo/redo, scene persistence, native NPC/field-actor visualization, persistent
naming and a game-wide World Objects repository. Native serialization supports
a smaller verified subset than the editor's preview operations.

**October 25, 2026:** the project owner successfully changed an original Rim Elm
environmental object's placement, exported a native PS1 IMG and observed the
changed placement in DuckStation. This established the first proven
editor-to-native-MAP-to-retail-engine pipeline.

## First foreign World Objects rendered by the retail engine

**October 27, 2026:** the project owner reports successful rendering of genuine
foreign World Objects transplanted between original PS1 cells. This is manual
retail-engine evidence supplied by the owner; the agent did not operate
DuckStation. The previous experiment also increased runtime registrations from
119 to 121. Registration alone had not proved visible actor creation; the
subsequent outdoor visual experiment isolates placement from resource changes.

The outdoor experiment uses these authentic source identities:

| Donor | Destination descriptor / selector | Spawn tile | PSX origin |
|---|---|---|---|
| Biron `bylon` #291 / model 10 | 509 / 114 | (31,20) | (4032,-240,2624) |
| Hunter's Spring `izumi` #36 / model 2 | 463 / 115 | (32,20) | (4160,-240,2624) |

Artifact basename:
`LCK_EXPERIMENTAL_VISUAL_ONLY_town01_outdoor_foreign_props_20261027.img`.
SHA-256: `3d2f0fd87192d3e24eedd51deb6da19051d30519ece3297d1de0948ab403f82f`.
Its manifest, pending-recipe scene and verification receipt remain local with
the image. No game image, TIM/TMD payload, executable, RAM/VRAM capture or save
state is included in Git.

The outdoor copies use independently allocated native records, preserve their
original donors and all unrelated placements, and share compatible immutable
resources without sharing editor identity. Neither copy swaps another object's
anchor. Original walls/floors/bindings and the retail executable are preserved.
The import resource bundle and PAC are byte-identical to the earlier visual
experiment; only the imported placements change between those experiments.

## Preserved implementation

The resource pipeline is reusable across compatible scene structures, not a
town/donor whitelist. The two named donors are regression cases, not the
definition of supported assets.

| Component | Responsibility |
|---|---|
| `native_asset_plan.rs` | Stable resource identities, provenance and dependency inventory |
| `texture_relocation.rs`, `multi_tim.rs` | Ordered TIM/CLUT dependency closure, VRAM placement and TMD references |
| `scene_registration.rs` | Plain bundles and supported streamed texture-PAC registration |
| `ps1_export.rs` | Deterministic native resource/placement candidates and strict normal export planning |
| `native_collision_import.rs` | Explicit authored additive wall compilation with original-grid preservation |
| `runtime_evidence.rs` | Conservative captured-VRAM hazard exclusions, never free-space certification |
| `experimental_native_export.rs` | Separate acknowledged, create-only visual IMG experiment |
| `disc-patch/src/disc.rs` | Transactional PROT growth with bounded TOC updates and readback |
| `tim/src/lib.rs` | Verified native-upload metadata policy, separate from strict scanning |
| `scripts/lck_runtime_capture.py` | Offline capture analysis and bounded user-operated GDB observation |

Authentic texture pixels, CLUT data, primitive attributes, UVs and native
metadata are retained through relocation and round-trip checks. Native resource
registration reparses model-selector lookup and package boundaries. Native
placement allocation preserves unrelated records and references. Archive
rebuilding validates resource readback and affected sector integrity.

The model-registration index-200 conflict guard remains enforced; indices below
200 are not certified safe capacity. Existing stricter limits still apply.
Preflight and accepted normal export continue using NativeExportPlan. The
separate experimental path never converts an unresolved candidate into an
accepted production plan or silently falls back from preflight failure.

## Evidence boundaries

- **Proven manually:** the original-object translation milestone and the
  owner's successful retail-engine rendering of foreign objects.
- **Proven by automated readback:** independent records, resource registration,
  relocated primitive/texture samples, preserved unrelated scene data and
  serialized package/image consistency.
- **Still experimental:** safe game-wide cross-cell placement, native physics,
  walkable surfaces, actor/script/animation dependencies, complete heap headroom,
  dynamic VRAM lifetime and runtime spawn/culling behavior in other contexts.
- **Still blocked normally:** unsupported foreign-resource or collision intent.
  Mesh-derived recipes remain proposals. Explicit wall authoring is additive;
  it cannot authorize deleting original walls or claim verified full behavior.

The successful visual test is a major native resource milestone, not evidence
that every repository category or destination is production-ready. Terrain,
NPC/animated and interactive imports need their own verified native writers.
General rotation/scale and MAN relocation are not enabled by this checkpoint.

The supplied runtime captures have their own provenance limitations. The saved
early arena-return dump is scene `0x02EC`, while post-load/settled identify
town01 scene 3 and 119 registrations. The owner's later town01 arena-return
observation (`v0=0x8015CBD0`, request `0x62C00`) has no corresponding dump.
Modified-image capture context and incomplete temporal evidence cannot become
production RAM/VRAM approval. See the focused reports below.

## Validation and reproduction

Checkpoint validation uses the current Studio build, the Studio/TIM/disc-patch
library suites and six mock/offline capture-tool tests. Retail-dependent tests
remain opt-in; they do not run in source-only CI. The outdoor native fixture
passed separately when creating the recorded image, including exact anchors,
drawable ported-window actors, preserved pending collision and image readback.
The milestone commit message records the final test totals and skips.

```bat
cargo build -p legaia-studio --locked
cargo test -p legaia-studio -p legaia-tim -p legaia-disc-patch --lib --locked
python -m unittest discover -s scripts -p test_lck_runtime_capture.py -v
```

Where Windows sandbox permissions prevent persistence tests using the default
TEMP directory, point TEMP/TMP to a writable workspace `target` subdirectory;
do not change the implementation to bypass atomic-write tests.

Focused implementation/evidence reports retained in this checkpoint:

- [Single-TIM relocation](lck-p1-native-texture-relocation.md)
- [Multi-TIM dependencies](lck-p1-multi-tim-integration.md)
- [Plain scene registration](lck-p1-plain-scene-registration.md)
- [Native resource registration](lck-p1-town01-native-resource-registration.md)
- [Native import integration](lck-p1-native-import-run2.md)
- [Additive collision](lck-p1-native-collision-run3.md)
- [Runtime capture requirements](lck-p1-run3-runtime-capture.md)
- [Captured evidence and experimental export](lck-p1-run4-runtime-export.md)
- [Bounded capture workflow](lck-p1-run4-bounded-capture.md)
- [Outdoor visibility fixture](lck-p1-outdoor-visual-verification.md)

Earlier reports describe the gates at their own stage; this milestone and the
latest focused reports distinguish subsequently proven rendering from remaining
behavioral and production-safety gaps.

## Exact checkpoint scope

The checkpoint contains these 30 files. Existing local aliases, historical
backup directories, experiment logs, generated outputs and the two stray
root-level shell artifacts are excluded and preserved on disk.

- `.gitignore`
- `README.md`
- `crates/disc-patch/src/disc.rs`
- `crates/studio/README.md`
- `crates/studio/src/app/collision_recipes.rs`
- `crates/studio/src/app/native_asset_plan.rs`
- `crates/studio/src/app/ps1_export.rs`
- `crates/tim/src/lib.rs`
- `crates/studio/src/app/experimental_native_export.rs`
- `crates/studio/src/app/multi_tim.rs`
- `crates/studio/src/app/multi_tim_tests.rs`
- `crates/studio/src/app/native_collision_import.rs`
- `crates/studio/src/app/runtime_evidence.rs`
- `crates/studio/src/app/scene_registration.rs`
- `crates/studio/src/app/scene_registration_tests.rs`
- `crates/studio/src/app/texture_relocation.rs`
- `crates/studio/src/app/texture_relocation_tests.rs`
- `docs/tooling/lck-native-cross-cell-milestone.md`
- `docs/tooling/lck-p1-multi-tim-integration.md`
- `docs/tooling/lck-p1-native-collision-run3.md`
- `docs/tooling/lck-p1-native-import-run2.md`
- `docs/tooling/lck-p1-native-texture-relocation.md`
- `docs/tooling/lck-p1-outdoor-visual-verification.md`
- `docs/tooling/lck-p1-plain-scene-registration.md`
- `docs/tooling/lck-p1-run3-runtime-capture.md`
- `docs/tooling/lck-p1-run4-bounded-capture.md`
- `docs/tooling/lck-p1-run4-runtime-export.md`
- `docs/tooling/lck-p1-town01-native-resource-registration.md`
- `scripts/lck_runtime_capture.py`
- `scripts/test_lck_runtime_capture.py`
