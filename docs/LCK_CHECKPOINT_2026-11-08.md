# Legaia Creation Kit - Editor Interaction Checkpoint

Date: 2026-11-08
Classification: Development checkpoint, not production release.

## Current implemented editor capabilities

- Game-wide World Objects and cell/scene browsing.
- Persistent editor projects, names and object transforms.
- Cross-cell static asset importing and duplication.
- Object, floor and collision multi-selection.
- Marquee and dual-mouse-button selection gestures.
- Persistent groups containing multiple component categories.
- Independent grouping for duplicated selections.
- Shared-center rotation for objects, floors and collision boxes.
- Right-click context menu gesture improvements.
- Empty-space click deselection with floating-window guards.
- Collision and floor visualization and editing.

## Native PS1 export status

Experimental native writing, collision, resources and texture
handling have progressed significantly.

However, editor functionality does not establish universal
native export compatibility or walkable collision for every
scene and object type.

Outstanding limitations include native terrain ownership,
walkability, NPC/MAN deletion, script and trigger dependencies,
and some mixed-resource runtime cases.

## Evidence and limitations

Source: current working local Rust editor snapshot.
Studio library tests: required to pass before Git publication.
Optimized Cargo build: required to pass before Git publication.
GUI interactions: tested incrementally by the project owner.
DuckStation gameplay: not performed by this Git checkpoint.

## Safety and Git strategy

- Derived from the existing lck/development history.
- No historical branches deleted or force-pushed.
- No retail IMG, ROM, save state or extracted Sony data added.
- Current snapshot published on its own reviewable branch.
- Existing default branch remains unchanged.

## Next milestones

1. Consolidate editor interaction and scene history handling.
2. Validate all group operations after save/reopen.
3. Finish native floor walkability and collision ownership.
4. Complete NPC/script/trigger serialization.
5. Expand gameplay verification for exported IMGs.
6. Prepare a reviewed merge into lck/development.