# Persistent native object lifecycle — 2026-10-30

R8 is user-reported native gameplay success for the recovered moved/deleted collision. The normal editing/export integration described here is a separate change. Automated checks do not establish GUI or gameplay behavior.

## Implemented behavior

The exporter records the actual authored wall cells for each independent native instance. Moving or rotating a previously exported, recorded mesh-collision owner replaces its layer using the final reparsed native geometry and its recorded collision policy. Deleting it removes its layer. Recomposition retains the baseline, other owners, overlapping contributions, and all native floor-tier nibbles. It does not clear walls by mesh bounds or distance.

Ownership is stored beside each experimental IMG in `<name>.lck-collision.json`. Keep this file beside the IMG when reopening, renaming or copying an editing source. Its IMG/MAP/model/descriptor hashes reject stale provenance without stopping unrestricted geometry attempts; missing ownership is reported rather than inferred. New journals are self-contained and support several native cells in one IMG. Legacy v1 journals migrate using their verified original baseline once.

The collision wireframe recomposes the same authored owner layers from the live editor transform, including drag gestures and undo snapshots. It reads no donor disc or resource bundle during a gesture. The preview uses editor precision; export uses and logs representable native precision.

Deleted editor-added model storage is retired only after proving the protected model prefix and native dependency payloads unchanged and finding no surviving MAP reference. A shared model remains until its last reference is removed. Empty model records preserve selector numbering; unrelated models are not shifted. An optional storage-release failure retains storage and is diagnosed independently of successful collision recomposition.

## Remaining capability gaps

This is not complete GECK parity. Collision ownership of original retail walls, scripted barriers, MAN actor colliders, floors and story-controlled dependencies is not established by an editor mesh. Unrecorded original contributions remain protected and their unresolved ownership is logged. Manually authored wall sub-cells without a generated mesh policy currently support exact native grid translation, not arbitrary rotation/scale/height.

Unused TIM images, shared CLUTs and active animation upload ranges are retained. Model retirement does not claim texture retirement. Resource deletion needs complete surviving geometry/animation/runtime texture references before those uploads can be released. Unrestricted export remains enabled and reports encoder/ownership failures as incomplete attempts.

## Manual check after installation

1. Open the preserved `ManualTests/RuntimeRecovery_20261030_01/R8_FULL_SCENE_OWNED_COLLISION_RECOVERY.img` and keep its adjacent collision JSON. Open Rim Elm `town01`.
2. Enable Collision wireframe. Select the moved foreign structure whose old collision was fixed in R8 (recorded reference cell 20,23; original descriptor 462; model selector 116). Move it twice and rotate it; watch its authored wall overlay follow each edit. Undo/redo should restore each overlay without accumulating old stencils.
3. Leave the existing duplicates alone. Export to a new filename such as `D:/AstraRimElmProject/ManualTests/OwnedLifecycle_Manual_01.img`. Check `debug.txt` for `WRITTEN OWNED COLLISION`, no `FAILED OWNED COLLISION`, and the output file/readback confirmation. Keep the new collision JSON.
4. Reopen that new IMG in LCK, move the same structure again, then export to another new filename. Verify the source and duplicates retain their separate identities.
5. Reopen the second IMG, delete the moved source, and export to a third new filename. The duplicates and their collision must remain.
6. Personally cold-boot each output in DuckStation using the same in-game save. Check old and intermediate positions are walkable, current visible positions block appropriately, deletion removes only that owner's blocking, duplicates still block, and Vahn's face textures remain correct. Report loading, rendering, collision and texture results separately.

Do not use the automated regression arrangement as gameplay evidence. Retail, M1.1, prior recovery IMGs and their rollback copies remain protected. No GUI/emulator was launched by the assistant.
