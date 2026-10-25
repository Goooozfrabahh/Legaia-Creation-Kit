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
    /// Session-only preferred angle for Frame Selected, calibrated by the artist.
    pub world_frame_angle: Option<(f32, f32)>,
    rendered: Option<(Camera, Shading, [usize; 2], u64, Option<u32>, bool, bool)>,
    tex: Option<TextureHandle>,
}

impl Viewport {
    pub fn new(shading: Shading) -> Self {
        Self {
            cam: Camera::default(), shading, version: 0,
            world_mirrored: true,
            world_orbit_anchor: None,
            world_frame_angle: None,
            rendered: None, tex: None,
        }
    }

    pub fn show(&mut self, ui: &mut egui::Ui, model: &Model, vram: &Vram) {
        self.show_limited(ui, model, vram, 1600, 1200);
    }

    /// Existing per-asset preview behavior; left-drag orbits, wheel zooms.
    pub fn show_limited(&mut self, ui: &mut egui::Ui, model: &Model, vram: &Vram, max_w: usize, max_h: usize) {
        let _ = self.show_internal(ui, model, vram, max_w, max_h, false, None, true);
    }

    /// World editor: LMB drag pans, MMB drags orbit in all directions,
    /// Shift+MMB also pans, Ctrl+MMB dollies, wheel zooms, RMB picks.
    /// Returns an instance ID when a native mesh is clicked.
    pub fn show_world_limited(&mut self, ui: &mut egui::Ui, model: &Model, vram: &Vram,
                              max_w: usize, max_h: usize, selected: Option<u32>, show_ground: bool) -> Option<u32> {
        self.show_internal(ui, model, vram, max_w, max_h, true, selected, show_ground)
    }

    /// Save the exact camera direction the artist is currently using.
    /// Subsequent Frame Selected commands reuse this direction for every piece.
    pub fn remember_frame_angle(&mut self) {
        self.world_frame_angle = Some((self.cam.yaw, self.cam.pitch));
    }

    pub fn frame_instance(&mut self, model: &Model, point: [f32; 3], radius: f32) {
        // Restore the editor's proven upright default (rather than the v0.4.5.3
        // below-ground angle). The artist can override it by pressing Set F view.
        editor_frame_front_direction(&mut self.cam);
        if let Some((yaw, pitch)) = self.world_frame_angle {
            self.cam.yaw = yaw;
            self.cam.pitch = pitch;
        }
        self.cam.target_offset = [0, 1, 2].map(|k| point[k] - model.center[k]);
        // UNCHANGED framing distance: retain v0.4.5.2's object-radius zoom.
        self.cam.zoom = (radius / model.radius.max(0.01) * 1.8).clamp(0.00015, 50.0);
    }

    pub fn frame_all(&mut self) { self.cam = Camera::default(); }

    fn show_internal(&mut self, ui: &mut egui::Ui, model: &Model, vram: &Vram,
                     max_w: usize, max_h: usize, world: bool, selected: Option<u32>, show_ground: bool) -> Option<u32> {
        let avail = ui.available_size();
        let (rect, resp) = ui.allocate_exact_size(avail, egui::Sense::click_and_drag());
        let mut picked = None;
        let mirror = world && self.world_mirrored;
        if world {
            let shift = ui.input(|i| i.modifiers.shift);
            let ctrl = ui.input(|i| i.modifiers.ctrl);
            let alt = ui.input(|i| i.modifiers.alt);
            let middle = resp.dragged_by(PointerButton::Middle);
            let left = resp.dragged_by(PointerButton::Primary);
            let delta = ui.input(|i| i.pointer.delta());
            let drag_x = if mirror { -delta.x } else { delta.x };
            if (middle && shift) || (left && !alt && !shift) {
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
            } else if middle || (left && alt) {
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
            let d = resp.drag_delta();
            self.cam.yaw += d.x * 0.01;
            self.cam.pitch = (self.cam.pitch - d.y * 0.01).clamp(-1.5, 1.5);
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
        let ppp = ui.ctx().pixels_per_point();
        let px = [
            ((rect.width() * ppp) as usize).clamp(16, max_w.max(16)),
            ((rect.height() * ppp) as usize).clamp(16, max_h.max(16)),
        ];
        let highlight = if world { selected } else { None };
        let state = (self.cam, self.shading, px, self.version, highlight, show_ground, mirror);
        if self.rendered != Some(state) || self.tex.is_none() {
            let mut rgba = if world {
                raster::render_highlight(model, vram, self.cam, self.shading, px[0], px[1], highlight, show_ground)
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
        }
        if let Some(tex) = &self.tex {
            ui.painter().image(tex.id(), rect,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                Color32::WHITE);
        }
        if world && resp.clicked_by(PointerButton::Primary) {
            if let Some(pos) = resp.interact_pointer_pos() {
                let screen_x = (pos.x - rect.left()) * px[0] as f32 / rect.width().max(1.0);
                let screen_point = [
                    if mirror { (px[0] as f32 - 1.0 - screen_x).max(0.0) } else { screen_x },
                    (pos.y - rect.top()) * px[1] as f32 / rect.height().max(1.0),
                ];
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
fn editor_frame_front_direction(cam: &mut Camera) {
    // Use the original studio camera's known-upright, front three-quarter
    // angle; it was designed against the renderer's actual yaw/pitch basis.
    // A negative pitch places the camera ABOVE the object (positive Y-up).
    // A custom reference angle can override this in frame_instance.
    let default = Camera::default();
    cam.yaw = default.yaw;
    cam.pitch = default.pitch;
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
    fn framing_orients_toward_native_model_front_not_top_down() {
        let mut cam = Camera::default();
        cam.pitch = std::f32::consts::FRAC_PI_2;
        cam.yaw = 0.65;
        let zoom = cam.zoom;
        editor_frame_front_direction(&mut cam);
        let forward = editor_camera_forward(cam);
        let expected = Camera::default();
        assert!((cam.yaw - expected.yaw).abs() < 0.000001);
        assert!((cam.pitch - expected.pitch).abs() < 0.000001);
        assert!(forward[2] < -0.7, "Camera should be on the model's front-facing side");
        assert!(forward[1] < -0.1 && forward[1] > -0.5,
            "The view should be mildly elevated, not below-ground or top-down");
        // Since eye = target - d * forward, negative forward.Y means the
        // camera stands ABOVE the selected point.
        assert!(editor_camera_eye(cam, [0.0;3], 100.0)[1] > 0.0);
        assert_eq!(cam.zoom, zoom);
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
