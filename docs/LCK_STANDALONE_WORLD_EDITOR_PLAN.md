# LCK - Standalone World Editor

Status: PLANNED. This document does not claim implementation.

## Goal
Make the World Editor a separate Windows program, launched directly
into the current editor workspace, instead of a tab in Legaia Studio.

Preserve the original PS1-native engine target and existing assets,
collision, camera, selection, project, and export functionality.

## Existing structure
- crates/studio/Cargo.toml declares the legaia-studio binary.
- crates/studio/src/main.rs starts the combined Studio application.
- crates/studio/src/app.rs owns the Editor mode and related modules.
- Editor systems currently share Studio application state.

## Stage 1: Architecture and checkpoints
- Preserve a known-working code and native-export baseline.
- Audit editor dependencies on Studio browsing, settings, and startup.
- Include newer local marquee changes only after verification.

## Stage 2: Independent executable
- Add a legaia-world-editor binary.
- Give it its own title, app identity, and settings.
- Launch directly into the World Editor workspace.
- Preserve legaia-studio as a separate application.
- Reuse shared assets and PS1-native libraries.
- Do not duplicate the entire editor codebase.

## Stage 3: Decouple application state
- Separate editor startup and editor-owned application state.
- Gradually isolate viewport, scenes, selection, and asset repository.
- Preserve project formats, persistent names, collision journals,
  native export, unrestricted diagnostics, and undo/redo.

## Stage 4: Desktop menu bar (later)
File: New, Open, Save, Import, Export, Exit.
Edit: Undo, Redo, Duplicate, Delete, Select.
View: Grid, Height Level, Collision, Frame Selected.
Help: Shortcuts, About, Diagnostics.
Settings: Input, Camera, Appearance, Preferences.

## Acceptance
- Both Windows applications build and start independently.
- Existing editor tests remain passing.
- No retail IMG or existing project is overwritten.
- No loss of native functionality.
- User performs DuckStation gameplay testing.
- No unrelated branch cleanup during the executable refactor.

## Native engine limitations remain separately tracked
- Original collision ownership and terrain deletion.
- Full native animated/scripted portability.
- B2B combined-resource freezing.
- Static material coverage and walkable-ground authoring.

See LCK_WORKSTREAMS.md for detailed project status.