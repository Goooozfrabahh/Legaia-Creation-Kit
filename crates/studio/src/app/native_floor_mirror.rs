//! Native 4bpp floor-atlas reflection. Each reflected tile receives an
//! independent 32x32 texel allocation and the destination grid cell receives
//! its own MAP descriptor. No retail material or donor cell is modified.
//! Supports grid-aligned X/Z reflections; arbitrary UV rotation/scale and
//! layered/non-grid floors remain separate engine work.
use anyhow::{Context,Result,ensure};
use std::{collections::{BTreeMap,BTreeSet},convert::TryInto,fs,path::Path};
use legaia_disc_patch::disc::DiscPatcher;
use legaia_engine_scene::scene::{Scene,ProtIndex};
use legaia_scene_formats::field_objects::{self,ObjectRecord,CELL_BIND_OWNED,CELL_WALK_VISIBLE};
use legaia_tim::Vram;
use super::{ground_edit_ops::{self,State},native_asset_plan::{NativeAssetPlan,Origin,ResourceKind,
    scene_registration as reg,texture_relocation::{Rect,multi_tim::MultiPlan}},ps1_export};

const WIDTH:usize=1024;
const TILES:usize=32;
const WORDS:usize=8;
const MAP_GRID:usize=0x8000;

fn rect_for(page:u8,tile:u8)->Result<Rect>{
    ensure!(page<32 && tile<64,"Invalid 4bpp PS1 terrain page/tile selector");
    Ok(Rect{x:(page as u16&15)*64+(tile as u16%8)*8,
        y:((page as u16>>4)&1)*256+(tile as u16/8)*32,w:8,h:32})
}
fn bytes_for(vram:&Vram,page:u8,tile:u8,flip_x:bool,flip_z:bool)->Result<Vec<u8>>{
    let rect=rect_for(page,tile)?;
    for y in rect.y as usize..rect.y as usize+TILES {
        for x in rect.x as usize..rect.x as usize+WORDS {
            ensure!(vram.is_written(x,y),"Native ground texture references unuploaded VRAM ({x},{y})");
        }
    }
    let mut result=vec![0u8;WORDS*TILES*2];
    for y in 0..TILES {
        for x in 0..TILES {
            let read_x=if flip_x {TILES-1-x}else{x};
            let read_y=if flip_z {TILES-1-y}else{y};
            let input=vram.as_u16()[(rect.y as usize+read_y)*WIDTH+rect.x as usize+read_x/4];
            let pixel=((input>>(4*(read_x%4)))&15) as u16;
            let at=(y*WORDS+x/4)*2;
            let mut word=u16::from_le_bytes([result[at],result[at+1]]);
            word|=pixel<<(4*(x%4));
            result[at..at+2].copy_from_slice(&word.to_le_bytes());
        }
    }
    Ok(result)
}
// LCK_R46_GROUND_COMPOSITOR
// Bake 90, 180, and 270 degree native 4bpp terrain tile rotations.

fn w46_rotated_bytes(
    vram:&Vram,
    page:u8,
    tile:u8,
    fx:bool,
    fz:bool,
    turns:u8
)->Result<Vec<u8>>
{
    let rect=rect_for(page,tile)?;

    for y in rect.y as usize..rect.y as usize+TILES {
        for x in rect.x as usize..rect.x as usize+WORDS {
            ensure!(
                vram.is_written(x,y),
                "Floor source VRAM is not resident"
            );
        }
    }

    let mut result=vec![0u8;WORDS*TILES*2];

    for y in 0..TILES {
        for x in 0..TILES {
            let (rx,rz)=match turns%4 {
                0=>(x,y),
                1=>(y,TILES-1-x),
                2=>(TILES-1-x,TILES-1-y),
                _=>(TILES-1-y,x)
            };

            let xx=if fx {TILES-1-rx}else{rx};
            let yy=if fz {TILES-1-rz}else{rz};

            let input=vram.as_u16()[
                (rect.y as usize+yy)*WIDTH
                +rect.x as usize+xx/4
            ];

            let pixel=((input>>(4*(xx%4)))&15) as u16;

            let at=(y*WORDS+x/4)*2;

            let mut word=u16::from_le_bytes([
                result[at],
                result[at+1]
            ]);

            word|=pixel<<(4*(x%4));

            result[at..at+2].copy_from_slice(
                &word.to_le_bytes()
            );
        }
    }

    Ok(result)
}

