//! Stage M3 experimental: native object insertion + MOVE/COPY collision stencil.
//! The PS1 uses TWO different collision sources: per-object static collider
//! offsets (32-byte MAP records) AND the *separate* walk-wall nibble grid at
//! MAP +0x4000. Stage M2 only touched the record; the wall grid never moved.
//! This stage conservatively infers original object collision walls by
//! intersecting the retail wall grid with the object's original X/Z geometry
//! bounding box, then translates that original stencil for moved and copied
//! objects. The collision grid's floor nibble is NEVER touched.
//! Ownership is inferred, not stored on disc: a stencil shared with terrain,
//! buildings or scripted wall painting cannot be separated perfectly.
//! Native 64-unit wall resolution limits sub-tile placement precision.
//! No native arbitrary TMD scaling or cloned door/script bindings.
use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::{bail, ensure, Context, Result};
use legaia_disc_patch::disc::DiscPatcher;
use legaia_scene_formats::field_objects::{self, ObjectRecord};
use legaia_engine_scene::scene_assembly::AssembledScene;
use legaia_engine_scene::scene::{Scene, ProtIndex};
use super::world_view;
use legaia_scene_formats::{man_section, scene_asset_table};
use legaia_disc_patch::compress_within;


use super::{scene_instances, world_view::WorldModel};

const MAP_LEN: usize = 0x12000;
const GRID_OFF: usize = 0x8000;
const WALL_OFF: usize = 0x4000;
const GRID_DIM: usize = 128;
const SUB_DIM: usize = GRID_DIM * 2;
const SUB_SIZE: i32 = 64;
const WALL_BOUND_PAD: i32 = 24;
const MAX_WALL_BOUNDS: i32 = 2048;
const EPS: f32 = 0.02;

#[derive(Clone, Debug)]
pub(super) struct MoveEdit {
    /// Original drawable's horizontal bounds in untouched game world units.
    /// None when no native mesh vertices could be resolved; then the
    /// record-local collider still moves but NO wall stencil is inferred.
    pub collision_bounds: Option<[i32;4]>, // minX,maxX,minZ,maxZ
    pub cell: (u8, u8),
    pub source_position: (i32, i32),
    pub env_slot: usize,
    pub source_rotation: [u16; 3],
    /// LCK view-space position adjustment; editor Y is opposite PSX Y.
    pub delta: [f32; 3],
}

/// A new editor copy referenced by its original PS1 placement (not by
/// a rendered vertex buffer or by a transient draw index).
#[derive(Clone, Debug)]
pub(super) struct InsertEdit {
    pub collision_bounds: Option<[i32;4]>,
    pub uid: u64,
    pub cell: (u8, u8),
    pub source_position: (i32, i32),
    pub env_slot: usize,
    pub source_rotation: [u16; 3],
    pub delta: [f32; 3],
}

/// Native MAN partition-1 actor. Clones replicate their source VM script,
/// not just a renderable mesh. The engine may subsequently reposition an NPC
/// via script; the exporter rejects known script-relocated spawn coordinates.
#[derive(Clone, Debug)]
pub(super) struct NpcEdit {
    pub uid: Option<u64>, // Some = cloned MAN record; None = original modification
    pub record: usize,
    pub source_position: (i32,i32),
    pub original_model: u8,
    pub original_anim: u8,
    pub model: u8,
    pub anim: u8,
    pub delta: [f32;3],
    pub kind: &'static str,
}

#[derive(Clone, Debug, Default)]
pub(super) struct ExportEdits {
    pub moves: Vec<MoveEdit>,
    pub copies: Vec<InsertEdit>,
    pub npcs: Vec<NpcEdit>,
    /// Native placed-object floor height in the original PSX Y-down frame,
    /// derived from real assembled ground vertices / source placements.
    pub tier_heights: [Option<i32>;16],
    /// Authored donor wall bits remapped to the destination grid, keyed by imported UID.
    /// Cross-cell models are ONLY allowed when already byte-exact native TMDs.
    pub foreign_wall_stencils: Vec<(u64,Vec<(i32,i32)>)>,
}

/// This derives bounds from ORIGINAL untouched editor mesh vertices (rest),
/// not from the user's moved, rotated or resized preview. The walk grid is
/// authored at PS1 world X/Z coordinates; editor Y sign is irrelevant.
fn original_collision_bounds(world: &WorldModel, index: usize) -> Option<[i32;4]> {
    let draw=world.at(false,index)?;
    let model=world.model.as_ref()?;
    if draw.vertex_start>=draw.vertex_end || draw.vertex_end>model.rest.len() {return None;}
    let mut limits=[f32::INFINITY,f32::NEG_INFINITY,f32::INFINITY,f32::NEG_INFINITY];
    for p in &model.rest[draw.vertex_start..draw.vertex_end] {
        if !p[0].is_finite() || !p[2].is_finite() {return None;}
        limits[0]=limits[0].min(p[0]);limits[1]=limits[1].max(p[0]);
        limits[2]=limits[2].min(p[2]);limits[3]=limits[3].max(p[2]);
    }
    if limits.iter().any(|v|!v.is_finite() || v.abs()>32768.0) ||
       limits[1]-limits[0]>MAX_WALL_BOUNDS as f32 ||
       limits[3]-limits[2]>MAX_WALL_BOUNDS as f32 {return None;}
    Some([limits[0].floor() as i32,limits[1].ceil() as i32,
          limits[2].floor() as i32,limits[3].ceil() as i32])
}

/// Foreign staging can carry its own local preview transform, but it is
/// not part of the destination's native MAP collision. Do NOT exempt original
/// or cloned native objects from unsupported rotate/scale preflight.
fn has_unhandled_native_transforms(doc: &scene_instances::SceneDocument, collision_preview: bool) -> bool {
    if !collision_preview { return !doc.rotations.is_empty() || !doc.scales.is_empty(); }
    let foreign_uids: HashSet<u64> = doc.foreign.iter().map(|entry| entry.uid).collect();
    doc.rotations.iter().chain(doc.scales.iter()).any(|(target, _)| {
        !matches!(target, scene_instances::NameTarget::Created(uid) if foreign_uids.contains(uid))
    })
}

/// Preflight the complete LCK editor state. Never silently ignore an
/// unsupported physical change that the user expects to see in-game.
pub(super) fn prepare(scene: &AssembledScene, world: &WorldModel) -> Result<ExportEdits> {
    prepare_with_mode(scene, world, false)
}

/// Build a read-only collision overlay from native MAP edits even if the user
/// also staged a foreign TMD mesh. Foreign previews have NO imported native
/// collision and are deliberately ignored; the strict IMG exporter above still
/// rejects the same scene until foreign resource relocation is implemented.
pub(super) fn prepare_collision_preview(scene: &AssembledScene, world: &WorldModel) -> Result<ExportEdits> {
    prepare_with_mode(scene, world, true)
}

/// Copy only data we can verify at original object placement time.
/// The PS1 wall nibble does not encode an owner, so wall footprints are
/// conservative source-MAP samples, not guaranteed object-owned collision.
/// Missing or scripted placements retain the TMD while omitting inferred walls.
pub(super) fn capture_donor_native(
    index:&ProtIndex,donor:&AssembledScene,kind:&str,record:usize,
    points:&[[f32;3]],
)->(Option<[u8;32]>,Vec<(i32,i32)>) {
    // A terrain/decor draw has no standalone MAP placement record, but its
    // ORIGINAL donor MAP may contain wall bits in the rendered footprint.
    // Sampling those bits is DIAGNOSTIC ONLY: wall bits have no owner ID,
    // and a terrain TMD does NOT include the walkable heightfield/floor LUT.
    if kind != "placement" && kind != "terrain" { return (None,Vec::new()); }
    let read = || -> Result<(Option<[u8;32]>,Vec<(i32,i32)>)> {
        let donor_scene=Scene::load(index,&donor.name)?;
        let map_id=donor_scene.field_map_index(index)
            .context("Donor has no native field MAP")?;
        let map=index.entry_bytes(map_id)?;
        ensure!(map.len()==MAP_LEN,"Donor MAP record size invalid");
        if kind == "terrain" {
            // Not object-owned collision and not proof this tile is walkable.
            return Ok((None,source_stencil(&map,point_horizontal_bounds(points))));
        }
        let placement=donor.placements.get(record).context("Missing donor placement")?;
        let (slot,record)=record_at(&map,placement.cell,
            (placement.world_x,placement.world_z),placement.env_slot,
            [placement.rot_x,placement.rot_y,placement.rot_z])?;
        let start=slot.checked_mul(32).context("MAP record overflow")?;
        let mut raw=[0u8;32];
        raw.copy_from_slice(map.get(start..start+32).context("MAP record truncated")?);
        if anchor_has_bind(&map,placement.cell,&record) {
            log::warn!("Donor {} placement {} is script-bound: keeping original model/record but not attributing shared wall bits",donor.name,slot);
            return Ok((Some(raw),Vec::new()));
        }
        Ok((Some(raw),source_stencil(&map,point_horizontal_bounds(points))))
    };
    match read() {
        Ok(data)=>data,
        Err(error)=>{
            log::warn!("Donor {} {} #{}: native MAP provenance not verified: {error:#}; retaining model and textures only",donor.name,kind,record);
            (None,Vec::new())
        }
    }
}

/// Stage Q5: READ-ONLY donor wall preview. The foreign mesh's visible
/// geometry is not itself proof of collision: native Legaia walls live in
/// the donor MAP upper-nibble grid. We carry only bits that already exist in
/// the source MAP within the source object's original mesh footprint.
/// Unlike prepare_cross_cell(), this preview does not require a matching
/// destination TMD; no native IMG write is authorized by this function.
/// The PS1 does not encode wall-bit ownership, so overlapping terrain and
/// scripted walls can still be part of the inferred stencil.
pub(super) fn prepare_cross_cell_collision_preview(
    scene: &AssembledScene, world: &WorldModel, _index: &ProtIndex,
) -> Result<ExportEdits> {
    let mut edits=prepare_collision_preview(scene, world)?;
    let doc=scene_instances::parse(&scene_instances::serialize(&scene.name,world),
        &scene.name,world).map_err(anyhow::Error::msg)?;
    for foreign in &doc.foreign {
        // Rotating or resizing the donor changes the collider's geometry;
        // translating the original bits would incorrectly suggest otherwise.
        ensure!(!doc.rotations.iter().any(|(t,_)|
                matches!(t, scene_instances::NameTarget::Created(uid) if *uid==foreign.uid)) &&
            !doc.scales.iter().any(|(t,_)|
                matches!(t, scene_instances::NameTarget::Created(uid) if *uid==foreign.uid)),
            "Foreign UID {} is rotated/scaled: exact native collision preview is not available. Reset its transform to inspect its original collision stencil", foreign.uid);
        ensure!(foreign.kind=="placement" || foreign.kind=="terrain",
            "Foreign UID {} is {}: no static MAP wall provenance for this asset category", foreign.uid,foreign.kind);
        // Native provenance was captured at the time of drag/drop or load.
        // Never reassemble or decompress the donor scene in the viewport loop.
        let owned=world.foreign.iter().find(|item|item.uid==foreign.uid)
            .with_context(||format!("Missing cached source copy for UID {}",foreign.uid))?;
        if foreign.kind=="placement" && owned.native_record.is_none() {
            log::debug!("Foreign UID {}: no verified source MAP collision",foreign.uid);
            continue;
        }
        // Terrain is sampled solely from the original donor MAP wall grid;
        // those source bits may belong to overlapping environment assets.
        let sx=collision_step(foreign.center[0]-owned.donor_center[0],"Donor preview wall X")?;
        let sz=collision_step(foreign.center[2]-owned.donor_center[2],"Donor preview wall Z")?;
        let stencil=owned.donor_wall_bits.iter().map(|&(x,z)|(x+sx,z+sz))
            .collect::<Vec<_>>();
        for &(x,z) in &stencil {
            ensure!(wall_offset(x,z).is_some(),
                "Foreign UID {} wall preview exceeds the destination MAP grid",foreign.uid);
        }
        log::info!("Foreign UID {} from {}: {} authored wall subcells found for read-only preview",
            foreign.uid,foreign.source,stencil.len());
        edits.foreign_wall_stencils.push((foreign.uid,stencil));
    }
    Ok(edits)
}

