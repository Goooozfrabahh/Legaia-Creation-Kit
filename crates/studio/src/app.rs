//! The egui front end over [`Studio`].

mod actors_ui;
mod multi_selection;
#[cfg(test)]
mod multi_selection_workspace_tests;
mod media_ui;
mod viewport;
mod world_view;
mod scene_instances;
mod collision_recipes;
mod collision_authoring;
mod collision_generation;
mod naming;
mod native_dependency_report;
mod native_script_dependencies;
mod native_wall_ownership;
mod native_cell_inventory;
mod native_loader_trace;
mod native_load_budget;
mod native_wall_journal;
mod native_wall_export;
mod native_elevation_codec;
mod native_floor_patch;
pub mod native_extended_floor;
mod native_script_relocation;
#[cfg(test)]
mod native_retail_floor_tests;
mod native_asset_plan;
mod placement_drop;
mod cell_catalog;
mod ps1_export;
mod export_integration;
mod collision_wire;
mod floor_debug;
mod object_collision;
mod global_repository;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::Receiver;

use eframe::egui::{self, Color32, ColorImage, RichText, TextureHandle, TextureOptions};

use crate::analyze::Analysis;
use crate::asset_catalog::{Catalog, Confidence, Record};
use crate::extract_job::{ExtractJob, OutDir as ExtractOutDir, Outcome};
use crate::player::Player;
use crate::raster::{Model, Shading};
use crate::scene_vram::SceneVram;
use crate::{Source, Studio, export};
use viewport::Viewport;
use cell_catalog::{CellSelection, CellStore};

/// One framing path for the toolbar and F key: never remember an arbitrary
/// global camera yaw or frame around a different object. Resolve the original
/// PS1 placement orientation (including tilt), then layer the selected editor
/// instance's rotation, center and scale on top.
fn frame_world_selection(
    viewport: &mut Viewport,
    world: &world_view::WorldModel,
    scene: &legaia_engine_scene::scene_assembly::AssembledScene,
    selected: Option<(bool, usize)>,
) {
    let (Some(model), Some((terrain, index))) = (world.model.as_ref(), selected) else { return; };
    let Some(draw) = world.at(terrain, index).filter(|d| !d.hidden && d.radius > 0.0)
        else { return; };
    // A duplicate inherits its native object's baked orientation. New editor
    // rotation is applied separately, rather than interpreting it as native.
    let (source_terrain, source_index) = draw.source_ref.unwrap_or((terrain, index));
    let native = if source_terrain {
        scene.terrain.get(source_index)
    } else {
        scene.placements.get(source_index)
    };
    let original_quat = native
        .map(legaia_engine_scene::scene_assembly::draw_rotation_quat)
        .unwrap_or([0.0, 0.0, 0.0, 1.0]);
    let radius = draw.radius * draw.scale.iter().copied().fold(1.0_f32, f32::max);
    viewport.frame_instance(model, draw.center, radius, original_quat, draw.rotation);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Browse,
    Actors,
    Media,
    Extract,
    Editor,
}

impl Mode {
    const ALL: [Mode; 5] = [Mode::Browse, Mode::Actors, Mode::Media, Mode::Extract, Mode::Editor];

    /// The settings / `LEGAIA_STUDIO_TAB` spelling.
    fn key(self) -> &'static str {
        match self {
            Mode::Browse => "browse",
            Mode::Actors => "actors",
            Mode::Media => "media",
            Mode::Extract => "extract",
            Mode::Editor => "editor",
        }
    }

    fn parse(s: &str) -> Option<Mode> {
        Mode::ALL
            .into_iter()
            .find(|m| m.key().eq_ignore_ascii_case(s.trim()))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Overview,
    Textures,
    Model,
    Audio,
    Text,
    Hex,
}

impl Tab {
    const ALL: [Tab; 6] = [
        Tab::Overview,
        Tab::Textures,
        Tab::Model,
        Tab::Audio,
        Tab::Text,
        Tab::Hex,
    ];

    fn name(self) -> &'static str {
        match self {
            Tab::Overview => "Overview",
            Tab::Textures => "Textures",
            Tab::Model => "3D model",
            Tab::Audio => "Audio",
            Tab::Text => "Text",
            Tab::Hex => "Hex",
        }
    }

    /// The settings / `LEGAIA_STUDIO_TAB` spelling.
    fn key(self) -> &'static str {
        match self {
            Tab::Overview => "overview",
            Tab::Textures => "textures",
            Tab::Model => "model",
            Tab::Audio => "audio",
            Tab::Text => "text",
            Tab::Hex => "hex",
        }
    }

    fn parse(s: &str) -> Option<Tab> {
        Tab::ALL
            .into_iter()
            .find(|&t| t.key().eq_ignore_ascii_case(s.trim()))
    }
}

/// Scripted-run hooks, read from the environment once at start-up. They let
/// a test (or a bug report) open a source, pick an entry and a tab, and save
/// a screenshot without anyone at the keyboard.
#[derive(Debug, Default, Clone)]
pub struct Automation {
    /// `LEGAIA_STUDIO_SELECT`: PROT index to select once loaded.
    pub select: Option<u32>,
    /// `LEGAIA_STUDIO_TMD`: which TMD of the selected entry the 3D model
    /// tab shows (the `TMD #` picker), instead of the first.
    pub tmd: Option<usize>,
    /// `LEGAIA_STUDIO_TAB`: preview tab (`textures`, `model`, `audio`, ...)
    /// or `extract`.
    pub tab: Option<String>,
    /// `LEGAIA_STUDIO_EXTRACT_TO`: start an extraction into this folder.
    pub extract_to: Option<PathBuf>,
    /// `LEGAIA_STUDIO_SCREENSHOT`: save a PNG of the window once the
    /// scripted state has settled (after the extraction, when one runs),
    /// then quit.
    pub screenshot: Option<PathBuf>,
    /// `LEGAIA_STUDIO_EXPORT_DIR`: export without a save dialog, into this
    /// folder.
    pub export_dir: Option<PathBuf>,
    /// `LEGAIA_STUDIO_ACTOR`: open the Actors view on `monster:<id>`,
    /// `char:<slot>` (0 Vahn .. 3 Terra), optionally `@<clip>:<frame>`.
    pub actor: Option<String>,
    /// `LEGAIA_STUDIO_MEDIA`: open the Movies & voice view on this ISO path
    /// (`MOV/MV1.STR`), optionally `@<frame>`.
    pub media: Option<String>,
    /// `LEGAIA_STUDIO_SEARCH`: start with this text in the archive search.
    pub search: Option<String>,
    /// `LEGAIA_STUDIO_PLAY`: press Play once the disc has loaded.
    pub play: bool,
}

impl Automation {
    pub fn from_env() -> Self {
        let path = |k: &str| std::env::var_os(k).map(PathBuf::from);
        Self {
            select: std::env::var("LEGAIA_STUDIO_SELECT")
                .ok()
                .and_then(|s| s.trim().parse().ok()),
            tmd: std::env::var("LEGAIA_STUDIO_TMD")
                .ok()
                .and_then(|s| s.trim().parse().ok()),
            tab: std::env::var("LEGAIA_STUDIO_TAB").ok(),
            extract_to: path("LEGAIA_STUDIO_EXTRACT_TO"),
            screenshot: path("LEGAIA_STUDIO_SCREENSHOT"),
            export_dir: path("LEGAIA_STUDIO_EXPORT_DIR"),
            actor: std::env::var("LEGAIA_STUDIO_ACTOR").ok(),
            media: std::env::var("LEGAIA_STUDIO_MEDIA").ok(),
            search: std::env::var("LEGAIA_STUDIO_SEARCH").ok(),
            play: std::env::var_os("LEGAIA_STUDIO_PLAY").is_some_and(|v| !v.is_empty()),
        }
    }
}

/// How long an informational status line ("... written to ...") stays up.
/// Errors stay until dismissed.
const STATUS_TTL: std::time::Duration = std::time::Duration::from_secs(8);

/// Which VRAM image the entry model view samples.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TexSource {
    /// Only the TIMs inside the selected entry.
    Entry,
    /// The VRAM the engine builds for the entry's scene (CDNAME block).
    Scene,
}

/// Native scene assembly; never derived from the exported GLB.
enum NativeSceneSlot {
    Loading(Receiver<Result<(legaia_engine_scene::scene_assembly::AssembledScene, world_view::WorldModel), String>>),
    Ready(Arc<legaia_engine_scene::scene_assembly::AssembledScene>, Arc<world_view::WorldModel>),
    Failed(String),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum EditorMoveMode { Fluid, Grid }

#[derive(Clone, Copy)]
struct EditorDrag {
    key: (bool, usize),
    pointer_start: egui::Pos2,
    offset_start: [f32; 3],
}

#[derive(Clone, Copy)]
struct EditorTransformEdit {
    key: (bool, usize),
    before: [f32; 3],
    after: [f32; 3],
}

#[derive(Clone, Copy)]
struct EditorRotateDrag {
    key: (bool, usize),
    pointer_start: egui::Pos2,
    rotation_start: [f32;3],
    /// Plain right-drag is Y (yaw), Alt+right-drag is X (pitch).
    pitch_axis: bool,
}

#[derive(Clone, Copy)]
struct EditorRotationEdit {
    key: (bool,usize),
    before: [f32;3],
    after: [f32;3],
}

#[derive(Clone, PartialEq)]
struct EditorSceneHistory {
    /// Full editor-only scene state: undo can restore deleted duplicates too.
    overlay: String,
    /// Stable selection identity, unaffected by recreated duplicate indices.
    selected: Option<scene_instances::NameTarget>,
    selection: Vec<scene_instances::NameTarget>,
}

fn editor_history_capture(world: &world_view::WorldModel, scene_name: &str,
                          selected: Option<(bool, usize)>) -> EditorSceneHistory {
    let selected=selected.and_then(|key|world.at(key.0,key.1).map(|draw|
        match draw.editor_uid {
            Some(uid) => scene_instances::NameTarget::Created(uid),
            None => scene_instances::NameTarget::Native(key.0,key.1),
        }));
    EditorSceneHistory {overlay: scene_instances::serialize(scene_name,world), selection: selected.iter().cloned().collect(), selected}
}

fn editor_history_capture_group(world: &world_view::WorldModel, name: &str,
    primary: Option<(bool,usize)>, selection: &std::collections::BTreeSet<(bool,usize)>) -> EditorSceneHistory {
    let mut history=editor_history_capture(world,name,primary);
    history.selection=multi_selection::keys(primary,selection).into_iter().filter_map(|key|
        world.at(key.0,key.1).map(|draw|match draw.editor_uid {
            Some(uid)=>scene_instances::NameTarget::Created(uid),
            None=>scene_instances::NameTarget::Native(key.0,key.1),
        })).collect();
    history
}

fn editor_history_selected(world: &world_view::WorldModel,
                           target: &scene_instances::NameTarget) -> Option<(bool,usize)> {
    match target {
        scene_instances::NameTarget::Native(t,i)=>Some((*t,*i)),
        scene_instances::NameTarget::Created(uid)=>world.draws.iter()
            .find(|d|d.editor_uid==Some(*uid)).map(|d|(d.terrain,d.index)),
    }
}

/// Rotation is in radians and normalized to avoid unbounded angular values
/// after prolonged dragging. Prevents NaNs reaching the CPU rasterizer.
fn editor_rotate_drag(start: [f32;3], pixels: egui::Vec2, pitch_axis: bool, mirror: bool) -> [f32;3] {
    let mut result=start;
    let axis=if pitch_axis {0} else {1};
    let delta=if pitch_axis { pixels.y } else if mirror { -pixels.x } else { pixels.x };
    let angle=start[axis] + delta*0.01;
    if angle.is_finite() {
        result[axis]=(angle + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU)-std::f32::consts::PI;
    }
    result
}

/// GECK Phase 1: session clipboard stores RELATIVE editor offsets and local
/// Euler rotation/scale, never original PS1/MAP records or absolute anchors.
#[derive(Clone, Copy)]
struct GeckTransformClipboard {
    offset: [f32; 3],
    rotation: [f32; 3],
    scale: [f32; 3],
}

/// Snap only the actively dragged Euler axis; existing native placement
/// rotations and the camera's upright/Frame Selected orientation are untouched.
fn geck_snap_euler_axis(mut value: [f32; 3], axis: usize, degrees: f32) -> [f32; 3] {
    if axis < 3 && degrees.is_finite() && degrees > 0.0 && value[axis].is_finite() {
        let step = degrees.to_radians();
        value[axis] = ((value[axis] / step).round() * step
            + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU)
            - std::f32::consts::PI;
    }
    value
}

#[cfg(test)]
mod geck_phase1_tests {
    use super::geck_snap_euler_axis;
    #[test]
    fn angle_snap_is_axis_specific_and_off_is_identity() {
        let input = [0.33_f32, 14.8_f32.to_radians(), -0.44];
        assert_eq!(geck_snap_euler_axis(input, 1, 0.0), input);
        let result = geck_snap_euler_axis(input, 1, 15.0);
        assert!((result[1].to_degrees() - 15.0).abs() < 0.001);
        assert_eq!(result[0], input[0]);
        assert_eq!(result[2], input[2]);
    }
}

/// Stable references to objects in the indexed native asset catalogue.
/// Source-only archive rows are deliberately NOT advertised as placeable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EditorCatalogItem {
    Draw(bool, usize),
    Mesh(usize),
    Character(usize),
    Monster(u16),
    Archive(usize),
}

struct EditorRename {
    key: (bool, usize),
    text: String,
}

enum EditorPreview {
    Mesh(Model),
    Actor(crate::actors::Actor),
    Unavailable(String),
}

/// A single on-demand donor scene is cached for viewing its authentic TMD
/// and source VRAM; never uses the destination scene's texture atlas.
struct GlobalAssetPreview {
    id: usize,
    scene: Option<Arc<legaia_engine_scene::scene_assembly::AssembledScene>>,
    model: Option<Model>,
    issue: Option<String>,
}


/// Extract only the triangles belonging to one native draw. The main
/// assembly, its source PS1 vertices and the active world view stay untouched.
fn editor_isolate_draw(model: &Model, id: u32) -> Option<Model> {
    let mut out = Model {
        positions: Vec::new(), rest: Vec::new(), object_ids: Vec::new(),
        uvs: Vec::new(), cba_tsb: Vec::new(), modulation: Vec::new(),
        fill: Vec::new(), textured: Vec::new(), indices: Vec::new(),
        center: [0.0; 3], radius: 1.0,
    };
    for tri in model.indices.chunks_exact(3) {
        let i0 = tri[0] as usize;
        if model.object_ids.get(i0) != Some(&id) { continue; }
        if tri.iter().any(|&j| j as usize >= model.positions.len()) { continue; }
        for &idx in tri {
            let i = idx as usize;
            let Ok(n) = u32::try_from(out.positions.len()) else { return None; };
            out.indices.push(n);
            out.positions.push(model.positions[i]);
            out.uvs.push(model.uvs.get(i).copied().unwrap_or([0, 0]));
            out.cba_tsb.push(model.cba_tsb.get(i).copied().unwrap_or([0, 0]));
            out.modulation.push(model.modulation.get(i).copied().unwrap_or([128; 3]));
            out.fill.push(model.fill.get(i).copied().unwrap_or([180; 3]));
            out.textured.push(model.textured.get(i).copied().unwrap_or(0));
            out.object_ids.push(0);
        }
    }
    if out.indices.is_empty() { return None; }
    out.rest = out.positions.clone();
    out.refit();
    Some(out)
}

/// Build a preview from an original placed TMD using precisely the same
/// native MAP placement rotation, position, and PS1-down-Y -> viewer-up-Y
/// conversion used by the world renderer.  This is a copy only: it cannot
/// rotate or rewrite the donor TMD, destination cell, or the exported IMG.
fn editor_preview_placed_tmd(
    scene: &legaia_engine_scene::scene_assembly::AssembledScene,
    kind: &str,
    record: usize,
) -> Option<Model> {
    let (mut mesh, display_points) =
        world_view::foreign_original_geometry(scene, kind, record).ok()?;
    editor_preview_positions(&mut mesh, display_points)?;
    Some(mesh)
}

/// A free-standing reusable TMD has no intrinsic "up" metadata.  Prefer a
/// real in-game placement of that SAME resource as the reference orientation
/// (some source models require native 180-degree placement rotations).
/// If the source TMD is never placed in its cell, preserve native geometry
/// except for the confirmed PS1 Y-down -> editor Y-up coordinate conversion.
fn editor_preview_reusable_tmd(
    scene: &legaia_engine_scene::scene_assembly::AssembledScene,
    slot: usize,
) -> Option<Model> {
    let resource = *scene.env_tmds.get(slot)?;
    if let Some((index, _)) = scene.placements.iter().enumerate()
        .find(|(_, draw)| draw.res_tmd == resource)
    {
        if let Some(mesh) = editor_preview_placed_tmd(scene, "placement", index) {
            return Some(mesh);
        }
    }
    if let Some((index, _)) = scene.terrain.iter().enumerate()
        .find(|(_, draw)| draw.res_tmd == resource)
    {
        if let Some(mesh) = editor_preview_placed_tmd(scene, "terrain", index) {
            return Some(mesh);
        }
    }
    let mut mesh = Model::from_tmd(&scene.res.tmds.get(resource)?.raw)?;
    let points = mesh.positions.iter().map(|p| [p[0], -p[1], p[2]]).collect();
    editor_preview_positions(&mut mesh, points)?;
    Some(mesh)
}

/// Transform vertex copies and restore consistent rest pose / bounds.
/// The common camera must remain upright; it does not compensate for
/// individual native TMDs having PS1-down-Y or nonzero placement rotation.
fn editor_preview_positions(mesh: &mut Model, points: Vec<[f32; 3]>) -> Option<()> {
    if points.is_empty() || points.len() != mesh.positions.len() ||
       points.iter().any(|p| p.iter().any(|v| !v.is_finite())) {
        return None;
    }
    mesh.positions = points;
    mesh.rest = mesh.positions.clone();
    mesh.refit();
    Some(())
}

/// Cross-panel repository gesture capture. Egui's ScrollArea may claim the
/// drag before a row's Response::drag_started; capture the PRIMARY PRESS on
/// the row rect immediately, then maintain the asset in editor state.
fn catalog_pointer_pressed_on(ui: &egui::Ui, rect: egui::Rect) -> bool {
    ui.input(|i| i.pointer.primary_pressed() &&
        i.pointer.interact_pos().is_some_and(|p| rect.contains(p)))
}

/// One indexed subgroup: the search filters locally and only expanded groups
/// draw their rows; opening a context menu never changes the camera.
fn editor_catalog_group(ui: &mut egui::Ui, title: &str,
                        rows: &[(String, EditorCatalogItem)], filter: &str,
                        selected: Option<(bool, usize)>,
                        catalog_active: Option<EditorCatalogItem>,
                        chosen: &mut Option<(EditorCatalogItem, bool)>,
                        clicked: &mut Option<EditorCatalogItem>,
                        drag_started: &mut Option<EditorCatalogItem>) {
    let term = filter.trim().to_lowercase();
    let matches = rows.iter().filter(|(label, _)| term.is_empty() ||
        label.to_lowercase().contains(&term)).count();
    if matches == 0 { return; }
    egui::CollapsingHeader::new(format!("{title} ({matches})"))
        .id_salt(("editor_catalog_group", title))
        .default_open(title == "Placed objects")
        .show(ui, |ui| {
            for (label, item) in rows {
                if !term.is_empty() && !label.to_lowercase().contains(&term) { continue; }
                let active = match item {
                    EditorCatalogItem::Draw(terrain, index) =>
                        catalog_active == Some(*item) || selected == Some((*terrain, *index)),
                    _ => catalog_active == Some(*item),
                };
                // Capture drag intent at MOUSE DOWN rather than waiting for
                // response.drag_started(). A parent ScrollArea can claim the
                // scroll gesture first, preventing cross-panel drag_started.
                // The global primary-press state is independent of widget capture.
                let placeable = matches!(item,
                    EditorCatalogItem::Draw(..) | EditorCatalogItem::Mesh(..));
                let display = if placeable {
                    format!("↳  {label}")
                } else { label.clone() };
                // Non-selectable label: Windows should never highlight letters
                // while the artist drags a complete repository record.
                let response = ui.add_sized(
                    egui::vec2(ui.available_width().max(40.0), 23.0),
                    egui::Label::new(display).selectable(false)
                        .sense(egui::Sense::click_and_drag()));
                if active || response.hovered() {
                    let stroke=if active { Color32::from_rgb(255,196,66) }
                        else { Color32::from_rgb(105,146,198) };
                    ui.painter().rect_stroke(response.rect, egui::CornerRadius::same(3),
                        egui::Stroke::new(if active { 2.0 } else { 1.0 }, stroke),
                        egui::StrokeKind::Inside);
                }
                // Use the press point, not the row's drag ownership: this
                // remains reliable while crossing egui panels.
                if placeable && (response.drag_started() ||
                    catalog_pointer_pressed_on(ui, response.rect)) {
                    *drag_started=Some(*item);
                }
                if response.clicked() { *clicked = Some(*item); }
                let response=if placeable {
                    response.on_hover_cursor(egui::CursorIcon::Grab)
                        .on_hover_text("Drag this whole asset into the viewport. Drop anywhere at its F framing distance; hold Shift for ground snap.")
                } else { response };
                response.context_menu(|ui| {
                    if ui.button("Properties").clicked() {
                        *chosen = Some((*item, false)); ui.close();
                    }
                    if ui.button("Preview").clicked() {
                        *chosen = Some((*item, true)); ui.close();
                    }
                });
            }
        });
}

/// Use the same duplicate parameters for the toolbar button and Ctrl+D.
/// Copies always point back to the original PS1 mesh, but inherit the selected
/// instance's editor-only offset before being moved +128 units along X.
fn editor_duplicate_request(
    world: &world_view::WorldModel,
    selected: Option<(bool, usize)>,
) -> Option<((bool, usize), [f32; 3], [f32; 3], Option<(u8,u8,usize)>)> {
    let key = selected?;
    let draw = world.at(key.0, key.1)?;
    if draw.hidden || draw.vertex_start==draw.vertex_end { return None; }
    let source = draw.source_ref.unwrap_or(key);
    let mut offset = draw.offset;
    offset[0] += 128.0;
    let actor=world.npc(key).map(|n|(n.model_index,n.clip,n.frame));
    Some((source, offset, draw.rotation,actor))
}

/// The native-scene hierarchy uses its own context menu: unlike the asset
/// browser's names, these rename individual scene instances only.
fn editor_hierarchy_row(ui: &mut egui::Ui, label: String,
    key: (bool,usize), selected: &mut Option<(bool,usize)>,
    selection: &mut std::collections::BTreeSet<(bool,usize)>,
    chosen: &mut Option<(EditorCatalogItem,bool)>,
    rename: &mut Option<(bool,usize)>, input:&mut multi_selection::HierarchyInput) -> egui::Rect {
    input.rows.push(key);
    let response = ui.selectable_label(multi_selection::keys(*selected,selection).contains(&key), label);
    if response.clicked() {input.click=Some((key,ui.input(|i|i.modifiers),false));}
    if response.secondary_clicked() {input.click=Some((key,ui.input(|i|i.modifiers),true));}
    response.context_menu(|ui| {
        if ui.button("Properties").clicked() {
            *chosen=Some((EditorCatalogItem::Draw(key.0,key.1),false)); ui.close();
        }
        if ui.button("Preview").clicked() {
            *chosen=Some((EditorCatalogItem::Draw(key.0,key.1),true)); ui.close();
        }
        if ui.button("Rename").clicked() { *rename=Some(key); ui.close(); }
    });
    response.rect
}

/// One editor cell is selected at a time; context menu operations are
/// deferred so they never mutate or invalidate the catalog while iterating.
fn editor_cell_row(ui: &mut egui::Ui, id: CellSelection, label: String,
    selected: &mut Option<CellSelection>, focus_cell: &mut bool,
    choose: &mut Option<CellSelection>, duplicate: &mut Option<CellSelection>,
    rename: &mut Option<CellSelection>, properties: &mut Option<CellSelection>) {
    let response=ui.selectable_label(selected.as_ref()==Some(&id),label);
    if response.clicked() { *selected=Some(id.clone()); *choose=Some(id.clone()); *focus_cell=true; }
    if response.secondary_clicked() { *selected=Some(id.clone()); *focus_cell=true; }
    response.context_menu(|ui| {
        if ui.button("Properties").clicked() { *properties=Some(id.clone()); ui.close(); }
        if ui.button("Duplicate (Ctrl+D)").clicked() { *duplicate=Some(id.clone()); ui.close(); }
        if ui.button("Rename").clicked() { *rename=Some(id); ui.close(); }
    });
}

/// Perspective-aware screen drag, projected onto the horizontal XZ plane.
/// Ctrl+left-drag instead moves on the vertical axis. Flip the view's horizontal
/// drag only in the editor's presentation layer, as v0.4.3.1 already does.
fn editor_drag_offset(start: [f32; 3], pixels: egui::Vec2,
                      cam: crate::raster::Camera, radius: f32,
                      view_height: f32, mirror: bool, vertical: bool) -> [f32; 3] {
    let half_fov = 45f32.to_radians() * 0.5;
    let distance = radius / half_fov.sin() * cam.zoom.max(0.0000001);
    let unit = (2.0 * distance * half_fov.tan() / view_height.max(1.0)).clamp(0.00001, 20000.0);
    if vertical { return [start[0], start[1] - pixels.y * unit, start[2]]; }
    let (sy, cy) = cam.yaw.sin_cos();
    let x = if mirror { -pixels.x } else { pixels.x };
    let pitch_sign = if cam.pitch.sin() < 0.0 { -1.0 } else { 1.0 };
    [start[0] + unit * (x * cy + pixels.y * sy * pitch_sign),
     start[1],
     start[2] + unit * (x * sy - pixels.y * cy * pitch_sign)]
}

// Sidecar format is deliberately plain text and NOT a PS1 native edit.
// It can be reloaded into the preview without changing the original disc.
fn serialize_editor_offsets(scene_name: &str, world: &world_view::WorldModel) -> String {
    let mut data = format!("LEGAIA_OFFSETS_V1\t{scene_name}\n");
    for d in &world.draws {
        if d.editor_uid.is_none() && d.offset != [0.0; 3] {
            data.push_str(&format!("{}\t{}\t{}\t{}\t{}\n",
                if d.terrain { "terrain" } else { "placement" }, d.index,
                d.offset[0], d.offset[1], d.offset[2]));
        }
    }
    data
}

fn parse_editor_offsets(text: &str, scene_name: &str,
                        world: &world_view::WorldModel) -> Result<Vec<((bool,usize),[f32;3])>, String> {
    let mut lines = text.lines();
    let header = lines.next().ok_or("Empty placement file")?;
    if header != format!("LEGAIA_OFFSETS_V1\t{scene_name}") {
        return Err(format!("Placement file belongs to another scene or format (expected {scene_name})"));
    }
    let mut seen = std::collections::HashSet::new();
    let mut edits = Vec::new();
    for (number, line) in lines.enumerate() {
        if line.trim().is_empty() { continue; }
        let parts: Vec<&str> = line.split('\t').collect();
        if parts.len() != 5 { return Err(format!("Malformed placement row {}", number+2)); }
        let terrain = match parts[0] { "terrain"=>true, "placement"=>false,
            _=>return Err(format!("Invalid object type at row {}", number+2)) };
        let index: usize = parts[1].parse().map_err(|_| format!("Invalid object index at row {}", number+2))?;
        if world.at(terrain,index).is_none() { return Err(format!("No drawable object at row {}",number+2)); }
        if !seen.insert((terrain,index)) { return Err(format!("Duplicate object at row {}",number+2)); }
        let mut values=[0.0; 3];
        for (k, item) in parts[2..].iter().enumerate() {
            values[k]=item.parse::<f32>().map_err(|_| format!("Invalid coordinate at row {}", number+2))?;
        }
        if !values.iter().all(|v| v.is_finite() && v.abs() < 1_000_000.0) {
            return Err(format!("Out-of-range coordinate at row {}", number+2));
        }
        edits.push(((terrain,index),values));
    }
    Ok(edits)
}

struct ModelView {
    key: (usize, usize),
    model: Option<Model>,
    entry_vram: legaia_tim::Vram,
    source: TexSource,
    /// The scene-cache key the view last drew with (`None` = entry VRAM).
    drawn_with: Option<String>,
    view: Viewport,
}

/// A scene VRAM build: in flight on a worker, or done.
enum SceneSlot {
    Building(Receiver<Result<SceneVram, String>>),
    Ready(Arc<SceneVram>),
    Failed(String),
}


/// Project-side metadata stays beside the source project; legacy names are imported.
fn editor_catalog_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..").join("..").join("LegaiaStudio_AssetCatalog.tsv")
}
fn editor_legacy_names_path() -> PathBuf {
    editor_catalog_path().with_file_name("LegaiaStudio_ObjectNames.tsv")
}

fn editor_mesh_name_key(scene: &legaia_engine_scene::scene_assembly::AssembledScene,
                        slot: usize) -> Option<String> {
    let res_tmd = *scene.env_tmds.get(slot)?;
    Some(format!("cell:{}|mesh:{}|resource:{}", scene.name, slot, res_tmd))
}

fn editor_item_name_key(scene: Option<&legaia_engine_scene::scene_assembly::AssembledScene>,
                        item: EditorCatalogItem) -> Option<String> {
    match item {
        EditorCatalogItem::Mesh(slot) => editor_mesh_name_key(scene?, slot),
        EditorCatalogItem::Draw(false,index) if index >= world_view::NPC_KEY_BASE =>
            Some(format!("cell:{}|npc:man:{}",scene?.name,index-world_view::NPC_KEY_BASE)),
        EditorCatalogItem::Draw(terrain, index) => {
            let scene = scene?;
            let draw = (if terrain { &scene.terrain } else { &scene.placements }).get(index)?;
            Some(format!("cell:{}|{}:{}|mesh:{}|anchor:{},{}|pos:{},{},{}",
                scene.name, if terrain { "terrain" } else { "placed" },
                index, draw.env_slot, draw.anchor.0, draw.anchor.1,
                draw.world_x, draw.world_y, draw.world_z))
        }
        EditorCatalogItem::Character(i) => Some(format!("actor:character:{i}")),
        EditorCatalogItem::Monster(id) => Some(format!("actor:monster:{id}")),
        EditorCatalogItem::Archive(pos) => Some(format!("source:archive:{pos}")),
    }
}

/// Native-semantic fallback, NOT an assertion about the visual object type.
fn editor_native_item_hint(scene: Option<&legaia_engine_scene::scene_assembly::AssembledScene>,
                           item: EditorCatalogItem) -> String {
    match item {
        EditorCatalogItem::Draw(terrain, i) => {
            if !terrain && i >= world_view::NPC_KEY_BASE {
                return format!("Unidentified field actor MAN #{}",i-world_view::NPC_KEY_BASE);
            }
            let Some(draw) = scene.and_then(|s| (if terrain { &s.terrain } else { &s.placements }).get(i)) else {
                return "Unidentified object".to_owned();
            };
            if terrain {
                if draw.view_skip != 0 { "Camera-facing decoration".to_owned() }
                else { "Terrain / environment panel".to_owned() }
            } else if draw.anim_id != 0 {
                "Animated environmental object".to_owned()
            } else {
                "Unidentified world object".to_owned()
            }
        }
        EditorCatalogItem::Mesh(_) => "Unidentified reusable mesh".to_owned(),
        EditorCatalogItem::Character(i) =>
            ["Vahn", "Noa", "Gala", "Terra"].get(i).copied().unwrap_or("Character").to_owned(),
        EditorCatalogItem::Monster(id) => format!("Monster #{id}"),
        EditorCatalogItem::Archive(_) => "Source resource".to_owned(),
    }
}

pub struct App {
    pub studio: Studio,
    mode: Mode,
    tab: Tab,
    native_scene: Option<(String, NativeSceneSlot)>,
    pending_native_scene: Option<(String, CellSelection, NativeSceneSlot)>,
    native_selection: std::collections::BTreeSet<(bool,usize)>,
    native_selected: Option<(bool, usize)>, // false = placement; true = terrain
    native_hierarchy_anchor: Option<(bool,usize)>,
    native_selection_click: Option<((bool,usize),egui::Pos2,bool)>,
    native_view: Viewport,
    native_ground_visible: bool,
    /// Show native field-wall collision without altering editor/game data.
    native_collision_wire_visible: bool,
    native_floor_debug_visible: bool,
    native_floor_debug: Option<Result<floor_debug::DebugFloor,String>>,
    native_floor_debug_key: String,
    /// Inspect this specific object's transformed mesh as a collision draft.
    /// Unlike the native MAP overlay, this is NOT in-game collision data.
    object_collision_draft_visible: bool,
    /// Properties preview uses a stable UID and never changes selection.
    generated_collision_target: Option<(String,scene_instances::NameTarget)>,
    native_collision_grid: Option<collision_wire::CollisionGrid>,
    /// Stage Q5 Fix 2: cache native donor MAP/TMD collision preflight by scene
    /// and complete LCK placement overlay. Never re-decode a donor every frame.
    native_collision_preview_cache: Option<(String, Result<ps1_export::ExportEdits,String>)>,
    native_move_mode: EditorMoveMode,
    native_grid_size: f32,
    native_anchor_snap: bool,
    /// None = free X/Z dragging; Some(0) = X-only; Some(2) = Z-only.
    geck_drag_axis: Option<usize>,
    /// Zero disables angle snap; degrees are converted to radians in the drag code.
    geck_angle_snap: f32,
    geck_transform_clipboard: Option<GeckTransformClipboard>,
    /// Session-only camera bookmarks: no changes to original scene or native IMG.
    geck_camera_bookmarks: [Option<crate::raster::Camera>; 3],
    native_drag_group: Vec<((bool,usize),[f32;3])>,
    native_drag: Option<EditorDrag>,
    native_rotate: Option<EditorRotateDrag>,
    native_rotate_undo: Vec<EditorRotationEdit>,
    native_rotate_redo: Vec<EditorRotationEdit>,
    editor_undo: Vec<EditorSceneHistory>,
    editor_redo: Vec<EditorSceneHistory>,
    editor_gesture_start: Option<EditorSceneHistory>,
    /// Animation previews change only the selected scene's raster vertices.
    npc_playing: Option<(bool,usize)>,
    npc_next_tick: f64,
    native_undo: Vec<EditorTransformEdit>,
    native_redo: Vec<EditorTransformEdit>,
    editor_search: String,
    /// Entire-disc original asset inventory, independent of the active cell.
    global_repository: Vec<global_repository::Asset>,
    global_repository_rx: Option<Receiver<global_repository::Batch>>,
    global_repository_started: bool,
    global_repository_completed: usize,
    global_repository_total: usize,
    global_repository_failures: usize,
    global_repository_drag: Option<usize>,
    global_repository_selected: Option<usize>,
    /// (global ID, preview=true / properties=false)
    global_repository_dialog: Option<(usize,bool)>,
    global_repository_preview: Option<GlobalAssetPreview>,
    global_repository_preview_view: Viewport,
    global_name_edit_key: Option<String>,
    global_name_draft: Record,
    global_name_status: Option<String>,
    /// Explicit single-open accordion. Nothing expanded when LCK starts.
    global_open_category: Option<usize>,
    global_open_branch: Option<bool>, // world objects: towns or other scenes
    global_open_scene: Option<String>,
    /// Independent left-pane cell browser (separate scroll and drag-to-resize splitter).
    editor_cell_search: String,
    editor_cells: CellStore,
    editor_cell_selected: Option<CellSelection>,
    editor_cell_active: Option<CellSelection>,
    editor_cell_rename: Option<(CellSelection, String)>,
    editor_cell_properties: Option<CellSelection>,
    editor_cell_last_autosave: f64,
    /// Background native test-image export; no edits to the currently open image.
    ps1_export_job: Option<Receiver<anyhow::Result<ps1_export::ExportResult>>>,
    ps1_export_output: Option<PathBuf>,
    ps1_export_imports:Option<ps1_export::ImportCoverage>,
    unrestricted_img_export: bool,
    /// Source being dragged across the panels (no mouse-driven text selection).
    editor_asset_drag: Option<EditorCatalogItem>,
    /// Highlight a single catalog entry while dragging or after clicking.
    editor_catalog_active: Option<EditorCatalogItem>,
    editor_modal: Option<(EditorCatalogItem, bool)>, // false: properties; true: preview
    editor_preview: Option<(EditorCatalogItem, EditorPreview)>,
    editor_preview_view: Viewport,
    editor_monsters: Option<Vec<crate::actors::MonsterRow>>,
    editor_archives: Option<Vec<(String, Vec<(String, EditorCatalogItem)>)>>,
    editor_names: Catalog,
    editor_catalog_error: Option<String>,
    editor_hierarchy_filter: String,
    editor_rename: Option<EditorRename>,
    /// Detached inspector never reserves height in the scene viewport.
    editor_properties_target: Option<(bool, usize)>,
    /// Right-click context menu appears only for an un-dragged selected mesh.
    editor_context_menu: Option<(egui::Pos2, (bool, usize))>,
    editor_name_edit_key: Option<String>,
    editor_name_draft: Record,
    editor_name_apply_mesh: bool,
    editor_name_status: Option<String>,
    about_open: bool,
    /// The window's inner size, tracked for the settings file.
    window_size: Option<[f32; 2]>,
    window_pos: Option<[i32; 2]>,
    path_text: String,
    filter: String,
    class_filter: Option<String>,
    // Textures tab.
    tim_sel: usize,
    clut: usize,
    tim_zoom: f32,
    tim_tex: Option<((usize, usize, usize), TextureHandle)>,
    /// The decode failure for the key [`Self::tim_tex`] would hold.
    tim_err: Option<((usize, usize, usize), String)>,
    // Model tab.
    tmd_sel: usize,
    model: Option<ModelView>,
    /// Scene VRAM images by `block:kind`, built on demand.
    scenes: HashMap<String, SceneSlot>,
    actors: actors_ui::ActorsState,
    media: media_ui::MediaState,
    /// Entry the browser should expand and scroll to on its next frame.
    reveal: Option<usize>,
    // Audio tab.
    vab_sel: usize,
    seq_loop: bool,
    player: Player,
    // Extract mode.
    out_text: String,
    job: Option<ExtractJob>,
    /// The extraction log shows only its warnings.
    log_warnings_only: bool,
    /// The game binary shipped beside this one, if any.
    engine: Option<PathBuf>,
    /// A game started with Play, while it runs.
    game: Option<crate::launch::Game>,
    /// The folder the last export or extraction wrote into, for the
    /// status line's *Show folder* button.
    last_written: Option<PathBuf>,
    /// The informational status text last seen and when it appeared (or
    /// was last hovered): it clears itself after [`STATUS_TTL`].
    status_seen: Option<(String, std::time::Instant)>,
    /// Theme and zoom from the settings have been pushed into egui.
    appearance_applied: bool,
    // Automation.
    auto: Automation,
    auto_selected: bool,
    auto_extract_started: bool,
    settle_since: Option<std::time::Instant>,
    screenshot_requested: bool,
}

