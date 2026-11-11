//! LCK native reflection prototype: per-instance independent TMD geometry.
//! This module does not encode editor group membership. Each group member is
//! compiled through the ordinary individual native placement/collision writers.
//! Every mirror must register/bind its own native TMD or the export fails.
use std::{collections::{BTreeSet,HashMap},convert::TryInto};
use anyhow::{Result,Context,ensure};
use legaia_disc_patch::disc::DiscPatcher;
use legaia_engine_scene::{scene::ProtIndex,scene_assembly::AssembledScene};
use legaia_scene_formats::field_objects::{self,ObjectRecord};
use super::{native_asset_plan::{NativeAssetPlan,Origin,ResourceKind,scene_registration as reg,
    texture_relocation::multi_tim::MultiPlan},
    scene_instances::{SceneDocument,NameTarget},world_view::WorldModel,ps1_export};

const MAP_GRID_OFFSET:usize=0x8000;
const MAP_GRID_WIDTH:usize=128;
const EPS:f32=0.0001;

/// Transform a complete native Legaia TMD in its own resource allocation.
/// Vertex coordinates and normals are transformed, and negative-determinant
/// transforms reverse primitive winding together with per-corner UVs, colours
/// and normal references. Do not mutate a shared source TMD in place.
fn transformed_tmd(source:&[u8],scale:[f32;3])->Result<Vec<u8>>{
    ensure!(scale.iter().all(|v|v.is_finite() && v.abs()>=0.05 && v.abs()<=20.0),
        "TMD scale outside the verified editor range");
    let decoded=legaia_tmd::parse(source).context("Read original native TMD")?;
    ensure!(!decoded.objects.is_empty(),"TMD has no mesh objects");
    let mut out=source.to_vec();
    let mut mins=[i16::MAX;3];let mut maxs=[i16::MIN;3];let mut count=0usize;
    for obj in &decoded.objects {for v in &obj.vertices {
        for (axis,n) in [v.x,v.y,v.z].into_iter().enumerate(){mins[axis]=mins[axis].min(n);maxs[axis]=maxs[axis].max(n);}
        count+=1;
    }}
    ensure!(count>0,"TMD contains no vertices");
    let center=[0,1,2].map(|i|(mins[i] as f32+maxs[i] as f32)*0.5);
    let flip=scale.iter().filter(|v|**v<0.).count()%2==1;
    let mut touched=BTreeSet::new();
    for obj in &decoded.objects {
        // The parser already bounds these spans, but guard arithmetic again
        // before performing any native byte modifications.
        for (top,size,is_normal) in [
            (obj.header.vert_top,obj.vertices.len(),false),
            (obj.header.normal_top,obj.normals.len(),true),
        ]{
            if size==0{continue;}
            let start=legaia_tmd::HEADER_SIZE.checked_add(top as usize)
                .context("TMD vector start overflow")?;
            let byte_len=size.checked_mul(legaia_tmd::VECTOR_SIZE)
                .context("TMD vector span overflow")?;
            ensure!(start.checked_add(byte_len).is_some_and(|end|end<=out.len()),
                "TMD vector span outside resource");
            for v in 0..size {
                let offset=start+v*legaia_tmd::VECTOR_SIZE;
                if !touched.insert(offset){continue;}
                let mut values=[0_i16;3];
                for axis in 0..3 {
                    let at=offset+axis*2;
                    values[axis]=i16::from_le_bytes(out[at..at+2].try_into()?);
                }
                let transformed=if is_normal {
                    let inv=[0,1,2].map(|i|values[i] as f32/scale[i]);
                    let length=(inv.iter().map(|v|v*v).sum::<f32>()).sqrt();
                    let original_length=(values.iter().map(|v|(*v as f32).powi(2)).sum::<f32>()).sqrt();
                    if length<EPS { [0.0;3] }
                    else {[0,1,2].map(|i|inv[i]*original_length/length)}
                }else{[0,1,2].map(|i|center[i]+(values[i] as f32-center[i])*scale[i])};
                for (axis,value) in transformed.into_iter().enumerate(){
                    ensure!(value.is_finite()&&value.round()>=i16::MIN as f32&&value.round()<=i16::MAX as f32,
                        "Native TMD vertex/normal exceeds signed 16-bit coordinate budget");
                    let at=offset+axis*2;
                    out[at..at+2].copy_from_slice(&(value.round() as i16).to_le_bytes());
                }
            }
        }
        let groups=legaia_tmd::legaia_prims::iter_groups(
            source,obj.primitives_byte_offset,obj.primitives_byte_size)?;
        ensure!(groups.iter().map(|g|g.prims.len()).sum::<usize>()==obj.claimed_n_primitive as usize,
            "Native primitive walk was incomplete; cannot mirror without corrupting winding");
        for group in groups {
            let descriptor=legaia_tmd::descriptor::Descriptor::for_flags(group.header.flags)
                .context("Unrecognized native primitive mode; mirror not exported")?;
            let n=group.header.n_vertices();
            let permutation:&[usize]=if n==4{&[1,0,3,2]}else{&[1,0,2]};
            for prim in group.prims {
                ensure!(prim.vertex_indices_raw.len()==n,
                    "TMD primitive vertex count mismatch");
                let offset=prim.bytes_offset;
                let size=prim.bytes_size;
                ensure!(offset.checked_add(size).is_some_and(|end|end<=out.len()),
                    "TMD primitive extends outside source");
                if !flip {continue;}
                let original=source[offset..offset+size].to_vec();
                let vertices=descriptor.vertex_offset;
                ensure!(vertices+n*2<=size,"Native vertex-index packet truncated");
                for (dest,from) in permutation.iter().copied().enumerate(){
                    let at=offset+vertices+dest*2;
                    out[at..at+2].copy_from_slice(&original[vertices+from*2..vertices+from*2+2]);
                }
                if let Some(uv)=descriptor.texture_block_offset {
                    ensure!(uv+4+n*2<=size,"Native texture packet truncated");
                    let uv_slots=[0usize,4,8,10];
                    for (dest,from) in permutation.iter().copied().enumerate(){
                        let at=offset+uv+uv_slots[dest];
                        out[at..at+2].copy_from_slice(&original[uv+uv_slots[from]..uv+uv_slots[from]+2]);
                    }
                }
                // The baked-colour Gouraud row carries one 4-byte word per
                // corner at packet start. Lit rows do not have this block.
                if descriptor.table_row==5 {
                    ensure!(n*4<=size,"Gouraud colour packet truncated");
                    for (dest,from) in permutation.iter().copied().enumerate(){
                        let at=offset+dest*4;
                        out[at..at+4].copy_from_slice(&original[from*4..from*4+4]);
                    }
                }
                // The lit Gouraud groups carry one normal index after each
                // vertex-index word. Other groups use a single face normal.
                if descriptor.table_row==1 || descriptor.table_row==3 {
                    let normal=vertices+n*2;
                    ensure!(normal+n*2<=size,"Lit Gouraud normal-index packet truncated");
                    for (dest,from) in permutation.iter().copied().enumerate(){
                        let at=offset+normal+dest*2;
                        out[at..at+2].copy_from_slice(&original[normal+from*2..normal+from*2+2]);
                    }
                }
            }
        }
    }
    let check=legaia_tmd::parse(&out)?;
    ensure!(check.objects.len()==decoded.objects.len(),"Mirrored TMD object count differs");
    for obj in &check.objects {
        let groups=legaia_tmd::legaia_prims::iter_groups(
            &out,obj.primitives_byte_offset,obj.primitives_byte_size)?;
        ensure!(groups.iter().map(|g|g.prims.len()).sum::<usize>()==obj.claimed_n_primitive as usize,
            "Mirrored TMD primitive readback incomplete");
        for group in groups {for prim in group.prims {
            ensure!(prim.vertex_indices_raw.iter().all(|i|
                *i%8==0 && (*i as usize/8)<obj.vertices.len()),
                "Mirrored TMD primitive has out-of-range vertex index");
        }}
    }
    Ok(out)
}

