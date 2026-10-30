//! Stage P: read-only native PS1 field collision debug overlay.
//!
//! Each upper MAP collision nibble bit is one 64x64 world-unit wall sample.
//! The retail wall-query coordinates are NOT the ordinary terrain-cell
//! coordinates: the sub-cell row carries a +2 bias (see Stage M3).
//! Orange cells are present in the opened image; cyan cells are additions
//! in the predicted Stage M3 edit. An illustrated vertical extrusion makes
//! the wall easy to inspect; the game stores NO wall height in this nibble.
//! Script-painted barriers, actor collisions, and doors are not represented.
//! No MAP, IMG, scene project, or renderer mesh is modified here.

use anyhow::{anyhow, Context, Result};
use eframe::egui::{self, Color32, Pos2, Rect, Stroke, Ui, Vec2};
use legaia_engine_scene::scene::{ProtIndex, Scene};
use crate::raster::{Camera, Model};
use super::{ps1_export::ExportEdits, viewport::Viewport};

const DIM: usize = 128;
const SUB_DIM: i32 = 256;
const SUB_SIZE: i32 = 64;
const WALL_PAD: i32 = 24;
// Stage P2: no global detail cap. Nearby walls get a complete 3D wire box.
// Distant occupied cells are coalesced into exact same-tier, same-color
// horizontal runs, keeping every occupied cell represented without filling
// eframe's draw list with hundreds of thousands of tiny segments.
// Above this projected diameter we prefer 3D boxes; distant collision
// remains visible as same-tier/category occupancy strips. This is a viewer
// LOD setting and does not affect any PS1 MAP data or collision export.
const DETAIL_MIN_POINTS: f32 = 12.0;
// Prevent egui from queuing tens of thousands of tiny vector line shapes
// when a populated scene fills the viewport. Remaining visible wall samples
// are still represented with their real native floor tier/occupancy strips.
const MAX_DETAILED_BOXES: usize = 360;

#[derive(Clone)]
pub(super) struct CollisionGrid {
    pub scene: String,
    pub source_key: String,
    /// Native 128x128 byte MAP grid, unmodified. Upper nibble holds wall
    /// occupancy in 2x2 sub-cells; lower nibble is original floor tier.
    bytes: Vec<u8>,
    /// Original floor height LUT. It is not changed by this viewer.
    lut: [i16; 16],
}

impl CollisionGrid {
    pub fn with_owned_map(&self,map:&[u8])->Result<Self>{anyhow::ensure!(map.len()==0x12000,"Incomplete owned MAP preview");let mut next=self.clone();next.bytes=map[0x4000..0x8000].to_vec();Ok(next)}
    pub fn load(index: &ProtIndex, name: &str, source_key: &str) -> Result<Self> {
        let scene = Scene::load(index, name)
            .with_context(|| format!("Find native scene {name}"))?;
        let bytes = scene.field_collision_grid(index)?
            .ok_or_else(|| anyhow!("{name} has no 128x128 native MAP wall grid"))?;
        anyhow::ensure!(bytes.len() == DIM * DIM,
            "Expected 0x4000 MAP grid bytes; got {}", bytes.len());
        let lut = scene.field_floor_height_lut(index)?.unwrap_or([0i16; 16]);
        Ok(Self { scene: name.to_string(), source_key: source_key.to_string(), bytes, lut })
    }

    fn native_wall(&self, sx: i32, sz: i32) -> bool {
        wall_offset(sx, sz)
            .is_some_and(|(idx, bit)| self.bytes[idx] & bit != 0)
    }

    fn ground_y(&self, wx: f32, wz: f32) -> f32 {
        let col = ((wx / 128.0).floor() as i32).clamp(0, 127) as usize;
        let row = ((wz / 128.0).floor() as i32).clamp(0, 127) as usize;
        self.lut[(self.bytes[row * DIM + col] & 15) as usize] as f32
    }
}

/// The 2-bit sub-cell selector occupies the UPPER nibble. The Z +2
/// offset exactly matches the live retail wall query and Stage M3 exporter.
fn wall_offset(sx: i32, sz: i32) -> Option<(usize, u8)> {
    if !(0..SUB_DIM).contains(&sx) || !(0..SUB_DIM).contains(&sz) {
        return None;
    }
    let off = (sz as usize / 2) * DIM + sx as usize / 2;
    let flag = 1u8 << (4 + (sx & 1) + 2 * (sz & 1));
    Some((off, flag))
}

