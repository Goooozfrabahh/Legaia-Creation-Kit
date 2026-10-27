# Multi-selection M1

This increment lives on `lck/multiselection-m1`, based on milestone `fc7bcd42`,
in a separate worktree. The original `foundation-02` checkout remains unchanged.
Development uses repository/shell tools and headless egui tests, with a separate
build directory. No installed application, desktop, emulator, local backup or
retail image is accessed or operated.

## Editor behavior

Viewport and hierarchy use one selection set and one primary object. Plain
click replaces the selection only when the object was unselected; clicking a
selected member preserves the group and makes that member primary. Shift-click
adds/removes a member. Escape clears the selection and dismisses an open
selection-related modal. The inspector, Frame Selected and rotations/scaling
continue to operate on the primary; there is no group rotation/scale mode.

Dragging any selected member captures the group at press time. The existing
cursor projection, grid/anchor snap, axis lock and Ctrl vertical movement compute
one primary delta, which is applied to each starting offset. Other members are
not snapped independently. Texture rendering uses the same depth/material path
with a set of highlighted IDs. A captured object drag does not pan the camera.

Batch Delete stages all removals transactionally. Original draws become
persisted HIDE edits; created/native-copy/import instances are actually removed
from the overlay along with their own recipes and material references. Their
donor resources and unrelated instances remain unchanged. Full scene history
captures stable created UIDs, all selected identities and the primary, so one
gesture/deletion is one undo/redo operation. Edited transforms, deletion and
collision recipes use the existing scene format; selection itself is session
state, not a new scene-file record.

## Native serialization and exact restrictions

Movement already iterates all saved scene edits, independent of selection.
M1 retains that pipeline and all collision, resource, transform and runtime
guards. No multi-selection-specific primary-only export route is introduced.

Native deletion now has an explicit plan entry per hidden original static
placement. It verifies source position/model/orientation and native bindings,
then clears only its object-index grid reference. Descriptor/model resources
are retained; scripts, floors and walls are never erased. All preflight/final
serialization uses the existing deterministic native plan and readback.

The first deletion adapter is deliberately narrow:

- It requires an entirely **wall-free MAP**, not inferred object ownership from
  geometry bounds. Consequently **Rim Elm original-object deletion remains
  blocked** while its original wall grid contains unresolved ownership.
- A script-bound/triggered placement is rejected, including verified fallback
  bindings. References carrying terrain/visibility flags cannot be cleared by
  this static-placement adapter.
- Native terrain and NPC/actor deletion require separate floor/MAN/dependency
  writers and reject explicitly. A deleted original's earlier preview transform
  is superseded by its final deletion; it is not exported as a surviving move.
- Removing a newly created duplicate/import removes its insertion request;
  it does not remove or alter its donor or other native originals.
- Remaining foreign imports, authored collision, native rotation/scale and
  resource/runtime obligations retain their previous restrictions. This run
  does not enable general cross-cell production export. An unsupported member
  causes failure of the complete native plan, not a partial successful IMG.

This is native reference serialization with conservative gates, not a claim of
verified retail physical behavior or complete M1 gameplay readiness.

## Validation

Studio library regression result: **214 passed, 0 failed, 14 ignored**.
The ignored retail-dependent cases are intentionally not exercised in this run.

All new assets and disc images in this run are generated synthetic fixtures.
The native test creates three independent placements, compiles a shared
translation (including Y), writes and reopens a synthetic IMG, and separately
writes/reopens the three-deletion result. It checks placement lookup, original
descriptor preservation for deletions, unchanged floor/wall/binding regions,
unchanged neighboring resource bytes, source preservation, deterministic plans,
sector EDC/ECC and rejection of output overwrites. These IMG files are structural
fixtures, **not playable Legend of Legaia game images**.

Headless tests cover:

- Real hierarchy Shift-clicks, toggle/primary rules and mixed shared selection.
- Viewport raster picking, capture of a selected non-primary member, multiple
  highlights, release without reselection and no camera movement.
- A real three-object pointer drag through the App workspace, relative spacing,
  and one complete keyboard undo/redo.
- Shared snapped translation and vertical movement; invalid-member rollback.
- Batch Delete through the App workspace with one undo/redo and selection restore.
- Scene save/parse/reopen, stable identities, recipes, UVs and palette references.
- Import metadata removal without touching the donor or unrelated instance.
- Native deletion refusal for walls, bindings, shared grid flags and conflicting
  commands. Existing cross-cell resource and native safety regressions also run.

Retail fixtures and emulator gameplay are not run. Real-town group movement,
script-driven actor behavior, cross-cell gameplay and physical deletion still
need the owner's manual validation after supported native contracts are met.

## Modified files

- `crates/studio/src/app.rs`
- `crates/studio/src/app/multi_selection.rs`
- `crates/studio/src/app/viewport.rs`
- `crates/studio/src/app/ps1_export.rs`
- `crates/studio/src/raster.rs`
- `docs/tooling/lck-multiselection-m1.md`

## Windows CMD review/build instructions

Use the isolated worktree directly; this does not install over the current LCK.

```bat
cd /d D:\AstraRimElmProject\legaiasource\lck-multiselection-m1
git status --short --branch
set "CARGO_TARGET_DIR=D:\AstraRimElmProject\SandboxBuilds\lck-m1"
if not exist target\test-temp mkdir target\test-temp
set "TEMP=D:\AstraRimElmProject\legaiasource\lck-multiselection-m1\target\test-temp"
set "TMP=%TEMP%"
cargo build -p legaia-studio --locked --offline
cargo test -p legaia-studio --lib --locked --offline
```

If dependencies are not cached on another machine, omit `--offline`.
The review executable is
`D:\AstraRimElmProject\SandboxBuilds\lck-m1\debug\legaia-studio.exe`;
the owner can launch it manually. No source merge or installation is performed.