/// Conservative native cross-cell bridge. This is NOT a general donor TMD/TIM
/// importer: a different mesh/atlas still needs scene-bundle repacking and
/// remains blocked. Native reuse passes only where the target already owns an
/// render-equivalent TMD, physically matching placement orientation, donor
/// texture colours, and static-collider footprint. The source authored MAP
/// wall bits then follow this specific foreign instance to its new position.
/// This path creates real native MAP records, not an editor-only cyan ghost.
pub(super) fn prepare_cross_cell(scene: &AssembledScene, world: &WorldModel,
    index: &ProtIndex) -> Result<ExportEdits> {
    let doc = scene_instances::parse(&scene_instances::serialize(&scene.name,world),
        &scene.name,world).map_err(anyhow::Error::msg)?;
    if doc.foreign.is_empty() { return prepare(scene,world); }
    ensure!(doc.rotations.is_empty() && doc.scales.is_empty(),
        "Cross-cell IMG export cannot yet represent rotations or scales. Reset transforms or save .lckscene");
    // Reuse the M3 exporter and its strict checks for all destination-native
    // edits. The preview mode only skips the foreign-asset guard; every foreign
    // item is validated below before the result is permitted to export.
    ensure!(!doc.created.iter().any(|created|world.npc(created.source).is_some()),
        "Native MAN NPC cloning alongside foreign resource imports needs a separate export preflight");
    let mut edits = prepare_with_mode(scene,world,true)?;
    let target_scene=Scene::load(index,&scene.name)
        .context("Resolve destination retail MAP for cross-cell import")?;
    let target_map_index=target_scene.field_map_index(index)
        .context("Destination does not have a native field MAP")?;
    let target_map=index.entry_bytes(target_map_index)?;
    ensure!(target_map.len()==MAP_LEN,"Destination native MAP has unexpected length");
    let model=world.model.as_ref().context("No destination scene geometry")?;
    let mut imported=HashSet::<u64>::new();
    for foreign in &doc.foreign {
        ensure!(foreign.kind=="placement",
            "Foreign UID {} from {} is {}: native cross-cell export currently supports only static MAP placements, not terrain, raw mesh or NPC imports",
            foreign.uid,foreign.source,foreign.kind);
        let donor=legaia_engine_world_views::scene_staging::assemble_field_scene(
            index,&foreign.source).map_err(anyhow::Error::msg).with_context(||format!(
                "Read donor {} for cross-cell UID {}",foreign.source,foreign.uid))?;
        let src_draw=donor.placements.get(foreign.record)
            .context("Original foreign placement missing from donor scene")?;
        donor.res.tmds.get(src_draw.res_tmd)
            .context("Original foreign TMD missing")?;
        let (mesh,points)=world_view::foreign_original_geometry(
            &donor,&foreign.kind,foreign.record).map_err(anyhow::Error::msg)?;
        let src_center=points_center(&points).context("Foreign donor is empty")?;
        let donor_scene=Scene::load(index,&foreign.source)?;
        let donor_map_index=donor_scene.field_map_index(index)
            .context("Donor has no native field MAP")?;
        let donor_map=index.entry_bytes(donor_map_index)?;
        ensure!(donor_map.len()==MAP_LEN,"Donor MAP has unexpected length");
        let (_,donor_rec)=record_at(&donor_map,src_draw.cell,
            (src_draw.world_x,src_draw.world_z),src_draw.env_slot,
            [src_draw.rot_x,src_draw.rot_y,src_draw.rot_z])
            .context("Cannot prove original donor MAP record")?;
        ensure!(donor_rec.is_placed() && !anchor_has_bind(&donor_map,src_draw.cell,&donor_rec),
            "Donor UID {} uses a script-bound placement; MAN object bindings cannot be silently transplanted",foreign.uid);
        let mut picked=None;
        let mut failures=Vec::new();
        for (i,placed) in scene.placements.iter().enumerate() {
            let Some(target_tmd)=scene.res.tmds.get(placed.res_tmd) else {continue};
            // Byte-different TMDs can still decode into identical rendering
            // primitives (header padding / unused resource records). Compare
            // *all* decoded triangle/material attributes, not hashes or names.
            let Some(target_mesh)=crate::raster::Model::from_tmd(&target_tmd.raw) else {continue};
            if target_mesh.indices!=mesh.indices || target_mesh.uvs!=mesh.uvs ||
                target_mesh.cba_tsb!=mesh.cba_tsb ||
                target_mesh.modulation!=mesh.modulation || target_mesh.fill!=mesh.fill ||
                target_mesh.textured!=mesh.textured {continue;}
            let Some(draw)=world.at(false,i) else {continue};
            if draw.hidden || draw.vertex_start>=draw.vertex_end || draw.editor_uid.is_some() {continue;}
            if !same_vertex_shape(&points,src_center,model,draw) {
                failures.push("same TMD with different native object rotation or pose");continue;
            }
            if !same_texture_output(&mesh,&donor.res.vram,&scene.res.vram) {
                failures.push("donor texture pixels or palette differ in destination VRAM");continue;
            }
            let Some((_,target_rec))=record_at(&target_map,placed.cell,
                (placed.world_x,placed.world_z),placed.env_slot,
                [placed.rot_x,placed.rot_y,placed.rot_z]).ok() else {continue};
            if anchor_has_bind(&target_map,placed.cell,&target_rec) ||
                !target_rec.is_placed() ||
                field_objects::collision_footprint_offset(&target_rec)!=
                    field_objects::collision_footprint_offset(&donor_rec) {
                failures.push("different static collider or script-bound destination record");continue;
            }
            picked=Some((i,placed,draw));break;
        }
        let (i,placed,draw)=picked.with_context(||format!(
            "Foreign UID {} ({}/{} #{}) requires a NEW TMD/TIM import and cannot be represented by a proven equivalent native model in {}. {}. IMG unchanged.",
            foreign.uid,foreign.source,foreign.kind,foreign.record,scene.name,
            if failures.is_empty(){"No destination model has identical decoded triangles/materials".to_owned()}
            else{failures.join("; ")}))?;
        ensure!(imported.insert(foreign.uid),"Duplicate foreign export UID");
        let delta=[0,1,2].map(|axis|foreign.center[axis]-draw.original_center[axis]);
        ensure!(delta.iter().all(|v|v.is_finite() && v.abs()<32768.0),
            "Foreign UID {} destination coordinate exceeds native range",foreign.uid);
        // Full native 64-unit authoring grid. Round only the collision stencil;
        // the placed object's visual coordinates use the original 16-bit MAP
        // offsets and retain the engine's 16-unit collider quantization.
        let dsx=collision_step(foreign.center[0]-src_center[0],"Donor wall X")?;
        let dsz=collision_step(foreign.center[2]-src_center[2],"Donor wall Z")?;
        let bounds=point_horizontal_bounds(&points)
            .context("Donor geometry has no provable collision bounds")?;
        let donor_bits=source_stencil(&donor_map,Some(bounds));
        let translated=donor_bits.into_iter().map(|(x,z)|(x+dsx,z+dsz))
            .collect::<Vec<_>>();
        for &(x,z) in &translated {
            ensure!(wall_offset(x,z).is_some(),
                "Foreign UID {} donor wall stencil crosses destination MAP border",foreign.uid);
        }
        edits.foreign_wall_stencils.push((foreign.uid,translated));
        edits.copies.push(InsertEdit {
            collision_bounds:None, // must NEVER borrow unrelated destination walls
            uid:foreign.uid,cell:placed.cell,
            source_position:(placed.world_x,placed.world_z),env_slot:placed.env_slot,
            source_rotation:[placed.rot_x,placed.rot_y,placed.rot_z],delta,
        });
        log::info!("Cross-cell UID {} resolved to render-equivalent native {} placement #{i}; donor authored wall stencil retained",
            foreign.uid,scene.name);
    }
    ensure!(imported.len()==doc.foreign.len(),"Some foreign assets did not resolve to native copies");
    ensure!(edits.copies.len()<=16,"Native IMG export permits no more than 16 total original/cross-cell object copies");
    ensure!(!edits.moves.is_empty() || !edits.copies.is_empty() || !edits.npcs.is_empty(),
        "No native changes to export");
    Ok(edits)
}

/// Read the native MAP anchor bind bit. Out-of-grid source records fail closed.
fn anchor_has_bind(map:&[u8],cell:(u8,u8),rec:&ObjectRecord)->bool {
    let col=cell.0 as i32+rec.col_delta as i32;
    let row=cell.1 as i32+rec.row_delta as i32;
    if !(0..128).contains(&col) || !(0..128).contains(&row){return true;}
    let off=GRID_OFF+((row as usize)*GRID_DIM+col as usize)*2;
    let Some(bytes)=map.get(off..off+2) else{return true};
    (u16::from_le_bytes([bytes[0],bytes[1]])&field_objects::CELL_BIND_OWNED)!=0
}

