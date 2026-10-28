# M2 native floor and script capability checkpoint

The exact six-object export is not complete. No retail engine patch or six-edit
retail IMG was produced. M1.1 remains Latest Build. This work adds native data
writers and tests the unchanged retail sampler, rather than another inventory.

## Native floor capability implemented

`native_floor_patch.rs` serializes explicitly selected primary kind-2 floor
components through the real MAP trigger-table layout. It supports whole-tile
moves, independently keyed copies, quarter rotations, deletion and representable
Y changes. It rebuilds native counts/offsets, preserves kind-0/1/3 row contents
and two-byte table gaps, and reparses every resulting elevation row. Allocation
is limited by the actual reserved 0x2000-byte primary trigger block; non-padding
bytes cannot be overwritten. Fallback shadowing, duplicate source keys,
destination contributions, conflicting edits and stale source bytes fail.

Only the elevation-enable mask changes in object-grid cells. Descriptor bytes,
wall/floor-tier bytes, other grid fields and the physical fallback remain
unchanged. Native edge sampling aliases into the first grid bytes; the writer
checks those low nibbles too, rather than corrupting an edge floor through a
seemingly independent flag edit.

Source/destination floor-tier **weights** must match, not merely their current
height averages. This preserves behavior under any later changes to the live
16-rung ladder without baking a static height snapshot into a moved override.
Different weights require floor-script closure before compensation is allowed.

These are floor-record operations. No mesh-to-floor ownership is fabricated.
The floor plan type is separate from `NativeExportPlan` and is not wired into
the six-object export as an ownership waiver.

## Original instruction evidence

The explicit read-only test loads the user's original executable/field overlay
and executes their unmodified instructions in the existing bounded R3000 subset
unit-test model. No LCK application, emulator application or desktop is launched.
`SRAV` support and its sign/count tests were added to that model. The model is
functional, not cycle-accurate, and does not model load delays or the full PS1.
Its results are automated native-code/format evidence, not gameplay evidence.

The test requires these NTSC-U profiles; it refuses unknown hashes:

- SCUS SHA-256 `292256e2e66db42727f613406785e444254d3f699569e611f65fcf1c6d2f3482`.
- Field overlay PROT 897 SHA-256 `0b39d892d783ffcc89dac06ee395540be0a7392c5cbc79fb9ba3442daccc09f6`.

The native code accepted the serialized full-tile floor moves, copies, quarter
rotations and deletion. It also returned the original kind-3 record after the
kind-2 table grew and relocated that region table. A second kind-2 row with the
same tile key remained shadowed: native lookup returned the first row for both
sampled sub-cells. Additional rows do not subdivide that tile.

| Native routine/data | Relevant contract |
|---|---|
| `FUN_80019278`, SCUS `0x80019278..0x800195a8` | Actor-position floor query; bilinear or kind-2 branch |
| `0x80019384..0x80019398` | Test cell `0x0800`; call native kind-2 lookup |
| `0x800193b4..0x800193d8` | Signed coarse byte ×32; shift selected packed field, mask 3, step ×16 |
| `0x80019444` onward | Ordinary bilinear tier interpolation |
| `FUN_801D5630` | Primary then physical-fallback table lookup |
| `FUN_801D5AE0` | Header offset/count, kind stride and exact tile-key match |
| `DAT_8007B318 + kind` | Native strides 4,4,4,8; kind-2 rows are four bytes |
| MAP `+0x4000` | Shared wall nibble and 4-bit floor-tier grid |
| MAP `+0x8000` | Tile references and floor-model flag |
| MAN `+2..+0x22`; scratchpad `0x1f80035c` | Stored/live floor ladders |
| `FUN_8001F7C0` | Scene/MAP loading, including the 0x14000-byte physical window |

The native function returned -64/-112 for the staircase and -48/-144 on the
retained bilinear slope. Changing geometry descriptor bytes did not change those
floor samples. TMD triangles, TIMs and visible ground geometry are not an
alternative walkable-surface input to this sampler. Scripted actor-Y writes
are actor behavior, not an independently serialized general floor surface.

## Why the exact half-tile move still cannot be encoded

The requested `(-320,+64)` X/Z translation moves stair samples across four
128-unit tile keys. A top destination contains retained height 0 and moved
height -64: its 64-unit span exceeds the kind-2 row's maximum 48-unit span.
Changing the common mean cannot reduce that span. A further -112 sample requires
an even wider discontinuity.

The new affine/rounding bound also excludes the ordinary bilinear path for ANY
choice of corner tiers. Along a fixed X, native bilinear sampling is an affine
function rounded toward zero at 14 fractional bits. A row that remains 0 at
Z=0 and Z=63 cannot suddenly become -64 at Z=64. The proof includes integer
rounding; it is not restricted to the currently installed sixteen ladder values.

