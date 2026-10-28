# M2 native MAP quarter-yaw adapter

This increment extends the existing game-wide export architecture. It is not
universal export completion and does not resolve the six-request Rim Elm batch.
The canonical global census at checkpoint 175a162 is reused, not regenerated.

## Native serialization

Original static MAP placements can now request quarter-turn yaw through the
normal editor-state/native export planner. Eligibility depends on actual source
records and dependencies, never scene names or object indices. The native
rotation triple is consumed by FUN_8003A55C / FUN_80026988; this adapter composes
pure native yaw only. Pitch/roll, arbitrary scale, animated/bound props, terrain
rotation, and created-instance rotations still require additional adapters.

The editor rotates around the mesh center, whereas the native engine rotates
around the spawn origin. For source spawn p, mesh center c, editor translation d
and quarter rotation R, the writer stores p' = c + R(p-c) + d. Whole-unit native
position and the existing spawn-window reach are checked. Fractional centers
are allowed when the resulting native position is representable.

The native record-local static collider is a +/-80 axis-aligned square centered
at p + o, using the actual descriptor footprint offset decoded by
field_objects::collision_footprint_offset. Quarter yaw preserves the square's
shape. The writer encodes p' + R(o), solving signed 16-unit sub-anchors while
retaining column/row binding deltas and the correction flag. It rejects an
inexact or overflowing collider rather than rounding it into a false success.
Movement and yaw of one object are compiled once from original source records.

Shared descriptors are detached into zeroed, unreferenced native slots; only
the selected grid reference changes. Multiple selected references receive
independent descriptors. Untouched references retain their original descriptor,
position, orientation, resources and collision. No TMD/TIM transplant is needed.

This does not establish ownership of the separate wall/floor grids. Existing
wall ownership, primary/fallback MAN binding and trigger checks remain active.
Receipt-tracked experimental imports remain blocked for yaw until angle-aware
provenance is implemented. Collider overlays explicitly reject native yaw
because their projected collider support is not implemented; no stale overlay
is represented as verified physics.

Yaw is scene-local. It is not propagated by matching grid coordinates to
other story-state MAPs. Existing legacy movement behavior is unchanged.

## Automated evidence and boundaries

Focused tests cover all four quarter rotations, corrected and uncorrected
colliders, pivot compensation, shared descriptor detachment, group identities,
unrelated-reference preservation, synthetic raw IMG readback, and rejection of
unowned walls, bindings, tilt and unrepresentable collision.

The controlled retail regression uses representative Retona and Vell fixtures.
It finds eligible objects by native predicates, applies an actual editor
rotation, runs the complete public export planner, writes a new raw IMG,
reparses it, reassembles the scene, and compares every selected render vertex
against the intended editor transform. It also compares unrelated PROT entries,
wall/floor/script bytes and unrelated grid references. These are automated
format/structural/reopening checks, not graphical or gameplay verification.

Initial probes in other cells were correctly blocked by unresolved wall,
height or script relationships. Their logs are retained; no guards were relaxed
to obtain these representative exports. An initial protected-path canonicalize
failure was resolved with authorized read access, without changing source data.

## Remaining M2 blockers

The exact six-object Rim Elm export still fails closed. Terrain 36's requested
half-tile relocation needs a 64-unit discontinuity plus retained bilinear
surfaces; the existing native kind-2 branch spans at most 48 units and duplicate
same-key rows are not subdivisions. A verified additional native sampler/data
capability is required; no original-engine patch has been implemented.

Terrain 36/37/43 wall contributions remain unresolved. Geometric proximity is
only a diagnostic and never establishes ownership. House placement 14 still
requires complete MAN/MOVE/animation, binding, script and collision closure.
The existing same-width MAN reference writer does not allocate an independent
house identity or prove story behavior. General cross-cell resource allocation,
terrain/floor ownership and other object categories remain incomplete.

Latest Build remains the user-verified M1.1 executable. New M2 artifacts are
separate candidates. No host LCK, DuckStation, desktop-control tool or VM was
launched. M2 GUI, sandbox GUI and native gameplay are not verified.

## Candidate evidence

Final controlled exports are in
`D:\AstraRimElmProject\ManualTests\M2_NativeYaw_20261027_06`:

- Retona: original placement index 3, native reference (37,36), selector 8.
- Vell: original placement index 0, native reference (12,7), selector 11.
- Each has a separate raw IMG and exact `.lckscene` request manifest.
- `MANUAL_TEST.md` gives user-run GUI/export/gameplay steps with new output paths.
- A separate release candidate is packaged under `bin`; Latest Build is unchanged.

The final suite passed 263 studio library tests and 9 integration tests; 23
controlled/external tests are excluded by default. The controlled two-cell
retail export/reopen/geometry regression and exact-six-request safe-rejection
regression were run explicitly and passed. The first restricted all-targets
attempt had 11 Windows access-denied failures in existing atomic filesystem
tests; the authorized rerun passed, with no validation disabled.

Logs: `target/m2-yaw-all-targets-authorized.log`, `target/m2-yaw-retail-06.log`,
`target/m2-yaw-six-request.log`, and `target/m2-yaw-release.log`.
