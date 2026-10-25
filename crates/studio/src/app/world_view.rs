//! PS1-native scene geometry assembled for the Studio CPU rasterizer.
//! TMD positions, walk-ground vertex elevations, per-cell texture coordinates,
//! CLUT palettes and tpages all come from the original game scene records.
//! The editor samples those ground tiles from assembled scene VRAM.

use std::collections::HashMap;
use legaia_engine_scene::scene_assembly::AssembledScene;
use crate::raster::Model;

pub(super) struct WorldModel {
    pub model: Option<Model>,
    pub instances: usize,
    pub unavailable: usize,
    pub unique_models: usize,
    pub ground_triangles: usize,
    /// Native textured ground triangles whose referenced TIM data is in VRAM.
    pub ground_vram_covered: usize,
    /// All triangles flagged textured from the MAP's native UV/page/CLUT data.
    pub ground_textured: usize,
    pub draws: Vec<DrawInfo>,
}

pub(super) struct DrawInfo {
    pub id: u32,
    pub terrain: bool,
    pub index: usize,
    pub center: [f32; 3],
    pub radius: f32,
    /// Original viewer-space center and vertex range for precise, reversible placement edits.
    pub original_center: [f32; 3],
    pub offset: [f32; 3],
    pub vertex_start: usize,
    pub vertex_end: usize,
}

impl WorldModel {
    /// Editor-only transform: leaves the original TMD, walk-ground, native
    /// placement records and loaded disc image unchanged. Rest coordinates
    /// are immutable, so repeated drags never introduce cumulative drift.
    pub fn set_offset(&mut self, terrain: bool, index: usize, offset: [f32; 3]) -> bool {
        if !offset.iter().all(|v| v.is_finite() && v.abs() < 1_000_000.0) { return false; }
        let Some(draw) = self.draws.iter_mut().find(|d| d.terrain == terrain && d.index == index) else { return false; };
        let Some(model) = self.model.as_mut() else { return false; };
        if draw.vertex_end > model.positions.len() || draw.vertex_end > model.rest.len() { return false; }
        for i in draw.vertex_start..draw.vertex_end {
            for axis in 0..3 {
                model.positions[i][axis] = model.rest[i][axis] + offset[axis];
            }
        }
        draw.offset = offset;
        draw.center = [0, 1, 2].map(|axis| draw.original_center[axis] + offset[axis]);
        // Deliberately DON'T refit the whole world here: that would cause
        // the camera to shift as the selected object is moved.
        true
    }

    /// Optional center-to-center magnetic anchors; only when near a target.
    pub fn snap_nearby_anchor(&self, key: (bool, usize), desired: [f32; 3], radius: f32) -> [f32; 3] {
        let Some(item) = self.at(key.0, key.1) else { return desired };
        if !radius.is_finite() || radius <= 0.0 { return desired; }
        let center: [f32; 3] = [0, 1, 2].map(|k| item.original_center[k] + desired[k]);
        self.draws.iter().filter(|other| other.id != item.id)
            .filter_map(|other| {
                let distance_sq: f32 = [0, 1, 2].iter().map(|&k| (center[k]-other.center[k]).powi(2)).sum();
                (distance_sq <= radius * radius).then_some((distance_sq, other.center))
            })
            .min_by(|a,b| a.0.total_cmp(&b.0))
            .map(|(_, target)| [0, 1, 2].map(|k| target[k] - item.original_center[k]))
            .unwrap_or(desired)
    }
}

/// Grid is anchored to each piece's ORIGINAL position, never world (0,0,0).
/// That preserves the original authentic placement when offset is zero.
pub(super) fn quantize_offset(delta: [f32; 3], size: f32) -> [f32; 3] {
    if !size.is_finite() || size <= 0.0 { return delta; }
    delta.map(|v| (v / size).round() * size)
}

#[cfg(test)]
mod transform_tests {
    use super::*;
    #[test]
    fn anchored_grid_retains_zero_and_snaps_relative_motion() {
        assert_eq!(quantize_offset([30.0, -31.0, 35.0], 32.0), [32.0, -32.0, 32.0]);
        assert_eq!(quantize_offset([15.0, 0.0, -15.0], 32.0), [0.0; 3]);
        assert_eq!(quantize_offset([1.5, -1.5, 0.0], 1.0), [2.0, -2.0, 0.0]);
    }
    #[test]
    fn a_piece_can_move_and_return_without_touching_other_vertices() {
        let mut m = empty_model();
        m.positions = vec![[1.0, 2.0, 3.0], [4.0, 5.0, 6.0]];
        m.rest = m.positions.clone();
        let mut w = WorldModel {
            model: Some(m), instances: 1, unavailable: 0, unique_models: 1,
            ground_triangles: 0, ground_vram_covered: 0, ground_textured: 0,
            draws: vec![DrawInfo {id:0, terrain:false, index:0,
                center:[1.0,2.0,3.0],radius:1.0,
                original_center:[1.0,2.0,3.0],offset:[0.0;3],
                vertex_start:0, vertex_end:1}],
        };
        assert!(w.set_offset(false, 0, [32.0,-8.0,64.0]));
        assert_eq!(w.model.as_ref().unwrap().positions[0], [33.0,-6.0,67.0]);
        assert_eq!(w.model.as_ref().unwrap().positions[1], [4.0,5.0,6.0]);
        assert!(w.set_offset(false, 0, [0.0;3]));
        assert_eq!(w.model.as_ref().unwrap().positions, w.model.as_ref().unwrap().rest);
    }
}