impl App {
    pub fn new(studio: Studio, initial: Option<PathBuf>, auto: Automation) -> Self {
        let mut studio = studio;
        // Recent entries whose disc or folder has gone are dropped at start.
        if studio.settings.prune_recent() {
            studio.save_settings();
        }
        // The disc or folder opened last, when it is still there. When it
        // has gone (moved, renamed, an unplugged drive) say so: the recent
        // list has just dropped it, and the start page alone would read as
        // a first run.
        let last = studio.settings.last_source.clone();
        let open = initial.or_else(|| last.clone().filter(|p| p.exists()));
        if open.is_none()
            && let Some(gone) = last.filter(|p| !p.exists())
        {
            studio.error = Some(format!(
                "The disc opened last is no longer at {} - open it from where it is now.",
                gone.display()
            ));
        }
        let path_text = open
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        if let Some(p) = &open {
            studio.open(p);
        }
        let out_text = studio
            .settings
            .last_out_dir
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        // The view and tab open when the app last closed.
        let mode = studio
            .settings
            .mode
            .as_deref()
            .and_then(Mode::parse)
            .unwrap_or(Mode::Browse);
        let tab = studio
            .settings
            .tab
            .as_deref()
            .and_then(Tab::parse)
            .unwrap_or(Tab::Overview);
        let (editor_names, editor_catalog_error) = match Catalog::load(&editor_catalog_path(), &editor_legacy_names_path()) {
            Ok(catalog) => (catalog, None),
            Err(error) => (Catalog::default(), Some(error)),
        };
        let (editor_cells, cell_catalog_error)=match CellStore::open(studio.settings.last_source.as_deref()) {
            Ok(cells)=>(cells,None),
            Err(error)=>(CellStore::default(),Some(error)),
        };
        let mut app = Self {
            studio,
            mode,
            tab,
            native_scene: None,
            pending_native_scene: None,
            native_selection: Default::default(),
            native_selected: None,
            native_hierarchy_anchor: None,
            native_selection_click: None,
            native_view: Viewport::new(Shading::Textured),
            native_ground_visible: true,
            native_collision_wire_visible: false,
            native_floor_debug_visible: false,
            native_floor_debug: None,
            native_floor_debug_key: String::new(),
            object_collision_draft_visible: false,
            generated_collision_target: None,
            native_collision_grid: None,
            native_collision_preview_cache: None,
            native_move_mode: EditorMoveMode::Grid,
            native_grid_size: 32.0,
            native_anchor_snap: false,
            geck_drag_axis: None,
            geck_angle_snap: 0.0,
            geck_transform_clipboard: None,
            geck_camera_bookmarks: [None; 3],
            native_drag_group: Vec::new(),
            native_drag: None,
            native_rotate: None,
            native_rotate_undo: Vec::new(),
            native_rotate_redo: Vec::new(),
            editor_undo: Vec::new(),
            editor_redo: Vec::new(),
            editor_gesture_start: None,
            npc_playing: None,
            npc_next_tick: 0.0,
            native_undo: Vec::new(),
            native_redo: Vec::new(),
            editor_search: String::new(),
            global_repository: Vec::new(),
            global_repository_rx: None,
            global_repository_started: false,
            global_repository_completed: 0,
            global_repository_total: 0,
            global_repository_failures: 0,
            global_repository_drag: None,
            global_repository_selected: None,
            global_repository_dialog: None,
            global_repository_preview: None,
            global_repository_preview_view: Viewport::new(Shading::Textured),
            global_name_edit_key: None,
            global_name_draft: Record::default(),
            global_name_status: None,
            global_open_category: None,
            global_open_branch: None,
            global_open_scene: None,
            editor_cell_search: String::new(),
            editor_cells,
            editor_cell_selected: None,
            editor_cell_active: None,
            editor_cell_rename: None,
            editor_cell_properties: None,
            editor_cell_last_autosave: 0.0,
            ps1_export_job: None,
            ps1_export_output: None,
            ps1_export_imports:None,
            unrestricted_img_export: true,
            editor_asset_drag: None,
            editor_catalog_active: None,
            editor_modal: None,
            editor_preview: None,
            editor_preview_view: Viewport::new(Shading::Textured),
            editor_monsters: None,
            editor_archives: None,
            editor_names,
            editor_catalog_error: editor_catalog_error.or(cell_catalog_error),
            editor_hierarchy_filter: String::new(),
            editor_rename: None,
            editor_properties_target: None,
            editor_context_menu: None,
            editor_name_edit_key: None,
            editor_name_draft: Record::default(),
            editor_name_apply_mesh: false,
            editor_name_status: None,
            about_open: false,
            window_size: None,
            window_pos: None,
            path_text,
            filter: String::new(),
            class_filter: None,
            tim_sel: 0,
            clut: 0,
            tim_zoom: 2.0,
            tim_tex: None,
            tim_err: None,
            tmd_sel: 0,
            model: None,
            scenes: HashMap::new(),
            actors: actors_ui::ActorsState::default(),
            media: media_ui::MediaState::default(),
            reveal: None,
            vab_sel: 0,
            seq_loop: false,
            player: Player::default(),
            out_text,
            job: None,
            log_warnings_only: false,
            engine: std::env::current_exe()
                .ok()
                .and_then(|exe| crate::launch::find_engine(&exe)),
            game: None,
            last_written: None,
            status_seen: None,
            appearance_applied: false,
            auto,
            auto_selected: false,
            auto_extract_started: false,
            settle_since: None,
            screenshot_requested: false,
        };
        if let Some(t) = app.auto.tab.clone() {
            if t.eq_ignore_ascii_case("about") {
                app.about_open = true;
            } else if let Some(m) = Mode::parse(&t) {
                app.mode = m;
            } else if let Some(tab) = Tab::parse(&t) {
                app.mode = Mode::Browse;
                app.tab = tab;
            }
        }
        if app.auto.actor.is_some() {
            app.mode = Mode::Actors;
        }
        if app.auto.media.is_some() {
            app.mode = Mode::Media;
        }
        if let Some(q) = app.auto.search.clone() {
            app.filter = q;
        }
        if let Some(dir) = app.auto.extract_to.clone() {
            app.out_text = dir.display().to_string();
            app.mode = Mode::Extract;
        }
        app
    }

    fn open_path(&mut self, p: &Path) {
        self.save_active_editor_cell();
        match CellStore::open(Some(p)) {
            Ok(cells)=>{self.editor_cells=cells;self.editor_cell_selected=None;self.editor_cell_active=None;
                self.editor_cell_rename=None;self.editor_cell_properties=None;},
            Err(err)=>{self.editor_cells=CellStore::default();
                self.editor_cell_selected=None;self.editor_cell_active=None;self.studio.error=Some(format!("Cell catalog: {err}"));},
        }
        // A newly opened disc has a different asset identity / index.
        // Dropping the receiver cancels the old scan on its next send.
        self.global_repository.clear();
        self.global_repository_rx = None;
        self.global_repository_started = false;
        self.global_repository_completed = 0;
        self.global_repository_total = 0;
        self.global_repository_failures = 0;
        self.global_repository_drag = None;
        self.global_repository_selected = None;
        self.global_repository_dialog = None;
        self.global_repository_preview = None;
        self.global_name_edit_key = None;
        self.global_name_status = None;
        self.global_open_category = None;
        self.global_open_branch = None;
        self.global_open_scene = None;
        self.native_scene = None;
        self.pending_native_scene = None;
        self.native_collision_preview_cache = None;
        self.native_selected = None; self.native_selection.clear(); self.native_hierarchy_anchor=None; self.native_selection_click=None;
        self.native_drag = None;
        self.editor_asset_drag = None;
        self.editor_catalog_active = None;
        self.native_rotate = None;
        self.native_rotate_undo.clear();
        self.native_rotate_redo.clear();
        self.editor_undo.clear();
        self.editor_redo.clear();
        self.editor_gesture_start=None;
        self.npc_playing=None;
        self.native_undo.clear();
        self.native_redo.clear();
        self.editor_modal = None;
        self.editor_preview = None;
        self.editor_rename = None;
        self.editor_properties_target = None;
        self.editor_context_menu = None;
        self.editor_monsters = None;
        self.editor_archives = None;
        self.native_view.version = self.native_view.version.wrapping_add(1);
        self.path_text = p.display().to_string();
        self.player.stop();
        self.model = None;
        self.tim_tex = None;
        self.tim_err = None;
        self.studio.open(p);
    }

    fn select(&mut self, pos: usize) {
        if self.studio.selected != Some(pos) {
            self.tim_sel = 0;
            self.clut = 0;
            self.tmd_sel = 0;
            self.vab_sel = 0;
            self.tim_tex = None;
            self.tim_err = None;
            self.model = None;
        }
        self.studio.select(pos);
    }

    /// Where an export goes: the scripted folder, else a save dialog.
    fn export_target(&mut self, default_name: &str, ext: &str) -> Option<PathBuf> {
        if let Some(dir) = &self.auto.export_dir {
            let _ = std::fs::create_dir_all(dir);
            return Some(dir.join(default_name));
        }
        let mut d = rfd::FileDialog::new()
            .set_file_name(default_name)
            .add_filter(ext.to_uppercase(), &[ext]);
        if let Some(dir) = &self.studio.settings.last_export_dir {
            d = d.set_directory(dir);
        }
        let picked = d.save_file();
        if let Some(p) = &picked {
            self.studio.settings.last_export_dir = p.parent().map(Path::to_path_buf);
            self.studio.save_settings();
        }
        picked
    }

    fn report(&mut self, what: &str, res: anyhow::Result<()>, path: &Path) {
        match res {
            Ok(()) => {
                self.studio.status = Some(format!("{what} written to {}", path.display()));
                self.studio.error = None;
                self.last_written = if path.is_dir() {
                    Some(path.to_path_buf())
                } else {
                    path.parent().map(Path::to_path_buf)
                };
            }
            Err(e) => self.studio.error = Some(format!("{what} export failed: {e:#}")),
        }
    }