fn points_center(points:&[[f32;3]])->Option<[f32;3]> {
    if points.is_empty(){return None;}
    let mut low=[f32::INFINITY;3];let mut high=[f32::NEG_INFINITY;3];
    for &p in points {for j in 0..3 {if !p[j].is_finite(){return None;}
        low[j]=low[j].min(p[j]);high[j]=high[j].max(p[j]);}}
    Some([0,1,2].map(|j|(low[j]+high[j])*0.5))
}
fn point_horizontal_bounds(points:&[[f32;3]])->Option<[i32;4]> {
    if points.is_empty(){return None;}
    let mut box2=[f32::INFINITY,f32::NEG_INFINITY,f32::INFINITY,f32::NEG_INFINITY];
    for p in points {if !p[0].is_finite()||!p[2].is_finite(){return None;}
        box2[0]=box2[0].min(p[0]);box2[1]=box2[1].max(p[0]);
        box2[2]=box2[2].min(p[2]);box2[3]=box2[3].max(p[2]);}
    if box2[1]-box2[0]>MAX_WALL_BOUNDS as f32 ||
        box2[3]-box2[2]>MAX_WALL_BOUNDS as f32{return None;}
    Some([box2[0].floor() as i32,box2[1].ceil() as i32,
        box2[2].floor() as i32,box2[3].ceil() as i32])
}
fn same_vertex_shape(points:&[[f32;3]],center:[f32;3],model:&crate::raster::Model,
    draw:&world_view::DrawInfo)->bool {
    if draw.vertex_end>model.rest.len() || draw.vertex_end-draw.vertex_start!=points.len(){return false;}
    points.iter().zip(&model.rest[draw.vertex_start..draw.vertex_end])
        .all(|(p,q)|(0..3).all(|j|((p[j]-center[j])-(q[j]-draw.original_center[j])).abs()<=0.15))
}
/// Compares every potentially sampled palette/texture colour in a TMD prim's
/// UV bounding rectangle, not just non-empty atlas coverage. This catches a
/// byte-identical TMD pointing at the WRONG source-cell TIM in retail VRAM.
fn same_texture_output(mesh:&crate::raster::Model,donor:&legaia_tim::Vram,
    target:&legaia_tim::Vram)->bool {
    let (covered,textured)=mesh.texture_coverage(donor);
    let (target_covered,target_textured)=mesh.texture_coverage(target);
    if covered!=textured || target_covered!=target_textured || textured!=target_textured {return false;}
    let mut checked=0usize;
    for tri in mesh.indices.chunks_exact(3) {
        let [a,b,c]=[tri[0] as usize,tri[1] as usize,tri[2] as usize];
        if [a,b,c].iter().any(|&i|i>=mesh.uvs.len() || i>=mesh.textured.len() || i>=mesh.cba_tsb.len()) {return false;}
        if mesh.textured[a]!=1 || mesh.textured[b]!=1 || mesh.textured[c]!=1 {continue;}
        let [cba,tsb]=mesh.cba_tsb[a];
        let (umin,umax)=[a,b,c].iter().map(|&i|mesh.uvs[i][0] as usize).fold((usize::MAX,0),|(lo,hi),v|(lo.min(v),hi.max(v)));
        let (vmin,vmax)=[a,b,c].iter().map(|&i|mesh.uvs[i][1] as usize).fold((usize::MAX,0),|(lo,hi),v|(lo.min(v),hi.max(v)));
        checked=checked.saturating_add((umax-umin+1)*(vmax-vmin+1));
        if checked>8_000_000{return false;}
        for v in vmin..=vmax {for u in umin..=umax {
            if vram_texel(donor,cba,tsb,u,v)!=vram_texel(target,cba,tsb,u,v) {return false;}
        }}
    }
    true
}
#[cfg(test)]
mod q4_cross_cell_tests {
    use super::*;
    #[test]
    fn donors_are_not_assigned_a_synthetic_all_solid_box() {
        let pts=[[50.0,3.0,60.0],[114.0,7.0,124.0]];
        assert_eq!(point_horizontal_bounds(&pts),Some([50,114,60,124]));
        assert!(point_horizontal_bounds(&[[0.0,0.0,0.0],[4096.0,0.0,0.0]]).is_none());
        assert!(points_center(&[]).is_none());
        let blank=vec![0u8;MAP_LEN];
        assert_eq!(source_stencil(&blank,Some([50,114,60,124])).len(),0);
    }
    #[test]
    fn donor_static_bind_grid_defaults_to_rejecting_bad_anchor() {
        let blank=vec![0u8;MAP_LEN];
        let rec=ObjectRecord::parse(&blank,4).unwrap();
        assert!(!anchor_has_bind(&blank,(16,16),&rec));
        let mut binding=blank.clone();
        let offset=GRID_OFF+(16*GRID_DIM+16)*2;
        binding[offset..offset+2].copy_from_slice(&field_objects::CELL_BIND_OWNED.to_le_bytes());
        assert!(anchor_has_bind(&binding,(16,16),&rec));
    }
}
fn vram_texel(vram:&legaia_tim::Vram,cba:u16,tsb:u16,u:usize,v:usize)->u16 {
    let px=(tsb as usize & 15)*64;
    let py=((tsb as usize>>4)&1)*256;
    let cx=(cba as usize&63)*16;
    let cy=(cba as usize>>6)&511;
    let u=u&255;let v=v&255;
    match (tsb>>7)&3 {
        0=>{let w=vram.pixel((px+u/4)&1023,(py+v)&511);
            vram.pixel((cx+((w>>((u&3)*4))&15) as usize)&1023,cy)},
        1=>{let w=vram.pixel((px+u/2)&1023,(py+v)&511);
            vram.pixel((cx+((w>>((u&1)*8))&255) as usize)&1023,cy)},
        _=>vram.pixel((px+u)&1023,(py+v)&511),
    }
}

/// `collision_preview` relaxes only irrelevant editor-preview restrictions;
/// native IMG writing ALWAYS goes through the strict prepare() path.
fn prepare_with_mode(scene: &AssembledScene, world: &WorldModel, collision_preview: bool) -> Result<ExportEdits> {
    let overlay = scene_instances::serialize(&scene.name, world);
    let doc = scene_instances::parse(&overlay, &scene.name, world)
        .map_err(anyhow::Error::msg)?;
    // Cross-cell TMD/texture/MAP resource dependency import is NOT implemented.
    // Never write an IMG missing visible foreign objects from its LCK scene.
    if !collision_preview {
        ensure!(doc.foreign.is_empty(),
            "This LCK scene contains cross-cell asset previews. Native IMG export is disabled until donor TMD/TIM resource relocation and collision serialization are implemented. Save the .lckscene instead.");
    }
    // Foreign asset rotations/scales are irrelevant to the native MAP wall
    // overlay; unsupported rotations/scales of a NATIVE object still block it.
    // IMG export is unchanged: ALL non-native TMD dependencies remain forbidden.
    ensure!(!has_unhandled_native_transforms(&doc, collision_preview),
        "Native rotation/scale requires a native transform/TMD export stage; reset rotation/scale or save .lckscene to preserve the edit. Stage M2 changes placement freedoms only.");
    ensure!(doc.hidden.is_empty(), "Native object hiding/deletion is not supported yet.");
    let mut result=ExportEdits::default();
    // NPC clip and model selections are represented in WorldModel; the parsed
    // overlay has already checked model/animation compatibility.
    // Scene actor edit controls are already wired to the MAN catalog.
    // Original actor appearances and positions must be saved even when no
    // environmental MAP objects are changed.
    for npc in world.npcs.iter().filter(|n|!n.is_copy) {
        // NPCs use MAN spawn/script records, not the static MAP wall nibble grid.
        // Their editor state must not hide otherwise valid wall previews.
        if collision_preview {continue;}
        let draw=world.at(false,npc.key).context("NPC hierarchy draw missing")?;
        let altered=draw.offset.iter().any(|v|v.abs()>EPS)
            || npc.model_index!=npc.original_model_index || npc.clip!=npc.anim_id;
        if !altered {continue;}
        ensure!(!npc.conditional && npc.drawable,
            "NPC MAN #{} is conditional or not renderable; cannot rewrite its native spawn",npc.record);
        ensure!(npc.frame==0,"NPC MAN #{} preview frame {} cannot persist in native spawn; reset frame to 0",npc.record,npc.frame);
        result.npcs.push(NpcEdit {
            uid:None,record:npc.record,
            source_position:(npc.psx_position[0].round() as i32,npc.psx_position[2].round() as i32),
            original_model:npc.original_model_index,original_anim:npc.anim_id,
            model:npc.model_index,anim:npc.clip,delta:draw.offset,kind:npc.kind,
        });
    }
    // A native MAP's 4-bit tier indexes the script's 16-entry floor-height
    // ladder. Ground position Y already contains -LUT[tier] in PSX space.
    if let Some(hf)=&scene.ground {
        for (&tier,pos) in hf.corner_tiers.iter().zip(&hf.positions) {
            let idx=(tier & 15) as usize;
            if pos[1].is_finite() {
                let value=pos[1].round() as i32;
                if let Some(existing)=result.tier_heights[idx] {
                    ensure!(existing==value,"Floor tier {idx} varies across the assembled scene; cannot re-anchor reliably");
                } else {result.tier_heights[idx]=Some(value);}
            }
        }
    }
    for ((terrain,index),delta) in doc.moves {
        ensure!(!terrain,"Terrain edits are not natively exportable");
        if world.npc((false,index)).is_some() {continue;}

        let native=scene.placements.get(index).with_context(||format!("Missing original placement #{index}"))?;
        let draw=world.at(false,index).context("Original source is absent from hierarchy")?;
        ensure!(draw.editor_uid.is_none() && !draw.hidden,"Moved source is a created or hidden object");
        ensure!(delta.iter().all(|v|v.is_finite()) && delta.iter().any(|v|v.abs()>EPS),"Bad native movement delta");
        result.moves.push(MoveEdit { collision_bounds:original_collision_bounds(world,index), cell:native.cell,
            source_position:(native.world_x,native.world_z), env_slot:native.env_slot,
            source_rotation:[native.rot_x,native.rot_y,native.rot_z], delta });
    }
    for created in doc.created {
        ensure!(!created.source.0,"Native terrain copies are not supported; select a placed environmental object");
        let draw=world.draws.iter().find(|d|d.editor_uid==Some(created.uid))
            .with_context(||format!("Editor copy UID {} disappeared",created.uid))?;
        if let Some(npc)=world.npc(created.source) {
            if collision_preview {continue;}
            ensure!(!npc.conditional && npc.drawable,"Cannot export an unavailable NPC source");
            let actor=world.npc((false,draw.index)).context("NPC copy has no appearance record")?;
            ensure!(actor.frame==0,"NPC copy UID {} preview frame {} is not a native spawn frame; reset to 0",created.uid,actor.frame);
            ensure!(!draw.hidden && draw.source_ref==Some(created.source),"NPC clone source mismatch");
            result.npcs.push(NpcEdit {
                uid:Some(created.uid),record:npc.record,
                source_position:(npc.psx_position[0].round() as i32,npc.psx_position[2].round() as i32),
                original_model:npc.original_model_index,original_anim:npc.anim_id,
                model:actor.model_index,anim:actor.clip,delta:created.offset,kind:npc.kind,
            });
            continue;
        }
        let source=scene.placements.get(created.source.1)
            .with_context(||format!("No original placed object for editor copy #{}",created.uid))?;
        ensure!(!draw.hidden && draw.source_ref==Some(created.source),"Editor copy #{0} has unsupported source/hiding",created.uid);
        ensure!(created.offset.iter().all(|v|v.is_finite()),"Copy has invalid coordinates");
        result.copies.push(InsertEdit { collision_bounds:original_collision_bounds(world,created.source.1),
            uid:created.uid,cell:source.cell,
            source_position:(source.world_x,source.world_z),env_slot:source.env_slot,
            source_rotation:[source.rot_x,source.rot_y,source.rot_z],delta:created.offset });
    }
    if !collision_preview {
        ensure!(!result.moves.is_empty() || !result.copies.is_empty() || !result.npcs.is_empty(),
            "No native objects changed. Move, duplicate or change a native environment object / NPC.");
        ensure!(result.npcs.iter().filter(|n|n.uid.is_some()).count()<=4,
            "NPC insertion is experimental: export at most 4 new actors per build");
        // Native IMG insertion count remains restricted; only the independent
        // wireframe preview may visualize further placements without exporting.
        ensure!(result.copies.len()<=16,"Stage M limits a test export to 16 added objects per cell");
    }
    Ok(result)
}

fn quantize(value:f32, name:&str) -> Result<i32> {
    ensure!(value.is_finite() && value.abs()<=16384.0,"{name} movement out of test limits (+/-16384 native units)");
    let v = value.round();
    ensure!((v-value).abs()<=EPS,"{name} movement must be a whole PS1 world unit; use grid mode or an integer inspector offset");
    Ok(v as i32)
}

fn record_at(data:&[u8], cell:(u8,u8), pos:(i32,i32),
             mesh:usize, angles:[u16;3]) -> Result<(usize,ObjectRecord)> {
    ensure!(data.len()==MAP_LEN,"Native field MAP expected 0x12000 bytes");
    let (col,row)=cell;
    let grid_off=GRID_OFF+(row as usize*GRID_DIM+col as usize)*2;
    let word=u16::from_le_bytes(data[grid_off..grid_off+2].try_into()?);
    let id=(word&field_objects::OBJECT_INDEX_MASK) as usize;
    let r=ObjectRecord::parse(data,id).context("MAP object record absent")?;
    ensure!(r.is_placed(),"Source cell ({col},{row}) is not a native placed object");
    ensure!(field_objects::pack_mesh_index(id as u16,&r)==Some(mesh as u16),
        "Mesh selector mismatch on source MAP record #{id}");
    ensure!(field_objects::world_x(col,r.x_off)==pos.0 && field_objects::world_z(row,r.z_off)==pos.1,
        "Object #{id} changed position since LCK loaded this scene");
    ensure!([r.rot_x,r.rot_y,r.rot_z]==angles,"Source record #{id} has different native rotation");
    Ok((id,r))
}

