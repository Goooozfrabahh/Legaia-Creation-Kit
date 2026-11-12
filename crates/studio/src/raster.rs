//! A small CPU rasteriser for the TMD preview: orbit camera, z-buffer,
//! textured triangles sampled straight out of a PSX VRAM image.
//!
//! Studio runs on egui's OpenGL backend, so it cannot reuse the wgpu
//! renderer; a preview at panel resolution is cheap enough on the CPU. The
//! mesh comes from `legaia_tmd::mesh::tmd_to_vram_mesh_field_hybrid`, and the
//! texel lookup is the page / CLUT addressing the site viewer's shader (and
//! `legaia_actor_formats::scene_gltf`) uses. UVs interpolate affinely, as the
//! PSX GPU does.

use legaia_tim::Vram;

/// A triangulated TMD ready to draw.
#[derive(Clone)]
pub struct Model {
    /// Drawn positions: the rest pose, or the last [`Model::pose`].
    pub positions: Vec<[f32; 3]>,
    /// TMD-local rest positions.
    pub rest: Vec<[f32; 3]>,
    /// The TMD object (= animation part) each vertex belongs to.
    pub object_ids: Vec<u32>,
    pub uvs: Vec<[u8; 2]>,
    pub cba_tsb: Vec<[u16; 2]>,
    /// Texture-modulation colour (`0x80` = neutral) for textured vertices.
    pub modulation: Vec<[u8; 3]>,
    /// Per-vertex fill colour for untextured prims.
    pub fill: Vec<[u8; 3]>,
    pub textured: Vec<u8>,
    pub indices: Vec<u32>,
    pub center: [f32; 3],
    pub radius: f32,
}

impl Model {
    /// Build from a TMD's bytes; `None` when it doesn't parse or has no
    /// triangles.
    pub fn from_tmd(bytes: &[u8]) -> Option<Self> {
        let tmd = legaia_tmd::parse(bytes).ok()?;
        let (mesh, obj, shading) = legaia_tmd::mesh::tmd_to_vram_mesh_field_hybrid(&tmd, bytes);
        Self::from_mesh(mesh, obj, Some(shading))
    }

    /// Build from a VRAM mesh the actor decoders produced (every prim
    /// textured unless `shading` says otherwise).
    pub fn from_mesh(
        mesh: legaia_tmd::mesh::VramMesh,
        object_ids: Vec<u32>,
        shading: Option<legaia_tmd::mesh::VertexShading>,
    ) -> Option<Self> {
        if mesh.indices.len() < 3 || mesh.positions.is_empty() {
            return None;
        }
        let n = mesh.positions.len();
        let pad = |v: Vec<[u8; 3]>, d: [u8; 3]| {
            let mut v = v;
            v.resize(n, d);
            v
        };
        let (fill, mut textured) = match shading {
            Some(s) => (s.colors, s.textured),
            None => (Vec::new(), vec![1; n]),
        };
        textured.resize(n, 1);
        let mut object_ids = object_ids;
        object_ids.resize(n, 0);
        let mut m = Self {
            modulation: pad(mesh.colors, [0x80; 3]),
            fill: pad(fill, [0xC0; 3]),
            textured,
            rest: mesh.positions.clone(),
            positions: mesh.positions,
            object_ids,
            uvs: mesh.uvs,
            cba_tsb: mesh.cba_tsb,
            indices: mesh.indices,
            center: [0.0; 3],
            radius: 1.0,
        };
        m.refit();
        Some(m)
    }

