# LCK universal persistent renaming

## Preservation and checkpoint

The preceding six-file collision Properties milestone was preserved under `D:\AstraRimElmProject\Backups\LCK_Collision_Properties_Review_20261026_232025_b140adcb`, compared against its SHA-256 manifest, staged as an exact six-file list, and committed after human approval as `dd2d7cf7a02565c48e2eea6f2ceb74c5de35cb4e` (`LCK P1 - integrate collision controls into object Properties`). Naming changes described here form a separate checkpoint approved by the user after manual persistence verification. Branch remains `foundation-02`.

## Behavior and identity

- World Objects Rename uses a shared alias, independent of scene-instance overrides. Exact original TMD bytes, complete VRAM pixels and upload coverage form a versioned SHA-256 visual identity. Copies and matching resources across cells share aliases only when this identity matches. NPC aliases use verified scene/MAN/native-record identity rather than appearance guesses.
- Original placement/terrain overrides use CDNAME, original MAP SHA-256, native grid cell, category and descriptor identity. Original NPC overrides use CDNAME, decompressed MAN SHA-256 and MAN record. Render indices only adapt these identities to the loaded document; they are not the new persisted naming keys. Missing or ambiguous original identities disable new instance overrides instead of guessing.
- New/native copies and imported objects retain their existing persistent editor UIDs. Ctrl+D inherits the selected instance's override as an independent value. World Objects drops inherit the shared alias without copying an unrelated scene override. Undo/redo, scene restoration and subsequent rename remain independent.
- Display precedence is instance override, verified shared alias, legacy catalog identification, then native/default label. Hierarchy and Properties use the same resolver. Names and search terms participate in filtering. Native identifiers remain visible. Rename and Restore default are available through the existing naming dialogs/context menus. Cell names are also shown in repository grouping/search.
- Original and editor-copy cells keep CDNAME/source IDs and copy UIDs unchanged. Only display labels change. Renaming does not restart asset scanning, reload a scene or alter donor lookup.

## Persistence and failure behavior

- Global aliases: `D:\AstraRimElmProject\legaiasource\legend-of-legaia-re-main\LegaiaStudio_AssetCatalog.tsv`, existing V2 catalog. Legacy `LegaiaStudio_ObjectNames.tsv` import and catalog keys remain readable. Conflicting legacy aliases are not silently merged.
- Cell labels: `%LOCALAPPDATA%\LegaiaStudio\CellProjects\<source-path fingerprint>\cells.lckcatalog` (APPDATA fallback). Copy overlays remain `copy_<16-digit hexadecimal UID>.lckscene` beside that manifest. Existing source-path scoping is retained.
- Per-instance names: normal user-saved `.lckscene` files, or existing copy-cell overlays. New original names serialize as `NATIVE_NAME`; created/imported names retain `INSTANCE_NAME` UID rows. Existing `NAME` rows remain readable. Native scene names require Save LCK scene and reload for restart persistence; this change does not introduce original-scene autosave.
- Metadata writes use a sibling lock, expected-content conflict check, durable pending file, previous-file `.bak`, and a single replacement operation. A failed write leaves the prior destination in place. Catalog/cell state updates only after successful persistence. Stale copy allocation cannot overwrite another session's overlay. Scene saves require `.lckscene`, never an image extension.
- New display names are trimmed and limited to 80 printable characters / 256 UTF-8 bytes. Empty clears an override. Legacy escaped catalog data remains compatible. Generated copy labels truncate safely at Unicode character boundaries.

## Exact implementation scope