fn source_collision(r:&ObjectRecord,col:u8,row:u8)->(i32,i32) {
    let (cx,cz)=field_objects::collision_footprint_offset(r);
    (field_objects::world_x(col,r.x_off)+cx,
     field_objects::world_z(row,r.z_off)+cz)
}

/// Fit the PS1's 16-unit sub-anchors without requiring editor grid snapping.
/// When correction flag 0x8 is clear, the collider already travels with the
/// mesh exactly, so there is no offset rewrite. When set, its offset uses
/// 16-unit quantized signed sub-anchors; we preserve the flag and fit within
/// the native signed range. Rounding error is at most 8 units per axis.
fn adjust_collider(r:&ObjectRecord, original:&ObjectRecord,
                   bytes:&mut [u8;32]) -> Result<()> {
    if original.flags & field_objects::FLAG_COLLIDER_CORRECTION == 0 { return Ok(()); }
    let dx=r.x_off as i32-original.x_off as i32;
    let dz=original.z_off as i32-r.z_off as i32;
    let sx=(original.sub_anchor_x as i32 * 16 + dx) as f64 / 16.0;
    let sz=(original.sub_anchor_z as i32 * 16 + dz) as f64 / 16.0;
    // Native collider offsets cannot represent arbitrarily far displacements.
    // Prevent silent overflow rather than generating corrupted actors.
    let ix=sx.round() as i32;
    let iz=sz.round() as i32;
    let ix=i8::try_from(ix).context("X collider correction exceeded native 8-bit reach")?;
    let iz=i8::try_from(iz).context("Z collider correction exceeded native 8-bit reach")?;
    bytes[14]=ix as u8;bytes[15]=iz as u8;
    Ok(())
}

fn move_originals(map:&mut [u8], edits:&[MoveEdit]) -> Result<Vec<usize>> {
    let mut touched=HashSet::new();
    let mut refs=[0u16;512];
    for i in 0..GRID_DIM*GRID_DIM {
        let o=GRID_OFF+i*2;
        let idx=(u16::from_le_bytes([map[o],map[o+1]])&0x1FF) as usize;
        refs[idx]+=1;
    }
    let mut changed=Vec::new();
    for edit in edits {
        let (id,r)=record_at(map,edit.cell,edit.source_position,edit.env_slot,edit.source_rotation)?;
        ensure!(touched.insert(id),"Multiple moves share native record #{id}");
        ensure!(refs[id]==1,"Record #{id} is referenced by {} cells; cannot move it independently",refs[id]);
        let dx=quantize(edit.delta[0],"X")?;
        let dy=quantize(edit.delta[1],"Y")?;
        let dz=quantize(edit.delta[2],"Z")?;
        let mut next=[0u8;32];next.copy_from_slice(&map[id*32..id*32+32]);
        let x=i16::try_from(r.x_off as i32+dx).context("X moved beyond i16 native offset")?;
        let y=i16::try_from(r.y_off as i32-dy).context("Y moved beyond i16 native offset")?;
        let z=i16::try_from(r.z_off as i32-dz).context("Z moved beyond i16 native offset")?;
        next[0..2].copy_from_slice(&x.to_le_bytes());
        next[2..4].copy_from_slice(&y.to_le_bytes());
        next[4..6].copy_from_slice(&z.to_le_bytes());
        let moved=ObjectRecord::parse(&next,0).context("Moved record invalid")?;
        adjust_collider(&moved,&r,&mut next)?;
        let moved=ObjectRecord::parse(&next,0).context("Moved collision record invalid")?;
        let old_c=source_collision(&r,edit.cell.0,edit.cell.1);
        let new_c=source_collision(&moved,edit.cell.0,edit.cell.1);
        ensure!((new_c.0-old_c.0-dx).abs()<=8 && (new_c.1-old_c.1-dz).abs()<=8,
            "Native static collider moved outside the engine's 16-unit precision");
        map[id*32..id*32+32].copy_from_slice(&next);
        changed.push(id);
    }
    Ok(changed)
}

/// V2 anchor policy: use the retail window-range allocation first. If there
/// are NO valid cells inside 3 tiles, try an extended range so an editor copy
/// can still be tested in a standalone test IMG. The extended placement may
/// pop in or disappear outside the retail window sweep -- not guaranteed.
const LCK_SAFE_ANCHOR_REACH: i32 = 384;
const LCK_TEST_ANCHOR_REACH: i32 = 1024;
const LCK_EXTENDED_ANCHOR_PENALTY: i32 = 1_000_000;

fn anchor_score(xoff:i32,zoff:i32)->Option<i32> {
    if xoff.abs()>LCK_TEST_ANCHOR_REACH || zoff.abs()>LCK_TEST_ANCHOR_REACH {
        return None;
    }
    let risk=if xoff.abs()>LCK_SAFE_ANCHOR_REACH || zoff.abs()>LCK_SAFE_ANCHOR_REACH {
        LCK_EXTENDED_ANCHOR_PENALTY
    } else {0};
    Some(risk+xoff.abs()+zoff.abs())
}

#[derive(Clone,Debug)]
struct Inserted { id:usize, col:u8, row:u8, uid:u64, collider:(i32,i32) }

fn insert_copies(map:&mut [u8], source_map:&[u8], edits:&[InsertEdit],
                 tiers:&[Option<i32>;16]) -> Result<Vec<Inserted>> {
    let mut used=[false;512];
    for i in 0..GRID_DIM*GRID_DIM {
        let o=GRID_OFF+i*2;
        let id=(u16::from_le_bytes(map[o..o+2].try_into()?)&0x1FF) as usize;
        used[id]=true;
    }
    let mut done=Vec::new();
    for edit in edits {
        let (source_idx,src)=record_at(source_map,edit.cell,edit.source_position,
            edit.env_slot,edit.source_rotation)
            .with_context(||format!("Validate original for copy #{}",edit.uid))?;
        ensure!(source_idx>3,"Player/NPC source cannot be copied as static environment geometry");
        let slot=(4..512usize).rev().find(|&i| !used[i] &&
            map[i*32..i*32+32].iter().all(|&b|b==0))
            .context("No completely blank and unreferenced 32-byte native object slot remains")?;
        let dx=quantize(edit.delta[0],"Copy X")?;
        let dy=quantize(edit.delta[1],"Copy Y")?;
        let dz=quantize(edit.delta[2],"Copy Z")?;
        let wx=edit.source_position.0.checked_add(dx).context("Copy X overflow")?;
        let wz=edit.source_position.1.checked_add(dz).context("Copy Z overflow")?;
        let source_tier=(source_map[0x4000+edit.cell.1 as usize*GRID_DIM+edit.cell.0 as usize]&0x0f) as usize;
        let mut best:Option<(i32,usize,usize,[u8;32])>=None;
        let mut blank=0usize;
        let mut in_reach=0usize;
        let mut usable_floor=0usize;
        let mut collider_fit=0usize;
        // Search for a SPAWN ANCHOR, not an unoccupied world-space collision
        // volume. A clone may intersect any number of visual/collision meshes;
        // the native grid still can only store one object reference per tile.
        for row in 0..GRID_DIM {
            for col in 0..GRID_DIM {
                let go=GRID_OFF+(row*GRID_DIM+col)*2;
                // Never displace terrain, decals, scripts, walkable ground,
                // an existing static object, or a saved field trigger.
                if map[go]!=0 || map[go+1]!=0 {continue;}
                blank+=1;
                let anchor_col=col as i32+src.col_delta as i32;
                let anchor_row=row as i32+src.row_delta as i32;
                if !(0..128).contains(&anchor_col) || !(0..128).contains(&anchor_row) {continue;}
                let anchor=GRID_OFF+(anchor_row as usize*GRID_DIM+anchor_col as usize)*2;
                let word=u16::from_le_bytes(map[anchor..anchor+2].try_into()?);
                if word&field_objects::CELL_BIND_OWNED!=0 {continue;}
                let xo=wx-(col as i32*128+64);
                let zo=(row as i32*128+64)-wz;
                // Prefer the proven three-tile window. Only if that cannot
                // accommodate this copy may we try extended TEST anchors.
                let Some(score)=anchor_score(xo,zo) else {continue};
                in_reach+=1;
                let Ok(x)=i16::try_from(xo) else {continue};
                let Ok(z)=i16::try_from(zo) else {continue};
                let candidate_tier=(map[0x4000+row*GRID_DIM+col]&15) as usize;
                let floor_correction=if source_tier==candidate_tier {0}
                    else if let (Some(src_h),Some(dst_h))=(tiers[source_tier],tiers[candidate_tier]) {
                        src_h-dst_h
                    } else {continue};
                // Retail: PSX Y = floor_height(tile) + record.y_off.
                // Compensating the difference allows above/below-ground
                // placements without modifying terrain or floor lookup.
                let Ok(y)=i16::try_from(src.y_off as i32-dy+floor_correction) else {continue};
                usable_floor+=1;
                let mut bytes=[0u8;32];
                bytes.copy_from_slice(&source_map[source_idx*32..source_idx*32+32]);
                bytes[0..2].copy_from_slice(&x.to_le_bytes());
                bytes[2..4].copy_from_slice(&y.to_le_bytes());
                bytes[4..6].copy_from_slice(&z.to_le_bytes());
                let next=ObjectRecord::parse(&bytes,0).context("Clone record invalid")?;
                if adjust_collider(&next,&src,&mut bytes).is_err() {continue;}
                let next=ObjectRecord::parse(&bytes,0).context("Adjusted clone invalid")?;
                let old=source_collision(&src,edit.cell.0,edit.cell.1);
                let new=source_collision(&next,col as u8,row as u8);
                // For corrected colliders, 16-unit quantization can round
                // up/down by 8 units. That's a property of PS1 records.
                if (new.0-old.0-dx).abs()>8 || (new.1-old.1-dz).abs()>8 {continue;}
                collider_fit+=1;
                if best.as_ref().map_or(true,|b|score<b.0) {
                    best=Some((score,col,row,bytes));
                }
            }
        }
        let (score,col,row,bytes)=best.with_context(||format!(
            "Copy UID {} has no encodable native spawn anchor within {} PS1 units ({} blank tiles; {} within test reach; {} with known floor; {} with matching native collider). A tile can hold only one MAP object reference. Check destination world coordinates, source correction flags, and occupied grid cells.",
            edit.uid,LCK_TEST_ANCHOR_REACH,blank,in_reach,usable_floor,collider_fit))?;
        // V2 demonstrated a collision-only ghost in DuckStation. The native
        // sub-area spawn sweep uses the GRID TILE, not the distant corrected
        // mesh world position. A >3-tile anchor can therefore leave the
        // static collider/walls present while the mesh never spawns.
        // Until the real spawn window is verified and an alternative native
        // allocation scheme exists, never report a ghost as a valid IMG.
        ensure!(score<LCK_EXTENDED_ANCHOR_PENALTY,
            "Copy UID {} would use a distant native MAP spawn tile ({},{}) for object world ({},{}). V2's extended anchor produced collision with invisible geometry in DuckStation. No IMG written. Native renderer-safe placement is not available near this position; move the copy toward a free nearby native tile or wait for native actor-spawn support.",
            edit.uid,col,row,wx,wz);
        map[slot*32..slot*32+32].copy_from_slice(&bytes);
        let grid=GRID_OFF+(row*GRID_DIM+col)*2;
        map[grid..grid+2].copy_from_slice(&(slot as u16).to_le_bytes());
        used[slot]=true;
        let rec=ObjectRecord::parse(&bytes,0).context("Inserted copy invalid")?;
        done.push(Inserted{id:slot,col:col as u8,row:row as u8,uid:edit.uid,
            collider:source_collision(&rec,col as u8,row as u8)});
    }
    let placed=field_objects::parse_placements(map);
    for copy in &done {
        let found=placed.iter().find(|p|p.obj_idx as usize==copy.id &&
            p.col==copy.col && p.row==copy.row)
            .with_context(||format!("Inserted copy UID {} not recognized by native placement parser",copy.uid))?;
        ensure!((found.collider_x,found.collider_z)==copy.collider,
            "Inserted UID {} collision center doesn't round-trip",copy.uid);
        ensure!(found.anchor_cell&field_objects::CELL_BIND_OWNED==0,
            "Copied object was incorrectly marked MAN-bind-owned");
    }
    Ok(done)
}