    fn run_automation(&mut self, ctx: &egui::Context) {
        if !self.auto_selected
            && let (Some(idx), Some(lib)) = (self.auto.select, self.studio.library.clone())
        {
            self.auto_selected = true;
            if let Some(pos) = lib.position_of(idx) {
                self.select(pos);
                self.reveal = Some(pos);
                if let Some(t) = self.auto.tmd {
                    self.tmd_sel = t;
                }
            }
        }
        if !self.auto_extract_started
            && let Some(dir) = self.auto.extract_to.clone()
            && self.studio.library.is_some()
        {
            self.auto_extract_started = true;
            self.out_text = dir.display().to_string();
            self.start_extract();
        }
        if self.auto.play && self.studio.library.is_some() && self.studio.loading().is_none() {
            self.auto.play = false;
            self.start_game();
        }
        let Some(shot) = self.auto.screenshot.clone() else {
            return;
        };
        // Settled once no load is in flight. With nothing to open (no path
        // given, no last disc) that is the first frame: the welcome screen
        // is the state to capture. The selection and extraction steps only
        // wait when there is a library for them to act on.
        let loaded = self.studio.loading().is_none();
        let has_lib = self.studio.library.is_some();
        let analysed = self.auto.select.is_none() || !has_lib || self.studio.analysis.is_some();
        let extracted = self.auto.extract_to.is_none()
            || !has_lib
            || self.job.as_ref().is_some_and(|j| j.outcome().is_some());
        let idle =
            loaded && analysed && extracted && !self.studio.is_analyzing() && !self.media.busy();
        if !idle && self.studio.error.is_none() {
            self.settle_since = None;
            ctx.request_repaint();
            return;
        }
        // Let the window map and a few frames land before reading it back.
        let since = *self.settle_since.get_or_insert_with(|| {
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            std::time::Instant::now()
        });
        if since.elapsed() > std::time::Duration::from_millis(1500) && !self.screenshot_requested {
            self.screenshot_requested = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
        }
        ctx.request_repaint();
        let image = ctx.input(|i| {
            i.raw.events.iter().find_map(|e| match e {
                egui::Event::Screenshot { image, .. } => Some(Arc::clone(image)),
                _ => None,
            })
        });
        if let Some(img) = image {
            let [w, h] = img.size;
            let rgba: Vec<u8> = img.pixels.iter().flat_map(|c| c.to_array()).collect();
            match legaia_tim::write_png(&shot, w, h, &rgba) {
                Ok(()) => log::info!("screenshot saved to {}", shot.display()),
                Err(e) => log::error!("screenshot: {e:#}"),
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    fn start_extract(&mut self) {
        let Some(lib) = &self.studio.library else {
            self.studio.error = Some("Open a disc image first.".into());
            return;
        };
        let Source::Disc(bin) = &lib.source else {
            self.studio.error = Some(
                "Extraction reads a disc image; the open source is an extracted folder.".into(),
            );
            return;
        };
        let out = PathBuf::from(self.out_text.trim());
        if self.out_text.trim().is_empty() {
            self.studio.error = Some("Choose an output folder.".into());
            return;
        }
        let s = &mut self.studio.settings;
        s.last_out_dir = Some(out.clone());
        let options = legaia_extract::Options {
            bin: bin.clone(),
            out,
            skip_verify: s.skip_verify,
            skip_png: s.skip_png,
            skip_xa: s.skip_xa,
            skip_catalog: s.skip_catalog,
            skip_font: s.skip_font,
            all_textures: s.all_textures,
            verbose: false,
        };
        self.studio.save_settings();
        self.studio.error = None;
        self.job = Some(ExtractJob::start(options));
    }

    // --- Panels ------------------------------------------------------------

    /// The native file dialog for a disc image, opened where the last one
    /// was.
    fn pick_disc(&mut self) {
        let mut dialog = rfd::FileDialog::new()
            .set_title("Choose your Legend of Legaia disc image")
            .add_filter("PlayStation disc image", &["bin", "cue", "BIN", "CUE"]);
        if let Some(dir) = self.last_dir() {
            dialog = dialog.set_directory(dir);
        }
        if let Some(p) = dialog.pick_file() {
            self.open_path(&p);
        }
    }

    /// The native folder dialog for a `legaia-extract` output folder.
    fn pick_folder(&mut self) {
        let mut dialog = rfd::FileDialog::new().set_title("Choose a legaia-extract output folder");
        if let Some(dir) = self.last_dir() {
            dialog = dialog.set_directory(dir);
        }
        if let Some(p) = dialog.pick_folder() {
            self.open_path(&p);
        }
    }

    /// The folder holding the source opened last, if it still exists.
    fn last_dir(&self) -> Option<PathBuf> {
        self.studio
            .settings
            .last_source
            .as_deref()
            .and_then(Path::parent)
            .filter(|d| d.is_dir())
            .map(Path::to_path_buf)
    }

    /// The app-wide shortcuts that are not tied to a widget: Ctrl+O, F5
    /// (Play), Ctrl+1..4 (the views).
    fn shortcuts(&mut self, ctx: &egui::Context) {
        let cmd = |k| egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, k);
        if ctx.input_mut(|i| i.consume_shortcut(&cmd(egui::Key::O))) {
            self.pick_disc();
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::F5)) {
            self.start_game();
        }
        for (key, mode) in [
            (egui::Key::Num1, Mode::Browse),
            (egui::Key::Num2, Mode::Actors),
            (egui::Key::Num3, Mode::Media),
            (egui::Key::Num4, Mode::Extract),
            (egui::Key::Num5, Mode::Editor),
        ] {
            if ctx.input_mut(|i| i.consume_shortcut(&cmd(key))) {
                self.mode = mode;
            }
        }
    }

    /// Start assembling the original game scene directly from the loaded PROT index.
    fn load_native_scene(&mut self, scene_name: String, cell:CellSelection) {
        let Some(lib) = self.studio.library.as_ref() else { return };
        let index = Arc::clone(&lib.index);
        let requested_name = scene_name.clone();
        let ownership_path = lib.source.path().to_path_buf();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let result = legaia_engine_world_views::scene_staging::assemble_field_scene(
                index.as_ref(), &requested_name,
            ).map(|scene| {
                let mut world = world_view::build_world(&scene, index.as_ref());
                native_wall_export::attach_preview(&ownership_path,&scene,&mut world,index.as_ref());
                (scene, world)
            });
            let _ = tx.send(result);
        });
        self.pending_native_scene = Some((scene_name,cell,NativeSceneSlot::Loading(rx)));
    }

    fn poll_native_scene_load(&mut self, ctx:&egui::Context) {
        let completed = self.pending_native_scene.as_ref().and_then(|(_,_,s)| match s {
            NativeSceneSlot::Loading(rx) => match rx.try_recv() {
                Ok(result)=>Some(result),
                Err(std::sync::mpsc::TryRecvError::Disconnected)=>Some(Err("Scene loader disconnected".into())),
                Err(std::sync::mpsc::TryRecvError::Empty)=>None,
            },
            _ => None,
        });
        if let Some(result) = completed {
            let (source,cell,_)=self.pending_native_scene.take().expect("completed pending load");
            let restored=result.and_then(|(scene,mut world)| {
                if let CellSelection::Copy(id)=&cell {
                    let text=self.editor_cells.read_overlay(*id)?;
                    let doc=scene_instances::parse(&text,&source,&world)?;
                    let index=self.studio.library.as_ref().ok_or("Original disc index unavailable")?;
                    scene_instances::apply(&doc,&mut world,scene.placements.len(),&index.index)?;
                }
                Ok((scene,world))
            });
            match restored {
                Ok((scene,world))=>{
                    self.save_active_editor_cell();
                    self.native_scene=Some((source,NativeSceneSlot::Ready(Arc::new(scene),Arc::new(world))));
                    self.editor_cell_active=Some(cell);
                    self.finish_native_scene_load();
                },
                Err(error)=>{
                    self.editor_cell_selected=self.editor_cell_active.clone();
                    self.studio.error=Some(format!("Scene load failed; previous document preserved: {error}"));
                    if self.native_scene.is_none() {
                        self.native_scene=Some((source,NativeSceneSlot::Failed(error)));
                    }
                },
            }
            ctx.request_repaint();
        }
        if self.pending_native_scene.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(75));
        }
    }

    /// Reset document-local UI only after a replacement scene is ready.
    fn finish_native_scene_load(&mut self) {
        self.native_collision_preview_cache = None;
        self.native_selected = None; self.native_selection.clear(); self.native_hierarchy_anchor=None; self.native_selection_click=None;
        self.native_drag = None;
        self.native_rotate = None;
        self.native_rotate_undo.clear();
        self.native_rotate_redo.clear();
        self.editor_undo.clear();
        self.editor_redo.clear();
        self.editor_gesture_start=None;
        self.npc_playing=None;
        self.native_undo.clear();
        self.native_redo.clear();
        self.editor_modal = None;
        self.editor_preview = None;
        self.editor_rename = None;
        self.editor_properties_target = None;
        self.editor_context_menu = None;
        // The normal Frame All reset now uses the upright Euler branch.
        // Frame Selected / F uses frame_instance and is deliberately unchanged.
        self.native_view.frame_all();
        self.native_view.version = self.native_view.version.wrapping_add(1);
    }



    fn shared_name_key(&self,scene:Option<&legaia_engine_scene::scene_assembly::AssembledScene>,item:EditorCatalogItem)->Option<String> {
        let (Some(scene),Some((_,NativeSceneSlot::Ready(loaded,world))))=(scene,self.native_scene.as_ref()) else {return None;};
        if scene.name!=loaded.name {return None;}
        match item {
            EditorCatalogItem::Draw(t,i)=>world.base_name_key((t,i)).map(str::to_owned),
            EditorCatalogItem::Mesh(i)=>world.naming.meshes.get(i).cloned().flatten(),_=>None,
        }
    }

    fn editor_name_for(&self,
            scene: Option<&legaia_engine_scene::scene_assembly::AssembledScene>,
            item: EditorCatalogItem) -> String {
        if let EditorCatalogItem::Draw(t,i)=item {
            if let Some((_,NativeSceneSlot::Ready(loaded,world)))=&self.native_scene {
                if scene.is_some_and(|s|s.name==loaded.name) {
                    if let Some(draw)=world.at(t,i) {
                        if let Some(name)=&draw.display_name {return name.clone();}
                        if let Some(name)=world.base_name_key((t,i)).and_then(|key|self.editor_names.get(key)) {return name.clone();}
                        if let Some(uid)=draw.editor_uid {
                            if let Some(name)=world.naming.foreign_legacy.get(&uid).and_then(|key|self.editor_names.get(key)) {return name.clone();}
                            if let Some(foreign)=world.foreign.iter().find(|f|f.uid==uid) {return format!("Foreign {} #{} from {}",foreign.kind,foreign.record,foreign.source);}
                            if let Some(origin)=draw.source_ref {
                                if let Some(name)=editor_item_name_key(scene,EditorCatalogItem::Draw(origin.0,origin.1)).and_then(|key|self.editor_names.get(&key)) {return name.clone();}
                                return editor_native_item_hint(scene,EditorCatalogItem::Draw(origin.0,origin.1));
                            }
                        }
                    }
                }
            }
        }
        if let Some(name)=self.shared_name_key(scene,item).and_then(|key|self.editor_names.get(&key)) {return name.clone();}
        if let EditorCatalogItem::Draw(false,index) = item {
            if index >= world_view::NPC_KEY_BASE {
                if let Some(name) = editor_item_name_key(scene,item)
                    .and_then(|key|self.editor_names.get(&key)) {
                    return name.clone();
                }
                if let Some((_,NativeSceneSlot::Ready(_,world))) = self.native_scene.as_ref()
                    && let Some(npc) = world.npc((false,index)) {
                    return npc.evidence_label();
                }
                return format!("Field actor MAN #{}",index-world_view::NPC_KEY_BASE);
            }
        }
        if let Some(key) = editor_item_name_key(scene, item) {
            if let Some(name) = self.editor_names.get(&key) { return name.clone(); }
        }
        if let (Some(s), EditorCatalogItem::Draw(terrain, i)) = (scene, item) {
            let list = if terrain { &s.terrain } else { &s.placements };
            if let Some(draw) = list.get(i) {
                if let Some(key) = editor_mesh_name_key(s, draw.env_slot) {
                    if let Some(name) = self.editor_names.get(&key) { return name.clone(); }
                }
            }
        }
        match item {
            EditorCatalogItem::Archive(pos) => self.studio.library.as_ref()
                .and_then(|lib| lib.entries.get(pos).map(|entry| {
                    let block = lib.retail_block(entry.index).unwrap_or("unidentified block");
                    let place = crate::place_name(block).unwrap_or(block);
                    format!("{place} / PROT {:04} ({block})", entry.index)
                }))
                .unwrap_or_else(|| editor_native_item_hint(scene, item)),
            EditorCatalogItem::Monster(id) => self.editor_monsters.as_ref()
                .and_then(|rows| rows.iter().find(|row| row.id == id))
                .map(|m| m.name.clone())
                .unwrap_or_else(|| editor_native_item_hint(scene, item)),
            _ => editor_native_item_hint(scene, item),
        }
    }

    fn repository_matches(&self,asset:&global_repository::Asset,term:&str)->bool {
        asset.matches(term,&self.editor_names) || self.editor_cells.label(&CellSelection::Original(asset.source.clone()))
            .is_some_and(|name|name.to_lowercase().contains(term))
    }

    fn editor_item_matches(&self,
        scene: Option<&legaia_engine_scene::scene_assembly::AssembledScene>,
        item: EditorCatalogItem, term: &str) -> bool {
        if term.is_empty() { return true; }
        let key = editor_item_name_key(scene, item).unwrap_or_default();
        let mut text = format!("{} {}", self.editor_name_for(scene, item), self.editor_names.search_text(&key));
        if let Some(key)=self.shared_name_key(scene,item) {text.push_str(&self.editor_names.search_text(&key));}
        if let EditorCatalogItem::Draw(false,index)=item {
            if let Some((_,NativeSceneSlot::Ready(_,world)))=self.native_scene.as_ref()
                && let Some(npc)=world.npc((false,index)) {
                    text.push_str(&format!(" {} ANM {} TMD {} MAN {} {}",
                        npc.dialog.as_deref().unwrap_or(""),npc.anim_id,npc.model_index,npc.record,npc.kind));
                }
        }
        if let (Some(s), EditorCatalogItem::Draw(terrain, i)) = (scene, item) {
            if let Some(d) = (if terrain { &s.terrain } else { &s.placements }).get(i) {
                if let Some(mesh_key) = editor_mesh_name_key(s, d.env_slot) {
                    text.push_str(&self.editor_names.search_text(&mesh_key));
                }
            }
        }
        text.to_lowercase().contains(term)
    }

    fn editor_name_properties(&mut self, ui: &mut egui::Ui,
            scene: Option<&legaia_engine_scene::scene_assembly::AssembledScene>,
            item: EditorCatalogItem) {
        let Some(key) = self.shared_name_key(scene,item).or_else(||editor_item_name_key(scene, item)) else { return; };
        if self.editor_name_edit_key.as_deref() != Some(&key) {
            self.editor_name_edit_key = Some(key.clone());
            self.editor_name_draft = self.editor_names.records.get(&key).cloned().or_else(||
                editor_item_name_key(scene,item).and_then(|old|self.editor_names.records.get(&old).cloned())).unwrap_or_default();
            self.editor_name_apply_mesh = false;
            self.editor_name_status = None;
        }
        ui.heading("Asset identification");
        ui.label(format!("Display: {}", self.editor_name_for(scene, item)));
        if let Some(error) = &self.editor_catalog_error {
            ui.colored_label(Color32::LIGHT_RED, error);
            ui.label("Naming is read-only until the catalog is repaired and LCK restarted.");
        }
        ui.add(egui::TextEdit::singleline(&mut self.editor_name_draft.name)
            .hint_text("Display name"));
        egui::ComboBox::from_id_salt("identification_confidence")
            .selected_text(self.editor_name_draft.confidence.label())
            .show_ui(ui, |ui| {
                for confidence in Confidence::ALL {
                    ui.selectable_value(&mut self.editor_name_draft.confidence, confidence, confidence.label());
                }
            });
        ui.add(egui::TextEdit::singleline(&mut self.editor_name_draft.category).hint_text("Category (evidence supported)"));
        ui.add(egui::TextEdit::singleline(&mut self.editor_name_draft.aliases).hint_text("Alternate names / search terms"));
        ui.add(egui::TextEdit::multiline(&mut self.editor_name_draft.evidence)
            .desired_rows(2).hint_text("Evidence: native reference or visual inspection details"));
        let mesh_key = if key.starts_with("visual-v1:") || key.starts_with("npc-asset-v1:") {None} else {match (scene, item) {
            (Some(s), EditorCatalogItem::Draw(terrain, index)) => {
                let draws = if terrain { &s.terrain } else { &s.placements };
                draws.get(index).and_then(|draw| editor_mesh_name_key(s, draw.env_slot))
            }
            _ => None,
        }};
        if mesh_key.is_some() {
            ui.checkbox(&mut self.editor_name_apply_mesh,
                "Also apply to the source mesh in this cell");
        }
        ui.add_enabled_ui(self.editor_catalog_error.is_none(), |ui| {
            ui.horizontal(|ui| {
                let save = ui.button("Save identification").clicked();
                let remove = ui.button("Remove override").clicked();
                if save || remove {
                    let mut next = self.editor_names.clone();
                    if remove {
                        next.records.remove(&key);
                        if let Some(old)=editor_item_name_key(scene,item) {next.records.remove(&old);}
                    } else {
                        let mut record = self.editor_name_draft.clone();
                        record.name = record.name.trim().to_owned();
                        next.records.insert(key.clone(), record.clone());
                        if self.editor_name_apply_mesh {
                            if let Some(mesh_key) = &mesh_key { next.records.insert(mesh_key.clone(), record); }
                        }
                    }
                    self.editor_name_status = Some(match next.save(&editor_catalog_path()) {
                        Ok(()) => {
                            self.editor_names = next;
                            if remove { self.editor_name_draft = Record::default(); }
                            "Identification saved. Original game data unchanged.".into()
                        }
                        Err(e) => format!("Not saved: {e}"),
                    });
                }
            });
        });
        if let Some(status) = &self.editor_name_status { ui.label(status); }
        ui.small("User labels are not verified identities. Probable / Verified require evidence. Removing an instance override retains its source mesh name.");
        ui.collapsing("Original source identifiers", |ui| {
            ui.monospace(&key);
            if ui.button("Copy identifier").clicked() { ui.ctx().copy_text(key.clone()); }
        });
        ui.separator();
    }

    fn editor_inventory(&self,
        scene: Option<&legaia_engine_scene::scene_assembly::AssembledScene>) -> String {
        fn cell(value: &str) -> String {
            value.replace('\t', " ").replace('\r', " ").replace('\n', " ")
        }
        let mut items = Vec::new();
        if let Some(scene) = scene {
            items.extend((0..scene.placements.len()).map(|i| EditorCatalogItem::Draw(false, i)));
            items.extend((0..scene.terrain.len()).map(|i| EditorCatalogItem::Draw(true, i)));
            items.extend((0..scene.env_tmds.len()).map(EditorCatalogItem::Mesh));
            if let Some((_,NativeSceneSlot::Ready(_,world)))=self.native_scene.as_ref() {
                items.extend(world.npcs.iter().filter(|n|!n.is_copy)
                .map(|n|EditorCatalogItem::Draw(false,n.key)));
            }
        }
        items.extend((0..4).map(EditorCatalogItem::Character));
        if let Some(monsters) = &self.editor_monsters {
            items.extend(monsters.iter().map(|m| EditorCatalogItem::Monster(m.id)));
        }
        if let Some(lib) = &self.studio.library {
            items.extend((0..lib.entries.len()).map(EditorCatalogItem::Archive));
        }
        let mut text = String::from("identifier\tdisplay_name\tstatus\tcategory\taliases\tevidence\tname_scope\tcapability\n");
        for item in items {
            let Some(key) = editor_item_name_key(scene, item) else { continue; };
            let inherited_key = match (scene, item) {
                (Some(s), EditorCatalogItem::Draw(terrain, i)) => {
                    (if terrain { &s.terrain } else { &s.placements }).get(i)
                        .and_then(|d| editor_mesh_name_key(s, d.env_slot))
                }
                _ => None,
            };
            let direct = self.shared_name_key(scene,item).and_then(|shared|self.editor_names.records.get(&shared)).or_else(||self.editor_names.records.get(&key));
            let record = direct.or_else(|| inherited_key.as_ref().and_then(|k| self.editor_names.records.get(k)));
            let name = self.editor_name_for(scene, item);
            let status = record.map(|r| r.confidence.label()).unwrap_or("Not catalog-identified");
            let scope = if direct.is_some() { "Direct override" } else if record.is_some() { "Inherited resource name" } else { "Built-in / technical fallback" };
            let capability = match item {
                EditorCatalogItem::Draw(..) => "Native record; editor offsets only; no scene export",
                EditorCatalogItem::Mesh(..) => "Preview source; new instance placement not implemented",
                EditorCatalogItem::Character(..) | EditorCatalogItem::Monster(..) => "Actor preview source",
                EditorCatalogItem::Archive(..) => "Archive source; not directly placeable",
            };
            let fields = [key.as_str(), name.as_str(), status,
                record.map(|r| r.category.as_str()).unwrap_or(""),
                record.map(|r| r.aliases.as_str()).unwrap_or(""),
                record.map(|r| r.evidence.as_str()).unwrap_or(""), scope, capability];
            text.push_str(&fields.into_iter().map(cell).collect::<Vec<_>>().join("\t"));
            text.push('\n');
        }
        text
    }

    /// Persist a copied cell's overlay without writing to the PS1 disc.
    /// Invoked when changing cells, closing the application and periodically
    /// while editing a project copy (avoids losing unsaved scene changes).
    fn save_active_editor_cell(&mut self) {
        let Some(CellSelection::Copy(id))=self.editor_cell_active.as_ref() else {return};
        let Some((source,NativeSceneSlot::Ready(_,world)))=self.native_scene.as_ref() else {return};
        let overlay=scene_instances::serialize(source,world);
        if let Err(error)=self.editor_cells.write_overlay(*id,&overlay) {
            self.studio.error=Some(format!("Cell copy autosave failed: {error}"));
        }
    }

    fn select_editor_cell(&mut self, cell: CellSelection) {
        if self.editor_cell_active.as_ref()==Some(&cell) &&
            self.native_scene.as_ref().is_some_and(|(_,slot)|matches!(slot,NativeSceneSlot::Ready(..)|NativeSceneSlot::Loading(..))) {
            self.pending_native_scene=None;
            return;
        }
        let Some(source)=self.editor_cells.source(&cell).map(str::to_owned) else {
            self.studio.error=Some("Selected project cell is missing".into());
            return;
        };
        self.save_active_editor_cell();
        self.editor_cell_selected=Some(cell.clone());
        self.load_native_scene(source,cell);
    }

    fn duplicate_editor_cell(&mut self, selected: CellSelection) {
        let Some(source)=self.editor_cells.source(&selected).map(str::to_owned) else {
            self.studio.error=Some("No source cell for selected copy".into());
            return;
        };
        let current=self.editor_cell_active.as_ref()==Some(&selected);
        let overlay=if current {
            self.native_scene.as_ref().and_then(|(name,slot)| {
                if name==&source {
                    if let NativeSceneSlot::Ready(_,world)=slot {
                        return Some(scene_instances::serialize(name,world));
                    }
                }
                None
            })
        } else {None};
        let overlay=match (overlay, &selected) {
            (Some(text),_) => text,
            (None,CellSelection::Copy(id))=> match self.editor_cells.read_overlay(*id) {
                Ok(text)=>text,
                Err(error)=>{self.studio.error=Some(format!("Cannot duplicate cell: {error}"));return;}
            },
            (None,CellSelection::Original(_))=>format!("LCK_SCENE_V1\t{source}\n"),
        };
        let base=self.editor_cells.label(&selected).unwrap_or_else(|| source.clone());
        let name=cell_catalog::duplicate_label(&base);
        match self.editor_cells.duplicate(source,name,&overlay) {
            Ok(id)=>{
                self.select_editor_cell(CellSelection::Copy(id));
                self.studio.status=Some(format!("Created independent editor cell copy #{id}. Native PS1 data unchanged; scene edits autosave."));
            }
            Err(error)=>self.studio.error=Some(format!("Cell duplication: {error}")),
        }
    }

    /// Separate, independently scrollable cell tree below All Objects.
    fn editor_cells_panel(&mut self,ui:&mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading("Cells / Scenes");
            ui.weak("↕ drag divider");
        });
        ui.add(egui::TextEdit::singleline(&mut self.editor_cell_search)
            .hint_text("Search cell names, native IDs..."));
        ui.small("Select to load · RMB: Properties / Duplicate / Rename · Ctrl+D");
        ui.separator();
        let mut choose=None;
        let mut duplicate=None;
        let mut rename=None;
        let mut properties=None;
        let mut focus_cell=false;
        let term=self.editor_cell_search.trim().to_lowercase();
        let copies=self.editor_cells.copies.values().cloned().collect::<Vec<_>>();
        egui::ScrollArea::vertical().id_salt("lck_cell_scroll")
            .auto_shrink([false,false]).show(ui, |ui| {
                egui::CollapsingHeader::new(format!("Original PS1 cells ({})", self.studio.blocks.len()))
                    .id_salt("lck_cell_original_tree").default_open(true).show(ui,|ui| {
                        // Group all source variants under their original location. Native
                        // CDNAME scene IDs remain independent; this is a browser-only tree.
                        // The grouping key is derived from the ORIGINAL ID, never a
                        // user-editable label. The first occurrence retains disc order.
                        let mut seen=std::collections::HashSet::new();
                        let mut families:Vec<(String,Vec<(CellSelection,String)>)>=Vec::new();
                        for cell in &self.studio.blocks {
                            if !seen.insert(cell.name.as_str()) {continue;}
                            let id=CellSelection::Original(cell.name.clone());
                            let label=self.editor_cells.label(&id).unwrap_or_else(||cell.name.clone());
                            let row=format!("{label}  ({})",cell.name);
                            let family=match cell.name.as_str() {
                                "town01"|"town0b"|"town0c"|"town0d"|"town0e"|"jou" => "Rim Elm".to_owned(),
                                "conc"|"conc2"|"conc3"|"concnow"|"concend" => "Conkram".to_owned(),
                                "rayman"|"rayman2"|"ropeway"|"ropeway2"|"station" => "Octam".to_owned(),
                                "bubu1"|"bubu2"|"edbubu" => "Buma".to_owned(),
                                "balden"|"balden2"|"edbalden" => "Vidna".to_owned(),
                                "retock"|"retockin"|"edretoin" => "Ratayu".to_owned(),
                                "bylon"|"edbylon" => "Biron Monastery".to_owned(),
                                "dolk"|"dolk2" => "Drake Castle".to_owned(),
                                "kor"|"kor3"|"kor4"|"kor5"|"korb2"|"korb3"|"korout"|
                                "koin1"|"koin1b"|"koin2"|"koin3"|"koin4"|"koin6"|"edkorout" => "Sol Tower".to_owned(),
                                "jouina"|"jouinb"|"jouinc"|"jouind"|"jouine"|"juui1"|"juui2" => "Bio Castle".to_owned(),
                                "deroa"|"chitei2" => "Jette's Fortress".to_owned(),
                                "taiku"|"taiku2" => "Zora's Floating Castle".to_owned(),
                                "nilboa"|"nilboa2" => "Nivora Ravine".to_owned(),
                                "uru"|"uru2" => "Uru Mais".to_owned(),
                                "tunnelb"|"tunnelc" => "Fire Path".to_owned(),
                                "teien"|"edteien" => "Sky Gardens".to_owned(),
                                "doman"|"eddoman" => "Usha Research Center".to_owned(),
                                "son"|"edson" => "Soren Camp".to_owned(),
                                _ => {let native=crate::place_name(&cell.name).unwrap_or(&cell.name);native.split(" (").next().unwrap_or(native).trim().to_owned()},
                            };
                            if let Some((_,entries))=families.iter_mut().find(|(key,_)|*key==family) {
                                entries.push((id,row));
                            } else {
                                families.push((family,vec![(id,row)]));
                            }
                        }
                        let mut singles=Vec::new();
                        for (family,entries) in families {
                            if entries.len()==1 {singles.extend(entries);continue;}
                            let family_match=family.to_lowercase().contains(&term);
                            let entries=entries.into_iter().filter(|(_,row)|
                                family_match || row.to_lowercase().contains(&term)).collect::<Vec<_>>();
                            if entries.is_empty(){continue;}
                            egui::CollapsingHeader::new(format!("{} ({})",family,entries.len()))
                                .id_salt(("lck_original_location",family.as_str()))
                                .default_open(false)
                                .open(if term.is_empty(){None}else{Some(true)})
                                .show(ui,|ui| {
                                    for (id,row) in entries {
                                        editor_cell_row(ui,id,row,&mut self.editor_cell_selected,&mut focus_cell,
                                            &mut choose,&mut duplicate,&mut rename,&mut properties);
                                    }
                                });
                        }
                        let singles=singles.into_iter().filter(|(_,row)|
                            row.to_lowercase().contains(&term)).collect::<Vec<_>>();
                        if !singles.is_empty() {
                            egui::CollapsingHeader::new(format!("Other / standalone cells ({})",singles.len()))
                                .id_salt("lck_original_standalone")
                                .default_open(false)
                                .open(if term.is_empty(){None}else{Some(true)})
                                .show(ui,|ui| {
                                    for (id,row) in singles {
                                        editor_cell_row(ui,id,row,&mut self.editor_cell_selected,&mut focus_cell,
                                            &mut choose,&mut duplicate,&mut rename,&mut properties);
                                    }
                                });
                        }
                    });
                egui::CollapsingHeader::new(format!("Project cell copies ({})",copies.len()))
                    .id_salt("lck_cell_copy_tree").default_open(true).show(ui,|ui| {
                        for cell in &copies {
                            let id=CellSelection::Copy(cell.id);
                            let label=format!("{}  [copy #{} of {}]",cell.name,cell.id,cell.source);
                            if !label.to_lowercase().contains(&term) {continue;}
                            editor_cell_row(ui,id,label,&mut self.editor_cell_selected,&mut focus_cell,
                                &mut choose,&mut duplicate,&mut rename,&mut properties);
                        }
                    });
            });
        if focus_cell || duplicate.is_some() || rename.is_some() || properties.is_some() {
            self.native_selected=None; self.native_selection.clear();
        }
        if let Some(cell)=choose {self.select_editor_cell(cell);}
        if let Some(cell)=duplicate {self.duplicate_editor_cell(cell);}
        if let Some(cell)=rename {
            let name=self.editor_cells.label(&cell).unwrap_or_default();
            self.editor_cell_rename=Some((cell,name));
        }
        if let Some(cell)=properties {self.editor_cell_properties=Some(cell);}
    }

    fn editor_cell_dialogs(&mut self,ctx:&egui::Context) {
        if let Some(cell)=self.editor_cell_properties.clone() {
            let mut open=true;
            egui::Window::new("Cell Properties")
                .id(egui::Id::new("lck_cell_properties"))
                .collapsible(false).resizable(true).open(&mut open).show(ctx,|ui| {
                    ui.heading(self.editor_cells.label(&cell).unwrap_or_default());
                    if let Some(source)=self.editor_cells.source(&cell) {
                        ui.label(format!("Original CDNAME / scene ID: {source}"));
                        if let Some(block)=self.studio.blocks.iter().find(|b|b.name==source) {
                            ui.label(format!("Source archive entries in this block: {}",block.entries.len()));
                        }
                    }
                    match cell {
                        CellSelection::Original(_) => {ui.label("Kind: original PS1 cell (source is read-only)");}
                        CellSelection::Copy(id) => {
                            ui.label(format!("Kind: independent LCK project cell, ID {id}"));
                            ui.label("Contents: original scene + its own .lckscene instance overlay");
                            if let Some(dir)=&self.editor_cells.dir {
                                ui.small(format!("Stored under: {}",dir.display()));
                            }
                        }
                    }
                    ui.small("Names, copies and scene edits are editor-only. This does not modify the original disc or add a PS1 CDNAME entry.");
                    if ui.button("Rename...").clicked() {self.editor_cell_rename=Some((cell.clone(),self.editor_cells.label(&cell).unwrap_or_default()));}
                    if ui.button("Close").clicked(){self.editor_cell_properties=None;}
                });
            if !open {self.editor_cell_properties=None;}
        }
        if let Some((cell,mut value))=self.editor_cell_rename.take() {
            let mut open=true;
            let mut save=false;
            let mut cancel=false;
            egui::Window::new("Rename Cell")
                .id(egui::Id::new("lck_cell_rename"))
                .collapsible(false).resizable(false).open(&mut open).show(ctx,|ui| {
                    ui.label("Editor display name (original native ID is preserved):");
                    let response=ui.add(egui::TextEdit::singleline(&mut value).desired_width(335.0));
                    save=ui.button("Save").clicked() ||
                        (response.lost_focus() && ui.input(|i|i.key_pressed(egui::Key::Enter)));
                    if ui.button("Restore default name").clicked() {value.clear();save=true;}
                    cancel=ui.button("Cancel").clicked();
                });
            if save {
                match self.editor_cells.rename(&cell,&value) {
                    Ok(())=>self.studio.status=Some("Cell display name saved to LCK catalog".into()),
                    Err(error)=>{
                        self.studio.error=Some(error);
                        self.editor_cell_rename=Some((cell,value));
                    }
                }
            } else if open && !cancel {self.editor_cell_rename=Some((cell,value));}
        }
    }


    /// Load one original donor scene only on Preview. Cached content always uses
    /// the SOURCE scene's authentic VRAM rather than the active target's VRAM.
    fn editor_load_global_asset_preview(&mut self, id: usize) {
        if self.global_repository_preview.as_ref().is_some_and(|p|p.id==id) { return; }
        self.global_repository_preview_view=Viewport::new(Shading::Textured);
        let mut preview=GlobalAssetPreview{id,scene:None,model:None,issue:None};
        let Some(asset)=self.global_repository.get(id) else { return; };
        let Some(lib)=self.studio.library.as_ref() else {
            preview.issue=Some("Open an original PS1 disc image first".into());
            self.global_repository_preview=Some(preview);
            return;
        };
        match legaia_engine_world_views::scene_staging::assemble_field_scene(&lib.index,&asset.source) {
            Err(e)=>preview.issue=Some(format!("Cannot assemble source scene: {e}")),
            Ok(scene)=>{
                preview.model=match asset.kind {
                    // World geometry is authored with PS1-down Y and can
                    // carry a native 180-degree MAP rotation.  Apply the
                    // source's REAL placement instead of spinning the camera.
                    global_repository::Kind::Placement(i) =>
                        editor_preview_placed_tmd(&scene, "placement", i),
                    global_repository::Kind::Terrain(i) =>
                        editor_preview_placed_tmd(&scene, "terrain", i),
                    global_repository::Kind::Mesh(slot) =>
                        editor_preview_reusable_tmd(&scene, slot),
                    global_repository::Kind::Npc(record) =>{
                        // A field actor's posed instance comes from its MAN record.
                        // Do not pass NPC assets through environment TMD indices.
                        let world=world_view::build_world(&scene,&lib.index);
                        let key=world.npcs.iter().find(|n|n.record==record).map(|n|n.key);
                        key.and_then(|key|world.at(false,key))
                            .and_then(|d|world.model.as_ref()
                                .and_then(|m|editor_isolate_draw(m,d.id)))
                    }
                };
                if preview.model.is_none() {
                    preview.issue=Some("This source has no decodable visible TMD in its original scene; original record metadata is still accessible.".into());
                }
                preview.scene=Some(Arc::new(scene));
            }
        }
        self.global_repository_preview=Some(preview);
    }

    /// The *only* left-hand repository. The current cell's independent
    /// hierarchy remains on the right; the cell tree stays below this pane.
    fn editor_global_repository(&mut self, ui: &mut egui::Ui,
        scene: Option<&legaia_engine_scene::scene_assembly::AssembledScene>) {
        ui.heading("Game-wide Asset Repository");
        let total=self.global_repository_total;
        ui.small(if self.global_repository_started {
            format!("{} original assets | {} / {} cells scanned ({} unavailable)",
                self.global_repository.len(),self.global_repository_completed,total,
                self.global_repository_failures)
        } else {"Open a disc to scan original game content.".into()});
        ui.add(egui::TextEdit::singleline(&mut self.editor_search)
            .hint_text("Search any source cell, object ID or identified name..."));
        ui.small("Select for details · RMB for Preview / Properties · drag ↳ to place");
        ui.small("↳ same-cell source  |  ⇢ donor-textured preview  |  ◇ foreign MAN actor (not imported)");
        if let Some(error)=&self.editor_catalog_error {
            ui.colored_label(Color32::LIGHT_RED,format!("Identification catalog: {error}"));
        }
        if let Some(id)=self.global_repository_selected {
            if let Some(asset)=self.global_repository.get(id) {
                let display=asset.display_name(&self.editor_names);
                let label=display.to_string();
                let source=asset.source.clone();
                let kind=asset.kind_label();
                let selected_key=asset.name_key();
                ui.group(|ui| {
                    ui.small("Selected original asset");
                    ui.label(egui::RichText::new(label).strong());
                    ui.small(format!("{kind} · {source}"));
                    if let Some(record)=self.editor_names.records.get(&selected_key) {
                        if !record.category.trim().is_empty() {
                            ui.small(format!("Identification category: {}",record.category));
                        }
                    }
                    ui.horizontal(|ui| {
                        if ui.button("Preview 3D").clicked() {
                            self.global_repository_dialog=Some((id,true));
                        }
                        if ui.button("Properties / Name").clicked() {
                            self.global_repository_dialog=Some((id,false));
                        }
                    });
                });
            }
        }
        if self.global_repository.is_empty() {return;}
        let term=self.editor_search.trim().to_lowercase();
        let mut counts=[0usize;4];
        // Count the entire 40K+ catalog without materializing GUI rows.
        for asset in &self.global_repository {
            if self.repository_matches(asset,&term) { counts[asset.group()]+=1; }
        }
        let titles=["World objects / MAP", "Terrain / decoration", "Reusable geometry / TMD", "Field NPCs / MAN"];
        let mut dragging=None;
        let mut selection=None;
        let mut open_dialog=None;
        let mut category=self.global_open_category;
        let mut branch=self.global_open_branch;
        let mut open_scene=self.global_open_scene.clone();
        ui.small("Verified town groupings from the original scene table; names require evidence.");
        for group in 0..4 {
            let label=format!("{} {} ({})", if category==Some(group) {"▾"} else {"▸"},titles[group],counts[group]);
            if ui.add_sized([ui.available_width().max(40.0),24.0],egui::Button::new(label)).clicked() {
                if category==Some(group) { category=None; } else { category=Some(group); }
                branch=None;
                open_scene=None;
            }
            if category!=Some(group) || counts[group]==0 { continue; }
            // One tier per original source cell; none depends on the loaded destination.
            // Grouping source provenance keeps similarly numbered TMDs distinct.
            let mut indexed:std::collections::BTreeMap<(String,String),Vec<usize>>=std::collections::BTreeMap::new();
            for (id,asset) in self.global_repository.iter().enumerate() {
                if asset.group()==group && self.repository_matches(asset,&term) {
                    let place=self.editor_cells.label(&CellSelection::Original(asset.source.clone())).unwrap_or(asset.source_label.clone());
                    indexed.entry((place,asset.source.clone())).or_default().push(id);
                }
            }
            for section in 0..(if group==0 {2} else {1}) {
                if group==0 {
                    let is_town=section==0;
                    let n=indexed.iter().filter(|((_,source),_)|
                        global_repository::is_verified_town(source)==is_town).count();
                    if n==0 { continue; }
                    let title=if is_town {"Towns & settlements"} else {"Other original locations"};
                    let active=branch==Some(is_town);
                    if ui.button(format!("  {} {} ({} scenes)",if active {"▾"} else {"▸"},title,n)).clicked() {
                        branch=if active {None} else {Some(is_town)};
                        open_scene=None;
                    }
                    if branch!=Some(is_town) { continue; }
                }
                for ((place,source),items) in &indexed {
                    if group==0 && global_repository::is_verified_town(source)!=(section==0) { continue; }
                    let active=open_scene.as_deref()==Some(source.as_str());
                    let header=format!("    {} {} ({}) [{}]",if active {"▾"} else {"▸"},place,items.len(),source);
                    if ui.add_sized([ui.available_width().max(40.0),24.0],egui::Button::new(header)).clicked() {
                        open_scene=if active {None} else {Some(source.clone())};
                    }
                    if !active { continue; }
                    if items.len()>125 {
                        ui.small(format!("Showing first 125 of {}. Search above to narrow.",items.len()));
                    }
                    for &id in items.iter().take(125) {
                        let asset=&self.global_repository[id];
                        let usable=scene.is_some_and(|s|global_repository::local_source(s,asset).is_some());
                        let symbol=if usable {"↳"}else if matches!(asset.kind,global_repository::Kind::Npc(_)) {"◇"} else {"⇢"};
                        let name=asset.display_name(&self.editor_names);
                        let text=format!("{symbol} [{place}] {name}");
                        let active=self.global_repository_selected==Some(id);
                        let response=ui.add_sized(egui::vec2(ui.available_width().max(40.0),24.0),
                            egui::Label::new(text).selectable(false).sense(egui::Sense::click_and_drag()));
                        if active || response.hovered() {
                            let color=if active {Color32::from_rgb(255,196,66)}
                                else if usable {Color32::from_rgb(96,210,180)}
                                else {Color32::from_rgb(160,164,180)};
                            ui.painter().rect_stroke(response.rect,egui::CornerRadius::same(3),
                                egui::Stroke::new(if active {2.0}else{1.0},color),egui::StrokeKind::Inside);
                        }
                        if response.drag_started() || catalog_pointer_pressed_on(ui,response.rect) {
                            dragging=Some(id);
                        }
                        if response.clicked() {selection=Some(id);}
                        response.context_menu(|ui| {
                            if ui.button("Preview original 3D model...").clicked() {
                                selection=Some(id);open_dialog=Some((id,true));ui.close();
                            }
                            if ui.button("Properties / identify asset...").clicked() {
                                selection=Some(id);open_dialog=Some((id,false));ui.close();
                            }
                            if ui.button("Rename...").clicked() {selection=Some(id);open_dialog=Some((id,false));ui.close();}
                            ui.separator();
                            ui.small(format!("Original source: {}",asset.source));
                            ui.small(if usable {"Exact original mesh in destination"}
                                else {"Needs native donor resource import"});
                        });
                        response.on_hover_text(if usable {
                            "Exact native match in target cell. Drag to create an independent native instance."
                        } else if matches!(asset.kind,global_repository::Kind::Npc(_)) {
                            "Original MAN actor is previewable, but cross-cell dialogue/animation/script import is not yet supported."
                        } else {
                            "Drag original PS1 geometry and donor textures for editor preview. Save .lckscene; native collision and IMG import still need resource relocation."
                        });
                    }
                }
            }
        }
        self.global_open_category=category;
        self.global_open_branch=branch;
        self.global_open_scene=open_scene;
        if let Some(id)=selection {self.global_repository_selected=Some(id);}
        if let Some(dialog)=open_dialog {self.global_repository_dialog=Some(dialog);}
        if let Some(id)=dragging {
            self.global_repository_selected=Some(id);
            self.global_repository_drag=Some(id);
            self.editor_asset_drag=None;
            ui.ctx().request_repaint();
        }
    }

    /// Separate source-asset window. Names/categories are user-maintained
    /// evidence records, never false verified identifications.
    fn editor_global_repository_dialog(&mut self,ctx:&egui::Context) {
        let Some((id,preview_mode))=self.global_repository_dialog else{return;};
        let Some(asset)=self.global_repository.get(id).cloned() else {
            self.global_repository_dialog=None;return;
        };
        if preview_mode {self.editor_load_global_asset_preview(id);}
        let name_key=asset.name_key();
        if !preview_mode && self.global_name_edit_key.as_deref()!=Some(&name_key) {
            self.global_name_edit_key=Some(name_key.clone());
            self.global_name_draft=asset.name_record(&self.editor_names);
            self.global_name_status=None;
        }
        let title=asset.display_name(&self.editor_names);
        let mut open=true;
        egui::Window::new(format!("{} — {title}",if preview_mode {"Preview"}else{"Asset Properties"}))
            .id(egui::Id::new("lck_global_asset_details"))
            .resizable(true).default_size([610.0,540.0]).min_size([390.0,240.0])
            .open(&mut open).show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if ui.selectable_label(preview_mode,"3D Preview").clicked() {
                        self.global_repository_dialog=Some((id,true));
                    }
                    if ui.selectable_label(!preview_mode,"Properties / Name").clicked() {
                        self.global_repository_dialog=Some((id,false));
                    }
                });
                ui.separator();
                ui.label(format!("Source: {} ({})",self.editor_cells.label(&CellSelection::Original(asset.source.clone())).unwrap_or(asset.source_label.clone()),asset.source));
                ui.label(format!("Record type: {}",asset.kind_label()));
                if asset.source!=self.native_scene.as_ref().map(|(name,_)|name.as_str()).unwrap_or("") {
                    ui.label("Foreign preview uses original donor textures. Native collision, scripts and IMG export still require resource import.");
                }
                ui.small(format!("Native record: {}",asset.label));
                ui.collapsing("Immutable original identifiers", |ui| {
                    ui.monospace(&asset.key);
                    ui.monospace(format!("Shared naming identity: {name_key}"));
                    if let Some(hash)=asset.mesh_fingerprint {ui.monospace(format!("TMD fingerprint: {hash:016x}"));}
                    if ui.button("Copy source identifier").clicked() {ui.ctx().copy_text(asset.key.clone());}
                });
                ui.separator();
                if preview_mode {
                    if let Some(model_preview)=self.global_repository_preview.as_ref() {
                        if let (Some(scene),Some(model))=(&model_preview.scene,&model_preview.model) {
                            ui.small(format!("Native source TMD • {} triangles • original scene VRAM",model.triangles()));
                            let available=ui.available_width().max(360.0);
                            ui.allocate_ui(egui::vec2(available,420.0),|ui| {
                                self.global_repository_preview_view.show_limited(
                                    ui,model,&scene.res.vram,800,600);
                            });
                        }
                        if let Some(issue)=&model_preview.issue {ui.label(issue);}
                    }
                    ui.small("Preview geometry and TIM palettes are from the donor scene; this does not add dependencies to the destination.");
                } else {
                    ui.heading("Asset identification");
                    ui.label("User-supplied label (not a verified original game name unless supported by evidence):");
                    ui.add(egui::TextEdit::singleline(&mut self.global_name_draft.name)
                        .hint_text("Name / label"));
                    egui::ComboBox::from_id_salt("lck_global_confidence")
                        .selected_text(self.global_name_draft.confidence.label())
                        .show_ui(ui,|ui| {
                            for confidence in Confidence::ALL {
                                ui.selectable_value(&mut self.global_name_draft.confidence,
                                    confidence,confidence.label());
                            }
                        });
                    ui.add(egui::TextEdit::singleline(&mut self.global_name_draft.category)
                        .hint_text("Category (e.g. architecture, foliage, props)"));
                    ui.add(egui::TextEdit::singleline(&mut self.global_name_draft.aliases)
                        .hint_text("Alternate names / search terms"));
                    ui.add(egui::TextEdit::multiline(&mut self.global_name_draft.evidence)
                        .desired_rows(3).hint_text("Evidence supporting identification"));
                    if asset.alias_key.is_none() {ui.colored_label(Color32::YELLOW,"Verified asset identity unavailable; existing legacy labels are read-only.");}
                    ui.add_enabled_ui(self.editor_catalog_error.is_none() && asset.alias_key.is_some(),|ui| {
                        ui.horizontal(|ui| {
                            let save=ui.button("Save identification").clicked();
                            let remove=ui.button("Restore default name").clicked();
                            if save||remove {
                                let mut next=self.editor_names.clone();
                                if remove {next.records.remove(&name_key);next.records.remove(&asset.key);}
                                else {
                                    let mut record=self.global_name_draft.clone();
                                    record.name=record.name.trim().to_owned();
                                    next.records.insert(name_key.clone(),record);
                                }
                                self.global_name_status=Some(match next.save(&editor_catalog_path()) {
                                    Ok(())=>{
                                        self.editor_names=next;
                                        if remove {self.global_name_draft=Record::default();}
                                        "Identification saved; original game assets unchanged.".into()
                                    }
                                    Err(e)=>format!("Could not save identification: {e}"),
                                });
                            }
                        });
                    });
                    if let Some(status)=&self.global_name_status {ui.label(status);}
                    ui.small("Shared aliases use verified original TMD + texture data, or a native MAN identity. Instance overrides stay in the LCK scene. No source identifiers are renamed.");
                }
            });
        if !open {self.global_repository_dialog=None;}
    }

    /// This panel intentionally contains no current-cell instance inventory;
    /// that already lives in the Scene hierarchy to the right.
    fn editor_catalog(&mut self,ui:&mut egui::Ui,
        scene: Option<&legaia_engine_scene::scene_assembly::AssembledScene>) {
        if ui.button("Export identification inventory...").clicked() {
            if let Some(path)=rfd::FileDialog::new().set_file_name("LCK_IdentificationInventory.tsv")
                .add_filter("Tab-separated inventory",&["tsv"]).save_file() {
                match std::fs::write(&path,self.editor_inventory(scene)) {
                    Ok(())=>self.studio.status=Some(format!("Inventory saved: {}",path.display())),
                    Err(e)=>self.studio.error=Some(format!("Cannot save inventory: {e}")),
                }
            }
        }
        egui::ScrollArea::vertical().id_salt("lck_global_assets_scroll")
            .drag_to_scroll(false).auto_shrink([false,false])
            .show(ui,|ui|self.editor_global_repository(ui,scene));
    }

    fn editor_catalog_popup(&mut self, ctx: &egui::Context,
                           scene: Option<&legaia_engine_scene::scene_assembly::AssembledScene>,
                           world: Option<&world_view::WorldModel>) {
        let Some((item, preview)) = self.editor_modal else { return; };
        // Preview models are built once per item; never rebuild the world scene
        // or re-decode an actor on each paint/tick.
        if preview && self.editor_preview.as_ref().is_none_or(|(key,_)| *key != item) {
            self.editor_preview_view = Viewport::new(Shading::Textured);
            let content = match item {
                EditorCatalogItem::Draw(terrain,i) => {
                    world.and_then(|w| w.at(terrain,i).and_then(|d|
                        w.model.as_ref().and_then(|m| editor_isolate_draw(m,d.id))))
                        .map(EditorPreview::Mesh)
                        .unwrap_or_else(|| EditorPreview::Unavailable("No drawable mesh in this cell".into()))
                }
                EditorCatalogItem::Mesh(slot) => {
                    scene.and_then(|s| editor_preview_reusable_tmd(s, slot))
                        .map(EditorPreview::Mesh)
                        .unwrap_or_else(|| EditorPreview::Unavailable("No decoded native mesh for this slot".into()))
                }
                EditorCatalogItem::Character(c) => self.studio.library.as_ref()
                    .ok_or_else(|| "Open a disc image first".to_string())
                    .and_then(|lib| crate::actors::character(lib,c,[0;5]).map_err(|e|e.to_string()))
                    .map(EditorPreview::Actor).unwrap_or_else(EditorPreview::Unavailable),
                EditorCatalogItem::Monster(id) => self.studio.library.as_ref()
                    .ok_or_else(|| "Open a disc image first".to_string())
                    .and_then(|lib| crate::actors::monster(lib,id).map_err(|e|e.to_string()))
                    .map(EditorPreview::Actor).unwrap_or_else(EditorPreview::Unavailable),
                EditorCatalogItem::Archive(_) => EditorPreview::Unavailable(
                    "This is a source archive record, not an independent renderable. Open it in Browse for the original bytes.".into()),
            };
            self.editor_preview = Some((item,content));
        }
        let mut open = true;
        let object_title = match item {
            EditorCatalogItem::Draw(terrain,i) => world.and_then(|w|w.at(terrain,i))
                .and_then(|d|d.display_name.clone())
                .unwrap_or_else(||self.editor_name_for(scene,item)),
            _=>self.editor_name_for(scene,item),
        };
        egui::Window::new(if preview {
            format!("Preview - {object_title}")
        } else {
            format!("Properties - {object_title}")
        })
            .id(egui::Id::new("editor_object_modal"))
            .open(&mut open)
            .resizable(true).default_size([660.0, 510.0])
            .show(ctx, |ui| {
                ui.small("Escape closes this window without changing the main camera.");
                ui.separator();
                if preview {
                    match self.editor_preview.as_ref().map(|(_,p)|p) {
                        Some(EditorPreview::Mesh(model)) => {
                            if let Some(s) = scene {
                                ui.label(format!("Native 3D mesh  •  {} triangles",model.triangles()));
                                ui.allocate_ui(egui::vec2(ui.available_width().max(300.0), 430.0), |ui| {
                                    self.editor_preview_view.show_limited(ui,model,&s.res.vram,800,600);
                                });
                            } else { ui.label("Select a cell to preview its original objects."); }
                        }
                        Some(EditorPreview::Actor(actor)) => {
                            ui.label(format!("{}  •  {} triangles",actor.label,actor.model.triangles()));
                            ui.allocate_ui(egui::vec2(ui.available_width().max(300.0),430.0), |ui| {
                                self.editor_preview_view.show_limited(ui,&actor.model,&actor.vram,800,600);
                            });
                        }
                        Some(EditorPreview::Unavailable(reason)) => { ui.label(reason); }
                        None => { ui.label("Preparing preview..."); }
                    }
                } else {
                    if !matches!(item, EditorCatalogItem::Draw(terrain,i)
                        if world.and_then(|w|w.at(terrain,i)).is_some_and(|d|d.editor_uid.is_some())) {
                        self.editor_name_properties(ui, scene, item);
                    }
                    match item {
                        EditorCatalogItem::Draw(terrain,i) => {
                            if let Some(npc)=world.and_then(|w|w.npc((terrain,i))) {
                                ui.heading(format!("{} field actor MAN #{}",
                                    if npc.is_copy {"Editor copy of"} else {"Original"},npc.record));
                                ui.label(format!("Model TMD index: {} | Animation ID: {} | class: {}",
                                    npc.model_index,npc.anim_id,npc.kind));
                                ui.label(format!("PSX starting position: X {:.0}, Y {:.0}, Z {:.0}",
                                    npc.psx_position[0],npc.psx_position[1],npc.psx_position[2]));
                                if let Some(dialog)=&npc.dialog {ui.label(format!("First dialog line (evidence, not identity): {dialog}"));}
                                ui.label(format!("Scene ANM clips with compatible bone count: {}",npc.clips.len()));
                                ui.label(format!("Current preview: ANM {} / frame {}",npc.clip,npc.frame));
                                if npc.conditional {ui.colored_label(Color32::YELLOW,
                                    "Script-conditional: not visible at initial scene load");}
                                if !npc.drawable {ui.label("No displayable initial mesh for this record.");}
                                else if npc.posed {ui.label("ANM first-frame pose applied.");}
                                ui.small("Field actor metadata is authentic; display names are not inferred from dialog.");
                            }
                            if let Some(s) = scene {
                                let rows = if terrain { &s.terrain } else { &s.placements };
                                if let Some(d) = rows.get(i) {
                                    ui.heading(self.editor_name_for(scene, item));
                                    ui.small(format!("{} original record #{i}", if terrain {"Terrain"}else{"Placement"}));
                                    ui.label(format!("Native mesh slot: {} / resource TMD {}",d.env_slot,d.res_tmd));
                                    ui.label(format!("PS1 position: X {}  Y {}  Z {}",d.world_x,d.world_y,d.world_z));
                                    ui.label(format!("PS1 rotation: X {}  Y {}  Z {}",d.rot_x,d.rot_y,d.rot_z));
                                    ui.label(format!("Cell: {:?}; anchor: {:?}; animation ID: {}",d.cell,d.anchor,d.anim_id));
                                }
                            }
                            if let Some(w) = world.and_then(|w|w.at(terrain,i)) {
                                ui.label(format!("Editor-only offset: X {:.1}  Y {:.1}  Z {:.1}",
                                    w.offset[0],w.offset[1],w.offset[2]));
                            }
                            ui.small("Edit offsets with the existing Placement Inspector; this window is informational.");
                        }
                        EditorCatalogItem::Mesh(slot) => {
                            ui.heading(format!("Reusable mesh #{slot}"));
                            if let Some(s)=scene {
                                if let Some(idx)=s.env_tmds.get(slot) {
                                    ui.label(format!("Scene resource TMD index: {idx}"));
                                }
                            }
                            ui.small("Drag this mesh into the viewport if the active scene has an original placed instance referencing it. Otherwise it needs a future direct-TMD instancer.");
                        }
                        EditorCatalogItem::Character(slot) => {
                            ui.heading(["Vahn","Noa","Gala","Terra"].get(slot).copied().unwrap_or("Character"));
                            ui.small(format!("Party character slot {slot}; battle model, not a field-placement record."));
                        }
                        EditorCatalogItem::Monster(id) => {
                            ui.heading(format!("Monster #{id}"));
                            if let Some(m)=self.editor_monsters.as_ref().and_then(|rows|rows.iter().find(|m|m.id==id)) {
                                ui.label(format!("{}  |  HP {}",m.name,m.hp));
                            }
                            ui.small("Battle actor asset; not yet placeable into a field scene.");
                        }
                        EditorCatalogItem::Archive(pos) => {
                            if let Some(lib)=self.studio.library.as_ref() {
                                if let Some(e)=lib.entries.get(pos) {
                                    ui.heading(format!("PROT entry #{}",e.index));
                                    ui.label(format!("Source size: {} bytes",e.size_bytes));
                                    ui.label(format!("Resource type: {}",self.studio.class_of(pos)
                                        .map(|c|c.name()).unwrap_or("Unknown")));
                                }
                            }
                            ui.small("Reference only. Browse preserves full extraction and hex inspection.");
                        }
                    }
                }
            });
        if !open { self.editor_modal=None; self.editor_preview=None; }
    }

    /// Native PS1 data browser: lists the original instance draws and their transforms.
    /// Editable scene preview with non-destructive mesh transforms.
    fn editor_workspace(&mut self, ctx: &egui::Context) {
        // Escape clears the current highlight and the inspector without
        // changing the camera or the native scene data. Leave the key visible
        // to a focused search field, which also uses Escape to clear its text.
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            if self.global_repository_dialog.take().is_some() {
                // Do not invalidate cached model: toggling Preview reuses it.
            } else if self.editor_modal.is_some() {
                self.editor_modal = None;
                self.editor_preview = None;
            } else if self.editor_context_menu.take().is_some() {
                // Escape dismisses the viewport context menu first.
            } else if self.editor_properties_target.take().is_some() {
                self.npc_playing = None;
            } else {
                self.native_selected = None; self.native_selection.clear(); self.native_hierarchy_anchor=None; self.native_selection_click=None;
                self.native_drag = None;
                self.native_rotate = None;
            }
            self.native_selected=None;self.native_selection.clear();self.native_hierarchy_anchor=None;self.native_selection_click=None;
            self.native_drag=None;self.native_rotate=None;
        }
        let mut pending_transform: Option<((bool, usize), [f32; 3], bool)> = None;
        let mut pending_rotation: Option<((bool, usize), [f32; 3], bool)> = None;
        let mut pending_scale: Option<((bool, usize), [f32; 3])> = None;
        let mut pending_npc_animation: Option<((bool,usize),u8,usize,bool)> = None;
        let mut pending_npc_model: Option<((bool,usize),u8)> = None;
        let mut pending_history_undo=false;
        let mut pending_history_redo=false;
        let mut pending_static_architecture:Option<(u64,bool)>=None;
        let mut pending_collision_edit:Option<((bool,usize),Option<collision_recipes::Recipe>)>=None;
        let mut finish_drag: Option<EditorDrag> = None;
        let mut finish_rotate: Option<EditorRotateDrag> = None;
        let mut pending_import: Option<Vec<((bool,usize),[f32;3])>> = None;
        let mut pending_project: Option<scene_instances::SceneDocument> = None;
        let mut pending_duplicate: Option<((bool, usize), [f32; 3], [f32; 3], Option<(u8,u8,usize)>)> = None;
        let mut pending_repository_drop = false;
        let mut pending_foreign: Option<(usize,[f32;2],[usize;2],bool)> = None;
        let mut pending_delete: Option<(bool,usize)> = None;
        let mut pending_restore: Option<(bool,usize)> = None;
        let mut pending_name: Option<((bool,usize),Option<String>)> = None;
        let mut hierarchy_popup: Option<(EditorCatalogItem,bool)> = None;
        let mut hierarchy_rename: Option<(bool,usize)> = None;
        let mut hierarchy_input=multi_selection::HierarchyInput::default();
        let ready = self.native_scene.as_ref().and_then(|(_, state)| match state {
            NativeSceneSlot::Ready(scene, _) => Some(Arc::clone(scene)),
            _ => None,
        });
        let world = self.native_scene.as_ref().and_then(|(_, state)| match state {
            NativeSceneSlot::Ready(_, world) => Some(Arc::clone(world)),
            _ => None,
        });
        egui::SidePanel::left("editor_object_window")
            .resizable(true)
            .default_width(305.0)
            .min_width(240.0)
            .show(ctx, |ui| {
                // A nested resizable bottom panel creates a persistent divider.
                // Its ScrollArea is independent of the top All Objects list.
                egui::TopBottomPanel::bottom("lck_editor_cell_split")
                    .resizable(true).default_height(205.0).min_height(105.0)
                    .show_inside(ui, |ui| self.editor_cells_panel(ui));
                self.editor_catalog(ui, ready.as_deref());
            });
        egui::SidePanel::right("native_instance_tree")
            .resizable(true)
            .default_width(300.0)
            .min_width(230.0)
            .show(ctx, |ui| {
                ui.heading("Scene hierarchy");
                ui.small("Click: single • Ctrl: toggle • Shift: range • Ctrl+Shift: add range");
                ui.label(format!("{} selected",multi_selection::keys(self.native_selected,&self.native_selection).len()));
                ui.add(egui::TextEdit::singleline(&mut self.editor_hierarchy_filter).hint_text("Filter names or identifiers"));
                let hierarchy_filter = self.editor_hierarchy_filter.trim().to_lowercase();
                ui.separator();
                match (&self.native_scene, &ready) {
                    (None, _) => { ui.label("Select a cell at left to assemble its native scene data."); }
                    (Some((name, NativeSceneSlot::Loading(_))), _) => {
                        ui.spinner();
                        ui.label(format!("Assembling {name}..."));
                    }
                    (Some((name, NativeSceneSlot::Failed(err))), _) => {
                        ui.colored_label(egui::Color32::LIGHT_RED, format!("{name}: {err}"));
                    }
                    (Some((source,_)), Some(scene)) => {
                        let display=self.editor_cell_active.as_ref()
                            .and_then(|id| self.editor_cells.label(id))
                            .unwrap_or_else(||crate::place_name(source).unwrap_or(source).to_owned());
                        ui.strong(display);
                        ui.small(format!("Original scene ID: {source}"));
                        ui.label(format!("Mesh slots: {}", scene.env_tmds.len()));
                        ui.label(format!("Placed objects: {}", scene.placements.len()));
                        ui.label(format!("Terrain tiles: {}", scene.terrain.len()));
                        ui.label(format!("Walk-ground: {}", if scene.ground.is_some() { "loaded" } else { "unavailable" }));
                        ui.separator();
                        egui::ScrollArea::vertical().id_salt("native_scene_instance_scroll").show(ui, |ui| {
                            egui::CollapsingHeader::new(format!("Placements ({})", scene.placements.len()))
                                .default_open(true).show(ui, |ui| {
                                    for (i, d) in scene.placements.iter().enumerate() {
                                        let key=(false,i);
                                        if world.as_ref().and_then(|w|w.at(false,i)).is_some_and(|d|d.hidden) {continue;}
                                        let default=self.editor_name_for(Some(scene),EditorCatalogItem::Draw(false,i));
                                        let custom=world.as_ref().and_then(|w|w.at(false,i)).and_then(|d|d.display_name.as_deref());
                                        let label=format!("{}  [#{i:03}]  ({}, {}, {})",
                                            custom.unwrap_or(&default),d.world_x,d.world_y,d.world_z);
                                        if !hierarchy_filter.is_empty() &&
                                            !label.to_lowercase().contains(&hierarchy_filter) &&
                                            !self.editor_item_matches(Some(scene),EditorCatalogItem::Draw(false,i),&hierarchy_filter) {continue;}
                                        editor_hierarchy_row(ui,label,key,&mut self.native_selected,&mut self.native_selection,
                                            &mut hierarchy_popup,&mut hierarchy_rename,&mut hierarchy_input);
                                    }
                                });
                            egui::CollapsingHeader::new(format!("Terrain ({})", scene.terrain.len()))
                                .show(ui, |ui| {
                                    for (i, d) in scene.terrain.iter().enumerate() {
                                        let key=(true,i);
                                        if world.as_ref().and_then(|w|w.at(true,i)).is_some_and(|d|d.hidden) {continue;}
                                        let default=self.editor_name_for(Some(scene),EditorCatalogItem::Draw(true,i));
                                        let custom=world.as_ref().and_then(|w|w.at(true,i)).and_then(|d|d.display_name.as_deref());
                                        let label=format!("{}  [#{i:03}]  cell ({}, {})",
                                            custom.unwrap_or(&default),d.cell.0,d.cell.1);
                                        if !hierarchy_filter.is_empty() &&
                                            !label.to_lowercase().contains(&hierarchy_filter) &&
                                            !self.editor_item_matches(Some(scene),EditorCatalogItem::Draw(true,i),&hierarchy_filter) {continue;}
                                        editor_hierarchy_row(ui,label,key,&mut self.native_selected,&mut self.native_selection,
                                            &mut hierarchy_popup,&mut hierarchy_rename,&mut hierarchy_input);
                                    }
                                });
                            if let Some(world) = &world {
                                egui::CollapsingHeader::new(format!("NPCs / field actors ({})",world.npcs.iter().filter(|n|!n.is_copy).count()))
                                    .default_open(true).show(ui, |ui| {
                                        for npc in world.npcs.iter().filter(|n|!n.is_copy) {
                                            let key=(false,npc.key);
                                            let Some(draw)=world.at(false,npc.key) else {continue};
                                            if draw.hidden {continue;}
                                            let status=if npc.conditional {" [conditional]"}
                                                else if !npc.drawable {" [no mesh]"} else {""};
                                            let label=format!("{}  [MAN #{:03} / TMD {}]{}",
                                                draw.display_name.clone().unwrap_or_else(||
                                                    self.editor_name_for(Some(scene),EditorCatalogItem::Draw(false,npc.key))),
                                                npc.record,npc.model_index,status);
                                            if !hierarchy_filter.is_empty() &&
                                                !label.to_lowercase().contains(&hierarchy_filter) &&
                                                !self.editor_item_matches(Some(scene),
                                                    EditorCatalogItem::Draw(false,npc.key),&hierarchy_filter) {continue;}
                                            editor_hierarchy_row(ui,label,key,&mut self.native_selected,&mut self.native_selection,
                                                &mut hierarchy_popup,&mut hierarchy_rename,&mut hierarchy_input);
                                        }
                                    });
                                if let Some(issue)=&world.npc_issue {
                                    ui.small(format!("NPC notes: {issue}"));
                                }
                                let created: Vec<_> = world.draws.iter()
                                    .filter(|d| d.editor_uid.is_some()).collect();
                                egui::CollapsingHeader::new(format!("New editor instances ({})", created.len()))
                                    .default_open(true).show(ui, |ui| {
                                        for d in created {
                                            let label=format!("{}  [copy #{}]",self.editor_name_for(Some(scene),EditorCatalogItem::Draw(d.terrain,d.index)),d.editor_uid.unwrap_or(0));
                                            if !hierarchy_filter.is_empty() &&
                                                !label.to_lowercase().contains(&hierarchy_filter) {continue;}
                                            editor_hierarchy_row(ui,label,(false,d.index),&mut self.native_selected,&mut self.native_selection,
                                                &mut hierarchy_popup,&mut hierarchy_rename,&mut hierarchy_input);
                                        }
                                    });
                                let hidden: Vec<_>=world.draws.iter().filter(|d|d.hidden && d.editor_uid.is_none()).collect();
                                if !hidden.is_empty() {
                                    egui::CollapsingHeader::new(format!("Hidden native objects ({})",hidden.len()))
                                        .show(ui,|ui| {
                                            for d in hidden {
                                                ui.horizontal(|ui| {
                                                    if ui.small_button("Restore").clicked() {
                                                        pending_restore=Some((d.terrain,d.index));
                                                    }
                                                    ui.label(format!("{} #{}: {}",if d.terrain {"Terrain"} else {"Placement"},
                                                        d.index,self.editor_name_for(Some(scene),EditorCatalogItem::Draw(d.terrain,d.index))))
                                                        .context_menu(|ui|{if ui.button("Rename").clicked(){hierarchy_rename=Some((d.terrain,d.index));ui.close();}});
                                                });
                                            }
                                        });
                                }
                            }
                            egui::CollapsingHeader::new(format!("Mesh references ({})", scene.env_tmds.len()))
                                .show(ui, |ui| {
                                    for (slot, idx) in scene.env_tmds.iter().enumerate() {
                                        if !self.editor_item_matches(Some(scene), EditorCatalogItem::Mesh(slot), &hierarchy_filter) { continue; }
                                        ui.label(format!("{}  [#{slot:03}]  (scene TMD {idx})",
                                            self.editor_name_for(Some(scene), EditorCatalogItem::Mesh(slot))));
                                    }
                                });
                        });
                    }
                    _ => { ui.label("Waiting for native scene data..."); }
                }
            });
        if hierarchy_input.apply(&mut self.native_selected,&mut self.native_selection,&mut self.native_hierarchy_anchor) {
            if self.editor_properties_target.is_some() {self.editor_properties_target=self.native_selected;}
            ctx.request_repaint();
        }
        self.editor_cell_dialogs(ctx);
        if let Some(modal)=hierarchy_popup {
            self.editor_modal=Some(modal);
            self.editor_preview=None;
        }
        // Hierarchy and repository Properties now share the same editable,
        // detached inspector instead of the old read-only asset popup.
        if let Some((EditorCatalogItem::Draw(terrain,index),false))=self.editor_modal {
            if world.as_ref().and_then(|w|w.at(terrain,index)).is_some() {
                self.native_selected=Some((terrain,index));
                self.editor_properties_target=Some((terrain,index));
                self.editor_context_menu=None;
            }
            self.editor_modal=None;
        }
        if let Some(key)=hierarchy_rename {
            if world.as_ref().and_then(|w|w.at(key.0,key.1)).is_some() {
                let default=self.editor_name_for(ready.as_deref(),EditorCatalogItem::Draw(key.0,key.1));
                self.editor_rename=Some(EditorRename {key,text:default});
            } else {
                self.studio.error=Some("This source record has no drawable instance to rename.".into());
            }
        }
        if let Some(edit)=&mut self.editor_rename {
            let mut open=true;
            let mut save=false;
            let mut cancel=false;
            egui::Window::new("Rename scene instance")
                .id(egui::Id::new("lck_scene_instance_rename"))
                .collapsible(false).resizable(false).open(&mut open).show(ctx,|ui| {
                    ui.label("Instance display name (editor-only; doesn't rename the PS1 asset):");
                    let response=ui.add(egui::TextEdit::singleline(&mut edit.text).desired_width(320.0));
                    save=ui.button("Save name").clicked() ||
                        (response.lost_focus() && ui.input(|i|i.key_pressed(egui::Key::Enter)));
                    if ui.button("Restore default name").clicked() {edit.text.clear();save=true;}
                    cancel=ui.button("Cancel").clicked();
                    ui.small("An empty name clears the custom label. Save LCK scene to persist it.");
                });
            if save {
                match scene_instances::valid_name(&edit.text) {
                    Ok(name) => { pending_name=Some((edit.key,name)); self.editor_rename=None; }
                    Err(error) => self.studio.error=Some(error),
                }
            } else if cancel || !open { self.editor_rename=None; }
        }
        // Selection must never move, zoom, or retarget the camera.
        // Press F to deliberately frame a selected piece instead.
        // Stage O: a floating, scrollable Properties window leaves every pixel
        // of the central viewport available for PS1 scene editing. The target
        // is fixed when the window opens; clicking elsewhere won't hijack it.
        if let Some(properties_key)=self.editor_properties_target {
            if world.as_ref().and_then(|w|w.at(properties_key.0,properties_key.1)).is_none() {
                self.editor_properties_target=None;
                self.npc_playing=None;
            } else {
                let title=world.as_ref().and_then(|w|w.at(properties_key.0,properties_key.1))
                    .and_then(|d|d.display_name.clone())
                    .unwrap_or_else(||self.editor_name_for(ready.as_deref(),
                        EditorCatalogItem::Draw(properties_key.0,properties_key.1)));
                let mut window_open=true;
                egui::Window::new(format!("Properties - {title}"))
                    .id(egui::Id::new("lck_scene_object_properties"))
                    .default_pos([380.0, 170.0])
                    .default_size([650.0, 525.0])
                    .min_size([375.0, 240.0])
                    .resizable(true)
                    .open(&mut window_open)
                    .show(ctx, |ui| {
                        let count=multi_selection::keys(self.native_selected,&self.native_selection).len();
                        ui.strong(format!("{count} selected • primary object shown"));
                        ui.small("Placement changes apply to the selection. Rotation uses each object's pivot; scale uses a shared ratio. Native export validates every edited object.");
                        if let Some(foreign)=world.as_ref().and_then(|w|w.at(properties_key.0,properties_key.1)
                            .and_then(|d|d.editor_uid)
                            .and_then(|uid|w.foreign.iter().find(|f|f.uid==uid))) {
                            ui.label(format!("Foreign original: {} / {} #{} — editor preview; native resource/collision import remains unresolved.",
                                foreign.source,foreign.kind,foreign.record));
                        }
                        ui.separator();
                        egui::ScrollArea::vertical().id_salt(("lck_properties_scroll_p1",properties_key))
                            .auto_shrink([false,false]).max_height(ui.available_height().max(140.0)).show(ui, |ui| {
                if let Some(world)=world.as_deref() {
                    if ui.button("Rename this instance...").clicked() {self.editor_rename=Some(EditorRename{key:properties_key,text:self.editor_name_for(ready.as_deref(),EditorCatalogItem::Draw(properties_key.0,properties_key.1))});}
                    ui.collapsing("Naming scope and original identifiers",|ui| {
                        let draw=world.at(properties_key.0,properties_key.1).unwrap();
                        ui.label(if draw.display_name.is_some(){"Scope: instance override in this LCK scene"}else{"Scope: shared asset alias or default label"});
                        if let Some(uid)=draw.editor_uid {ui.monospace(format!("Persistent instance UID {uid}"));}
                        else if let Some(id)=world.naming.native.get(&properties_key) {ui.monospace(id);}
                        else {ui.colored_label(Color32::YELLOW,"Stable native identity unavailable; new original-instance naming is disabled.");}
                        if let Some(id)=world.base_name_key(properties_key) {ui.monospace(format!("Shared alias identity: {id}"));}
                    });
                    let scene_name=ready.as_ref().map(|s|s.name.clone()).unwrap_or_default();
                    let target=world.at(properties_key.0,properties_key.1).map(|d|match d.editor_uid {
                        Some(uid)=>scene_instances::NameTarget::Created(uid),None=>scene_instances::NameTarget::Native(properties_key.0,properties_key.1)});
                    let mut visible=self.object_collision_draft_visible && match &self.generated_collision_target {
                        Some((name,id))=>name==&scene_name && Some(id)==target.as_ref(),
                        None=>self.native_selected==Some(properties_key) && collision_recipes::generated_for(world,properties_key).is_some(),
                    };
                    let before=visible;
                    if let Some(edit)=collision_recipes::show_properties(ui,world,properties_key,
                        self.studio.library.as_ref().map(|lib|lib.index.as_ref()),&mut visible) {
                        pending_collision_edit=Some((properties_key,edit));
                    }
                    if visible!=before {
                        self.object_collision_draft_visible=visible;
                        self.generated_collision_target=if visible {target.map(|target|(scene_name,target))}else{None};
                        ctx.request_repaint();
                    }
                    ui.separator();
                }
                if let (Some(scene),Some(world),Some(lib))=(&ready,&world,&self.studio.library) {
                    ui.collapsing("Native dependencies (read only)",|ui| {
                        native_dependency_report::show(ui,scene,world,properties_key,&lib.index);
                    });
                }
                if let Some(key)=Some(properties_key) {
                    if let Some(draw)=world.as_ref().and_then(|w|w.at(key.0,key.1)) {
                        if draw.vertex_start < draw.vertex_end && !draw.hidden {
                            ui.horizontal_wrapped(|ui| {
                                ui.label("Rotation (degrees)");
                                for (axis,label) in [(0,"X tilt"),(1,"Y spin"),(2,"Z roll")] {
                                    let mut value=draw.rotation[axis].to_degrees();
                                    ui.label(label);
                                    if ui.add(egui::DragValue::new(&mut value).speed(0.5).max_decimals(1)).changed() {
                                        let mut next=draw.rotation;
                                        next[axis]=value.to_radians();
                                        pending_rotation=Some((key,next,true));
                                    }
                                }
                                if ui.small_button("Reset rotation").clicked() {
                                    pending_rotation=Some((key,[0.0;3],true));
                                }
                            });
                            ui.horizontal_wrapped(|ui| {
                                ui.label("Scale (1.00 = original)");
                                let mut uniform=draw.scale[0];
                                ui.label("Uniform");
                                if ui.add(egui::DragValue::new(&mut uniform).speed(0.01)
                                    .range(0.05..=20.0).max_decimals(3)).changed() {
                                    pending_scale=Some((key,[uniform;3]));
                                }
                                for (axis,label) in [(0,"X"),(1,"Y"),(2,"Z")] {
                                    let mut value=draw.scale[axis];
                                    ui.label(label);
                                    if ui.add(egui::DragValue::new(&mut value).speed(0.01)
                                        .range(0.05..=20.0).max_decimals(3)).changed() {
                                        let mut next=draw.scale;
                                        next[axis]=value;
                                        pending_scale=Some((key,next));
                                    }
                                }
                                if ui.small_button("Reset scale").clicked() {
                                    pending_scale=Some((key,[1.0;3]));
                                }
                            });
                            ui.small("RMB: rotate Y | Alt+RMB: rotate X | Scale 0.05-20x. Save .lckscene to persist.");
                        }
                    }
                }
                if let (Some(world),Some(key))=(&world,Some(properties_key)) {
                    if let Some(npc)=world.npc(key) {
                        ui.separator();
                        ui.strong(format!("Native NPC / field actor #{}",npc.record));
                        ui.label(format!("MAN {} | TMD {} | ANM {} | {}",
                            npc.record,npc.model_index,npc.anim_id,npc.kind));
                        ui.label(format!("PSX spawn X {:.0} Y {:.0} Z {:.0}",
                            npc.psx_position[0],npc.psx_position[1],npc.psx_position[2]));
                        if let Some(dialog)=&npc.dialog {
                            ui.label(format!("Dialog evidence (speaker unverified): {dialog}"));
                        }
                        if npc.is_copy { ui.small("Editor NPC copy: editable and saved in LCK scene overlay."); }
                        if npc.conditional {ui.small("Conditional script spawn; initial visibility off.");}
                        if !npc.drawable { ui.small("Static actor mesh unavailable; animation editing disabled."); }
                        if npc.drawable {
                            ui.horizontal_wrapped(|ui| {
                                ui.label("Appearance (cell TMD)");
                                let mut model=npc.model_index;
                                egui::ComboBox::from_id_salt(("npc_model",npc.key))
                                    .selected_text(format!("TMD {}",npc.model_index))
                                    .show_ui(ui,|ui| {
                                        for (index,entry) in npc.model_pool.iter().enumerate() {
                                            if let Some((_,parts))=entry {
                                                ui.selectable_value(&mut model,index as u8,
                                                    format!("TMD {index:03} ({parts} parts)"));
                                            }
                                        }
                                    });
                                if model!=npc.model_index {
                                    self.npc_playing=None;
                                    pending_npc_model=Some((key,model));
                                }
                                if ui.small_button("Original model").clicked() {
                                    self.npc_playing=None;
                                    pending_npc_model=Some((key,npc.original_model_index));
                                }
                            });
                            ui.small("Model changes reuse original cell TMDs and textures. Incompatible multipart models are refused.");
                        }
                        if npc.drawable && !npc.clips.is_empty() {
                            ui.horizontal_wrapped(|ui| {
                                ui.label("Animation (native PS1 ANM)");
                                let mut clip=npc.clip;
                                egui::ComboBox::from_id_salt(("npc_clip",npc.key))
                                    .selected_text(format!("ANM {}",npc.clip)).show_ui(ui,|ui| {
                                        if npc.nobj==1 {
                                            ui.selectable_value(&mut clip,0,"Rest pose");
                                        }
                                        for &(id,frames) in &npc.clips {
                                            ui.selectable_value(&mut clip,id,
                                                format!("ANM {id} ({frames} frames)"));
                                        }
                                    });
                                if clip!=npc.clip {
                                    self.npc_playing=None;
                                    pending_npc_animation=Some((key,clip,0,true));
                                }
                                let count=npc.frames_for(clip).unwrap_or(1);
                                let mut frame=if clip==npc.clip {npc.frame.min(count-1)} else {0};
                                ui.label("Frame");
                                if ui.add(egui::Slider::new(&mut frame,0..=count-1)
                                    .show_value(true)).changed() {
                                    self.npc_playing=None;
                                    pending_npc_animation=Some((key,clip,frame,true));
                                }
                                if self.npc_playing==Some(key) {
                                    if ui.button("Pause").clicked() {self.npc_playing=None;}
                                } else if ui.button("Play").clicked() {
                                    self.npc_playing=Some(key);
                                    self.npc_next_tick=ctx.input(|i|i.time)+1.0/15.0;
                                }
                                if ui.small_button("Original pose").clicked() {
                                    self.npc_playing=None;
                                    pending_npc_animation=Some((key,npc.anim_id,0,true));
                                }
                            });
                            if self.npc_playing==Some(key) {
                                let now=ctx.input(|i|i.time);
                                if now>=self.npc_next_tick {
                                    let frames=npc.frames_for(npc.clip).unwrap_or(1);
                                    if frames>1 {
                                        pending_npc_animation=Some((key,npc.clip,(npc.frame+1)%frames,false));
                                    }
                                    self.npc_next_tick=now+1.0/15.0;
                                }
                                ctx.request_repaint_after(std::time::Duration::from_millis(67));
                            }
                        }
                        ui.small("Animation is a static/looping editor preview. Clips are indexed by verified bone-count compatibility, NOT named idle/walk/talk without supporting evidence.");
                        ui.small("Preview is editable. Save .lckscene to persist; Export PS1 test IMG to test supported native edits.");
                    }
                }
                if let (Some(scene), Some((terrain, i))) = (&ready, Some(properties_key)) {
                    let records = if terrain { &scene.terrain } else { &scene.placements };
                    if let Some(d) = records.get(i) {
                        ui.label(format!("{}  [#{i:03}] | {} | model slot {} | res TMD {} | anchor {:?} | source cell {:?}",
                            world.as_ref().and_then(|w|w.at(terrain,i))
                                .and_then(|d|d.display_name.clone())
                                .unwrap_or_else(||self.editor_name_for(Some(scene), EditorCatalogItem::Draw(terrain,i))),
                            if terrain { "Terrain" } else { "Placement" }, d.env_slot, d.res_tmd, d.anchor, d.cell));
                        ui.label(format!("Original PSX coordinates: X={} Y={} Z={}", d.world_x, d.world_y, d.world_z));
                        ui.label(format!("Original PSX rotations: X={} Y={} Z={} | anim ID={}", d.rot_x, d.rot_y, d.rot_z, d.anim_id));
                        ui.small("Native coordinates are shown for reference. Edited offsets use +Y upward; PS1 export support depends on object type.");
                        if let Some(world) = &world {
                            if let Some(draw) = world.at(terrain, i) {
                                ui.horizontal_wrapped(|ui| {
                                    ui.selectable_value(&mut self.native_move_mode, EditorMoveMode::Fluid, "Fluid");
                                    ui.selectable_value(&mut self.native_move_mode, EditorMoveMode::Grid, "Grid snap");
                                    if self.native_move_mode == EditorMoveMode::Grid {
                                        egui::ComboBox::from_id_salt("native_grid_step")
                                            .selected_text(format!("{} units", self.native_grid_size))
                                            .show_ui(ui, |ui| {
                                                for size in [1.0, 2.0, 4.0, 8.0, 16.0, 32.0, 64.0, 128.0, 256.0, 512.0] {
                                                    ui.selectable_value(&mut self.native_grid_size, size, format!("{size}"));
                                                }
                                            });
                                    }
                                    ui.checkbox(&mut self.native_anchor_snap, "Magnet: object centers")
                                        .on_hover_text("Optionally lock near another piece's center, within half the selected grid size. Disabled by default.");
                                    egui::ComboBox::from_id_salt("geck_drag_axis")
                                        .selected_text(match self.geck_drag_axis {
                                            Some(0) => "Drag X only",
                                            Some(2) => "Drag Z only",
                                            _ => "Drag free X/Z",
                                        })
                                        .show_ui(ui, |ui| {
                                            ui.selectable_value(&mut self.geck_drag_axis, None, "Free X/Z");
                                            ui.selectable_value(&mut self.geck_drag_axis, Some(0), "X only");
                                            ui.selectable_value(&mut self.geck_drag_axis, Some(2), "Z only");
                                        });
                                    egui::ComboBox::from_id_salt("geck_angle_snap")
                                        .selected_text(if self.geck_angle_snap <= 0.0 {
                                            "Angle: off".to_owned()
                                        } else {
                                            format!("Angle: {}°", self.geck_angle_snap)
                                        })
                                        .show_ui(ui, |ui| {
                                            for (value, label) in [(0.0,"Off"),(5.0,"5°"),(15.0,"15°"),(45.0,"45°"),(90.0,"90°")] {
                                                ui.selectable_value(&mut self.geck_angle_snap, value, label);
                                            }
                                        });
                                });
                                ui.small("Snap grid starts at THIS piece's original position, so zero offset restores the original placement.");
                                ui.horizontal_wrapped(|ui| {
                                    for (axis, label) in [(0,"X"), (1,"Y"), (2,"Z")] {
                                        let mut value = draw.offset[axis];
                                        ui.label(label);
                                        let changed = ui.add(egui::DragValue::new(&mut value).speed(1.0).max_decimals(2)).changed();
                                        if changed {
                                            let mut next = draw.offset;
                                            next[axis] = value;
                                            pending_transform = Some(((terrain, i), next, false));
                                        }
                                    }
                                    if ui.button("Reset position").clicked() {
                                        pending_transform = Some(((terrain, i), [0.0; 3], true));
                                    }
                                });
                                ui.horizontal_wrapped(|ui| {
                                    let step = self.native_grid_size;
                                    for (axis, label) in [(0,"X"), (1,"Y"), (2,"Z")] {
                                        for (direction, symbol) in [(-1.0,"-"),(1.0,"+")] {
                                            if ui.small_button(format!("{symbol}{label}")).clicked() {
                                                let mut next = draw.offset;
                                                next[axis] += direction * step;
                                                pending_transform = Some(((terrain, i), next, true));
                                            }
                                        }
                                    }
                                });
                                ui.small("Left-drag starting ON selected object: move X/Z | Ctrl+left-drag on object: height (Y) | Left-drag elsewhere: pan");
                            } else { ui.small("This native record has no drawable mesh to move."); }
                        }
                    } else if let Some(d) = world.as_ref().and_then(|w| w.at(terrain,i)) {
                        if d.editor_uid.is_some() {
                            let origin = d.source_ref.unwrap_or((false,0));
                            ui.strong(format!("Editor-created instance #{}", d.editor_uid.unwrap_or(0)));
                            ui.label(format!("Source: {} #{} | editor index {}",
                                if origin.0 { "Terrain" } else { "Placement" }, origin.1, i));
                            ui.small("Editor copy. Save as .lckscene or use Export PS1 test IMG for supported native instances.");
                            ui.horizontal_wrapped(|ui| {
                                for (axis,label) in [(0,"X"),(1,"Y"),(2,"Z")] {
                                    let mut value=d.offset[axis];
                                    ui.label(label);
                                    if ui.add(egui::DragValue::new(&mut value).speed(1.0).max_decimals(2)).changed() {
                                        let mut next=d.offset;
                                        next[axis]=value;
                                        pending_transform=Some(((terrain,i),next,true));
                                    }
                                }
                            });
                            ui.small("Drag selected object directly to move it. Drag empty space to pan. Ctrl+Z undoes changes including duplicate/delete, rotation and scale.");
                        }
                    }
                } else if let (Some(lib), Some(pos)) = (&self.studio.library, self.studio.selected) {
                    if let Some(e) = lib.entries.get(pos) {
                        ui.label(format!("Selected archive entry: PROT #{} | {}", e.index, lib.label(e.index).unwrap_or("unnamed")));
                    }
                    ui.small("Select a native instance in the right hierarchy for its placement transform.");
                } else {
                    ui.label("Select a cell, then choose a placed object or terrain tile in the hierarchy.");
                }
                        });
                    });
                if !window_open {
                    self.editor_properties_target=None;
                    self.npc_playing=None;
                }
            }
        }
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("World Viewport");
            if let (Some(scene), Some(world)) = (&ready, &world) {
                ui.small(format!("Native '{}' | {} environmental instances | {} unique TMDs | {} unavailable",
                    scene.name, world.instances, world.unique_models, world.unavailable));
                ui.small(format!("MAN field actors: {} catalogued | {} initially rendered | {} conditional",
                    world.npcs.iter().filter(|n|!n.is_copy).count(),
                    world.npcs.iter().filter(|n|!n.is_copy && n.drawable).count(),
                    world.npcs.iter().filter(|n|!n.is_copy && n.conditional).count()));
                // Stage Q7: collision sources are different for visible terrain,
                // walls, NPCs and walkable floor. A yellow/tan TMD preview must
                // never be represented as having native PS1 collision merely
                // because another cell's wall bits overlap its silhouette.
                if let Some(foreign) = self.native_selected
                    .and_then(|(terrain, index)| world.at(terrain,index))
                    .and_then(|draw|draw.editor_uid)
                    .and_then(|uid|world.foreign.iter().find(|f|f.uid==uid)) {
                    let wall_count=foreign.donor_wall_bits.len();
                    let message=match foreign.kind.as_str() {
                        "terrain" => format!("Selected foreign terrain UID {}: {} source MAP wall samples inside its footprint (NOT object-owned). Imported mesh is NOT native walk-ground; floor heightfield still belongs to destination. IMG export unsupported.",foreign.uid,wall_count),
                        "placement" => format!("Selected foreign MAP object UID {}: original 32-byte placement {}. {} inferred source wall samples (NOT proven object-owned). Native IMG export requires target resource compatibility.",foreign.uid,if foreign.native_record.is_some() {"captured"} else {"unverified"},wall_count),
                        "mesh" => format!("Selected foreign raw TMD UID {}: no native placement/wall ownership. Original texture data is editor-only until scene-resource transplantation is implemented.",foreign.uid),
                        _ => format!("Selected foreign {} UID {}: actor or other collision is not represented by static MAP walls; native IMG import requires dedicated support.",foreign.kind,foreign.uid),
                    };
                    ui.colored_label(Color32::from_rgb(225,175,70),message)
                        .on_hover_text("This is a source-provenance diagnostic, NOT a claim that the destination PS1 engine will collide with or walk on this object. The retail IMG is unchanged.");
                    let mut architecture=foreign.static_architecture;
                    if ui.checkbox(&mut architecture,"Static architecture copy — no donor story scripts or animation").changed(){pending_static_architecture=Some((foreign.uid,architecture));}
                    ui.small("Copies the visible raw mesh. Elevator movement, interactions and story behavior are detached; collision remains separately reported. New foreign drops use this mode.");
                }
                ui.checkbox(&mut self.unrestricted_img_export,"Unrestricted IMG Export Mode (experimental)")
                    .on_hover_text("Attempt available native writers without compatibility preflight. Each export creates debug.txt with warnings and exact failures; an IMG may be incomplete.");
                if let Some(path)=self.ps1_export_output.clone(){ui.horizontal_wrapped(|ui|{
                    if ui.button("Open IMG output folder").clicked(){if let Some(parent)=path.parent(){self.show_folder(parent);}}
                    ui.label(format!("IMG output: {}",path.display()));
                });}
                if let Some(coverage)=self.ps1_export_imports {if let Some(message)=coverage.failure_message(){ui.colored_label(Color32::from_rgb(255,90,70),egui::RichText::new(message).strong());}}
                ui.horizontal_wrapped(|ui| {
                    self.native_view.controls(ui, "Sample original PS1 TIM/CLUT atlas from assembled scene VRAM");
                    ui.checkbox(&mut self.native_ground_visible, "Show walk-ground")
                        .on_hover_text("Original per-cell PS1 walkable terrain: UV, CLUT and texture page are read from the disc's native heightfield and scene VRAM.");
                    ui.checkbox(&mut self.native_floor_debug_visible, "Native floor comparison");
                    ui.checkbox(&mut self.native_collision_wire_visible, "Collision wireframe")
                        .on_hover_text("View native MAP wall collision at its actual 64-unit resolution. Orange: original walls; cyan: newly positioned wall cells for supported object moves/copies. Does not modify the disc. NPC collider shapes and script-painted walls are not shown.");
                    if ui.checkbox(&mut self.object_collision_draft_visible, "Selected mesh collider DRAFT")
                        .on_hover_text("Shows saved generated collision when available, otherwise the selected mesh draft. Purple = steep faces; green = unverified floor candidates. Separate from native MAP walls.").changed() {
                        self.generated_collision_target=None;
                    }
                    ui.checkbox(&mut self.native_view.world_mirrored, "Mirror view (X)")
                        .on_hover_text("Checked by default to restore your previous left/right view. Display only: native PS1 world coordinates, geometry and textured materials are not modified. Uncheck to compare orientations.");
                    if ui.button("Frame all").clicked() { self.native_view.frame_all(); }
                    ui.menu_button("Camera marks", |ui| {
                        ui.small("Session-only; no PS1 or .lckscene changes.");
                        for index in 0..3 {
                            ui.horizontal(|ui| {
                                if ui.small_button(format!("Save {}", index + 1)).clicked() {
                                    self.geck_camera_bookmarks[index] = Some(self.native_view.cam);
                                    self.studio.status = Some(format!("Camera bookmark {} saved for this session.", index + 1));
                                    ui.close();
                                }
                                if ui.add_enabled(self.geck_camera_bookmarks[index].is_some(),
                                    egui::Button::new(format!("Go {}", index + 1))).clicked() {
                                    if let Some(cam) = self.geck_camera_bookmarks[index] {
                                        self.native_view.cam = cam;
                                        self.native_view.version = self.native_view.version.wrapping_add(1);
                                        ui.ctx().request_repaint();
                                    }
                                    ui.close();
                                }
                            });
                        }
                    });
                    if ui.button("Save layout...").on_hover_text("Save editor move offsets as a separate .lgmove file; doesn't write to game disc.").clicked() {
                        if let Some((name, _)) = &self.native_scene {
                            if let Some(path) = rfd::FileDialog::new().set_file_name(format!("{name}.lgmove"))
                                .add_filter("Legaia placement sidecar", &["lgmove"]).save_file() {
                                let path = path.with_extension("lgmove");
                                match std::fs::write(&path, serialize_editor_offsets(name, world)) {
                                    Ok(()) => self.studio.status = Some(format!("Saved preview placement overrides: {}", path.display())),
                                    Err(err) => self.studio.error = Some(format!("Could not save placement file: {err}")),
                                }
                            }
                        }
                    }
                    if ui.button("Open layout...").on_hover_text("Load a saved .lgmove offset overlay for this cell; doesn't change PS1 disc.").clicked() {
                        if let Some((name, _)) = &self.native_scene {
                            if let Some(path) = rfd::FileDialog::new().add_filter("Legaia placement sidecar", &["lgmove"]).pick_file() {
                                let parsed = std::fs::read_to_string(&path)
                                    .map_err(|err| err.to_string())
                                    .and_then(|text| parse_editor_offsets(&text, name, world));
                                match parsed {
                                    Ok(edits) => pending_import = Some(edits),
                                    Err(err) => self.studio.error = Some(format!("Layout not loaded: {err}")),
                                }
                            }
                        }
                    }
                    if ui.add_enabled(!self.editor_undo.is_empty() || self.editor_gesture_start.is_some(),
                        egui::Button::new("Undo (Ctrl+Z)")).clicked() {
                        pending_history_undo=true;
                    }
                    if ui.add_enabled(!self.editor_redo.is_empty(),
                        egui::Button::new("Redo (Ctrl+Y)")).clicked() {
                        pending_history_redo=true;
                    }
                    if ui.button("Save LCK scene...").on_hover_text("Save original move offsets AND new editor-created instances to an .lckscene file. Does NOT modify the PS1 disc.").clicked() {
                        if let Some((name, _)) = &self.native_scene {
                            if let Some(path) = rfd::FileDialog::new().set_file_name(format!("{name}.lckscene"))
                                .add_filter("LCK scene overlay", &["lckscene"]).save_file() {
                                let path = path.with_extension("lckscene");
                                match scene_instances::save(&path,name,world) {
                                    Ok(()) => self.studio.status = Some(format!("Saved LCK editor project: {}", path.display())),
                                    Err(err) => self.studio.error = Some(format!("Could not save LCK project: {err}")),
                                }
                            }
                        }
                    }
                    if let Some((terrain, index)) = self.native_selected {
                        if let Some(draw) = world.at(terrain, index) {
                            if ui.button("Copy transform")
                                .on_hover_text("Copy editor RELATIVE offset, local rotation and scale; original MAP anchor is not copied.")
                                .clicked() {
                                self.geck_transform_clipboard = Some(GeckTransformClipboard {
                                    offset: draw.offset, rotation: draw.rotation, scale: draw.scale,
                                });
                                self.studio.status = Some("Editor transform copied (relative offset + rotation + scale).".into());
                            }
                            if ui.add_enabled(self.geck_transform_clipboard.is_some(),
                                egui::Button::new("Paste transform"))
                                .on_hover_text("Apply copied local editor transforms. Native PS1 export may reject rotation/scale; use Check IMG before exporting.")
                                .clicked() {
                                if let Some(copy) = self.geck_transform_clipboard {
                                    let key = (terrain, index);
                                    pending_transform = Some((key, copy.offset, true));
                                    pending_rotation = Some((key, copy.rotation, true));
                                    pending_scale = Some((key, copy.scale));
                                    self.studio.status = Some("Pasted editor transform. Use Check IMG to verify native support.".into());
                                }
                            }
                        }
                    }
                    if ui.button("Check IMG")
                        .on_hover_text("Unrestricted mode bypasses compatibility preflight. With the checkbox off, run strict preparation without writing a file. Neither mode verifies gameplay.")
                        .clicked() {
                        let result = (|| -> anyhow::Result<String> {
                            let lib = self.studio.library.as_ref()
                                .ok_or_else(||anyhow::anyhow!("No disc image loaded"))?;
                            let Source::Disc(source)=&lib.source else {
                                anyhow::bail!("Open the native PS1 disc source first");
                            };
                            if self.unrestricted_img_export {return Ok("Compatibility preflight disabled. Export attempts all manifest operations; debug.txt records written fields and exact failures.".into());}
                            let plan=ps1_export::plan_export(source,scene,world,lib.index.as_ref())?;
                            Ok(plan.summary())
                        })();
                        match result {
                            Ok(summary) if self.unrestricted_img_export => self.studio.status=Some(summary),
                            Ok(summary) => self.studio.status = Some(format!(
                                "PS1 serialization preflight PASSED for {}: {summary}. No IMG written; PS1 runtime behavior remains unverified.", scene.name)),
                            Err(e) => self.studio.error = Some(format!(
                                "PS1 export preflight BLOCKED for {}: {e:#}", scene.name)),
                        }
                    }
                    if ui.add_enabled(self.ps1_export_job.is_none(), egui::Button::new("Export PS1 test IMG..."))
                        .on_hover_text("Unrestricted mode attempts native writes without compatibility gates and creates per-export debug.txt. Failed operations produce an explicitly partial IMG; protected source files are never overwritten.")
                        .clicked() {
                        let prepared = (|| -> anyhow::Result<(PathBuf,String)> {
                            let lib=self.studio.library.as_ref().ok_or_else(||anyhow::anyhow!("No disc image loaded"))?;
                            let source=match &lib.source {
                                Source::Disc(path) => path.clone(),
                                Source::Extracted(_) => anyhow::bail!("Open the actual 2352-byte test .img, not extracted assets"),
                            };
                            let name=&scene.name;
                            // Integration V1: capture the COMPLETE editor change manifest
                            // before native preflight. Names remain editor-side metadata.
                            let overlay=scene_instances::serialize(name,world);
                            if !self.unrestricted_img_export && export_integration::only_editor_labels(&overlay) {
                                let report=export_integration::render_report(name,&overlay,Some(&source));
                                let note="Only hierarchy labels changed: no native PS1 data needs writing. Save the .lckscene to preserve the labels; IMG unchanged.";
                                let log_path=export_integration::write_log(name,&report,note)
                                    .map(|p|p.display().to_string()).unwrap_or_else(|e|e);
                                anyhow::bail!("{note} Integration V1 log: {log_path}");
                            }
                            Ok((source,name.to_string()))
                        })();
                        match prepared {
                            Ok((source,cell_name)) => {
                                let suggestion=format!("{}_LCK_{}.img",
                                    source.file_stem().and_then(|s|s.to_str()).unwrap_or("LegaiaTest"),cell_name);
                                let mut dialog=rfd::FileDialog::new().set_file_name(&suggestion)
                                    .add_filter("Raw PS1 test image", &["img"]);
                                if let Some(parent)=source.parent() {dialog=dialog.set_directory(parent);}
                                if let Some(destination)=dialog.save_file() {
                                    self.ps1_export_output=Some(destination.clone());
                                    self.ps1_export_imports=None;
                                    self.studio.error=None;
                                    let output_message=destination.display().to_string();
                                    let overlay=scene_instances::serialize(&cell_name,world);
                                    let report=export_integration::render_report(&cell_name,&overlay,Some(&source));
                                    let (tx,rx)=std::sync::mpsc::channel();
                                    let repaint=ctx.clone();
                                    let export_scene=Arc::clone(scene);let export_world=world.as_ref().clone();
                                    let export_index=Arc::clone(&self.studio.library.as_ref().unwrap().index);
                                    let unrestricted=self.unrestricted_img_export;
                                    std::thread::spawn(move|| {
                                        let result=std::panic::catch_unwind(std::panic::AssertUnwindSafe(||->anyhow::Result<ps1_export::ExportResult>{
                                            let plan=if unrestricted {ps1_export::unrestricted::prepare(&source,&export_scene,&export_world,&export_index)}else{ps1_export::plan_export(&source,&export_scene,&export_world,&export_index)}?;
                                            ps1_export::export(plan,&destination)
                                        })).unwrap_or_else(|_|Err(anyhow::anyhow!("Native export worker panicked; no completed output claimed"))).map_err(|error|{
                                            let message=format!("Output {}: {error:#}",destination.display());
                                            let debug=ps1_export::unrestricted::failure_log(&destination,&overlay,&message)
                                                .map(|p|p.display().to_string()).unwrap_or_else(|e|format!("debug.txt could not be written: {e:#}"));
                                            anyhow::anyhow!("{message}. {debug}")
                                        });
                                        let outcome=match &result {
                                            Ok(done)=>format!("{}\nPatched: {}\nSkipped: {}",if done.imports.is_some(){done.experimental_summary()}else{format!("Native IMG file verified: {}",done.path.display())},done.patched.join(" | "),done.skipped.join(" | ")),
                                            Err(error)=>format!("NATIVE EXPORT FAILED: {}: {error:#}",destination.display()),
                                        };
                                        if let Err(error)=export_integration::write_log(&cell_name,&report,&outcome) {
                                            log::warn!("Export Integration V1 diagnostic log unavailable: {error}");
                                        }
                                        let _=tx.send(result);
                                        repaint.request_repaint();
                                    });
                                    self.ps1_export_job=Some(rx);
                                    self.studio.status=Some(format!("Writing IMG to {output_message}. Completion requires file existence, length and disk readback verification; native operation failures remain in debug.txt."));
                                }
                            }
                            Err(error) => {
                                if error.to_string().starts_with("Only hierarchy labels changed:") {
                                    self.studio.error=None;
                                    // An editor-only rename is not an invalid IMG. No native
                                    // changes exist to compile; the LCK scene keeps the name.
                                    self.studio.status=Some(format!("Editor-only rename: IMG export skipped (no native bytes changed). Save .lckscene. {error:#}"));
                                } else {
                                    self.studio.error=Some(format!("Cannot export native PS1 placement: {error:#}"));
                                }
                            },
                        }
                    }
                    if ui.button("Open LCK scene...").on_hover_text("Rebuild the preview from an editor-only .lckscene file. Does NOT write the game disc.").clicked() {
                        if let Some((name, _)) = &self.native_scene {
                            if let Some(path) = rfd::FileDialog::new()
                                .add_filter("LCK scene overlay", &["lckscene"]).pick_file() {
                                let parsed = std::fs::read_to_string(&path).map_err(|e| e.to_string())
                                    .and_then(|text| scene_instances::parse(&text, name, world));
                                match parsed {
                                    Ok(doc) => pending_project = Some(doc),
                                    Err(err) => self.studio.error = Some(format!("LCK project not loaded: {err}")),
                                }
                            }
                        }
                    }
                    if ui.add_enabled(self.native_selected.and_then(|key| world.at(key.0,key.1)).is_some(),
                        egui::Button::new("Duplicate selected (Ctrl+D)"))
                        .on_hover_text("Ctrl+D: create an editable copy of the selected PS1 mesh, shifted 128 units along X. Save LCK scene to persist it.")
                        .clicked() {
                        pending_duplicate=editor_duplicate_request(world, self.native_selected);
                    }
                    if ui.add_enabled(self.native_selected.is_some(), egui::Button::new("Delete selected (Del)"))
                        .on_hover_text("Remove a new editor copy, or hide a native object until restored. .lckscene saves this without altering the PS1 disc.")
                        .clicked() { pending_delete=self.native_selected; }
                    if ui.add_enabled(self.native_selected.is_some(), egui::Button::new("Frame selected (F)"))
                        .on_hover_text("Automatically center and frame this object from the reference elevated front view, relative to its original PS1 placement rotation and editor transform. No calibration step required.")
                        .clicked() {
                        frame_world_selection(&mut self.native_view, world, scene, self.native_selected);
                    }
                });
                ui.small("Click: single | Shift+click: add | Ctrl+click: toggle | Drag selected: move group | Ctrl+drag selected: move Y | RMB drag: rotate group at individual pivots | Ctrl+D: duplicate group | Del: delete group | Ctrl+Z/Y: undo/redo | LMB elsewhere: pan | MMB/Alt+LMB: orbit | Esc: deselect | F: frame | Home: frame all");
                // World-space camera eye: live scene coordinates, NOT selection/pivot.
                // Keep the current v0.4.5.4 camera framing and movement unchanged.
                if let Some(model) = world.model.as_ref() {
                    let [x, y, z] = viewport::editor_camera_eye(
                        self.native_view.cam, model.center, model.radius);
                    ui.allocate_ui_with_layout(egui::vec2(ui.available_width(), 20.0), egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(RichText::new(format!(
                            "Camera X: {x:.1}   Y: {y:.1}   Z: {z:.1}"
                        )).monospace().small())
                        .on_hover_text("Native scene world-space camera location (Y up). Mirror view is visual only.");
                    });
                }
                ui.separator();
                if let Some(model) = &world.model {
                    if ui.rect_contains_pointer(ui.max_rect()) && ui.input(|i| i.key_pressed(egui::Key::F)) {
                        if let Some((terrain, index)) = self.native_selected {
                            if world.at(terrain, index).is_some() {
                                frame_world_selection(&mut self.native_view, world, scene, self.native_selected);
                            }
                        }
                    }
                    ui.small(format!("{} vertices, {} triangles ({} native walk-ground) | {} original TIM uploads",
                        model.positions.len(), model.triangles(), world.ground_triangles, scene.res.tim_count));
                    ui.small(format!("Native ground atlas: {} / {} textured triangles have VRAM coverage ({} ground triangles total).",
                        world.ground_vram_covered, world.ground_textured, world.ground_triangles));
                    ui.small("Native ground uses original per-cell UVs, CLUT palettes and VRAM texture pages. Mirror view is editor-only.");
                    // The orbit pivot follows the actual EDITED position of the
                    // selected native object. No selection means camera free-look:
                    // the camera eye stays fixed when MMB/Alt+LMB rotates.
                    self.native_view.world_orbit_anchor = self.native_selected.and_then(|(terrain, index)|
                        world.at(terrain, index).map(|d| d.center));
                    self.native_view.world_selected=multi_selection::keys(self.native_selected,&self.native_selection).iter()
                        .filter_map(|key|world.at(key.0,key.1).filter(|d|!d.hidden).map(|d|d.id)).collect();
                    let selected_id = self.native_selected.and_then(|(terrain, index)|
                        world.at(terrain, index).map(|d| d.id));
                    // Gesture is determined at mouse-down from the actual rasterized
                    // mesh hit: dragging the selected mesh edits it, elsewhere pans.
                    // The viewport suppresses camera navigation for captured edits.
                    let view_rect = ui.available_rect_before_wrap();
                    let picked = self.native_view.show_world_limited_with_sources(ui, model, &scene.res.vram,
                        900, 680, selected_id, self.native_ground_visible,
                        self.editor_asset_drag.is_some(), &world.foreign_vram);
                    if let Some(draw)=picked.and_then(|id|world.by_id(id)) {
                        let key=(draw.terrain,draw.index);
                        let modifiers=ui.input(|i|i.modifiers);
                        let was_selected=multi_selection::keys(self.native_selected,&self.native_selection).contains(&key);
                        self.native_selection_click=ui.input(|i|i.pointer.interact_pos()).filter(|_|!modifiers.shift)
                            .map(|pos|(key,pos,modifiers.ctrl && was_selected));
                        // Ctrl on a new member adds it immediately. On an existing
                        // member, defer removal until release to preserve Y dragging.
                        multi_selection::viewport_press(&mut self.native_selected,&mut self.native_selection,key,modifiers.shift,modifiers.ctrl);
                        if modifiers.ctrl && !was_selected {self.native_selection_click=None;}
                        self.native_hierarchy_anchor=Some(key);
                        if self.editor_properties_target.is_some() {self.editor_properties_target=self.native_selected;}
                        ui.ctx().request_repaint();
                    }
                    if self.native_floor_debug_visible {
                        if let Some(lib)=self.studio.library.as_ref() {
                            let key=format!("{}:{}:{:p}",lib.source.path().display(),scene.name,lib.index.as_ref());
                            if self.native_floor_debug_key!=key {
                                self.native_floor_debug=Some(floor_debug::DebugFloor::load(lib.index.as_ref(),&scene.name,key.clone()).map_err(|e|format!("{e:#}")));
                                self.native_floor_debug_key=key;
                            }
                            match &self.native_floor_debug {
                                Some(Ok(debug))=>{debug.paint(ui,view_rect,&self.native_view,model);debug.window(ui.ctx());},
                                Some(Err(error))=>{egui::Window::new("Native floor comparison unavailable").show(ui.ctx(),|ui|{ui.label(error);});},
                                None=>{}
                            }
                        }
                    }
                    if self.native_collision_wire_visible {
                        let source_key = self.studio.library.as_ref()
                            .map(|lib| match &lib.source {
                                Source::Disc(path) | Source::Extracted(path) => path.display().to_string(),
                            }).unwrap_or_default();
                        if self.native_collision_grid.as_ref().is_none_or(|grid|
                            grid.scene != scene.name || grid.source_key != source_key) {
                            let loaded = self.studio.library.as_ref()
                                .ok_or_else(|| anyhow::anyhow!("Open an original PS1 disc image or extracted field data"))
                                .and_then(|lib| collision_wire::CollisionGrid::load(lib.index.as_ref(), &scene.name, &source_key));
                            match loaded {
                                Ok(grid) => self.native_collision_grid = Some(grid),
                                Err(err) => {
                                    self.native_collision_grid = None;
                                    self.native_collision_wire_visible = false;
                                    self.studio.error = Some(format!("Collision wireframe unavailable: {err:#}"));
                                }
                            }
                        }
                        if let Some(grid) = &self.native_collision_grid {
                            let owned_preview=native_wall_export::preview(world);
                            if let Some(warning)=&world.collision.ownership_warning{ui.colored_label(egui::Color32::YELLOW,warning);}
                            if let Ok(Some(map))=&owned_preview {
                                if let Ok(live)=grid.with_owned_map(map){collision_wire::paint(ui,view_rect,&self.native_view,model,&live,Ok(ps1_export::ExportEdits::default()));}
                            } else {
                            if let Err(error)=owned_preview{ui.colored_label(egui::Color32::YELLOW,format!("Owned collision preview incomplete: {error:#}"));}
                            // The overlay is drawn AFTER the raster image using the
                            // identical camera projection. It never changes
                            // camera gestures, hit testing, or IMG export.
                            // The Q5 preview originally assembled donor scenes and decoded
                            // their meshes/MAP on EVERY repaint. That triggers severe stutter
                            // in the CPU raster editor, especially while moving an object.
                            // A complete serialized LCK scene overlay is a deterministic
                            // change key; moving or deleting objects invalidates this cache.
                            // The disc and cell are included to prevent cross-disc reuse.
                            let overlay = scene_instances::serialize(&scene.name, world);
                            let key = format!("{}\n{}\n{}", source_key, scene.name, overlay);
                            let dragging = self.native_drag.is_some() || self.native_rotate.is_some()
                                || self.editor_asset_drag.is_some();
                            let changed = self.native_collision_preview_cache.as_ref()
                                .is_none_or(|(cached,_)| cached != &key);
                            // Do not reload/decompress whole donor scenes on every mouse
                            // move. While dragging show base native collision only, then
                            // compute once when the gesture has finished.
                            if changed && !dragging {
                                let outcome = self.studio.library.as_ref()
                                    .map(|lib| ps1_export::prepare_cross_cell_collision_preview(
                                        scene, world, lib.index.as_ref()))
                                    .unwrap_or_else(|| Err(anyhow::anyhow!("Missing open disc index")))
                                    .map_err(|err| format!("{err:#}"));
                                self.native_collision_preview_cache=Some((key.clone(),outcome));
                            }
                            let edits = match self.native_collision_preview_cache.as_ref() {
                                Some((cached,result)) if cached==&key =>
                                    result.clone().map_err(anyhow::Error::msg),
                                _ => ps1_export::prepare_collision_preview(scene,world),
                            };
                            collision_wire::paint(ui, view_rect, &self.native_view, model,
                                grid, edits);
                            }
                        }
                    }
                    if self.object_collision_draft_visible {
                        let preview=match &self.generated_collision_target {
                            Some((name,target)) if name==&scene.name=>editor_history_selected(world,target),
                            Some(_)=>None,None=>self.native_selected,
                        };
                        if let Some((terrain,index))=preview {
                            if let Some(draw)=world.at(terrain,index) {
                                // Use live transformed mesh vertices: native originals,
                                // duplicates, and imported game-wide draws all follow
                                // the same non-destructive editor transforms.
                                if let Some(generated)=collision_recipes::generated_for(world,(terrain,index)) {
                                    object_collision::paint_generated(ui,view_rect,&self.native_view,model,draw,generated);
                                } else if self.generated_collision_target.is_none() {
                                    object_collision::paint(ui,view_rect,&self.native_view,model,draw);
                                }
                            }
                        }
                    }
                    let (pointer, down, pressed, ctrl, alt) = ui.input(|inp| (
                        inp.pointer.interact_pos(), inp.pointer.primary_down(),
                        inp.pointer.primary_pressed(), inp.modifiers.ctrl, inp.modifiers.alt));
                    if self.native_drag.is_some() && (!down || alt) {
                        finish_drag = self.native_drag.take();
                    }
                    if ui.input(|i|i.pointer.primary_released()) {
                        if let Some((key,start,toggle))=self.native_selection_click.take() {
                            if pointer.is_some_and(|pos|(pos-start).length_sq()<=36.0) {
                                if toggle {multi_selection::toggle(&mut self.native_selected,&mut self.native_selection,key);}
                                else {multi_selection::single(&mut self.native_selected,&mut self.native_selection,key);}
                                if self.editor_properties_target.is_some() {self.editor_properties_target=self.native_selected;}
                                ctx.request_repaint();
                            }
                        }
                    }
                    // Start an object rotation ONLY when RMB is pressed directly on
                    // the highlighted mesh (not on empty space or another object).
                    // MMB/Alt+LMB camera orbit and hierarchy RMB remain untouched.
                    let (right_down,right_pressed)=ui.input(|inp|
                        (inp.pointer.secondary_down(),inp.pointer.secondary_pressed()));
                    if self.native_rotate.is_some() && !right_down {
                        if let Some(gesture)=self.native_rotate.take() {
                            let released_at=pointer.unwrap_or(gesture.pointer_start);
                            if (released_at-gesture.pointer_start).length_sq() <= 36.0 &&
                                !gesture.pitch_axis {
                                self.editor_context_menu=Some((released_at,gesture.key));
                            } else {
                                finish_rotate=Some(gesture);
                            }
                        }
                    }
                    if right_pressed && self.editor_modal.is_none() && self.editor_rename.is_none() {
                        if let (Some(key),Some(pos))=(self.native_selected,pointer) {
                            if view_rect.contains(pos) {
                                if let Some(draw)=world.at(key.0,key.1) {
                                    if !draw.hidden && draw.vertex_start < draw.vertex_end {
                                        // Match the raster viewport's pixel scale and
                                        // optional horizontal display-only mirroring.
                                        let scale=ui.ctx().pixels_per_point();
                                        let px=[((view_rect.width()*scale) as usize).clamp(16,900),
                                                ((view_rect.height()*scale) as usize).clamp(16,680)];
                                        let x=(pos.x-view_rect.left())*px[0] as f32/view_rect.width().max(1.0);
                                        let y=(pos.y-view_rect.top())*px[1] as f32/view_rect.height().max(1.0);
                                        let hit_x=if self.native_view.world_mirrored {
                                            (px[0] as f32 - 1.0 - x).max(0.0)
                                        } else { x };
                                        let id=crate::raster::pick_instance(model,self.native_view.cam,
                                            px[0],px[1],[hit_x,y]);
                                        if id==Some(draw.id) {
                                            self.native_rotate=Some(EditorRotateDrag {
                                                key,pointer_start:pos,rotation_start:draw.rotation,
                                                pitch_axis:alt,
                                            });
                                        }
                                    }
                                }
                            }
                        }
                    }
                    if right_down {
                        if let (Some(drag),Some(pos))=(self.native_rotate,pointer) {
                            if self.native_selected==Some(drag.key) &&
                                (pos-drag.pointer_start).length_sq()>36.0 {
                                let rotation=editor_rotate_drag(drag.rotation_start,
                                    pos-drag.pointer_start,drag.pitch_axis,self.native_view.world_mirrored);
                                let rotation=geck_snap_euler_axis(rotation,
                                    if drag.pitch_axis {0} else {1}, self.geck_angle_snap);
                                pending_rotation=Some((drag.key,rotation,false));
                            }
                        }
                    }
                    if down && !alt && self.editor_asset_drag.is_none() && self.global_repository_drag.is_none() {
                        if let (Some(key), Some(pos)) = (self.native_selected, pointer) {
                            // A previously selected *visible* mesh must be hit at mouse-down.
                            // Never start moving an object when a drag begins over ground,
                            // empty space or another object (those gestures pan normally).
                            if pressed && !ui.input(|i|i.modifiers.shift) && view_rect.contains(pos) {
                                if let Some(draw) = world.at(key.0, key.1) {
                                    if !draw.hidden && draw.vertex_start < draw.vertex_end &&
                                        self.native_view.world_left_drag_selected == Some(draw.id) {
                                        self.native_drag_group=multi_selection::keys(self.native_selected,&self.native_selection).into_iter()
                                            .filter_map(|k|world.at(k.0,k.1).filter(|d|!d.hidden).map(|d|(k,d.offset))).collect();
                                        self.native_drag = Some(EditorDrag {
                                            key, pointer_start: pos, offset_start: draw.offset,
                                        });
                                    }
                                }
                            }
                            if let Some(drag) = self.native_drag.filter(|d| d.key == key) {
                                let mut desired = editor_drag_offset(drag.offset_start, pos - drag.pointer_start,
                                    self.native_view.cam, model.radius, view_rect.height(),
                                    self.native_view.world_mirrored, ctrl);
                                if self.native_move_mode == EditorMoveMode::Grid {
                                    desired = world_view::quantize_offset(desired, self.native_grid_size);
                                }
                                if self.native_anchor_snap {
                                    desired = world.snap_nearby_anchor(key, desired, self.native_grid_size * 0.5);
                                }
                                // Lock after magnet so snapping cannot move the locked axis.
                                // Ctrl+drag remains the established vertical/Y movement.
                                if !ctrl {
                                    match self.geck_drag_axis {
                                        Some(0) => desired[2] = drag.offset_start[2],
                                        Some(2) => desired[0] = drag.offset_start[0],
                                        _ => {}
                                    }
                                }
                                pending_transform = Some((key, desired, false));
                            }
                        }
                    }
                    // Repository drag starts in the LEFT panel; release is handled in
                    // the scene panel only. The viewport cannot mistake this for
                    // camera pan or movement of an already-selected instance.
                    if let Some(global_id)=self.global_repository_drag {
                        let released=ui.input(|i|i.pointer.primary_released());
                        let hovering=ui.input(|i|i.pointer.latest_pos())
                            .filter(|p|view_rect.contains(*p));
                        if let Some(asset)=self.global_repository.get(global_id) {
                            // Architecture drops keep their source identity even
                            // within the same cell. Ctrl+D of existing native
                            // objects retains its established native-copy path.
                            let compatible=matches!(asset.kind,global_repository::Kind::Npc(_)).then(||global_repository::local_source(scene,asset)).flatten()
                                .and_then(|local| placement_drop::native_source_key(scene,world,local).map(|_|local));
                            if let Some(pointer)=hovering {
                                ui.painter().text(pointer+egui::vec2(12.0,15.0),
                                    egui::Align2::LEFT_TOP,
                                    if compatible.is_some() {"Release: create destination-native instance"}
                                    else if !matches!(asset.kind,global_repository::Kind::Npc(_)) {
                                        "Release: create static architecture copy (donor scripts detached)"
                                    } else {"Foreign MAN actors need script/animation importing"},
                                    egui::FontId::proportional(14.0),Color32::YELLOW);
                            }
                            if released {
                                if let Some(pointer)=hovering {
                                    if let Some(local)=compatible {
                                        let px=placement_drop::viewport_pixels(view_rect,
                                            ui.ctx().pixels_per_point(),[900,680]);
                                        let pixel=placement_drop::viewport_pixel(view_rect,pointer,
                                            px,self.native_view.world_mirrored);
                                        let snap=ui.input(|i|i.modifiers.shift);
                                        match placement_drop::prepare_drop(scene,world,local,
                                            self.native_view.cam,pixel,px,snap) {
                                            Ok(request)=>{
                                                pending_duplicate=Some(request);
                                                pending_repository_drop=true;
                                            },
                                            Err(error)=>self.studio.error=Some(format!(
                                                "Global asset placement: {error}")),
                                        }
                                    } else if !matches!(asset.kind,global_repository::Kind::Npc(_)) {
                                        let px=placement_drop::viewport_pixels(view_rect,
                                            ui.ctx().pixels_per_point(),[900,680]);
                                        let pixel=placement_drop::viewport_pixel(view_rect,pointer,
                                            px,self.native_view.world_mirrored);
                                        pending_foreign=Some((global_id,pixel,px,ui.input(|i|i.modifiers.shift)));
                                    } else {
                                        self.studio.error=Some("Cross-cell MAN actors need resource, animation and script relocation before staging as functioning NPCs. Original actor preview remains available.".into());
                                    }
                                }
                                self.global_repository_drag=None;
                            }
                        }
                    }
                    if let Some(asset)=self.editor_asset_drag {
                        let released=ui.input(|i|i.pointer.primary_released());
                        let hovering=ui.input(|i|i.pointer.latest_pos())
                            .filter(|p|view_rect.contains(*p));
                        if let Some(pointer)=hovering {
                            let label=if placement_drop::native_source_key(scene,world,asset).is_some() {
                                "Release: place at F distance | Shift: snap to walk-ground"
                            } else {"No drawable native source in this cell"};
                            ui.painter().text(pointer+egui::vec2(12.0,15.0),
                                egui::Align2::LEFT_TOP,label,
                                egui::FontId::proportional(14.0),Color32::YELLOW);
                        }
                        if released {
                            self.editor_asset_drag=None;
                            if let Some(pointer)=hovering {
                                let px=placement_drop::viewport_pixels(view_rect,ui.ctx().pixels_per_point(),[900,680]);
                                let pixel=placement_drop::viewport_pixel(view_rect,pointer,px,self.native_view.world_mirrored);
                                let ground_snap=ui.input(|i| i.modifiers.shift);
                                match placement_drop::prepare_drop(scene,world,asset,
                                    self.native_view.cam,pixel,px,ground_snap) {
                                    Ok(request)=>{
                                        pending_duplicate=Some(request);
                                        pending_repository_drop=true;
                                    },
                                    Err(message)=>self.studio.error=Some(format!("Cannot place asset: {message}")),
                                }
                            }
                        }
                    }
                } else {
                    ui.label("The native scene assembled, but no drawable TMD triangles were resolved.");
                }
            } else {
                ui.small("Select a cell on the left to load its original PS1 geometry.");
                ui.separator();
                self.preview(ui);
            }
        });
        if let Some(global_id)=self.global_repository_drag {
            if ctx.input(|i|i.key_pressed(egui::Key::Escape)) {
                self.global_repository_drag=None;
                self.studio.status=Some("Cross-cell placement cancelled".into());
            } else if let (Some(pointer),Some(asset))=(ctx.input(|i|i.pointer.latest_pos()),
                self.global_repository.get(global_id)) {
                let painter=ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip,
                    egui::Id::new("lck_global_asset_drag_cursor")));
                let text=format!("{} / {}\nDrag into target viewport · Shift: ground snap",asset.source,asset.label);
                painter.rect_filled(egui::Rect::from_min_size(pointer+egui::vec2(12.0,18.0),
                    egui::vec2(390.0,44.0)),egui::CornerRadius::same(5),
                    Color32::from_rgba_unmultiplied(26,38,50,225));
                painter.text(pointer+egui::vec2(20.0,23.0),egui::Align2::LEFT_TOP,text,
                    egui::FontId::proportional(13.0),Color32::YELLOW);
                ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
                ctx.request_repaint();
            }
        }
        if let Some(asset)=self.editor_asset_drag {
            if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                self.editor_asset_drag=None;
                self.studio.status=Some("Repository placement cancelled".into());
            } else if let Some(pointer)=ctx.input(|i| i.pointer.latest_pos()) {
                let kind=match asset {
                    EditorCatalogItem::Draw(true,_)=>"Terrain",
                    EditorCatalogItem::Draw(false,_)=>"Native object / NPC",
                    EditorCatalogItem::Mesh(_)=>"Native mesh",
                    _=>"Resource",
                };
                let painter=ctx.layer_painter(egui::LayerId::new(
                    egui::Order::Tooltip, egui::Id::new("lck_asset_drag_cursor")));
                let name=self.editor_name_for(ready.as_deref(),asset);
                painter.rect_filled(egui::Rect::from_min_size(pointer+egui::vec2(12.0,18.0),
                    egui::vec2(350.0,42.0)),egui::CornerRadius::same(5),
                    Color32::from_rgba_unmultiplied(26,38,50,225));
                painter.text(pointer+egui::vec2(20.0,23.0),
                    egui::Align2::LEFT_TOP,
                    format!("{kind}: {name}\nDrop: F distance · Shift: ground"),
                    egui::FontId::proportional(13.0),Color32::YELLOW);
                ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
                // Keep hover/drop guidance alive while crossing panel borders.
                ctx.request_repaint();
            }
        }
        if ctx.input(|i|i.pointer.primary_released()) {
            self.editor_asset_drag=None;
            self.global_repository_drag=None;
        }
        // RMB click (without moving) opens a context dropdown on the selected
        // mesh. RMB drag continues to rotate Y; Alt+RMB drag tilts X.
        // Keeping the menu outside the viewport allocation prevents resizing
        // and does not affect existing pan/orbit or rendering code.
        if let Some((position,key))=self.editor_context_menu {
            let panel=egui::Area::new(egui::Id::new("lck_viewport_properties_context"))
                .order(egui::Order::Foreground)
                .fixed_pos(position)
                .interactable(true)
                .show(ctx, |ui| {
                    egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.set_min_width(175.0);
                        if ui.button("Properties...").clicked() {
                            self.native_selected=Some(key);
                            self.editor_properties_target=Some(key);
                            self.editor_context_menu=None;
                        }
                        if ui.button("Rename...").clicked() {
                            self.editor_rename=Some(EditorRename{key,text:self.editor_name_for(ready.as_deref(),EditorCatalogItem::Draw(key.0,key.1))});
                            self.editor_context_menu=None;
                        }
                    });
                });
            if self.editor_context_menu.is_some() && ctx.input(|i|i.pointer.any_pressed()) &&
                !panel.response.rect.contains(ctx.input(|i|i.pointer.interact_pos())
                    .unwrap_or(egui::Pos2::ZERO)) {
                self.editor_context_menu=None;
            }
        }
        self.editor_catalog_popup(ctx, ready.as_deref(), world.as_deref());
        self.editor_global_repository_dialog(ctx);
        // Ctrl+D and the Duplicate button share one path. Keep this shortcut
        // local to the World Editor and disable it while typing, renaming or
        // using an object properties/preview dialog.
        if self.editor_modal.is_none() && self.global_repository_dialog.is_none() &&
            self.editor_rename.is_none() &&
            self.editor_cell_rename.is_none() && self.editor_cell_properties.is_none() &&
            !ctx.wants_keyboard_input() &&
            ctx.input_mut(|i| i.consume_shortcut(&egui::KeyboardShortcut::new(
                egui::Modifiers::COMMAND, egui::Key::D))) {
            if let Some(key)=self.native_selected {
                pending_duplicate=world.as_deref().and_then(|w|
                    editor_duplicate_request(w, Some(key)));
            } else if let Some(cell)=self.editor_cell_selected.clone() {
                self.duplicate_editor_cell(cell);
            }
        }
        // Guard Delete so text entry (hierarchy filter, rename, coordinates)
        // and preview dialogs never delete scene objects by accident.
        if self.editor_modal.is_none() && self.global_repository_dialog.is_none() &&
            self.editor_rename.is_none() &&
            self.editor_cell_rename.is_none() && self.editor_cell_properties.is_none() &&
            !ctx.wants_keyboard_input() && ctx.input(|i|i.key_pressed(egui::Key::Delete)) {
            pending_delete=self.native_selected;
        }
        // Capture lightweight scene metadata before edits, never clone raster
        // vertices per frame. One mouse gesture becomes one undo operation.
        let editor_changed_this_frame = pending_name.is_some() || pending_restore.is_some() ||
            pending_delete.is_some() || pending_duplicate.is_some() || pending_foreign.is_some() ||
            pending_transform.is_some() || pending_rotation.is_some() ||
            pending_scale.is_some() || pending_import.is_some() ||
            pending_npc_animation.as_ref().is_some_and(|item|item.3) ||
            pending_npc_model.is_some() ||
            pending_project.is_some() || pending_collision_edit.is_some() || pending_static_architecture.is_some() || self.editor_gesture_start.is_some();
        let before_frame=if editor_changed_this_frame {
            match (self.native_scene.as_ref(),world.as_deref()) {
                (Some((name,NativeSceneSlot::Ready(_, _))),Some(w))=>
                    Some(editor_history_capture_group(w,name,self.native_selected,&self.native_selection)),
                _=>None,
            }
        } else {None};
        // Keyboard shortcuts are disabled while entering text or using dialogs.
        if self.editor_modal.is_none() && self.editor_rename.is_none() &&
            !ctx.wants_keyboard_input() {
            pending_history_undo |= ctx.input_mut(|i| i.consume_shortcut(
                &egui::KeyboardShortcut::new(egui::Modifiers::COMMAND,egui::Key::Z)));
            pending_history_redo |= ctx.input_mut(|i| i.consume_shortcut(
                &egui::KeyboardShortcut::new(egui::Modifiers::COMMAND,egui::Key::Y)));
        }
        // All panels have released their Arc clones before the mutation.
        // Arc::get_mut keeps one writable world, without cloning thousands
        // of vertices on every mouse frame. The original scene remains intact.
        drop(world);
        if let Some((uid,enabled))=pending_static_architecture {
            if let Some((_,NativeSceneSlot::Ready(_,world)))=self.native_scene.as_mut(){if let Some(world)=Arc::get_mut(world){
                if let Some(foreign)=world.foreign.iter_mut().find(|f|f.uid==uid){foreign.static_architecture=enabled;self.studio.status=Some("Copy behavior mode updated. Save LCK scene to persist; native visibility and collision still require manual verification.".into());ctx.request_repaint();}
            }}
        }
        if let Some((key,recipe))=pending_collision_edit {
            if let Some((_,NativeSceneSlot::Ready(_,world)))=self.native_scene.as_mut() {
                if let Some(world)=Arc::get_mut(world) {
                    let result=match recipe {Some(recipe)=>world.set_collision_recipe(key,recipe),None=>world.remove_collision_recipe(key)};
                    match result {
                        Ok(())=>{self.studio.status=Some("Collision recipe updated (editor/in-memory only). Save LCK scene to preserve it.".into());ctx.request_repaint();},
                        Err(error)=>self.studio.error=Some(error),
                    }
                }
            }
        }
        if let Some((key,name))=pending_name {
            if let Some((_,NativeSceneSlot::Ready(_,world)))=self.native_scene.as_mut() {
                if let Some(world)=Arc::get_mut(world) {
                    if name.is_some() && world.at(key.0,key.1).is_some_and(|d|d.editor_uid.is_none()) && !world.naming.native.contains_key(&key) {
                        self.studio.error=Some("Cannot rename this original instance without a verified stable MAP/MAN identity. Existing names are retained.".into());
                    } else if world.set_display_name(key,name) {
                        self.studio.status=Some("Scene instance renamed. Save LCK scene to preserve the name.".into());
                        ctx.request_repaint();
                    }
                }
            }
        }
        if let Some(key)=pending_restore {
            if let Some((_,NativeSceneSlot::Ready(_,world)))=self.native_scene.as_mut() {
                if let Some(world)=Arc::get_mut(world) {
                    if world.set_hidden(key,false) {
                        self.native_view.version=self.native_view.version.wrapping_add(1);
                        self.studio.status=Some("Native scene object restored (editor only).".into());
                        ctx.request_repaint();
                    }
                }
            }
        }
        if let Some(key)=pending_delete {
            if let Some((_,NativeSceneSlot::Ready(_,world)))=self.native_scene.as_mut() {
                if let Some(world)=Arc::get_mut(world) {
                    let keys=multi_selection::keys(Some(key),&self.native_selection);
                    let result=multi_selection::delete(world,&keys);
                    match result {
                        Ok(())=>{
                            self.native_selected=None; self.native_selection.clear();
                            self.native_drag=None;
                            self.native_rotate=None;
                            self.native_undo.clear(); self.native_redo.clear();
                            self.native_rotate_undo.clear(); self.native_rotate_redo.clear();
                            self.editor_preview=None; self.editor_modal=None;
                            self.editor_properties_target=None; self.editor_context_menu=None;
                            self.npc_playing=None;
                            self.native_view.version=self.native_view.version.wrapping_add(1);
                            self.studio.status=Some("Deleted from editor scene. Native PS1 files unchanged. Save LCK scene to persist.".into());
                            ctx.request_repaint();
                        },
                        Err(error)=>self.studio.error=Some(format!("Cannot delete selected object: {error}")),
                    }
                }
            }
        }
        if let Some((global_id,pixel,px,snap))=pending_foreign {
            let asset=self.global_repository.get(global_id).cloned();
            let index=self.studio.library.as_ref().map(|lib|Arc::clone(&lib.index));
            if let (Some(asset),Some(index))=(asset,index) {
                let donor=legaia_engine_world_views::scene_staging::assemble_field_scene(&index,&asset.source);
                match donor {
                    Ok(donor)=>{
                        let (kind,record)=match asset.kind {
                            global_repository::Kind::Placement(i)=>("placement",i),
                            global_repository::Kind::Terrain(i)=>("terrain",i),
                            global_repository::Kind::Mesh(i)=>("mesh",i),
                            global_repository::Kind::Npc(_)=>("unsupported",0),
                        };
                        let result=world_view::foreign_size(&donor,kind,record).and_then(|(radius,bottom)|{
                            if let Some((_,NativeSceneSlot::Ready(_,world)))=self.native_scene.as_mut(){
                                let world=Arc::get_mut(world).ok_or("Destination world is borrowed")?;
                                let center=placement_drop::foreign_drop_center(world,
                                    self.native_view.cam,pixel,px,radius,bottom,snap)?;
                                let uid=world.draws.iter().filter_map(|d|d.editor_uid)
                                    .max().unwrap_or(0).saturating_add(1);
                                world.insert_foreign(&donor,kind,record,uid,center,&index)
                            } else {Err("Destination scene is not ready".into())}
                        });
                        match result {
                            Ok(key)=>{
                                self.native_selected=Some(key);
                                if let Some((_,NativeSceneSlot::Ready(scene,world)))=self.native_scene.as_ref() {
                                    frame_world_selection(&mut self.native_view,world,scene,Some(key));
                                }
                                self.native_view.version=self.native_view.version.wrapping_add(1);
                                self.studio.status=Some(format!("Foreign original {} staged as independent editor mesh. Save .lckscene to persist. Geometry and source VRAM materials are displayed; native collision and IMG export are not imported.",asset.label));
                                ctx.request_repaint();
                            }
                            Err(error)=>self.studio.error=Some(format!("Cross-cell placement: {error}")),
                        }
                    }
                    Err(error)=>self.studio.error=Some(format!("Could not open original donor {}: {error}",asset.source)),
                }
            }
        }
        if pending_duplicate.is_some() && !pending_repository_drop {
            let old_primary=self.native_selected;
            let keys=multi_selection::keys(self.native_selected,&self.native_selection);
            if let Some((_,NativeSceneSlot::Ready(scene,world)))=self.native_scene.as_mut() {
                if let Some(world)=Arc::get_mut(world) {
                    match multi_selection::duplicate(world,&keys,scene.placements.len()) {
                        Ok(created)=>{
                            self.native_selected=keys.iter().zip(&created).find_map(|(old,new)|(Some(*old)==old_primary).then_some(*new));
                            self.native_selection=created.into_iter().collect();
                            if self.editor_properties_target.is_some() {self.editor_properties_target=self.native_selected;}
                            self.native_view.version=self.native_view.version.wrapping_add(1);
                            self.studio.status=Some(format!("{} independent editor copies created and selected; save LCK scene to persist.",self.native_selection.len()));
                            ctx.request_repaint();
                        }
                        Err(error)=>self.studio.error=Some(format!("Could not duplicate selection: {error}")),
                    }
                }
            }
            pending_duplicate=None;
        }
        if let Some((source,offset,rotation,actor)) = pending_duplicate {
            if let Some((_,NativeSceneSlot::Ready(scene,world)))=self.native_scene.as_mut() {
                if let Some(world)=Arc::get_mut(world) {
                    let uid=world.draws.iter().filter_map(|d|d.editor_uid).max().unwrap_or(0).saturating_add(1);
                    // Repository drops are fresh instances: never inherit a
                    // different selected object's scale by accident.
                    let selected_scale=if pending_repository_drop { [1.0;3] } else {
                        self.native_selected.and_then(|old|
                            world.at(old.0,old.1).map(|d|d.scale)).unwrap_or([1.0;3])
                    };
                    let selected_name=if pending_repository_drop {None}else{self.native_selected.and_then(|old|world.at(old.0,old.1)).and_then(|d|d.display_name.clone())};
                    let duplicated=world.duplicate_native(source,scene.placements.len(),uid,offset).and_then(|key| {
                        if !pending_repository_drop {
                            if let Some(old)=self.native_selected {
                                if world.collision.source(world,old).is_some() &&
                                    world.collision.source(world,old)==world.collision.source(world,source) {
                                    if let Err(error)=world.inherit_collision_recipe(old,key) {
                                        world.delete_created(key)?;
                                        return Err(error);
                                    }
                                }
                            }
                        }
                        Ok(key)
                    });
                    match duplicated {
                        Ok(key) => {
                            world.set_display_name(key,selected_name);
                            // Copy the *selected instance* appearance and frame, not just the
                            // source's original MAN record. Each copy stays independent.
                            if let Some((model,clip,frame))=actor {
                                if let Err(error)=world.set_npc_model(key,model)
                                    .and_then(|_|world.set_npc_animation(key,clip,frame)) {
                                    self.studio.error=Some(format!("NPC copy appearance: {error}"));
                                }
                            }
                            if selected_scale != [1.0;3] && !world.set_scale(key.0,key.1,selected_scale) {
                                self.studio.error=Some("Copy created, but its scale could not be inherited".into());
                            }
                            if rotation != [0.0;3] && !world.set_rotation(key.0,key.1,rotation) {
                                self.studio.error=Some("Copy created, but its rotation could not be inherited".into());
                            }
                            self.native_selected=Some(key);
                            if pending_repository_drop {
                                // The selected item appears at the exact F
                                // reference viewing distance and original
                                // native orientation, with no extra key press.
                                frame_world_selection(&mut self.native_view,world,scene,Some(key));
                            }
                            self.native_view.version=self.native_view.version.wrapping_add(1);
                            self.studio.status=Some(format!("New editor instance #{uid} created and selected{}; save LCK scene to persist.",
                                if pending_repository_drop { " (auto-framed as F)" } else { "" }));
                            ctx.request_repaint();
                        },
                        Err(error)=>self.studio.error=Some(format!("Could not duplicate object: {error}")),
                    }
                }
            }
        }
        if let Some((key, desired, record)) = pending_transform {
            if let Some((_, NativeSceneSlot::Ready(_, world))) = self.native_scene.as_mut() {
                if let Some(world) = Arc::get_mut(world) {
                    if let Some(draw) = world.at(key.0, key.1) {
                        let before = draw.offset;
                        let group:Vec<_>=if !record && self.native_drag.is_some_and(|d|d.key==key) {
                            let delta=[0,1,2].map(|a|desired[a]-self.native_drag.unwrap().offset_start[a]);
                            self.native_drag_group.iter().map(|(k,start)|(*k,[0,1,2].map(|a|start[a]+delta[a]))).collect()
                        }else{
                            let delta=[0,1,2].map(|a|desired[a]-before[a]);
                            multi_selection::keys(Some(key),&self.native_selection).into_iter().map(|k|{
                                let start=world.at(k.0,k.1).map(|d|d.offset).unwrap_or([f32::NAN;3]);
                                (k,[0,1,2].map(|a|start[a]+delta[a]))
                            }).collect()
                        };
                        let moved=if before!=desired {multi_selection::move_offsets(world,&group)}else{Ok(())};
                        if let Err(error)=moved {self.studio.error=Some(error);}
                        else if before != desired {
                            self.native_view.version = self.native_view.version.wrapping_add(1);
                            ctx.request_repaint();
                            if record {
                                self.native_undo.push(EditorTransformEdit { key, before, after: desired });
                                self.native_redo.clear();
                            }
                        }
                    }
                }
            }
        }
        if let Some((key,desired,record))=pending_rotation {
            if let Some((_,NativeSceneSlot::Ready(_,world)))=self.native_scene.as_mut() {
                if let Some(world)=Arc::get_mut(world) {
                    if let Some(draw)=world.at(key.0,key.1) {
                        let before=draw.rotation;
                        let result=if before!=desired {multi_selection::rotate(world,key,&self.native_selection,desired)}else{Ok(())};
                        if let Err(error)=result {self.studio.error=Some(error);}
                        else if before != desired {
                            self.native_view.version=self.native_view.version.wrapping_add(1);
                            ctx.request_repaint();
                            if record {
                                self.native_rotate_undo.push(EditorRotationEdit{key,before,after:desired});
                                self.native_rotate_redo.clear();
                            }
                        }
                    }
                }
            }
        }
        if let Some((key,desired))=pending_scale {
            if let Some((_,NativeSceneSlot::Ready(_,world)))=self.native_scene.as_mut() {
                if let Some(world)=Arc::get_mut(world) {
                    if world.at(key.0,key.1).is_some_and(|d|d.scale!=desired) {
                        match multi_selection::scale(world,key,&self.native_selection,desired) {
                            Ok(())=>{self.native_view.version=self.native_view.version.wrapping_add(1);ctx.request_repaint();}
                            Err(error)=>self.studio.error=Some(error),
                        }
                    }
                }
            }
        }
        if let Some((key,index))=pending_npc_model {
            if let Some((_,NativeSceneSlot::Ready(_,world)))=self.native_scene.as_mut() {
                if let Some(world)=Arc::get_mut(world) {
                    match world.set_npc_model(key,index) {
                        Ok(())=>{
                            self.native_view.version=self.native_view.version.wrapping_add(1);
                            self.studio.status=Some(format!("NPC uses scene TMD {index}; save LCK scene to persist appearance."));
                            ctx.request_repaint();
                        }
                        Err(reason)=>self.studio.error=Some(format!("NPC appearance: {reason}")),
                    }
                }
            }
        }
        if let Some((key,clip,frame,_))=pending_npc_animation {
            if let Some((_,NativeSceneSlot::Ready(_,world)))=self.native_scene.as_mut() {
                if let Some(world)=Arc::get_mut(world) {
                    match world.set_npc_animation(key,clip,frame) {
                        Ok(())=>{
                            self.native_view.version=self.native_view.version.wrapping_add(1);
                            ctx.request_repaint();
                        }
                        Err(reason)=>{
                            self.npc_playing=None;
                            self.studio.error=Some(format!("NPC animation preview: {reason}"));
                        }
                    }
                }
            }
        }
        if let Some(edits) = pending_import {
            if let Some((_, NativeSceneSlot::Ready(_, world))) = self.native_scene.as_mut() {
                if let Some(world) = Arc::get_mut(world) {
                    if let Err(error)=scene_instances::apply_offsets(world,&edits) {
                        self.studio.error=Some(format!("Could not restore layout: {error}"));
                    } else {
                    self.native_undo.clear();
                    self.native_redo.clear();
                    self.native_drag = None;
                    self.native_rotate = None;
                    self.native_rotate_undo.clear(); self.native_rotate_redo.clear();
                    self.native_view.version = self.native_view.version.wrapping_add(1);
                    ctx.request_repaint();
                    self.studio.status = Some("Preview layout offsets loaded; the PS1 disc remains untouched.".into());
                    }
                }
            }
        }
        if let Some(doc) = pending_project {
            let index=self.studio.library.as_ref().map(|lib|Arc::clone(&lib.index));
            if let Some((_,NativeSceneSlot::Ready(scene,world)))=self.native_scene.as_mut() {
                if let Some(world)=Arc::get_mut(world) {
                    let result=index.as_ref().ok_or("No disc index for project restoration".to_string())
                        .and_then(|index|scene_instances::apply(&doc,world,scene.placements.len(),index));
                    match result {
                        Ok(())=>{
                            self.native_selected=None; self.native_selection.clear();
                            self.editor_properties_target=None;
                            self.editor_context_menu=None;
                            self.npc_playing=None;
                            self.native_undo.clear();
                            self.native_redo.clear();
                            self.native_drag=None;
                            self.native_rotate=None;
                            self.native_rotate_undo.clear(); self.native_rotate_redo.clear();
                            self.native_view.version=self.native_view.version.wrapping_add(1);
                            self.studio.status=Some(format!("Loaded LCK scene with {} additional native copies and {} foreign geometry previews; PS1 disc unchanged.",doc.created.len(),doc.foreign.len()));
                            ctx.request_repaint();
                        },
                        Err(err)=>self.studio.error=Some(format!("Could not restore LCK scene: {err}")),
                    }
                }
            }
        }
        if let Some(drag)=finish_rotate {
            if let Some((_,NativeSceneSlot::Ready(_,world)))=self.native_scene.as_ref() {
                if let Some(draw)=world.at(drag.key.0,drag.key.1) {
                    if draw.rotation!=drag.rotation_start {
                        self.native_rotate_undo.push(EditorRotationEdit {
                            key:drag.key,before:drag.rotation_start,after:draw.rotation,
                        });
                        self.native_rotate_redo.clear();
                    }
                }
            }
        }
        if let Some(drag) = finish_drag {
            if let Some((_, NativeSceneSlot::Ready(_, world))) = self.native_scene.as_ref() {
                if let Some(draw) = world.at(drag.key.0, drag.key.1) {
                    if draw.offset != drag.offset_start {
                        self.native_undo.push(EditorTransformEdit {
                            key: drag.key, before: drag.offset_start, after: draw.offset,
                        });
                        self.native_redo.clear();
                    }
                }
            }
        }
        // Coalesce live mouse drags (translation, rotation, numeric scale) to
        // one history entry on mouse release. Snapshot captures only metadata.
        let after_frame=if editor_changed_this_frame || pending_history_undo || pending_history_redo {
            self.native_scene.as_ref().and_then(|(name,state)|match state {
                NativeSceneSlot::Ready(_,w)=>Some(editor_history_capture_group(w,name,self.native_selected,&self.native_selection)),
                _=>None,
            })
        } else {None};
        if let (Some(before),Some(after))=(&before_frame,&after_frame) {
            if before.overlay!=after.overlay && self.editor_gesture_start.is_none() {
                self.editor_gesture_start=Some(before.clone());
            }
        }
        let any_button_down=ctx.input(|i|i.pointer.primary_down() || i.pointer.secondary_down());
        if !any_button_down {
            if let Some(start)=self.editor_gesture_start.take() {
                if after_frame.as_ref().is_some_and(|after|after.overlay!=start.overlay) {
                    self.editor_undo.push(start);
                    if self.editor_undo.len()>100 {self.editor_undo.remove(0);}
                    self.editor_redo.clear();
                }
            }
        }
        if pending_history_undo || pending_history_redo {
            // Undo/redo intentionally does not create a new undo record.
            // A still-held mouse gesture must not be rolled back mid-gesture.
            if !any_button_down {
                let history=if pending_history_undo {
                    &mut self.editor_undo
                } else { &mut self.editor_redo };
                if let Some(target)=history.last().cloned() {
                    let current=after_frame;
                    let index=self.studio.library.as_ref().map(|lib|Arc::clone(&lib.index));
                    let restore=self.native_scene.as_mut().ok_or("No scene selected".to_owned())
                        .and_then(|(name,slot)| {
                            let NativeSceneSlot::Ready(scene,world)=slot else {
                                return Err("Scene is not ready".into());
                            };
                            let Some(world)=Arc::get_mut(world) else {
                                return Err("Scene is still borrowed".into());
                            };
                            let doc=scene_instances::parse(&target.overlay,name,world)?;
                            let index=index.as_ref().ok_or("Original disc index unavailable")?;
                            scene_instances::apply(&doc,world,scene.placements.len(),index)?;
                            self.native_selected=target.selected.as_ref()
                                .and_then(|id|editor_history_selected(world,id));
                            self.native_selection=target.selection.iter().filter_map(|id|editor_history_selected(world,id)).collect();
                            self.native_hierarchy_anchor=self.native_selected;
                            if self.editor_properties_target.is_some() {self.editor_properties_target=self.native_selected;}
                            Ok(())
                        });
                    match restore {
                        Ok(())=>{
                            if pending_history_undo {self.editor_undo.pop();
                                if let Some(current)=current {self.editor_redo.push(current);}}
                            else {self.editor_redo.pop();
                                if let Some(current)=current {self.editor_undo.push(current);}}
                            self.native_drag=None;
                            self.native_rotate=None;
                            self.native_view.version=self.native_view.version.wrapping_add(1);
                            self.studio.status=Some(if pending_history_undo {
                                "Undo applied (editor scene only).".into()
                            } else {"Redo applied (editor scene only).".into()});
                            ctx.request_repaint();
                        }
                        Err(error)=>self.studio.error=Some(format!("Cannot restore edit: {error}")),
                    }
                }
            }
        }
    }
    fn top_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(
                RichText::new("Legaia Studio")
                    .heading()
                    .color(crate::theme::accent(ui)),
            );
            ui.separator();
            for (mode, label, key) in [
                (Mode::Browse, "Browse", "Ctrl+1"),
                (Mode::Actors, "Characters & monsters", "Ctrl+2"),
                (Mode::Media, "Movies & voice", "Ctrl+3"),
                (Mode::Extract, "Extract", "Ctrl+4"),
                (Mode::Editor, "World Editor", "Ctrl+5"),
            ] {
                ui.selectable_value(&mut self.mode, mode, label)
                    .on_hover_text(key);
            }
            ui.separator();
            if ui
                .button("Open disc image...")
                .on_hover_text("Ctrl+O")
                .clicked()
            {
                self.pick_disc();
            }
            if ui.button("Open extracted folder...").clicked() {
                self.pick_folder();
            }
            ui.add_enabled_ui(!self.studio.settings.recent.is_empty(), |ui| {
                ui.menu_button("Recent", |ui| {
                    if let Some(p) = self.recent_list(ui) {
                        ui.close();
                        self.open_path(&p);
                    }
                })
                .response
                .on_disabled_hover_text("Discs and folders you open are listed here.");
            });
            ui.separator();
            self.play_button(ui);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("About").on_hover_text("F1").clicked() {
                    self.about_open = !self.about_open;
                }
                ui.menu_button("View", |ui| self.view_menu(ui));
            });
        });
        ui.horizontal(|ui| {
            ui.label("Path:");
            let resp = ui.add(
                egui::TextEdit::singleline(&mut self.path_text)
                    .hint_text(
                        "disc .bin / .cue, or an extracted folder - or drop it on the window",
                    )
                    .desired_width(ui.available_width() - 70.0),
            );
            let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if (ui.button("Open").clicked() || enter) && !self.path_text.trim().is_empty() {
                let p = PathBuf::from(self.path_text.trim());
                self.open_path(&p);
            }
        });
        ui.horizontal(|ui| {
            if let Some(p) = self.studio.loading() {
                ui.spinner();
                ui.label(format!("Loading {} ...", p.display()));
            } else if let Some(lib) = &self.studio.library {
                let kind = match lib.source {
                    Source::Disc(_) => "disc image",
                    Source::Extracted(_) => "extracted folder",
                };
                ui.label(format!(
                    "{kind}: {} - {} PROT entries, {} ISO files{}",
                    lib.source.path().display(),
                    lib.entries.len(),
                    lib.iso_files.len(),
                    lib.region
                        .as_ref()
                        .map(|r| format!(", region {r}"))
                        .unwrap_or_default()
                ));
                let n = self.studio.classified();
                if n < lib.entries.len() {
                    ui.spinner();
                    ui.label(format!("classifying {n}/{}", lib.entries.len()));
                }
            } else {
                ui.label("No disc open. Legaia Studio reads your own Legend of Legaia disc image; no game data is included.");
            }
        });
        let mut dismiss = false;
        let mut show: Option<PathBuf> = None;
        // An informational line clears itself after `STATUS_TTL` (the clock
        // restarts while the pointer is on it); an error stays until it is
        // dismissed, and can be copied for a bug report.
        let now = std::time::Instant::now();
        match &self.studio.status {
            Some(s) if self.status_seen.as_ref().is_none_or(|(seen, _)| seen != s) => {
                self.status_seen = Some((s.clone(), now));
            }
            None => self.status_seen = None,
            _ => {}
        }
        if let Some(e) = &self.studio.error {
            // A banner that wraps, with its buttons first so a long message
            // (a path, a log tail) can never push them off the window.
            let red = ui.visuals().error_fg_color;
            egui::Frame::new()
                .fill(red.gamma_multiply(0.12))
                .stroke(egui::Stroke::new(1.0_f32, red.gamma_multiply(0.5)))
                .corner_radius(4.0)
                .inner_margin(egui::Margin::symmetric(8, 5))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal_top(|ui| {
                        if ui
                            .small_button("Copy")
                            .on_hover_text("Copy this message to the clipboard")
                            .clicked()
                        {
                            ui.ctx().copy_text(e.clone());
                        }
                        dismiss = ui.small_button("Dismiss").clicked();
                        // In a vertical child the wrapped lines count
                        // toward the banner's height; laid straight into
                        // the horizontal row, every line past the first
                        // spilled out of the frame and off the panel.
                        ui.vertical(|ui| {
                            ui.add(egui::Label::new(RichText::new(e).color(red)).wrap());
                        });
                    });
                });
        } else if let Some(s) = &self.studio.status {
            let green = crate::theme::good(ui);
            let row = ui.horizontal(|ui| {
                ui.colored_label(green, s);
                if let Some(dir) = &self.last_written
                    && ui
                        .small_button("Show folder")
                        .on_hover_text(dir.display().to_string())
                        .clicked()
                {
                    show = Some(dir.clone());
                }
                dismiss = ui.small_button("Dismiss").clicked();
            });
            if let Some((_, at)) = &mut self.status_seen {
                if row.response.contains_pointer() {
                    *at = now;
                }
                let left = STATUS_TTL.saturating_sub(now.duration_since(*at));
                if left.is_zero() {
                    dismiss = true;
                } else {
                    ui.ctx().request_repaint_after(left);
                }
            }
        }
        if dismiss {
            self.studio.error = None;
            self.studio.status = None;
            self.status_seen = None;
        }
        if let Some(dir) = show {
            self.show_folder(&dir);
        }
    }

    /// The recent discs and folders, newest first, each with a remove
    /// button; the one clicked to open, if any. Entries whose path has gone
    /// since start-up are dropped as the list is drawn.
    fn recent_list(&mut self, ui: &mut egui::Ui) -> Option<PathBuf> {
        if self.studio.settings.prune_recent() {
            self.studio.save_settings();
        }
        let mut open = None;
        let mut forget = None;
        for p in &self.studio.settings.recent {
            let kind = if p.is_dir() { "folder" } else { "disc" };
            let name = p
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| p.display().to_string());
            ui.horizontal(|ui| {
                if ui
                    .small_button("x")
                    .on_hover_text("Remove from this list (the file is not touched)")
                    .clicked()
                {
                    forget = Some(p.clone());
                }
                let current = self
                    .studio
                    .library
                    .as_ref()
                    .is_some_and(|_| self.studio.settings.last_source.as_ref() == Some(p));
                let text = format!("{name}  ({kind})");
                if ui
                    .add(egui::Button::selectable(current, text))
                    .on_hover_text(p.display().to_string())
                    .clicked()
                {
                    open = Some(p.clone());
                }
                if let Some(dir) = p.parent() {
                    ui.label(RichText::new(dir.display().to_string()).weak());
                }
            });
        }
        if let Some(p) = forget {
            self.studio.settings.forget_recent(&p);
            self.studio.save_settings();
        }
        if !self.studio.settings.recent.is_empty() {
            ui.separator();
            if ui.button("Clear list").clicked() {
                self.studio.settings.recent.clear();
                self.studio.save_settings();
            }
        }
        open
    }

    /// The View menu: theme and UI zoom, both remembered.
    fn view_menu(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("Theme").strong());
        let mut theme = self.studio.settings.theme;
        for t in crate::settings::Theme::ALL {
            ui.radio_value(&mut theme, t, t.name());
        }
        if theme != self.studio.settings.theme {
            self.studio.settings.theme = theme;
            ui.ctx().set_theme(theme_preference(theme));
            self.studio.save_settings();
        }
        ui.separator();
        ui.label(RichText::new(format!("Zoom ({:.0}%)", ui.ctx().zoom_factor() * 100.0)).strong());
        egui::gui_zoom::zoom_menu_buttons(ui);
    }

    /// Push the remembered theme and zoom into egui once, then keep the
    /// settings in step with zoom changes (the keyboard shortcuts egui
    /// handles itself, and the View menu).
    fn sync_appearance(&mut self, ctx: &egui::Context) {
        let s = &mut self.studio.settings;
        if !self.appearance_applied {
            self.appearance_applied = true;
            crate::theme::install(ctx);
            ctx.set_theme(theme_preference(s.theme));
            ctx.set_zoom_factor(crate::settings::sane_zoom(s.zoom));
            return;
        }
        let z = ctx.zoom_factor();
        if (z - crate::settings::sane_zoom(s.zoom)).abs() > 1e-3 {
            s.zoom = (z.is_finite() && (z - 1.0).abs() > 1e-3).then_some(z);
            self.studio.save_settings();
        }
    }

    /// Open `dir` in the file manager, reporting a failure on the error line.
    fn show_folder(&mut self, dir: &Path) {
        if let Err(e) = crate::launch::open_folder(dir) {
            self.studio.error = Some(format!(
                "Could not open a file manager on {}: {e}",
                dir.display()
            ));
        }
    }

    /// *Play*: start the game on the open disc, when the game binary ships
    /// beside this one and the disc is the one it runs on. The hover text
    /// says why it is unavailable otherwise.
    fn play_button(&mut self, ui: &mut egui::Ui) {
        let running = self.game.is_some();
        let disc = match &self.studio.library {
            Some(lib) => crate::launch::playable_disc(lib),
            None => Err("Open your Legend of Legaia (USA) disc image first.".to_string()),
        };
        let blocked = match (&self.engine, &disc) {
            _ if running => Some("The game is running.".to_string()),
            (None, _) => Some(format!(
                "{} was not found beside legaia-studio. It ships in the same \
                 release folder; keep the two programs together.",
                crate::launch::engine_file_name()
            )),
            (_, Err(why)) => Some(why.clone()),
            _ => None,
        };
        let button = if running {
            egui::Button::new("Rust port running")
        } else if blocked.is_none() {
            crate::theme::primary_button("Run Rust port")
        } else {
            egui::Button::new("Run Rust port")
        };
        let resp = ui.add_enabled(blocked.is_none(), button);
        let resp = match &blocked {
            Some(why) => resp.on_disabled_hover_text(why),
            None => resp.on_hover_text(
                "Launch the separate Rust engine reimplementation (F5). This does not launch DuckStation, \
                 compile editor changes, or validate a modified native PS1 disc.",
            ),
        };
        if resp.clicked() {
            self.start_game();
        }
    }

    /// Start the game on the open disc (the Play button, `LEGAIA_STUDIO_PLAY`).
    fn start_game(&mut self) {
        let disc = match &self.studio.library {
            Some(lib) => crate::launch::playable_disc(lib),
            None => return,
        };
        if self.game.is_some() {
            return;
        }
        if let (Some(engine), Ok(bin)) = (self.engine.clone(), disc) {
            self.player.stop();
            // The launch switches the game's own launcher keeps
            // (`--psx-render`, `--retail`), so both starts look the same.
            let extra = crate::launch::engine_launcher_settings()
                .map(|f| crate::launch::launch_switches(&f))
                .unwrap_or_default();
            match crate::launch::Game::start(
                &engine,
                &bin,
                &extra,
                crate::launch::engine_data_dir().as_deref(),
            ) {
                Ok(g) => {
                    self.studio.status = Some(format!("Started the game on {}", bin.display()));
                    self.studio.error = None;
                    self.last_written = None;
                    self.game = Some(g);
                    // Remember the disc for the game itself, so starting it
                    // directly later does not ask for it again. The path as
                    // chosen (a `.cue` stays a `.cue`), like its own picker.
                    let chosen = self
                        .studio
                        .settings
                        .last_source
                        .clone()
                        .filter(|p| p.is_file())
                        .unwrap_or(bin);
                    if let Some(file) = crate::launch::engine_launcher_settings()
                        && let Err(e) = crate::launch::record_disc_for_engine(&file, &chosen)
                    {
                        log::warn!("not recording the disc for the game: {e:#}");
                    }
                }
                Err(e) => self.studio.error = Some(format!("Could not start the game: {e:#}")),
            }
        }
    }

    /// Notice when a started game ends; a failed exit is reported with the
    /// last lines of its log.
    fn poll_game(&mut self, ctx: &egui::Context) {
        let Some(game) = &mut self.game else { return };
        ctx.request_repaint_after(std::time::Duration::from_millis(500));
        let Some(res) = game.poll() else { return };
        let log = game.log.clone();
        self.game = None;
        match res {
            Ok(()) => self.studio.status = Some("The game has closed.".into()),
            Err(status) => {
                let mut msg = format!("The game stopped with an error ({status}).");
                if let Some(log) = &log {
                    for line in crate::launch::log_tail(log, 3) {
                        msg.push_str("\n  ");
                        msg.push_str(&line);
                    }
                    msg.push_str(&format!("\nFull log: {}", log.display()));
                }
                self.studio.error = Some(msg);
            }
        }
    }

    fn browser(&mut self, ui: &mut egui::Ui) {
        let Some(lib) = self.studio.library.clone() else {
            ui.label("Open a disc image to browse its PROT.DAT archive.");
            return;
        };
        ui.horizontal(|ui| {
            ui.label("Search:");
            // The field takes the panel's width, less the clear button and
            // the field's own inner margin (outside `desired_width`). A
            // resizable panel grows to fit its content, so a field even a
            // few points too wide would widen the panel on every frame.
            let clear_w = if self.filter.is_empty() { 0.0 } else { 28.0 };
            let resp = ui.add(
                egui::TextEdit::singleline(&mut self.filter)
                    .hint_text("scene, place name, category or index (Ctrl+F)")
                    .desired_width((ui.available_width() - clear_w - 16.0).max(80.0)),
            );
            if !self.filter.is_empty()
                && ui
                    .small_button("x")
                    .on_hover_text("Clear the search (Esc)")
                    .clicked()
            {
                self.filter.clear();
            }
            if ui.input_mut(|i| {
                i.consume_shortcut(&egui::KeyboardShortcut::new(
                    egui::Modifiers::COMMAND,
                    egui::Key::F,
                ))
            }) {
                resp.request_focus();
            }
            if resp.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                self.filter.clear();
            }
        });
        let mut classes: Vec<&'static str> = (0..lib.entries.len())
            .filter_map(|p| self.studio.class_of(p).map(|c| c.name()))
            .collect();
        classes.sort_unstable();
        classes.dedup();
        egui::ComboBox::from_label("Category")
            .selected_text(self.class_filter.as_deref().unwrap_or("all"))
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut self.class_filter, None, "all");
                for c in classes {
                    ui.selectable_value(&mut self.class_filter, Some(c.to_string()), c);
                }
            });
        let filter = self.filter.trim().to_string();
        let filtering = !filter.is_empty() || self.class_filter.is_some();
        // The rows the search shows, per block, in disc order.
        let shown: Vec<(usize, Vec<usize>)> = self
            .studio
            .blocks
            .iter()
            .enumerate()
            .map(|(b, block)| {
                let rows = block
                    .entries
                    .iter()
                    .copied()
                    .filter(|&pos| {
                        self.studio.entry_matches(
                            pos,
                            &block.name,
                            &filter,
                            self.class_filter.as_deref(),
                        )
                    })
                    .collect();
                (b, rows)
            })
            .filter(|(_, rows): &(usize, Vec<usize>)| !rows.is_empty())
            .collect();
        // Every row the search shows, in disc order: what the arrow keys walk.
        let visible: Vec<usize> = shown.iter().flat_map(|(_, r)| r.iter().copied()).collect();
        if filtering {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(format!(
                        "{} of {} entries match, in {} blocks",
                        visible.len(),
                        lib.entries.len(),
                        shown.len()
                    ))
                    .weak(),
                );
                if ui.small_button("Show all").clicked() {
                    self.filter.clear();
                    self.class_filter = None;
                }
            });
        }
        ui.separator();
        if filtering && visible.is_empty() {
            ui.add_space(8.0);
            ui.label(RichText::new("Nothing matches.").strong());
            ui.add(
                egui::Label::new(
                    "Every word has to match a scene label (town01), a place name (Rim Elm), \
                     a kind of data (tim, vab, scene_tmd_stream) or an entry number (865 or \
                     0x361).",
                )
                .wrap(),
            );
        }
        let mut clicked = None;
        let mut iso_save: Option<crate::library::IsoFile> = None;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
                for (b, rows) in shown {
                    let block = &self.studio.blocks[b];
                    let first = lib.entries[rows[0]].index;
                    let open = filtering.then_some(true).or_else(|| {
                        self.reveal
                            .filter(|s| block.entries.contains(s))
                            .map(|_| true)
                    });
                    let title = match crate::place_name(&block.name) {
                        Some(place) => {
                            format!("{first:04}  {}  -  {place}  ({})", block.name, rows.len())
                        }
                        None => format!("{first:04}  {}  ({})", block.name, rows.len()),
                    };
                    egui::CollapsingHeader::new(title)
                        .id_salt(("block", first))
                        .open(open)
                        .show(ui, |ui| {
                            for pos in rows {
                                let e = &lib.entries[pos];
                                let class =
                                    self.studio.class_of(pos).map(|c| c.name()).unwrap_or("...");
                                let text = format!(
                                    "{:04}  {:>8}  {}",
                                    e.index,
                                    human_size(e.size_bytes),
                                    class
                                );
                                let sel = self.studio.selected == Some(pos);
                                let resp =
                                    ui.selectable_label(sel, RichText::new(text).monospace());
                                if self.reveal == Some(pos) {
                                    resp.scroll_to_me(Some(egui::Align::Center));
                                }
                                if resp.clicked() {
                                    clicked = Some(pos);
                                }
                            }
                        });
                }
                if !lib.iso_files.is_empty() {
                    ui.separator();
                    egui::CollapsingHeader::new(format!("Disc files ({})", lib.iso_files.len()))
                        .id_salt("iso")
                        .show(ui, |ui| {
                            for f in lib.iso_files.clone() {
                                ui.horizontal(|ui| {
                                    ui.monospace(format!(
                                        "{:<20} {:>9}",
                                        f.path,
                                        human_size(f.size as u64)
                                    ));
                                    if ui.small_button("Save...").clicked() {
                                        iso_save = Some(f.clone());
                                    }
                                });
                            }
                        });
                }
            });
        self.reveal = None;
        if let Some(pos) = clicked {
            self.select(pos);
        }
        // Keyboard: with no text field focused, the arrows, Page Up / Down,
        // Home and End walk the rows the search shows, opening a block and
        // scrolling to the entry as they go.
        let typing = ui.ctx().wants_keyboard_input();
        if !typing && !visible.is_empty() {
            let cur = self
                .studio
                .selected
                .and_then(|s| visible.iter().position(|&p| p == s));
            let last = visible.len() - 1;
            let step = ui.input(|i| {
                if i.key_pressed(egui::Key::ArrowDown) {
                    Some(cur.map_or(0, |c| (c + 1).min(last)))
                } else if i.key_pressed(egui::Key::ArrowUp) {
                    Some(cur.map_or(0, |c| c.saturating_sub(1)))
                } else if i.key_pressed(egui::Key::PageDown) {
                    Some(cur.map_or(0, |c| (c + 20).min(last)))
                } else if i.key_pressed(egui::Key::PageUp) {
                    Some(cur.map_or(0, |c| c.saturating_sub(20)))
                } else if i.key_pressed(egui::Key::Home) {
                    Some(0)
                } else if i.key_pressed(egui::Key::End) {
                    Some(last)
                } else {
                    None
                }
            });
            if let Some(k) = step
                && Some(k) != cur
            {
                let pos = visible[k];
                self.select(pos);
                self.reveal = Some(pos);
            }
        }
        if let Some(f) = iso_save {
            let name = f.path.rsplit('/').next().unwrap_or(&f.path).to_string();
            if let Some(out) = self.export_target(&name, "bin") {
                let res = lib.iso_file_bytes(&f).and_then(|b| export::bytes(&b, &out));
                self.report(&f.path, res, &out);
            }
        }
    }

    fn preview(&mut self, ui: &mut egui::Ui) {
        let Some(lib) = self.studio.library.clone() else {
            self.start_page(ui);
            return;
        };
        let Some(pos) = self.studio.selected else {
            ui.add_space(12.0);
            ui.label(
                RichText::new("Pick an entry on the left to preview it")
                    .size(17.0)
                    .color(crate::theme::accent(ui)),
            );
            ui.add_space(6.0);
            for line in [
                "Each block is one scene or system file of the game, named as on the disc \
                 and by the place it is (town01 - Rim Elm).",
                "Search by place, kind of data or entry number (Ctrl+F); the arrow keys \
                 walk the list.",
                "Characters & monsters (Ctrl+2) and Movies & voice (Ctrl+3) show the \
                 game's actors and movies directly; Extract (Ctrl+4) unpacks everything.",
            ] {
                ui.add(egui::Label::new(line).wrap());
            }
            return;
        };
        let Some(a) = self.studio.analysis.clone().filter(|a| a.pos == pos) else {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Reading entry...");
            });
            return;
        };
        let e = &lib.entries[pos];
        ui.horizontal(|ui| {
            ui.heading(lib.stem(e.index));
            ui.label(format!("{} - {}", a.class.name(), human_size(e.size_bytes)));
        });
        let count = |t: Tab| match t {
            Tab::Textures => Some(a.tims.len()),
            Tab::Model => Some(a.tmds.len()),
            Tab::Audio => Some(a.vabs.len() + a.seqs.len()),
            Tab::Text => Some(a.mes.len() + a.dialog.len()),
            _ => None,
        };
        // The chosen tab stays the preference while walking entries; an entry
        // with nothing for it shows its Overview instead of an empty tab.
        let shown = if count(self.tab) == Some(0) {
            Tab::Overview
        } else {
            self.tab
        };
        ui.horizontal(|ui| {
            for t in Tab::ALL {
                let n = count(t);
                let label = match n {
                    Some(n) => format!("{} ({n})", t.name()),
                    None => t.name().to_string(),
                };
                let enabled = n.is_none_or(|n| n > 0);
                let resp = ui.add_enabled(enabled, egui::Button::selectable(shown == t, label));
                if resp.clicked() {
                    self.tab = t;
                }
            }
        });
        ui.separator();
        match shown {
            Tab::Overview => self.overview(ui, &lib, &a),
            Tab::Textures => self.textures(ui, &lib, &a),
            Tab::Model => self.model_tab(ui, &lib, &a),
            Tab::Audio => self.audio(ui, &lib, &a),
            Tab::Text => self.text(ui, &lib, &a),
            Tab::Hex => hex_view(ui, &a.bytes),
        }
    }

    fn overview(&mut self, ui: &mut egui::Ui, lib: &crate::Library, a: &Analysis) {
        let e = &lib.entries[a.pos];
        egui::Grid::new("overview").num_columns(2).striped(true).show(ui, |ui| {
            ui.label("PROT index");
            ui.monospace(format!("{} (0x{:X})", e.index, e.index));
            ui.end_row();
            ui.label("Filename label");
            ui.monospace(lib.label(e.index).unwrap_or("-"));
            ui.end_row();
            ui.label("Retail CDNAME block");
            ui.monospace(lib.retail_block(e.index).unwrap_or("-"))
                .on_hover_text("CDNAME #define numbers are raw-TOC indices, two above extraction indices (docs/formats/cdname.md)");
            ui.end_row();
            ui.label("Category");
            ui.monospace(a.class.name());
            ui.end_row();
            ui.label("Location");
            ui.monospace(format!(
                "PROT.DAT LBA {} (+0x{:X}), {} sectors, {} bytes",
                e.start_lba, e.byte_offset, e.size_sectors, e.size_bytes
            ));
            ui.end_row();
            ui.label("Contents");
            ui.monospace(format!(
                "{} TIM, {} TMD, {} VAB, {} SEQ, {} MES, {} LZS sections",
                a.tims.len(),
                a.tmds.len(),
                a.vabs.len(),
                a.seqs.len(),
                a.mes.len(),
                a.tim_sections.len()
            ));
            ui.end_row();
        });
        if !a.chunks.is_empty() {
            ui.add_space(8.0);
            ui.label(RichText::new("DATA_FIELD chunks").strong());
            egui::Grid::new("chunks").striped(true).show(ui, |ui| {
                ui.label("#");
                ui.label("offset");
                ui.label("type");
                ui.label("size");
                ui.end_row();
                for (i, c) in a.chunks.iter().enumerate() {
                    ui.monospace(i.to_string());
                    ui.monospace(format!("0x{:X}", c.header_offset));
                    ui.monospace(c.type_name);
                    ui.monospace(c.size.to_string());
                    ui.end_row();
                }
            });
        }
        ui.add_space(8.0);
        if ui.button("Export raw entry (.BIN)...").clicked() {
            let name = format!("{}.BIN", lib.stem(e.index));
            if let Some(out) = self.export_target(&name, "BIN") {
                let res = export::bytes(&a.bytes, &out);
                self.report("entry", res, &out);
            }
        }
    }

    fn textures(&mut self, ui: &mut egui::Ui, lib: &crate::Library, a: &Analysis) {
        if a.tims.is_empty() {
            ui.label("No TIM textures in this entry.");
            return;
        }
        self.tim_sel = self.tim_sel.min(a.tims.len() - 1);
        egui::SidePanel::left("tim_list")
            .resizable(true)
            // Wide enough for one unwrapped row ("#0   320x8 4bpp lzs#0 @0x4F00").
            .default_width(270.0)
            .show_inside(ui, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    for (i, t) in a.tims.iter().enumerate() {
                        let text = format!(
                            "#{i:<3} {}x{} {}bpp {} @0x{:X}",
                            t.width,
                            t.height,
                            t.bpp,
                            t.origin.describe(),
                            t.offset
                        );
                        if ui
                            .selectable_label(self.tim_sel == i, RichText::new(text).monospace())
                            .clicked()
                        {
                            self.tim_sel = i;
                            self.clut = 0;
                        }
                    }
                });
            });
        let t = a.tims[self.tim_sel].clone();
        // Wraps rather than clipping the export buttons off a narrow pane.
        ui.horizontal_wrapped(|ui| {
            if t.palettes > 1 {
                ui.label("CLUT:");
                ui.add(egui::Slider::new(&mut self.clut, 0..=t.palettes - 1));
            } else if t.palettes == 1 {
                ui.label("1 CLUT");
            } else if t.bpp <= 8 {
                ui.label("no CLUT in the file: indices shown in grey")
                    .on_hover_text(
                        "The game draws this texture with a palette another upload left \
                         in VRAM, so the file alone has no colours. The 3D model tab's \
                         scene VRAM shows it as the game does.",
                    );
            } else {
                ui.label("direct colour (no CLUT)");
            }
            ui.separator();
            ui.label("Zoom:");
            ui.add(egui::Slider::new(&mut self.tim_zoom, 0.5..=8.0).step_by(0.5));
            ui.separator();
            if ui.button("Export PNG...").clicked() {
                let name = format!(
                    "{}_tim{:02}_clut{}.png",
                    lib.stem(a.index),
                    self.tim_sel,
                    self.clut
                );
                if let Some(out) = self.export_target(&name, "png") {
                    let res = export::tim_png(a, &t, self.clut, &out);
                    self.report("PNG", res, &out);
                }
            }
            if ui.button("Export all PNGs...").clicked() {
                let name = format!("{}_tim00.png", lib.stem(a.index));
                if let Some(first) = self.export_target(&name, "png") {
                    let dir = first.parent().map(Path::to_path_buf).unwrap_or_default();
                    let mut n = 0;
                    for (i, t) in a.tims.iter().enumerate() {
                        let p = dir.join(format!("{}_tim{i:02}.png", lib.stem(a.index)));
                        if export::tim_png(a, t, 0, &p).is_ok() {
                            n += 1;
                        }
                    }
                    self.studio.status = Some(format!("{n} PNGs written to {}", dir.display()));
                }
            }
        });
        let key = (a.pos, self.tim_sel, self.clut);
        // A failed decode is remembered with its key like a texture, so the
        // message stays up and the TIM is not re-decoded every frame.
        let have = self.tim_tex.as_ref().map(|(k, _)| *k) == Some(key)
            || self.tim_err.as_ref().map(|(k, _)| *k) == Some(key);
        if !have {
            self.tim_tex = None;
            self.tim_err = None;
            match export::tim_rgba(a, &t, self.clut) {
                Ok((w, h, rgba)) if w > 0 && h > 0 => {
                    let img = ColorImage::from_rgba_unmultiplied([w, h], &rgba);
                    self.tim_tex = Some((
                        key,
                        ui.ctx().load_texture("tim", img, TextureOptions::NEAREST),
                    ));
                }
                Ok((w, h, _)) => {
                    self.tim_err = Some((key, format!("empty TIM ({w}x{h})")));
                }
                Err(e) => self.tim_err = Some((key, format!("{e:#}"))),
            }
        }
        if let Some((_, e)) = &self.tim_err {
            ui.colored_label(ui.visuals().error_fg_color, e);
        }
        if let Some((_, tex)) = &self.tim_tex {
            let size = tex.size_vec2() * self.tim_zoom;
            egui::ScrollArea::both().show(ui, |ui| {
                let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
                checkerboard(ui.painter(), rect);
                ui.painter().image(
                    tex.id(),
                    rect,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
            });
        }
    }

    /// The scene VRAM for entry `a`, starting its build if needed. `Err`
    /// carries a status line (building / failed / no scene).
    fn scene_vram(
        &mut self,
        lib: &Arc<crate::Library>,
        a: &Analysis,
    ) -> Result<(String, Arc<SceneVram>), String> {
        let Some(name) = lib.retail_block(a.index).map(str::to_string) else {
            return Err("this entry belongs to no CDNAME scene".into());
        };
        let kind = crate::scene_vram::load_kind(&name, a.class);
        let key = format!("{name}:{kind:?}");
        let slot = self.scenes.entry(key.clone()).or_insert_with(|| {
            let (tx, rx) = std::sync::mpsc::channel();
            let lib = Arc::clone(lib);
            let (index, class) = (a.index, a.class);
            std::thread::spawn(move || {
                let _ = tx.send(
                    crate::scene_vram::build(&lib, index, class).map_err(|e| format!("{e:#}")),
                );
            });
            SceneSlot::Building(rx)
        });
        if let SceneSlot::Building(rx) = slot
            && let Ok(res) = rx.try_recv()
        {
            *slot = match res {
                Ok(v) => SceneSlot::Ready(Arc::new(v)),
                Err(e) => SceneSlot::Failed(e),
            };
        }
        match slot {
            SceneSlot::Ready(v) => Ok((key, Arc::clone(v))),
            SceneSlot::Building(_) => Err(format!("building scene {name} ({kind:?} load)...")),
            SceneSlot::Failed(e) => Err(format!("scene {name}: {e}")),
        }
    }

    fn model_tab(&mut self, ui: &mut egui::Ui, lib: &Arc<crate::Library>, a: &Analysis) {
        if a.tmds.is_empty() {
            ui.label("No TMD meshes in this entry.");
            return;
        }
        self.tmd_sel = self.tmd_sel.min(a.tmds.len() - 1);
        let t = a.tmds[self.tmd_sel].clone();
        let key = (a.pos, self.tmd_sel);
        if self.model.as_ref().map(|m| m.key) != Some(key) {
            // Start on the entry's own TIMs only when they hold every CLUT
            // and texel the mesh's textured prims read; otherwise the mesh
            // samples pages another entry of its scene uploads, so start
            // on the scene.
            let model = Model::from_tmd(a.tmd_bytes(&t));
            let entry_vram = a.vram();
            let source = default_tex_source(model.as_ref(), &entry_vram);
            self.model = Some(ModelView {
                key,
                model,
                entry_vram,
                source,
                drawn_with: None,
                view: Viewport::new(Shading::Textured),
            });
        }
        // Wraps: at the default window width the export buttons used to run
        // off the right edge.
        ui.horizontal_wrapped(|ui| {
            egui::ComboBox::from_id_salt("tmd")
                .selected_text(format!("TMD #{}", self.tmd_sel))
                .show_ui(ui, |ui| {
                    for (i, t) in a.tmds.iter().enumerate() {
                        ui.selectable_value(
                            &mut self.tmd_sel,
                            i,
                            format!(
                                "#{i} {} @0x{:X}: {} obj, {} verts, {} prims{}",
                                t.origin.describe(),
                                t.offset,
                                t.objects,
                                t.verts,
                                t.prims,
                                if t.scene_stream {
                                    " (scene stream)"
                                } else {
                                    ""
                                }
                            ),
                        );
                    }
                });
            if let Some(mv) = &mut self.model {
                mv.view.controls(
                    ui,
                    "texel x colour / 128, sampled from the chosen VRAM image",
                );
                if let Some(m) = &mv.model {
                    ui.label(format!("{} triangles", m.triangles()));
                }
            }
            ui.separator();
            if ui.button("Export OBJ...").clicked() {
                let name = format!("{}_tmd{:02}.obj", lib.stem(a.index), self.tmd_sel);
                if let Some(out) = self.export_target(&name, "obj") {
                    let res = export::tmd_obj(a, &t, &out);
                    self.report("OBJ", res, &out);
                }
            }
            if ui.button("Export glTF (.glb)...").clicked() {
                let name = format!("{}_tmd{:02}.glb", lib.stem(a.index), self.tmd_sel);
                let scene = match self.model.as_ref().map(|m| m.source) {
                    Some(TexSource::Scene) => self.scene_vram(lib, a).ok().map(|(_, v)| v),
                    _ => None,
                };
                if let Some(out) = self.export_target(&name, "glb") {
                    let res = export::tmd_glb(a, &t, scene.as_ref().map(|v| &v.vram), &out);
                    self.report("glTF", res, &out);
                }
            }
        });
        let mut status = None;
        let scene = if self
            .model
            .as_ref()
            .is_some_and(|m| m.source == TexSource::Scene)
        {
            match self.scene_vram(lib, a) {
                Ok(v) => Some(v),
                Err(e) => {
                    ui.ctx()
                        .request_repaint_after(std::time::Duration::from_millis(100));
                    status = Some(e);
                    None
                }
            }
        } else {
            None
        };
        let Some(mv) = &mut self.model else { return };
        ui.horizontal_wrapped(|ui| {
            ui.label("Textures:");
            ui.selectable_value(
                &mut mv.source,
                TexSource::Entry,
                format!("this entry ({} TIM)", a.tims.len()),
            )
            .on_hover_text("Only the TIMs stored inside the selected entry");
            ui.selectable_value(
                &mut mv.source,
                TexSource::Scene,
                format!("scene {}", lib.retail_block(a.index).unwrap_or("-")),
            )
            .on_hover_text(
                "The VRAM the engine builds for this entry's CDNAME scene: every TIM of the block, \
                 the shared init_data / player_data blocks, the system-UI bundle, and the \
                 field-character atlas and effect-texture pool the play host adds",
            );
            match (&scene, &status) {
                (Some((_, v)), _) => {
                    ui.label(
                        RichText::new(format!(
                            "{:?} load: {} TIMs ({} shared) + {} host layers",
                            v.kind,
                            v.tims,
                            v.shared_tims,
                            v.layers.len()
                        ))
                        .weak(),
                    )
                    .on_hover_text(v.layers.join("\n"));
                }
                (None, Some(e)) if mv.source == TexSource::Scene => {
                    ui.spinner();
                    ui.label(RichText::new(e).weak());
                }
                _ => {}
            }
            ui.label(RichText::new("Drag to orbit, scroll to zoom.").weak());
        });
        let Some(model) = &mv.model else {
            ui.colored_label(
                ui.visuals().error_fg_color,
                "This TMD has no drawable triangles.",
            );
            return;
        };
        let (vram, with) = match (&scene, mv.source) {
            (Some((k, v)), TexSource::Scene) => (&v.vram, Some(k.clone())),
            _ => (&mv.entry_vram, None),
        };
        if mv.drawn_with != with {
            mv.drawn_with = with;
            mv.view.version += 1;
        }
        let dark = model.dark_baked_share();
        if mv.view.shading == Shading::Textured && dark >= legaia_tmd::mesh::MOSTLY_DARK_SHARE {
            ui.label(
                RichText::new(format!(
                    "{:.0}% of this mesh's surface carries near-black colour words on the disc. \
                     Retail scales every texel by that word, so it draws black here (in the \
                     game the scene's depth cue then blends it toward the far colour): the \
                     darkness is the data, not a missing texture. \
                     \"Untextured + viewer light\" shows the shape.",
                    dark * 100.0
                ))
                .weak(),
            );
        }
        mv.view.show(ui, model, vram);
    }

    fn audio(&mut self, ui: &mut egui::Ui, lib: &crate::Library, a: &Analysis) {
        if a.vabs.is_empty() && a.seqs.is_empty() {
            ui.label("No VAB sound banks or SEQ sequences in this entry.");
            return;
        }
        ui.horizontal(|ui| {
            match &self.player.now_playing {
                Some(n) => {
                    ui.label(format!("Playing: {n}"));
                    if let Some((tick, bpm, done)) = self.player.seq_progress() {
                        ui.label(format!(
                            "tick {tick}, {bpm:.1} BPM{}",
                            if done { ", finished" } else { "" }
                        ));
                    }
                    ui.ctx()
                        .request_repaint_after(std::time::Duration::from_millis(200));
                }
                None => {
                    ui.label("Stopped");
                }
            }
            if ui.button("Stop").clicked() {
                self.player.stop();
            }
        });
        ui.separator();
        if !a.vabs.is_empty() {
            self.vab_sel = self.vab_sel.min(a.vabs.len() - 1);
            ui.horizontal(|ui| {
                ui.label(RichText::new("Sound bank").strong());
                egui::ComboBox::from_id_salt("vab")
                    .selected_text(format!("VAB @ 0x{:X}", a.vabs[self.vab_sel].offset))
                    .show_ui(ui, |ui| {
                        for (i, v) in a.vabs.iter().enumerate() {
                            ui.selectable_value(
                                &mut self.vab_sel,
                                i,
                                format!(
                                    "VAB @ 0x{:X}: {} programs, {} samples",
                                    v.offset,
                                    v.report.programs.len(),
                                    v.report.vag_samples.len()
                                ),
                            );
                        }
                    });
                if ui.button("Export all samples (WAV)...").clicked() {
                    let stem =
                        format!("{}_vab{:X}", lib.stem(a.index), a.vabs[self.vab_sel].offset);
                    if let Some(first) = self.export_target(&format!("{stem}_000.wav"), "wav") {
                        let dir = first.parent().map(Path::to_path_buf).unwrap_or_default();
                        match export::vab_all_wavs(a, &a.vabs[self.vab_sel], &dir, &stem) {
                            Ok(n) => {
                                self.studio.status =
                                    Some(format!("{n} WAVs written to {}", dir.display()))
                            }
                            Err(e) => self.studio.error = Some(format!("{e:#}")),
                        }
                    }
                }
            });
        }
        if !a.seqs.is_empty() {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Sequences").strong());
                ui.checkbox(&mut self.seq_loop, "loop");
            });
            for s in a.seqs.clone() {
                ui.horizontal(|ui| {
                    ui.monospace(format!(
                        "SEQ @ 0x{:X}  {:.1} BPM, {} PPQN",
                        s.offset, s.bpm, s.ppqn
                    ));
                    let vab = a.vabs.get(self.vab_sel);
                    if ui
                        .add_enabled(vab.is_some(), egui::Button::new("Play with selected bank"))
                        .clicked()
                        && let Some(v) = vab
                        && let Err(e) = self.player.play_seq(a, v, &s, self.seq_loop)
                    {
                        self.studio.error = Some(format!("{e:#}"));
                    }
                    if ui.button("Save .seq...").clicked() {
                        let name = format!("{}_{:X}.seq", lib.stem(a.index), s.offset);
                        if let Some(out) = self.export_target(&name, "seq") {
                            let res = export::bytes(&a.bytes[s.offset..], &out);
                            self.report("SEQ", res, &out);
                        }
                    }
                });
            }
            ui.separator();
        }
        let Some(v) = a.vabs.get(self.vab_sel).cloned() else {
            return;
        };
        egui::ScrollArea::vertical().show_rows(
            ui,
            22.0,
            v.report.vag_samples.len(),
            |ui, range| {
                for i in range {
                    let span = &v.report.vag_samples[i];
                    ui.horizontal(|ui| {
                        ui.monospace(format!("sample {i:3}  {:>7} B", span.size));
                        if ui.small_button("Play").clicked() {
                            match export::vag_pcm(a, &v, i) {
                                Ok(pcm) => {
                                    if let Err(e) = self.player.play_pcm(
                                        pcm,
                                        legaia_engine_audio::vab_bind::VAB_SAMPLE_RATE,
                                        format!("sample {i}"),
                                    ) {
                                        self.studio.error = Some(format!("{e:#}"));
                                    }
                                }
                                Err(e) => self.studio.error = Some(format!("{e:#}")),
                            }
                        }
                        if ui.small_button("WAV...").clicked() {
                            let name =
                                format!("{}_vab{:X}_{i:03}.wav", lib.stem(a.index), v.offset);
                            if let Some(out) = self.export_target(&name, "wav") {
                                let res = export::vag_wav(a, &v, i, &out);
                                self.report("WAV", res, &out);
                            }
                        }
                    });
                }
            },
        );
    }

    fn text(&mut self, ui: &mut egui::Ui, lib: &crate::Library, a: &Analysis) {
        if a.mes.is_empty() && a.dialog.is_empty() {
            ui.label("No dialog text or MES blobs in this entry.");
            return;
        }
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if !a.dialog.is_empty() {
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(format!("{} dialog lines", a.dialog.len())).strong(),
                        );
                        ui.label(
                            RichText::new("language-pack markup: | = line break, {c1:00} = a name")
                                .weak(),
                        );
                        if ui.small_button("Save text...").clicked() {
                            let name = format!("{}_dialog.txt", lib.stem(a.index));
                            if let Some(out) = self.export_target(&name, "txt") {
                                let body: String = a
                                    .dialog
                                    .iter()
                                    .map(|l| {
                                        let space = if l.from_man { "man" } else { "raw" };
                                        format!("{space}:0x{:X}\t{}\n", l.offset, l.text)
                                    })
                                    .collect();
                                let res = export::bytes(body.as_bytes(), &out);
                                self.report("text", res, &out);
                            }
                        }
                    });
                    egui::Grid::new("dialog")
                        .striped(true)
                        .num_columns(2)
                        .show(ui, |ui| {
                            for l in &a.dialog {
                                let space = if l.from_man { "man" } else { "raw" };
                                ui.monospace(format!("{space}:0x{:05X}", l.offset));
                                ui.label(l.text.replace('|', "\n"));
                                ui.end_row();
                            }
                        });
                    ui.separator();
                }
                for (k, m) in a.mes.iter().enumerate() {
                    let place = match m.chunk {
                        Some(c) => format!("chunk {c}"),
                        None => "whole entry".into(),
                    };
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(format!(
                                "MES {k}: {} layout, {place}, 0x{:X}",
                                m.format, m.offset
                            ))
                            .strong(),
                        );
                        if ui.small_button("Save text...").clicked() {
                            let name = format!("{}_mes{k}.txt", lib.stem(a.index));
                            if let Some(out) = self.export_target(&name, "txt") {
                                let body: String = m
                                    .messages
                                    .iter()
                                    .enumerate()
                                    .map(|(i, s)| format!("[{i}]\n{s}\n\n"))
                                    .collect();
                                let res = export::bytes(body.as_bytes(), &out);
                                self.report("text", res, &out);
                            }
                        }
                    });
                    if m.messages.is_empty() {
                        ui.label(if m.records > 0 {
                            format!(
                                "{} records, none carrying a 0x1F-led text segment.",
                                m.records
                            )
                        } else {
                            "No messages decode from this blob.".to_string()
                        });
                    }
                    for (i, s) in m.messages.iter().enumerate() {
                        ui.horizontal_wrapped(|ui| {
                            ui.monospace(format!("{i:3}"));
                            ui.label(RichText::new(s).monospace());
                        });
                    }
                    ui.separator();
                }
            });
    }

    fn extract_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("Extract the whole disc");
        ui.label(
            "Runs the full legaia-extract pipeline: ISO9660 files, PROT entries with CDNAME names, \
             categorize.json, streaming sub-assets, TIM -> PNG, CD-XA audio -> WAV, the TIM catalogs \
             and the dialog font. The output folder is yours; nothing is uploaded anywhere.",
        );
        ui.add_space(6.0);
        let running = self.job.as_ref().is_some_and(|j| j.outcome().is_none());
        let disc = match self.studio.library.as_ref().map(|l| &l.source) {
            Some(Source::Disc(p)) => Some(p.clone()),
            _ => None,
        };
        // A first extraction proposes a folder beside the disc image.
        if self.out_text.trim().is_empty()
            && let Some(dir) = disc.as_deref().and_then(crate::launch::suggested_out_dir)
        {
            self.out_text = dir.display().to_string();
        }
        egui::Grid::new("extract_opts")
            .num_columns(2)
            .show(ui, |ui| {
                ui.label("Disc image");
                match &disc {
                    Some(p) => ui.monospace(p.display().to_string()),
                    None => {
                        let r = ui.add(crate::theme::primary_button("Open disc image..."));
                        if r.clicked() {
                            self.pick_disc();
                        }
                        r
                    }
                };
                ui.end_row();
                ui.label("Output folder");
                ui.horizontal(|ui| {
                    ui.add_enabled(
                        !running,
                        egui::TextEdit::singleline(&mut self.out_text)
                            .desired_width((ui.available_width() - 90.0).clamp(420.0, 900.0)),
                    );
                    if ui
                        .add_enabled(!running, egui::Button::new("Choose..."))
                        .clicked()
                        && let Some(p) = rfd::FileDialog::new()
                            .set_title("Choose the extraction output folder")
                            .pick_folder()
                    {
                        self.out_text = p.display().to_string();
                    }
                    let out = PathBuf::from(self.out_text.trim());
                    if !self.out_text.trim().is_empty()
                        && out.is_dir()
                        && ui
                            .button("Open")
                            .on_hover_text("Open this folder in the file manager")
                            .clicked()
                    {
                        self.show_folder(&out);
                    }
                });
                ui.end_row();
                // What the run will do to the chosen folder, before it starts.
                if !running && !self.out_text.trim().is_empty() {
                    ui.label("");
                    let out = PathBuf::from(self.out_text.trim());
                    match ExtractOutDir::of(&out) {
                        ExtractOutDir::New => ui.label(RichText::new("A new folder.").weak()),
                        ExtractOutDir::Empty => ui.label(RichText::new("An empty folder.").weak()),
                        ExtractOutDir::HasFiles => ui.label(
                            RichText::new(
                                "This folder already has files: ones with the same names are \
                                 replaced, the rest are left alone.",
                            )
                            .color(ui.visuals().warn_fg_color),
                        ),
                        ExtractOutDir::NotAFolder => ui.label(
                            RichText::new("This is a file, not a folder.")
                                .color(ui.visuals().error_fg_color),
                        ),
                    };
                    ui.end_row();
                }
                ui.label("Options");
                ui.vertical(|ui| {
                    let s = &mut self.studio.settings;
                    ui.add_enabled_ui(!running, |ui| {
                        ui.checkbox(
                            &mut s.skip_verify,
                            "Skip SHA-256 verification (--skip-verify)",
                        );
                        ui.checkbox(&mut s.skip_png, "Skip TIM -> PNG conversion (--skip-png)");
                        ui.checkbox(&mut s.skip_xa, "Skip CD-XA audio -> WAV (--skip-xa)");
                        ui.checkbox(
                            &mut s.skip_catalog,
                            "Skip TIM catalog TSVs (--skip-catalog)",
                        );
                        ui.checkbox(&mut s.skip_font, "Skip dialog font (--skip-font)");
                        ui.checkbox(
                            &mut s.all_textures,
                            "Export all textures as PNG: every TIM, raw and compressed, \
                             into tim_scan/ (--all-textures)",
                        );
                    });
                });
                ui.end_row();
            });
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            let can_start = !running && disc.is_some();
            let start = if can_start {
                crate::theme::primary_button("Start extraction")
            } else {
                egui::Button::new("Start extraction")
            };
            if ui
                .add_enabled(can_start, start)
                .on_disabled_hover_text(if running {
                    "An extraction is running."
                } else {
                    "Open a disc image first (an extracted folder is already extracted)."
                })
                .clicked()
            {
                self.start_extract();
            }
            if let Some(job) = &self.job
                && running
            {
                if ui.button("Cancel").clicked() {
                    job.cancel();
                }
                if job.is_cancelling() {
                    ui.label("cancelling...");
                }
            }
        });
        let Some(job) = &self.job else { return };
        let outcome = job.outcome();
        let p = job.progress();
        let steps = crate::extract_job::step_count(&job.options);
        let elapsed = crate::extract_job::human_duration(job.elapsed());
        let out = job.options.out.clone();
        ui.add_space(4.0);
        let units = if p.total > 0 {
            format!(" - {} of {}", p.done, p.total)
        } else {
            String::new()
        };
        let text = match &outcome {
            // Numbered the way the log numbers them: verification is step 0,
            // the rest count from 1.
            None if p.step == 0 => format!("Checking the disc image: {}{units}", p.step_name),
            None => format!("Step {} of {}: {}{units}", p.step, steps - 1, p.step_name,),
            Some(Outcome::Done(s)) => format!(
                "Done: {} disc files, {} PROT entries, {} streams, {} PNGs, {} WAVs{}",
                s.iso_files,
                s.prot_entries,
                s.streams,
                s.pngs,
                s.wavs,
                if s.all_textures.0 > 0 {
                    format!(
                        ", {} textures -> {} PNGs in tim_scan/",
                        s.all_textures.0, s.all_textures.1
                    )
                } else {
                    String::new()
                }
            ),
            Some(Outcome::Cancelled) => "Cancelled".into(),
            Some(Outcome::Failed(e)) => format!("Failed: {e}"),
        };
        let frac = if matches!(outcome, Some(Outcome::Done(_))) {
            1.0
        } else {
            p.fraction(steps)
        };
        let failed = matches!(outcome, Some(Outcome::Failed(_)));
        let mut bar = egui::ProgressBar::new(frac).text(text);
        if running {
            bar = bar.animate(true);
        }
        if failed {
            bar = bar.fill(ui.visuals().error_fg_color.gamma_multiply(0.6));
        }
        ui.add(bar);
        let mut show = false;
        ui.horizontal_wrapped(|ui| {
            let timing = match &outcome {
                None => match p.step_remaining(std::time::Instant::now()) {
                    Some(left) => format!(
                        "{elapsed} so far; about {} left in this step",
                        crate::extract_job::human_duration(left)
                    ),
                    None => format!("{elapsed} so far"),
                },
                Some(_) => format!("took {elapsed}"),
            };
            ui.label(RichText::new(timing).weak());
            if p.warnings > 0 {
                ui.separator();
                ui.label(
                    RichText::new(format!("{} warning(s)", p.warnings))
                        .color(ui.visuals().warn_fg_color),
                );
                ui.checkbox(&mut self.log_warnings_only, "Show only warnings");
            }
            if out.is_dir() {
                ui.separator();
                show = ui
                    .button("Open output folder")
                    .on_hover_text(out.display().to_string())
                    .clicked();
            }
        });
        if show {
            self.show_folder(&out);
        }
        if running {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(100));
        }
        ui.separator();
        let lines: Vec<&String> = if self.log_warnings_only && p.warnings > 0 {
            p.warning_lines().collect()
        } else {
            p.log.iter().collect()
        };
        let row = ui.text_style_height(&egui::TextStyle::Monospace);
        egui::ScrollArea::both()
            .stick_to_bottom(true)
            .auto_shrink([false, false])
            .show_rows(ui, row, lines.len(), |ui, range| {
                // One row per line, so the virtualised rows stay aligned.
                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
                let warn = ui.visuals().warn_fg_color;
                let err = ui.visuals().error_fg_color;
                for line in &lines[range] {
                    let text = RichText::new(line.as_str()).monospace();
                    let text = if line.starts_with(crate::extract_job::WARNING_PREFIX) {
                        text.color(warn)
                    } else if line.starts_with("FAILED") {
                        text.color(err)
                    } else {
                        text
                    };
                    ui.label(text);
                }
            });
    }

    /// Announce a finished extraction once, on whatever view is open: the
    /// run goes on in the background while the user browses, and its end
    /// used to show only on the Extract view.
    fn announce_extraction(&mut self) {
        let Some(job) = &mut self.job else { return };
        let Some(outcome) = job.take_finished() else {
            return;
        };
        let out = job.options.out.clone();
        let took = crate::extract_job::human_duration(job.elapsed());
        match outcome {
            Outcome::Done(_) => {
                self.studio.status = Some(format!(
                    "Extraction finished in {took}: written to {}",
                    out.display()
                ));
                self.last_written = Some(out);
            }
            Outcome::Cancelled => {
                self.studio.status = Some(format!(
                    "Extraction cancelled; the files written so far are in {}",
                    out.display()
                ));
                self.last_written = out.is_dir().then_some(out);
            }
            Outcome::Failed(e) => {
                self.studio.error = Some(format!("Extraction failed: {e}"));
            }
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.sync_appearance(ctx);
        // Process native .img export completion on the UI thread.
        if let Some(rx)=&self.ps1_export_job {
            use std::sync::mpsc::TryRecvError;
            match rx.try_recv() {
                Ok(Ok(result))=>{
                    if let Err(error)=ps1_export::verify_output_file(&result.path,None){
                        self.studio.status=None;self.studio.error=Some(format!("IMG completion verification failed: {error:#}"));self.ps1_export_job=None;return;
                    }
                    self.ps1_export_output=Some(result.path.clone());
                    self.ps1_export_imports=result.imports;
                    self.last_written=result.path.parent().map(Path::to_path_buf);
                    let details=if result.skipped.is_empty(){String::new()}else{format!(" Not updated: {}",result.skipped.join(" | "))};
                    self.studio.status=Some(format!("Native PS1 image saved: {}. Patched: {}.{} In DuckStation: boot THIS IMG (not an old CUE or save state), then reload Rim Elm. Native writes were read back. Original walls/floors and scripts are preserved unless an explicit supported plan changes them; gameplay verification remains required.",result.path.display(),result.patched.join(", "),details));
                    if result.skipped.iter().any(|s|s.starts_with("UNRESTRICTED EXPERIMENT")) {
                        self.studio.status=Some(result.experimental_summary());
                        self.studio.error=result.verification_warning();
                    }
                    self.ps1_export_job=None;
                }
                Ok(Err(error))=>{
                    self.studio.status=None;
                    self.studio.error=Some(format!("Native IMG export failed: {error:#}"));
                    self.ps1_export_job=None;
                }
                Err(TryRecvError::Disconnected)=>{
                    self.studio.status=None;
                    self.studio.error=Some("Native IMG export worker disconnected without a result".into());
                    self.ps1_export_job=None;
                }
                Err(TryRecvError::Empty)=>{}
            }
        }

        if self.studio.poll() {
            ctx.request_repaint();
        }
        // Index the disc exactly once, asynchronously. The currently opened
        // cell and its placement state do not affect the global asset catalog.
        if self.mode==Mode::Editor && !self.global_repository_started {
            if let Some(lib)=self.studio.library.as_ref() {
                let names=self.studio.blocks.iter().map(|b|b.name.clone())
                    .collect::<std::collections::BTreeSet<_>>().into_iter().collect::<Vec<_>>();
                if !names.is_empty() {
                    let index=Arc::clone(&lib.index);
                    let (tx,rx)=std::sync::mpsc::channel();
                    self.global_repository_rx=Some(rx);
                    self.global_repository_started=true;
                    self.global_repository_total=names.len();
                    std::thread::spawn(move ||global_repository::scan_disc(index,names,tx));
                    ctx.request_repaint();
                }
            }
        }
        if let Some(rx)=self.global_repository_rx.as_ref() {
            let mut done=false;
            for _ in 0..10 {
                match rx.try_recv() {
                    Ok(batch)=>{
                        self.global_repository_completed+=1;
                        if batch.failed {self.global_repository_failures+=1;}
                        self.global_repository.extend(batch.assets);
                        ctx.request_repaint();
                    },
                    Err(std::sync::mpsc::TryRecvError::Empty)=>break,
                    Err(std::sync::mpsc::TryRecvError::Disconnected)=>{
                        done=true;
                        break;
                    },
                }
            }
            if done {self.global_repository_rx=None;}
            else {ctx.request_repaint_after(std::time::Duration::from_millis(180));}
        }
        self.poll_native_scene_load(ctx);
        if self.studio.loading().is_some() || self.studio.is_analyzing() {
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
        }
        if let Some(lib) = &self.studio.library
            && self.studio.classified() < lib.entries.len()
        {
            ctx.request_repaint_after(std::time::Duration::from_millis(250));
        }
        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .filter_map(|f| f.path.clone())
                .collect()
        });
        if let Some(p) = dropped.first() {
            self.open_path(p);
        }
        self.run_automation(ctx);
        self.announce_extraction();
        if self.job.as_ref().is_some_and(|j| j.outcome().is_none()) {
            // Keep polling while a run goes on behind another view.
            ctx.request_repaint_after(std::time::Duration::from_millis(250));
        }
        self.poll_game(ctx);
        self.shortcuts(ctx);
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::F1)) {
            self.about_open = !self.about_open;
        }

        if let Some(r) = ctx.input(|i| i.viewport().inner_rect) {
            // egui points include the UI zoom; the viewport's inner size at
            // start does not, so store the unzoomed size or a zoomed window
            // would shrink on every start.
            let z = ctx.zoom_factor();
            self.window_size = Some([r.width() * z, r.height() * z]);
        }
        // The outer corner in physical pixels (`placement`): points times
        // pixels per point, which already folds in the UI zoom. Wayland
        // reports no position, so nothing is kept there.
        if let Some(r) = ctx.input(|i| i.viewport().outer_rect) {
            let ppp = ctx.pixels_per_point();
            self.window_pos = Some([
                (r.min.x * ppp).round() as i32,
                (r.min.y * ppp).round() as i32,
            ]);
        }
        egui::TopBottomPanel::top("top").show(ctx, |ui| self.top_bar(ui));
        self.about_window(ctx);
        match self.mode {
            // Nothing open yet: the start page takes the whole window, in
            // every view that needs a disc to show anything (a view restored
            // from the last session used to open on a bare "open a disc
            // first" line with no way to do it).
            Mode::Browse | Mode::Actors | Mode::Media | Mode::Editor if self.studio.library.is_none() => {
                egui::CentralPanel::default().show(ctx, |ui| self.start_page(ui));
            }
            Mode::Browse => {
                egui::SidePanel::left("browser")
                    .resizable(true)
                    .default_width(430.0)
                    .min_width(300.0)
                    .show(ctx, |ui| self.browser(ui));
                egui::CentralPanel::default().show(ctx, |ui| self.preview(ui));
            }
            Mode::Actors => self.actors_mode(ctx),
            Mode::Media => self.media_mode(ctx),
            Mode::Editor => self.editor_workspace(ctx),
            Mode::Extract => {
                egui::CentralPanel::default().show(ctx, |ui| self.extract_panel(ui));
            }
        }
        if self.mode == Mode::Editor && self.native_scene.is_some() {
            let seconds=ctx.input(|i|i.time);
            if seconds-self.editor_cell_last_autosave > 1.5 {
                self.editor_cell_last_autosave=seconds;
                self.save_active_editor_cell();
            }
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.save_active_editor_cell();
        self.player.stop();
        let s = &mut self.studio.settings;
        s.mode = Some(self.mode.key().to_string());
        s.tab = Some(self.tab.key().to_string());
        if let Some(size) = self.window_size {
            s.window_size = Some(size);
        }
        if let Some(pos) = self.window_pos {
            s.window_pos = Some(pos);
        }
        self.studio.save_settings();
        if let Some(job) = &mut self.job {
            job.cancel();
            job.join();
        }
    }
}

