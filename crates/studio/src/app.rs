//! The egui front end over [`Studio`].

mod actors_ui;
mod media_ui;
mod viewport;
mod world_view;

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

enum EditorPreview {
    Mesh(Model),
    Actor(crate::actors::Actor),
    Unavailable(String),
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

/// One indexed subgroup: the search filters locally and only expanded groups
/// draw their rows; opening a context menu never changes the camera.
fn editor_catalog_group(ui: &mut egui::Ui, title: &str,
                        rows: &[(String, EditorCatalogItem)], filter: &str,
                        selected: Option<(bool, usize)>,
                        chosen: &mut Option<(EditorCatalogItem, bool)>,
                        clicked: &mut Option<EditorCatalogItem>) {
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
                        selected == Some((*terrain, *index)),
                    _ => false,
                };
                let response = ui.selectable_label(active, label);
                if response.clicked() { *clicked = Some(*item); }
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

/// Perspective-aware screen drag, projected onto the horizontal XZ plane.
/// Ctrl+Shift instead moves on the vertical axis. Flip the view's horizontal
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
        if d.offset != [0.0; 3] {
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
    native_selected: Option<(bool, usize)>, // false = placement; true = terrain
    native_view: Viewport,
    native_ground_visible: bool,
    native_move_mode: EditorMoveMode,
    native_grid_size: f32,
    native_anchor_snap: bool,
    native_drag: Option<EditorDrag>,
    native_undo: Vec<EditorTransformEdit>,
    native_redo: Vec<EditorTransformEdit>,
    editor_search: String,
    editor_modal: Option<(EditorCatalogItem, bool)>, // false: properties; true: preview
    editor_preview: Option<(EditorCatalogItem, EditorPreview)>,
    editor_preview_view: Viewport,
    editor_monsters: Option<Vec<crate::actors::MonsterRow>>,
    editor_archives: Option<Vec<(String, Vec<(String, EditorCatalogItem)>)>>,
    editor_names: Catalog,
    editor_catalog_error: Option<String>,
    editor_hierarchy_filter: String,
    editor_inspector_open: bool,
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
        let mut app = Self {
            studio,
            mode,
            tab,
            native_scene: None,
            native_selected: None,
            native_view: Viewport::new(Shading::Lit),
            native_ground_visible: true,
            native_move_mode: EditorMoveMode::Grid,
            native_grid_size: 32.0,
            native_anchor_snap: false,
            native_drag: None,
            native_undo: Vec::new(),
            native_redo: Vec::new(),
            editor_search: String::new(),
            editor_modal: None,
            editor_preview: None,
            editor_preview_view: Viewport::new(Shading::Textured),
            editor_monsters: None,
            editor_archives: None,
            editor_names,
            editor_catalog_error,
            editor_hierarchy_filter: String::new(),
            editor_inspector_open: true,
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
        self.native_scene = None;
        self.native_selected = None;
        self.native_drag = None;
        self.native_undo.clear();
        self.native_redo.clear();
        self.editor_modal = None;
        self.editor_preview = None;
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
    fn load_native_scene(&mut self, scene_name: String) {
        let Some(lib) = self.studio.library.as_ref() else { return };
        let index = Arc::clone(&lib.index);
        let requested_name = scene_name.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let result = legaia_engine_world_views::scene_staging::assemble_field_scene(
                index.as_ref(), &requested_name,
            ).map(|scene| {
                let world = world_view::build_world(&scene);
                (scene, world)
            });
            let _ = tx.send(result);
        });
        self.native_scene = Some((scene_name, NativeSceneSlot::Loading(rx)));
        self.native_selected = None;
        self.native_drag = None;
        self.native_undo.clear();
        self.native_redo.clear();
        self.editor_modal = None;
        self.editor_preview = None;
        self.native_view.cam = crate::raster::Camera::default();
        self.native_view.version = self.native_view.version.wrapping_add(1);
    }



