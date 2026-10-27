//! Shared CPU-rasterized viewport. Asset previews retain legacy left-drag
//! orbit; the world editor uses left-drag pan and middle-mouse orbit.

use eframe::egui::{self, Color32, ColorImage, PointerButton, TextureHandle, TextureOptions};
use legaia_tim::Vram;
use crate::raster::{self, Camera, Model, Shading};

pub(crate) struct Viewport {
    pub cam: Camera,
    pub shading: Shading,
    pub version: u64,
    pub world_mirrored: bool,
    /// Editor-only pivot for MMB/Alt+LMB, or None for free-look in place.
    pub world_orbit_anchor: Option<[f32; 3]>,
    /// ID captured at primary mouse-down on the already selected mesh.
    /// While held, LMB moves that instance instead of panning the camera.
    pub world_left_drag_selected: Option<u32>,
    pub world_selected: Vec<u32>,
    rendered_selection: Vec<u32>,
    rendered: Option<(Camera, Shading, [usize; 2], u64, Option<u32>, bool, bool)>,
    tex: Option<TextureHandle>,
}

/// The raster camera's default Euler branch renders scene-up down on screen.
/// Keep its eye and forward vector EXACTLY the same but use the equivalent
/// upright Euler branch (a 180-degree camera roll, not a model rotation).
/// Never apply this to Frame Selected: frame_selected_pose already chooses
/// its own upright Euler branch from native placement + editor transforms.
fn upright_default_camera() -> Camera {
    let mut cam = Camera::default();
    cam.yaw = (cam.yaw + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU);
    cam.pitch = std::f32::consts::PI - cam.pitch;
    cam
}

impl Viewport {
    pub fn new(shading: Shading) -> Self {
        Self {
            cam: upright_default_camera(), shading, version: 0,
            world_mirrored: true,
            world_orbit_anchor: None,
            world_left_drag_selected: None,
            world_selected: Vec::new(), rendered_selection: Vec::new(),
            rendered: None, tex: None,
        }
    }

    pub fn show(&mut self, ui: &mut egui::Ui, model: &Model, vram: &Vram) {
        self.show_limited(ui, model, vram, 1600, 1200);
    }

    /// Existing per-asset preview behavior; left-drag orbits, wheel zooms.
    pub fn show_limited(&mut self, ui: &mut egui::Ui, model: &Model, vram: &Vram, max_w: usize, max_h: usize) {
        let _ = self.show_internal(ui, model, vram, max_w, max_h, false, None, true, false);
    }

    /// World editor: LMB drags a mesh ONLY when starting on the selected
    /// instance, otherwise pans; MMB or Alt+LMB orbits. Ctrl+LMB on the
    /// selected instance changes height. Shift+MMB pans and Ctrl+MMB dollies.
    /// Returns an instance ID when a native mesh is clicked.
    pub fn show_world_limited(&mut self, ui: &mut egui::Ui, model: &Model, vram: &Vram,
                              max_w: usize, max_h: usize, selected: Option<u32>,
                              show_ground: bool, repository_drag: bool) -> Option<u32> {
        self.show_internal(ui, model, vram, max_w, max_h, true, selected, show_ground, repository_drag)
    }

    /// Identical world navigation/Frame Selected; select native donor palettes by draw ID.
    pub fn show_world_limited_with_sources(&mut self, ui: &mut egui::Ui, model: &Model,
        vram: &Vram, max_w: usize, max_h: usize, selected: Option<u32>,
        show_ground: bool, repository_drag: bool,
        sources:&std::collections::HashMap<u32,std::sync::Arc<Vram>>) -> Option<u32> {
        self.show_internal_sources(ui,model,vram,max_w,max_h,true,selected,show_ground,repository_drag,Some(sources))
    }

    /// Center and frame the selected instance using a universal reference
    /// composition, transformed by BOTH native PS1 placement orientation and
    /// editor-only rotation. The eye offset is recomputed for each object's
    /// actual center and scale; the current browsing camera angle is ignored.
    pub fn frame_instance(&mut self, model: &Model, point: [f32; 3], radius: f32,
                          native_quat: [f32; 4], editor_rotation: [f32; 3]) {
        frame_selected_pose(&mut self.cam, model.center, model.radius,
            point, radius, native_quat, editor_rotation);
    }

    /// Reset the whole-cell and asset preview camera to an upright pose.
    /// Does NOT affect Frame Selected (frame_instance / F).
    pub fn frame_all(&mut self) { self.cam = upright_default_camera(); }

    fn show_internal(&mut self, ui: &mut egui::Ui, model: &Model, vram: &Vram,
                     max_w: usize, max_h: usize, world: bool, selected: Option<u32>,
                     show_ground: bool, repository_drag: bool) -> Option<u32> {
        self.show_internal_sources(ui,model,vram,max_w,max_h,world,selected,show_ground,repository_drag,None)
    }