    /// Re-centre the camera target on the current positions.
    pub fn refit(&mut self) {
        let mut lo = [f32::MAX; 3];
        let mut hi = [f32::MIN; 3];
        for p in &self.positions {
            for k in 0..3 {
                lo[k] = lo[k].min(p[k]);
                hi[k] = hi[k].max(p[k]);
            }
        }
        let c = [
            (lo[0] + hi[0]) * 0.5,
            (lo[1] + hi[1]) * 0.5,
            (lo[2] + hi[2]) * 0.5,
        ];
        self.center = c;
        self.radius = self
            .positions
            .iter()
            .map(|p| {
                let d = [p[0] - c[0], p[1] - c[1], p[2] - c[2]];
                (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt()
            })
            .fold(1.0f32, f32::max);
    }

    /// Pose every object by its part's rigid transform: the PSX
    /// `Rz * Ry * Rx` rotation about the object origin, then the
    /// translation - the per-object assembly `FUN_8004998C` applies and
    /// `legaia_tmd::mesh::tmd_to_vram_mesh_posed_rot` mirrors. Objects past
    /// the end of `parts` stay at rest.
    pub fn pose(&mut self, parts: &[legaia_actor_formats::monster_archive::PartPose]) {
        const A2R: f32 = std::f32::consts::TAU / 4096.0;
        let trig: Vec<([f32; 3], [f32; 6])> = parts
            .iter()
            .map(|p| {
                let (sx, cx) = (p.rx as f32 * A2R).sin_cos();
                let (sy, cy) = (p.ry as f32 * A2R).sin_cos();
                let (sz, cz) = (p.rz as f32 * A2R).sin_cos();
                (
                    [p.tx as f32, p.ty as f32, p.tz as f32],
                    [cx, sx, cy, sy, cz, sz],
                )
            })
            .collect();
        for (i, v) in self.rest.iter().enumerate() {
            let o = self.object_ids.get(i).copied().unwrap_or(0) as usize;
            self.positions[i] = match trig.get(o) {
                Some((t, [cx, sx, cy, sy, cz, sz])) => {
                    let r = legaia_tmd::mesh::rot_zyx(*v, *cx, *sx, *cy, *sy, *cz, *sz);
                    [r[0] + t[0], r[1] + t[1], r[2] + t[2]]
                }
                None => *v,
            };
        }
    }

    pub fn triangles(&self) -> usize {
        self.indices.len() / 3
    }

    /// How many textured triangles `vram` can draw: their CLUT entries and
    /// the texels under their UV box are populated
    /// (`legaia_tim::Vram::prim_texture_status`, the test the asset viewer
    /// drops prims by). `(covered, textured)`.
    pub fn texture_coverage(&self, vram: &legaia_tim::Vram) -> (usize, usize) {
        let mut covered = 0;
        let mut textured = 0;
        for tri in self.indices.as_chunks::<3>().0 {
            let v = tri.map(|i| i as usize);
            if v.iter().any(|&i| self.textured.get(i) != Some(&1)) {
                continue;
            }
            textured += 1;
            let [cba, tsb] = self.cba_tsb[v[0]];
            let uvs = v.map(|i| (self.uvs[i][0], self.uvs[i][1]));
            if vram.prim_texture_status(cba, tsb, &uvs).ok() {
                covered += 1;
            }
        }
        (covered, textured)
    }

    /// Share of the mesh's surface whose baked colour words are near black
    /// (`legaia_tmd::mesh::dark_baked_share` over the rest pose, with the
    /// modulation of a textured prim and the fill of an untextured one).
    /// The dark-sky shell of `town01` (PROT 6) is the shape this reads: its
    /// sky dome is untextured gouraud authored `(0, 0, 0)` at every corner -
    /// few triangles, most of the surface.
    pub fn dark_baked_share(&self) -> f32 {
        let colors: Vec<[u8; 3]> = (0..self.rest.len())
            .map(|i| {
                if self.textured.get(i) == Some(&1) {
                    self.modulation.get(i)
                } else {
                    self.fill.get(i)
                }
                .copied()
                .unwrap_or([0x80; 3])
            })
            .collect();
        legaia_tmd::mesh::dark_baked_share(&self.rest, &colors, &self.indices)
    }
}

/// The lowest colour [`Shading::Lit`] shades a prim from, per channel, so a
/// prim baked near black still shows its shape under the viewer light.
const LIT_FLOOR: f32 = 72.0;

/// Orbit camera around the model's bounding-sphere centre.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Camera {
    /// Radians about the vertical axis.
    pub yaw: f32,
    /// Radians up/down, clamped to just short of the poles.
    pub pitch: f32,
    /// Distance multiplier (1 = the sphere fills the view).
    pub zoom: f32,
    /// Editor-only orbit/pan pivot offset from Model::center, in world units.
    pub target_offset: [f32; 3],
}

impl Default for Camera {
    /// A three-quarter view of the model's front with some margin.
    /// Character and monster meshes face `+Z` and the camera sits on the
    /// `-Z` side at yaw 0, so the half turn puts the face toward the viewer.
    fn default() -> Self {
        Self {
            yaw: std::f32::consts::PI + 0.6,
            pitch: -0.35,
            zoom: 1.3,
            target_offset: [0.0; 3],
        }
    }
}

/// How pixels get their colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shading {
    /// Retail: `texel * colour / 128` for textured prims, the baked fill
    /// for untextured ones; no light source.
    Textured,
    /// No textures: every prim in its colour under a fixed viewer light,
    /// so geometry reads when the entry carries no textures of its own.
    /// Colours are floored at a mid-dark grey so a prim baked black (a
    /// dark-sky shell's dome) still shows its shape. A viewer aid, not a
    /// claim about retail.
    Lit,
}

fn bgr555(c: u16) -> [u8; 3] {
    let e = |v: u16| ((v & 0x1F) as u32 * 255 / 31) as u8;
    [e(c), e(c >> 5), e(c >> 10)]
}

/// The 16-bit VRAM word a textured prim samples at page-local `(u, v)`.
fn texel(vram: &Vram, cba: u16, tsb: u16, u: usize, v: usize) -> u16 {
    let tpage_x = ((tsb & 0xF) as usize) * 64;
    let tpage_y = (((tsb >> 4) & 1) as usize) * 256;
    let clut_x = ((cba & 0x3F) as usize) * 16;
    let clut_y = ((cba >> 6) & 0x1FF) as usize;
    let (u, v) = (u & 0xFF, v & 0xFF);
    match (tsb >> 7) & 3 {
        0 => {
            let w = vram.pixel((tpage_x + (u >> 2)) & 1023, (tpage_y + v) & 511);
            let idx = (w >> ((u & 3) * 4)) & 0xF;
            vram.pixel((clut_x + idx as usize) & 1023, clut_y & 511)
        }
        1 => {
            let w = vram.pixel((tpage_x + (u >> 1)) & 1023, (tpage_y + v) & 511);
            let idx = (w >> ((u & 1) * 8)) & 0xFF;
            vram.pixel((clut_x + idx as usize) & 1023, clut_y & 511)
        }
        _ => vram.pixel((tpage_x + u) & 1023, (tpage_y + v) & 511),
    }
}