#[cfg(test)]
mod lck_r46_texture_turn_tests {
    use super::*;

    #[test]
    fn rotating_an_asymmetric_tile_changes_pixels()->Result<()> {
        let mut v=Vram::new();
        let rect=rect_for(0x1a,0)?;

        let mut raw=vec![0u8;WORDS*TILES*2];

        let put=|bytes:&mut [u8],x:usize,y:usize,value:u16| {
            let at=(y*WORDS+x/4)*2;

            let mut w=u16::from_le_bytes([
                bytes[at],
                bytes[at+1]
            ]);

            w|=value<<(4*(x%4));

            bytes[at..at+2].copy_from_slice(
                &w.to_le_bytes()
            );
        };

        put(&mut raw,0,0,1);
        put(&mut raw,31,0,2);
        put(&mut raw,0,31,3);

        v.write_block(
            rect.x,rect.y,rect.w,rect.h,&raw
        );

        let rotated=w46_rotated_bytes(
            &v,0x1a,0,false,false,1
        )?;

        assert_ne!(rotated,raw);

        let mut another=Vram::new();

        another.write_block(
            rect.x,rect.y,rect.w,rect.h,&rotated
        );

        let restored=w46_rotated_bytes(
            &another,0x1a,0,false,false,3
        )?;

        assert_eq!(restored,raw);

        Ok(())
    }
}
fn free_tile(occupied:&Vram,allocated:&BTreeSet<(u8,u8)>)->Option<(u8,u8,Rect)>{
    let protected=super::native_asset_plan::texture_relocation::field_runtime_reservations();
    for page in 0u8..32 {
        for tile in 0u8..64 {
            if allocated.contains(&(page,tile)){continue;}
            let rect=rect_for(page,tile).ok()?;
            let overlap=protected.iter().any(|p|{
                rect.x<p.x+p.w && p.x<rect.x+rect.w &&
                rect.y<p.y+p.h && p.y<rect.y+rect.h
            });
            if overlap {continue;}
            if (rect.y as usize..rect.y as usize+TILES).all(|y|
                (rect.x as usize..rect.x as usize+WORDS).all(|x|!occupied.is_written(x,y))) {
                return Some((page,tile,rect));
            }
        }
    }
    None
}
fn upload_tim(rect:Rect,bytes:&[u8])->Result<Vec<u8>>{
    ensure!(bytes.len()==WORDS*TILES*2,"Mirrored native tile has invalid pixel size");
    let mut tim=Vec::with_capacity(20+bytes.len());
    tim.extend_from_slice(&16u32.to_le_bytes());
    tim.extend_from_slice(&0u32.to_le_bytes()); // 4bpp image-only, existing CLUT is reused
    tim.extend_from_slice(&(12u32+bytes.len() as u32).to_le_bytes());
    for word in [rect.x,rect.y,rect.w,rect.h]{tim.extend_from_slice(&word.to_le_bytes());}
    tim.extend_from_slice(bytes);
    let parsed=legaia_tim::parse_native_upload(&tim)?;
    ensure!(parsed.image.data.as_slice()==bytes,"Mirrored native TIM image payload differs");
    Ok(tim)
}
fn record_at(map:&[u8],cell:[u8;2])->Result<(usize,ObjectRecord)>{
    ensure!(map.len()>=0x12000,"Incomplete MAP ground-grid data");
    let at=MAP_GRID+(cell[1] as usize*128+cell[0] as usize)*2;
    let word=u16::from_le_bytes(map[at..at+2].try_into()?);
    ensure!(word&CELL_WALK_VISIBLE!=0,"Mirrored floor {:?} is not walkable in native MAP",cell);
    ensure!(word&CELL_BIND_OWNED==0,"Mirrored floor {:?} has a script-owned MAP grid reference",cell);
    let id=(word&0x01ff) as usize;
    let record=ObjectRecord::parse(map,id).context("Missing native floor owner descriptor")?;
    ensure!(!record.is_placed(),"Mirrored floor shares a placed actor descriptor");
    Ok((id,record))
}

