# Collision Properties integration

2026-10-26. Baseline `0a24a4a2`. No commit, image export or emulator operation.

## Findings

The existing right-click scene Properties window already called `collision_recipes::show_editor`, inside a collapsed **Authored collision recipe (in memory)** section. That entire section required ready scene data, world data and an open library. The viewport's separate **Selected mesh collider DRAFT** toggle drew current mesh triangles, not the saved generated definition. Properties had no local wireframe control and no regeneration action for an existing recipe.

Binary checks found the previous collision panel string in `target/debug/legaia-studio.exe`, but not in either `target/release/legaia-studio.exe` or the packaged `D:\AstraRimElmProject\legaia-tools-1.8.0-x86_64-pc-windows-gnu\legaia-studio.exe`. Before this run the release executable was dated 2026-10-26 14:25, and the packaged executable 2026-10-19. These are stale relative to the P1 UI. No running Studio executable could be identified, so the exact binary used for the user's observation is unconfirmed.

Recipe creation was intact. The read-only retail tests confirm town01 descriptor 230 at cell (94,78) receives a generated recipe through the actual Ctrl+D handler. Existing native-placement and raw-mesh repository creation paths also retain recipes. Loading an older scene without a recipe intentionally preserves that absence rather than silently regenerating it.

## Changes

- Properties always renders **Collision Properties (P1)**, saved generation status and provenance for the fixed Properties target, even without a library. Missing-library/source conditions are explicit warnings. Previously cached validation results are also visible above the editing section.
- **Show this object's generated collision wireframe** is separate from the native MAP wall toggle. Properties pins the preview to a stable created UID/native identity and scene name, without changing selection or the camera. The toolbar retains its selection-following draft mode.
- Saved generated geometry uses its existing transforms, face filtering and review band. Purple indicates steep candidates and green indicates unverified floor candidates. Rendering is cached and capped like the existing draft overlay. It neither changes native collision nor treats candidate floors as walkable. Manual grid corrections remain reviewed in the Properties grid; the geometry overlay does not claim to render the final corrected native wall union.
- **Collision authoring, regeneration and validation** opens the existing grid/text editor. Regeneration keeps saved blocking/nonblocking corrections and authored evidence by default. Clearing those corrections requires explicitly checking the replacement option and pressing the corresponding regeneration button. Ordinary Properties rendering never regenerates anything.
- The existing pending collision-edit/undo path handles regeneration. Visibility is session-only and creates no document/history edit.
- Properties remains resizable, with a nonshrinking bounded scroll area and a new per-object scroll identifier so an old globally remembered scroll position does not initially hide the new controls.

No double-click box selection, Delete-box removal or right-click box creation was added. Mesh-only donors expose saved geometry, provenance and preview; native-grid editing still requires verified MAP provenance. Native export checks, native wall ownership rejection and original MAP/floor handling are unchanged.

## Access and manual test

1. Run the newly built executable at `D:\AstraRimElmProject\legaiasource\legend-of-legaia-re-main\target\debug\legaia-studio.exe`. The older release/package copies were not overwritten.
2. Open the source disc and Rim Elm. Select an original environmental World Object and press Ctrl+D. Right-click the duplicate and choose **Properties...**.
3. Verify **Collision Properties (P1)** appears above the transform controls, with **Saved generated collision** and the source identity. If an older saved instance reports no recipe, expand the authoring section and explicitly generate its default.
4. Toggle the generated wireframe checkbox. It shows saved generated geometry for this Properties target without moving the camera. Toggle the original MAP **Collision wireframe** independently to compare the two sources.
5. Expand **Collision authoring, regeneration and validation** for manual painting, saved contribution validation and regeneration. Use **Save authored recipe** for grid/text corrections and normal Save LCK scene for disk persistence. Resize the window or scroll to reach all controls.
6. Regenerate with the replacement option off to retain saved corrections. Replacement is explicit. Unsaved grid/text drafts are applied separately with Save authored recipe. Undo/redo remains available through the existing editor history controls.

## Validation and scope

- `cargo test -p legaia-studio --lib --locked app::`: 127 passed, 0 failed, 7 opt-in tests ignored; 33 unrelated tests filtered out.
- `rim_elm_ctrl_d_opens_generated_collision_properties`: passed separately against read-only retail data. It injects the actual Ctrl+D shortcut (including egui's platform command alias), opens the actual Properties window, checks saved-recipe status, and verifies unchanged camera/document on opening.
- `unbound_candidate_reports_scene_wall_dependencies`: passed separately. Native placement and mesh-only repository defaults, persistence, rollback and continued native export rejection remain covered.
- A synthetic UI test clicks the Properties preview toggle and verifies unchanged camera, selection, document, native MAP visibility and undo/redo stacks, including the missing-library case. A regeneration test verifies corrections/evidence are retained unless explicit replacement is requested. Preview filtering remains covered by the geometry fixture.
- Build completion, final whitespace/binary checks and retail hash are recorded in the delivery response. Existing compiler warnings remain. Manual mouse/drag/resizing checks and emulator behavior were not performed.

Source files: `app.rs`, `app/collision_recipes.rs`, `app/collision_generation.rs`, `app/object_collision.rs`, and `app/native_export_verification.rs`. No changes to the native exporter, camera/viewport controls, scene serialization, texture rendering or original game resources. Changes remain uncommitted for review.