/// `THIRD-PARTY-LICENSES.txt` where the release archives put it
/// (`docs/tooling/releases.md`): beside the binary, or beside the macOS
/// `.app` bundle the binary sits three folders inside.
fn third_party_licenses() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    exe.ancestors()
        .skip(1)
        .take(4)
        .map(|d| d.join("THIRD-PARTY-LICENSES.txt"))
        .find(|p| p.is_file())
}

impl App {
    fn about_window(&mut self, ctx: &egui::Context) {
        let mut open = self.about_open;
        egui::Window::new("About Legaia Studio")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .show(ctx, |ui| {
                ui.heading(format!("Legaia Studio {}", env!("CARGO_PKG_VERSION")));
                ui.label(
                    "Browse, preview and extract the assets on your own Legend of Legaia disc.",
                );
                ui.add_space(6.0);
                ui.label(format!("License: {}", env!("CARGO_PKG_LICENSE")));
                ui.label(
                    "The third-party crates this program is built from keep their own \
                     licenses; their texts and notices ship in THIRD-PARTY-LICENSES.txt \
                     next to the program.",
                );
                match third_party_licenses() {
                    Some(p) => {
                        ui.horizontal(|ui| {
                            ui.monospace(p.display().to_string());
                            if ui.small_button("Copy path").clicked() {
                                ui.ctx().copy_text(p.display().to_string());
                            }
                        });
                    }
                    None => {
                        ui.label(
                            RichText::new(
                                "(not found beside this binary - it is part of every release \
                                 archive; a source build can generate it with \
                                 scripts/ci/third-party-licenses.py)",
                            )
                            .weak(),
                        );
                    }
                }
                ui.add_space(6.0);
                ui.label(
                    RichText::new(
                        "No game data ships with this program; it reads only the disc image you \
                         choose, and writes only into folders you pick.",
                    )
                    .weak(),
                );
                ui.add_space(6.0);
                ui.label(RichText::new("Keyboard").strong());
                egui::Grid::new("about_keys")
                    .num_columns(2)
                    .spacing([16.0, 3.0])
                    .show(ui, |ui| {
                        for (keys, what) in [
                            ("Ctrl+O", "open a disc image"),
                            ("Ctrl+1 .. Ctrl+4", "Browse, Characters, Movies, Extract"),
                            ("F5", "play the open disc"),
                            ("Ctrl+F, Esc", "search the archive, clear the search"),
                            (
                                "Up / Down, PgUp / PgDn, Home / End",
                                "walk the archive tree",
                            ),
                            (
                                "Ctrl+= / Ctrl+- / Ctrl+0",
                                "zoom in, out, reset (also View)",
                            ),
                            ("F1", "this box"),
                        ] {
                            ui.monospace(keys);
                            ui.label(what);
                            ui.end_row();
                        }
                    });
            });
        self.about_open = open;
    }
}