/// Infer exactly the same authored source stencil as Stage M3 native
/// collision export. This does not create collider geometry from mesh bounds.
fn source_stencil(grid: &CollisionGrid, bounds: Option<[i32; 4]>) -> Vec<(i32, i32)> {
    let Some([minx, maxx, minz, maxz]) = bounds else { return vec![]; };
    let (minx, maxx) = (minx - WALL_PAD, maxx + WALL_PAD);
    let (minz, maxz) = (minz - WALL_PAD, maxz + WALL_PAD);
    let min_sx = (minx - 32).div_euclid(SUB_SIZE).clamp(0, SUB_DIM - 1);
    let max_sx = (maxx - 32).div_euclid(SUB_SIZE).clamp(0, SUB_DIM - 1);
    let min_sz = ((minz - 32).div_euclid(SUB_SIZE) + 2).clamp(0, SUB_DIM - 1);
    let max_sz = ((maxz - 32).div_euclid(SUB_SIZE) + 2).clamp(0, SUB_DIM - 1);
    let mut bits = Vec::new();
    for sz in min_sz..=max_sz {
        for sx in min_sx..=max_sx {
            let wx = sx * SUB_SIZE + 32;
            let wz = (sz - 2) * SUB_SIZE + 32;
            if (minx..=maxx).contains(&wx) && (minz..=maxz).contains(&wz)
                && grid.native_wall(sx, sz) {
                bits.push((sx, sz));
            }
        }
    }
    bits
}

fn collision_steps(dx: f32, dz: f32) -> Result<(i32, i32)> {
    anyhow::ensure!(dx.is_finite() && dz.is_finite(), "Non-finite collision movement");
    anyhow::ensure!((dx.round() - dx).abs() <= 0.02 && (dz.round() - dz).abs() <= 0.02,
        "Collision preview requires integer-unit X/Z editor offsets");
    Ok(((dx.round() / SUB_SIZE as f32).round() as i32,
        (dz.round() / SUB_SIZE as f32).round() as i32))
}

/// Return the projected post-edit upper-wall mask, keeping original low floor
/// nibbles untouched. Does NOT change the input or write a disc image.
fn edited_mask(grid: &CollisionGrid, edits: &ExportEdits) -> Result<Vec<u8>> {
    let mut remove = vec![0u8; DIM * DIM];
    let mut add = vec![0u8; DIM * DIM];
    for moved in &edits.moves {
        let (sx, sz) = collision_steps(moved.delta[0], moved.delta[2])?;
        for (x, z) in source_stencil(grid, moved.collision_bounds) {
            let (old, old_bit) = wall_offset(x, z).context("Invalid source collision cell")?;
            let (new, new_bit) = wall_offset(x + sx, z + sz)
                .context("Moved collision crosses MAP boundary")?;
            remove[old] |= old_bit;
            add[new] |= new_bit;
        }
    }
    for copy in &edits.copies {
        let (sx, sz) = collision_steps(copy.delta[0], copy.delta[2])?;
        for (x, z) in source_stencil(grid, copy.collision_bounds) {
            let (new, new_bit) = wall_offset(x + sx, z + sz)
                .context("Cloned collision crosses MAP boundary")?;
            add[new] |= new_bit;
        }
    }
    // Stage Q5: donor MAP bits are already translated into target subcell
    // coordinates by prepare_cross_cell_collision_preview(). Preserve every
    // native destination floor tier and wall; never replace with mesh AABBs.
    for (_, stencil) in &edits.foreign_wall_stencils {
        for &(x,z) in stencil {
            let (new,new_bit)=wall_offset(x,z)
                .context("Foreign source wall crosses target MAP border")?;
            add[new] |= new_bit;
        }
    }
    let mut output = grid.bytes.clone();
    for (idx, byte) in output.iter_mut().enumerate() {
        let original = grid.bytes[idx];
        *byte = (original & 0x0F) | ((original & 0xF0 & !remove[idx]) | add[idx]);
    }
    Ok(output)
}

pub(super) struct Projector {
    viewport: Rect,
    w: f32,
    h: f32,
    f: f32,
    dist: f32,
    near: f32,
    centre: [f32; 3],
    offset: [f32; 3],
    sy: f32, cy: f32, sp: f32, cp: f32,
    mirror: bool,
}