fn source_cell(scene:&AssembledScene,key:(bool,usize))->Result<(u8,u8)>{
    Ok(if key.0{scene.terrain.get(key.1)}else{scene.placements.get(key.1)}
       .context("Native original draw not found")?.cell)
}
fn source_rotation(index:&ProtIndex,scene:&AssembledScene,world:&WorldModel,
    target:&NameTarget,entry:usize)->Result<(i32,i32,i32)>{
    let original_record=|key:(bool,usize)|->Result<ObjectRecord>{
        let cell=source_cell(scene,key)?;
        let source_map=index.entry_bytes(entry as u32)?;
        let id=ps1_export::unrestricted::id_at(&source_map,cell)?;
        ObjectRecord::parse(&source_map,id).context("Original native rotation source missing")
    };
    let rec=match target {
        NameTarget::Native(terrain,index)=>original_record((*terrain,*index))?,
        NameTarget::Created(uid)=>{
            if let Some(foreign)=world.foreign.iter().find(|f|f.uid==*uid){
                if let Some(raw)=&foreign.native_record {ObjectRecord::parse(raw,0).context("Foreign mirror native record missing")?}
                else{return Ok((0,0,0))}
            }else{
                let draw=world.draws.iter().find(|d|d.editor_uid==Some(*uid))
                    .context("Copy has no stable original draw")?;
                original_record(draw.source_ref.context("Copy native donor identity unavailable")?)?
            }
        }
    };
    Ok((rec.rot_x.into(),rec.rot_y.into(),rec.rot_z.into()))
}