fn theme_preference(t: crate::settings::Theme) -> egui::ThemePreference {
    match t {
        crate::settings::Theme::System => egui::ThemePreference::System,
        crate::settings::Theme::Light => egui::ThemePreference::Light,
        crate::settings::Theme::Dark => egui::ThemePreference::Dark,
    }
}

impl App {
    /// The start page: what the app is for, the two ways in as large
    /// buttons, the recent list, and what each view does.
    fn start_page(&mut self, ui: &mut egui::Ui) {
        const COLUMN: f32 = 700.0;
        let accent = crate::theme::accent(ui);
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let side = ((ui.available_width() - COLUMN) / 2.0).max(12.0);
                ui.horizontal(|ui| {
                    ui.add_space(side);
                    ui.vertical(|ui| {
                        ui.set_max_width(COLUMN);
                        ui.add_space(28.0);
                        ui.label(RichText::new("Legaia Studio").size(30.0).color(accent));
                        ui.label(
                            RichText::new(
                                "Browse, preview and extract everything on your own \
                                 Legend of Legaia disc - and start the game on it.",
                            )
                            .size(15.0),
                        );
                        ui.add_space(18.0);
                        ui.horizontal(|ui| {
                            let big = egui::vec2(230.0, 38.0);
                            if ui
                                .add(
                                    crate::theme::primary_button("Open disc image...")
                                        .min_size(big),
                                )
                                .on_hover_text(
                                    "Your .bin (a raw Mode2/2352 dump) or its .cue. Ctrl+O",
                                )
                                .clicked()
                            {
                                self.pick_disc();
                            }
                            if ui
                                .add(egui::Button::new("Open extracted folder...").min_size(big))
                                .on_hover_text("A folder legaia-extract (or Extract) wrote")
                                .clicked()
                            {
                                self.pick_folder();
                            }
                        });
                        ui.label(
                            RichText::new(
                                "or drop the file (or folder) on this window, or type its \
                                 path in the bar above",
                            )
                            .weak(),
                        );
                        if let Some(p) = self.studio.loading() {
                            ui.add_space(10.0);
                            ui.horizontal(|ui| {
                                ui.spinner();
                                ui.label(format!("Opening {} ...", p.display()));
                            });
                        }
                        if !self.studio.settings.recent.is_empty() {
                            ui.add_space(20.0);
                            ui.label(RichText::new("Open again").strong().size(15.0));
                            egui::Frame::new()
                                .fill(ui.visuals().faint_bg_color)
                                .corner_radius(6.0)
                                .inner_margin(10.0)
                                .show(ui, |ui| {
                                    ui.set_width(ui.available_width());
                                    if let Some(p) = self.recent_list(ui) {
                                        self.open_path(&p);
                                    }
                                });
                        }
                        ui.add_space(20.0);
                        ui.label(RichText::new("What you can do").strong().size(15.0));
                        egui::Grid::new("start_features")
                            .num_columns(2)
                            .spacing([18.0, 8.0])
                            .show(ui, |ui| {
                                for (title, what) in [
                                    (
                                        "Browse",
                                        "Every entry of the game's archive by scene, with \
                                         textures, 3D models, sound, music and dialog to \
                                         preview and export.",
                                    ),
                                    (
                                        "Characters & monsters",
                                        "The party with any equipment and every monster, \
                                         each battle animation and Tactical Art.",
                                    ),
                                    (
                                        "Movies & voice",
                                        "The disc's movies with their soundtrack, and the \
                                         streamed voice and music.",
                                    ),
                                    (
                                        "Extract",
                                        "Unpack the whole disc into a folder of PNG, WAV, \
                                         OBJ and glTF files.",
                                    ),
                                    (
                                        "Run Rust port",
                                        "Launch the separate Rust engine on the USA disc. F5. Native PS1 export and DuckStation testing are separate workflows.",
                                    ),
                                ] {
                                    ui.label(RichText::new(title).strong().color(accent));
                                    ui.add(egui::Label::new(what).wrap());
                                    ui.end_row();
                                }
                            });
                        ui.add_space(20.0);
                        ui.label(
                            RichText::new(
                                "No game data ships with this program: it reads only the \
                                 disc image you choose, and writes only into folders you \
                                 pick. F1 lists the keyboard shortcuts.",
                            )
                            .weak(),
                        );
                    });
                });
            });
    }
}