    fn show_internal_sources(&mut self, ui: &mut egui::Ui, model: &Model, vram: &Vram,
        max_w: usize, max_h: usize, world: bool, selected: Option<u32>, show_ground: bool,
        repository_drag: bool,
        donor_vram:Option<&std::collections::HashMap<u32,std::sync::Arc<Vram>>>) -> Option<u32> {
        let avail = ui.available_size();
        let (rect, resp) = ui.allocate_exact_size(avail, egui::Sense::click_and_drag());
        let mut picked = None;
        let mirror = world && self.world_mirrored;
        // Calculate the exact same pixel resolution used by both the renderer
        // and the hit tester (no new GPU readbacks or per-frame picks).
        let ppp = ui.ctx().pixels_per_point();
        let px = [
            ((rect.width() * ppp) as usize).clamp(16, max_w.max(16)),
            ((rect.height() * ppp) as usize).clamp(16, max_h.max(16)),
        ];
        if world {
            // Capture on mouse-down, BEFORE handling any camera movement.
            // Egui's dragged_by fires later, but the press must reserve the
            // gesture immediately so the camera never jumps at drag start.
            if !ui.input(|i| i.pointer.primary_down()) {
                self.world_left_drag_selected = None;
            } else if ui.input(|i| i.pointer.primary_pressed()) {
                let alt = ui.input(|i| i.modifiers.alt);
                self.world_left_drag_selected = if !repository_drag && !alt && resp.hovered() {
                    resp.interact_pointer_pos().and_then(|pos| {
                        let point = world_screen_pick_point(rect, pos, px, mirror);
                        let hit = raster::pick_instance(model, self.cam, px[0], px[1], point);
                        hit.filter(|id|selected==Some(*id)||self.world_selected.contains(id)||ui.input(|i|i.modifiers.shift))
                    })
                } else { None };
            }
        }
        if world {
            let shift = ui.input(|i| i.modifiers.shift);
            let ctrl = ui.input(|i| i.modifiers.ctrl);
            let alt = ui.input(|i| i.modifiers.alt);
            let middle = resp.dragged_by(PointerButton::Middle);
            let left = resp.dragged_by(PointerButton::Primary);
            let delta = ui.input(|i| i.pointer.delta());
            let drag_x = if mirror { -delta.x } else { delta.x };
            if repository_drag {
                // An asset held from All Objects owns the pointer until release;
                // do not pan/orbit/dolly a scene while placing it.
            } else if (middle && shift) || (left && !alt && self.world_left_drag_selected.is_none()) {
                // Shift the camera's orbit pivot parallel to the view plane;
                // dragging right/down moves the scenery right/down.
                let (sy, cy) = self.cam.yaw.sin_cos();
                let (sp, cp) = self.cam.pitch.sin_cos();
                let right = [cy, 0.0, sy];
                // Match the existing rasterizer exactly: after yaw about Y and
                // pitch about X, projected screen-down is (sy*sp, cp, -cy*sp).
                // The previous negative X sign caused vertical drags to skew
                // sideways or feel reversed at particular camera yaw angles.
                let screen_y = [sy * sp, cp, -cy * sp];
                let half_fov = 45f32.to_radians() * 0.5;
                let distance = model.radius / half_fov.sin() * self.cam.zoom.max(0.0000001);
                let units_per_point = (2.0 * distance * half_fov.tan() / rect.height().max(1.0)).max(0.35);
                for k in 0..3 {
                    self.cam.target_offset[k] -= (drag_x * right[k] + delta.y * screen_y[k]) * units_per_point;
                }
            } else if middle && ctrl {
                self.cam.zoom = (self.cam.zoom * (delta.y * 0.012).exp()).clamp(0.0000001, 50.0);
            } else if middle || (left && alt && self.world_left_drag_selected.is_none()) {
                // Orbit around the current pivot and keep the camera upright at the poles.
                if let Some(piece) = self.world_orbit_anchor.filter(|_| !ctrl) {
                    // The first drag uses the CURRENT eye and orientation.
                    // Orbit around this piece without recentering or zooming.
                    editor_orbit_around_piece(&mut self.cam, model.center, model.radius,
                        piece, drag_x * 0.01, -delta.y * 0.01);
                } else {
                    // Nothing selected: rotate in place, without camera translation.
                    editor_look_in_place(&mut self.cam, model.radius, drag_x * 0.01, -delta.y * 0.01);
                }
            }
            if resp.hovered() {
                if ui.input(|i| i.key_pressed(egui::Key::Home)) { self.frame_all(); }
            }
        } else if resp.dragged() {
            // Asset previews formerly clamped pitch to the *other* Euler
            // branch, instantly turning the model upside down on first drag.
            // Also use per-frame pointer delta (drag_delta is cumulative).
            let d = ui.input(|i| i.pointer.delta());
            self.cam.yaw = (self.cam.yaw + d.x * 0.01)
                .rem_euclid(std::f32::consts::TAU);
            self.cam.pitch = (self.cam.pitch - d.y * 0.01).clamp(
                std::f32::consts::FRAC_PI_2 + 0.02,
                std::f32::consts::PI * 1.5 - 0.02,
            );
        }
        if resp.hovered() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll != 0.0 {
                if world {
                    self.cam.zoom = (self.cam.zoom * (-scroll * 0.002).exp()).clamp(0.0000001, 50.0);
                } else {
                    self.cam.zoom = (self.cam.zoom * (1.0 - scroll * 0.002)).clamp(0.1, 10.0);
                }
            }
        }
        let highlight = if world { selected } else { None };
        let state = (self.cam, self.shading, px, self.version, highlight, show_ground, mirror);
        if self.rendered != Some(state) || self.rendered_selection!=self.world_selected || self.tex.is_none() {
            let mut rgba = if world {
                raster::render_highlights_with_sources(model, vram, self.cam, self.shading, px[0], px[1],
                    if self.world_selected.is_empty() {highlight.as_slice()} else {&self.world_selected}, show_ground, donor_vram)
            } else {
                raster::render(model, vram, self.cam, self.shading, px[0], px[1])
            };
            if mirror {
                flip_rgba_rows(&mut rgba, px[0]);
            }
            let img = ColorImage::from_rgba_unmultiplied(px, &rgba);
            match &mut self.tex {
                Some(t) => t.set(img, TextureOptions::LINEAR),
                None => self.tex = Some(ui.ctx().load_texture("model", img, TextureOptions::LINEAR)),
            }
            self.rendered = Some(state);
            self.rendered_selection=self.world_selected.clone();
        }
        if let Some(tex) = &self.tex {
            ui.painter().image(tex.id(), rect,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                Color32::WHITE);
        }
        if world && !repository_drag && resp.hovered() && ui.input(|i|i.pointer.primary_pressed() && !i.modifiers.alt) {
            if let Some(pos) = resp.interact_pointer_pos() {
                let screen_point = world_screen_pick_point(rect, pos, px, mirror);
                picked = raster::pick_instance(model, self.cam, px[0], px[1], screen_point);
            }
        }
        picked
    }

    /// Shared shading controls; world editor adds its own navigation controls.
    pub fn controls(&mut self, ui: &mut egui::Ui, textured_hint: &str) {
        ui.selectable_value(&mut self.shading, Shading::Textured, "Textured (retail shading)")
            .on_hover_text(textured_hint);
        ui.selectable_value(&mut self.shading, Shading::Lit, "Untextured + viewer light")
            .on_hover_text("Viewer light; not retail lighting");
        if ui.button("Reset view").clicked() { self.frame_all(); }
    }
}