fn native_scale(index:&ProtIndex,scene:&AssembledScene,world:&WorldModel,
    target:&NameTarget,entry:usize,scale:[f32;3])->Result<[f32;3]>{
    let (rx,ry,rz)=source_rotation(index,scene,world,target,entry)?;
    ensure!(rx.rem_euclid(4096)==0 && rz.rem_euclid(4096)==0,
        "Native source has pitched/rolled geometry; world-space reflection needs a full orientation adapter");
    let turn=ry.rem_euclid(4096);
    ensure!(turn%1024==0,
        "Native source rotation is not a 90-degree multiple; reflection cannot use an axis-only TMD transform");
    let mut local=scale;
    if turn%2048!=0 {local.swap(0,2);}
    Ok(local)
}

/// Register a new native model for every scaled/mirrored MAP instance. Does
/// not infer script binding ownership; rejects MAN NPCs and script-owned MAP
/// anchors. Existing scene resources and all other MAP references are retained.
/// A failing mirror aborts the export preparation, never writes a ghost.
pub(in crate::app) fn apply(
    disc:&mut DiscPatcher,entry:usize,scene:&AssembledScene,world:&WorldModel,
    index:&ProtIndex,doc:&SceneDocument,
    targets:&HashMap<NameTarget,(u8,u8)>,log:&mut Vec<String>,
)->Result<()> {
    for (target,scale) in &doc.scales {
        // The mirror tool sets negative X/Z scales. Keep unrelated legacy
        // positive-only scales on their previous writer path for now.
        if scale[0]>=0. && scale[2]>=0. {continue;}
        ensure!(scale[1]>0.,"Negative Y mirroring is not a supported LCK operation");
        let draw=match target {
            NameTarget::Native(t,i)=>world.at(*t,*i),
            NameTarget::Created(uid)=>world.draws.iter().find(|d|d.editor_uid==Some(*uid)),
        }.context("Mirrored object no longer exists")?;
        ensure!(!draw.hidden,"Cannot register a TMD for a deleted object");
        let draw_key=(draw.terrain,draw.index);
        ensure!(world.npc(draw_key).is_none(),
            "MAN NPC reflection requires a separate native MAN actor resource writer");
        let local_scale=native_scale(index,scene,world,target,entry,*scale)?;
        let cell=match target {
            NameTarget::Native(t,i)=>source_cell(scene,(*t,*i))?,
            NameTarget::Created(_)=>*targets.get(target)
                .context("Mirrored copy has no exported MAP reference")?,
        };
        let mut map=disc.read_entry_footprint(entry)?;
        let ref_id=ps1_export::unrestricted::id_at(&map,cell)?;
        ensure!(ref_id>=4,"Mirrored MAP reference does not resolve to a placed/static mesh");
        let record=ObjectRecord::parse(&map,ref_id).context("Mirrored MAP descriptor missing")?;
        let col=cell.0 as i32+record.col_delta as i32;
        let row=cell.1 as i32+record.row_delta as i32;
        ensure!((0..128).contains(&col)&&(0..128).contains(&row),
            "Mirrored MAP anchor outside native grid");
        let at=MAP_GRID_OFFSET+(row as usize*MAP_GRID_WIDTH+col as usize)*2;
        let anchor_word=u16::from_le_bytes(map[at..at+2].try_into()?);
        ensure!(anchor_word&field_objects::CELL_BIND_OWNED==0,
            "Mirrored object has an active MAP/MAN script binding; script-aware model registration is required");
        ensure!(record.is_placed()||record.flags==0x0012,
            "Native mirror supports placed MAP objects and unbound static decorations only");
        let bundle=disc.read_entry_footprint(entry+3)?;
        let pac=reg::native_stream_mode(&bundle)
            .map(|_|disc.read_entry_footprint(entry+4)).transpose()?;
        let models=reg::native_model_pool(&bundle,pac.as_deref());
        let original=models.get(record.pack_index_field as usize)
            .context("Current native model selector is not registered")?;
        let mirrored=transformed_tmd(&original.bytes,local_scale)?;
        ensure!(mirrored!=original.bytes,"Mirroring produced no geometry change");
        let mut inventory=NativeAssetPlan::default();
        let tmd=inventory.register(ResourceKind::Tmd,&mirrored,
            Origin{scene:scene.name.clone(),entry:None,offset:None},false)?;
        let limits=reg::Limits{
            max_models:u16::MAX as usize-field_objects::FIELD_ACTOR_PACK_BIAS as usize,
            max_decoded:usize::MAX,max_encoded:usize::MAX,
        };
        let (_,before)=reg::native_texture_sequence(&bundle,pac.as_deref(),limits)?;
        // Native mirror changes geometry only. No texture allocation or VRAM
        // changes are permitted: reuse the original resident material words.
        let relocation=MultiPlan::existing_material_tmd(&inventory,&before,&tmd)?;
        let (mut encoded,registration)=ps1_export::NativeExportPlan::compile_scene_resources(
            disc,entry,&inventory,&before,&relocation,limits)?;
        map=encoded.read_entry_footprint(entry)?;
        // Clone shared descriptors so another placement of the original TMD
        // does not inherit this member's mirror.
        let descriptor=ps1_export::unrestricted::detach(&mut map,cell)?;
        let raw:[u8;32]=map[descriptor*32..descriptor*32+32].try_into()?;
        let bound=if record.is_placed(){registration.bind_record(&tmd,&raw)?}
            else{registration.bind_static_decoration(&tmd,&raw)?};
        map[descriptor*32..descriptor*32+32].copy_from_slice(&bound);
        encoded.patch_prot_entry(entry,0,&map)?;
        ensure!(encoded.read_entry_footprint(entry)?==map,
            "Mirrored MAP selector physical readback mismatch");
        let bundle=encoded.read_entry_footprint(entry+3)?;
        let pac=reg::native_stream_mode(&bundle)
            .map(|_|encoded.read_entry_footprint(entry+4)).transpose()?;
        let pool=reg::native_model_pool(&bundle,pac.as_deref());
        let selector=registration.selector(&tmd)?;
        ensure!(pool.get(selector as usize).is_some(),"Mirrored native TMD selector missing after export");
        let new_record=ObjectRecord::parse(&map,descriptor).context("Mirrored native descriptor missing after binding")?;
        ensure!(new_record.pack_index_field==selector,
            "Mirrored native MAP selector does not match the registered TMD");
        *disc=encoded;
        log.push(format!("WRITTEN NATIVE MIRROR {target:?}: X/Z signs {:?}, independent model selector {selector}, MAP descriptor {descriptor}, grid {cell:?}; TMD and sector readback verified, DuckStation gameplay unverified",scale));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn synthetic_three_vertex_tmd()->Vec<u8>{
        let mut tmd=vec![0u8;12+28+3*8];
        tmd[0..4].copy_from_slice(&0x8000_0041u32.to_le_bytes());
        tmd[8..12].copy_from_slice(&1u32.to_le_bytes());
        tmd[12..16].copy_from_slice(&28u32.to_le_bytes());
        tmd[16..20].copy_from_slice(&3u32.to_le_bytes());
        tmd[28..32].copy_from_slice(&28u32.to_le_bytes());
        let points=[[-100i16,0,50],[100i16,0,50],[0i16,0,150]];
        for (i,point) in points.iter().enumerate(){
            for axis in 0..3 {
                let at=40+i*8+axis*2;
                tmd[at..at+2].copy_from_slice(&point[axis].to_le_bytes());
            }
        }
        tmd
    }
    #[test]fn native_reflection_changes_geometry_and_preserves_source(){
        let original=synthetic_three_vertex_tmd();
        let mirrored=transformed_tmd(&original,[-1.,1.,1.]).unwrap();
        let before=legaia_tmd::parse(&original).unwrap();
        let after=legaia_tmd::parse(&mirrored).unwrap();
        assert_eq!(before.objects[0].vertices[0].x,-100);
        assert_eq!(after.objects[0].vertices[0].x,100);
        assert_eq!(after.objects[0].vertices[1].x,-100);
        assert_eq!(after.objects[0].vertices[2].x,0);
        assert_eq!(after.objects[0].vertices[0].z,50);
        assert_eq!(original,synthetic_three_vertex_tmd());
        let back=transformed_tmd(&mirrored,[-1.,1.,1.]).unwrap();
        assert_eq!(back,original);
    }
    #[test]fn non_tmd_mirror_is_rejected(){
        assert!(transformed_tmd(&[],[-1.,1.,1.]).is_err());
        assert!(transformed_tmd(&[],[1.,1.,-1.]).is_err());
    }
}