1. `crates/studio/src/app.rs`: common display resolution, Rename/default UI, searches, copy naming, atomic scene-save call and headless UI regression.
2. `crates/studio/src/app/naming.rs` (new): cached native/visual identities, material coverage fingerprint and alias restart tests.
3. `crates/studio/src/app/global_repository.rs`: shared alias metadata and conservative source identity integration.
4. `crates/studio/src/app/world_view.rs`: naming inventory/cache lifecycle, independent copied names and donor alias provenance.
5. `crates/studio/src/app/scene_instances.rs`: stable native-name extension, legacy compatibility, atomic scene save and history/persistence tests.
6. `crates/studio/src/app/cell_catalog.rs`: reset labels, atomic/conflict-safe cell persistence, stale-overlay protection and restart/Unicode tests.
7. `crates/studio/src/asset_catalog.rs`: shared name validation and metadata atomic writer; existing catalog compatibility retained.
8. `crates/studio/src/app/native_export_verification.rs`: read-only Rim Elm naming identity/copy/import roundtrip regression.
9. `crates/studio/src/app/ps1_export.rs`: test fixture initialization only, supplying the new default naming inventory. No export implementation changes.
10. `docs/tooling/lck-universal-persistent-renaming.md`: this report.

## Validation

- `cargo build -p legaia-studio --locked --message-format short`: passed. Existing float fallback, deprecated API, unused-code and filesystem hard-link warnings remain; no build failures.
- `cargo test -p legaia-studio --lib --locked --message-format short`: 170 passed, 0 failed, 8 ignored.
- Explicit read-only opt-in tests passed separately: `rim_elm_names_use_native_identity_and_verified_visual_aliases`, `rim_elm_ctrl_d_opens_generated_collision_properties`, and `unbound_candidate_reports_scene_wall_dependencies`.
- Rim Elm naming validation confirms every original rendered draw has an unambiguous stable naming identity, and original/copy/imported names roundtrip independently. Synthetic tests cover cross-cell alias reuse, different texture contexts, restart, native adapter reordering, terrain/NPC naming, restore defaults, Ctrl+D, history, atomic failure, and stale sessions.
- Existing collision Properties controls, automatic collision persistence, transactional restoration and native export rejection remain covered. The unbound-candidate test continues to reject unresolved native wall dependencies. No safety gate was relaxed.
- `git diff --check`: passed. No camera, viewport, renderer, native allocation, collision compilation or export algorithm edits. No Cargo manifests/dependencies changed. No deferred collision-box interactions added.
- Retail source SHA-256 remains `750dbcbadb70362feb3468467bbe7efbf3d151a414caa428c66c368d4b0b73e6`. Retail opened only for read-only checks. No IMG generated, emulator operated, branch changed, or push performed.

## Review and limitations

The user manually verified that Scene Hierarchy names, imported object names and global World Objects aliases persist after saving, closing and reopening LCK. Final checkpoint validation reran the library suite: 170 passed, 0 failed, 8 ignored. The sandbox initially denied temporary-file replacement in eight filesystem tests; running the unchanged suite with the required filesystem access passed.

For further review, run `D:\AstraRimElmProject\legaiasource\legend-of-legaia-re-main\target\debug\legaia-studio.exe`. Rename a World Objects asset, find it by its custom name, place/copy it, then rename only the instance through hierarchy or Properties. Save the LCK scene, restart, reload and confirm both naming scopes. Rename a native/copy cell and confirm source IDs remain unchanged. Clear an instance override to reveal the shared alias; restore the repository default separately. Additional interactive cell/default checks are not claimed as manually verified; automated UI tests use headless egui.

Whole-resource visual matching is intentionally conservative: changes anywhere in the texture context prevent alias sharing. NPC aliases do not merge actors across different native MAN identities. Legacy labels remain fallbacks. Moving the source image to a different path does not migrate the existing cell-project directory automatically. New Studio reads old scene files; older Studio versions may reject the new `NATIVE_NAME` extension. Naming does not add PS1-native duplication/collision capabilities or resolve unresolved script dependencies.

Approved checkpoint: the ten files listed above, committed as `LCK P1 - universal persistent asset and instance naming`. Exclude user-generated `LegaiaStudio_AssetCatalog.tsv`, historical untracked files and build outputs. No functionality changed during final checkpoint review.
