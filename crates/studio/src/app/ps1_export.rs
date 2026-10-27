//! Native MAP/MAN export planning and serialization.
//! Record-local colliders are distinct from the MAP wall/floor grid. Wall
//! ownership is not encoded there: geometry bounds only identify potential
//! conflicts, never authorize clearing or copying wall bits. Ambiguous edits
//! fail closed; floor tiers and unrelated walls remain byte-for-byte intact.
//! Native arbitrary scale, general TMD/TIM relocation and script-bound static
//! object duplication remain unsupported.
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

#[path = "native_dependencies.rs"]
pub(super) mod native_dependencies;
#[path = "experimental_native_export.rs"]
pub(super) mod experimental;
use native_dependencies::NativeBindings;
#[path = "native_deletion.rs"]
mod native_deletion;
#[path = "insertion_provenance.rs"]
mod insertion_provenance;

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
    /// Diagnostic search area only, never an ownership claim. Missing bounds
    /// require a wall-free map before the record-local collider can move.
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
    /// Non-placed decoration references, not the walkable heightfield.
    pub terrain_moves: Vec<MoveEdit>,
    pub deletes: Vec<MoveEdit>,
    insertion_provenance:Option<insertion_provenance::VerifiedInsertions>,
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
    original_draw_bounds(world,(false,index))
}

fn original_draw_bounds(world: &WorldModel, key: (bool,usize)) -> Option<[i32;4]> {
    let draw=world.at(key.0,key.1)?;
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
    prepare_with_mode(scene, world, PreparationMode::NativeExport)
}