impl Projector {
    pub(super) fn new(ui: &Ui, viewport: Rect, view: &Viewport, model: &Model) -> Self {
        let ppp = ui.ctx().pixels_per_point();
        // Keep exactly the raster resolution limits used in Viewport.
        let w = ((viewport.width() * ppp) as usize).clamp(16, 900) as f32;
        let h = ((viewport.height() * ppp) as usize).clamp(16, 680) as f32;
        let cam: Camera = view.cam;
        let fov = 45_f32.to_radians();
        let dist = model.radius / (fov * 0.5).sin() * cam.zoom.max(0.0000001);
        let (sy, cy) = cam.yaw.sin_cos();
        let (sp, cp) = cam.pitch.sin_cos();
        Self {
            viewport, w, h,
            f: (h * 0.5) / (fov * 0.5).tan(),
            dist, near: (dist * 0.003).max(0.0001),
            centre: model.center, offset: cam.target_offset,
            sy, cy, sp, cp, mirror: view.world_mirrored,
        }
    }

    /// Approximate 64-unit collision-cell width on the UI in logical points.
    /// This decides ONLY whether to draw a detailed 3D box or an overview run;
    /// it does not cull distant walls or hide any collision samples.
    fn projected_cell_points(&self, p: [f32; 3]) -> Option<f32> {
        let x = p[0] - (self.centre[0] + self.offset[0]);
        let y = p[1] - (self.centre[1] + self.offset[1]);
        let z = p[2] - (self.centre[2] + self.offset[2]);
        let z1 = -x * self.sy + z * self.cy;
        let y2 = y * self.cp - z1 * self.sp;
        let depth = y * self.sp + z1 * self.cp + self.dist;
        if !depth.is_finite() || depth <= self.near || !y2.is_finite() {
            return None;
        }
        let span = self.f * SUB_SIZE as f32 / depth * self.viewport.width() / self.w;
        if span.is_finite() { Some(span.abs()) } else { None }
    }

    pub(super) fn project(&self, p: [f32; 3]) -> Option<Pos2> {
        let x = p[0] - (self.centre[0] + self.offset[0]);
        let y = p[1] - (self.centre[1] + self.offset[1]);
        let z = p[2] - (self.centre[2] + self.offset[2]);
        let x1 = x * self.cy + z * self.sy;
        let z1 = -x * self.sy + z * self.cy;
        let y2 = y * self.cp - z1 * self.sp;
        let z2 = y * self.sp + z1 * self.cp + self.dist;
        if !z2.is_finite() || z2 < self.near { return None; }
        let mut px = self.w * 0.5 + self.f * x1 / z2;
        let py = self.h * 0.5 + self.f * y2 / z2;
        if self.mirror { px = self.w - 1.0 - px; }
        if !px.is_finite() || !py.is_finite() { return None; }
        Some(Pos2::new(self.viewport.left() + px / self.w * self.viewport.width(),
            self.viewport.top() + py / self.h * self.viewport.height()))
    }
}

/// A wall's visual category and native floor height. Adjacent overview
/// samples can only merge when their category AND floor height both match.
/// This preserves occupancy in the native 64-unit grid; there is no occupancy
/// interpolation, fake collision, or global drawing cap.
fn wall_key(grid: &CollisionGrid, mask: &[u8], sx: i32, sz: i32) -> Option<(bool, i16)> {
    let (idx, bit) = wall_offset(sx, sz)?;
    if mask[idx] & bit == 0 { return None; }
    let is_added = grid.bytes[idx] & bit == 0;
    let wx = sx * SUB_SIZE + SUB_SIZE / 2;
    let wz = (sz - 2) * SUB_SIZE + SUB_SIZE / 2;
    Some((is_added, grid.ground_y(wx as f32, wz as f32) as i16))
}

/// The exclusive end of a contiguous same-floor, same-category overview strip.
/// `can_join` allows the projected-size cutoff to vary across the strip.
fn overview_run_end(
    grid: &CollisionGrid, mask: &[u8], start: i32, sz: i32,
    mut can_join: impl FnMut(i32) -> bool,
) -> i32 {
    let Some(key) = wall_key(grid, mask, start, sz) else { return start; };
    let mut end = start + 1;
    // Bound a ribbon to 16 cells. This also prevents long runs from being
    // discarded as a unit when one endpoint passes behind the near plane.
    while end < SUB_DIM && end < start + 16
        && wall_key(grid, mask, end, sz) == Some(key) && can_join(end) {
        end += 1;
    }
    end
}