/// Native patch output plus coverage details: vital for Rim Elm story variants.
#[derive(Clone, Debug)]
pub(super) struct ExportResult {
    pub path: PathBuf,
    pub patched: Vec<String>,
    pub skipped: Vec<String>,
}

/// PS1 Rim Elm is not one cell: story states town01 / town0b / town0c
/// use byte-identical source MAPs on the uploaded test disc, and town0d / town0e
/// have related but different layouts. Stage L patched only the selected version,
/// so a saved game in another state looked unchanged. Only edit another variant
/// when its *own* object/grid/mesh/coordinates pass the full native checks.
const RIM_ELM_MAPS: [(&str, usize); 5] = [
    ("town01", 1),
    ("town0b", 10),
    ("town0c", 19),
    ("town0d", 345),
    ("town0e", 739),
];

/// A MAN actor's tile-byte encodes centres and half-tile boundaries in 64-unit
/// increments. PS1 cannot store arbitrary single-unit NPC coordinates here.
/// Snap only the actor spawn; environmental object coordinates stay exact.
fn native_npc_coord(value:i32)->Result<(u8,i32)> {
    let mut best:Option<(i32,u8,i32)>=None;
    for tile in 0..128i32 {
        for (enc,actual) in [(tile as u8,tile*128+64),((tile as u8)|0x80,(tile+1)*128)] {
            let diff=(actual-value).abs();
            if best.is_none_or(|(d,_,_)|diff<d) {best=Some((diff,enc,actual));}
        }
    }
    let (diff,encoded,actual)=best.context("No valid native actor tile")?;
    ensure!(diff<=32,"Actor position {value} lies outside encodable MAN tile coordinates");
    Ok((encoded,actual))
}

fn man_actor_header(man:&[u8], rec:usize)->Result<(usize,man_section::ActorPlacement)> {
    let mf=man_section::parse(man).map_err(anyhow::Error::msg)?;
    let actor=mf.actor_placement(man,rec)
        .with_context(||format!("MAN actor record #{rec} not found"))?;
    let header=actor.record_offset+1+actor.local_count*2;
    ensure!(header+4<=man.len(),"Actor header out of range");
    Ok((header,actor))
}

/// Locate ONLY the relocation instruction reached by the cold-game (all flags
/// clear) spawn prologue. This is the same flag state used by the scene's NPC
/// catalog when it computes the position presented in LCK. This walker follows
/// decoded opcode boundaries, not a blind byte scan into dialogue payloads.
/// We deliberately do not rewrite alternate story-flag branches or cutscenes.
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
struct ColdSpawnSite { x_operand:usize, z_operand:usize, encoded:(u8,u8) }

fn cold_spawn_site(body:&[u8], pc0:usize)->Option<ColdSpawnSite> {
    use legaia_scene_formats::field_disasm::{decode, FlagKind, InsnInfo, MenuCtrlKind};
    if !matches!(body.get(pc0).copied(),Some(0x24 | 0x25)) {return None;}
    let mut pc=pc0;
    let mut seen=HashSet::new();
    for _ in 0..192 {
        if !seen.insert(pc) {return None;}
        let byte=*body.get(pc)?;
        if byte==0x21 || byte&0x7f<0x20 {return None;}
        let insn=decode(body,pc).ok()?;
        let own=insn.extended.is_none();
        let next=pc.checked_add(insn.size)?;
        if next>body.len() || insn.size==0 {return None;}
        match insn.info {
            // In the fresh-game catalog, every story flag is false; a TEST
            // takes its branch only when set.
            InsnInfo::SystemFlag{kind:FlagKind::Test,..}=>{},
            InsnInfo::JmpRel{target,..}=>{
                if target>=body.len(){return None;}
                pc=target;
                continue;
            },
            InsnInfo::MenuCtrl{kind:MenuCtrlKind::Nibble5NpcRun{x_enc,z_enc,..},..} if own=>{
                if body.get(pc..pc+2)!=Some(&[0x4c,0x51][..]) {return None;}
                return Some(ColdSpawnSite{x_operand:pc+2,z_operand:pc+3,encoded:(x_enc,z_enc)});
            },
            InsnInfo::MoveTo{xb,zb} if own=>{
                if byte!=0x23{return None;}
                return Some(ColdSpawnSite{x_operand:pc+1,z_operand:pc+2,encoded:(xb,zb)});
            },
            // Don't guess a cold branch when an additional conditional opcode
            // with unknown current-state semantics appears in the prologue.
            InsnInfo::CondJmp{..}|InsnInfo::BBoxTest{..}|InsnInfo::Byte{..}=>return None,
            _=>{},
        }
        pc=next;
    }
    None
}

fn man_actor_record_end(mf:&man_section::ManFile, len:usize, start:usize)->usize {
    let mut end=mf.sections.iter().map(|s|s.offset).filter(|&o|o>start).min().unwrap_or(len);
    for part in &mf.partitions {
        for &rel in part {
            let absolute=mf.data_region_offset+rel as usize;
            if absolute>start && absolute<end {end=absolute;}
        }
    }
    end.min(len)
}

fn man_edit_actor(man:&mut [u8], edit:&NpcEdit)->Result<(u8,u8)> {
    let mf=man_section::parse(man).map_err(anyhow::Error::msg)?;
    let (header,p)=man_actor_header(man,edit.record)?;
    ensure!(p.model_index==edit.original_model && p.anim_id==edit.original_anim,
        "MAN actor #{} model/ANM differs from LCK loaded source",edit.record);
    let dx=quantize(edit.delta[0],"NPC X")?;
    let dz=quantize(edit.delta[2],"NPC Z")?;
    ensure!(edit.delta[1].abs()<=EPS,
        "Native MAN NPC spawn has no Y position; reset NPC Y-offset before export");
    // The NPC catalog shows the actual position AFTER the retail actor's
    // spawn-script prologue has run. It is often NOT the header coordinate!
    // Therefore movement is relative to the observed location, not the raw
    // staging square. P1[12] in town01 stages at (3776,1856) but teleports
    // to (3904,1984) in its fresh-game prologue.
    let desired_x=edit.source_position.0.checked_add(dx).context("NPC X overflow")?;
    let desired_z=edit.source_position.1.checked_add(dz).context("NPC Z overflow")?;
    let (bx,actual_x)=native_npc_coord(desired_x)?;
    let (bz,actual_z)=native_npc_coord(desired_z)?;
    if (actual_x,actual_z)!=(desired_x,desired_z) {
        log::warn!("Native NPC #{} requested ({desired_x},{desired_z}), snapped to ({actual_x},{actual_z})",edit.record);
    }
    let end=man_actor_record_end(&mf,man.len(),p.record_offset);
    ensure!(end>p.record_offset+p.script_pc0,"NPC MAN script is truncated");
    let site=cold_spawn_site(&man[p.record_offset..end],p.script_pc0);
    if let Some(site)=site {
        let observed=(
            (site.encoded.0 as i32 & 0x7f)*128 + if site.encoded.0&0x80!=0 {128}else{64},
            (site.encoded.1 as i32 & 0x7f)*128 + if site.encoded.1&0x80!=0 {128}else{64},
        );
        ensure!(observed==edit.source_position,
            "NPC MAN #{} cold spawn-script target ({},{}) differs from LCK-observed ({},{}); refusing to overwrite an unverified script",
            edit.record,observed.0,observed.1,edit.source_position.0,edit.source_position.1);
        man[p.record_offset+site.x_operand]=bx;
        man[p.record_offset+site.z_operand]=bz;
        log::info!("NPC MAN #{}: rewrote verified cold-boot relocation in script and spawn header",edit.record);
    } else {
        ensure!((p.world_x as i32,p.world_z as i32)==edit.source_position,
            "NPC MAN #{} is script-repositioned: header ({},{}) vs visible ({},{}), but no safe cold-boot relocation instruction was decoded",
            edit.record,p.world_x,p.world_z,edit.source_position.0,edit.source_position.1);
    }
    man[header]=edit.model;
    man[header+1]=edit.anim;
    man[header+2]=bx;
    man[header+3]=bz;
    Ok((bx,bz))
}

/// Copy an entire MAN partition-1 record including its native VM bytecode;
/// add one new P1 table entry and insert the cloned record before section 0.
/// All existing record offsets remain unchanged relative to the shifted table;
/// the section-relative pointer grows by exactly the cloned record size.
/// Cloned scripts can repeat original dialog/side effects. No synthesized AI.
fn man_append_actor(man:&[u8],record:usize,spawn:[u8;4], source_override:Option<&[u8]>)->Result<Vec<u8>> {
    let mf=man_section::parse(man).map_err(anyhow::Error::msg)?;
    let n0=mf.partitions[0].len();
    let n1=mf.partitions[1].len();
    ensure!(n1<64,"Refusing to exceed provisional 64 MAN actor slots (runtime capacity is not yet fully verified)");
    let source=mf.actor_placement(man,record).context("Clone actor missing")?;
    let src_start=source.record_offset;
    let mut boundaries=vec![mf.sections[0].offset];
    for part in &mf.partitions {
        for &off in part {
            let abs=mf.data_region_offset+off as usize;
            if abs>src_start {boundaries.push(abs);}
        }
    }
    let src_end=*boundaries.iter().min().context("Cannot find actor record end")?;
    ensure!(src_end>src_start+source.script_pc0,"MAN source actor lacks script body");
    let mut copy=source_override.map(|s|s.to_vec())
        .unwrap_or_else(||man[src_start..src_end].to_vec());
    ensure!(copy.len()==src_end-src_start,"NPC source override record length mismatch");
    let hdr=1+source.local_count*2;
    copy[hdr..hdr+4].copy_from_slice(&spawn);
    let section=mf.sections[0].offset;
    ensure!(section==mf.data_region_offset+mf.header.u24_at_28 as usize,"MAN section0 offset inconsistent");
    ensure!(src_end<=section && section<=man.len(),"MAN record overlaps section0");
    let copy_len=copy.len();
    let relative=mf.header.u24_at_28 as usize;
    ensure!(relative<0x1000000 && relative+copy_len<0x1000000,"MAN section relative offset overflow");
    let table=man_section::RECORDS_BEGIN_OFFSET+3*(n0+n1);
    let mut result=Vec::with_capacity(man.len()+3+copy_len);
    result.extend_from_slice(&man[..table]);
    result.extend_from_slice(&[(relative&255) as u8,((relative>>8)&255) as u8,((relative>>16)&255) as u8]);
    result.extend_from_slice(&man[table..section]);
    result.extend_from_slice(&copy);
    result.extend_from_slice(&man[section..]);
    result[0x24..0x26].copy_from_slice(&((n1+1) as i16).to_le_bytes());
    let next=relative+copy_len;
    result[0x28..0x2b].copy_from_slice(&[(next&255) as u8,((next>>8)&255) as u8,((next>>16)&255) as u8]);
    let reloaded=man_section::parse(&result).map_err(anyhow::Error::msg)?;
    let duplicate=reloaded.actor_placement(&result,n1).context("New MAN actor absent after round trip")?;
    ensure!([duplicate.model_index,duplicate.anim_id,result[duplicate.record_offset+1+duplicate.local_count*2+2],
        result[duplicate.record_offset+1+duplicate.local_count*2+3]]==spawn,
        "Native MAN duplicate round-trip differs");
    ensure!(reloaded.sections[0].offset==mf.sections[0].offset+3+copy_len,
        "MAN section 0 displacement mismatch");
    // Source VM scripts, section data and all original actor records remain byte-identical.
    for idx in 1..n1 {
        let old=mf.actor_placement(man,idx).context("Old MAN actor missing")?;
        let new=reloaded.actor_placement(&result,idx).context("Existing MAN actor vanished")?;
        ensure!(old.model_index==new.model_index && old.anim_id==new.anim_id && old.world_x==new.world_x && old.world_z==new.world_z,
            "Cloning mutated preexisting MAN actor #{idx}");
    }
    Ok(result)
}