/// Render `model` into a `w * h` RGBA8 buffer.
pub fn render(
    model: &Model,
    vram: &Vram,
    cam: Camera,
    shading: Shading,
    w: usize,
    h: usize,
) -> Vec<u8> {
    render_highlight(model, vram, cam, shading, w, h, None, true)
}

/// The identical rasterizer with an optional world-instance ID tint.
/// Only World Editor supplies IDs; normal actor/asset previews are unchanged.
pub fn render_highlight(
    model: &Model,
    vram: &Vram,
    cam: Camera,
    shading: Shading,
    w: usize,
    h: usize,
    selected: Option<u32>,
    show_ground: bool,
) -> Vec<u8> {
    render_highlight_with_sources(model,vram,cam,shading,w,h,selected,show_ground,None)
}

/// Editor-only multiple-original VRAM render. All geometry shares one depth buffer;
/// source materials are chosen by the original object ID, not a fabricated atlas.
pub fn render_highlight_with_sources(
    model: &Model,
    vram: &Vram,
    cam: Camera,
    shading: Shading,
    w: usize,
    h: usize,
    selected: Option<u32>,
    show_ground: bool,
    donor_vram: Option<&std::collections::HashMap<u32,std::sync::Arc<Vram>>>,
) -> Vec<u8> {
    render_highlights_with_sources(model,vram,cam,shading,w,h,selected.as_slice(),show_ground,donor_vram)
}

/// Same texture/depth path with independent highlighted object IDs.
// LCK_R54_VISIBLE_PICK
// Retain the original color-only API for existing callers.
pub fn render_highlights_with_sources(model:&Model,vram:&Vram,cam:Camera,shading:Shading,
    w:usize,h:usize,selected:&[u32],show_ground:bool,
    donor_vram:Option<&std::collections::HashMap<u32,std::sync::Arc<Vram>>>) -> Vec<u8> {
    render_highlights_with_sources_and_ids(
        model,vram,cam,shading,w,h,selected,show_ground,donor_vram
    ).0
}

// The visible ID is written only when the same pixel passes
// texture transparency and depth testing as the color renderer.
// LCK_R609_PER_PIXEL_HEIGHT
// The overlay uses the interpolated world-space Y at every raster
// pixel. It does not alter native MAP data, textures or pick IDs.
#[derive(Clone,Debug,PartialEq)]
pub struct HeightOverlay {
    pub min_y:f32,
    pub max_y:f32,
    pub contour_step:f32,
    pub native_vertex_limit:usize,
    pub copied_start:usize,
    pub copied_end:usize,
    pub imported_floor_ids:Vec<u32>,
}

const R609_COLORS:[[f32;3];7]=[
    [141.0,69.0,193.0],
    [62.0,98.0,218.0],
    [38.0,170.0,198.0],
    [54.0,180.0,110.0],
    [214.0,197.0,52.0],
    [240.0,146.0,49.0],
    [225.0,63.0,67.0],
];

fn r609_surface_color(
    height:f32,
    overlay:&HeightOverlay
)->[f32;3] {
    let range=(overlay.max_y-overlay.min_y).max(0.001);
    let t=((height-overlay.min_y)/range)
        .clamp(0.0,1.0)*6.0;

    let index=(t.floor() as usize).min(5);
    let factor=if t>=6.0 {1.0}else{t-index as f32};

    let mut rgb=[0.0;3];

    for channel in 0..3 {
        rgb[channel]=R609_COLORS[index][channel]
            +(R609_COLORS[index+1][channel]
                -R609_COLORS[index][channel])*factor;
    }

    // Contour lines remain visible even when a scene's total
    // elevation range is much larger than one ramp.
    let step=overlay.contour_step.max(1.0);

    let phase=height.rem_euclid(step)/step;

    if phase<0.075 {
        for channel in 0..3 {
            rgb[channel]*=0.53;
        }
    }

    rgb
}