Lower destinations retain continuous bilinear slopes. Replacing them with
constant sub-cell steps changes unrelated samples. Thus simply widening the
packed steps or allocating more ordinary rows is also insufficient for the
whole requested surface. Duplicate-key rows were tested against native lookup;
the retail engine does not select a different row per 64-unit quadrant.

Existing floor records cannot be subdivided/re-keyed to 64-unit cells: the native
sampler and lookup derive the 128-unit key with fixed coordinate shifts. Table
reallocation changes storage addresses, not that spatial sampling rule.

### Required original-engine work; not implemented

An exact solution needs a new spatial sampling capability, such as sparse
64-unit overrides with a per-sub-cell mode: retain the original bilinear sampler
outside the moved component, and evaluate the moved stair contribution inside
it, including live source-tier dependencies. Independent heights alone would
not preserve retained continuous slopes.

That requires changes to the `FUN_80019278` path and versioned scene data
lookup/loading. The new path must retain actor-flag side effects, native return
and rounding behavior, source/destination live ladder behavior and fallback
rules. Any new row format needs its own interpreted stride/lookup path; changing
`DAT_8007B31A` alone does not teach the old sampler to read extra fields. Additional
scene storage, runtime allocation and PS1 RAM/CPU budgets must be proved. The
existing primary/fallback blocks cannot be extended into unrelated resources.
No such unverified engine patch was authored or applied.

## Collision status

There are two distinct native mechanisms: scene wall bits and record-local
actor colliders. The former are authored in the MAP high nibble and can be
modified by MAN wall-paint instructions. The latter are read by the actor box
tests (`FUN_801CFC40`/`FUN_801CF9F4`) from the actor's own MAP descriptor and flags.
Native context flags can disable collision/touch; the house's incoming scripts
explicitly set/clear those flags. These relationships are not mesh bounds.

The original selected objects still have no proven exclusive contributions to
the static wall grid. The dependency graph identifies stored scene bits and
decoded script effects, but no native record assigns those bits to the selected
object IDs. The existing reviewed-contribution move/copy/delete reconstruction
remains guarded; no new ownership mask or authoring provenance was invented.
Native actor-collider support does not resolve house wall ownership.

## Native MAN reference capability implemented

`native_script_relocation.rs` writes supported, explicitly remapped static
context-reference operands without shifting records, branches, positions,
transition data or unrelated bytes. It reparses the MAN and every changed
reference. It requires existing allocated target identities, injective remaps,
supported operand semantics and complete linear decoding. It rejects player/
system channel reassignment and extended system-flag operands which alias the
context byte as flag data. Spawn targets must remain partition-2 records.

The original house has 38 incoming reference operands from records 35, 92, 95
and 104. Its flag guards, field-68 guards, local/context flags, actor ramp and
EXEC_MOVE reference forms are now supported by this same-width writer. No house
was actually rebound to another existing object's context. MAP bindings,
independent actor allocation, animation/motion resources, helpers and story
conditions remain separate closure requirements. Records 92/95 invoke MOVE 1
on house context 0. No direct named scene-change opcode was found in the bound
record and its direct incoming records. No exact-anchor kind-0 teleport was
found in the physical primary/fallback tables; absence is not transition closure.

Keeping or relocating the MAN byte reference alone therefore does not establish
a correct independent moved/cloned house or preserve every scene transition.

## Verification levels and artifacts

- Format/native-code automated: native serializers, all encoded elevation
  patterns, integer bilinear obstruction, unchanged retail routine evaluation,
  supported MAN reference rewriting and stale/alias/conflict rejection.
- Structural IMG automated: synthetic raw-IMG floor-copy sector rewrite,
  reparse of native rows, exact MAP readback and unchanged unrelated entry.
- Six-request retail IMG: still rejected with every request retained; none
  silently omitted. No six-edit retail output was published.
- Actual gameplay / M2 GUI: not verified. The user performs those checks.

Logs are under the development worktree's `target`: `m2-retail-floor-probe.log`,
`m2-house-native-dependencies.log`, `m2-capabilities-all-targets.log`,
`m2-cpu-model-tests.log`, `m2-capabilities-six-request.log` and
`m2-capabilities-release.log`. Original discs, inventory checkpoint 175a162,
recovery bundles and the installed M1.1 release are preserved.

Final automated results: 258 studio library tests and 9 integration tests passed;
22 controlled/external cases are excluded by the default run. The original-code
floor test, house dependency test and exact-six-request rejection regression
were run explicitly and passed. The CPU-toolkit suite passed 82 tests (one
controlled case excluded). Release compilation passed with 122 warnings,
including currently unused backend entry points. These results do not constitute
a successful six-object retail export or gameplay verification.