/// Convert egui logical pointer coordinates to the rasterizer's native
/// pixel coordinates. The editor's optional mirror is presentation-only.
fn world_screen_pick_point(rect: egui::Rect, pos: egui::Pos2,
                           px: [usize;2], mirror: bool) -> [f32;2] {
    let screen_x = (pos.x - rect.left()) * px[0] as f32 / rect.width().max(1.0);
    [if mirror { (px[0] as f32 - 1.0 - screen_x).max(0.0) } else { screen_x },
     (pos.y - rect.top()) * px[1] as f32 / rect.height().max(1.0)]
}

/// Flip RGBA8 rows horizontally, without touching the native model or VRAM.
fn flip_rgba_rows(rgba: &mut [u8], width: usize) {
    if width == 0 || width > usize::MAX / 4 { return; }
    for row in rgba.chunks_exact_mut(width * 4) {
        for x in 0..width / 2 {
            let other = width - 1 - x;
            for channel in 0..4 {
                row.swap(x * 4 + channel, other * 4 + channel);
            }
        }
    }
}

#[cfg(test)]
mod world_navigation_tests {
    use super::*;
    #[test]
    fn group_member_press_captures_drag_without_camera_pan_or_release_reselection() {
        let world=super::super::collision_recipes::properties_fixture();let model=world.model.as_ref().unwrap();
        let ctx=egui::Context::default();let mut view=Viewport::new(Shading::Textured);
        view.world_mirrored=false;view.world_selected=vec![0,1];
        let mut rect=egui::Rect::NOTHING;
        let frame=|view:&mut Viewport,rect:&mut egui::Rect,events:Vec<egui::Event>| {
            let mut picked=None;
            let _=ctx.run(egui::RawInput{screen_rect:Some(egui::Rect::from_min_size(egui::Pos2::ZERO,egui::vec2(320.0,240.0))),events,..Default::default()},|ctx|{
                egui::CentralPanel::default().show(ctx,|ui|{*rect=ui.available_rect_before_wrap();
                    picked=view.show_world_limited_with_sources(ui,model,&Vram::new(),400,300,Some(0),true,false,&world.foreign_vram);
                });
            });picked
        };
        frame(&mut view,&mut rect,vec![]);let camera=view.cam;
        let px=[rect.width() as usize,rect.height() as usize];
        let point=(0..px[1]).step_by(3).find_map(|y|(0..px[0]).step_by(3).find_map(|x|
            (raster::pick_instance(model,camera,px[0],px[1],[x as f32,y as f32])==Some(1)).then_some([x,y]))).expect("Fixture's second object must be visible");
        let pos=rect.min+egui::vec2(point[0] as f32,point[1] as f32);
        let press=egui::Event::PointerButton{pos,button:PointerButton::Primary,pressed:true,modifiers:Default::default()};
        assert_eq!(frame(&mut view,&mut rect,vec![egui::Event::PointerMoved(pos),press]),Some(1));
        assert_eq!(view.world_left_drag_selected,Some(1)); // not just primary 0
        frame(&mut view,&mut rect,vec![egui::Event::PointerMoved(pos+egui::vec2(16.0,8.0))]);
        assert_eq!(view.cam,camera);
        let release=egui::Event::PointerButton{pos,button:PointerButton::Primary,pressed:false,modifiers:Default::default()};
        assert_eq!(frame(&mut view,&mut rect,vec![release]),None);
        assert_eq!(view.world_left_drag_selected,None);
        let single=raster::render_highlight_with_sources(model,&Vram::new(),camera,Shading::Textured,px[0],px[1],Some(0),true,None);
        let group=raster::render_highlights_with_sources(model,&Vram::new(),camera,Shading::Textured,px[0],px[1],&[0,1],true,None);
        assert_ne!(single,group,"Both independent meshes must receive highlights");
    }
    #[test]
    fn world_object_drag_picks_exact_selected_instance_with_mirror() {
        let rect = egui::Rect::from_min_size(egui::pos2(100.0, 40.0), egui::vec2(200.0, 100.0));
        let raw = world_screen_pick_point(rect, egui::pos2(150.0, 65.0), [400, 200], false);
        assert_eq!(raw, [100.0, 50.0]);
        let mirrored = world_screen_pick_point(rect, egui::pos2(150.0, 65.0), [400, 200], true);
        assert_eq!(mirrored, [299.0, 50.0]);
        // Object ID equality is strict. Dragging a different piece never
        // captures or moves the highlighted selection.
        let select = Some(15u32);
        for hit in [None, Some(0), Some(16)] {
            assert!(select.filter(|id| Some(*id) == hit).is_none());
        }
        assert_eq!(select.filter(|id| Some(*id) == Some(15)), Some(15));
    }
    #[test]
    fn viewport_mirror_reverses_rows_without_touching_pixels_within_each_rgba() {
        let original = vec![1,2,3,4, 5,6,7,8, 9,10,11,12, 13,14,15,16];
        let mut rgba = original.clone();
        flip_rgba_rows(&mut rgba, 2);
        assert_eq!(rgba, vec![5,6,7,8, 1,2,3,4, 13,14,15,16, 9,10,11,12]);
        flip_rgba_rows(&mut rgba, 2);
        assert_eq!(rgba, original);
    }
    #[test]
    fn screen_vertical_pan_is_perpendicular_to_horizontal_at_every_yaw() {
        for yaw in [0.0_f32, 0.1, 0.7, 1.3, 2.1, 3.7, 5.9] {
            for pitch in [-1.2_f32, -0.35, 0.0, 0.5, 1.57] {
                let (sy, cy) = yaw.sin_cos();
                let (sp, cp) = pitch.sin_cos();
                let right = [cy, 0.0, sy];
                let screen_y = [sy * sp, cp, -cy * sp];
                let dot = right[0]*screen_y[0]+right[1]*screen_y[1]+right[2]*screen_y[2];
                assert!(dot.abs() < 0.000001, "yaw={yaw} pitch={pitch} dot={dot}");
            }
        }
    }
}

