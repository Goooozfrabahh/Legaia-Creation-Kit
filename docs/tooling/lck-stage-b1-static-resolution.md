# Stage B1 static material resolution

Baseline: `03e23ab`. Implementation branch: `lck/stage-b1-static-resource-resolution`.
This changes source and headless tests only. Latest Build `4476a72`, existing projects,
retail images and previous experimental images are not installation targets.

## Resolver contract

`static_resources.rs` is the shared source of static dependency evidence for native
foreign imports, Base Form export diagnostics and the full scene-model census.
The optional Base Form record preserves FormKey identity and legacy project loading.
Export diagnostics resolve cloned forms; they do not rewrite user projects.
Matching payload hashes never merge Base Forms or placed References.

Evidence includes the original encoded carrier hash, stored and decoded coordinate
namespaces, native model selector/member, ordered boot/descriptor/typed-stream
uploads, original and normalized upload hashes, sampled texture depth, primitive
UVs, image and CLUT word requirements, and contributing last-writer sequences.
Flag `0x0A` uses its established native TIM-list route; count-driven descriptor banks
and typed streaming contexts follow their corresponding native formats. No source
town, object whitelist, invented palette or preview-only substitute chooses a route.

The relocation writer now distinguishes LoadImage envelope depth from primitive
sampling depth. A 16-bit upload can supply indexed texture words. CLUT upload strips
need not start on a CBA boundary; the actual sampled CBA still must be representable.
Composite views collect real last-written words from every contributing upload and
retain shared resource provenance. Reused composite views expand only across
fully observed regions. Missing words remain explicit technical encoding failures.
Purely untextured TMDs preserve exact geometry and emit a verified empty TIM pack.

Original UV sampling comparisons, native package reparse, resident character and
alternate-face protections, runtime VRAM reservations and transactional allocation
remain active. Unrestricted experimental export remains enabled; its technical
writer failures must still be reported rather than fabricated away.

## Complete matrix results

Every pass used the same 8,546 registered scene-model identities and unchanged
retail SHA256 `750dbcbadb70362feb3468467bbe7efbf3d151a414caa428c66c368d4b0b73e6`.

| Pass | Verified native TIM/TMD packages | Coverage |
|---|---:|---:|
| Audit baseline | 5,597 / 8,546 | 65.49% |
| Untextured geometry | 6,759 / 8,546 | 79.09% |
| Carrier contexts, intermediate | 7,658 / 8,546 | 89.61% |
| Word closure and complete carrier routing | 8,501 / 8,546 | 99.47% |

Final baseline regressions: **zero**. Original failed cohorts now passing:
carrier contexts 1,343/1,344; untextured 1,162/1,162; mixed sampling depth 234/234;
palette alignment 107/107; composite ownership 58/91. The 11 originally missing-word
cases remain unresolved. The intermediate carrier pass was not a release candidate:
some contexts required the subsequently completed flag-0x0A route.

All 45 remaining failures identify unobserved source words: 33 exposed after
composite ownership resolution, 11 previously missing, one newly decoded carrier
context needing CLUT `(0,484)` words. This is an unresolved dependency or sampling
proof gap, not a demonstrated original-engine limitation. Conservative UV bounding
boxes can include cells outside ideal triangle support; real PS1 raster-edge and
texture-window behavior must be established before narrowing requirements. Some
addresses overlap display/capture/clear reservations, but overlap alone does not
prove the runtime producer. No missing input is filled with black or discarded.

The separate 360 raw payload hashes (680 source discovery identities) without an
established scene-bank registration remain in the original inventory and future
Object Window investigation. They are not included in, hidden by, or promoted into
the 8,546 registered-identity coverage figure.

## Reproduction and evidence

Run the existing `scripts/audit-gamewide-resources.ps1` with the retail source and
a **new** output directory. It runs a headless Rust harness, never the application
or emulator. Retain partial directories and logs on failure.

`scripts/report-stage-b1-resolution.py --baseline BASELINE --output NEW_REPORT PHASE1 PHASE2 PHASE3`
compares complete identity sets and independently reconstructs missing-word coverage
from serialized ordered upload rectangles. Its per-failure JSON/TSV contains every
model hash, registration, primitive footprint, missing-word span and upload chain;
ideal UV support and runtime-address overlap are explicitly non-runtime evidence.

Local complete evidence: `D:\AstraRimElmProject\ManualTests\LCK_StageB1_20261031_Results`.
Full phase inventories use adjacent `LCK_StageB1_20261031_01_Untextured`,
`02_Carriers` and `03_WordClosure` directories. Historical census metadata retained
an older hardcoded checkpoint label; the comparison manifest and this document
identify the actual `03e23ab` baseline and implementation phases.

Automated regression logs: `D:\AstraRimElmProject\ManualTests\LCK_StageB1_20261031_Regression`.
The regular suite passed 314 tests (59 opt-in diagnostics ignored by default);
22 explicitly run fixtures passed, including Stage A, native registration, Bylon,
R8 move/rotate/delete/reopen, five-object export and native uploader fixtures were
run separately. Initial two fixture failures were corrected: the historical Bylon
witness now remains the same witness despite expanded discovery; descriptor identity
is checked by native source kind and exact bytes rather than a positional table
slot assumption. Original failure logs are preserved alongside passing reruns.
Supplemental binding and six-request runs initially lacked fixture environment
paths; both passed after supplying their required paths, including the exact
historical six-request source. `FINAL_REGRESSION_RESULTS.json` identifies all final
logs and their hashes. `cargo check -p legaia-studio --bin legaia-studio --locked`
also passed; this compiles/checks source without launching the application.

## Verification boundaries

Package success establishes static material closure and native TIM/TMD readback,
not fit in an arbitrary destination cell. Encoded staging, decoded destinations,
fixed asset arenas, dynamic heap requests and cumulative allocations remain distinct
budgets. This work does not enlarge PS1 RAM or certify full scene memory use.
Animation, scripts, runtime texture production, visibility, interaction and collision
behavior are not certified by these material checks. Historical user-reported Bylon
and R8 gameplay successes remain historical evidence; no new GUI or gameplay test
was performed for Stage B1, and no host LCK or DuckStation was launched.
