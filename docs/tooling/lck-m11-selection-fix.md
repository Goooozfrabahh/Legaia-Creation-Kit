# M1.1 multi-selection correction and manual acceptance

## Cause

The user's desktop `LatestBuild__LCK.cmd` ran `cargo run -p legaia-studio
--release --locked` from `legend-of-legaia-re-main`, whose HEAD was `fc7bcd4`.
That checkout predates M1/M1.1. Using the launcher rebuilt the older source and
replaced the installed executable. Its observed SHA-256 was
`9ddf544d5f5cb84e1ff7cc8cc0e4bdaec703ffd2518346e9c2cf7d8e273fa3bd`.
It was not the recorded M1.1 executable (`87235b71...`).

The development source also needed the user's updated selection contract:
Shift previously toggled, Ctrl and hierarchy range selection were absent,
rotation/scale/duplication used the primary only, and numeric translation was
single-object outside a viewport drag. The corrected paths use the shared set.
The launcher now opens the installed executable directly; it never recompiles
the older production checkout. The assistant does not execute this launcher.

## Implemented behavior

- Viewport: click selects one, Shift-click adds without removing an existing
  member, Ctrl-click toggles. A drag starting on any selected member keeps and
  moves the group. Ctrl-drag preserves vertical movement; its click toggle is
  deferred until release and cancelled for a drag.
- Hierarchy: click selects one; Ctrl-click toggles; Shift-click selects the
  inclusive range in expanded/filtered display order; Ctrl+Shift adds a range.
- Highlighting, primary selection and an open inspector follow that same set.
- Inspector XYZ edits translate the group by the primary's delta. Rotation adds
  the same angular delta at each object's individual pivot. Scale applies the
  primary's per-axis ratio at individual pivots. This is not shared-pivot orbiting.
- Ctrl+D/toolbar duplicate every selected supported source independently with
  the same +128 X displacement, including transform, naming, collision recipe
  and supported NPC appearance metadata. New copies become the selection.
  Invalid members reject the whole batch. Delete and undo/redo use full groups.
- Native export consumes the resulting scene document. Existing unsupported
  native rotation/scale, unknown collision, actor and resource gates remain.
  Editor support is not a promise that every group is native-exportable.

## Exact manual procedure for the corrected release

The user performs every GUI action. The assistant has not launched LCK or an
emulator. Use the existing desktop **LatestBuild__LCK.cmd** after installation.
If an old LCK window is still open, close it yourself and launch again. The
corrected hierarchy displays `Click: single • Ctrl: toggle • Shift: range` and
a selection count. If that guidance is absent, report it before editing.

Dedicated test directory, already created:

`D:\AstraRimElmProject\ManualTests\M11_SelectionFix_20261027_01`

Open this IMG, retaining its adjacent JSON receipt at its existing path:

`D:\AstraRimElmProject\Backups\LCK_Outdoor_Visual_20261027\LCK_EXPERIMENTAL_VISUAL_ONLY_town01_outdoor_foreign_props_20261027.img`

Open **World Editor**, then the native **town01** scene. Identify the two added
native placements by the coordinates shown in the hierarchy and original PSX
coordinates in Properties:

| Object | Native descriptor | Source cell | Original PSX X,Y,Z |
|---|---:|---|---|
| A: Biron donor | 509 | (31,20) | (4032,-240,2624) |
| B: Hunter's Spring donor | 463 | (32,20) | (4160,-240,2624) |

These are native placements already present in this experimental input, not new
repository imports. Hierarchy `#nnn` labels are draw-list indices, not descriptor
IDs; use the coordinate identity above. Clear the hierarchy filter after locating
each. Press F deliberately to frame the primary object when useful.

### Selection and editor operations

1. Click A in the hierarchy, then Ctrl-click B. Expect selection count 2, both
   rows highlighted, and both viewport objects highlighted. Click A normally:
   expect one selected. Ctrl-click B again: expect two.
2. Click a hierarchy row, then Shift-click a different visible row. Expect every
   visible row between the endpoints selected, inclusively. Repeat backwards;
   hidden/collapsed/filtered-out rows must not enter the range. Ctrl+Shift-click
   should add a range. Restore selection to A/B before any edit.