/// Build a read-only collision overlay from native MAP edits even if the user
/// also staged a foreign TMD mesh. Foreign previews have NO imported native
/// collision and are deliberately ignored; the strict IMG exporter above still
/// rejects the same scene until foreign resource relocation is implemented.
pub(super) fn prepare_collision_preview(scene: &AssembledScene, world: &WorldModel) -> Result<ExportEdits> {
    prepare_with_mode(scene, world, PreparationMode::CollisionPreview)
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
    // Keep all destination-native edits and checks, including MAN actors.
    // Foreign dependencies are validated below before export is permitted.
    ensure!(!doc.created.iter().any(|created|world.npc(created.source).is_some()),
        "Native MAN NPC cloning alongside foreign resource imports needs a separate export preflight");
    let mut edits = prepare_with_mode(scene,world,PreparationMode::CrossCellExport)?;
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
        require_no_unowned_walls(&donor_map,Some(bounds))
            .with_context(||format!("Foreign UID {} donor collision",foreign.uid))?;
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
    ensure!(!edits.terrain_moves.is_empty() || !edits.moves.is_empty() || !edits.deletes.is_empty() || !edits.copies.is_empty() || !edits.npcs.is_empty(),
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
pub(super) fn vram_texel(vram:&legaia_tim::Vram,cba:u16,tsb:u16,u:usize,v:usize)->u16 {
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

/// Export modes always preserve NPC edits. Only wall visualization may omit
/// actors and relax foreign transforms; cross-cell export resolves donors later.
#[derive(Clone, Copy, PartialEq, Eq)]
enum PreparationMode { NativeExport, CrossCellExport, CollisionPreview }

fn prepare_with_mode(scene: &AssembledScene, world: &WorldModel, mode: PreparationMode) -> Result<ExportEdits> {
    let collision_preview = mode == PreparationMode::CollisionPreview;
    let overlay = scene_instances::serialize(&scene.name, world);
    let mut doc = scene_instances::parse(&overlay, &scene.name, world)
        .map_err(anyhow::Error::msg)?;
    // Final deletion supersedes earlier preview transforms of that original.
    doc.moves.retain(|(key,_)|!doc.hidden.contains(key));
    doc.rotations.retain(|(key,_)|!matches!(key,scene_instances::NameTarget::Native(t,i) if doc.hidden.contains(&(*t,*i))));
    doc.scales.retain(|(key,_)|!matches!(key,scene_instances::NameTarget::Native(t,i) if doc.hidden.contains(&(*t,*i))));
    // Local export cannot resolve foreign dependencies. Cross-cell export
    // must validate them separately before the edits can reach the writer.
    if mode == PreparationMode::NativeExport {
        ensure!(doc.foreign.is_empty(),
            "This LCK scene contains cross-cell asset previews. Native IMG export is disabled until donor TMD/TIM resource relocation and collision serialization are implemented. Save the .lckscene instead.");
    }
    // Foreign asset rotations/scales are irrelevant to the native MAP wall
    // overlay; unsupported rotations/scales of a NATIVE object still block it.
    // Both export modes reject every unsupported rotation or scale.
    ensure!(!has_unhandled_native_transforms(&doc, collision_preview),
        "Native rotation/scale requires a native transform/TMD export stage; reset rotation/scale or save .lckscene to preserve the edit. Stage M2 changes placement freedoms only.");

    let mut result=ExportEdits::default();
    for &(terrain,index) in &doc.hidden {
        ensure!(!terrain,"Native deletion of terrain/ground is unsupported; no output written");
        ensure!(world.npc((terrain,index)).is_none(),"Native NPC/actor deletion requires MAN script/dependency removal; no output written");
        let native=scene.placements.get(index).with_context(||format!("No native placement for deletion #{index}"))?;
        result.deletes.push(MoveEdit{collision_bounds:original_collision_bounds(world,index),cell:native.cell,
            source_position:(native.world_x,native.world_z),env_slot:native.env_slot,
            source_rotation:[native.rot_x,native.rot_y,native.rot_z],delta:[0.0;3]});
    }
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
            || npc.model_index!=npc.original_model_index || npc.clip!=npc.anim_id || npc.frame!=0;
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
        if !terrain && world.npc((false,index)).is_some() {continue;}

        let native=if terrain {scene.terrain.get(index)} else {scene.placements.get(index)}
            .with_context(||format!("Missing original native draw ({terrain}, {index})"))?;
        let draw=world.at(terrain,index).context("Original source is absent from hierarchy")?;
        ensure!(draw.editor_uid.is_none() && !draw.hidden,"Moved source is a created or hidden object");
        ensure!(delta.iter().all(|v|v.is_finite()) && delta.iter().any(|v|v.abs()>EPS),"Bad native movement delta");
        let edits=if terrain {&mut result.terrain_moves} else {&mut result.moves};
        edits.push(MoveEdit { collision_bounds:original_draw_bounds(world,(terrain,index)), cell:native.cell,
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
        ensure!(mode == PreparationMode::CrossCellExport || !result.terrain_moves.is_empty() || !result.moves.is_empty() || !result.deletes.is_empty() || !result.copies.is_empty() || !result.npcs.is_empty(),
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
    record_at_kind(data,cell,pos,mesh,angles,false)
}

fn record_at_kind(data:&[u8], cell:(u8,u8), pos:(i32,i32),
             mesh:usize, angles:[u16;3], terrain:bool) -> Result<(usize,ObjectRecord)> {
    ensure!(data.len()==MAP_LEN,"Native field MAP expected 0x12000 bytes");
    let (col,row)=cell;
    ensure!(col<128 && row<128,"Source reference outside native grid");
    let grid_off=GRID_OFF+(row as usize*GRID_DIM+col as usize)*2;
    let word=u16::from_le_bytes(data[grid_off..grid_off+2].try_into()?);
    let id=(word&field_objects::OBJECT_INDEX_MASK) as usize;
    let r=ObjectRecord::parse(data,id).context("MAP object record absent")?;
    if terrain {
        ensure!(id!=0 && !r.is_placed() && word&field_objects::CELL_VISIBLE!=0,
            "Source cell ({col},{row}) is not a native terrain decoration");
    } else {
        ensure!(r.is_placed(),"Source cell ({col},{row}) is not a native placed object");
    }
    let selector=if terrain {Some(r.pack_index_field)} else {field_objects::pack_mesh_index(id as u16,&r)};
    ensure!(selector==Some(mesh as u16),
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

#[derive(Debug, PartialEq, Eq)]
struct MovedRecord {
    id: usize,
    /// A shared descriptor is detached by changing only this reference.
    detached_cell: Option<(u8,u8)>,
}

/// Allocate only zeroed descriptors that no native grid reference uses.
/// The search order is fixed so preflight and serialization stay deterministic.
fn blank_object_record(map:&[u8]) -> Result<usize> {
    let mut used=[false;512];
    for word in map[GRID_OFF..GRID_OFF+GRID_DIM*GRID_DIM*2].chunks_exact(2) {
        used[(u16::from_le_bytes([word[0],word[1]])&field_objects::OBJECT_INDEX_MASK) as usize]=true;
    }
    (4..512).rev().find(|&id|!used[id] && map[id*32..id*32+32].iter().all(|&b|b==0))
        .context("No completely blank and unreferenced 32-byte native object slot remains")
}

fn move_originals(map:&mut [u8], edits:&[MoveEdit]) -> Result<Vec<MovedRecord>> {
    move_records(map,edits,false)
}

fn move_records(map:&mut [u8], edits:&[MoveEdit], terrain:bool) -> Result<Vec<MovedRecord>> {
    let mut touched=HashSet::new();
    let mut refs=[0u16;512];
    for i in 0..GRID_DIM*GRID_DIM {
        let o=GRID_OFF+i*2;
        let idx=(u16::from_le_bytes([map[o],map[o+1]])&0x1FF) as usize;
        refs[idx]+=1;
    }
    let mut changed=Vec::new();
    for edit in edits {
        let (id,r)=record_at_kind(map,edit.cell,edit.source_position,edit.env_slot,edit.source_rotation,terrain)?;
        ensure!(!anchor_has_bind(map,edit.cell,&r),"Record #{id} is script-bound; native movement requires script ownership resolution");
        ensure!(touched.insert(edit.cell),"Multiple moves target the same native reference {:?}",edit.cell);
        // Native mesh selection is record +0x10, not the record index. Keep
        // every other reference on the original descriptor; detach only this
        // selected unbound spawn. Bound objects still require MAN resolution.
        let detached=refs[id]>1;
        let target=if detached {blank_object_record(map)?} else {id};
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
        if terrain {
            // The decoration sweep selects by grid cell before applying the
            // descriptor offset. Keep the mesh inside its authored cull reach;
            // do not move the grid word (also consumed by ground/walkability).
            let reach=i32::from(r.cull_radius)*128;
            ensure!(reach>0 && i32::from(x).abs()<=reach && i32::from(z).abs()<=reach,
                "Terrain descriptor #{id} translation exceeds authored cull radius {}; native draw-window relocation is required",r.cull_radius);
        } else {
            ensure!(anchor_score(x as i32,z as i32).is_some_and(|score|score<LCK_EXTENDED_ANCHOR_PENALTY),
                "Moved record #{id} exceeds the supported native spawn-anchor reach");
        }
        let moved=ObjectRecord::parse(&next,0).context("Moved record invalid")?;
        // Non-placed decorations have no spawned static-actor collider.
        // Their wall/floor dependencies are validated separately.
        if !terrain {adjust_collider(&moved,&r,&mut next)?;}
        let moved=ObjectRecord::parse(&next,0).context("Moved collision record invalid")?;
        let old_c=source_collision(&r,edit.cell.0,edit.cell.1);
        let new_c=source_collision(&moved,edit.cell.0,edit.cell.1);
        ensure!(terrain || ((new_c.0-old_c.0-dx).abs()<=8 && (new_c.1-old_c.1-dz).abs()<=8),
            "Native static collider moved outside the engine's 16-unit precision");
        map[target*32..target*32+32].copy_from_slice(&next);
        if detached {
            let grid=GRID_OFF+(edit.cell.1 as usize*GRID_DIM+edit.cell.0 as usize)*2;
            let word=u16::from_le_bytes(map[grid..grid+2].try_into()?);
            let next_word=(word & !field_objects::OBJECT_INDEX_MASK)|target as u16;
            map[grid..grid+2].copy_from_slice(&next_word.to_le_bytes());
        }
        changed.push(MovedRecord{id:target,detached_cell:detached.then_some(edit.cell)});
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

#[cfg(test)]
fn insert_copies(map:&mut [u8], source_map:&[u8], edits:&[InsertEdit],
                 tiers:&[Option<i32>;16]) -> Result<Vec<Inserted>> {
    let bindings=NativeBindings::parse(&source_map[0x10000..],&[])?;
    insert_copies_with_bindings(map,source_map,edits,tiers,&bindings)
}

fn insert_copies_with_bindings(map:&mut [u8], source_map:&[u8], edits:&[InsertEdit],
                 tiers:&[Option<i32>;16],bindings:&NativeBindings) -> Result<Vec<Inserted>> {
    insert_copies_with_source_tiers(map,source_map,edits,tiers,None,bindings)
}
fn insert_copies_with_source_tiers(map:&mut [u8], source_map:&[u8], edits:&[InsertEdit],
                 tiers:&[Option<i32>;16],source_tiers:Option<&[Option<i32>;16]>,bindings:&NativeBindings) -> Result<Vec<Inserted>> {
    let mut done=Vec::new();
    for edit in edits {
        let (source_idx,src)=record_at(source_map,edit.cell,edit.source_position,
            edit.env_slot,edit.source_rotation)
            .with_context(||format!("Validate original for copy #{}",edit.uid))?;
        ensure!(!anchor_has_bind(source_map,edit.cell,&src),"Copy UID {} has a script-bound source; cannot discard its native spawn binding",edit.uid);
        ensure!(source_idx>3,"Player/NPC source cannot be copied as static environment geometry");
        let slot=blank_object_record(map)?;
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
                if bindings.has_trigger((col as u8,row as u8)) {continue;}
                blank+=1;
                let anchor_col=col as i32+src.col_delta as i32;
                let anchor_row=row as i32+src.row_delta as i32;
                if !(0..128).contains(&anchor_col) || !(0..128).contains(&anchor_row) {continue;}
                if bindings.at((anchor_col as u8,anchor_row as u8)).is_some() {continue;}
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
                let source_heights=source_tiers.unwrap_or(tiers);
                let floor_correction=if source_tiers.is_none()&&source_tier==candidate_tier {0}
                    else if let (Some(src_h),Some(dst_h))=(source_heights[source_tier],tiers[candidate_tier]) {
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

/// MAP wall bits have no object owner IDs. Geometry bounds are only a
/// diagnostic search area, never evidence that a bit belongs to an instance.
/// Until ownership is established, reject affected edits before changing any
/// byte. Proven record-local colliders remain independently exportable.
fn require_no_unowned_walls(original:&[u8], bounds:Option<[i32;4]>) -> Result<()> {
    let count=match bounds {
        Some(_) => source_stencil(original,bounds).len(),
        None => original[WALL_OFF..GRID_OFF].iter().map(|b|(b & 0xf0).count_ones() as usize).sum(),
    };
    ensure!(count==0,
        "Unresolved collision ownership: {count} native wall bits may belong to terrain or overlapping objects. Move/copy blocked; original walls preserved");
    Ok(())
}

pub(super) fn collision_ownership_diagnostic(world:&WorldModel,key:(bool,usize),map:&[u8])->String {
    let source=world.at(key.0,key.1).and_then(|d|d.source_ref).unwrap_or(key);
    match require_no_unowned_walls(map,original_draw_bounds(world,source)) {
        Ok(())=>"No spatial wall conflicts observed; this does not establish ownership or complete collision".into(),
        Err(error)=>error.to_string(),
    }
}

fn compile_walls(map:&mut [u8], original:&[u8], edits:&ExportEdits)->Result<WallReport> {
    ensure!(map.len()==MAP_LEN && original.len()==MAP_LEN,"Invalid native field MAP length");
    for (terrain,edit) in edits.moves.iter().map(|e|(false,e)).chain(edits.terrain_moves.iter().map(|e|(true,e))) {
        if terrain || !edits.insertion_provenance.as_ref().is_some_and(|p|p.covers(original,edit)) {
            require_no_unowned_walls(original,edit.collision_bounds)?;
        }
    }
    for edit in &edits.copies {
        if let Some((_,bits))=edits.foreign_wall_stencils.iter().find(|(uid,_)|*uid==edit.uid) {
            ensure!(bits.is_empty(),"Unresolved donor wall ownership for UID {}; import blocked",edit.uid);
        } else {
            require_no_unowned_walls(original,edit.collision_bounds)?;
        }
    }
    // Neither deletions nor guessed additions are permitted. Preserve both
    // wall bits and walkable floor tiers exactly, including shared walls.
    map[WALL_OFF..GRID_OFF].copy_from_slice(&original[WALL_OFF..GRID_OFF]);
    Ok(WallReport::default())
}

/// Return native MAP writes only after checking every other byte remains
/// untouched. Object slots and grid links are validated independently.
#[cfg(test)]
fn map_patches(original: &[u8], edits: &ExportEdits) -> Result<Vec<(usize,Vec<u8>)>> {
    ensure!(original.len()==MAP_LEN,"MAP must be exactly 0x12000 bytes");
    let bindings=NativeBindings::parse(&original[0x10000..],&[])?;
    map_patches_with_bindings(original,edits,&bindings)
}

fn map_patches_with_bindings(original:&[u8],edits:&ExportEdits,bindings:&NativeBindings) -> Result<Vec<(usize,Vec<u8>)>> {
    ensure!(original.len()==MAP_LEN,"MAP must be exactly 0x12000 bytes");
    let mut unresolved=Vec::new();
    for edit in &edits.terrain_moves {
        if bindings.has_trigger(edit.cell) {
            let triggers=bindings.triggers_at(edit.cell).map(|t|format!("{} kind-{} byte {:#x}, payload {:02x?}",t.block,t.kind,t.offset,t.payload)).collect::<Vec<_>>().join("; ");
            unresolved.push(format!("Terrain reference {:?} has native trigger/height dependencies ({triggers}); ownership resolution is required",edit.cell));
        }
    }
    for cell in edits.moves.iter().chain(&edits.terrain_moves).chain(&edits.deletes).map(|e|e.cell).chain(edits.copies.iter().map(|e|e.cell)) {
        let go=GRID_OFF+(cell.1 as usize*GRID_DIM+cell.0 as usize)*2;
        ensure!(cell.0<128 && cell.1<128,"Source reference outside native grid");
        let id=(u16::from_le_bytes(original[go..go+2].try_into()?)&511) as usize;
        let rec=ObjectRecord::parse(original,id).context("Missing source descriptor")?;
        let x=cell.0 as i32+rec.col_delta as i32;let z=cell.1 as i32+rec.row_delta as i32;
        ensure!((0..128).contains(&x)&&(0..128).contains(&z),"Invalid source bind anchor");
        if let Some(binding)=bindings.at((x as u8,z as u8)) {
            unresolved.push(format!("Native binding unresolved for record #{id}: {} trigger byte {:#x}, MAN flat record {}, gate {}. Independent script/animation/collision dependencies must be resolved before export",binding.block,binding.offset,binding.record,binding.gate));
        }
    }
    // Report every moving object's unresolved dependency in one attempt. A
    // category route is not a waiver of wall ownership or native behavior.
    for (kind,edit) in edits.moves.iter().map(|e|("placement",e)).chain(edits.terrain_moves.iter().map(|e|("terrain",e))) {
        if kind=="placement" && edits.insertion_provenance.as_ref().is_some_and(|p|p.covers(original,edit)) {continue;}
        if let Err(error)=require_no_unowned_walls(original,edit.collision_bounds) {
            unresolved.push(format!("{kind} reference {:?}: {error:#}",edit.cell));
        }
    }
    ensure!(unresolved.is_empty(),"Native dependency validation failed; no output written:\n{}",unresolved.join("\n"));
    let mut updated=original.to_vec();
    let mut deleted=HashSet::new();
    for edit in &edits.deletes {
        ensure!(deleted.insert(edit.cell)&&!edits.moves.iter().any(|e|e.cell==edit.cell),"Conflicting repeated move/delete request");
        let dependency=native_deletion::analyze(original,edit,bindings,edits.insertion_provenance.as_ref())?;
        dependency.validate()?;
        // Only the independently validated spawn reference is removed. Its
        // actor (and record-local collider) is no longer created by that sweep.
        // Native descriptor/resources, MAP walls, floors and scripts remain.
        let off=GRID_OFF+(edit.cell.1 as usize*GRID_DIM+edit.cell.0 as usize)*2;
        updated[off..off+2].copy_from_slice(&dependency.replacement_word.to_le_bytes());
    }
    let mut moved=move_originals(&mut updated,&edits.moves)?;
    moved.extend(move_records(&mut updated,&edits.terrain_moves,true)?);
    let inserted=insert_copies_with_bindings(&mut updated,original,&edits.copies,&edits.tier_heights,bindings)?;
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
    for moved in moved {
        let off=moved.id*32;
        if let Some((col,row))=moved.detached_cell {
            for p in off..off+32 {allowed[p]=true;}
            let grid=GRID_OFF+(row as usize*GRID_DIM+col as usize)*2;
            allowed[grid]=true;allowed[grid+1]=true;
        } else {
            for p in off..off+6 {allowed[p]=true;}
            for p in off+14..off+16 {allowed[p]=true;}
        }
    }
    for &(col,row) in &deleted {
        let off=GRID_OFF+(row as usize*GRID_DIM+col as usize)*2;
        allowed[off]=true;allowed[off+1]=true;
        ensure!(!field_objects::parse_placements(&updated).iter().any(|p|(p.col,p.row)==(col,row)),"Deleted placement survived native readback");
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

/// Complete deterministic native serialization, including sector fixups.
/// Fields are private: only the validated planner can create a writable plan.
/// Keeping the compiled bytes makes preflight and execution use exactly the
/// same allocations and compressed payloads, without a second interpretation.
pub(super) struct NativeExportPlan {
    dependencies:super::native_asset_plan::NativeAssetPlan,
    source:PathBuf,
    image:Vec<u8>,
    patched:Vec<String>,
    skipped:Vec<String>,
    continuation_receipt:Option<serde_json::Value>,
}

/// Serialized resource+placement candidate, deliberately NOT an exportable
/// NativeExportPlan. Collision/surface and runtime acceptance remain mandatory.
pub(in crate::app) struct NativeImportCandidate {
    disc:DiscPatcher,
    pub descriptor:usize,pub cell:(u8,u8),pub uid:u64,pub selector:u16,
    pub pending_collision_recipes:usize,
    pub placements:Vec<(u64,usize,(u8,u8),u16)>,
    pub native_walls:Option<super::collision_recipes::native_import::ImportWallPlan>,
    pub collision_diagnostics:Vec<String>,
}
impl NativeImportCandidate {
    pub fn unresolved(&self)->&'static str {
        "Native bytes verified; object collision/surface/actor contract, complete RAM/VRAM lifetime and runtime spawn-window acceptance remain unresolved. This candidate cannot be written by export()."
    }
    pub fn image_bytes(&self)->&[u8]{self.disc.image()}
    pub fn native_map(&self,entry:usize)->Result<Vec<u8>>{self.disc.read_entry_footprint(entry)}
    fn compile_collision(&mut self,scene:&AssembledScene,world:&WorldModel,index:&ProtIndex)->Result<()> {
        use super::collision_recipes::{ObjectId,Evidence,native_import::{self,ImportContribution}};
        let native=Scene::load(index,&scene.name)?;let map_entry=native.field_map_index(index).context("Destination collision MAP unavailable")?;
        let map=self.disc.read_entry_footprint(map_entry as usize)?;
        let mut inputs=Vec::new();let mut missing=0;
        for &(uid,descriptor,cell,_) in &self.placements {
            let Some(assignment)=world.collision.recipes.iter().find(|a|a.owner==ObjectId::Created(uid)) else{
                self.collision_diagnostics.push(format!("UID {uid}: no native collision recipe; nonblocking status UNKNOWN, not verified absent"));missing+=1;continue;
            };
            let source=&assignment.recipe.source;
            let record=ObjectRecord::parse(&source.descriptor,0).context("Invalid collision source descriptor")?;
            let surfaces=assignment.recipe.generated.as_ref().map(|g|g.surface_counts());
            self.collision_diagnostics.push(format!("UID {uid}: source {} descriptor {} flags {:#x}, collider offset {:?}; evidence {:?}, {} manual walls, {} open requirements, mesh face counts {:?}. Native actor class/extents and walkability remain UNKNOWN; floor-tier placement data is not object-owned walkable surface data",source.object.scene,source.object.descriptor_id,record.flags,field_objects::collision_footprint_offset(&record),assignment.recipe.evidence,assignment.recipe.walls.len(),assignment.recipe.nonblocking.len(),surfaces));
            if !matches!(assignment.recipe.evidence,Evidence::UserAuthored{..}) {continue;}
            let cached=world.foreign.iter().find(|f|f.uid==uid).context("Collision UID lost source provenance")?;
            ensure!(cached.source==source.object.scene&&world.collision.foreign_sources.get(&uid)==Some(source),"Collision donor provenance does not match imported UID {uid}");
            let donor=Scene::load(index,&source.object.scene)?;
            ensure!(donor.field_map_index(index)==Some(source.object.map_entry),"Collision donor MAP identity mismatch");
            let source_map=index.entry_bytes(source.object.map_entry)?;
            let source_man=donor.field_man_payload(index)?;
            let actual=ObjectRecord::parse(&map,descriptor).context("Collision destination record absent")?;
            let sx=field_objects::world_x(source.object.cell[0],record.x_off);let sz=field_objects::world_z(source.object.cell[1],record.z_off);
            let draw=world.draws.iter().find(|d|d.editor_uid==Some(uid)).context("Collision owner draw missing")?;
            let final_center=[cached.donor_center[0]+(field_objects::world_x(cell.0,actual.x_off)-sx) as f32,draw.original_center[1]+draw.offset[1],cached.donor_center[2]+(field_objects::world_z(cell.1,actual.z_off)-sz) as f32];
            inputs.push(ImportContribution{assignment:assignment.clone(),source_map:source_map.as_ref().clone(),source_man:source_man.map(|m|m.to_vec()),destination_descriptor:descriptor as u16,destination_cell:[cell.0,cell.1],source_center:cached.donor_center,final_center});
        }
        self.pending_collision_recipes+=missing;
        ensure!(world.collision.mesh_recipes.is_empty(),"Mesh-only collision has no verified native wall/floor contract");
        ensure!(world.collision.recipes.iter().all(|a|matches!(a.owner,ObjectId::Created(uid) if self.placements.iter().any(|p|p.0==uid))),"Uncompiled collision intent outside imported copies; original collision removal needs verified ownership");
        ensure!(inputs.len()==self.placements.len(),"Imported collision requires explicit UserAuthored review for every UID; automatic mesh proposals and missing recipes remain unknown");
        let plan=native_import::compile(&scene.name,map_entry,&map,&inputs).map_err(anyhow::Error::msg)?;
        let output=plan.apply(&map).map_err(anyhow::Error::msg)?;
        let mut staged=DiscPatcher::open(self.disc.image().to_vec())?;
        staged.patch_prot_entry(map_entry as usize,0,&output)?;
        let readback=staged.read_entry_footprint(map_entry as usize)?;
        ensure!(readback==output,"Native imported collision sector readback failed");
        let placed=field_objects::parse_placements(&readback);
        for &(_,descriptor,cell,selector) in &self.placements {
            ensure!(placed.iter().any(|p|p.obj_idx as usize==descriptor&&(p.col,p.row)==cell),"Collision changed native placement lookup");
            let record=ObjectRecord::parse(&readback,descriptor).context("Collision removed native descriptor")?;
            ensure!(field_objects::pack_mesh_index(descriptor as u16,&record)==Some(selector),"Collision changed native model selector");
        }
        let old=legaia_engine_field::minigame_floor::FloorGrid::new(&map);let new=legaia_engine_field::minigame_floor::FloorGrid::new(&readback);
        for z in 0..128 {for x in 0..128 {ensure!(old.height_index(x,z)==new.height_index(x,z)&&old.cell(x,z)==new.cell(x,z)&&old.wall_nibble(x,z)&new.wall_nibble(x,z)==old.wall_nibble(x,z),"Native collision readback changed floor/grid data or cleared original walls");}}
        let lba=staged.entry_disc_lba(map_entry as usize).context("Collision MAP address missing")? as usize;
        for sector in 0..output.len()/2048 {let at=(lba+sector)*2352;ensure!(legaia_iso::write::mode2_form1_sector_is_valid(&staged.image()[at..at+2352]),"Collision MAP sector EDC/ECC failed");}
        self.collision_diagnostics.push(format!("Explicit authored wall intent compiled for UIDs {:?}: {} added bits in {} changed bytes. Original walls/floors retained. Empty authored contributions are NOT proof of native nonblocking behavior; actor/surface/script acceptance is still required",plan.owners,plan.added_bits,plan.changed_bytes));
        self.disc=staged;self.pending_collision_recipes=0;self.native_walls=Some(plan);Ok(())
    }
}

impl NativeExportPlan {
    /// Resolve persisted drag/drop instances through actual native records and
    /// upload declarations, then compile resources and placements together.
    /// This inspectable candidate retains collision intent but is not a writable
    /// export plan; caller budgets/reservations do not certify runtime safety.
    pub(in crate::app) fn compile_saved_imports(
        disc:DiscPatcher,scene:&AssembledScene,world:&WorldModel,index:&ProtIndex,
        limits:super::native_asset_plan::scene_registration::Limits,
        protected:&[super::native_asset_plan::texture_relocation::Rect],
    )->Result<NativeImportCandidate> {
        Self::compile_saved_imports_observed(disc,scene,world,index,limits,protected,None)
    }
    pub(in crate::app) fn compile_saved_imports_observed(
        disc:DiscPatcher,scene:&AssembledScene,world:&WorldModel,index:&ProtIndex,
        limits:super::native_asset_plan::scene_registration::Limits,
        protected:&[super::native_asset_plan::texture_relocation::Rect],
        observations:Option<&super::native_asset_plan::runtime_evidence::VramObservations>,
    )->Result<NativeImportCandidate> {
        use super::native_asset_plan::{NativeAssetPlan,Origin,ResourceKind,texture_relocation::{Planner,multi_tim::{Upload,UploadTrace}},scene_registration};
        let doc=scene_instances::parse(&scene_instances::serialize(&scene.name,world),&scene.name,world).map_err(anyhow::Error::msg)?;
        ensure!(!doc.foreign.is_empty(),"No persisted foreign instances to compile");
        ensure!(doc.moves.is_empty()&&doc.created.is_empty()&&doc.rotations.is_empty()&&doc.scales.is_empty()&&doc.animations.is_empty()&&doc.models.is_empty()&&doc.hidden.is_empty(),"Import candidate cannot omit other transform, actor, visibility or copy edits; compile them in a complete export plan");
        validate_source_archive(&disc,index)?;
        let target=Scene::load(index,&scene.name)?;let map_entry=target.field_map_index(index).context("No destination MAP")? as usize;
        let bundle=disc.read_entry_footprint(map_entry+3)?;
        let pac=if scene_asset_table::streams_scene_pac(&bundle){Some(disc.read_entry_footprint(map_entry+4)?)}else{None};
        let (_,mut before)=scene_registration::native_texture_sequence(&bundle,pac.as_deref(),limits)?;
        let tiers=target.field_floor_height_lut(index)?.context("Destination native floor ladder unresolved")?.map(|y|Some(-(y as i32)));
        let mut staged=disc;let mut last=None;let mut placements=Vec::new();let mut foreign=doc.foreign.clone();foreign.sort_by_key(|f|f.uid);
        let mut reservations=super::native_asset_plan::texture_relocation::field_runtime_reservations().to_vec();reservations.extend_from_slice(protected);
        let mut planner=Planner::new(&before,&reservations)?;
        let mut boot=legaia_tim::Vram::new();index.system_ui_bundle()?.upload_boot_resident_to_vram(&mut boot);planner.reserve_written(&boot);
        if let Some(observed)=observations {planner.reserve_observations(observed)?;}
        for foreign in foreign {
            ensure!(foreign.kind=="placement","Foreign UID {} category {} requires a native actor/terrain/behavior writer",foreign.uid,foreign.kind);
            let cached=world.foreign.iter().find(|f|f.uid==foreign.uid).context("Persisted foreign UID has no provenance")?;
            ensure!(cached.source==foreign.source&&cached.kind==foreign.kind&&cached.record==foreign.record,"Foreign provenance identity mismatch");
            let native=Scene::load(index,&foreign.source)?;
            let assembled=legaia_engine_world_views::scene_staging::assemble_field_scene(index,&foreign.source).map_err(anyhow::Error::msg)?;
            let placed=assembled.placements.get(foreign.record).context("Persisted donor placement missing")?;
            let donor_map=native.field_map_index(index).context("No donor native MAP")? as usize;
            let source_map=index.entry_bytes(donor_map as u32)?;
            let (record_id,_)=record_at(&source_map,placed.cell,(placed.world_x,placed.world_z),placed.env_slot,[placed.rot_x,placed.rot_y,placed.rot_z])?;
            ensure!(cached.native_record.as_ref().is_some_and(|r|source_map[record_id*32..record_id*32+32]==r[..]),"Saved donor descriptor provenance changed or unresolved");
            let source_bundle=index.entry_bytes((donor_map+3) as u32)?;
            let source_pac=if scene_asset_table::streams_scene_pac(&source_bundle){Some(index.entry_bytes((donor_map+4) as u32)?)}else{None};
            let pool=scene_asset_table::mesh_pool(&source_bundle);let model=pool.get(placed.env_slot).context("Donor selector requires another native model adapter")?;
            let mut inventory=NativeAssetPlan::default();let id=inventory.register(ResourceKind::Tmd,&model.bytes,Origin{scene:foreign.source.clone(),entry:Some((donor_map+3) as u32),offset:Some(model.offset)},false)?;
            let (textures,expected)=scene_registration::native_texture_sequence(&source_bundle,source_pac.as_deref().map(|p|p.as_slice()),limits)?;
            let mut events=Vec::new();
            for (i,raw) in textures.iter().enumerate(){let tim=legaia_tim::parse_native_upload(raw)?;let texture=inventory.register(ResourceKind::Tim,raw,Origin{scene:foreign.source.clone(),entry:None,offset:None},false)?;
                events.push(Upload{sequence:Some(i as u32),resource:texture,image:true,clut:tim.clut.is_some(),merge_clut_zeros:false});}
            let trace=UploadTrace::verify_native_replay(&inventory,&events,&expected,"Native descriptor/PAC dispatch order; passive CLUT walker; uploader FUN_800198E0")?;
            let relocation=planner.plan_multi(&inventory,&trace,&[id.clone()])?;
            let (compiled,registration)=Self::compile_scene_resources(&staged,map_entry,&inventory,&before,&relocation,limits)?;
            let source_tiers=native.field_floor_height_lut(index)?.context("Donor native floor ladder unresolved")?.map(|y|Some(-(y as i32)));
            let delta=[0,1,2].map(|axis|foreign.center[axis]-cached.donor_center[axis]);
            let candidate=Self::compile_registered_copy(compiled,map_entry,donor_map,placed.cell,foreign.uid,&id,&registration,delta,&source_tiers,&tiers)?;
            before=relocation.apply(&before)?;
            let NativeImportCandidate{disc,descriptor,cell,uid,selector,..}=candidate;staged=disc;placements.push((uid,descriptor,cell,selector));last=Some((descriptor,cell,uid,selector));
        }
        let (descriptor,cell,uid,selector)=last.context("No native instances compiled")?;
        let mut candidate=NativeImportCandidate{disc:staged,descriptor,cell,uid,selector,pending_collision_recipes:doc.collision_recipes.len()+doc.mesh_collision.len(),placements,native_walls:None,collision_diagnostics:Vec::new()};
        if let Some(observed)=observations {candidate.collision_diagnostics.push(format!("Runtime reservation evidence (not collision ownership): {}",observed.summary()));}
        if let Err(error)=candidate.compile_collision(scene,world,index) {
            candidate.collision_diagnostics.push(format!("Native collision compilation blocked: {error:#}. No unsupported collision intent was accepted"));
        }
        Ok(candidate)
    }
    /// Combine a registered donor resource with an independently allocated
    /// native MAP descriptor. Uses real donor records, never destination donor
    /// swapping. Caller supplies verified floor ladders in native Y coordinates.
    /// No collision ownership is inferred or existing wall/floor bit removed.
    pub(in crate::app) fn compile_registered_copy(
        mut disc:DiscPatcher,map_entry:usize,donor_map_entry:usize,source_cell:(u8,u8),uid:u64,
        model:&super::native_asset_plan::ResourceId,
        registration:&super::native_asset_plan::scene_registration::SceneRegistration,
        delta:[f32;3],source_tiers:&[Option<i32>;16],destination_tiers:&[Option<i32>;16],
    )->Result<NativeImportCandidate> {
        ensure!(source_cell.0<128&&source_cell.1<128,"Donor source outside native grid");
        let donor=disc.read_entry_footprint(donor_map_entry)?;
        ensure!(donor.len()==MAP_LEN,"Donor MAP footprint invalid");
        let grid=GRID_OFF+(source_cell.1 as usize*GRID_DIM+source_cell.0 as usize)*2;
        let source_id=(u16::from_le_bytes(donor[grid..grid+2].try_into()?)&511) as usize;
        let record=ObjectRecord::parse(&donor,source_id).context("Donor record missing")?;
        let pack_index=field_objects::pack_mesh_index(source_id as u16,&record).context("Donor is not a static MAP model")?;
        let donor_bundle=disc.read_entry_footprint(donor_map_entry.checked_add(3).context("Donor bundle overflow")?)?;
        // This supported route has its source models in the descriptor bundle.
        // PAC model contributors need a separate complete selector mapping.
        let source_pool=scene_asset_table::mesh_pool(&donor_bundle);
        let raw=source_pool.get(pack_index as usize).context("Donor model needs another native selector adapter")?;
        ensure!(super::collision_recipes::digest(&raw.bytes)==model.sha256,"Registered model does not match the original donor placement");
        let donor_lba=disc.entry_disc_lba(donor_map_entry).context("Donor MAP address")?;
        let loaded=disc.read_disc_sectors(donor_lba,0x28)?;
        let source_bindings=NativeBindings::parse(&loaded[0x10000..0x12000],&loaded[0x12000..])?;
        let anchor=(source_cell.0 as i32+record.col_delta as i32,source_cell.1 as i32+record.row_delta as i32);
        ensure!((0..128).contains(&anchor.0)&&(0..128).contains(&anchor.1),"Invalid donor anchor");
        ensure!(!anchor_has_bind(&donor,source_cell,&record)&&source_bindings.at((anchor.0 as u8,anchor.1 as u8)).is_none(),"Donor native binding requires MAN/animation relocation");
        let original=disc.read_entry_footprint(map_entry)?;ensure!(original.len()==MAP_LEN,"Destination MAP footprint invalid");
        let lba=disc.entry_disc_lba(map_entry).context("Destination MAP address")?;
        let loaded=disc.read_disc_sectors(lba,0x28)?;
        let bindings=NativeBindings::parse(&loaded[0x10000..0x12000],&loaded[0x12000..])?;
        let edit=InsertEdit{collision_bounds:None,uid,cell:source_cell,
            source_position:(field_objects::world_x(source_cell.0,record.x_off),field_objects::world_z(source_cell.1,record.z_off)),
            env_slot:pack_index as usize,source_rotation:[record.rot_x,record.rot_y,record.rot_z],delta};
        let mut map=original.clone();let copies=insert_copies_with_source_tiers(&mut map,&donor,&[edit],destination_tiers,Some(source_tiers),&bindings)?;
        let copy=copies.first().context("Independent donor allocation missing")?;
        let raw:[u8;32]=map[copy.id*32..copy.id*32+32].try_into()?;
        map[copy.id*32..copy.id*32+32].copy_from_slice(&registration.bind_record(model,&raw)?);
        let selector=registration.selector(model)?;
        let destination_bundle=disc.read_entry_footprint(map_entry+3)?;let pool=scene_asset_table::mesh_pool(&destination_bundle);
        ensure!(pool.get(selector as usize).is_some(),"Final destination model selector unresolved");
        ensure!(original[WALL_OFF..GRID_OFF]==map[WALL_OFF..GRID_OFF]&&original[0x10000..]==map[0x10000..],"Import altered original walls, floors or bindings");
        for i in 0..MAP_LEN {
            let grid=GRID_OFF+(copy.row as usize*GRID_DIM+copy.col as usize)*2;
            ensure!(original[i]==map[i]||(copy.id*32..copy.id*32+32).contains(&i)||(grid..grid+2).contains(&i),"Import changed unrelated MAP byte {i:#x}");
        }
        disc.patch_prot_entry(map_entry,0,&map)?;ensure!(disc.read_entry_footprint(map_entry)?==map,"Imported MAP sector readback mismatch");
        ensure!(disc.read_entry_footprint(donor_map_entry)?==donor,"Original donor MAP changed");
        Ok(NativeImportCandidate{disc,descriptor:copy.id,cell:(copy.col,copy.row),uid,selector,pending_collision_recipes:0,placements:vec![(uid,copy.id,(copy.col,copy.row),selector)],native_walls:None,collision_diagnostics:Vec::new()})
    }
    /// Production resource-byte compiler. This does not construct a writable
    /// NativeExportPlan: physics, complete runtime budgets and placement/spawn
    /// acceptance must still pass plan_export before IMG output is permitted.
    /// Resolve native CDNAME +3/+4 roles relative to the selected MAP entry.
    pub(in crate::app) fn compile_scene_resources(
        disc:&DiscPatcher,map_entry:usize,
        inventory:&super::native_asset_plan::NativeAssetPlan,before:&legaia_tim::Vram,
        relocation:&super::native_asset_plan::texture_relocation::multi_tim::MultiPlan,
        limits:super::native_asset_plan::scene_registration::Limits,
    )->Result<(DiscPatcher,super::native_asset_plan::scene_registration::SceneRegistration)> {
        let bundle_entry=map_entry.checked_add(3).context("Scene bundle index overflow")?;
        let bundle=disc.read_entry_footprint(bundle_entry)?;
        let pac_entry=map_entry.checked_add(4).context("Scene PAC index overflow")?;
        let pac=if scene_asset_table::streams_scene_pac(&bundle){Some(disc.read_entry_footprint(pac_entry)?)}else{None};
        let registered=super::native_asset_plan::scene_registration::register_scene(inventory,&bundle,pac.as_deref(),before,relocation,limits)?;
        registered.validate_source(&bundle)?;
        let mut payloads=std::collections::BTreeMap::new();payloads.insert(bundle_entry,registered.bytes().to_vec());
        if let Some(bytes)=registered.pac_bytes(){registered.validate_pac_source(pac.as_deref().context("Missing source PAC")?)?;payloads.insert(pac_entry,bytes.to_vec());}
        // Work on a private image. A later failure leaves the caller's valid
        // image unchanged. Reuse the existing PROT sector writer.
        for (idx,payload) in &mut payloads {
            let minimum=disc.read_entry_footprint(*idx)?.len();
            let length=payload.len().max(minimum).checked_add(2047).context("Resource sector size overflow")?/2048*2048;
            super::native_asset_plan::scene_registration::validate_stored_extent(length,limits)?;
            payload.resize(length,0);
        }
        let mut compiled=DiscPatcher::open(disc.image().to_vec())?;
        if payloads.iter().any(|(idx,bytes)|disc.entry_footprint(*idx).is_none_or(|n|bytes.len() as u64>n)) {
            compiled.grow_prot_entries(&payloads)?;
        }else{for (idx,bytes) in &payloads {compiled.patch_prot_entry(*idx,0,bytes)?;}}
        ensure!(compiled.entry_count()==disc.entry_count(),"Resource registration changed native entry identities");
        for idx in 0..disc.entry_count() {
            let expected=if let Some(bytes)=payloads.get(&idx){bytes.clone()}else{disc.read_entry_footprint(idx)?};
            ensure!(compiled.read_entry_footprint(idx)?==expected,"Resource patch altered PROT entry {idx}");
        }
        for (idx,bytes) in &payloads {
            let lba=compiled.entry_disc_lba(*idx).context("Registered resource address missing")? as usize;
            for sector in 0..bytes.len()/2048 {
                let start=(lba+sector).checked_mul(2352).context("Resource physical offset overflow")?;
                let raw=compiled.image().get(start..start+2352).context("Resource sector outside image")?;
                ensure!(legaia_iso::write::mode2_form1_sector_is_valid(raw),"Registered resource PROT {idx} sector {sector} failed EDC/ECC readback");
            }
        }
        // Reparse actual sector-readback bytes, not only pre-packaging buffers.
        let check=compiled.read_entry_footprint(bundle_entry)?;
        let expected_pool=scene_asset_table::mesh_pool(registered.bytes());
        let actual_pool=scene_asset_table::mesh_pool(&check);
        ensure!(actual_pool.len()==expected_pool.len()&&actual_pool.iter().zip(&expected_pool).all(|(a,b)|a.bytes==b.bytes),"Registered model lookup failed after native PROT packaging");
        Ok((compiled,registered))
    }
    /// Explicit fixture-only registration bridge. Its return type cannot be
    /// passed to export(); no writable disc plan is constructed here.
    #[cfg(test)]
    pub(in crate::app) fn register_resources_fixture(
        inventory:&super::native_asset_plan::NativeAssetPlan,bundle:&[u8],before:&legaia_tim::Vram,
        relocation:&super::native_asset_plan::texture_relocation::multi_tim::MultiPlan,
        limits:super::native_asset_plan::scene_registration::Limits,
    )->Result<super::native_asset_plan::scene_registration::SceneRegistration> {
        super::native_asset_plan::scene_registration::register(inventory,bundle,before,relocation,limits)
    }
    pub(super) fn summary(&self)->String {
        let mut summary=self.patched.join("; ");
        if !self.dependencies.requests.is_empty() {summary.push_str("\n");summary.push_str(&self.dependencies.summary());}
        if !self.skipped.is_empty() {
            summary.push_str(". Limitations: ");
            summary.push_str(&self.skipped.join("; "));
        }
        summary
    }
}

fn validate_source_archive(disc:&DiscPatcher,index:&ProtIndex)->Result<()> {
    ensure!(disc.entry_count()==index.entry_count(),"Source archive changed; reload the disc");
    for entry in 0..index.entry_count() {
        ensure!(disc.read_entry(entry)?.as_slice()==index.entry_bytes(entry as u32)?.as_slice(),
            "Source PROT entry {entry} changed; reload the disc before export");
    }
    Ok(())
}

pub(super) fn plan_export(source:&Path, scene:&AssembledScene, world:&WorldModel,
    index:&ProtIndex) -> Result<NativeExportPlan> {
    let dependencies=super::native_asset_plan::collect(scene,world,index);
    let inventory=match &dependencies {Ok(plan)=>plan.summary(),Err(error)=>format!("Native asset inventory unresolved: {error:#}")};
    ensure!(world.collision.recipes.is_empty() && world.collision.mesh_recipes.is_empty(),"Authored collision recipes are in-memory only; native IMG export is not enabled for these edits. No collision intent was discarded.\n{}",
        format!("{}\n{}",inventory,super::collision_recipes::export_diagnostics(scene,world,index)));
    let dependencies=dependencies.context("Build native asset dependency inventory")?;
    dependencies.validate_routes().with_context(||inventory.clone())?;
    let source=source.canonicalize().context("Cannot find opened disc image")?;
    ensure!(source.is_file(),"Open a raw 2352-byte .img/.bin source");
    let image=std::fs::read(&source).context("Read source for native export preflight")?;
    ensure!(!image.is_empty() && image.len()%2352==0,"Source is not raw Mode 2 / 2352-byte sector layout");
    let image_length=image.len();
    let disc=DiscPatcher::open(image).context("Parse disc ISO and PROT archive")?;
    // Bind the editor's resource provenance to this exact source archive.
    // Reject stale/replaced source data rather than validate one resource set
    // and serialize another. Includes shared TIM/CLUT and MAN dependencies.
    validate_source_archive(&disc,index)?;
    let original=Scene::load(index,&scene.name)?;
    let map_entry=original.field_map_index(index).context("Selected scene has no native MAP")? as usize;
    let mut edits=prepare_cross_cell(scene,world,index).with_context(||inventory.clone())?;
    if let Some((origin_name,origin_map))=insertion_provenance::context(&source)? {
        let origin=Scene::load(index,&origin_name)?;
        ensure!(origin.field_map_index(index).map(|i|i as usize)==Some(origin_map),"Receipt scene/MAP identity mismatch");
        edits.insertion_provenance=insertion_provenance::load(&source,&disc,origin_map,&origin_name,
            origin.field_floor_height_lut(index)?.context("Native floor ladder needed for insertion provenance")?)?;
    }
    edits.moves.sort_by_key(|e|e.cell);
    edits.terrain_moves.sort_by_key(|e|e.cell);
    edits.deletes.sort_by_key(|e|e.cell);
    edits.copies.sort_by_key(|e|e.uid);
    edits.npcs.sort_by_key(|e|(e.uid,e.record));
    let mut plan=compile_plan(source,disc,image_length,map_entry,&edits).with_context(||inventory)?;
    plan.dependencies=dependencies;
    Ok(plan)
}

fn compile_plan(source:PathBuf, mut disc:DiscPatcher, image_length:usize,
    map_entry:usize, edits:&ExportEdits) -> Result<NativeExportPlan> {
    ensure!(!edits.terrain_moves.is_empty() || !edits.moves.is_empty() || !edits.deletes.is_empty() || !edits.copies.is_empty() || !edits.npcs.is_empty(),"Nothing to export");
    let has_map_edits=!edits.terrain_moves.is_empty() || !edits.moves.is_empty() || !edits.deletes.is_empty() || !edits.copies.is_empty();
    let rim_elm=RIM_ELM_MAPS.iter().any(|(_,id)|*id==map_entry);
    // Terrain draw identity is scene-local. A matching grid coordinate in
    // another story state is not proof that it is the same decoration.
    let has_unscoped=!edits.copies.is_empty() || edits.moves.iter().chain(&edits.deletes)
        .any(|e|!edits.insertion_provenance.as_ref().is_some_and(|p|p.applies_to(map_entry,e.cell)));
    let targets:Vec<(&str,usize)>=if rim_elm&&has_unscoped {RIM_ELM_MAPS.to_vec()} else {vec![("selected",map_entry)]};
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
            // Retail streams 0x28 sectors: MAP plus the next 0x2000-byte
            // fallback window. Read physical adjacency, not guessed entry IDs.
            let lba=disc.entry_disc_lba(idx).context("Missing MAP disc address")?;
            let loaded=disc.read_disc_sectors(lba,0x28).context("Read native MAP trigger fallback")?;
            ensure!(loaded.len()==0x14000,"Incomplete native MAP load window");
            let bindings=NativeBindings::parse(&loaded[0x10000..0x12000],&loaded[0x12000..])?;
            let mut local=edits.clone();
            if idx!=map_entry {local.terrain_moves.clear();}
            if let Some(proof)=&edits.insertion_provenance {
                if map_entry==proof.map_entry&&idx!=proof.map_entry {
                    // Proven additions exist only in their recorded source MAP.
                    // Never remove a different story state's unrelated object.
                    local.moves.retain(|e|!proof.contains(e.cell));
                    local.deletes.retain(|e|!proof.contains(e.cell));
                    local.insertion_provenance=None;
                }
                if map_entry!=proof.map_entry {local.insertion_provenance=None;}
            }
            map_patches_with_bindings(&original,&local,&bindings)
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
    if rim_elm && has_map_edits && has_unscoped {
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
    let continuation_receipt=edits.insertion_provenance.as_ref().map(|p|p.output_receipt(&written)).transpose()?;
    if continuation_receipt.is_some() {
        skipped.push(if has_unscoped {
            "EXPERIMENTAL SOURCE: original edits retain existing story-variant validation; insertion collision evidence is never transferred between scenes. Existing resource/physics/runtime limitations remain; no production approval."
        }else{
            "EXPERIMENTAL SOURCE: verified inserted references are scoped to their recorded MAP; other story-state originals are unchanged. Existing resource/physics/runtime limitations remain; no production approval."
        }.into());
    }
    Ok(NativeExportPlan {dependencies:Default::default(),source,image:written,patched:names,skipped,continuation_receipt})
}

/// Write only bytes already compiled and verified by native preflight.
/// No resource resolution, allocation or compression occurs here.
pub(super) fn export(plan:NativeExportPlan, output:&Path) -> Result<ExportResult> {
    let output_abs=if output.is_absolute(){output.to_path_buf()}else{std::env::current_dir()?.join(output)};
    ensure!(output_abs!=plan.source,"Refusing to overwrite the source image");
    ensure!(!output_abs.exists(),"Destination already exists: {}",output_abs.display());
    let parent=output_abs.parent().context("Output image has no parent directory")?;
    ensure!(parent.is_dir(),"Output directory does not exist");
    let staging=output_abs.with_extension("lckbuilding");
    ensure!(!staging.exists(),"An unfinished staging file exists at {}",staging.display());
    let NativeExportPlan {image:written,patched:names,skipped,continuation_receipt,..}=plan;
    if continuation_receipt.is_some() {
        ensure!(output_abs.file_name().and_then(|n|n.to_str()).is_some_and(|n|n.contains("EXPERIMENTAL")),
            "Input IMG retains experimental native behavior; output filename must contain EXPERIMENTAL");
        ensure!(!output_abs.with_extension("json").exists(),"Experimental continuation receipt already exists");
    }
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
    let receipt_path=output_abs.with_extension("json");
    if let Some(mut receipt)=continuation_receipt {
        receipt["output_image"]=serde_json::json!(output_abs);
        let bytes=serde_json::to_vec_pretty(&receipt)?;
        let mut file=std::fs::OpenOptions::new().create_new(true).write(true).open(&receipt_path)?;
        file.write_all(&bytes)?;file.sync_all()?;
    }
    // Create-only publication also protects against a destination appearing
    // between preflight and finalization. Never overwrite an existing IMG.
    std::fs::hard_link(&staging,&output_abs).context("Publish new native IMG without replacement")?;
    std::fs::remove_file(&staging).context("Remove own staging link")?;
    log::info!("LCK native PS1 Rim Elm export: {}",names.join(", "));
    for entry in &skipped {log::warn!("{entry}");}
    Ok(ExportResult{path:output_abs,patched:names,skipped})
}

#[cfg(test)]
pub(super) mod tests {
    pub(crate) fn actor_fixture() -> (AssembledScene, WorldModel) {
        let scene=AssembledScene {
            name:"synthetic".into(),
            res:legaia_engine_scene::scene_resources::SceneResources {
                vram:legaia_tim::Vram::new(), tim_count:0, tim_parse_failures:0,
                tmds:vec![], anm_packs:vec![], shared_tim_count:0, shared_tmd_count:0,
                render_policy:Default::default(), world_map_slot4:None,
            },
            env_tmds:vec![], placements:vec![], terrain:vec![], ground:None,
            coplanar_offsets:Default::default(),
        };
        let npc=world_view::NpcInfo::synthetic_export_actor();
        let draw=world_view::DrawInfo {
            id:0, terrain:false, index:npc.key, center:[0.0;3], radius:1.0,
            original_center:[0.0;3], offset:[64.0,0.0,0.0], rotation:[0.0;3],
            scale:[1.0;3], vertex_start:0, vertex_end:0, source_ref:None,
            editor_uid:None, display_name:None, hidden:false, hidden_triangles:vec![],
        };
        let world=WorldModel {model:None, instances:1, unavailable:0, unique_models:0,
            ground_triangles:0, ground_vram_covered:0, ground_textured:0,
            draws:vec![draw], npcs:vec![npc], npc_issue:None, foreign:vec![],
            naming:Default::default(),collision:Default::default(),foreign_vram:Default::default()};
        (scene,world)
    }

    fn add_foreign_preview(world:&mut WorldModel) {
        world.foreign.push(world_view::ForeignRef {
            uid:7, source:"donor".into(), kind:"placement".into(), record:3,
            raw_tmd:vec![], native_record:None, donor_wall_bits:vec![], donor_center:[0.0;3],
        });
        world.draws.push(world_view::DrawInfo {
            id:1, terrain:false, index:7, center:[0.0;3], radius:1.0,
            original_center:[0.0;3], offset:[0.0;3], rotation:[0.0;3], scale:[1.0;3],
            vertex_start:0, vertex_end:0, source_ref:Some((false,usize::MAX)),
            editor_uid:Some(7), display_name:None, hidden:false, hidden_triangles:vec![],
        });
    }

    #[test]
    fn cross_cell_preparation_keeps_existing_actor_movement_and_appearance() {
        let (scene,mut world)=actor_fixture();
        world.npcs[0].model_index=1;
        world.npcs[0].clip=1;
        assert_eq!(prepare(&scene,&world).unwrap().npcs.len(),1);
        add_foreign_preview(&mut world);
        let edits=prepare_with_mode(&scene,&world,PreparationMode::CrossCellExport).unwrap();
        assert_eq!(edits.npcs.len(),1);
        let npc=&edits.npcs[0];
        assert_eq!(npc.uid,None);
        assert_eq!(npc.record,1);
        assert_eq!(npc.delta,[64.0,0.0,0.0]);
        assert_eq!((npc.original_model,npc.model,npc.original_anim,npc.anim),(0,1,0,1));
        assert!(prepare_collision_preview(&scene,&world).unwrap().npcs.is_empty());
        assert!(prepare(&scene,&world).is_err(), "strict local export must reject unresolved donors");
    }

    #[test]
    fn cross_cell_preparation_rejects_unsupported_actor_edits_instead_of_omitting_them() {
        let (scene,mut world)=actor_fixture();
        add_foreign_preview(&mut world);
        world.npcs[0].conditional=true;
        assert!(prepare_with_mode(&scene,&world,PreparationMode::CrossCellExport)
            .unwrap_err().to_string().contains("conditional"));
        world.npcs[0].conditional=false;
        world.npcs[0].clip=1;
        world.npcs[0].frame=1;
        assert!(prepare_with_mode(&scene,&world,PreparationMode::CrossCellExport)
            .unwrap_err().to_string().contains("preview frame"));
        assert!(prepare_collision_preview(&scene,&world).is_ok());
    }

    #[test]
    fn native_export_cannot_silently_drop_a_frame_only_edit() {
        let (scene,mut world)=actor_fixture();
        world.draws[0].offset=[0.0;3];
        world.npcs[0].anim_id=1;
        world.npcs[0].clip=1;
        world.npcs[0].frame=1;
        assert!(prepare(&scene,&world).unwrap_err().to_string().contains("preview frame"));
    }

    #[test]
    fn foreign_only_export_can_resolve_dependencies_after_native_preparation() {
        let (scene,mut world)=actor_fixture();
        world.draws[0].offset=[0.0;3];
        add_foreign_preview(&mut world);
        assert!(prepare_with_mode(&scene,&world,PreparationMode::CrossCellExport).is_ok());
        assert!(prepare(&scene,&world).is_err());
    }

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
        // Unrelated binding must remain untouched; no copy may take it over.
        let bind=GRID_OFF+(10usize*128+10)*2;
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
    fn authored_collision_intent_cannot_be_silently_omitted_from_export() {
        let (scene,mut world)=actor_fixture();
        let (_,recipe)=super::super::collision_recipes::fixture();
        world.collision.recipes.push(recipe);
        let index=ProtIndex::from_bytes(legaia_disc_patch::disc::synth::synth_prot(&[]),None).unwrap();
        let error=plan_export(Path::new("not-read.img"),&scene,&world,&index).err().unwrap();
        assert!(error.to_string().contains("Authored collision recipes are in-memory only"));
    }

    #[test]
    fn complete_plan_reads_physical_fallback_before_allocating() {
        let (map,mov,_)=test_map(false);
        let image=map_disc(&map);
        let length=image.len();
        let mut disc=DiscPatcher::open(image).unwrap();
        let rec=ObjectRecord::parse(&map,138).unwrap();
        let mut fallback=vec![0;22];
        fallback[6..8].copy_from_slice(&18i16.to_le_bytes());
        fallback[8..10].copy_from_slice(&1i16.to_le_bytes());
        fallback[18..22].copy_from_slice(&[(mov.cell.0 as i32+rec.col_delta as i32) as u8,
            (mov.cell.1 as i32+rec.row_delta as i32) as u8,17,0]);
        disc.patch_prot_entry(1,0,&fallback).unwrap();
        let error=compile_plan(PathBuf::new(),disc,length,0,&ExportEdits{moves:vec![mov],..Default::default()}).err().unwrap();
        assert!(format!("{error:#}").contains("fallback trigger"));
        assert!(format!("{error:#}").contains("MAN flat record 17"));
    }

    #[test]
    fn fallback_binding_blocks_source_without_grid_bind_bit() {
        let (map,mov,_)=test_map(false);
        let rec=ObjectRecord::parse(&map,138).unwrap();
        let anchor=((mov.cell.0 as i32+rec.col_delta as i32) as u8,(mov.cell.1 as i32+rec.row_delta as i32) as u8);
        let mut fallback=vec![0;22];fallback[6..8].copy_from_slice(&18i16.to_le_bytes());
        fallback[8..10].copy_from_slice(&1i16.to_le_bytes());
        fallback[18..22].copy_from_slice(&[anchor.0,anchor.1,255,7]);
        let bindings=NativeBindings::parse(&map[0x10000..],&fallback).unwrap();
        let error=map_patches_with_bindings(&map,&ExportEdits{moves:vec![mov],..Default::default()},&bindings).unwrap_err();
        assert!(error.to_string().contains("fallback trigger"));
        assert!(error.to_string().contains("MAN flat record 255"));
    }

    #[test]
    fn copy_allocation_avoids_unmarked_fallback_script_anchor() {
        let (mut map,_,copy)=test_map(false);
        map[WALL_OFF..GRID_OFF].fill(4);
        let first=insert_copies(&mut map.clone(),&map,&[copy.clone()],&[None;16]).unwrap().remove(0);
        let rec=ObjectRecord::parse(&map,138).unwrap();
        let anchor=((first.col as i32+rec.col_delta as i32) as u8,(first.row as i32+rec.row_delta as i32) as u8);
        let mut fallback=vec![0;22];fallback[6..8].copy_from_slice(&18i16.to_le_bytes());
        fallback[8..10].copy_from_slice(&1i16.to_le_bytes());
        fallback[18..22].copy_from_slice(&[anchor.0,anchor.1,6,0]);
        let bindings=NativeBindings::parse(&map[0x10000..],&fallback).unwrap();
        let mut after=map.clone();
        let added=insert_copies_with_bindings(&mut after,&map,&[copy],&[None;16],&bindings).unwrap().remove(0);
        assert_ne!((first.col,first.row),(added.col,added.row));
        let grid=GRID_OFF+(first.row as usize*GRID_DIM+first.col as usize)*2;
        assert_eq!(&after[grid..grid+2],&map[grid..grid+2]);
        assert_eq!(&after[0x10000..],&map[0x10000..]);
    }

    #[test]
    fn shared_reference_move_allocates_without_moving_other_instances() {
        let (mut map,mov,mut copy)=test_map(true);
        let other=GRID_OFF+(35*GRID_DIM+34)*2;
        map[other..other+2].copy_from_slice(&138u16.to_le_bytes());
        let selected=GRID_OFF+(mov.cell.1 as usize*GRID_DIM+mov.cell.0 as usize)*2;
        map[selected..selected+2].copy_from_slice(&(138u16|0x2000).to_le_bytes());
        // Copy and move the same base in one plan: the copy must use the
        // original source, not the newly moved independent descriptor.
        copy.delta=[0.0;3];
        let edits=ExportEdits{moves:vec![mov.clone()],copies:vec![copy],..Default::default()};
        let image=map_disc(&map);
        let compile=||compile_plan(PathBuf::new(),DiscPatcher::open(image.clone()).unwrap(),image.len(),0,&edits).unwrap();
        let a=compile();let b=compile();assert_eq!(a.image,b.image);
        let after=DiscPatcher::open(a.image).unwrap().read_entry_footprint(0).unwrap();
        assert_eq!(&after[138*32..139*32],&map[138*32..139*32]);
        assert_eq!(&after[other..other+2],&map[other..other+2]);
        assert_eq!(u16::from_le_bytes(after[selected..selected+2].try_into().unwrap()),511|0x2000);
        let placed=field_objects::parse_placements(&after);
        let moved=placed.iter().find(|p|p.obj_idx==511).unwrap();
        assert_eq!((moved.col,moved.row),mov.cell);
        assert_eq!((moved.world_x,moved.world_z),(mov.source_position.0-16,mov.source_position.1+32));
        let duplicated=placed.iter().find(|p|p.obj_idx==510).unwrap();
        assert_eq!((duplicated.world_x,duplicated.world_z),mov.source_position);
        assert_eq!(&after[WALL_OFF..GRID_OFF],&map[WALL_OFF..GRID_OFF]);
        assert_eq!(&after[0x10000..],&map[0x10000..]);
    }

    #[test]
    fn shared_reference_moves_are_independent_in_one_plan() {
        let (mut map,first,_)=test_map(false);
        let other=GRID_OFF+(35*GRID_DIM+34)*2;
        map[other..other+2].copy_from_slice(&138u16.to_le_bytes());
        let mut second=first.clone();second.cell=(34,35);
        second.source_position.1+=128;second.delta=[32.0,0.0,0.0];
        let mut after=map.clone();
        let changed=move_originals(&mut after,&[first.clone(),second.clone()]).unwrap();
        assert_eq!(changed.iter().map(|r|r.id).collect::<Vec<_>>(),vec![511,510]);
        assert_eq!(&after[138*32..139*32],&map[138*32..139*32]);
        let placed=field_objects::parse_placements(&after);
        assert_eq!(placed.iter().find(|p|p.obj_idx==510).unwrap().world_x,second.source_position.0+32);
        assert_eq!(placed.iter().find(|p|p.obj_idx==511).unwrap().world_x,first.source_position.0-16);
    }

    #[test]
    fn shared_reference_allocation_still_rejects_walls_and_exhaustion() {
        let (mut map,mov,_)=test_map(false);
        let other=GRID_OFF+(35*GRID_DIM+34)*2;
        map[other..other+2].copy_from_slice(&138u16.to_le_bytes());
        let edits=ExportEdits{moves:vec![mov],..Default::default()};
        map[WALL_OFF]=0x10;
        assert!(map_patches(&map,&edits).unwrap_err().to_string().contains("Unresolved collision ownership"));
        map[WALL_OFF]=0;
        for id in 4..512 {if id!=138 {map[id*32]=1;}}
        assert!(map_patches(&map,&edits).unwrap_err().to_string().contains("No completely blank"));
    }

    #[test]
    fn moving_corrected_original_moves_static_collider_same_distance() {
        let (mut m,mov,_)=test_map(true);
        let before=ObjectRecord::parse(&m,138).unwrap();
        let collider=source_collision(&before,34,34);
        assert_eq!(move_originals(&mut m,&[mov]).unwrap(),vec![MovedRecord{id:138,detached_cell:None}]);
        let after=ObjectRecord::parse(&m,138).unwrap();
        assert_eq!(source_collision(&after,34,34),(collider.0-16,collider.1+32));
        assert_eq!(after.sub_anchor_x,-1);
        assert_eq!(after.sub_anchor_z,2);
    }
    #[test]
    fn foreign_floor_tier_numbers_do_not_imply_equal_native_heights() {
        let (source,_,copy)=test_map(true);let mut map=source.clone();
        let bindings=NativeBindings::parse(&source[0x10000..],&[]).unwrap();
        let mut destination=[None;16];destination[4]=Some(0);
        let mut donor=[None;16];donor[4]=Some(-128);
        let inserted=insert_copies_with_source_tiers(&mut map,&source,&[copy.clone()],&destination,Some(&donor),&bindings).unwrap();
        let record=ObjectRecord::parse(&map,inserted[0].id).unwrap();assert_eq!(record.y_off,-128);
        assert_eq!(source[WALL_OFF..GRID_OFF],map[WALL_OFF..GRID_OFF]);
        assert!(insert_copies_with_source_tiers(&mut source.clone(),&source,&[copy],&destination,Some(&[None;16]),&bindings).is_err());
    }
    #[test]
    fn copies_use_empty_slot_and_empty_grid_keep_collision_and_floor() {
        let (original,mov,copy)=test_map(true);
        let edits=ExportEdits{moves:vec![mov],copies:vec![copy],..Default::default()};
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
    fn movement_rejects_unowned_walls_without_changing_grid() {
        let (mut map,mut mov,_)=test_map(false);
        let (off,bit)=wall_offset(68,70).unwrap();
        map[off]=bit|6;
        let before=map.clone();
        mov.collision_bounds=Some([4354,4414,4354,4414]);
        mov.delta=[128.0,0.0,64.0];
        let edits=ExportEdits{moves:vec![mov],..Default::default()};
        assert!(compile_walls(&mut map,&before,&edits).unwrap_err().to_string().contains("ownership"));
        assert_eq!(map,before);
        assert!(map_patches(&before,&edits).is_err());
    }

    #[test]
    fn duplication_does_not_copy_unowned_or_shared_walls() {
        let (mut map,_,mut copy)=test_map(false);
        let (off,bit)=wall_offset(68,70).unwrap();
        map[off]=bit|9;
        copy.collision_bounds=Some([4354,4414,4354,4414]);
        for delta in [[0.0;3],[192.0,0.0,0.0]] {
            copy.delta=delta;
            let before=map.clone();
            let edits=ExportEdits{copies:vec![copy.clone()],..Default::default()};
            assert!(compile_walls(&mut map,&before,&edits).is_err());
            assert_eq!(map,before);
        }
    }

    #[test]
    fn overlapping_movement_and_copy_cannot_claim_shared_wall() {
        let (mut map,mut mov,mut copy)=test_map(false);
        let (off,bit)=wall_offset(68,70).unwrap();
        map[off]=bit|3;
        mov.collision_bounds=Some([4354,4414,4354,4414]);
        copy.collision_bounds=mov.collision_bounds;
        let before=map.clone();
        let edits=ExportEdits{moves:vec![mov],copies:vec![copy],..Default::default()};
        assert!(compile_walls(&mut map,&before,&edits).is_err());
        assert_eq!(map,before);
    }

    #[test]
    fn record_local_collider_edit_preserves_unrelated_walls_and_all_floor_tiers() {
        let (mut map,mut mov,_)=test_map(false);
        let (off,bit)=wall_offset(74,70).unwrap();
        map[off]=bit|11;
        mov.collision_bounds=Some([4354,4414,4354,4414]);
        let before=map.clone();
        let edits=ExportEdits{moves:vec![mov],..Default::default()};
        let patches=map_patches(&before,&edits).unwrap();
        for (off,bytes) in patches {map[off..off+bytes.len()].copy_from_slice(&bytes);}
        assert_eq!(&map[WALL_OFF..GRID_OFF],&before[WALL_OFF..GRID_OFF]);
    }

    #[test]
    fn unknown_bounds_and_foreign_wall_claims_are_not_silently_accepted() {
        let (mut map,mov,copy)=test_map(false);
        let (off,bit)=wall_offset(74,70).unwrap();
        map[off]=bit;
        let before=map.clone();
        assert!(compile_walls(&mut map,&before,&ExportEdits{moves:vec![mov],..Default::default()}).is_err());
        let edits=ExportEdits {foreign_wall_stencils:vec![(copy.uid,vec![(1,1)])],
            copies:vec![copy],..Default::default()};
        assert!(compile_walls(&mut map,&before,&edits).is_err());
        assert_eq!(map,before);
    }

    pub(super) fn map_disc(map:&[u8])->Vec<u8> {
        // The engine's 0x28-sector read includes a fallback trigger window.
        let mut prot=vec![0u8;2048+MAP_LEN+0x2000];
        prot[4..8].copy_from_slice(&2u32.to_le_bytes());
        prot[8..12].copy_from_slice(&1u32.to_le_bytes());
        prot[16..20].copy_from_slice(&1u32.to_le_bytes());
        prot[20..24].copy_from_slice(&37u32.to_le_bytes());
        prot[24..28].copy_from_slice(&41u32.to_le_bytes());
        prot[2048..2048+MAP_LEN].copy_from_slice(map);
        legaia_disc_patch::disc::synth::synth_disc(&prot)
    }

    #[test]
    fn preflight_rejects_stale_resource_bytes_and_script_bound_sources() {
        use legaia_disc_patch::disc::synth::{synth_prot,synth_disc};
        let prot=synth_prot(&[1,2,3]);
        let index=ProtIndex::from_bytes(prot.clone(),None).unwrap();
        let mut disc=DiscPatcher::open(synth_disc(&prot)).unwrap();
        validate_source_archive(&disc,&index).unwrap();
        disc.patch_prot_entry(1,0,&[9]).unwrap();
        assert!(validate_source_archive(&disc,&index).is_err());
        let (mut map,mov,copy)=test_map(false);
        let bind=GRID_OFF+(33*128+33)*2;
        map[bind..bind+2].copy_from_slice(&field_objects::CELL_BIND_OWNED.to_le_bytes());
        assert!(map_patches(&map,&ExportEdits{moves:vec![mov],..Default::default()}).is_err());
        assert!(map_patches(&map,&ExportEdits{copies:vec![copy],..Default::default()}).is_err());
    }

    #[test]
    fn complete_plan_is_deterministic_and_contains_roundtripped_native_allocations() {
        let (map,_,copy)=test_map(false);
        let image=map_disc(&map);
        let edits=ExportEdits{copies:vec![copy],..Default::default()};
        let build=||compile_plan(PathBuf::from("synthetic.img"),DiscPatcher::open(image.clone()).unwrap(),image.len(),0,&edits).unwrap();
        let a=build(); let b=build();
        assert_eq!(a.image,b.image);
        assert_eq!(a.image.len(),image.len());
        let compiled=DiscPatcher::open(a.image).unwrap().read_entry_footprint(0).unwrap();
        assert!(field_objects::parse_placements(&compiled).iter().any(|p|p.obj_idx==511));
        assert_eq!(&compiled[WALL_OFF..GRID_OFF],&map[WALL_OFF..GRID_OFF]);
    }

    fn three_native_placements()->(Vec<u8>,Vec<MoveEdit>) {
        let (mut map,first,_)=test_map(false);let mut edits=vec![first.clone()];
        let record=map[138*32..139*32].to_vec();
        for (id,cell) in [(139usize,(37u8,33u8)),(140,(41,33))] {
            map[id*32..id*32+32].copy_from_slice(&record);
            let off=GRID_OFF+(cell.1 as usize*128+cell.0 as usize)*2;
            map[off..off+2].copy_from_slice(&(id as u16).to_le_bytes());
            map[WALL_OFF+cell.1 as usize*128+cell.0 as usize]=4;
            let rec=ObjectRecord::parse(&map,id).unwrap();
            let mut edit=first.clone();edit.cell=cell;
            edit.source_position=(field_objects::world_x(cell.0,rec.x_off),field_objects::world_z(cell.1,rec.z_off));edits.push(edit);
        }
        for edit in &mut edits {
            edit.delta=[64.0,32.0,-64.0];
            // Explicit synthetic geometry diagnostic extent, not native ownership.
            let (x,z)=edit.source_position;edit.collision_bounds=Some([x-64,x+64,z-64,z+64]);
        }
        (map,edits)
    }
    #[test]
    fn three_terrain_and_three_placements_share_one_native_map_plan() -> Result<()> {
        let (mut map,mut moves)=three_native_placements();
        for edit in &mut moves {edit.delta=[-320.0,0.0,64.0];}
        let mut terrain_moves=Vec::new();
        for (id,cell,selector) in [(82usize,(37u8,18u8),70u16),(180,(41,18),6),(162,(41,22),10)] {
            map[id*32..id*32+32].fill(0);
            map[id*32+0x10..id*32+0x12].copy_from_slice(&selector.to_le_bytes());
            map[id*32+0x12..id*32+0x14].copy_from_slice(&0x812u16.to_le_bytes());
            map[id*32+0x1e]=3;
            let at=GRID_OFF+(cell.1 as usize*128+cell.0 as usize)*2;
            map[at..at+2].copy_from_slice(&(0x2000|id as u16).to_le_bytes());
            let (x,z)=(field_objects::world_x(cell.0,0),field_objects::world_z(cell.1,0));
            terrain_moves.push(MoveEdit{cell,source_position:(x,z),env_slot:selector as usize,source_rotation:[0;3],delta:[-320.0,0.0,64.0],collision_bounds:Some([x-64,x+64,z-64,z+64])});
        }
        let edits=ExportEdits{moves,terrain_moves,..Default::default()};
        let image=map_disc(&map);let original=DiscPatcher::open(image.clone())?;
        let plan=compile_plan(PathBuf::new(),DiscPatcher::open(image.clone())?,image.len(),0,&edits)?;
        let read=DiscPatcher::open(plan.image)?;let updated=read.read_entry_footprint(0)?;
        for (terrain,group) in [(true,&edits.terrain_moves),(false,&edits.moves)] {
            let draws=if terrain {field_objects::parse_terrain_tiles(&updated)}else{field_objects::parse_placements(&updated)};
            for edit in group {
                let found=draws.iter().find(|p|(p.col,p.row)==edit.cell).context("Requested draw missing after mixed native write")?;
                ensure!((found.world_x,found.world_z)==(edit.source_position.0-320,edit.source_position.1+64),"Mixed native translation failed");
            }
        }
        ensure!(updated[WALL_OFF..GRID_OFF]==map[WALL_OFF..GRID_OFF] && updated[0x10000..]==map[0x10000..],"Floor/wall/trigger data changed");
        ensure!(read.read_entry_footprint(1)?==original.read_entry_footprint(1)?,"Unrelated native entry changed");
        Ok(())
    }

    #[test]
    #[ignore="explicit read-only M2 six-request native format investigation"]
    fn m2_six_request_native_inventory()->Result<()> {
        let path=PathBuf::from(std::env::var_os("LCK_M2_SOURCE").context("LCK_M2_SOURCE required")?);
        let library=crate::library::Library::open(&path)?;
        let native=Scene::load(&library.index,"town01")?;
        let scene=legaia_engine_world_views::scene_staging::assemble_field_scene(&library.index,"town01").map_err(anyhow::Error::msg)?;
        let world=world_view::build_world(&scene,&library.index);
        let map_entry=native.field_map_index(&library.index).context("No MAP")?;
        let map=library.index.entry_bytes(map_entry)?;
        println!("Scene {}: {} placements, {} terrain; MAP {}",scene.name,scene.placements.len(),scene.terrain.len(),map_entry);
        for key in [(true,36),(true,37),(true,43),(false,12),(false,13),(false,14)] {
            let draw=if key.0 {&scene.terrain[key.1]}else{&scene.placements[key.1]};
            let at=GRID_OFF+(draw.cell.1 as usize*128+draw.cell.0 as usize)*2;
            let word=u16::from_le_bytes(map[at..at+2].try_into()?);let id=(word&511) as usize;
            let record=ObjectRecord::parse(&map,id).context("No descriptor")?;
            let refs=map[GRID_OFF..0x10000].chunks_exact(2).filter(|r|u16::from_le_bytes([r[0],r[1]])&511==id as u16).count();
            println!("{key:?}: {draw:?}; grid={word:#06x}, descriptor={id}, references={refs}; record={record:?}");
            println!("Geometry {} vertices, {} objects; diagnostic walls: {}",scene.res.tmds[draw.res_tmd].tmd.objects.iter().map(|o|o.vertices.len()).sum::<usize>(),scene.res.tmds[draw.res_tmd].tmd.objects.len(),collision_ownership_diagnostic(&world,key,&map));
            if !key.0 {println!("{}",super::super::native_dependency_report::report(&scene,&world,key,&library.index).map_err(anyhow::Error::msg)?);}
        }
        Ok(())
    }
    #[test]
    fn three_moved_and_three_deleted_placements_export_and_reopen_synthetic_imgs() {
        let (map,edits)=three_native_placements();let image=map_disc(&map);
        let folder=std::env::current_dir().unwrap().join("target").join(format!("m1-native-{}-{}",std::process::id(),std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        std::fs::create_dir_all(&folder).unwrap();
        let source=folder.join("synthetic-source.img");std::fs::write(&source,&image).unwrap();
        for deleting in [false,true] {
            let batch=if deleting {ExportEdits{deletes:edits.clone(),..Default::default()}}
                else {ExportEdits{moves:edits.clone(),..Default::default()}};
            let make=||compile_plan(source.clone(),DiscPatcher::open(image.clone()).unwrap(),image.len(),0,&batch).unwrap();
            let a=make();let b=make();assert_eq!(a.image,b.image);
            let output=folder.join(if deleting {"three-deleted.img"}else{"three-moved.img"});
            export(a,&output).unwrap();
            let bytes=std::fs::read(&output).unwrap();assert!(export(make(),&output).is_err());
            assert_eq!(std::fs::read(&output).unwrap(),bytes);assert_eq!(std::fs::read(&source).unwrap(),image);
            let read=DiscPatcher::open(bytes).unwrap();let result=read.read_entry_footprint(0).unwrap();
            let placed=field_objects::parse_placements(&result);
            for edit in &edits {
                let found=placed.iter().find(|p|(p.col,p.row)==edit.cell);
                if deleting {assert!(found.is_none());}
                else {let p=found.unwrap();assert_eq!((p.world_x,p.world_z),(edit.source_position.0+64,edit.source_position.1-64));
                    let r=ObjectRecord::parse(&result,p.obj_idx as usize).unwrap();let old=ObjectRecord::parse(&map,p.obj_idx as usize).unwrap();assert_eq!(r.y_off,old.y_off-32);}
            }
            if deleting {assert_eq!(&result[..WALL_OFF],&map[..WALL_OFF]);}
            assert_eq!(&result[WALL_OFF..GRID_OFF],&map[WALL_OFF..GRID_OFF]);assert_eq!(&result[0x10000..],&map[0x10000..]);
            let original=DiscPatcher::open(image.clone()).unwrap();assert_eq!(read.read_entry_footprint(1).unwrap(),original.read_entry_footprint(1).unwrap());
            let start=read.entry_disc_lba(0).unwrap() as usize;
            for sector in 0..MAP_LEN/2048 {let offset=(start+sector)*2352;assert!(legaia_iso::write::mode2_form1_sector_is_valid(&read.image()[offset..offset+2352]));}
        }
    }
    #[test]
    fn native_batch_deletion_rejects_unknown_walls_bindings_shared_grid_and_conflicts() {
        let (map,edits)=three_native_placements();let batch=ExportEdits{deletes:edits.clone(),..Default::default()};
        let mut distant=map.clone();distant[WALL_OFF+1]|=0x10;
        let patches=map_patches(&distant,&batch).unwrap();
        assert!(patches.iter().all(|(off,_)|*off>=GRID_OFF));
        let mut walls=distant.clone();let (x,z)=edits[0].source_position;
        let (off,bit)=wall_offset(x.div_euclid(64),z.div_euclid(64)+2).unwrap();walls[off]|=bit;
        let error=format!("{:#}",map_patches(&walls,&batch).unwrap_err());
        assert!(error.contains("spatially relevant")&&error.contains("unresolved ownership")&&error.contains("sub-cell examples"));
        assert_eq!(walls[WALL_OFF+1],0x10);
        let mut shared=map.clone();let off=GRID_OFF+(edits[0].cell.1 as usize*128+edits[0].cell.0 as usize)*2;
        shared[off+1]|=0xb8; // draw gates + slope and ground-order bits
        let mut result=shared.clone();for (at,bytes) in map_patches(&shared,&batch).unwrap(){result[at..at+bytes.len()].copy_from_slice(&bytes);}
        assert_eq!(u16::from_le_bytes(result[off..off+2].try_into().unwrap()),0x8800);
        assert_eq!(&result[WALL_OFF..GRID_OFF],&shared[WALL_OFF..GRID_OFF]);
        let mut conflict=batch.clone();conflict.moves.push(edits[0].clone());assert!(map_patches(&map,&conflict).is_err());
        conflict=batch.clone();conflict.deletes.push(edits[0].clone());assert!(map_patches(&map,&conflict).is_err());
        let mut bound=map.clone();let rec=ObjectRecord::parse(&bound,138).unwrap();
        let (c,r)=edits[0].cell;let anchor=GRID_OFF+((r as i32+rec.row_delta as i32) as usize*128+(c as i32+rec.col_delta as i32) as usize)*2;
        bound[anchor+1]|=(field_objects::CELL_BIND_OWNED>>8) as u8;assert!(map_patches(&bound,&batch).is_err());
    }

    #[test]
    #[ignore="explicit read-only real Rim Elm deletion dependency inventory"]
    fn real_rim_elm_deletion_dependency_inventory()->Result<()> {
        let path=PathBuf::from(std::env::var_os("LCK_P0_SOURCE").context("LCK_P0_SOURCE required")?);
        let library=crate::library::Library::open(&path)?;
        let native=Scene::load(&library.index,"town01")?;
        let assembled=legaia_engine_world_views::scene_staging::assemble_field_scene(&library.index,"town01").map_err(anyhow::Error::msg)?;
        let world=world_view::build_world(&assembled,&library.index);
        let disc=DiscPatcher::open(std::fs::read(&path)?)?;
        let map_entry=native.field_map_index(&library.index).context("No MAP")? as usize;
        let map=disc.read_entry_footprint(map_entry)?;
        let loaded=disc.read_disc_sectors(disc.entry_disc_lba(map_entry).context("No MAP LBA")?,0x28)?;
        let bindings=NativeBindings::parse(&loaded[0x10000..0x12000],&loaded[0x12000..])?;
        let mut rows=Vec::new();let mut accepted=0;
        for (index,p) in assembled.placements.iter().enumerate() {
            let edit=MoveEdit{cell:p.cell,source_position:(p.world_x,p.world_z),env_slot:p.env_slot,
                source_rotation:[p.rot_x,p.rot_y,p.rot_z],delta:[0.0;3],collision_bounds:original_collision_bounds(&world,index)};
            let id=(u16::from_le_bytes(map[GRID_OFF+(p.cell.1 as usize*128+p.cell.0 as usize)*2..][..2].try_into()?)&511) as usize;
            let status=(||->Result<()>{native_deletion::analyze(&map,&edit,&bindings,None)?.validate()?;
                let edits=ExportEdits{deletes:vec![edit.clone()],..Default::default()};
                let mut output=map.clone();for (off,bytes) in map_patches_with_bindings(&map,&edits,&bindings)? {output[off..off+bytes.len()].copy_from_slice(&bytes);}
                ensure!(output[..GRID_OFF]==map[..GRID_OFF]&&output[0x10000..]==map[0x10000..],"Deletion altered native records/walls/floors/bindings");Ok(())})();
            let outcome=match status {Ok(())=>{accepted+=1;"Native deletion bytes/readback accepted; runtime not verified".to_string()},Err(error)=>format!("{error:#}")};
            println!("Rim Elm record #{id}, cell {:?}, model {}: {outcome}",p.cell,p.env_slot);
            rows.push(serde_json::json!({"descriptor":id,"cell":p.cell,"model":p.env_slot,"bounds":edit.collision_bounds,"result":outcome}));
        }
        println!("Rim Elm: {accepted}/{} per-instance deletion candidates; no IMG generated",rows.len());
        if let Some(out)=std::env::var_os("LCK_M11_REPORT") {
            use std::io::Write;
            let mut file=std::fs::OpenOptions::new().write(true).create_new(true).open(out)?;
            file.write_all(&serde_json::to_vec_pretty(&serde_json::json!({"source":path,"scene":"town01","accepted":accepted,"rows":rows}))?)?;
        }
        Ok(())
    }

    #[test]
    #[ignore="explicit genuine native insertion continuation; optional new experimental IMG outputs"]
    fn real_native_insertions_group_move_delete_and_mixed_export()->Result<()> {
        let path=PathBuf::from(std::env::var_os("LCK_P0_SOURCE").context("LCK_P0_SOURCE required")?);
        let library=crate::library::Library::open(&path)?;
        let name=std::env::var("LCK_M11_SCENE").context("LCK_M11_SCENE required")?;
        let native=Scene::load(&library.index,&name)?;
        let assembled=legaia_engine_world_views::scene_staging::assemble_field_scene(&library.index,&name).map_err(anyhow::Error::msg)?;
        let world=world_view::build_world(&assembled,&library.index);
        let original=DiscPatcher::open(std::fs::read(&path)?)?;
        let map_entry=native.field_map_index(&library.index).context("No MAP")? as usize;
        let lut=native.field_floor_height_lut(&library.index)?.context("No native floor ladder")?;
        let proof=insertion_provenance::load(&path,&original,map_entry,&name,lut)?.context("Verified native addition receipt required")?;
        let selected=assembled.placements.iter().enumerate().filter(|(_,p)|proof.contains(p.cell)).map(|(i,_)|(false,i)).collect::<Vec<_>>();
        ensure!(selected.len()==2,"Fixture expects two real independent added instances, not object-specific donor IDs");
        let source_hash=super::super::collision_recipes::digest(original.image());
        let out=if let Some(dir)=std::env::var_os("LCK_M11_EXPORT_DIR") {
            let dir=PathBuf::from(dir);ensure!(dir.is_absolute()&&!dir.exists()&&dir.parent().is_some_and(|p|p.is_dir()),"Use a NEW output directory");std::fs::create_dir(&dir)?;Some(dir)
        }else{None};
        for case in ["move_two","delete_two","move_one_delete_one"] {
            let mut edited=world.clone();
            // Route the actual selected set through the same batch editing
            // helpers as the workspace, before persistence and native planning.
            let mut primary=None;let mut selection=std::collections::BTreeSet::new();
            for &key in &selected {super::super::multi_selection::viewport_press(&mut primary,&mut selection,key,true,false);}
            ensure!(selection.len()==2,"Shared selection state lost a native instance");
            if case=="delete_two" {
                super::super::multi_selection::delete(&mut edited,&selection).map_err(anyhow::Error::msg)?;
            } else {
                let moving=if case=="move_two" {selection.clone()}else{std::collections::BTreeSet::from([selected[0]])};
                let moves=moving.iter().map(|&key|(key,[64.0,0.0,0.0])).collect::<Vec<_>>();
                super::super::multi_selection::move_offsets(&mut edited,&moves).map_err(anyhow::Error::msg)?;
                if case=="move_one_delete_one" {super::super::multi_selection::delete(&mut edited,&std::collections::BTreeSet::from([selected[1]])).map_err(anyhow::Error::msg)?;}
            }
            // Save/reopen uses the same source and existing transactional restore.
            let text=scene_instances::serialize(&name,&edited);let doc=scene_instances::parse(&text,&name,&world).map_err(anyhow::Error::msg)?;
            let mut reopened=world.clone();scene_instances::apply(&doc,&mut reopened,assembled.placements.len(),&library.index).map_err(anyhow::Error::msg)?;
            ensure!(scene_instances::serialize(&name,&reopened)==text,"Saved group intent changed on restore");
            let plan=plan_export(&path,&assembled,&reopened,&library.index)?;
            ensure!(plan.continuation_receipt.is_some(),"Experimental input status was lost");
            let read=DiscPatcher::open(plan.image.clone())?;let map=read.read_entry_footprint(map_entry)?;
            let before=original.read_entry_footprint(map_entry)?;
            ensure!(map[WALL_OFF..GRID_OFF]==before[WALL_OFF..GRID_OFF]&&map[0x10000..]==before[0x10000..],"Native wall/floor/binding data changed");
            let start=original.entry_disc_lba(map_entry).context("No native MAP LBA")? as usize*2352;
            let end=start+MAP_LEN/2048*2352;
            ensure!(plan.image[..start]==original.image()[..start]&&plan.image[end..]==original.image()[end..],"Unrelated raw IMG/resource/other-story-state bytes changed");
            for (i,&key) in selected.iter().enumerate() {
                let p=&assembled.placements[key.1];let off=GRID_OFF+(p.cell.1 as usize*128+p.cell.0 as usize)*2;
                let word=u16::from_le_bytes(map[off..off+2].try_into()?);
                if case=="delete_two"||(case=="move_one_delete_one"&&i==1) {
                    ensure!(word&511==0,"Deleted native reference survived");
                    ensure!(!legaia_engine_field::field_regions::window_rebuild_spawns(&map,(0,0,128,128),lut.map(|v|-v)).iter().any(|s|s.tile==p.cell),"Deleted actor still appears in native window-sweep readback");
                }else{
                    let record=ObjectRecord::parse(&map,(word&511) as usize).context("Moved native record missing")?;
                    ensure!(field_objects::world_x(p.cell.0,record.x_off)==p.world_x+64&&field_objects::world_z(p.cell.1,record.z_off)==p.world_z,"Shared native movement delta lost");
                }
            }
            for sector in start/2352..end/2352 {ensure!(legaia_iso::write::mode2_form1_sector_is_valid(&plan.image[sector*2352..(sector+1)*2352]),"Native sector EDC/ECC failed");}
            if let Some(dir)=&out {
                let output=dir.join(format!("LCK_EXPERIMENTAL_M11_{name}_{case}.img"));
                export(plan,&output)?;
                let disk=DiscPatcher::open(std::fs::read(&output)?)?;
                ensure!(disk.read_entry_footprint(map_entry)?==map,"Final on-disk MAP readback changed");
                ensure!(insertion_provenance::load(&output,&disk,map_entry,&name,lut)?.is_some(),"Continuation receipt failed actual-byte verification");
                println!("{case}: {} ; actual native bytes verified, gameplay NOT verified",output.display());
            }
        }
        ensure!(super::super::collision_recipes::digest(&std::fs::read(&path)?)==source_hash,"Source IMG changed");
        Ok(())
    }

    #[test]
    fn complete_plan_rejects_exhausted_record_allocation_and_unowned_collision() {
        let (mut map,_,mut copy)=test_map(false);
        for id in 4..512 {if id!=138 {map[id*32]=1;}}
        let image=map_disc(&map);
        let edits=ExportEdits{copies:vec![copy.clone()],..Default::default()};
        assert!(compile_plan(PathBuf::new(),DiscPatcher::open(image.clone()).unwrap(),image.len(),0,&edits).is_err());
        let (mut map,_,_)=test_map(false);
        let (off,bit)=wall_offset(68,70).unwrap(); map[off]=bit;
        copy.collision_bounds=Some([4354,4414,4354,4414]);
        let image=map_disc(&map);
        assert!(compile_plan(PathBuf::new(),DiscPatcher::open(image.clone()).unwrap(),image.len(),0,
            &ExportEdits{copies:vec![copy],..Default::default()}).is_err());
    }

    #[test]
    fn complete_plan_does_not_accept_npc_edits_without_serializable_man() {
        let (map,_,copy)=test_map(false);
        let (scene,world)=actor_fixture();
        let mut edits=prepare(&scene,&world).unwrap(); edits.copies.push(copy);
        let image=map_disc(&map);
        let error=compile_plan(PathBuf::new(),DiscPatcher::open(image.clone()).unwrap(),image.len(),0,&edits).err().expect("missing MAN must fail");
        assert!(format!("{error:#}").contains("exactly one nearby scene MAN"));
    }

    #[test]
    fn rim_elm_story_state_indices_are_preserved() {
        assert_eq!(RIM_ELM_MAPS.iter().map(|(_,i)|*i).collect::<Vec<_>>(),vec![1,10,19,345,739]);
    }
}

#[cfg(test)]
#[path = "native_export_verification.rs"]
mod controlled_verification;

#[cfg(test)]
#[path = "native_terrain_tests.rs"]
mod terrain_tests;