/// Draw all MAP collision samples in the camera view (no arbitrary 2300-wall
/// limit). Only actual camera clipping applies. At close range each sample
/// gets the original 12-edge 3D box; at long range it joins a same-tier strip
/// so that even large, crowded cells such as Fire Path stay visible from afar.
/// The distant strip is an OCCUPANCY OVERVIEW, not a physical wall-height mesh.
pub(super) fn paint(
    ui: &Ui, rect: Rect, view: &Viewport, model: &Model,
    grid: &CollisionGrid, edits: Result<ExportEdits>,
) {
    // Display how many authored donor MAP wall bits were actually found.
    // The absence of such bits does NOT prove the source mesh has no collider:
    // Legaia has several kinds of collision and MAP bits have no per-model owner.
    let foreign_stats = edits.as_ref().ok().map(|result| (
        result.foreign_wall_stencils.len(),
        result.foreign_wall_stencils.iter().map(|(_, bits)| bits.len()).sum::<usize>(),
    ));
    let (mask, note) = match edits.and_then(|edits| edited_mask(grid, &edits)) {
        Ok(edited) => (edited, None),
        Err(err) => (grid.bytes.clone(), Some(err.to_string())),
    };
    let original = Color32::from_rgb(255, 138, 64);
    let added = Color32::from_rgb(56, 235, 236);
    let p = Projector::new(ui, rect, view, model);
    let painter = ui.painter().with_clip_rect(rect);
    let viewport_margin = rect.expand(115.0);
    let mut total = 0usize;
    let mut represented = 0usize;
    let mut overview_strips = 0usize;
    let mut detailed = 0usize;

    for sz in 0..SUB_DIM {
        let mut sx = 0;
        while sx < SUB_DIM {
            let Some((is_added, floor)) = wall_key(grid, &mask, sx, sz) else {
                sx += 1;
                continue;
            };
            let color = if is_added { added } else { original };
            let x = sx as f32 * SUB_SIZE as f32;
            let z = (sz - 2) as f32 * SUB_SIZE as f32;
            let y = floor as f32 + 6.0;
            let centre = [x + 32.0, y + 48.0, z + 32.0];
            let size = p.projected_cell_points(centre).unwrap_or(0.0);

            // Precise nearby collision: retain the original detailed boxes.
            if size >= DETAIL_MIN_POINTS && detailed < MAX_DETAILED_BOXES {
                total += 1;
                sx += 1;
                let Some(centre_pos) = p.project(centre) else { continue; };
                if !viewport_margin.contains(centre_pos) { continue; }
                let corners = [[x,y,z], [x+64.0,y,z],
                    [x+64.0,y,z+64.0], [x,y,z+64.0]];
                let mut floor_pos = [Pos2::ZERO; 4];
                let mut roof_pos = [Pos2::ZERO; 4];
                let mut valid = true;
                for (i, corner) in corners.iter().enumerate() {
                    if let (Some(bottom), Some(top)) =
                        (p.project(*corner), p.project([corner[0], corner[1]+95.0, corner[2]]))
                    {
                        floor_pos[i] = bottom;
                        roof_pos[i] = top;
                    } else {
                        valid = false;
                        break;
                    }
                }
                if !valid { continue; }
                let line = Stroke::new(if is_added { 1.8 } else { 1.15 }, color);
                for i in 0..4 {
                    let next = (i+1)%4;
                    painter.line_segment([floor_pos[i], floor_pos[next]], line);
                    painter.line_segment([roof_pos[i], roof_pos[next]], line);
                    painter.line_segment([floor_pos[i], roof_pos[i]], line);
                }
                detailed += 1;
                represented += 1;
                continue;
            }

            // Distant collision: group only genuinely adjacent, occupied cells
            // on the same native floor level and with the same orange/cyan
            // category. This never merges across an empty wall cell.
            let end = overview_run_end(grid, &mask, sx, sz, |next| {
                let nx = next as f32 * SUB_SIZE as f32 + 32.0;
                p.projected_cell_points([nx, y + 48.0, z + 32.0])
                    .is_some_and(|width| width < DETAIL_MIN_POINTS || detailed >= MAX_DETAILED_BOXES)
            });
            let count = (end - sx) as usize;
            total += count;
            let end_x = end as f32 * SUB_SIZE as f32;
            sx = end;

            // The overview footprint is a flat ribbon at native terrain tier;
            // line endpoints describe every covered wall bit, even if the
            // individual 64-unit samples occupy sub-pixel screen widths.
            let corners = [
                [x, y+48.0, z], [end_x, y+48.0, z],
                [end_x, y+48.0, z+64.0], [x, y+48.0, z+64.0],
            ];
            let mut points = [Pos2::ZERO; 4];
            let mut valid = true;
            for (i, corner) in corners.iter().enumerate() {
                if let Some(point) = p.project(*corner) { points[i] = point; }
                else { valid = false; break; }
            }
            if !valid { continue; }
            let mut projected_rect = Rect::from_min_max(points[0], points[0]);
            for pt in points.iter().skip(1) {
                projected_rect = projected_rect.union(Rect::from_min_max(*pt, *pt));
            }
            if !projected_rect.intersects(viewport_margin) { continue; }
            let stroke = Stroke::new(if is_added { 1.65 } else { 1.1 }, color);
            for i in 0..4 {
                painter.line_segment([points[i], points[(i+1)%4]], stroke);
            }
            // Persistent colored centre-line keeps distant single-cell ribbons
            // visible even when the footprint collapses to a fraction of a px.
            if let (Some(a), Some(b)) = (
                p.project([x, y+48.0, z+32.0]),
                p.project([end_x, y+48.0, z+32.0]),
            ) {
                painter.line_segment([a, b], Stroke::new(1.6, color));
            }
            overview_strips += 1;
            represented += count;
        }
    }

    let bg = Rect::from_min_size(rect.min + Vec2::new(7.0, 7.0),
        Vec2::new(465.0, if note.is_some(){95.0}else{78.0}));
    painter.rect_filled(bg.intersect(rect), 3.0, Color32::from_black_alpha(190));
    let title = format!("PS1 collision  |  {represented} in view / {total} wall samples");
    painter.text(bg.min + Vec2::new(8.0, 6.0), egui::Align2::LEFT_TOP,
        title, egui::FontId::monospace(11.0), Color32::WHITE);
    painter.text(bg.min + Vec2::new(8.0, 23.0), egui::Align2::LEFT_TOP,
        format!("{detailed} detailed (max {MAX_DETAILED_BOXES})  |  {overview_strips} occupancy strips"),
        egui::FontId::monospace(10.0), Color32::LIGHT_GRAY);
    painter.text(bg.min + Vec2::new(8.0, 40.0), egui::Align2::LEFT_TOP,
        "Orange = source MAP walls  |  Cyan = editor additions",
        egui::FontId::monospace(10.0), Color32::LIGHT_GRAY);
    let donor_text = match foreign_stats {
        Some((0,_)) => "Donor collision: none staged".to_owned(),
        Some((count,bits)) => format!("Donor MAP preview: {count} foreign objects / {bits} authored wall bits"),
        None => "Donor preview unsupported: showing source MAP walls".to_owned(),
    };
    painter.text(bg.min + Vec2::new(8.0, 57.0), egui::Align2::LEFT_TOP,
        donor_text, egui::FontId::monospace(10.0), Color32::LIGHT_GRAY);
    if let Some(reason) = note.as_ref() {
        // Full reason remains in the log; keep a short enough in-viewport clue.
        let head = reason.chars().take(66).collect::<String>();
        painter.text(bg.min + Vec2::new(8.0, 76.0), egui::Align2::LEFT_TOP,
            head, egui::FontId::monospace(9.0), Color32::YELLOW);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_grid() -> CollisionGrid {
        CollisionGrid { scene: "test".into(), source_key: "".into(), bytes: vec![0; DIM*DIM], lut: [0;16] }
    }
    #[test]
    fn upper_nibble_bits_select_different_subcells_without_overwriting_floor() {
        for z in 0..2 {
            for x in 0..2 {
                let (off, bit) = wall_offset(x, z).unwrap();
                assert_eq!(off, 0);
                assert_eq!(bit, 0x10 << (x+2*z));
            }
        }
        assert!(wall_offset(-1,0).is_none());
        assert!(wall_offset(0,256).is_none());
    }
    #[test]
    fn wall_query_row_bias_is_two_subcells() {
        let mut grid = empty_grid();
        let (off, bit) = wall_offset(3,6).unwrap();
        grid.bytes[off] = bit | 0x0A;
        assert_eq!(source_stencil(&grid,Some([210,235,275,300])), vec![(3,6)]);
        assert!(!grid.native_wall(3,7));
    }
    #[test]
    fn distant_runs_cover_every_occupied_cell_without_a_detail_cap() {
        let mut grid = empty_grid();
        for z in 2..=254 {
            for x in 0..SUB_DIM {
                let (off, bit) = wall_offset(x,z).unwrap();
                grid.bytes[off] |= bit;
            }
        }
        let mut represented = 0usize;
        for z in 2..=254 {
            let mut x = 0;
            while x < SUB_DIM {
                let end = overview_run_end(&grid, &grid.bytes, x,z, |_| true);
                assert!(end>x);
                represented += (end-x) as usize;
                x=end;
            }
        }
        assert_eq!(represented, 253 * SUB_DIM as usize);
        assert!(represented > 2300);
    }
    #[test]
    fn distant_runs_never_merge_gaps_or_new_walls() {
        let mut grid = empty_grid();
        for x in [10,11,13,14] {
            let (off,bit)=wall_offset(x,14).unwrap();
            grid.bytes[off] |= bit;
        }
        let mut mask = grid.bytes.clone();
        let (idx,bit)=wall_offset(11,14).unwrap();
        mask[idx] &= !bit;
        let (idx,bit)=wall_offset(12,14).unwrap();
        mask[idx] |= bit; // cyan cell (not in source)
        assert_eq!(overview_run_end(&grid,&mask,10,14, |_| true),11);
        assert_eq!(overview_run_end(&grid,&mask,12,14, |_| true),13);
        assert_eq!(overview_run_end(&grid,&mask,13,14, |_| true),15);
    }
    #[test]
    fn overview_runs_respect_native_floor_tiers() {
        let mut grid=empty_grid();
        let (a,b)=wall_offset(10,14).unwrap();
        let (c,d)=wall_offset(11,14).unwrap();
        grid.bytes[a] |= b;
        grid.bytes[c] |= d;
        // Both bits share the same native 128-wide byte, so both use same floor.
        assert_eq!(overview_run_end(&grid,&grid.bytes,10,14, |_| true),12);
        // An adjacent cell crossing a native MAP tile boundary can differ.
        let (c,d)=wall_offset(12,14).unwrap();
        grid.bytes[c] |= d;
        // Ground tier sampling uses un-biased Z, unlike wall-bit lookup.
        // Native wall sz=14 corresponds to ground tile row 6.
        grid.bytes[6*DIM + 6] = (grid.bytes[6*DIM + 6] & 0xF0) | 1;
        grid.lut[1]=128;
        assert_eq!(overview_run_end(&grid,&grid.bytes,10,14, |_| true),12);
    }
    #[test]
    fn donor_wall_preview_composites_and_preserves_floor_nibbles() {
        let mut grid=empty_grid();
        let (base,bit)=wall_offset(44,38).unwrap();
        grid.bytes[base]=bit | 0x07;
        let (foreign_off,foreign_bit)=wall_offset(46,40).unwrap();
        grid.bytes[foreign_off]=0x0C;
        let mut edits=ExportEdits::default();
        edits.foreign_wall_stencils.push((4,vec![(46,40)]));
        let predicted=edited_mask(&grid,&edits).unwrap();
        assert_eq!(predicted[base],bit|0x07);
        assert_eq!(predicted[foreign_off],foreign_bit|0x0C);
        assert_eq!(grid.bytes[foreign_off],0x0C);
    }
    #[test]
    fn foreign_wall_preview_fails_at_map_boundary() {
        let grid=empty_grid();
        let mut edits=ExportEdits::default();
        edits.foreign_wall_stencils.push((17,vec![(256,1)]));
        assert!(edited_mask(&grid,&edits).is_err());
    }
    #[test]
    fn moved_and_copied_walls_composite_without_touching_tiers() {
        use super::super::ps1_export::{MoveEdit, InsertEdit};
        let mut grid = empty_grid();
        let (idx,bit) = wall_offset(20,20).unwrap();
        grid.bytes[idx] = bit | 6;
        let bounds = Some([20*64+18,20*64+46,18*64+18,18*64+46]);
        let mut edits = ExportEdits::default();
        edits.moves.push(MoveEdit { collision_bounds: bounds, cell:(10,10),
            source_position:(0,0),env_slot:0,source_rotation:[0;3],delta:[128.0,0.0,0.0] });
        edits.copies.push(InsertEdit { collision_bounds: bounds, uid:10,cell:(10,10),
            source_position:(0,0),env_slot:0,source_rotation:[0;3],delta:[0.0;3] });
        let result = edited_mask(&grid,&edits).unwrap();
        let (target,flag) = wall_offset(22,20).unwrap();
        assert_ne!(result[target]&flag,0);
        assert_ne!(result[idx]&bit,0);
        assert_eq!(result[idx]&15,6);
        assert_eq!(grid.bytes[idx],bit|6);
    }
}