impl WorldModel {
    pub fn by_id(&self, id: u32) -> Option<&DrawInfo> {
        self.draws.get(id as usize).filter(|d| d.id == id)
    }
    pub fn at(&self, terrain: bool, index: usize) -> Option<&DrawInfo> {
        self.draws.iter().find(|d| d.terrain == terrain && d.index == index)
    }
}

fn empty_model() -> Model {
    Model {
        positions: Vec::new(), rest: Vec::new(), object_ids: Vec::new(),
        uvs: Vec::new(), cba_tsb: Vec::new(), modulation: Vec::new(),
        fill: Vec::new(), textured: Vec::new(), indices: Vec::new(),
        center: [0.0; 3], radius: 1.0,
    }
}

fn bounds(positions: &[[f32; 3]]) -> ([f32; 3], f32) {
    let mut lo = [f32::INFINITY; 3];
    let mut hi = [f32::NEG_INFINITY; 3];
    for p in positions {
        for k in 0..3 { lo[k] = lo[k].min(p[k]); hi[k] = hi[k].max(p[k]); }
    }
    let center = [0, 1, 2].map(|k| (lo[k] + hi[k]) * 0.5);
    let radius = positions.iter().map(|p| {
        let d = [0, 1, 2].map(|k| p[k] - center[k]);
        (d[0]*d[0] + d[1]*d[1] + d[2]*d[2]).sqrt()
    }).fold(1.0_f32, f32::max);
    (center, radius)
}


/// Copy the original game's textured ground quads into the CPU render stream.
///
/// Each walk-ground vertex already has an atlas UV, PS1 [CLUT, tpage], and
/// GP0 modulation colour. The old editor overwrote all of them with zero,
/// then forced `textured = 0`, so the rasterizer could only show grey.
/// The general-purpose rasterizer already knows how to sample scene VRAM.
///
/// If older/malformed scene data lacks the complete native texture attributes,
/// retain the previous diagnostic-grey fallback without misaligning streams.
fn append_native_ground(
    out: &mut Model,
    hf: &legaia_scene_formats::field_objects::WalkHeightfield,
) -> usize {
    let vertices = legaia_engine_scene::field_ground::render_positions(hf);
    let indices = legaia_engine_scene::field_ground::render_indices(hf);
    if vertices.is_empty() || indices.is_empty() { return 0; }
    let Ok(base) = u32::try_from(out.positions.len()) else { return 0; };
    let Ok(count) = u32::try_from(vertices.len()) else { return 0; };
    if base.checked_add(count).is_none() { return 0; }

    let n = vertices.len();
    // The builder emits one UV/page/palette/colour per position. Check all
    // vectors together to preserve the renderer's attribute indexing.
    let textured = hf.uvs.len() == n && hf.cba_tsb.len() == n && hf.colors.len() == n;
    out.positions.extend(vertices.iter().map(|p| [p[0], -p[1], p[2]]));
    // Synthetic floor is visible but cannot be selected as a world object.
    out.object_ids.extend(std::iter::repeat_n(u32::MAX, n));
    if textured {
        out.uvs.extend_from_slice(&hf.uvs);
        out.cba_tsb.extend_from_slice(&hf.cba_tsb);
        out.modulation.extend_from_slice(&hf.colors);
        out.textured.extend(std::iter::repeat_n(1, n));
    } else {
        out.uvs.extend(std::iter::repeat_n([0, 0], n));
        out.cba_tsb.extend(std::iter::repeat_n([0, 0], n));
        out.modulation.extend(std::iter::repeat_n([128, 128, 128], n));
        out.textured.extend(std::iter::repeat_n(0, n));
    }
    // Used only in untextured diagnostic fallback / viewer-light mode.
    out.fill.extend(std::iter::repeat_n([94, 103, 96], n));
    out.indices.extend(indices.iter().map(|&i| base + i));
    indices.len() / 3
}