/// The texture source a model view opens on: the entry's own TIMs when they
/// cover every textured prim of the mesh (`Model::texture_coverage`), else
/// the scene's VRAM, where the missing pages and CLUTs are uploaded.
fn default_tex_source(model: Option<&Model>, entry_vram: &legaia_tim::Vram) -> TexSource {
    match model.map(|m| m.texture_coverage(entry_vram)) {
        Some((covered, textured)) if covered < textured => TexSource::Scene,
        _ => TexSource::Entry,
    }
}

fn human_size(n: u64) -> String {
    if n >= 1 << 20 {
        format!("{:.1} MB", n as f64 / (1 << 20) as f64)
    } else if n >= 1 << 10 {
        format!("{:.1} KB", n as f64 / 1024.0)
    } else {
        format!("{n} B")
    }
}

fn checkerboard(p: &egui::Painter, rect: egui::Rect) {
    p.rect_filled(rect, 0.0, Color32::from_gray(60));
    let s = 8.0;
    let cols = (rect.width() / s).ceil() as i32;
    let rows = (rect.height() / s).ceil() as i32;
    if cols * rows > 40_000 {
        return;
    }
    for y in 0..rows {
        for x in 0..cols {
            if (x + y) % 2 == 0 {
                let min = rect.min + egui::vec2(x as f32 * s, y as f32 * s);
                let r = egui::Rect::from_min_size(min, egui::vec2(s, s)).intersect(rect);
                p.rect_filled(r, 0.0, Color32::from_gray(80));
            }
        }
    }
}

