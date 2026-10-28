# Temporary Unrestricted IMG Export Mode — 2026-10-28

User-requested experimental export attempts; no new test scenes or engine patch.
The mode defaults on. In World Viewport, leave **Unrestricted IMG Export Mode
(experimental)** checked, make your own edits, and use **Export IMG**. Choose a
new output filename. Existing files and the source IMG are never replaced.
Turning the checkbox off restores the previous strict exporter.

The mode bypasses whole-document native dependency/compatibility preflight and
attempts native operations separately. Existing writer checks become warnings
inside a scoped export attempt; strict behavior is preserved outside that scope.
Occupied visual positions, distant anchors, native bindings, ownership uncertainty,
and estimated resource budgets do not prevent attempting an output image.

Native MAP offset/deletion/copy allocation, representable yaw, existing MAN spawn
and appearance writers, cross-cell placement resource registration, explicit
collision rasterization, and applicable native primary floor-row writers are
attempted. There is no new geometry/engine-format implementation. Missing writers
(including general scale, pitch/roll composition, some imported categories, actor
deletion, and mesh-only walkability) are explicit failures. Exact native numeric
and storage encodings can still fail. A failure does not become a replacement
edit, and does not stop other requests or publication of the experimental IMG.

Each output has a unique sibling directory named `<output-stem>_export_<stamp>`
containing **debug.txt**. It records the complete editor manifest, attempts,
bypassed checks, successful encodings, actual committed native writes, failures,
structural reparse result, output status and SHA256. ENCODED/RESOLVED is not
WRITTEN: MAP operations reach the image only when their collective MAP commit
succeeds. Missing external wall/floor/script relocation is reported explicitly;
native behavior remains unverified even if no encoder failed. Partial output is
always marked experimental and **NOT PROVEN COMPLETE** in the editor.

Hierarchy labels remain editor metadata; save `.lckscene` to preserve those names.
Review debug.txt when testing. Do not interpret unchanged or partially changed
gameplay as evidence that all requested operations were serialized.

## Preservation and verification

Installed M1.1 rollback executable:
`D:\AstraRimElmProject\Backups\LCK_Unrestricted_20261028_01\legaia-studio-M1.1-preserved.exe`

SHA256: `ce3dc080a0a7427310176be1bf0c7be33af4e569451bf2b596ea040392612b32`.
The same directory includes a verified complete pre-change Git recovery bundle.
The original checkpoints, retail disc, and Stage E artifacts remain preserved.

Automated verification: 268 studio unit tests + 9 integration tests passed;
26 optional tests ignored. Added regressions cover scoped bypass/strict isolation,
occupied and bound native records, distant independent copies, and explicit
encoder failures without false WRITTEN claims. Existing selection event tests
passed after placing the new checkbox in a fixed row.

Separate unedited pipeline proof:
`D:\AstraRimElmProject\ManualTests\Unrestricted_IMG_20261028_01\LCK_EXPERIMENTAL_UNEDITED_PIPELINE.img`

It reparsed and is byte-identical to the retail source (SHA256
`750dbcbadb70362feb3468467bbe7efbf3d151a414caa428c66c368d4b0b73e6`).
No edited scene or gameplay arrangement was generated. This checks output and
diagnostic publication only. The current experimental GUI and native gameplay
remain **not verified**. No LCK or DuckStation application was launched or
controlled; the user performs all manual testing.

Latest Build target:
`D:\AstraRimElmProject\legaiasource\legend-of-legaia-re-main\target\release\legaia-studio.exe`.
Installation copies the compiled executable there after preserving M1.1, without
launching it. With LCK closed, restoring the preserved executable to this target
rolls back the installed build. Source checkpoints remain available in Git.