fn compile_man_npcs(original:&[u8],edits:&[NpcEdit])->Result<Vec<u8>> {
    ensure!(!edits.is_empty(),"No NPC edits");
    let mut updated=original.to_vec();
    let mut touched=HashSet::new();
    // Edit original actors first; copies must always use original source scripts.
    for e in edits.iter().filter(|e|e.uid.is_none()) {
        ensure!(touched.insert(e.record),"Multiple edits to MAN actor #{}",e.record);
        man_edit_actor(&mut updated,e)?;
    }
    for e in edits.iter().filter(|e|e.uid.is_some()) {
        // Give every duplicate its own independently moved spawn prologue.
        // The existing append path copied its record from `updated`, ignoring
        // the relocation patched in `temp`, so duplicates still walked home.
        let mut temp=original.to_vec();
        let (bx,bz)=man_edit_actor(&mut temp,e)?;
        let mf=man_section::parse(original).map_err(anyhow::Error::msg)?;
        let actor=mf.actor_placement(original,e.record).context("Clone source MAN record absent")?;
        let from=actor.record_offset;
        let to=man_actor_record_end(&mf,original.len(),from);
        ensure!(to>from,"Invalid source record bounds");
        updated=man_append_actor(&updated,e.record,[e.model,e.anim,bx,bz],Some(&temp[from..to]))?;
    }
    Ok(updated)
}

/// Find the selected scene's native MAN-bearing bundle, which is a separate
/// PROT entry from the MAP. Validate exactly one bundle in the nearby block.
fn find_native_man(disc:&DiscPatcher,map_entry:usize)->Result<(usize,usize,usize,usize,Vec<u8>)> {
    let mut options=Vec::new();
    for idx in map_entry+1..map_entry+7 {
        let Ok(entry)=disc.read_entry_footprint(idx) else {break};
        let Some(table)=scene_asset_table::detect(&entry) else {continue};
        let Some(slot)=table.descriptor_index(3) else {continue};
        let descriptor=table.used()[slot];
        let off=descriptor.data_offset as usize;
        if off==0 || descriptor.size==0 {continue;}
        let Some(bytes)=entry.get(off..) else {continue};
        let Ok((decoded,consumed))=legaia_lzs::decompress_tracked(bytes,descriptor.size as usize) else {continue};
        let budget=table.used().iter().map(|d|d.data_offset as usize)
            .filter(|&x|x>off).min().map(|next|next-off).unwrap_or(consumed);
        if man_section::parse(&decoded).is_ok() {options.push((idx,off,budget,slot,decoded));}
    }
    ensure!(options.len()==1,"Expected exactly one nearby scene MAN for MAP PROT #{map_entry}; found {}. Refusing wrong-scene actor patch",options.len());
    Ok(options.remove(0))
}

/// Live retail wall query in FUN_801CFE4C is biased differently from the
/// ordinary floor tile read. A wall nibble at sub-cell (sx,sz) blocks near
/// world X=sx*64+32, world Z=(sz-2)*64+32. For stencil ownership, use
/// *these* physical coordinates, rather than raw MAP row*128.
fn wall_world_center(sx:i32,sz:i32)->(i32,i32) {
    (sx*SUB_SIZE+32,(sz-2)*SUB_SIZE+32)
}

fn wall_offset(sx:i32,sz:i32)->Option<(usize,u8)> {
    if !(0..SUB_DIM as i32).contains(&sx) || !(0..SUB_DIM as i32).contains(&sz) {return None;}
    let offset=WALL_OFF + (sz as usize/2)*GRID_DIM + sx as usize/2;
    let bit=1u8 << ((sx&1)+2*(sz&1));
    Some((offset,bit<<4))
}

/// Extract only already-authored wall bits touched by the selected mesh's
/// horizontal projection. NOT synthetic "all-solid AABB" collision, because
/// that would erase the original game's doors, stair gaps and open space.
fn source_stencil(map:&[u8],bounds:Option<[i32;4]>)->Vec<(i32,i32)> {
    let Some([minx,maxx,minz,maxz])=bounds else{return Vec::new()};
    let minx=minx-WALL_BOUND_PAD;
    let maxx=maxx+WALL_BOUND_PAD;
    let minz=minz-WALL_BOUND_PAD;
    let maxz=maxz+WALL_BOUND_PAD;
    let mut bits=Vec::new();
    let min_sx=((minx-32).div_euclid(SUB_SIZE)).clamp(0,SUB_DIM as i32-1);
    let max_sx=((maxx-32).div_euclid(SUB_SIZE)).clamp(0,SUB_DIM as i32-1);
    // Runtime wall lookup has an intentional +2 sub-cell bias on Z.
    let min_sz=((minz-32).div_euclid(SUB_SIZE)+2).clamp(0,SUB_DIM as i32-1);
    let max_sz=((maxz-32).div_euclid(SUB_SIZE)+2).clamp(0,SUB_DIM as i32-1);
    for sz in min_sz..=max_sz {for sx in min_sx..=max_sx {
        let (wx,wz)=wall_world_center(sx,sz);
        if wx<minx || wx>maxx || wz<minz || wz>maxz {continue;}
        if let Some((off,bit))=wall_offset(sx,sz) {
            if map[off]&bit!=0 {bits.push((sx,sz));}
        }
    }}
    bits
}

/// The engine's grid can only represent walls at 64-unit spacing. Preserve
/// the MAP's original stencil shape and snap only the COLLISION step to that
/// resolution; visual object placement remains at the full native precision.
fn collision_step(delta:f32, axis:&str)->Result<i32> {
    let movement=quantize(delta,axis)?;
    Ok((movement as f32/SUB_SIZE as f32).round() as i32)
}

#[derive(Default,Clone,Debug)]
struct WallReport {changed_bytes:usize,source_bits:usize,placed_bits:usize,cleared_bits:usize}

/// Composite all object moves/copies into one wall-grid update. Rebuild from
/// the original grid instead of edit order: destination collisions win over
/// source removals, so overlapping moved/duplicated objects remain solid.
fn compile_walls(map:&mut [u8], original:&[u8], edits:&ExportEdits)->Result<WallReport> {
    ensure!(map.len()==MAP_LEN && original.len()==MAP_LEN,"Invalid native field MAP length");
    let mut remove=vec![0u8;GRID_DIM*GRID_DIM];
    let mut add=vec![0u8;GRID_DIM*GRID_DIM];
    let mut report=WallReport::default();
    for edit in &edits.moves {
        let sx=collision_step(edit.delta[0],"Moved collision X")?;
        let sz=collision_step(edit.delta[2],"Moved collision Z")?;
        let stencil=source_stencil(original,edit.collision_bounds);
        report.source_bits+=stencil.len();
        for (x,z) in stencil {
            let (src_off,src_bit)=wall_offset(x,z).context("Source wall sub-cell out of bounds")?;
            let (dst_off,dst_bit)=wall_offset(x+sx,z+sz)
                .context("Moved collision would cross native MAP border; move object inward")?;
            // Ownership is NOT encoded in native MAP. Clearing this inferred
            // footprint can affect terrain/other objects at the old position.
            // Only the upper nibble is modified; original floor tiers remain.
            remove[src_off-WALL_OFF]|=src_bit;
            add[dst_off-WALL_OFF]|=dst_bit;
            report.placed_bits+=1;
        }
    }
    for edit in &edits.copies {
        if let Some((_,donor_bits))=edits.foreign_wall_stencils.iter().find(|(uid,_)|*uid==edit.uid) {
            report.source_bits+=donor_bits.len();
            for &(x,z) in donor_bits {
                let (dst_off,dst_bit)=wall_offset(x,z).context("Imported donor wall crosses destination MAP")?;
                add[dst_off-WALL_OFF]|=dst_bit;
                report.placed_bits+=1;
            }
            continue;
        }
        let sx=collision_step(edit.delta[0],"Duplicate collision X")?;
        let sz=collision_step(edit.delta[2],"Duplicate collision Z")?;
        let stencil=source_stencil(original,edit.collision_bounds);
        report.source_bits+=stencil.len();
        for (x,z) in stencil {
            let (dst_off,dst_bit)=wall_offset(x+sx,z+sz)
                .context("Duplicate collision would cross native MAP border; move inward")?;
            add[dst_off-WALL_OFF]|=dst_bit;
            report.placed_bits+=1;
        }
    }
    for i in 0..GRID_DIM*GRID_DIM {
        let old=original[WALL_OFF+i];
        let before=old&0xf0;
        let updated=(before & !remove[i]) | add[i];
        report.cleared_bits+=(before & !updated).count_ones() as usize;
        if updated!=before {report.changed_bytes+=1;}
        map[WALL_OFF+i]=(old&0x0f)|updated;
    }
    Ok(report)
}

/// Return native MAP writes only after checking every other byte remains
/// untouched. Object slots and grid links are validated independently.
fn map_patches(original: &[u8], edits: &ExportEdits) -> Result<Vec<(usize,Vec<u8>)>> {
    ensure!(original.len()==MAP_LEN,"MAP must be exactly 0x12000 bytes");
    let mut updated=original.to_vec();
    let moved=move_originals(&mut updated,&edits.moves)?;
    let inserted=insert_copies(&mut updated,original,&edits.copies,&edits.tier_heights)?;
    let wall_report=compile_walls(&mut updated,original,edits)?;
    for i in WALL_OFF..GRID_OFF {
        ensure!(original[i]&0x0f==updated[i]&0x0f,
            "Floor-tier nibble changed unexpectedly at MAP byte {i:#x}");
    }
    log::info!("Native wall grid: {} source bits, {} destination bits, {} cleared bits, {} changed cells", 
        wall_report.source_bits,wall_report.placed_bits,
        wall_report.cleared_bits,wall_report.changed_bytes);
    ensure!(original[0x10000..]==updated[0x10000..],"MAP tail unexpectedly changed");
    let mut allowed=vec![false;MAP_LEN];
    for i in WALL_OFF..GRID_OFF {if original[i]!=updated[i] {allowed[i]=true;}}
    for idx in moved {
        let off=idx*32;
        for p in off..off+6 {allowed[p]=true;}
        for p in off+14..off+16 {allowed[p]=true;}
    }
    for copy in &inserted {
        for p in copy.id*32..copy.id*32+32 {allowed[p]=true;}
        let o=GRID_OFF+(copy.row as usize*GRID_DIM+copy.col as usize)*2;
        for p in o..o+2 {allowed[p]=true;}
    }
    for i in 0..MAP_LEN {
        ensure!(original[i]==updated[i] || allowed[i],
            "Exporter unexpectedly changed forbidden MAP byte at {i:#x}");
    }
    let mut patches=Vec::new();
    for i in WALL_OFF..GRID_OFF {
        if original[i]!=updated[i] { patches.push((i,vec![updated[i]])); }
    }
    for idx in 0..512 {
        let o=idx*32;
        if original[o..o+32]!=updated[o..o+32] {
            patches.push((o,updated[o..o+32].to_vec()));
        }
    }
    for i in 0..GRID_DIM*GRID_DIM {
        let o=GRID_OFF+i*2;
        if original[o..o+2]!=updated[o..o+2] {
            patches.push((o,updated[o..o+2].to_vec()));
        }
    }
    ensure!(!patches.is_empty(),"No actual native changes could be compiled");
    Ok(patches)
}