    fn editor_name_for(&self,
            scene: Option<&legaia_engine_scene::scene_assembly::AssembledScene>,
            item: EditorCatalogItem) -> String {
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

    fn editor_item_matches(&self,
        scene: Option<&legaia_engine_scene::scene_assembly::AssembledScene>,
        item: EditorCatalogItem, term: &str) -> bool {
        if term.is_empty() { return true; }
        let key = editor_item_name_key(scene, item).unwrap_or_default();
        let mut text = format!("{} {}", self.editor_name_for(scene, item), self.editor_names.search_text(&key));
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
        let Some(key) = editor_item_name_key(scene, item) else { return; };
        if self.editor_name_edit_key.as_deref() != Some(&key) {
            self.editor_name_edit_key = Some(key.clone());
            self.editor_name_draft = self.editor_names.record(&key);
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
        let mesh_key = match (scene, item) {
            (Some(s), EditorCatalogItem::Draw(terrain, index)) => {
                let draws = if terrain { &s.terrain } else { &s.placements };
                draws.get(index).and_then(|draw| editor_mesh_name_key(s, draw.env_slot))
            }
            _ => None,
        };
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
            let direct = self.editor_names.records.get(&key);
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

    /// Object Window, redesigned as a scrollable indexed asset list.
    /// Only the scene's native mesh slots and placed instances are actual
    /// environment renderables; actor/archive entries are source references.
    fn editor_catalog(&mut self, ui: &mut egui::Ui,
                      scene: Option<&legaia_engine_scene::scene_assembly::AssembledScene>) {
        ui.heading("All Objects");
        ui.add(egui::TextEdit::singleline(&mut self.editor_search)
            .hint_text("Search objects, characters, monsters..."));
        ui.small("Right-click for identification or preview. Esc closes preview.");
        if let Some(error) = &self.editor_catalog_error {
            ui.colored_label(Color32::LIGHT_RED, format!("Catalog: {error}"));
        }
        ui.small(format!("{} saved name records", self.editor_names.records.len()));
        ui.separator();
        if self.editor_monsters.is_none() {
            if let Some(lib) = self.studio.library.as_ref() {
                self.editor_monsters = Some(crate::actors::monster_rows(lib).unwrap_or_default());
            }
        }
        if self.editor_archives.is_none() {
            if let Some(lib) = self.studio.library.as_ref() {
                let mut by_type = std::collections::BTreeMap::<String, Vec<(String,EditorCatalogItem)>>::new();
                for pos in 0..lib.entries.len() {
                    let e = &lib.entries[pos];
                    let kind = self.studio.class_of(pos)
                        .map(|c| c.name().to_string())
                        .unwrap_or_else(|| "Unknown archive data".into());
                    let place = lib.retail_block(e.index).unwrap_or("unidentified block");
                    let display_place = crate::place_name(place).unwrap_or(place);
                    by_type.entry(format!("Source / {kind}"))
                        .or_default().push((format!("PROT {:04}  {display_place} ({place})  {kind}",e.index),
                            EditorCatalogItem::Archive(pos)));
                }
                self.editor_archives = Some(by_type.into_iter().collect());
            }
        }
        let mut groups: Vec<(String, Vec<(String,EditorCatalogItem)>)> = Vec::new();
        if let Some(s) = scene {
            groups.push(("Placed objects".into(), s.placements.iter().enumerate().map(|(i,d)| {
                let item = EditorCatalogItem::Draw(false,i);
                (format!("{}  [#{i:03}]  ({} / {})",
                    self.editor_name_for(Some(s), item), d.cell.0, d.cell.1), item)
            }).collect()));
            groups.push(("Terrain / panels".into(), s.terrain.iter().enumerate().map(|(i,_)| {
                let item = EditorCatalogItem::Draw(true,i);
                (format!("{}  [#{i:03}]", self.editor_name_for(Some(s), item)), item)
            }).collect()));
            groups.push(("Reusable cell meshes".into(), s.env_tmds.iter().enumerate().map(|(slot,_)| {
                let item = EditorCatalogItem::Mesh(slot);
                (format!("{}  [#{slot:03}]", self.editor_name_for(Some(s), item)), item)
            }).collect()));
        }
        groups.push(("Characters".into(), ["Vahn","Noa","Gala","Terra"]
            .into_iter().enumerate().map(|(i,_)| {
                let item = EditorCatalogItem::Character(i);
                (self.editor_name_for(scene, item), item)
            }).collect()));
        if let Some(monsters) = &self.editor_monsters {
            groups.push(("Monsters".into(), monsters.iter().map(|m| {
                let item = EditorCatalogItem::Monster(m.id);
                (format!("{}  [#{:03}]",self.editor_name_for(scene,item),m.id), item)
            }).collect()));
        }
        let term = self.editor_search.trim().to_lowercase();
        for (title, rows) in &mut groups {
            if !title.to_lowercase().contains(&term) {
                rows.retain(|(label, item)| label.to_lowercase().contains(&term)
                    || self.editor_item_matches(scene, *item, &term));
            }
        }
        if ui.button("Export identification inventory...")
            .on_hover_text("Export this cell, loaded actors and archive references. Includes unidentified entries and evidence; does not modify the disc.").clicked() {
            if let Some(path) = rfd::FileDialog::new().set_file_name("LCK_IdentificationInventory.tsv")
                .add_filter("Tab-separated inventory", &["tsv"]).save_file() {
                match std::fs::write(&path, self.editor_inventory(scene)) {
                    Ok(()) => self.studio.status = Some(format!("Identification inventory saved: {}", path.display())),
                    Err(e) => self.studio.error = Some(format!("Cannot export identification inventory: {e}")),
                }
            }
        }
        let mut chosen = None;
        let mut clicked = None;
        egui::ScrollArea::vertical().id_salt("editor_catalog_scroll")
            .auto_shrink([false,false]).show(ui, |ui| {
                for (title, rows) in &groups {
                    editor_catalog_group(ui, title, rows, "",
                        self.native_selected, &mut chosen, &mut clicked);
                }
                if let Some(archives) = &self.editor_archives {
                    egui::CollapsingHeader::new("Original archive resources (not directly placeable)")
                        .id_salt("editor_source_resources").show(ui, |ui| {
                            for (title, rows) in archives {
                                let display_rows: Vec<(String, EditorCatalogItem)> = rows.iter()
                                    .map(|(technical, item)| {
                                        let nickname = editor_item_name_key(scene, *item)
                                            .and_then(|key| self.editor_names.get(&key));
                                        (nickname.map(|name| format!("{}  ({technical})",name))
                                            .unwrap_or_else(|| technical.clone()), *item)
                                    }).filter(|(label, item)| title.to_lowercase().contains(&term)
                                        || label.to_lowercase().contains(&term)
                                        || self.editor_item_matches(scene, *item, &term)).collect();
                                editor_catalog_group(ui, title, &display_rows, "",
                                    self.native_selected, &mut chosen, &mut clicked);
                            }
                        });
                }
                ui.separator();
                egui::CollapsingHeader::new(format!("Cells / scenes ({})",self.studio.blocks.len()))
                    .id_salt("editor_scene_picker").show(ui, |ui| {
                        let mut choose_cell = None;
                        for b in &self.studio.blocks {
                            let label = format!("{} ({})",
                                crate::place_name(&b.name).unwrap_or(&b.name), b.name);
                            if !self.editor_search.is_empty() &&
                                !label.to_lowercase().contains(&self.editor_search.to_lowercase()) {
                                continue;
                            }
                            if ui.selectable_label(self.native_scene.as_ref()
                                .is_some_and(|(s,_)| s == &b.name), label).clicked() {
                                choose_cell = Some(b.name.clone());
                            }
                        }
                        if let Some(name) = choose_cell { self.load_native_scene(name); }
                    });
            });
        if let Some(item) = clicked {
            match item {
                EditorCatalogItem::Draw(terrain,idx) => self.native_selected = Some((terrain,idx)),
                _ => self.editor_modal = Some((item, false)),
            }
        }
        if let Some(next) = chosen { self.editor_modal = Some(next); }
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
                    scene.and_then(|s| s.env_tmds.get(slot).and_then(|&index|
                        s.res.tmds.get(index).and_then(|tmd|
                            Model::from_tmd(&tmd.raw))))
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
        let object_title = self.editor_name_for(scene, item);
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
                    self.editor_name_properties(ui, scene, item);
                    match item {
                        EditorCatalogItem::Draw(terrain,i) => {
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
                            ui.small("This native resource may be reused once new-instance placement is implemented.");
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
            if self.editor_modal.is_some() {
                self.editor_modal = None;
                self.editor_preview = None;
            } else {
                self.native_selected = None;
                self.native_drag = None;
            }
        }
        let mut pending_transform: Option<((bool, usize), [f32; 3], bool)> = None;
        let mut finish_drag: Option<EditorDrag> = None;
        let mut pending_import: Option<Vec<((bool,usize),[f32;3])>> = None;
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
                self.editor_catalog(ui, ready.as_deref());
            });
        egui::SidePanel::right("native_instance_tree")
            .resizable(true)
            .default_width(300.0)
            .min_width(230.0)
            .show(ctx, |ui| {
                ui.heading("Scene hierarchy");
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
                    (Some(_), Some(scene)) => {
                        ui.strong(&scene.name);
                        ui.label(format!("Mesh slots: {}", scene.env_tmds.len()));
                        ui.label(format!("Placed objects: {}", scene.placements.len()));
                        ui.label(format!("Terrain tiles: {}", scene.terrain.len()));
                        ui.label(format!("Walk-ground: {}", if scene.ground.is_some() { "loaded" } else { "unavailable" }));
                        ui.separator();
                        egui::ScrollArea::vertical().id_salt("native_scene_instance_scroll").show(ui, |ui| {
                            egui::CollapsingHeader::new(format!("Placements ({})", scene.placements.len()))
                                .default_open(true).show(ui, |ui| {
                                    for (i, d) in scene.placements.iter().enumerate() {
                                        if !self.editor_item_matches(Some(scene), EditorCatalogItem::Draw(false, i), &hierarchy_filter) { continue; }
                                        if ui.selectable_label(self.native_selected == Some((false, i)),
                                            format!("{}  [#{i:03}]  ({}, {}, {})",
                                                self.editor_name_for(Some(scene), EditorCatalogItem::Draw(false,i)),
                                                d.world_x, d.world_y, d.world_z)
                                        ).clicked() { self.native_selected = Some((false, i)); }
                                    }
                                });
                            egui::CollapsingHeader::new(format!("Terrain ({})", scene.terrain.len()))
                                .show(ui, |ui| {
                                    for (i, d) in scene.terrain.iter().enumerate() {
                                        if !self.editor_item_matches(Some(scene), EditorCatalogItem::Draw(true, i), &hierarchy_filter) { continue; }
                                        if ui.selectable_label(self.native_selected == Some((true, i)),
                                            format!("{}  [#{i:03}]  cell ({}, {})",
                                                self.editor_name_for(Some(scene), EditorCatalogItem::Draw(true,i)),
                                                d.cell.0, d.cell.1)
                                        ).clicked() { self.native_selected = Some((true, i)); }
                                    }
                                });
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
        // Selection must never move, zoom, or retarget the camera.
        // Press F to deliberately frame a selected piece instead.
        egui::TopBottomPanel::bottom("editor_readonly_inspector")
            .resizable(self.editor_inspector_open)
            .default_height(235.0)
            .min_height(if self.editor_inspector_open { 150.0 } else { 28.0 })
            .max_height(if self.editor_inspector_open { f32::INFINITY } else { 28.0 })
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.checkbox(&mut self.editor_inspector_open, "Placement inspector");
                    ui.small("Editor-only transforms");
                });
                if !self.editor_inspector_open { return; }
                if let (Some(scene), Some((terrain, i))) = (&ready, self.native_selected) {
                    let records = if terrain { &scene.terrain } else { &scene.placements };
                    if let Some(d) = records.get(i) {
                        ui.label(format!("{}  [#{i:03}] | {} | model slot {} | res TMD {} | anchor {:?} | source cell {:?}",
                            self.editor_name_for(Some(scene), EditorCatalogItem::Draw(terrain,i)),
                            if terrain { "Terrain" } else { "Placement" }, d.env_slot, d.res_tmd, d.anchor, d.cell));
                        ui.label(format!("Original PSX coordinates: X={} Y={} Z={}", d.world_x, d.world_y, d.world_z));
                        ui.label(format!("Original PSX rotations: X={} Y={} Z={} | anim ID={}", d.rot_x, d.rot_y, d.rot_z, d.anim_id));
                        ui.small("Original PSX placement stays unchanged. Editor offsets below are preview-only, with +Y upward.");
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
                                ui.small("Shift + left-drag in scene: move on X/Z | Ctrl+Shift + left-drag: height (Y) | Normal left-drag: pan");
                            } else { ui.small("This native record has no drawable mesh to move."); }
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
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("World Viewport");
            if let (Some(scene), Some(world)) = (&ready, &world) {
                ui.small(format!("Native '{}' | {} drawn instances | {} unique TMDs | {} unavailable",
                    scene.name, world.instances, world.unique_models, world.unavailable));
                ui.horizontal_wrapped(|ui| {
                    self.native_view.controls(ui, "Sample original PS1 TIM/CLUT atlas from assembled scene VRAM");
                    ui.checkbox(&mut self.native_ground_visible, "Show walk-ground")
                        .on_hover_text("Original per-cell PS1 walkable terrain: UV, CLUT and texture page are read from the disc's native heightfield and scene VRAM.");
                    ui.checkbox(&mut self.native_view.world_mirrored, "Mirror view (X)")
                        .on_hover_text("Checked by default to restore your previous left/right view. Display only: native PS1 world coordinates, geometry and textured materials are not modified. Uncheck to compare orientations.");
                    if ui.button("Frame all").clicked() { self.native_view.frame_all(); }
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
                    if ui.add_enabled(!self.native_undo.is_empty(), egui::Button::new("Undo move")).clicked() {
                        if let Some(edit) = self.native_undo.pop() {
                            pending_transform = Some((edit.key, edit.before, false));
                            self.native_redo.push(edit);
                        }
                    }
                    if ui.add_enabled(!self.native_redo.is_empty(), egui::Button::new("Redo move")).clicked() {
                        if let Some(edit) = self.native_redo.pop() {
                            pending_transform = Some((edit.key, edit.after, false));
                            self.native_undo.push(edit);
                        }
                    }
                    if ui.add_enabled(self.native_selected.is_some(), egui::Button::new("Frame selected (F)")).clicked() {
                        if let (Some(model), Some((terrain, index))) = (world.model.as_ref(), self.native_selected) {
                            if let Some(draw) = world.at(terrain, index) {
                                self.native_view.frame_instance(model, draw.center, draw.radius);
                            }
                        }
                    }
                    // Capture the artist's chosen front/three-quarter framing angle.
                    // F reuses that yaw/pitch without changing the zoom rule.
                    if ui.add_enabled(self.native_selected.is_some(), egui::Button::new("Set F view"))
                        .on_hover_text("Orbit until the selected piece looks right, then click to save this as the angle used by Frame Selected (F). This lasts until the editor is closed.")
                        .clicked() {
                        self.native_view.remember_frame_angle();
                        self.studio.status = Some("Frame Selected will now reuse your current camera angle (this session).".into());
                    }
                });
                ui.small("Click: select | Shift+LMB: move X/Z | Ctrl+Shift+LMB: move Y | LMB: pan | MMB/Alt+LMB: orbit or free-look | Esc: deselect | F: frame | Set F view: remember current angle | Home: frame all");
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
                            if let Some(draw) = world.at(terrain, index) {
                                self.native_view.frame_instance(model, draw.center, draw.radius);
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
                    let selected_id = self.native_selected.and_then(|(terrain, index)|
                        world.at(terrain, index).map(|d| d.id));
                    // A Shift-drag is an edit gesture; the viewport's standard
                    // LMB pan skips Shift so selection and navigation stay intact.
                    let view_rect = ui.available_rect_before_wrap();
                    let picked = self.native_view.show_world_limited(ui, model, &scene.res.vram, 900, 680, selected_id, self.native_ground_visible);
                    let (pointer, down, pressed, shift, ctrl, alt) = ui.input(|inp| (
                        inp.pointer.interact_pos(), inp.pointer.primary_down(),
                        inp.pointer.primary_pressed(), inp.modifiers.shift,
                        inp.modifiers.ctrl, inp.modifiers.alt));
                    if self.native_drag.is_some() && (!down || !shift || alt) {
                        finish_drag = self.native_drag.take();
                    }
                    if down && shift && !alt {
                        if let (Some(key), Some(pos)) = (self.native_selected, pointer) {
                            if pressed && view_rect.contains(pos) {
                                if let Some(draw) = world.at(key.0, key.1) {
                                    self.native_drag = Some(EditorDrag { key, pointer_start: pos, offset_start: draw.offset });
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
                                pending_transform = Some((key, desired, false));
                            }
                        }
                    }
                    if let Some(id) = picked {
                        if let Some(draw) = world.by_id(id) {
                            self.native_selected = Some((draw.terrain, draw.index));
                            ui.ctx().request_repaint();
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
        self.editor_catalog_popup(ctx, ready.as_deref(), world.as_deref());
        // All panels have released their Arc clones before the mutation.
        // Arc::get_mut keeps one writable world, without cloning thousands
        // of vertices on every mouse frame. The original scene remains intact.
        drop(world);
        if let Some((key, desired, record)) = pending_transform {
            if let Some((_, NativeSceneSlot::Ready(_, world))) = self.native_scene.as_mut() {
                if let Some(world) = Arc::get_mut(world) {
                    if let Some(draw) = world.at(key.0, key.1) {
                        let before = draw.offset;
                        if before != desired && world.set_offset(key.0, key.1, desired) {
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
        if let Some(edits) = pending_import {
            if let Some((_, NativeSceneSlot::Ready(_, world))) = self.native_scene.as_mut() {
                if let Some(world) = Arc::get_mut(world) {
                    // Restore every original anchor first; absent rows mean zero offset.
                    for draw in world.draws.iter().map(|d|(d.terrain,d.index)).collect::<Vec<_>>() {
                        world.set_offset(draw.0, draw.1, [0.0;3]);
                    }
                    for (key, offset) in edits {
                        world.set_offset(key.0, key.1, offset);
                    }
                    self.native_undo.clear();
                    self.native_redo.clear();
                    self.native_drag = None;
                    self.native_view.version = self.native_view.version.wrapping_add(1);
                    ctx.request_repaint();
                    self.studio.status = Some("Preview layout offsets loaded; the PS1 disc remains untouched.".into());
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
        if self.studio.poll() {
            ctx.request_repaint();
        }
        let completed = self.native_scene.as_ref().and_then(|(_, s)| match s {
            NativeSceneSlot::Loading(rx) => rx.try_recv().ok(),
            _ => None,
        });
        if let Some(result) = completed {
            if let Some((_, slot)) = &mut self.native_scene {
                *slot = match result {
                    Ok((scene, world)) => NativeSceneSlot::Ready(Arc::new(scene), Arc::new(world)),
                    Err(e) => NativeSceneSlot::Failed(e),
                };
            }
            ctx.request_repaint();
        }
        if matches!(self.native_scene.as_ref(), Some((_, NativeSceneSlot::Loading(_)))) {
            ctx.request_repaint_after(std::time::Duration::from_millis(75));
        }
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
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
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
        app.editor_inspector_open = false;
        frame(&mut app, &ctx, vec![]);
        app.editor_inspector_open = true;
        app.editor_modal = Some((EditorCatalogItem::Character(0), false));
        frame(&mut app, &ctx, vec![]);
        assert!(app.editor_modal.is_some());
        assert_eq!(app.editor_name_edit_key.as_deref(), Some("actor:character:0"));
    }
    #[test]
    fn escape_closes_modal_before_clearing_selection_without_moving_camera() {
        let mut app = app();
        let ctx = egui::Context::default();
        app.native_selected = Some((false, 7));
        app.editor_modal = Some((EditorCatalogItem::Character(0), false));
        let camera = app.native_view.cam;
        let escape = || egui::Event::Key { key: egui::Key::Escape, physical_key: None,
            pressed: true, repeat: false, modifiers: egui::Modifiers::default() };
        frame(&mut app, &ctx, vec![escape()]);
        assert!(app.editor_modal.is_none());
        assert_eq!(app.native_selected, Some((false, 7)));
        assert_eq!(app.native_view.cam, camera);
        frame(&mut app, &ctx, vec![egui::Event::Key { key: egui::Key::Escape, physical_key: None,
            pressed: false, repeat: false, modifiers: egui::Modifiers::default() }]);
        frame(&mut app, &ctx, vec![escape()]);
        assert_eq!(app.native_selected, None);
        assert_eq!(app.native_view.cam, camera);
    }
}