3. In the viewport, click A normally, then Shift-click B. Expect two selected.
   Shift-click B again: expect two. Ctrl-click A: expect only B. Ctrl-click A
   again: expect two. A normal click on B should select only B. Restore A/B.
4. Open Properties for the primary, then select the other member in either
   surface. The inspector should follow the primary and show the group count.
   Editing/selection alone should not move the camera.
5. Set **Grid snap**, **64 units**, **Drag X only**, and turn **Magnet: object
   centers** off. Drag an already-selected member along X. Inspect each member:
   both must have the same X delta, with Y/Z unchanged. Ctrl+Z must undo both in
   one operation; Ctrl+Y must restore both. Undo again.
6. Ctrl-drag a selected member vertically. Both Y offsets should change by the
   same amount with X/Z unchanged. Undo once. Click-and-release with Ctrl
   without dragging must toggle selection rather than move geometry.
7. Restore A/B and open Properties. Set primary X offset to **64**, Y/Z to **0**.
   Both should receive +64 X. Undo once. Set primary Y rotation to **15 degrees**:
   both should rotate 15 degrees about their individual pivots. Undo once. Set
   uniform scale to **1.25**: both should scale by that ratio. Undo once. Confirm
   all editor rotations are zero and scales are 1 before native exports.
8. With A/B selected, Ctrl+D. Expect two new independent copies, both selected,
   each +128 X from its source, with unrelated objects unchanged. Undo once:
   both copies disappear. Redo once: both return. Delete them: both disappear
   in one action. Undo that deletion, then undo duplication so no staged copies
   or generated collision recipes remain before the native export cases.
9. Save a scene document to
   `D:\AstraRimElmProject\ManualTests\M11_SelectionFix_20261027_01\scenes\town01-group-ui.lckscene`.
   Reopen it and confirm saved object edits/names. Selection itself is session
   state; undo/redo restores its recorded group, while reopening a scene is not
   required to restore selection.

### Native export cases

Start each case by reopening the original experimental input and `town01`,
without applying earlier scene documents. Select only A/B. Use **Check IMG**,
then **Export PS1 test IMG...**. Record the complete status/error. A rejected
preflight must not create a successful partial image. Never overwrite prior
images; if a listed output already exists, use a new `_02` suffix.

| Case | Exact edits | New output path |
|---|---|---|
| Move both | With both selected, set primary X offset 64, Y/Z 0. Keep rotation 0 and scale 1. | `D:\AstraRimElmProject\ManualTests\M11_SelectionFix_20261027_01\exports\LCK_EXPERIMENTAL_M11_SELECTION_move_two_01.img` |
| Delete both | Select A/B, press Delete. | `D:\AstraRimElmProject\ManualTests\M11_SelectionFix_20261027_01\exports\LCK_EXPERIMENTAL_M11_SELECTION_delete_two_01.img` |
| Mixed | Select A alone; set X offset 64. Select B alone; Delete. | `D:\AstraRimElmProject\ManualTests\M11_SelectionFix_20261027_01\exports\LCK_EXPERIMENTAL_M11_SELECTION_mixed_01.img` |

Expected native positions: move-two A=(4096,-240,2624), B=(4224,-240,2624);
delete-two has neither added placement; mixed has A=(4096,-240,2624) and no B.
Other placements, MAP walls/floors, scripts and other Rim Elm story states should
remain unchanged. Resource registrations may remain at 121 after deletion.

For gameplay, launch DuckStation yourself and cold boot the exact new IMG.
Do not load a full save state. Verify that the game actually loads `town01`;
these additions are scoped to that MAP. Record visible placements, walking and
collision behavior near the original and edited locations, textures, camera
movement, transitions and any hang/crash. Original walls near a deleted addition
can remain because the baseline walls were not created by that addition.

## Results and evidence

Save screenshots/logs under the test directory's `results` folder. Report which
numbered step failed, input/scene, selected-object count and primary identity,
actual offsets, full export message and output/receipt paths. The installation
manifest records the corrected executable hash and source checkpoint.

Automated event, persistence and native-byte tests are separate from the user's
manual GUI/gameplay results. The user's first manual failure of the old shortcut
build is recorded; the corrected release remains manually unverified until the
user supplies new results.