// LCK_R610_EVEN_SLOPE_BANDS
// Sloped surfaces use seven equally spaced elevation bands.
// Flat surfaces continue to use the absolute scene palette.
// Dark contour lines still use actual world-space Y.
fn r610_even_slope_color(
    height:f32,
    low:f32,
    high:f32,
    contour_step:f32,
)->[f32;3]{
    let span=(high-low).max(0.001);
    let normalized=((height-low)/span).clamp(0.0,1.0);

    // Seven equal intervals, not six point-to-point intervals.
    let scaled=(normalized*7.0).min(6.999999);
    let index=(scaled.floor() as usize).min(6);
    let progress=scaled-index as f32;

    // Each color occupies an equal-height zone.
    // The last 35% transitions smoothly to the next color.
    let fade=((progress-0.65)/0.35).clamp(0.0,1.0);
    let fade=fade*fade*(3.0-2.0*fade);
    let next=(index+1).min(6);

    let mut rgb=[0.0f32;3];

    for c in 0..3 {
        rgb[c]=R609_COLORS[index][c]*(1.0-fade)
            +R609_COLORS[next][c]*fade;
    }

    // LCK_R611A_SMOOTH_GRADIENT
    // Keep a continuous terrain-height gradient with no contour breaks.
    let _=contour_step;
    rgb
}
// LCK_R611_GLOBAL_HEIGHT
// Every floor uses ONE scene-wide absolute elevation range.
// No per-tile or per-object rainbow resets.
// The height overlay is based on world-space Y, independent of
// camera angle, pan and zoom.
fn r611_absolute_height_color(
    y:f32,
    overlay:&HeightOverlay,
)->[f32;3] {
    r610_even_slope_color(
        y,
        overlay.min_y,
        overlay.max_y,
        overlay.contour_step
    )
}

#[cfg(test)]
mod lck_r611_global_height_tests {
    use super::*;

    fn test_overlay()->HeightOverlay {
        HeightOverlay{
            min_y:0.0,
            max_y:140.0,
            contour_step:16.0,
            native_vertex_limit:4,
            copied_start:4,
            copied_end:4,
            imported_floor_ids:Vec::new(),
        }
    }

    #[test]
    fn seven_equal_scene_height_intervals_have_seven_colors(){
        let overlay=test_overlay();

        for band in 0..7usize {
            // Each band occupies 20 native height units.
            // Sample its midpoint, away from contour lines.
            let y=(band as f32+0.5)*20.0;

            let color=r611_absolute_height_color(y,&overlay);

            for channel in 0..3 {
                assert!(
                    (color[channel]-R609_COLORS[band][channel])
                        .abs()<0.01,
                    "Band {} channel {} has incorrect color",
                    band,channel
                );
            }
        }
    }

    #[test]
    fn equal_world_height_always_has_equal_color(){
        let overlay=test_overlay();

        let first=r611_absolute_height_color(54.0,&overlay);
        let second=r611_absolute_height_color(54.0,&overlay);

        assert_eq!(first,second);
    }

    #[test]
    fn different_absolute_heights_do_not_restart_rainbow(){
        let overlay=test_overlay();

        let low=r611_absolute_height_color(10.0,&overlay);
        let high=r611_absolute_height_color(130.0,&overlay);

        assert_ne!(low,high);
    }

    #[test]
    fn changing_height_changes_color_with_fixed_scene_range(){
        let overlay=test_overlay();

        let before=r611_absolute_height_color(25.0,&overlay);
        let after=r611_absolute_height_color(65.0,&overlay);

        assert_ne!(before,after);
    }
}
pub fn render_highlights_with_sources_and_ids(
    model:&Model,vram:&Vram,cam:Camera,shading:Shading,
    w:usize,h:usize,selected:&[u32],show_ground:bool,
    donor_vram:Option<&std::collections::HashMap<u32,std::sync::Arc<Vram>>>
) -> (Vec<u8>,Vec<u32>) {
    render_highlights_with_sources_and_ids_r609(
        model,vram,cam,shading,w,h,selected,show_ground,
        donor_vram,None
    )
}