/// Native TMD instances plus native walk-ground collision heightfield.
pub(super) fn build_world(scene: &AssembledScene) -> WorldModel {
    let mut cache: HashMap<usize, Option<Model>> = HashMap::new();
    let mut out = empty_model();
    let mut instances = 0usize;
    let mut unavailable = 0usize;
    let mut draws = Vec::new();
    let mut ground_triangles = 0usize;

    if let Some(hf) = &scene.ground {
        ground_triangles = append_native_ground(&mut out, hf);
    }
    // The scene's existing VRAM contains every original TIM upload we know.
    // Audit the ground while it is the only mesh, once per scene assembly.
    // This distinguishes missing ground UVs from missing native atlas pages.
    let (ground_vram_covered, ground_textured) = out.texture_coverage(&scene.res.vram);

    // IDs match this draw order and are carried in Model::object_ids, allowing
    // object-level selection and highlighting without new geometry formats.
    for (terrain, index, draw) in scene.terrain.iter().enumerate().map(|(i,d)| (true,i,d))
        .chain(scene.placements.iter().enumerate().map(|(i,d)| (false,i,d))) {
        let mesh = cache.entry(draw.res_tmd).or_insert_with(|| {
            scene.res.tmds.get(draw.res_tmd)
                .and_then(|tmd| Model::from_tmd(&tmd.raw))
        });
        let Some(mesh) = mesh.as_ref() else { unavailable += 1; continue; };
        let Ok(base) = u32::try_from(out.positions.len()) else { unavailable += 1; continue; };
        let Ok(count) = u32::try_from(mesh.positions.len()) else { unavailable += 1; continue; };
        if base.checked_add(count).is_none() { unavailable += 1; continue; }
        let Ok(id) = u32::try_from(draws.len()) else { unavailable += 1; continue; };
        let native_position = scene.draw_world_position(draw);
        let lift = [
            native_position[0] - draw.world_x as f32,
            native_position[1] - draw.world_y as f32,
            native_position[2] - draw.world_z as f32,
        ];
        let positions: Vec<[f32; 3]> = mesh.positions.iter().map(|&p| {
            let p = draw.place_point(p, lift);
            [p[0], -p[1], p[2]]
        }).collect();
        let (center, radius) = bounds(&positions);
        out.positions.extend_from_slice(&positions);
        out.object_ids.extend(std::iter::repeat_n(id, positions.len()));
        out.indices.extend(mesh.indices.iter().map(|i| base + *i));
        out.uvs.extend_from_slice(&mesh.uvs);
        out.cba_tsb.extend_from_slice(&mesh.cba_tsb);
        out.modulation.extend_from_slice(&mesh.modulation);
        out.fill.extend_from_slice(&mesh.fill);
        out.textured.extend_from_slice(&mesh.textured);
        draws.push(DrawInfo {
            id, terrain, index, center, radius, original_center: center,
            offset: [0.0; 3], vertex_start: base as usize,
            vertex_end: base as usize + positions.len(),
        });
        instances += 1;
    }
    let model = if out.indices.is_empty() { None } else {
        out.rest = out.positions.clone();
        out.refit();
        Some(out)
    };
    WorldModel {
        model, instances, unavailable,
        unique_models: cache.values().filter(|v| v.is_some()).count(),
        ground_triangles, ground_vram_covered, ground_textured, draws,
    }
}

#[cfg(test)]
mod ground_texture_tests {
    use super::*;
    use legaia_scene_formats::field_objects::WalkHeightfield;

    fn one_tile() -> WalkHeightfield {
        WalkHeightfield {
            positions: vec![[0.0, 0.0, 0.0], [128.0, 0.0, 0.0],
                            [0.0, 0.0, 128.0], [128.0, 0.0, 128.0]],
            uvs: vec![[32, 63], [63, 63], [32, 32], [63, 32]],
            cba_tsb: vec![[0x7C40, 0x001A]; 4],
            colors: vec![[128, 128, 128]; 4],
            indices: vec![0, 1, 2, 1, 3, 2],
            ..WalkHeightfield::default()
        }
    }

    #[test]
    fn ground_uses_native_uv_clut_page_and_modulation() {
        let hf = one_tile();
        let mut m = empty_model();
        assert_eq!(append_native_ground(&mut m, &hf), 2);
        assert_eq!(m.uvs, hf.uvs);
        assert_eq!(m.cba_tsb, hf.cba_tsb);
        assert_eq!(m.modulation, hf.colors);
        assert_eq!(m.textured, vec![1; 4]);
        assert_eq!(m.object_ids, vec![u32::MAX; 4]);
        assert_eq!(m.indices.len(), 6);
    }

    #[test]
    fn incomplete_native_attributes_fall_back_to_diagnostic_fill() {
        let mut hf = one_tile();
        hf.cba_tsb.clear();
        let mut m = empty_model();
        assert_eq!(append_native_ground(&mut m, &hf), 2);
        assert_eq!(m.textured, vec![0; 4]);
        assert_eq!(m.cba_tsb, vec![[0, 0]; 4]);
        assert_eq!(m.fill, vec![[94, 103, 96]; 4]);
        assert_eq!(m.indices.len(), 6);
    }
}