/// PS1 World Editor camera helpers. Only the editor view changes here;
/// source meshes, placements, terrain collision and disc data remain native.
// The renderer accepts unrestricted Euler pitch. Wrapping at TAU permits
// repeated complete loops over both poles, rather than stopping at +/-85 deg.
// Camera pose remains unchanged on mere selection; only dragging rotates.
/// Relative eye direction inferred from the screenshot's selected prop:
/// eye ~ (4790.3,1453.1,3732.5) and native PSX placement (4800,-512,5440).
/// Render coordinates flip PSX Y, giving ~(-9.7,+941.1,-1707.5).
/// These are a fixed *local-space* reference, not absolute world coordinates.
const FRAME_REFERENCE_EYE_LOCAL: [f32; 3] = [-9.7, 941.1, -1707.5];

/// Apply the same editor Euler composition as world_view::editor_rotate_vertex
/// (X then Y then Z). No scale here: scale affects distance, not viewpoint.
fn rotate_frame_euler(v: [f32; 3], r: [f32; 3]) -> [f32; 3] {
    let (sx, cx) = r[0].sin_cos();
    let (sy, cy) = r[1].sin_cos();
    let (sz, cz) = r[2].sin_cos();
    let x = v[0];
    let y = v[1] * cx - v[2] * sx;
    let z = v[1] * sx + v[2] * cx;
    let xx = x * cy + z * sy;
    let zz = -x * sy + z * cy;
    [xx * cz - y * sz, xx * sz + y * cz, zz]
}