pub fn render_highlights_with_sources_and_ids_r609(
    model:&Model,vram:&Vram,cam:Camera,shading:Shading,
    w:usize,h:usize,selected:&[u32],show_ground:bool,
    donor_vram:Option<&std::collections::HashMap<u32,std::sync::Arc<Vram>>>,
    height_overlay:Option<&HeightOverlay>
) -> (Vec<u8>,Vec<u32>) {
    let mut rgba = vec![0u8; w * h * 4];
    let mut visible_ids = vec![u32::MAX; w * h];
    for px in rgba.as_chunks_mut::<4>().0 {
        px.copy_from_slice(&[30, 32, 36, 255]);
    }
    if w == 0 || h == 0 {
        return (rgba,visible_ids);
    }
    let mut depth = vec![f32::INFINITY; w * h];


    let (sy, cy) = cam.yaw.sin_cos();
    let (sp, cp) = cam.pitch.sin_cos();
    let fov = 45f32.to_radians();
    let f = (h as f32 * 0.5) / (fov * 0.5).tan();
    let dist = model.radius / (fov * 0.5).sin() * cam.zoom.max(0.0000001);
    let near = (dist * 0.003).max(0.0001);

    let project = |p: [f32; 3]| -> [f32; 3] {
        let x = p[0] - (model.center[0] + cam.target_offset[0]);
        let y = p[1] - (model.center[1] + cam.target_offset[1]);
        let z = p[2] - (model.center[2] + cam.target_offset[2]);
        // Yaw about Y, then pitch about X.
        let x1 = x * cy + z * sy;
        let z1 = -x * sy + z * cy;
        let y2 = y * cp - z1 * sp;
        let z2 = y * sp + z1 * cp + dist;
        [
            w as f32 * 0.5 + f * x1 / z2,
            h as f32 * 0.5 + f * y2 / z2,
            z2,
        ]
    };
    let screen: Vec<[f32; 3]> = model.positions.iter().map(|&p| project(p)).collect();
    // Viewer light, from the upper left in view space.
    let light = {
        let l = [-0.4f32, -0.7, -0.6];
        let n = (l[0] * l[0] + l[1] * l[1] + l[2] * l[2]).sqrt();
        [l[0] / n, l[1] / n, l[2] / n]
    };
    let view_normal = |n: [f32; 3]| -> [f32; 3] {
        let x1 = n[0] * cy + n[2] * sy;
        let z1 = -n[0] * sy + n[2] * cy;
        [x1, n[1] * cp - z1 * sp, n[1] * sp + z1 * cp]
    };

    for tri in model.indices.as_chunks::<3>().0 {
        let i = [tri[0] as usize, tri[1] as usize, tri[2] as usize];

        let r609_floor_triangle=height_overlay.is_some_and(|overlay|{
            let native=i.iter().all(|&vertex|
                vertex<overlay.native_vertex_limit
            );

            let copied=i.iter().all(|&vertex|
                vertex>=overlay.copied_start
                    && vertex<overlay.copied_end
            );

            let imported=model.object_ids.get(i[0])
                .is_some_and(|id|
                    overlay.imported_floor_ids
                        .binary_search(id).is_ok()
                );

            native||copied||imported
        });


        if !show_ground && model.object_ids.get(i[0]) == Some(&u32::MAX) {
            continue;
        }
        if i.iter().any(|&k| k >= screen.len()) {
            continue;
        }
        let s = [screen[i[0]], screen[i[1]], screen[i[2]]];
        if s.iter().any(|p| p[2] < near) {
            continue;
        }
        let area =
            (s[1][0] - s[0][0]) * (s[2][1] - s[0][1]) - (s[2][0] - s[0][0]) * (s[1][1] - s[0][1]);
        if area.abs() < 1e-6 {
            continue;
        }
        let min_x = s
            .iter()
            .map(|p| p[0])
            .fold(f32::MAX, f32::min)
            .floor()
            .max(0.0) as usize;
        let max_x = s
            .iter()
            .map(|p| p[0])
            .fold(f32::MIN, f32::max)
            .ceil()
            .min(w as f32 - 1.0);
        let min_y = s
            .iter()
            .map(|p| p[1])
            .fold(f32::MAX, f32::min)
            .floor()
            .max(0.0) as usize;
        let max_y = s
            .iter()
            .map(|p| p[1])
            .fold(f32::MIN, f32::max)
            .ceil()
            .min(h as f32 - 1.0);
        if max_x < 0.0 || max_y < 0.0 {
            continue;
        }
        let (max_x, max_y) = (max_x as usize, max_y as usize);

        let tex =
            model.textured.get(i[0]).copied().unwrap_or(0) != 0 && shading == Shading::Textured;
        let material_vram = donor_vram.and_then(|sources|
            model.object_ids.get(i[0]).and_then(|id|sources.get(id)))
            .map(|v|v.as_ref()).unwrap_or(vram);
        let highlighted = model.object_ids.get(i[0]).is_some_and(|id|selected.contains(id));
        let lit = if shading == Shading::Lit {
            // Face normal of the drawn (posed) triangle, in view space.
            let p = [
                model.positions[i[0]],
                model.positions[i[1]],
                model.positions[i[2]],
            ];
            let a = [p[1][0] - p[0][0], p[1][1] - p[0][1], p[1][2] - p[0][2]];
            let b = [p[2][0] - p[0][0], p[2][1] - p[0][1], p[2][2] - p[0][2]];
            let n = [
                a[1] * b[2] - a[2] * b[1],
                a[2] * b[0] - a[0] * b[2],
                a[0] * b[1] - a[1] * b[0],
            ];
            let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt().max(1e-6);
            let n = view_normal([n[0] / len, n[1] / len, n[2] / len]);
            let d = (n[0] * light[0] + n[1] * light[1] + n[2] * light[2]).abs();
            0.35 + 0.65 * d
        } else {
            1.0
        };
        let [cba, tsb] = model.cba_tsb.get(i[0]).copied().unwrap_or([0, 0]);
        // Lit mode: a textured prim has no fill of its own, so it shows its
        // modulation colour (neutral 0x80, lifted to read as mid grey).
        let (lit_src, boost) = if model.textured.get(i[0]).copied().unwrap_or(0) != 0 {
            (&model.modulation, 1.5)
        } else {
            (&model.fill, 1.0)
        };

        for y in min_y..=max_y {
            let py = y as f32 + 0.5;
            for x in min_x..=max_x {
                let px = x as f32 + 0.5;
                let w0 = ((s[1][0] - px) * (s[2][1] - py) - (s[2][0] - px) * (s[1][1] - py)) / area;
                let w1 = ((s[2][0] - px) * (s[0][1] - py) - (s[0][0] - px) * (s[2][1] - py)) / area;
                let w2 = 1.0 - w0 - w1;
                if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                    continue;
                }
                let z = w0 * s[0][2] + w1 * s[1][2] + w2 * s[2][2];
                let di = y * w + x;
                // Stable selection depth bias for coplanar
                // editor floor copies. Native geometry unchanged.
                let display_depth=if highlighted {
                    z-(z.abs()*0.00001).max(0.001)
                }else{z};

                if display_depth >= depth[di] {
                    continue;
                }
                let lerp3 = |c: &[[u8; 3]]| -> [f32; 3] {
                    let a = c.get(i[0]).copied().unwrap_or([128; 3]);
                    let b = c.get(i[1]).copied().unwrap_or([128; 3]);
                    let d = c.get(i[2]).copied().unwrap_or([128; 3]);
                    [0, 1, 2].map(|k| w0 * a[k] as f32 + w1 * b[k] as f32 + w2 * d[k] as f32)
                };
                let rgb: [f32; 3] = if tex {
                    let uv = |k: usize| model.uvs.get(i[k]).copied().unwrap_or([0, 0]);
                    let u = w0 * uv(0)[0] as f32 + w1 * uv(1)[0] as f32 + w2 * uv(2)[0] as f32;
                    let v = w0 * uv(0)[1] as f32 + w1 * uv(1)[1] as f32 + w2 * uv(2)[1] as f32;
                    let t = texel(material_vram, cba, tsb & 0x1FF, u as usize, v as usize);
                    if t == 0 {
                        // PSX transparent texel.
                        continue;
                    }
                    let c = bgr555(t);
                    let m = lerp3(&model.modulation);
                    [0, 1, 2].map(|k| c[k] as f32 * m[k] / 128.0)
                } else if shading == Shading::Lit {
                    let c = lerp3(lit_src);
                    [0, 1, 2].map(|k| (c[k] * boost).max(LIT_FLOOR) * lit)
                } else {
                    lerp3(&model.fill)
                };
                depth[di] = display_depth;
                // LCK_R54_VISIBLE_PICK
                visible_ids[di] =
                    model.object_ids.get(i[0]).copied().unwrap_or(u32::MAX);
                let rgb = if highlighted {
                    // Gold instance highlight: keep enough of the underlying
                    // original texture to recognize the selected mesh.
                    [rgb[0] * 0.50 + 255.0 * 0.50,
                     rgb[1] * 0.50 + 190.0 * 0.50,
                     rgb[2] * 0.50 + 42.0 * 0.50]
                } else { rgb };

                // Pixel elevation uses the SAME barycentric weights
                // used for texture, depth and visible-ID rendering.
                // This resolves slopes inside a single floor tile.
                let rgb=if r609_floor_triangle {
                    if let Some(overlay)=height_overlay {
                        let y=w0*model.positions[i[0]][1]
                            +w1*model.positions[i[1]][1]
                            +w2*model.positions[i[2]][1];

                        // A genuinely sloped surface receives the
                        // full seven-band gradient at equal height
                        // intervals. Flat tiles retain absolute colors.
                        let height_color=r611_absolute_height_color(y,overlay);

                        [0usize,1,2].map(|channel|{
                            let base=rgb[channel]*0.38;
                            let terrain=height_color[channel]*0.62;

                            let selected_accent=if highlighted {
                                [255.0,205.0,75.0][channel]*0.08
                            }else{
                                0.0
                            };

                            (base+terrain)*(if highlighted {0.92}else{1.0})
                                +selected_accent
                        })
                    }else{
                        rgb
                    }
                }else{
                    rgb
                };

                let o = di * 4;
                rgba[o] = rgb[0].clamp(0.0, 255.0) as u8;
                rgba[o + 1] = rgb[1].clamp(0.0, 255.0) as u8;
                rgba[o + 2] = rgb[2].clamp(0.0, 255.0) as u8;
                rgba[o + 3] = 255;
            }
        }
    }
    (rgba,visible_ids)
}