fn hex_view(ui: &mut egui::Ui, bytes: &[u8]) {
    let rows = bytes.len().div_ceil(16);
    let h = ui.text_style_height(&egui::TextStyle::Monospace);
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show_rows(ui, h, rows, |ui, range| {
            for r in range {
                let off = r * 16;
                let line = &bytes[off..(off + 16).min(bytes.len())];
                let mut s = format!("{off:08X}  ");
                for (i, b) in line.iter().enumerate() {
                    s.push_str(&format!("{b:02X} "));
                    if i == 7 {
                        s.push(' ');
                    }
                }
                for i in line.len()..16 {
                    s.push_str("   ");
                    if i == 7 {
                        s.push(' ');
                    }
                }
                s.push(' ');
                s.extend(line.iter().map(|&b| {
                    if (0x20..0x7F).contains(&b) {
                        b as char
                    } else {
                        '.'
                    }
                }));
                ui.monospace(s);
            }
        });
}

#[cfg(test)]
mod catalog_ui_tests {
    use super::*;
    fn app() -> App {
        let mut app = App::new(Studio::new(None), None, Automation::default());
        app.editor_names = Catalog::default();
        app.editor_catalog_error = None;
        app
    }
    fn frame(app: &mut App, ctx: &egui::Context, events: Vec<egui::Event>) {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 820.0))),
            events,
            ..Default::default()
        };
        let result = ctx.run(input, |ctx| app.editor_workspace(ctx));
        assert!(!result.shapes.is_empty());
    }
    #[test]
    fn universal_name_precedence_search_and_ctrl_d_keep_instance_overrides() {
        let mut app=app();let (scene,_)=ps1_export::tests::actor_fixture();
        let mut world=collision_recipes::properties_fixture();
        let alias=naming::visual_key(&[1,2,3],&collision_recipes::digest(&[4,5])).unwrap();
        world.naming.assets.insert((false,0),alias.clone());world.naming.native.insert((false,0),"stable-original".into());
        app.editor_names.records.insert(alias.clone(),Record{name:"Lantern".into(),aliases:"lamp".into(),..Default::default()});
        app.native_scene=Some(("synthetic".into(),NativeSceneSlot::Ready(Arc::new(scene),Arc::new(world))));
        let scene=match &app.native_scene {Some((_,NativeSceneSlot::Ready(s,_)))=>s.clone(),_=>unreachable!()};
        assert_eq!(app.editor_name_for(Some(&scene),EditorCatalogItem::Draw(false,0)),"Lantern");
        assert_eq!(app.editor_name_for(Some(&scene),EditorCatalogItem::Draw(false,1)),"Lantern");
        assert!(app.editor_item_matches(Some(&scene),EditorCatalogItem::Draw(false,1),"lamp"));
        if let Some((_,NativeSceneSlot::Ready(_,world)))=&mut app.native_scene {
            let world=Arc::get_mut(world).unwrap();world.set_display_name((false,0),Some("Original lamp".into()));world.set_display_name((false,1),Some("Side entrance lamp".into()));
        }
        app.native_selected=Some((false,1));let ctx=egui::Context::default();frame(&mut app,&ctx,vec![]);
        let camera=app.native_view.cam;
        frame(&mut app,&ctx,vec![egui::Event::Key{key:egui::Key::D,physical_key:None,pressed:true,repeat:false,modifiers:egui::Modifiers{ctrl:true,command:true,..Default::default()}}]);
        let copied=app.native_selected.unwrap();assert_ne!(copied,(false,1));
        assert_eq!(app.editor_name_for(Some(&scene),EditorCatalogItem::Draw(copied.0,copied.1)),"Side entrance lamp");
        assert_eq!(app.editor_name_for(Some(&scene),EditorCatalogItem::Draw(false,0)),"Original lamp");
        assert_eq!(app.native_view.cam,camera);
        app.editor_names.records.get_mut(&alias).unwrap().name="New global alias".into();
        assert_eq!(app.editor_name_for(Some(&scene),EditorCatalogItem::Draw(false,1)),"Side entrance lamp");
        if let Some((_,NativeSceneSlot::Ready(_,world)))=&mut app.native_scene {Arc::get_mut(world).unwrap().set_display_name((false,1),None);}
        assert_eq!(app.editor_name_for(Some(&scene),EditorCatalogItem::Draw(false,1)),"New global alias");
    }
    #[test]
    fn right_click_properties_shows_collision_without_library_and_preview_does_not_edit() {
        fn text_shapes(shape:&egui::Shape,out:&mut Vec<(String,egui::Pos2)>) {
            match shape {
                egui::Shape::Text(t)=>out.push((t.galley.job.text.clone(),t.pos)),
                egui::Shape::Vec(shapes)=>for shape in shapes {text_shapes(shape,out)},_=>{}
            }
        }
        let mut app=app();let (scene,_)=ps1_export::tests::actor_fixture();
        let world=collision_recipes::properties_fixture();let key=(false,1);
        let overlay=scene_instances::serialize("test",&world);
        app.native_scene=Some(("test".into(),NativeSceneSlot::Ready(Arc::new(scene),Arc::new(world))));
        app.native_selected=Some(key);app.editor_properties_target=Some(key);
        let camera=app.native_view.cam;let ctx=egui::Context::default();
        let render=|app:&mut App|ctx.run(egui::RawInput {
            screen_rect:Some(egui::Rect::from_min_size(egui::Pos2::ZERO,egui::vec2(1280.0,820.0))),..Default::default()
        },|ctx|app.editor_workspace(ctx));
        let _=render(&mut app);let output=render(&mut app);
        let mut texts=Vec::new();for shape in &output.shapes {text_shapes(&shape.shape,&mut texts);}
        assert!(texts.iter().any(|(text,_)|text.contains("Collision Properties (P1)")));
        assert!(texts.iter().any(|(text,_)|text.contains("Saved generated collision:")));
        assert!(texts.iter().any(|(text,_)|text.contains("Open the source library")));
        let pos=texts.iter().find(|(text,_)|text.contains("Show this object's generated collision wireframe")).unwrap().1+egui::vec2(4.0,4.0);
        frame(&mut app,&ctx,vec![egui::Event::PointerMoved(pos),egui::Event::PointerButton {pos,button:egui::PointerButton::Primary,pressed:true,modifiers:Default::default()}]);
        frame(&mut app,&ctx,vec![egui::Event::PointerButton {pos,button:egui::PointerButton::Primary,pressed:false,modifiers:Default::default()}]);
        assert!(app.object_collision_draft_visible);
        assert!(matches!(&app.generated_collision_target,Some((_,scene_instances::NameTarget::Created(90)))));
        assert!(!app.native_collision_wire_visible);
        assert_eq!(app.native_selected,Some(key));assert_eq!(app.native_view.cam,camera);
        assert!(app.editor_undo.is_empty()&&app.editor_redo.is_empty());
        let Some((_,NativeSceneSlot::Ready(_,world)))=&app.native_scene else {panic!("Scene disappeared")};
        assert_eq!(scene_instances::serialize("test",world),overlay);
    }
    #[test]
    fn failed_scene_switch_preserves_document_selection_camera_and_history() {
        let mut app=app();
        let (scene,world)=ps1_export::tests::actor_fixture();
        let world=Arc::new(world);
        app.native_scene=Some(("synthetic".into(),NativeSceneSlot::Ready(Arc::new(scene),world.clone())));
        app.native_selected=Some((false,world_view::NPC_KEY_BASE+1));
        app.editor_undo.push(EditorSceneHistory{overlay:"undo".into(),selected:None,selection:vec![]});
        app.editor_redo.push(EditorSceneHistory{overlay:"redo".into(),selected:None,selection:vec![]});
        let version=app.native_view.version;
        let camera=app.native_view.cam;
        let (tx,rx)=std::sync::mpsc::channel();
        tx.send(Err("synthetic load failure".into())).unwrap();
        app.pending_native_scene=Some(("missing".into(),CellSelection::Copy(999),NativeSceneSlot::Loading(rx)));
        app.poll_native_scene_load(&egui::Context::default());
        let Some((name,NativeSceneSlot::Ready(_,current)))=&app.native_scene else {panic!("lost document")};
        assert_eq!(name,"synthetic");
        assert!(Arc::ptr_eq(current,&world));
        assert_eq!(app.native_selected,Some((false,world_view::NPC_KEY_BASE+1)));
        assert_eq!(app.editor_undo[0].overlay,"undo");
        assert_eq!(app.editor_redo[0].overlay,"redo");
        assert_eq!(app.native_view.version,version);
        assert_eq!(format!("{camera:?}"),format!("{:?}",app.native_view.cam));
        let (tx,rx)=std::sync::mpsc::channel();
        tx.send(Ok(ps1_export::tests::actor_fixture())).unwrap();
        app.pending_native_scene=Some(("synthetic".into(),CellSelection::Copy(999),NativeSceneSlot::Loading(rx)));
        app.poll_native_scene_load(&egui::Context::default());
        let Some((_,NativeSceneSlot::Ready(_,current)))=&app.native_scene else {panic!("lost document on overlay failure")};
        assert!(Arc::ptr_eq(current,&world));
        assert_eq!(app.editor_undo[0].overlay,"undo");
        assert_eq!(app.editor_redo[0].overlay,"redo");
        assert!(app.pending_native_scene.is_none());
    }

    #[test]
    fn history_selection_uses_stable_created_uid_not_transient_index() {
        let source = scene_instances::NameTarget::Created(7);
        let native = scene_instances::NameTarget::Native(false,9);
        assert_ne!(source,native);
        let restored = EditorSceneHistory {overlay:"LCK_SCENE_V1\ttown\n".into(),
            selected:Some(source.clone()),selection:vec![source.clone()]};
        assert_eq!(restored.selected,Some(source));
    }
    #[test]
    fn right_drag_rotates_y_and_alt_right_drag_rotates_x() {
        let normal=editor_rotate_drag([0.0;3],egui::vec2(100.0,20.0),false,false);
        assert!((normal[1]-1.0).abs()<0.00001);
        assert_eq!(normal[0],0.0);
        let alt=editor_rotate_drag([0.0;3],egui::vec2(100.0,20.0),true,false);
        assert!((alt[0]-0.2).abs()<0.00001);
        assert_eq!(alt[1],0.0);
        assert!((editor_rotate_drag([0.0;3],egui::vec2(100.0,0.0),false,true)[1]+1.0).abs()<0.00001);
    }
    #[test]
    fn duplicate_button_and_ctrl_d_share_native_source_and_copy_relative_offset() {
        use super::world_view::{DrawInfo, WorldModel};
        let original = DrawInfo {
            id: 0, terrain: false, index: 2, center: [0.0; 3], radius: 1.0,
            original_center: [0.0; 3], offset: [12.0, 3.0, -8.0], rotation:[0.0;3], scale:[1.0;3],
            vertex_start: 0, vertex_end: 3, source_ref: None, editor_uid: None,
            display_name: None, hidden: false, hidden_triangles: Vec::new(),
        };
        let copy = DrawInfo {
            id: 1, terrain: false, index: 3, center: [0.0; 3], radius: 1.0,
            original_center: [0.0; 3], offset: [140.0, 3.0, -8.0], rotation:[0.0;3], scale:[1.0;3],
            vertex_start: 3, vertex_end: 6, source_ref: Some((false, 2)), editor_uid: Some(1),
            display_name: None, hidden: false, hidden_triangles: Vec::new(),
        };
        let mut world = WorldModel {
            model: None, instances: 2, unavailable: 0, unique_models: 1,
            ground_triangles: 0, ground_vram_covered: 0, ground_textured: 0,
            npcs:Vec::new(),npc_issue:None,foreign:Vec::new(),naming:Default::default(),collision:Default::default(),foreign_vram:std::collections::HashMap::new(),
            draws: vec![original, copy],
        };
        assert_eq!(editor_duplicate_request(&world, Some((false, 2))),
            Some(((false, 2), [140.0, 3.0, -8.0], [0.0;3], None)));
        assert_eq!(editor_duplicate_request(&world, Some((false, 3))),
            Some(((false, 2), [268.0, 3.0, -8.0], [0.0;3], None)));
        assert_eq!(editor_duplicate_request(&world, None), None);
        assert_eq!(editor_duplicate_request(&world, Some((false, 9))), None);
        // Unrendered native actor records have no source geometry and must
        // remain unavailable for duplication until a drawable TMD is loaded.
        world.draws[0].vertex_end = world.draws[0].vertex_start;
        assert_eq!(editor_duplicate_request(&world, Some((false, 2))), None);
    }
    #[test]
    fn metadata_search_and_inventory_do_not_promote_user_names() {
        let mut app = app();
        app.editor_names.records.insert("actor:character:0".into(), Record {
            name: "Hero".into(), aliases: "protagonist".into(), category: "Party".into(),
            ..Record::default()
        });
        assert!(app.editor_item_matches(None, EditorCatalogItem::Character(0), "protagonist"));
        assert!(app.editor_item_matches(None, EditorCatalogItem::Character(0), "party"));
        assert!(app.editor_item_matches(None, EditorCatalogItem::Character(0), "actor:character:0"));
        assert!(!app.editor_item_matches(None, EditorCatalogItem::Character(0), "door"));
        let report = app.editor_inventory(None);
        assert!(report.contains("actor:character:0\tHero\tUser assigned\tParty\tprotagonist"));
        assert!(report.contains("Not catalog-identified"));
        assert!(!report.contains("\tVerified\t"));
        assert_eq!(report.lines().count(), 5);
    }
    #[test]
    fn editor_panels_and_identification_dialog_render_headlessly() {
        let mut app = app();
        let ctx = egui::Context::default();
        frame(&mut app, &ctx, vec![]);
        assert!(app.editor_properties_target.is_none());
        assert!(app.editor_context_menu.is_none());
        frame(&mut app, &ctx, vec![]);
        app.editor_modal = Some((EditorCatalogItem::Character(0), false));
        frame(&mut app, &ctx, vec![]);
        assert!(app.editor_modal.is_some());
        assert_eq!(app.editor_name_edit_key.as_deref(), Some("actor:character:0"));
    }
    #[test]
    fn viewport_drag_moves_three_members_as_one_workspace_undo_redo() {
        let mut app=app();let ctx=egui::Context::default();let (scene,_)=ps1_export::tests::actor_fixture();
        let mut world=collision_recipes::properties_fixture();world.duplicate_native((false,0),1,91,[256.0,0.0,0.0]).unwrap();
        let before=scene_instances::serialize("synthetic",&world);
        let offsets=world.draws.iter().map(|d|d.offset).collect::<Vec<_>>();
        let index=legaia_engine_scene::scene::ProtIndex::from_bytes(legaia_disc_patch::disc::synth::synth_prot(&[]),None).unwrap();
        app.studio.library=Some(Arc::new(crate::Library{source:Source::Disc("unused-synthetic.img".into()),entries:vec![],cdname:None,iso_files:vec![],region:None,exe:None,index:Arc::new(index)}));
        app.native_scene=Some(("synthetic".into(),NativeSceneSlot::Ready(Arc::new(scene),Arc::new(world))));
        app.native_selected=Some((false,0));app.native_selection.extend([(false,0),(false,1),(false,2)]);
        let run=|app:&mut App,events|ctx.run(egui::RawInput{screen_rect:Some(egui::Rect::from_min_size(egui::Pos2::ZERO,egui::vec2(1280.0,820.0))),events,..Default::default()},|ctx|app.editor_workspace(ctx));
        let output=run(&mut app,vec![]);
        let rect=output.shapes.iter().filter_map(|s|match &s.shape {
            egui::Shape::Mesh(m) if m.texture_id!=egui::TextureId::Managed(0)=>Some(m.calc_bounds()),_=>None,
        }).max_by(|a,b|a.area().total_cmp(&b.area())).expect("World viewport image missing");
        let camera=app.native_view.cam;let px=[(rect.width() as usize).clamp(16,900),(rect.height() as usize).clamp(16,680)];
        let point=match &app.native_scene {Some((_,NativeSceneSlot::Ready(_,world)))=>{
            let model=world.model.as_ref().unwrap();
            (0..px[1]).step_by(3).find_map(|y|(0..px[0]).step_by(3).find_map(|x|
                (crate::raster::pick_instance(model,camera,px[0],px[1],[x as f32,y as f32])==Some(2)).then_some([x,y]))).expect("Third fixture member missing")
        },_=>unreachable!()};
        let x=if app.native_view.world_mirrored {px[0]-1-point[0]}else{point[0]};
        let pos=rect.min+egui::vec2(x as f32/px[0] as f32*rect.width(),point[1] as f32/px[1] as f32*rect.height());
        run(&mut app,vec![egui::Event::PointerMoved(pos),egui::Event::PointerButton{pos,button:egui::PointerButton::Primary,pressed:true,modifiers:Default::default()}]);
        assert_eq!(app.native_selected,Some((false,2)));assert_eq!(app.native_selection.len(),3);
        let end=pos+egui::vec2(34.0,19.0);
        run(&mut app,vec![egui::Event::PointerMoved(end)]);
        run(&mut app,vec![egui::Event::PointerButton{pos:end,button:egui::PointerButton::Primary,pressed:false,modifiers:Default::default()}]);
        assert_eq!(app.editor_undo.len(),1);assert_eq!(app.native_view.cam,camera);
        let after=match &app.native_scene {Some((_,NativeSceneSlot::Ready(_,world)))=>{
            let delta=[0,1,2].map(|a|world.draws[0].offset[a]-offsets[0][a]);assert_ne!(delta,[0.0;3]);
            for (d,start) in world.draws.iter().zip(&offsets) {assert_eq!([0,1,2].map(|a|d.offset[a]-start[a]),delta);}
            scene_instances::serialize("synthetic",world)
        },_=>unreachable!()};
        for (key,expected) in [(egui::Key::Z,&before),(egui::Key::Y,&after)] {
            run(&mut app,vec![egui::Event::Key{key,physical_key:None,pressed:true,repeat:false,modifiers:egui::Modifiers{ctrl:true,command:true,..Default::default()}}]);
            match &app.native_scene {Some((_,NativeSceneSlot::Ready(_,world)))=>assert_eq!(&scene_instances::serialize("synthetic",world),expected),_=>unreachable!()};
        }
        assert_eq!(app.editor_undo.len(),1);assert_eq!(app.native_view.cam,camera);
    }
    #[test]
    fn batch_delete_keyboard_is_one_complete_undo_redo_in_the_workspace() {
        let mut app=app();let ctx=egui::Context::default();
        let (scene,_)=ps1_export::tests::actor_fixture();let world=collision_recipes::properties_fixture();
        let before=scene_instances::serialize("synthetic",&world);
        let index=legaia_engine_scene::scene::ProtIndex::from_bytes(legaia_disc_patch::disc::synth::synth_prot(&[]),None).unwrap();
        app.studio.library=Some(Arc::new(crate::Library{source:Source::Disc("unused-synthetic.img".into()),entries:vec![],cdname:None,iso_files:vec![],region:None,exe:None,index:Arc::new(index)}));
        app.native_scene=Some(("synthetic".into(),NativeSceneSlot::Ready(Arc::new(scene),Arc::new(world))));
        app.native_selected=Some((false,1));app.native_selection.extend([(false,0),(false,1)]);
        let camera=app.native_view.cam;
        let key=|key,command|egui::Event::Key{key,physical_key:None,pressed:true,repeat:false,modifiers:egui::Modifiers{ctrl:command,command,..Default::default()}};
        frame(&mut app,&ctx,vec![]);
        frame(&mut app,&ctx,vec![key(egui::Key::Delete,false)]);
        assert_eq!(app.editor_undo.len(),1);assert_eq!(app.native_selected,None);
        let saved=match &app.native_scene {Some((_,NativeSceneSlot::Ready(_,w)))=>scene_instances::serialize("synthetic",w),_=>unreachable!()};
        assert_ne!(saved,before);
        frame(&mut app,&ctx,vec![key(egui::Key::Z,true)]);
        assert!(app.studio.error.is_none(),"{:?}",app.studio.error);
        assert!(app.editor_undo.is_empty());assert_eq!(app.editor_redo.len(),1);
        assert_eq!(app.native_selection.len(),2);
        match &app.native_scene {Some((_,NativeSceneSlot::Ready(_,w)))=>assert_eq!(scene_instances::serialize("synthetic",w),before),_=>unreachable!()};
        frame(&mut app,&ctx,vec![key(egui::Key::Y,true)]);
        assert_eq!(app.editor_undo.len(),1);assert!(app.editor_redo.is_empty());
        match &app.native_scene {Some((_,NativeSceneSlot::Ready(_,w)))=>assert_eq!(scene_instances::serialize("synthetic",w),saved),_=>unreachable!()};
        assert_eq!(app.native_view.cam,camera);
    }
    #[test]
    fn escape_closes_modal_and_clears_entire_selection_without_moving_camera() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.native_selected = Some((false, 7));
        app.editor_modal = Some((EditorCatalogItem::Character(0), false));
        let camera = app.native_view.cam;
        let escape = || egui::Event::Key { key: egui::Key::Escape, physical_key: None,
            pressed: true, repeat: false, modifiers: egui::Modifiers::default() };
        frame(&mut app, &ctx, vec![escape()]);
        assert!(app.editor_modal.is_none());
        assert_eq!(app.native_selected, None);
        assert_eq!(app.native_view.cam, camera);
        frame(&mut app, &ctx, vec![egui::Event::Key { key: egui::Key::Escape, physical_key: None,
            pressed: false, repeat: false, modifiers: egui::Modifiers::default() }]);
        frame(&mut app, &ctx, vec![escape()]);
        assert_eq!(app.native_selected, None);
        assert_eq!(app.native_view.cam, camera);
    }
}