/// Rotate a local direction by the original PS1 object's render-space
/// quaternion (native orientation is baked into vertex positions).
fn rotate_frame_quat(v: [f32; 3], q: [f32; 4]) -> [f32; 3] {
    let u = [q[0], q[1], q[2]];
    let t = [
        2.0 * (u[1] * v[2] - u[2] * v[1]),
        2.0 * (u[2] * v[0] - u[0] * v[2]),
        2.0 * (u[0] * v[1] - u[1] * v[0]),
    ];
    [
        v[0] + q[3] * t[0] + u[1] * t[2] - u[2] * t[1],
        v[1] + q[3] * t[1] + u[2] * t[0] - u[0] * t[2],
        v[2] + q[3] * t[2] + u[0] * t[1] - u[1] * t[0],
    ]
}

/// Universal framing from the reference screenshot, independent of where the
/// object is located or what camera angle the user last navigated from.
/// Object center is the center of the EDITED mesh, not the raw PSX anchor.
fn frame_selected_pose(
    cam: &mut Camera, scene_center: [f32; 3], scene_radius: f32,
    object_center: [f32; 3], object_radius: f32,
    native_quat: [f32; 4], editor_rotation: [f32; 3],
) {
    let native_direction = rotate_frame_quat(FRAME_REFERENCE_EYE_LOCAL, native_quat);
    let relative_eye = rotate_frame_euler(native_direction, editor_rotation);
    let length = relative_eye.iter().map(|v| v*v).sum::<f32>().sqrt().max(0.00001);
    let forward: [f32; 3] = relative_eye.map(|v| -v / length);
    // The rasterizer supports two Euler yaw/pitch representations for the
    // SAME camera eye and forward vector. The principal asin() solution
    // (pitch between -90 and +90 degrees) makes +world-Y point DOWN in the
    // frame's pixel coordinates, so Frame Selected rolls the scene upside
    // down even though the camera POSITION is correct. Choose the equivalent
    // opposite Euler branch to keep world-up pointing UP on screen.
    //
    // yaw' = yaw + PI, pitch' = PI - pitch preserves forward exactly:
    // [-sin(yaw') cos(pitch'), sin(pitch'), cos(yaw') cos(pitch')]
    // == [-sin(yaw) cos(pitch), sin(pitch), cos(yaw) cos(pitch)]
    // but reverses BOTH horizontal and vertical camera image axes (180 roll).
    // Do not flip geometry, texture UVs, global renderer or camera POSITION.
    let base_yaw = (-forward[0]).atan2(forward[2]);
    let base_pitch = forward[1].clamp(-1.0, 1.0).asin();
    cam.yaw = (base_yaw + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU);
    cam.pitch = std::f32::consts::PI - base_pitch;
    cam.target_offset = [0, 1, 2].map(|k| object_center[k] - scene_center[k]);
    // Preserve the existing radius-aware frame distance; scaled objects fit
    // without introducing selection-dependent changes to the model center.
    cam.zoom = (object_radius.max(0.01) / scene_radius.max(0.01) * 1.8)
        .clamp(0.00015, 50.0);
}

fn editor_camera_forward(cam: Camera) -> [f32; 3] {
    let (sy, cy) = cam.yaw.sin_cos();
    let (sp, cp) = cam.pitch.sin_cos();
    [-sy * cp, sp, cy * cp]
}

fn editor_camera_distance(cam: Camera, model_radius: f32) -> f32 {
    model_radius / (45f32.to_radians() * 0.5).sin() * cam.zoom.max(0.0000001)
}

pub(super) fn editor_camera_eye(cam: Camera, center: [f32; 3], model_radius: f32) -> [f32; 3] {
    let d = editor_camera_distance(cam, model_radius);
    let f = editor_camera_forward(cam);
    [0, 1, 2].map(|k| center[k] + cam.target_offset[k] - d * f[k])
}

// The viewer rasterizer transforms by yaw about world Y, then pitch about X.
// Convert a world-space direction into the camera's local coordinate basis.
fn editor_world_to_camera(cam: Camera, v: [f32; 3]) -> [f32; 3] {
    let (sy, cy) = cam.yaw.sin_cos();
    let (sp, cp) = cam.pitch.sin_cos();
    let x1 = v[0] * cy + v[2] * sy;
    let z1 = -v[0] * sy + v[2] * cy;
    [x1, v[1] * cp - z1 * sp, v[1] * sp + z1 * cp]
}

// Inverse of the editor's yaw-then-pitch view matrix.
fn editor_camera_to_world(cam: Camera, v: [f32; 3]) -> [f32; 3] {
    let (sy, cy) = cam.yaw.sin_cos();
    let (sp, cp) = cam.pitch.sin_cos();
    let y = v[1] * cp + v[2] * sp;
    let z1 = -v[1] * sp + v[2] * cp;
    [v[0] * cy - z1 * sy, y, v[0] * sy + z1 * cy]
}

/// Nothing selected: turn in place. The camera EYE and zoom never change.
/// Both axes permit continuous full revolutions; the viewer may deliberately
/// look over and under an object without hitting a vertical stop.
fn editor_look_in_place(cam: &mut Camera, model_radius: f32, yaw_delta: f32, pitch_delta: f32) {
    let eye = editor_camera_eye(*cam, [0.0; 3], model_radius);
    cam.yaw = (cam.yaw + yaw_delta).rem_euclid(std::f32::consts::TAU);
    cam.pitch = (cam.pitch + pitch_delta).rem_euclid(std::f32::consts::TAU);
    let dist = editor_camera_distance(*cam, model_radius);
    let forward = editor_camera_forward(*cam);
    // model.center cancels out: target_offset is relative to center.
    for k in 0..3 {
        cam.target_offset[k] = eye[k] + dist * forward[k];
    }
}