/// Rewrite the STAGING image only. A registration or verification failure never
/// replaces the source retail image or publishes a partially mirrored IMG.
pub(super) fn overlay_existing(target:&Path,scene:&str,index:&ProtIndex,
    state:&State)->Result<Option<String>>{
    let targets=ground_edit_ops::w46_texture_targets(state).map_err(anyhow::Error::msg)?;
    if targets.is_empty(){return Ok(None);}
    ensure!(target.exists(),"Staged native floor IMG is missing");
    let native=Scene::load(index,scene)?;
    let entry=native.field_map_index(index).context("No native MAP for reflected floors")? as usize;
    let original=fs::read(target).context("Read staged floor IMG")?;
    let disc=DiscPatcher::open(original.clone())?;
    let mut map=disc.read_entry_footprint(entry)?;
    let bundle=disc.read_entry_footprint(entry+3)?;
    let pac=reg::native_stream_mode(&bundle)
        .map(|_|disc.read_entry_footprint(entry+4)).transpose()?;
    let limits=reg::Limits{max_models:u16::MAX as usize-
        field_objects::FIELD_ACTOR_PACK_BIAS as usize,
        max_decoded:usize::MAX,max_encoded:usize::MAX};
    let (_,scene_vram)=reg::native_texture_sequence(&bundle,pac.as_deref(),limits)?;
    let (_,mut occupancy)=reg::native_donor_texture_sequence(index,&bundle,pac.as_deref(),limits)?;
    let mut allocations=BTreeSet::new();
    let mut resources=NativeAssetPlan::default();
    let mut new_uploads=Vec::new();
    let mut reuse=BTreeMap::<(u8,u8,bool,bool,u8),(u8,u8)>::new();
    let mut rewrites=Vec::new();
    let mut owned=BTreeSet::new();

    for (cell,flip_x,flip_z,turns) in targets {
        ensure!(owned.insert(cell),"Multiple reflected floors claim the same MAP grid cell {:?}",cell);
        let (_,rec)=record_at(&map,cell)?;
        let (page,tile,clut)=if rec.terrain_tpage==0 {
            (field_objects::GROUND_ATLAS_TPAGE as u8,rec.terrain_tile,
                field_objects::GROUND_ATLAS_CLUT)
        } else {(rec.terrain_tpage as u8,rec.terrain_tile,rec.terrain_clut)};
        let key=(page,tile,flip_x,flip_z,turns);
        let (new_page,new_tile)=if let Some(&existing)=reuse.get(&key){existing}else{
            let pixels=w46_rotated_bytes(&occupancy,page,tile,flip_x,flip_z,turns)?;
            let (new_page,new_tile,rect)=free_tile(&occupancy,&allocations)
                .context("No protected-free native VRAM tile for mirrored floor (32x32 4bpp)")?;
            let tim=upload_tim(rect,&pixels)?;
            let id=resources.register(ResourceKind::Tim,&tim,
                Origin{scene:scene.into(),entry:None,offset:None},false)?;
            let parsed=legaia_tim::parse_native_upload(&tim)?;
            occupancy.upload_tim(&parsed);
            allocations.insert((new_page,new_tile));
            new_uploads.push((rect,id));
            reuse.insert(key,(new_page,new_tile));
            (new_page,new_tile)
        };
        rewrites.push((cell,new_page,new_tile,clut));
    }
    ensure!(!new_uploads.is_empty(),"No native ground reflection uploads produced");
    let plan=MultiPlan::native_floor_overlays(&resources,&scene_vram,&new_uploads)?;
    let (mut encoded,_)=ps1_export::NativeExportPlan::compile_scene_resources(
        &disc,entry,&resources,&scene_vram,&plan,limits)?;
    map=encoded.read_entry_footprint(entry)?;
    for &(cell,page,tile,clut) in &rewrites {
        let id=ps1_export::unrestricted::detach(&mut map,(cell[0],cell[1]))?;
        ensure!(id>=4,"Reflected floor requires independently owned MAP descriptor");
        let at=id*32;
        ensure!(at+32<=map.len(),"Reflected MAP descriptor outside buffer");
        map[at+0x14]=tile;
        map[at+0x15]=page;
        map[at+0x16..at+0x18].copy_from_slice(&clut.to_le_bytes());
        let parsed=ObjectRecord::parse(&map,id).context("Reflected floor record missing after edit")?;
        ensure!(parsed.terrain_tile==tile && parsed.terrain_tpage==page as u16 &&
            parsed.terrain_clut==clut,"Reflected native terrain texture record readback failed");
    }
    encoded.patch_prot_entry(entry,0,&map)?;
    ensure!(encoded.read_entry_footprint(entry)?==map,
        "Reflected floor MAP physical readback failed");
    let final_bundle=encoded.read_entry_footprint(entry+3)?;
    let final_pac=reg::native_stream_mode(&final_bundle)
        .map(|_|encoded.read_entry_footprint(entry+4)).transpose()?;
    let (_,new_vram)=reg::native_texture_sequence(&final_bundle,final_pac.as_deref(),limits)?;
    for &(rect,_) in &new_uploads {
        for y in rect.y as usize..rect.y as usize+TILES {
            for x in rect.x as usize..rect.x as usize+WORDS {
                ensure!(new_vram.is_written(x,y) &&
                    new_vram.as_u16()[y*WIDTH+x]==occupancy.as_u16()[y*WIDTH+x],
                    "Mirrored floor native TIM upload readback mismatch");
            }
        }
    }
    let result=encoded.into_image();
    ensure!(result.len()==original.len(),"Mirrored floor changed raw IMG size");
    // This is a private stage IMG; the mixed publisher verifies it again.
    fs::write(target,&result).context("Write reflected staging IMG")?;
    let reloaded=DiscPatcher::open(fs::read(target)?)?;
    ensure!(reloaded.read_entry_footprint(entry)?==map,
        "Reflected staging IMG MAP reread failed");
    Ok(Some(format!("NATIVE GROUND MIRROR: {} floor cells, {} independent 4bpp tile uploads and MAP descriptors; physical IMG reread passed; runtime not verified",
        rewrites.len(),new_uploads.len())))
}

