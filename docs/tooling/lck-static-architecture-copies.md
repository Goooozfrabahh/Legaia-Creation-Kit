# Static architecture copies — experimental native export

The user identified the missing Rayman structure as an elevator shaft and
requested copies independent of donor story flags until behavior editing exists.
New World Objects placement/terrain drops, including drops from the current cell,
now explicitly request static architecture. Ctrl+D of an imported instance inherits
its mode. Existing native Ctrl+D retains the established M1.1 path.

## Contract

This mode copies the **raw geometry currently displayed by World Objects**, with
the real donor TMD/TIM/CLUT references and independent native MAP identities.
It does not copy the donor MAN context, ANM playback, story conditions, door
triggers, elevator movement or interactions. It does not bake an animated donor
frame: World Objects' existing architecture preview is raw geometry. Retaining
the donor script/animation evidence in the export report permits later dependency
editing without claiming those dependencies were transplanted.

The native allocator selects an empty reference whose footprint anchor is in
bounds, lacks `CELL_BIND_OWNED`, and has no primary/fallback trigger. It leaves
existing trigger tables, original object descriptors and collision/floor grids
untouched. This makes the copy eligible for the ordinary unbound window sweep;
it does **not** disable native camera/window culling or prove live visibility.
No global story flags or PS1 engine instructions are changed. The existing
record-local collision and explicitly requested collision recipes remain
separate; script-driven collision/walkability is not synthesized.

`FOREIGN_STATIC\t<uid>` follows its `FOREIGN` record in saved scenes. Old scenes
without that marker retain their prior dependency intent. The World Viewport
shows a per-import checkbox, **Static architecture copy — no donor story scripts
or animation**, allowing an old imported selection to be converted explicitly.
Mode changes participate in scene undo/redo and persistence. Reports distinguish
intentionally detached behavior from a missing dependency on a behavior-preserving
request. Unrestricted export remains enabled and real writer failures remain
visible.

The previously failed shaft already had no destination MAN binding. Therefore
story flags are **not a proven cause** of its disappearance, and this checkpoint
does not claim to solve that runtime failure. Its original MAN context 7 / clip 3
and incoming context-reference sites remain recorded as donor evidence.

## Reusable native carrier improvements

* TIM_LIST/PAC upload members can contain opaque trailing bytes. Original carrier
  bytes are retained; only the CLUT/image extent actually consumed by
  `FUN_800198E0` enters the upload dependency trace. The original SCUS uploader
  was executed against 225 representative TIMs, including one nonzero suffix:
  full-member and trimmed-member GPU words match. GPU queue/DrawSync substituted;
  no rendering or gameplay executed.
* PAC size checks now apply after selecting the final carrier. Oversized raw
  candidates can use the compressed type-1 TIM_LIST route. When a PAC also
  registers models, original contributors remain ordered and only new models
  are appended to that stream. Texture descriptors register no model slots.
  The final sector-rounded PAC/bundle extent checks remain in place; unrestricted
  mode continues to expose genuine over-budget attempts as diagnostics.
* The format-family test now compares cumulative exported banks with original
  SCUS registration instructions, not solely the export reader. It preserves
  each donor's original yaw before checking the additional quarter-turn.

The existing 13-family audit now has **7 passing resource/MAP contract fixtures**.
Six remain explicit gaps: other1, dolk2, urudre1 and town0c expose incomplete
external texture/palette closure; conc and other7 lack an established texture
upload route. These names identify regression representatives, not production
special cases or confirmed engine limitations. Original-instruction simulation
substitutes heap/LZS/GPU and non-model installers. Universal compatibility is
not achieved.

## Existing user layout and manual procedure

No new object arrangement was authored. The user's exact three-import layout
was replayed with static mode requested for each copy:

`D:\AstraRimElmProject\ManualTests\Runtime_StaticArchitecture_20261028_01\LCK_EXPERIMENTAL_MIXED_STATIC_ARCHITECTURE.img`

All three native references/resources were encoded and the IMG was written,
read back and reopened through the native scene loader. Existing collision/floor
and trigger regions and unrelated PROT entries compare unchanged against the
earlier mixed export. Intended visible positions remain the user's positions;
native precision rounding is logged. Some new reference cells differ because
static copies require unbound anchors; this does not move visible geometry.

1. Launch Latest Build manually. Open the source IMG and the saved editing scene.
2. Select the imported elevator shaft. In the World Viewport, enable the static
   architecture checkbox if loading an older saved scene. New drops default on.
3. Duplicate it, move/rotate the copies, then save the LCK scene. Reopen and check
   that the checkbox, independent identities and transforms persist.
4. Export to a new filename. Check `debug.txt` and `object-verification.json` for
   `copy_mode: static_architecture_raw_visible_mesh`, three requested references
   (for the provided benchmark), no failed encodings, and behavior detachment.
5. For gameplay, cold-boot the new IMG and load an in-game memory-card save for
   the edited cell. Record which scene variant actually loads. A savestate can
   retain old scene resources; the user's previous launch method is unknown.
6. Inspect the shaft and other imports, Vahn's face, collision, and appearance
   after camera/movement transitions. The architecture copy should not move as
   an elevator or execute donor interactions. Report missing/incorrect geometry
   separately from texture, collision and story behavior.

Use the supplied static IMG to compare the **same existing layout** directly.
Its graphics and gameplay remain unverified until the user reports results.
No LCK/DuckStation or host desktop/input was launched or controlled. There is no
established graphical sandbox. M1.1, 31e125b, earlier Bylon artifacts, original
retail disc and rollback images remain protected.

## Automated evidence at this checkpoint

* Studio regression: 285 unit tests and 9 integration tests passed; 35 optional
  source/runtime fixtures ignored by the default command and reported separately.
* Exact static user-layout export: 3/3 references encoded, zero writer failures,
  on-disk readback and native reopen passed. SHA-256:
  `bac60a7bea880e1177f5c4e7108b3a4c33c0a0f05493c8aa53aed211f9bbb133`.
* Original registration/window instruction, material-alignment and resident
  texture integrity fixtures: 4 passed. Heap/GPU/actor-constructor services are
  substituted; this is not a live PS1 run.
* Bylon preservation: MAP, collision/floors/bindings and unrelated entries remain
  byte-identical; 29,164 imported material samples match. The separate framebuffer
  overwrite simulation still corrupts zero corrected samples.
* Thirteen cross-cell family fixtures retain six explicit gaps and seven passing
  contracts, with original SCUS registration instructions matching every passing
  cumulative model bank. The original TIM uploader suffix fixture also passed.

Evidence lives beside the static IMG and in the associated `runtime-*.log` files.
Runtime rendering, collision/interaction and GUI verification remain false.