/// Build a new whole-disc PS1 image, including native Mode-2 EDC/ECC fixups.
/// The input image is only read. The output must not already exist.
/// Rim Elm source scenes automatically update compatible story-state variants.
pub(super) fn export(source:&Path, output:&Path, map_entry:usize, edits:&ExportEdits) -> Result<ExportResult> {
    ensure!(!edits.moves.is_empty() || !edits.copies.is_empty() || !edits.npcs.is_empty(),"Nothing to export");
    let source=source.canonicalize().context("Cannot find opened test image")?;
    ensure!(source.is_file(),"Open a raw 2352-byte .img/.bin, not an extracted folder");
    ensure!(source.extension().and_then(|s|s.to_str()).is_some_and(|s|s.eq_ignore_ascii_case("img") || s.eq_ignore_ascii_case("bin")),
        "Native export requires a 2352-byte .img or .bin source");
    let output_abs=if output.is_absolute(){output.to_path_buf()}else{std::env::current_dir()?.join(output)};
    ensure!(output_abs!=source,"Refusing to overwrite the open source image");
    ensure!(!output_abs.exists(),"Destination already exists: {}. Choose a new filename to protect earlier tests.",output_abs.display());
    let parent=output_abs.parent().context("Output image has no parent directory")?;
    ensure!(parent.is_dir(),"Output directory does not exist");
    let staging=output_abs.with_extension("lckbuilding");
    ensure!(!staging.exists(),"An unfinished staging file exists at {}. Inspect it before retrying.",staging.display());
    let image=std::fs::read(&source).with_context(||format!("Read source image {}",source.display()))?;
    ensure!(image.len()%2352==0,"Source is not raw Mode 2 / 2352-byte sector layout");
    let image_length=image.len();
    let mut disc=DiscPatcher::open(image).context("Parse disc ISO and PROT archive")?;

    let has_map_edits=!edits.moves.is_empty() || !edits.copies.is_empty();
    let rim_elm=RIM_ELM_MAPS.iter().any(|(_,id)|*id==map_entry);
    let targets:Vec<(&str,usize)>=if rim_elm {RIM_ELM_MAPS.to_vec()} else {vec![("selected",map_entry)]};
    // IMPORTANT: validate every candidate against *its own* original MAP before
    // touching any sector. A variant with a different building cannot be forced
    // to use the coordinates/model of a similar-looking source placement.
    let mut plans:Vec<(String, usize, Vec<(usize,Vec<u8>)>)>=Vec::new();
    let mut skipped=Vec::new();
    for (name,idx) in targets {
        if !has_map_edits {break;}
        let planned=(|| -> Result<Vec<(usize,Vec<u8>)>> {
            let original=disc.read_entry_footprint(idx)
                .with_context(||format!("Read native MAP for {name} / PROT {idx}"))?;
            map_patches(&original,edits)
        })();
        match planned {
            Ok(records)=>plans.push((name.to_string(),idx,records)),
            Err(e) if idx!=map_entry => {
                skipped.push(format!("{name} (PROT {idx}) NOT modified: {e:#}"));
            }
            Err(e)=>bail!("Selected scene's own MAP did not validate: {e:#}"),
        }
    }
    if has_map_edits {
        ensure!(plans.iter().any(|(_,idx,_)|*idx==map_entry),"Selected MAP was not planned");
    }
    // Never silently export only the wrong Rim Elm story state. A normal
    // story save most often loads town0b or town0c rather than town01.
    if rim_elm && has_map_edits {
        let core=[1usize,10,19];
        for idx in core {
            ensure!(plans.iter().any(|(_,found,_)|*found==idx),
                "Cannot export all three principal Rim Elm variants (town01/town0b/town0c). Source IMG has differing original objects. {}",
                skipped.join(" | "));
        }
    }

    let mut names=Vec::new();
    for (name,idx,records) in &plans {
        for (off,bytes) in records {
            disc.patch_prot_entry(*idx,*off as u64,bytes)
                .with_context(||format!("Patch {name} native MAP record at {off:#x}"))?;
        }
        // Verify complete moved object records and inserted grid links after physical sector updates.
        let verified=disc.read_entry_footprint(*idx)?;
        for (off,bytes) in records {
            ensure!(verified.get(*off..*off+bytes.len())==Some(bytes.as_slice()),
                "Native write verification failed in {name} at {off:#x}");
        }
        let walls=records.iter().filter(|(off,_)| *off>=WALL_OFF && *off<GRID_OFF).count();
        names.push(format!("{name} ({} MAP writes, {} wall-grid bytes)",records.len(),walls));
        if walls==0 {
            log::warn!("{name}: no native wall-grid bytes changed; this source may have no authored collision stencil within its mesh footprint");
        }
    }
    if !edits.npcs.is_empty() {
        let (idx,offset,budget,slot,decoded)=find_native_man(&disc,map_entry)?;
        let rewritten=compile_man_npcs(&decoded,&edits.npcs)?;
        let compressed=compress_within(&rewritten,budget)
            .context("New NPC MAN bytecode does not fit existing compressed scene bundle footprint. Reduce copies or choose a simpler source actor; no IMG written")?;
        // Preserve trailing payload/slack; do not touch neighboring assets.
        disc.patch_prot_entry(idx,offset as u64,&compressed)
            .context("Patch scene MAN compressed payload")?;
        if rewritten.len()!=decoded.len() {
            let size=scene_asset_table::encode_size_word(3,rewritten.len() as u32);
            disc.patch_prot_entry(idx,scene_asset_table::SceneAssetTable::size_word_offset(slot) as u64,&size.to_le_bytes())?;
            // Type 3 decompressed size is reflected in the bundle's total.
            let original_entry=disc.read_entry_footprint(idx)?;
            let total=u32::from_le_bytes(original_entry[4..8].try_into()?);
            let next=total.checked_sub(decoded.len() as u32)
                .and_then(|v|v.checked_add(rewritten.len() as u32))
                .context("Scene descriptor total size overflow")?;
            disc.patch_prot_entry(idx,4,&next.to_le_bytes())?;
        }
        let test=disc.read_entry_footprint(idx)?;
        let (check,_)=legaia_lzs::decompress_tracked(&test[offset..],rewritten.len())
            .context("MAN decompression failed after physical disc patch")?;
        ensure!(check==rewritten,"Native MAN disc write did not round-trip");
        let clones=edits.npcs.iter().filter(|e|e.uid.is_some()).count();
        names.push(format!("scene MAN PROT {idx} ({} actor edits, {clones} native actor copies; original scripts reused)",edits.npcs.len()));
        for actor in &edits.npcs {
            log::info!("Native field actor #{} ({}) {}",actor.record,actor.kind,
                if actor.uid.is_some(){"script-bearing copy"}else{"original edit"});
        }
        if rim_elm {
            skipped.push("NPC MAN edits apply only to selected Rim Elm story state; other story-state actor scripts were not changed".into());
        }
    }
    let written=disc.into_image();
    ensure!(written.len()==image_length,"Native patch unexpectedly changed total image size");
    use std::io::Write;
    let mut file=std::fs::OpenOptions::new().create_new(true).write(true).open(&staging)
        .with_context(||format!("Create staging file {}",staging.display()))?;
    if let Err(error)=file.write_all(&written).and_then(|_|file.sync_all()) {
        drop(file);let _=std::fs::remove_file(&staging);
        return Err(error).context("Write/sync staging disc image");
    }
    drop(file);
    if output_abs.exists() {
        let _=std::fs::remove_file(&staging);
        bail!("Destination appeared during export; not replacing it");
    }
    std::fs::rename(&staging,&output_abs).context("Finalize new test disc image")?;
    log::info!("LCK native PS1 Rim Elm export: {}",names.join(", "));
    for entry in &skipped {log::warn!("{entry}");}
    Ok(ExportResult{path:output_abs,patched:names,skipped})
}

#[cfg(test)]
mod tests {
    #[test]
    fn foreign_editor_previews_do_not_disable_native_collision_wireframes() {
        use super::scene_instances::{ForeignInstance, NameTarget, SceneDocument};
        let mut doc = SceneDocument::default();
        doc.foreign.push(ForeignInstance { uid: 123, source: "balden".into(),
            kind: "TMD".into(), record: 82, center: [400.0,0.0,700.0] });
        doc.rotations.push((NameTarget::Created(123), [0.0,3.14,0.0]));
        assert!(!has_unhandled_native_transforms(&doc, true));
        assert!(has_unhandled_native_transforms(&doc, false));
        doc.rotations.push((NameTarget::Native(false, 5), [0.0,0.5,0.0]));
        assert!(has_unhandled_native_transforms(&doc, true));
    }