/// Hit-test original world triangles at a viewport pixel; nearest depth wins.
/// IDs are stored in `Model::object_ids` for the editor's assembled world.
/// Synthetic walk-ground is tagged `u32::MAX` and is not selectable here.
// LCK_FOREIGN_FLOOR_PICK_PRIORITY_V1
fn pick_instance_depth(model: &Model, cam: Camera, w: usize, h: usize, pixel: [f32; 2], only: Option<u32>) -> Option<(u32,f32)> {
    if w == 0 || h == 0 { return None; }
    let (sy, cy) = cam.yaw.sin_cos();
    let (sp, cp) = cam.pitch.sin_cos();
    let fov = 45f32.to_radians();
    let f = (h as f32 * 0.5) / (fov * 0.5).tan();
    let dist = model.radius / (fov * 0.5).sin() * cam.zoom.max(0.0000001);
    let near = (dist * 0.003).max(0.0001);
    let projected: Vec<[f32; 3]> = model.positions.iter().map(|p| {
        let x = p[0] - (model.center[0] + cam.target_offset[0]);
        let y = p[1] - (model.center[1] + cam.target_offset[1]);
        let z = p[2] - (model.center[2] + cam.target_offset[2]);
        let x1 = x * cy + z * sy;
        let z1 = -x * sy + z * cy;
        let y2 = y * cp - z1 * sp;
        let z2 = y * sp + z1 * cp + dist;
        [w as f32 * 0.5 + f * x1 / z2, h as f32 * 0.5 + f * y2 / z2, z2]
    }).collect();
    let mut winner = None;
    let mut nearest = f32::INFINITY;
    // LCK_R51_ALIGNMENT
    // The CPU renderer samples integer pixels at +0.5.
    let px = pixel[0].floor() + 0.5;
    let py = pixel[1].floor() + 0.5;
    for tri in model.indices.as_chunks::<3>().0 {
        let i = [tri[0] as usize, tri[1] as usize, tri[2] as usize];
        if i.iter().any(|&k| k >= projected.len()) { continue; }
        let id = model.object_ids.get(i[0]).copied().unwrap_or(u32::MAX);
        if id >= u32::MAX-1 || only.is_some_and(|requested|requested != id) { continue; } // LCK_R49_WORLD_FIXED_GRID
        let s = [projected[i[0]], projected[i[1]], projected[i[2]]];
        if s.iter().any(|p| !p[2].is_finite() || p[2] < near) { continue; }
        let area = (s[1][0] - s[0][0]) * (s[2][1] - s[0][1])
                 - (s[2][0] - s[0][0]) * (s[1][1] - s[0][1]);
        if area.abs() < 1e-6 { continue; }
        let w0 = ((s[1][0] - px) * (s[2][1] - py) - (s[2][0] - px) * (s[1][1] - py)) / area;
        let w1 = ((s[2][0] - px) * (s[0][1] - py) - (s[0][0] - px) * (s[2][1] - py)) / area;
        let w2 = 1.0 - w0 - w1;
        if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 { continue; }
        let z = w0 * s[0][2] + w1 * s[1][2] + w2 * s[2][2];
        if z < nearest { nearest = z; winner = Some(id); }
    }
    winner.map(|id|(id,nearest))
}