/// A selected piece is ONLY a rotation pivot, not an instant focus command.
/// On each actual mouse-drag delta, rigidly rotate the existing camera pose
/// about that pivot. Selection alone does nothing to yaw, pitch, zoom or eye.
///
/// The selected point keeps its original screen position throughout the
/// orbit, even if it's off-center. No snap, centering or zoom adjustment.
fn editor_orbit_around_piece(
    cam: &mut Camera,
    center: [f32; 3],
    model_radius: f32,
    piece: [f32; 3],
    yaw_delta: f32,
    pitch_delta: f32,
) {
    if yaw_delta == 0.0 && pitch_delta == 0.0 { return; }
    let old_cam = *cam;
    let old_eye = editor_camera_eye(old_cam, center, model_radius);
    let eye_relative_to_piece: [f32; 3] = [0, 1, 2].map(|k| old_eye[k] - piece[k]);
    let local = editor_world_to_camera(old_cam, eye_relative_to_piece);

    // Unlike v0.4.5, do not point the camera directly at the piece or
    // recalculate zoom from the piece's distance. That was the 'teleport'.
    cam.yaw = (cam.yaw + yaw_delta).rem_euclid(std::f32::consts::TAU);
    cam.pitch = (cam.pitch + pitch_delta).rem_euclid(std::f32::consts::TAU);
    let new_relative = editor_camera_to_world(*cam, local);
    let new_eye: [f32; 3] = [0, 1, 2].map(|k| piece[k] + new_relative[k]);
    let d = editor_camera_distance(*cam, model_radius);
    let forward = editor_camera_forward(*cam);
    for k in 0..3 {
        // Rebuild the camera's existing pivot representation from the new
        // eye without moving the object or changing zoom.
        cam.target_offset[k] = new_eye[k] + d * forward[k] - center[k];
    }
}

#[cfg(test)]
mod selected_orbit_navigation_tests {
    use super::*;

    fn near(a: [f32; 3], b: [f32; 3]) {
        for k in 0..3 {
            let tolerance = (a[k].abs().max(b[k].abs()) * 0.00002).max(0.02);
            assert!((a[k] - b[k]).abs() <= tolerance,
                "axis {k}: {} != {} (tolerance {})", a[k], b[k], tolerance);
        }
    }

    #[test]
    fn camera_basis_conversions_roundtrip() {
        for yaw in [0.0_f32, 0.4, 1.9, 4.8] {
            for pitch in [-1.2_f32, -0.35, 0.3, 1.2] {
                let cam = Camera { yaw, pitch, ..Camera::default() };
                let vec = [173.25, -94.5, -298.75];
                near(editor_camera_to_world(cam, editor_world_to_camera(cam, vec)), vec);
            }
        }
    }

    #[test]
    fn orbit_changes_only_on_drag_and_preserves_zoom() {
        let mut cam = Camera::default();
        cam.target_offset = [180.0, -65.0, 400.0];
        let center = [1000.0, 20.0, -500.0];
        let piece = [1140.0, 150.0, -130.0];
        let old = cam;
        editor_orbit_around_piece(&mut cam, center, 1800.0, piece, 0.0, 0.0);
        assert_eq!(cam, old);
        let d = cam.zoom;
        editor_orbit_around_piece(&mut cam, center, 1800.0, piece, 0.12, -0.07);
        assert_eq!(cam.zoom, d);
        assert_ne!(cam.yaw, old.yaw);
        assert_ne!(cam.pitch, old.pitch);
    }

    #[test]
    fn orbit_preserves_distance_and_screen_position_of_selected_pivot() {
        let mut cam = Camera::default();
        cam.target_offset = [38.0, 33.0, 103.0];
        let center = [900.0, -33.0, -700.0];
        let piece = [1111.0, 44.0, -345.0];
        let radius = 800.0;
        let initial_eye = editor_camera_eye(cam, center, radius);
        let relative: [f32; 3] = [0, 1, 2].map(|k| piece[k] - initial_eye[k]);
        let starting_camera_space = editor_world_to_camera(cam, relative);
        for _ in 0..40 {
            editor_orbit_around_piece(&mut cam, center, radius, piece, 0.019, 0.008);
            let eye = editor_camera_eye(cam, center, radius);
            let rel = [0, 1, 2].map(|k| piece[k] - eye[k]);
            near(editor_world_to_camera(cam, rel), starting_camera_space);
        }
    }