#[cfg(test)]mod tests{
 use super::*;
 #[test]fn native_tile_addressing(){
    assert_eq!((rect_for(0x1a,0).unwrap().x,rect_for(0x1a,0).unwrap().y),(640,256));
    assert_eq!((rect_for(0x1a,63).unwrap().x,rect_for(0x1a,63).unwrap().y),(696,480));
 }
 #[test]fn four_bit_horizontal_vertical_reflection(){
    let mut v=Vram::new();let r=rect_for(0x1a,0).unwrap();
    let mut raw=vec![0;WORDS*TILES*2];
    for y in 0..32usize{for x in 0..32usize{
       let at=(y*WORDS+x/4)*2;
       let mut word=u16::from_le_bytes([raw[at],raw[at+1]]);
       word|=(((x+3*y)%16) as u16)<<(4*(x%4));
       raw[at..at+2].copy_from_slice(&word.to_le_bytes());
    }}
    v.write_block(r.x,r.y,r.w,r.h,&raw);
    let horizontal=bytes_for(&v,0x1a,0,true,false).unwrap();
    let vertical=bytes_for(&v,0x1a,0,false,true).unwrap();
    assert_ne!(horizontal,raw);assert_ne!(vertical,raw);
    assert_eq!(horizontal.len(),512);assert_eq!(vertical.len(),512);
    let mut m=Vram::new();m.write_block(r.x,r.y,r.w,r.h,&horizontal);
    assert_eq!(bytes_for(&m,0x1a,0,true,false).unwrap(),raw);
    let mut n=Vram::new();n.write_block(r.x,r.y,r.w,r.h,&vertical);
    assert_eq!(bytes_for(&n,0x1a,0,false,true).unwrap(),raw);
 }
}
