# M1.1 manual acceptance and development continuation

The user operates LCK and DuckStation. The assistant must not launch either
application or control the host desktop. VM provisioning is paused indefinitely;
development continues without waiting for a sandbox. Current AGENTS.md is binding.

## Preserved baseline

- Source commit: `2667653216e82a14a48678f0d078836fd5add48b`.
- Installed build: `D:\AstraRimElmProject\legaiasource\legend-of-legaia-re-main\target\release\legaia-studio.exe`.
- Executable SHA-256: `87235b719b5c03fac85a4f6ee82842fd9294a0f0bac614b4c1566d58f15b1cc4`.
- Recovery bundle: `D:\AstraRimElmProject\Backups\LCK_M11_Source_Checkpoint_2667653\lck-m11.bundle`.
- Rollback executables: `D:\AstraRimElmProject\Backups\LCK_M11_rollback_20261027_01`.

## First manual editor scenario

Launch the installed executable yourself. Open this existing experimental input,
retaining its adjacent JSON receipt at the same path:

`D:\AstraRimElmProject\Backups\LCK_Outdoor_Visual_20261027\LCK_EXPERIMENTAL_VISUAL_ONLY_town01_outdoor_foreign_props_20261027.img`

Open native scene `town01`. The two previously inserted native objects are
descriptor 509 (Biron donor) and descriptor 463 (Hunter's Spring donor), at
native spawn cells (31,20) and (32,20). Their original native PSX origins are
(4032,-240,2624) and (4160,-240,2624). Record the scene name and selected-object
identities alongside screenshots; other Rim Elm story states are different MAPs.

1. Shift-select both through the hierarchy, then repeat using the viewport.
   Both should highlight, with one primary object in Properties. Clicking an
   already selected member should preserve the group.
2. Translate the group. Both should receive the same delta. Check snapping and
   Ctrl vertical movement. Undo should restore both in one operation; redo
   should reapply both in one operation. Record the actual deltas.
3. Check that rotation and scale affect the primary object under the existing
   M1 behavior; this is an editor test, not native rotation/scale approval. Undo
   these changes before trying the translation-only native export scenario.
4. Duplicate a selected object, delete that new staged copy, then undo/redo.
   Check the donor and unrelated placements remain intact. Staged-copy deletion
   is distinct from deletion of an already inserted native placement.
5. Delete both inserted native objects and check one-operation undo/redo.
   Verify Frame Selected, camera movement, and Properties remain usable.
6. Save to a new scene-document path using the editor's save dialog, reopen it,
   and check transforms, hidden/deleted state, selection behavior and names.
7. Restore a clean input scene. Move the two native additions +64 on native X
   only; preflight and export to a new path whose filename contains
   `EXPERIMENTAL`. Record the full success/error text and output/receipt paths.
   Separately test delete-two and move-one/delete-one from fresh input scenes.

Do not reuse earlier output filenames. A rejected preflight must not be counted
as a successful native edit. Capture the exact error and requested object/edit.

## Existing M1.1 gameplay scenarios

Launch DuckStation yourself and cold boot each exact IMG below. Full save states
restore old RAM/resources and cannot verify these changes. Ensure the runtime
actually loads `town01`; the edits intentionally do not change other story MAPs.

All three are under:
`D:\AstraRimElmProject\Backups\LCK_M11_Native_20261027_01`

| IMG basename | Native readback result to check visually |
|---|---|
| `LCK_EXPERIMENTAL_M11_town01_move_two.img` | Both additions remain; native origins (4096,-240,2624), (4224,-240,2624) |
| `LCK_EXPERIMENTAL_M11_town01_delete_two.img` | Both added placement references are absent |
| `LCK_EXPERIMENTAL_M11_town01_move_one_delete_one.img` | Descriptor 509 at (4096,-240,2624); descriptor 463 absent |

For each, report whether the scene loads, whether both expected visibility and
placement results occur, and whether walking, camera movement, transitions and
nearby original objects behave normally. Compare relevant collision with the
experimental input: original MAP wall/floor bits were preserved, so do not infer
that every pre-existing wall near a deleted addition should disappear. Record
ghost geometry/collision, popping, texture corruption, hangs or crashes.
The 121 registered models can remain after deletion; that count does not prove
that either added instance is visible or deleted.

## Negative checks and next development work

M1.1 does not solve general original-object deletion: the existing read-only
retail inventory accepted 0 of 35 placements. Vahn's house (#137) has a binding;
descriptor #230 has spatial wall conflicts with unknown ownership. If tested,
these must produce explicit rejection with no successful partial IMG. They are
remaining development cases, not fulfilled unrestricted deletion support.

Continue in bounded native-export increments while preserving existing editor
behavior and successful resources/placements:

1. Establish explicit native ownership/reconstruction for original collision,
   including shared contributions, bound objects and story-state identities.
2. Extend native placement allocation and movement without distant/invisible
   anchors, unrelated object relocation, or invented capacity limits.
3. Carry authored walls and eventually owned walkable surfaces through move,
   duplicate, delete, persistence, undo/redo and native IMG readback.
4. Complete resource/actor/script and RAM/VRAM lifetime contracts for general
   imports and reliable export. Preserve index-200 conflict protection.

Each implementation increment gets targeted automated tests, a Git checkpoint,
rollback-protected installation when needed, and a user-operated acceptance
scenario. No safety gate is removed merely to make an unsupported export pass.

## Results to return

For each scenario provide: build hash/version, input IMG and scene, object
identity, exact actions and deltas, expected versus observed behavior, screenshot
or log, and output IMG/receipt paths if applicable. The assistant records these
as user-reported manual GUI or user-reported native gameplay evidence, with the
specific tested scope. Automated tests and untested assumptions remain separate.