    #[test]
    fn repeated_vertical_orbits_go_past_both_poles_and_return() {
        let center = [800.0, 45.0, -1300.0];
        let piece = [760.0, 110.0, -1170.0];
        let radius = 620.0;
        let mut cam = Camera::default();
        cam.target_offset = [35.0, -40.0, 90.0];
        let original_eye = editor_camera_eye(cam, center, radius);
        let old_zoom = cam.zoom;
        let old_local = editor_world_to_camera(cam,
            [0, 1, 2].map(|k| piece[k] - original_eye[k]));
        let start_pitch = cam.pitch;
        let mut saw_overhead = false;
        let mut saw_underneath = false;
        for _ in 0..480 {
            editor_orbit_around_piece(&mut cam, center, radius, piece,
                0.0, std::f32::consts::TAU / 240.0);
            let p = cam.pitch.rem_euclid(std::f32::consts::TAU);
            saw_overhead |= (1.50..1.65).contains(&p);
            saw_underneath |= (4.63..4.80).contains(&p);
            let eye = editor_camera_eye(cam, center, radius);
            let current_local = editor_world_to_camera(cam,
                [0, 1, 2].map(|k| piece[k] - eye[k]));
            near(current_local, old_local);
        }
        assert!(saw_overhead && saw_underneath, "Full pitch must pass both poles");
        // A full 480-frame loop accumulates tiny f32 roundoff in world space;
        // the eye returns to its start to within sub-unit numerical precision.
        let final_eye = editor_camera_eye(cam, center, radius);
        for k in 0..3 { assert!((final_eye[k] - original_eye[k]).abs() < 0.1); }
        assert!((cam.pitch - start_pitch.rem_euclid(std::f32::consts::TAU)).abs() < 0.002);
        assert_eq!(cam.zoom, old_zoom);
    }

    #[test]
    fn frame_selected_matches_reference_eye_direction_and_edited_center() {
        let mut cam = Camera::default();
        let world_center = [1000.0, -50.0, 2000.0];
        let center = [4800.0, 512.0, 5440.0];
        let world_radius = 12_000.0;
        let object_radius = 380.0;
        let identity = [0.0, 0.0, 0.0, 1.0];
        frame_selected_pose(&mut cam, world_center, world_radius,
            center, object_radius, identity, [0.0; 3]);
        let eye = editor_camera_eye(cam, world_center, world_radius);
        let v: [f32; 3] = [0, 1, 2].map(|k| eye[k] - center[k]);
        let length = v.iter().map(|x| x*x).sum::<f32>().sqrt();
        let reference_length = FRAME_REFERENCE_EYE_LOCAL.iter().map(|x|x*x).sum::<f32>().sqrt();
        for i in 0..3 {
            assert!((v[i]/length - FRAME_REFERENCE_EYE_LOCAL[i]/reference_length).abs() < 0.0001,
                "camera axis {i}: expected screenshot direction, got {}", v[i]/length);
        }
        assert!(eye[1] > center[1], "camera must look down from ABOVE the model");
        // The previous branch preserved position but rolled the raster view
        // upside down. +Y-up must now project toward SMALLER image-Y.
        let screen_up = editor_world_to_camera(cam, [0.0, 1.0, 0.0]);
        assert!(screen_up[1] < -0.01, "up direction must project UP in the viewport, not down: {screen_up:?}");
        assert!(cam.pitch.cos() < 0.0, "framing must choose the upright Euler branch");
        // Camera's target is the edited object, never the scene origin.
        assert_eq!(cam.target_offset, [3800.0, 562.0, 3440.0]);
    }

    #[test]
    fn frame_selected_tracks_each_objects_position_size_and_baked_native_yaw() {
        let scene_center = [1000.0, 30.0, 2000.0];
        let mut cam = Camera::default();
        let identity = [0.0, 0.0, 0.0, 1.0];
        let a = [2400.0, 180.0, 3900.0];
        frame_selected_pose(&mut cam, scene_center, 10000.0,
            a, 130.0, identity, [0.0;3]);
        let eye_a = editor_camera_eye(cam, scene_center, 10000.0);
        let vec_a = [0,1,2].map(|k|eye_a[k]-a[k]);
        let dist_a = vec_a.iter().map(|v|v*v).sum::<f32>().sqrt();
        // A larger duplicate with a different placement should preserve the
        // screenshot-based relative angle while doubling the camera distance.
        let b = [-1500.0, 300.0, 1100.0];
        frame_selected_pose(&mut cam, scene_center, 10000.0,
            b, 260.0, identity, [0.0;3]);
        let eye_b = editor_camera_eye(cam, scene_center, 10000.0);
        let vec_b = [0,1,2].map(|k|eye_b[k]-b[k]);
        for k in 0..3 { assert!((vec_b[k]/2.0 - vec_a[k]).abs() < 0.05); }
        assert!(((vec_b.iter().map(|v|v*v).sum::<f32>().sqrt()/dist_a)-2.0).abs()<0.0001);
        // Quarter turn of the ORIGINAL PS1 object rotates the camera's local
        // approach, rather than using a scene-global yaw.
        let q = [0.0, std::f32::consts::FRAC_1_SQRT_2, 0.0,
            std::f32::consts::FRAC_1_SQRT_2];
        frame_selected_pose(&mut cam, scene_center, 10000.0,
            b, 130.0, q, [0.0;3]);
        let eye_native = editor_camera_eye(cam, scene_center, 10000.0);
        let vec_native: [f32;3] = [0,1,2].map(|k|eye_native[k]-b[k]);
        assert!((vec_native[0]-vec_a[2]).abs()<0.05);
        assert!((vec_native[2]+vec_a[0]).abs()<0.05);
        assert!((vec_native[1]-vec_a[1]).abs()<0.05);
        // Editor rotation is applied on top of baked native transform.
        frame_selected_pose(&mut cam, scene_center, 10000.0,
            b, 130.0, identity, [0.0, std::f32::consts::FRAC_PI_2, 0.0]);
        let eye_editor = editor_camera_eye(cam, scene_center, 10000.0);
        let vec_editor: [f32;3] = [0,1,2].map(|k|eye_editor[k]-b[k]);
        for k in 0..3 {assert!((vec_editor[k]-vec_native[k]).abs() < 0.05);}
    }