    use super::*;
    #[test]
    fn actual_rim_elm_man_12_cold_spawn_has_script_relocation() {
        // Byte-accurate prefix of town01's MAN P1[12], beginning at script_pc0.
        // Its raw placement is (3776,1856), but the false-flag branch moves
        // it to grid (0x1e,0x0f) = (3904,1984) before drawing.
        let bytecode=[0x25,0x31,0x02,0x71,0x46,0x1f,0x00,0x72,0x2e,0x10,0x00,
            0x26,0x02,0x00,0x32,0x16,0x4c,0x51,0x1e,0x0f,0x00,0x0d,
            0x26,0x18,0x00,0x32,0x16,0x4c,0x51,0x23,0x0f,0x00,0x0d,0x21];
        let site=cold_spawn_site(&bytecode,0).expect("cold spawn script opcode");
        assert_eq!((site.x_operand,site.z_operand),(18,19));
        assert_eq!(site.encoded,(0x1e,0x0f));
        let mut moved=bytecode;
        moved[site.x_operand]=0x1f;
        assert_eq!(cold_spawn_site(&moved,0).unwrap().encoded,(0x1f,0x0f));
        assert_eq!(&moved[29..31],&[0x23,0x0f]); // alternate flag arm untouched
    }
    #[test]
    fn unknown_spawn_prologue_must_not_be_guessed() {
        assert_eq!(cold_spawn_site(&[0x25,0x21,0x4c,0x51,0x20,0x20],0),None);
        assert_eq!(cold_spawn_site(&[0x25,0x42,0,0,0,0],0),None);
    }
    #[test]
    fn native_man_actor_positions_quantize_to_64_units() {
        assert_eq!(native_npc_coord(64).unwrap(),(0,64));
        assert_eq!(native_npc_coord(128).unwrap(),(0x80,128));
        assert_eq!(native_npc_coord(176).unwrap(),(1,192));
        assert!(native_npc_coord(-33).is_err());
    }
    fn test_map(flag_corrected:bool)->(Vec<u8>,MoveEdit,InsertEdit) {
        let mut m=vec![0u8;MAP_LEN];
        let idx=138usize;
        let (c,r)=(34u8,34u8);
        let o=idx*32;
        m[o..o+2].copy_from_slice(&(-32i16).to_le_bytes());
        m[o+4..o+6].copy_from_slice(&32i16.to_le_bytes());
        m[o+6]=255;m[o+7]=255;
        m[o+0x10..o+0x12].copy_from_slice(&50u16.to_le_bytes());
        let flags:u16=0x1816 | (if flag_corrected {8} else {0});
        m[o+0x12..o+0x14].copy_from_slice(&flags.to_le_bytes());
        let old_grid=GRID_OFF+(r as usize*128+c as usize)*2;
        m[old_grid..old_grid+2].copy_from_slice(&(idx as u16).to_le_bytes());
        // Source has a bind, but a new copy must use an unbound tile.
        let bind=GRID_OFF+(33usize*128+33)*2;
        m[bind..bind+2].copy_from_slice(&0x0400u16.to_le_bytes());
        let floor=0x4000+r as usize*128+c as usize;
        m[floor]=4;
        m[0x4000+33usize*128+34]=4; // available same-tier tile
        let pos=(field_objects::world_x(c,-32),field_objects::world_z(r,32));
        let mov=MoveEdit{collision_bounds:None,cell:(c,r),source_position:pos,env_slot:50,
            source_rotation:[0;3],delta:[-16.0,0.0,32.0]};
        let copy=InsertEdit{collision_bounds:None,uid:1,cell:(c,r),source_position:pos,env_slot:50,
            source_rotation:[0;3],delta:[128.0,0.0,0.0]};
        (m,mov,copy)
    }
    #[test]
    fn moving_corrected_original_moves_static_collider_same_distance() {
        let (mut m,mov,_)=test_map(true);
        let before=ObjectRecord::parse(&m,138).unwrap();
        let collider=source_collision(&before,34,34);
        assert_eq!(move_originals(&mut m,&[mov]).unwrap(),vec![138]);
        let after=ObjectRecord::parse(&m,138).unwrap();
        assert_eq!(source_collision(&after,34,34),(collider.0-16,collider.1+32));
        assert_eq!(after.sub_anchor_x,-1);
        assert_eq!(after.sub_anchor_z,2);
    }
    #[test]
    fn copies_use_empty_slot_and_empty_grid_keep_collision_and_floor() {
        let (original,mov,copy)=test_map(true);
        let edits=ExportEdits{moves:vec![mov],copies:vec![copy],npcs:vec![],tier_heights:[None;16],foreign_wall_stencils:Vec::new()};
        let plan=map_patches(&original,&edits).unwrap();
        let mut changed=original.clone();
        for (o,v) in plan {changed[o..o+v.len()].copy_from_slice(&v);}
        assert_eq!(&original[0x4000..0x8000],&changed[0x4000..0x8000]);

        let placed=field_objects::parse_placements(&changed);
        let added=placed.iter().find(|p|p.obj_idx==511).unwrap();
        assert_eq!((added.world_x,added.world_z),(4384+128,4384));
        let old=ObjectRecord::parse(&original,138).unwrap();
        let before=source_collision(&old,34,34);
        assert_eq!((added.collider_x,added.collider_z),(before.0+128,before.1));
        assert_eq!(added.anchor_cell&field_objects::CELL_BIND_OWNED,0);
    }
    #[test]
    fn duplicate_can_overlap_original_at_unchanged_world_coordinates() {
        let (original,_,mut copy)=test_map(false);
        copy.delta=[0.0;3];
        let mut modified=original.clone();
        let out=insert_copies(&mut modified,&original,&[copy],&[None;16]).unwrap();
        assert_eq!(out.len(),1);
        let placed=field_objects::parse_placements(&modified);
        let original_pos=placed.iter().find(|p|p.obj_idx==138).unwrap();
        let clone=placed.iter().find(|p|p.obj_idx as usize==out[0].id).unwrap();
        assert_eq!((original_pos.world_x,original_pos.world_z),(clone.world_x,clone.world_z));
        assert_ne!((original_pos.col,original_pos.row),(clone.col,clone.row));
    }
    #[test]
    fn clone_can_use_another_floor_tier_when_ladder_heights_known() {
        let (mut original,_,mut copy)=test_map(false);
        copy.delta=[128.0,31.0,0.0];
        // Force candidate tiles onto a different floor and give exact ladder heights.
        for row in 0..128 {
            for col in 0..128 {
                let o=GRID_OFF+(row*128+col)*2;
                if original[o]==0 && original[o+1]==0 {
                    original[0x4000+row*128+col]=3;
                }
            }
        }
        let mut modified=original.clone();
        let mut heights=[None;16]; heights[4]=Some(400); heights[3]=Some(300);
        let added=insert_copies(&mut modified,&original,&[copy],&heights).unwrap();
        let record=ObjectRecord::parse(&modified,added[0].id).unwrap();
        assert_eq!(record.y_off,-31+100);
    }
    #[test]
    fn anchor_scoring_prefers_proven_window_over_extended_distance() {
        assert_eq!(anchor_score(300,300),Some(600));
        assert_eq!(anchor_score(640,0),Some(LCK_EXTENDED_ANCHOR_PENALTY+640));
        assert_eq!(anchor_score(1025,0),None);
        assert!(anchor_score(320,320).unwrap()<anchor_score(385,0).unwrap());
    }
    #[test]
    fn clone_rejects_distant_anchor_that_causes_invisible_mesh() {
        let (mut original,_,copy)=test_map(false);
        // Occupy every otherwise empty grid tile, except one 6 cells away.
        for row in 0..128usize { for col in 0..128usize {
            let o=GRID_OFF+(row*128+col)*2;
            if original[o]==0 && original[o+1]==0 {original[o+1]=0x10;}
        }}
        let free_col=40usize;
        let free_row=34usize;
        let o=GRID_OFF+(free_row*128+free_col)*2;
        original[o..o+2].copy_from_slice(&[0,0]);
        original[WALL_OFF+free_row*128+free_col]=4;
        let mut modified=original.clone();
        let message=insert_copies(&mut modified,&original,&[copy],&[None;16])
            .expect_err("unsafe extended spawn anchor must not export").to_string();
        assert!(message.contains("collision with invisible geometry"),"{message}");
        assert_eq!(modified,original,"failed allocation must leave the MAP unchanged");
    }
    #[test]
    fn cannot_take_over_terrain_or_displace_existing_placement() {
        let (mut m,_,copy)=test_map(false);
        // Remove every safe candidate tile. Export must fail, not write on terrain.
        for row in 0..128 {
            for col in 0..128 {
                let p=GRID_OFF+(row*128+col)*2;
                if m[p]==0 && m[p+1]==0 {m[p+1]=0x10;}
            }
        }
        let before=m.clone();
        assert!(insert_copies(&mut m,&before,&[copy],&[None;16]).is_err());
    }
    #[test]
    fn collision_corrected_moves_accept_unaligned_positions_with_bounded_rounding() {
        let (mut m,mut mov,_)=test_map(true);
        mov.delta[0]=17.0;
        let before=ObjectRecord::parse(&m,138).unwrap();
        let from=source_collision(&before,34,34);
        move_originals(&mut m,&[mov]).unwrap();
        let after=ObjectRecord::parse(&m,138).unwrap();
        let to=source_collision(&after,34,34);
        assert!((to.0-from.0-17).abs()<=8);
    }
    #[test]
    fn moved_house_wall_stencil_follows_mesh_and_preserves_floor_nibbles() {
        let (mut m,mut mov,_)=test_map(false);
        // Physical source wall footprint centred on native (4384,4384).
        let sx=4384/64;
        let sz=4384/64+2; // z wall-query bias
        let (o,b)=wall_offset(sx,sz).unwrap();
        m[o]=b|6;
        let old=m.clone();
        mov.collision_bounds=Some([4384-30,4384+30,4384-30,4384+30]);
        mov.delta=[128.0,0.0,64.0];
        let edits=ExportEdits{moves:vec![mov],copies:vec![],npcs:vec![],tier_heights:[None;16],foreign_wall_stencils:Vec::new()};
        let report=compile_walls(&mut m,&old,&edits).unwrap();
        assert_eq!(report.source_bits,1);
        assert_eq!(m[o],6); // low nibble (floor) unchanged; old wall cleared
        let (dest,bit)=wall_offset(sx+2,sz+1).unwrap();
        assert_eq!(m[dest]&bit,bit);
        assert_eq!(m[dest]&0xf,old[dest]&0xf);
    }
    #[test]
    fn duplicated_house_receives_same_mask_at_independent_position() {
        let (mut m,_,mut copy)=test_map(false);
        let sx=4384/64;
        let sz=4384/64+2;
        let (o,b)=wall_offset(sx,sz).unwrap();
        m[o]=b|9;
        let old=m.clone();
        copy.collision_bounds=Some([4354,4414,4354,4414]);
        copy.delta=[192.0,0.0,0.0];
        let edits=ExportEdits{moves:vec![],copies:vec![copy],npcs:vec![],tier_heights:[None;16],foreign_wall_stencils:Vec::new()};
        let report=compile_walls(&mut m,&old,&edits).unwrap();
        assert_eq!(report.source_bits,1);
        assert_eq!(m[o],old[o]); // original collision intact
        let (dest,bit)=wall_offset(sx+3,sz).unwrap();
        assert!(m[dest]&bit !=0);
    }
    #[test]
    fn moved_source_and_overlapping_clone_collision_are_composited_not_erased() {
        let (mut m,mut mov,mut copy)=test_map(false);
        let sx=4384/64;
        let sz=4384/64+2;
        let (o,b)=wall_offset(sx,sz).unwrap();
        m[o]=b|3;
        let old=m.clone();
        mov.collision_bounds=Some([4354,4414,4354,4414]);
        copy.collision_bounds=mov.collision_bounds;
        mov.delta=[64.0,0.0,0.0];
        copy.delta=[0.0;3]; // clone remains on original footprint
        let edits=ExportEdits{moves:vec![mov],copies:vec![copy],npcs:vec![],tier_heights:[None;16],foreign_wall_stencils:Vec::new()};
        compile_walls(&mut m,&old,&edits).unwrap();
        assert!(m[o]&b != 0); // clone keeps source wall when original moved
        let (dest,bit)=wall_offset(sx+1,sz).unwrap();
        assert!(m[dest]&bit !=0);
    }
    #[test]
    fn grid_export_patches_wall_bytes_and_never_floor_nibbles() {
        let (mut original,mut mov,_)=test_map(false);
        let sx=4384/64;
        let sz=4384/64+2;
        let (o,b)=wall_offset(sx,sz).unwrap();
        original[o]=b|4;
        mov.collision_bounds=Some([4354,4414,4354,4414]);
        mov.delta=[64.0,0.0,0.0];
        let edits=ExportEdits{moves:vec![mov],copies:vec![],npcs:vec![],tier_heights:[None;16],foreign_wall_stencils:Vec::new()};
        let patches=map_patches(&original,&edits).unwrap();
        assert!(patches.iter().any(|(off,_)| (*off>=WALL_OFF)&&(*off<GRID_OFF)));
        let mut compiled=original.clone();
        for (off,bytes) in patches {compiled[off..off+bytes.len()].copy_from_slice(&bytes);}
        for i in WALL_OFF..GRID_OFF {assert_eq!(compiled[i]&15,original[i]&15);}
        let (dest,bit)=wall_offset(sx+1,sz).unwrap();
        assert!(compiled[dest]&bit !=0);
    }
    #[test]
    fn unrelated_walls_outside_original_bounds_stay_untouched() {
        let (mut m,mut mov,_)=test_map(false);
        let (owned,bit)=wall_offset(4384/64,4384/64+2).unwrap();
        let (unrelated,otherbit)=wall_offset(4384/64+6,4384/64+2).unwrap();
        m[owned]=bit|7;
        m[unrelated]=otherbit|11;
        let before=m.clone();
        mov.collision_bounds=Some([4354,4414,4354,4414]);
        mov.delta=[64.0,0.0,0.0];
        let edits=ExportEdits{moves:vec![mov],copies:vec![],npcs:vec![],tier_heights:[None;16],foreign_wall_stencils:Vec::new()};
        compile_walls(&mut m,&before,&edits).unwrap();
        assert_eq!(m[unrelated],before[unrelated]);
    }
    #[test]
    fn rim_elm_story_state_indices_are_preserved() {
        assert_eq!(RIM_ELM_MAPS.iter().map(|(_,i)|*i).collect::<Vec<_>>(),vec![1,10,19,345,739]);
    }
}