#[cfg(test)]
mod source_asset_upright_preview_tests {
    use super::*;

    #[test]
    fn ps1_coordinate_conversion_keeps_geometry_and_rest_consistent() {
        let mut mesh = Model {
            positions: vec![[1.0, -25.0, 3.0], [5.0, -55.0, 7.0], [5.0, -25.0, 7.0]],
            rest: vec![], object_ids: vec![0; 3],
            uvs: vec![[0, 0]; 3], cba_tsb: vec![[0, 0]; 3],
            modulation: vec![[128; 3]; 3], fill: vec![[180; 3]; 3],
            textured: vec![0; 3], indices: vec![0, 1, 2],
            center: [0.0; 3], radius: 1.0,
        };
        let original = mesh.positions.clone();
        let points = original.iter().map(|p| [p[0], -p[1], p[2]]).collect();
        assert!(editor_preview_positions(&mut mesh, points).is_some());
        assert_eq!(mesh.positions, vec![[1.0,25.0,3.0], [5.0,55.0,7.0], [5.0,25.0,7.0]]);
        assert_eq!(mesh.rest, mesh.positions);
        assert_eq!(mesh.indices, vec![0, 1, 2]);
        assert!(mesh.center.iter().all(|v| v.is_finite()));
        assert!(mesh.radius > 0.0);
        assert_eq!(original[0][1], -25.0, "native original must not be mutated");
    }

    #[test]
    fn preview_rejects_incomplete_or_invalid_geometry() {
        let mut mesh = Model {
            positions: vec![[1.0, 2.0, 3.0]], rest: vec![[1.0, 2.0, 3.0]],
            object_ids: vec![0], uvs: vec![[0,0]], cba_tsb: vec![[0,0]],
            modulation: vec![[128;3]], fill: vec![[128;3]], textured: vec![0],
            indices: vec![0,0,0], center: [0.0;3], radius: 1.0,
        };
        let before = mesh.positions.clone();
        assert!(editor_preview_positions(&mut mesh, vec![[0.0,0.0,0.0];2]).is_none());
        assert!(editor_preview_positions(&mut mesh, vec![[0.0,f32::NAN,0.0]]).is_none());
        assert_eq!(mesh.positions, before);
    }
}