    #[test]
    fn frame_selected_stays_upright_across_native_and_editor_yaws() {
        let scene_center = [0.0, 0.0, 0.0];
        let selected = [4000.0, 240.0, -1000.0];
        for angle in [0.0_f32, 0.35, std::f32::consts::FRAC_PI_2,
                      std::f32::consts::PI, 4.7] {
            let (sin_half, cos_half) = (angle * 0.5).sin_cos();
            let quat = [0.0, sin_half, 0.0, cos_half];
            for editor_y in [0.0_f32, std::f32::consts::FRAC_PI_2] {
                let mut cam = Camera::default();
                let editor_rotation = [0.0, editor_y, 0.0];
                frame_selected_pose(&mut cam, scene_center, 12000.0,
                    selected, 300.0, quat, editor_rotation);
                let eye = editor_camera_eye(cam, scene_center, 12000.0);
                let expected = rotate_frame_euler(
                    rotate_frame_quat(FRAME_REFERENCE_EYE_LOCAL, quat), editor_rotation);
                let d: [f32;3] = [0,1,2].map(|k| eye[k] - selected[k]);
                let dl = d.iter().map(|v| v*v).sum::<f32>().sqrt();
                let el = expected.iter().map(|v| v*v).sum::<f32>().sqrt();
                for k in 0..3 {
                    assert!((d[k] / dl - expected[k] / el).abs() < 0.0002,
                        "angle {angle} editor_y {editor_y} axis {k}: camera eye moved");
                }
                let screen_up = editor_world_to_camera(cam, [0.0, 1.0, 0.0]);
                assert!(screen_up[1] < 0.0,
                    "angle {angle} editor_y {editor_y}: world-up is inverted");
            }
        }
    }

    #[test]
    fn switching_selection_never_recenters_until_drag() {
        let mut cam = Camera::default();
        let center = [500.0, 0.0, -340.0];
        let radius = 300.0;
        let before = cam;
        editor_orbit_around_piece(&mut cam, center, radius, [-40.0, 33.0, 80.0], 0.0, 0.0);
        editor_orbit_around_piece(&mut cam, center, radius, [90.0, -21.0, 460.0], 0.0, 0.0);
        assert_eq!(before, cam);
    }

    #[test]
    fn look_in_place_preserves_eye_through_unlimited_pitch() {
        let mut cam = Camera::default();
        cam.target_offset = [44.0, 33.0, -100.0];
        let original_eye = editor_camera_eye(cam, [500.0, 70.0, 350.0], 900.0);
        for _ in 0..100 {
            editor_look_in_place(&mut cam, 900.0, 0.03, -0.04);
            assert!((0.0..std::f32::consts::TAU).contains(&cam.pitch));
        }
        near(original_eye, editor_camera_eye(cam, [500.0, 70.0, 350.0], 900.0));
    }
}


#[cfg(test)]
mod upright_defaults_regression_tests {
    use super::*;

    fn forward(cam: Camera) -> [f32; 3] {
        let (sy, cy) = cam.yaw.sin_cos();
        let (sp, cp) = cam.pitch.sin_cos();
        [-sy * cp, sp, cy * cp]
    }

    #[test]
    fn cell_open_and_frame_all_use_upright_roll_without_moving_eye() {
        let raw = Camera::default();
        let mut viewport = Viewport::new(Shading::Textured);
        let corrected = viewport.cam;
        for (a, b) in forward(raw).into_iter().zip(forward(corrected)) {
            assert!((a - b).abs() < 1e-5, "camera look vector must not move");
        }
        assert!(corrected.pitch.cos() < 0.0, "screen-up must remain upright");
        assert_eq!(corrected.zoom, raw.zoom);
        assert_eq!(corrected.target_offset, raw.target_offset);
        viewport.cam.pitch = 0.2;
        viewport.frame_all();
        assert_eq!(viewport.cam, corrected, "Frame All must restore upright camera");
    }

    #[test]
    fn preview_drag_pitch_limits_stay_on_upright_euler_branch() {
        let cam = upright_default_camera();
        assert!(cam.pitch > std::f32::consts::FRAC_PI_2 + 0.02);
        assert!(cam.pitch < std::f32::consts::PI * 1.5 - 0.02);
    }
}