/// Preserve ordinary nearest-depth picking.
pub fn pick_instance(model: &Model, cam: Camera, w: usize, h: usize, pixel: [f32; 2]) -> Option<u32> {
    pick_instance_depth(model,cam,w,h,pixel,None).map(|(id,_)|id)
}

/// Prefer the selected surface when it is nearly coplanar with another.
/// Do not select through substantially closer opaque geometry.
pub fn pick_instance_prefer(
    model:&Model,
    cam:Camera,
    w:usize,
    h:usize,
    pixel:[f32;2],
    preferred:Option<u32>
)->Option<u32> {
    let front=pick_instance_depth(model,cam,w,h,pixel,None);
    let selected=preferred.and_then(
        |id|pick_instance_depth(model,cam,w,h,pixel,Some(id))
    );

    match (front,selected) {
        (Some((_,z)),Some((chosen,selected_z)))
            if selected_z <= z + (z.abs()*0.0005).max(0.25)
                => Some(chosen),
        (Some((id,_)),_) => Some(id),
        (None,Some((id,_))) => Some(id),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_model_input_is_rejected() {
        assert!(Model::from_tmd(&[]).is_none());
        assert!(Model::from_tmd(&[0u8; 64]).is_none());
    }

    #[test]
    fn texture_coverage_reads_the_clut_and_the_page_under_the_uvs() {
        // One 4bpp triangle: CLUT at (0, 480), texture page 5 (x 320),
        // UVs 0..=15 (4 VRAM words wide, 16 rows).
        let cba = 480 << 6;
        let tsb = 5;
        let m = Model {
            positions: vec![[0.0; 3]; 3],
            rest: vec![[0.0; 3]; 3],
            object_ids: vec![0; 3],
            uvs: vec![[0, 0], [15, 0], [0, 15]],
            cba_tsb: vec![[cba, tsb]; 3],
            modulation: vec![[0x80; 3]; 3],
            fill: vec![[0; 3]; 3],
            textured: vec![1; 3],
            indices: vec![0, 1, 2],
            center: [0.0; 3],
            radius: 1.0,
        };
        let mut vram = legaia_tim::Vram::new();
        assert_eq!(m.texture_coverage(&vram), (0, 1), "nothing uploaded");
        vram.write_clut_row(0, 480, &[0x11; 32]);
        assert_eq!(m.texture_coverage(&vram), (0, 1), "palette, no texels");
        vram.write_block(320, 0, 4, 16, &[0x22; 4 * 2 * 16]);
        assert_eq!(m.texture_coverage(&vram), (1, 1));
        // An untextured triangle is not counted either way.
        let flat = Model {
            textured: vec![0; 3],
            ..m
        };
        assert_eq!(flat.texture_coverage(&legaia_tim::Vram::new()), (0, 0));
    }

    #[test]
    fn bgr555_expands_to_full_range() {
        assert_eq!(bgr555(0x7FFF), [255, 255, 255]);
        assert_eq!(bgr555(0x001F), [255, 0, 0]);
        assert_eq!(bgr555(0x7C00), [0, 0, 255]);
    }
}


// LCK_R54_VISIBLE_PICK
#[cfg(test)]
mod lck_r54_visible_pick_tests {
    use super::*;

    fn triangle(textured:bool)->Model {
        Model {
            positions:vec![
                [-1.0,-1.0,0.0],
                [ 1.0,-1.0,0.0],
                [-1.0, 1.0,0.0]
            ],
            rest:vec![
                [-1.0,-1.0,0.0],
                [ 1.0,-1.0,0.0],
                [-1.0, 1.0,0.0]
            ],
            object_ids:vec![71337;3],
            uvs:vec![[0,0];3],
            cba_tsb:vec![[0,0];3],
            modulation:vec![[128;3];3],
            fill:vec![[200;3];3],
            textured:vec![if textured {1}else{0};3],
            indices:vec![0,1,2],
            center:[0.0;3],
            radius:1.0
        }
    }

    fn camera()->Camera {
        Camera {
            yaw:0.0,
            pitch:0.0,
            zoom:1.0,
            target_offset:[0.0;3]
        }
    }

    #[test]
    fn visible_surface_has_same_id_as_drawn_pixel() {
        let model=triangle(false);
        let vram=Vram::new();

        let (rgba,ids)=render_highlights_with_sources_and_ids(
            &model,&vram,camera(),Shading::Textured,
            64,64,&[],true,None
        );

        let pixel=31*64+31;

        assert_eq!(ids[pixel],71337);
        assert_eq!(rgba[pixel*4+3],255);

        assert_eq!(
            rgba,
            render_highlights_with_sources(
                &model,&vram,camera(),Shading::Textured,
                64,64,&[],true,None
            )
        );
    }

    #[test]
    fn transparent_texture_cannot_steal_a_click() {
        let model=triangle(true);
        let vram=Vram::new();

        let (_,ids)=render_highlights_with_sources_and_ids(
            &model,&vram,camera(),Shading::Textured,
            64,64,&[],true,None
        );

        assert!(ids.iter().all(|&id|id==u32::MAX));
    }
}
// LCK_R609_PER_PIXEL_HEIGHT_TESTS
#[cfg(test)]
mod lck_r609_raster_tests {
    use super::*;

    fn overlay()->HeightOverlay {
        HeightOverlay{
            min_y:0.0,
            max_y:160.0,
            contour_step:16.0,
            native_vertex_limit:4,
            copied_start:10,
            copied_end:14,
            imported_floor_ids:Vec::new(),
        }
    }

    #[test]
    fn ramp_has_distinct_low_middle_and_high_colors(){
        let o=overlay();

        let low=r609_surface_color(20.0,&o);
        let middle=r609_surface_color(80.0,&o);
        let high=r609_surface_color(140.0,&o);

        assert_ne!(low,middle);
        assert_ne!(middle,high);
        assert_ne!(low,high);
    }

    #[test]
    fn every_fractional_height_can_change_gradient(){
        let o=overlay();

        let a=r609_surface_color(54.0,&o);
        let b=r609_surface_color(55.0,&o);

        assert_ne!(a,b);
    }

    #[test]
    fn contour_lines_are_present(){
        let o=overlay();

        let line=r609_surface_color(64.0,&o);
        let nearby=r609_surface_color(68.0,&o);

        assert!(
            line.iter().sum::<f32>()
                <nearby.iter().sum::<f32>()
        );
    }
}
// LCK_R610_EVEN_BANDS_TESTS
#[cfg(test)]
mod lck_r610_tests {
    use super::*;

    #[test]
    fn all_seven_colors_have_equal_height_intervals(){
        for i in 0..7usize {
            let y=(i as f32+0.5)*20.0;
            let rgb=r610_even_slope_color(y,0.0,140.0,16.0);

            for channel in 0..3 {
                assert!(
                    (rgb[channel]-R609_COLORS[i][channel]).abs()<0.1,
                    "Incorrect band {} channel {}",
                    i,channel
                );
            }
        }
    }

    #[test]
    fn slope_gradient_interpolates_between_color_bands(){
        let lower=r610_even_slope_color(15.0,0.0,140.0,16.0);
        let upper=r610_even_slope_color(19.0,0.0,140.0,16.0);
        assert_ne!(lower,upper);
    }

    #[test]
    fn same_elevation_always_has_same_relative_slope_color(){
        let first=r610_even_slope_color(60.0,0.0,140.0,16.0);
        let second=r610_even_slope_color(60.0,0.0,140.0,16.0);
        assert_eq!(first,second);
    }
}
// LCK_R611A_SMOOTH_GRADIENT_TESTS
#[cfg(test)]
mod lck_r611a_smooth_gradient_tests {
    use super::*;

    #[test]
    fn contour_step_no_longer_changes_color() {
        let a=r610_even_slope_color(64.0,0.0,140.0,16.0);
        let b=r610_even_slope_color(64.0,0.0,140.0,32.0);
        assert_eq!(a,b);
    }

    #[test]
    fn nearby_heights_transition_smoothly() {
        let a=r610_even_slope_color(63.9,0.0,140.0,16.0);
        let b=r610_even_slope_color(64.1,0.0,140.0,16.0);

        let delta=(0..3).map(|i|(a[i]-b[i]).abs()).sum::<f32>();
        assert!(delta < 30.0);
    }
}