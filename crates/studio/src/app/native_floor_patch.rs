//! Data-only native floor-record transforms. This operates on explicitly
//! selected kind-2 floor components, NOT guessed mesh-owned surfaces. Its
//! output is not an IMG export authorization or an engine modification.
use std::collections::BTreeSet;
use anyhow::{Result,Context,ensure};
use legaia_engine_field::field_elevation;
use legaia_scene_formats::field_map;
use super::{native_elevation_codec as codec,collision_recipes::digest};

#[derive(Clone,Debug)]
pub(super) struct Edit {
    pub expected:[u8;4],
    /// None removes the selected override; Some is a whole native tile.
    pub destination:Option<[u8;2]>,
    pub duplicate:bool,
    /// Positive native Y rotation: one turn is PSX angle 0x400.
    pub quarter_turns:u8,
    /// PSX Y-down units, independent of editor Y sign.
    pub height_delta:i32,
}
impl Edit {
    pub fn translated(expected:[u8;4],delta:[i32;2],duplicate:bool)->Result<Self> {
        ensure!(delta.iter().all(|v|v%128==0),"Native height records are keyed to 128-unit tiles; half-tile moves require surface repartition and representability proof, not rounded coordinates");
        let x=i64::from(expected[0])+i64::from(delta[0])/128;let z=i64::from(expected[1])+i64::from(delta[1])/128;
        ensure!((0..128).contains(&x)&&(0..128).contains(&z),"Native floor translation outside grid");
        Ok(Self{expected,destination:Some([x as u8,z as u8]),duplicate,quarter_turns:0,height_delta:0})
    }
}
pub(super) struct Plan {source_sha256:String,output:Vec<u8>}
impl Plan {
    pub fn apply(&self,load:&[u8])->Result<Vec<u8>> {
        ensure!(digest(load)==self.source_sha256,"Native floor plan source changed");Ok(self.output.clone())
    }
}
fn word(load:&[u8],cell:[u8;2])->u16 {
    let at=0x8000+(cell[1] as usize*128+cell[0] as usize)*2;
    u16::from_le_bytes(load[at..at+2].try_into().unwrap())
}
fn flag(load:&mut [u8],cell:[u8;2],enabled:bool) {
    let at=0x8000+(cell[1] as usize*128+cell[0] as usize)*2;
    let old=word(load,cell);let next=if enabled {old|0x800}else{old&!0x800};
    load[at..at+2].copy_from_slice(&next.to_le_bytes());
}
fn mean(load:&[u8],lut:&[i32;16],cell:[u8;2])->i32 {
    let at=0x4000+cell[1] as usize*128+cell[0] as usize;
    [0,1,128,129].into_iter().map(|d|lut[(load[at+d]&15) as usize]).sum::<i32>()>>2
}
fn tiers(load:&[u8],cell:[u8;2])->[u8;4] {
    let at=0x4000+cell[1] as usize*128+cell[0] as usize;
    let mut indices=[0,1,128,129].map(|d|load[at+d]&15);indices.sort();indices
}
/// Native LUT is the runtime signed ladder. Whole-tile transforms preserve
/// shared corner tiers; the new row compensates for the destination mean.
pub(super) fn compile(load:&[u8],lut:&[i32;16],edits:&[Edit])->Result<Plan> {
    ensure!(load.len()==0x14000 && !edits.is_empty(),"Native floor edit needs the exact MAP/fallback window");
    ensure!(lut.iter().all(|&v|(-32768..=32767).contains(&v)),"Runtime floor ladder is outside native signed range");
    super::ps1_export::native_dependencies::NativeBindings::parse(&load[0x10000..0x12000],&load[0x12000..])?;
    let map=field_map::detect(&load[..0x12000]).context("Native MAP trigger layout not established")?;
    let header=map.trigger_block().context("No primary native trigger table")?;
    let block=&load[0x10000..0x12000];
    let old_rows=field_elevation::parse_elevation_overrides(block);
    let fallback=field_elevation::parse_elevation_overrides(&load[0x12000..]);
    let mut removed=BTreeSet::new();let mut destinations=BTreeSet::new();let mut appended=Vec::new();
    let mut output=load.to_vec();
    let mut allowed_cells=BTreeSet::new();
    for edit in edits {
        let source=[edit.expected[0],edit.expected[1]];
        ensure!(source.iter().all(|&v|v<128),"Source floor reference outside native grid");
        ensure!(old_rows.iter().filter(|r|[r.tile_x,r.tile_z]==source).count()==1,
            "Source primary elevation row is absent/shared by duplicate keys; fallback/first-match editing requires separate closure");
        let index=old_rows.iter().position(|r|[r.tile_x,r.tile_z]==source).unwrap();
        ensure!(codec::serialize(&old_rows[index])?==edit.expected,"Native elevation source changed");
        ensure!(word(load,source)&0x800!=0,"Source native elevation row is not enabled");
        ensure!(!fallback.iter().any(|r|[r.tile_x,r.tile_z]==source),"Removing/shadowing this floor row would expose a fallback contribution");
        ensure!(edit.destination.is_some() || (!edit.duplicate && edit.height_delta==0 && edit.quarter_turns==0),"Invalid native floor deletion");
        if !edit.duplicate {ensure!(removed.insert(index),"Conflicting edits to one native floor component");flag(&mut output,source,false);allowed_cells.insert(source);}
        if let Some(target)=edit.destination {
            ensure!(target.iter().all(|&v|v<128)&&destinations.insert(target),"Conflicting/out-of-grid native floor destinations");
            if target!=source || edit.duplicate {
                ensure!(word(load,target)&0x800==0 && !old_rows.iter().chain(&fallback).any(|r|[r.tile_x,r.tile_z]==target),
                    "Destination already has a native elevation contribution; no unrelated surface was overwritten");
            }
            ensure!(!edit.duplicate || target!=source,"Floor duplicate requires an independent native cell");
            ensure!(tiers(load,source)==tiers(load,target),
                "Source/destination floor-tier weights differ; live floor-ladder/script closure is required. No static height snapshot was baked into the native row");
            let samples=codec::decode(&old_rows[index],mean(load,lut,source))?;
            let mut heights=samples.map(|s|s.height);
            for _ in 0..(edit.quarter_turns%4) {heights=[heights[1],heights[3],heights[0],heights[2]];}
            let target_mean=mean(load,lut,target);let mut offsets=[0;4];
            for i in 0..4 {offsets[i]=heights[i].checked_add(edit.height_delta).and_then(|h|h.checked_sub(target_mean)).context("Native elevation transform overflow")?;}
            let row=codec::encode(target,offsets)?;appended.push(codec::serialize(&row)?);flag(&mut output,target,true);
            allowed_cells.insert(target);
        }
    }
    let rows=old_rows.iter().enumerate().filter(|(i,_)|!removed.contains(i)).map(|(_,r)|codec::serialize(r)).collect::<Result<Vec<_>>>()?;
    let mut rows=rows;rows.extend(appended);
    let mut cursor=18usize;let mut replacement=vec![0u8;18];
    for kind in 0..4 {
        let table=&header.tables[kind];
        let body=if kind==2 {rows.iter().flatten().copied().collect::<Vec<_>>()}else{block[table.body_range()].to_vec()};
        let count=body.len()/table.stride;
        ensure!(count<=i16::MAX as usize,"Native floor record count exceeds signed lookup range");
        replacement[kind*4+2..kind*4+4].copy_from_slice(&(cursor as u16).to_le_bytes());
        replacement[kind*4+4..kind*4+6].copy_from_slice(&(count as u16).to_le_bytes());
        replacement.extend_from_slice(&body);
        let end=table.offset+table.count*table.stride;
        replacement.extend_from_slice(&block[end..end+2]);cursor=replacement.len();
    }
    ensure!(cursor<=0x2000,"Native primary trigger block is full; more floor records require a new supported allocation path");
    if cursor>header.end_offset {
        ensure!(block[header.end_offset..cursor].iter().all(|&b|b==0),"New floor rows would overwrite non-padding native bytes");
    }
    replacement[..2].copy_from_slice(&(cursor as u16).to_le_bytes());
    let changed_end=cursor.max(header.end_offset);
    output[0x10000..0x10000+changed_end].fill(0);
    output[0x10000..0x10000+cursor].copy_from_slice(&replacement);
    let read=field_map::detect(&output[..0x12000]).context("Serialized floor MAP failed native reparse")?;
    for kind in [0,1,3] {ensure!(map.trigger_records(kind)==read.trigger_records(kind),"Unrelated trigger records changed");}
    let read_rows=field_elevation::parse_elevation_overrides(&output[0x10000..0x12000]).iter().map(codec::serialize).collect::<Result<Vec<_>>>()?;
    ensure!(read_rows==rows,"Serialized floor rows differ from intent");
    ensure!(output[..0x8000]==load[..0x8000] && output[0x12000..]==load[0x12000..],"Floor plan altered descriptors, wall/tier bytes or physical fallback");
    // Retail reads flat +0/+1/+128/+129 corner bytes even at the grid edge.
    // Some of those alias the first object-grid bytes. A changed 0x800 flag
    // must not silently change an unrelated edge cell's sampled floor tier.
    ensure!(output[0x8000..=0x8080].iter().zip(&load[0x8000..=0x8080]).all(|(a,b)|a&15==b&15),
        "Native elevation flag aliases a retained edge-floor sample; edge dependency closure is required");
    for i in (0x8000..0x10000).step_by(2) {
        let a=u16::from_le_bytes(load[i..i+2].try_into()?);let b=u16::from_le_bytes(output[i..i+2].try_into()?);
        ensure!((a^b)&!0x800==0,"Floor plan changed an unrelated grid field");
        let cell=[(((i-0x8000)/2)%128) as u8,(((i-0x8000)/2)/128) as u8];
        ensure!(a==b || allowed_cells.contains(&cell),"Floor plan changed an unrelated native cell");
    }
    Ok(Plan{source_sha256:digest(load),output})
}

#[cfg(test)]
pub(super) fn fixture()->Vec<u8> {
    let mut load=vec![0;0x14000];
    // Canonical native chained header with a retained teleport and region.
    let bodies=[vec![1,2,3,4],vec![5,6,7,0],vec![37,18,2,0xcc],vec![1,2,3,4,5,6,7,8]];
    let mut cursor=18;
    for kind in 0..4 {
        let stride=if kind==3 {8}else{4};let at=0x10000+kind*4;
        load[at+2..at+4].copy_from_slice(&(cursor as u16).to_le_bytes());
        load[at+4..at+6].copy_from_slice(&((bodies[kind].len()/stride) as u16).to_le_bytes());
        load[0x10000+cursor..0x10000+cursor+bodies[kind].len()].copy_from_slice(&bodies[kind]);cursor+=bodies[kind].len()+2;
    }
    load[0x10000..0x10002].copy_from_slice(&(cursor as u16).to_le_bytes());flag(&mut load,[37,18],true);load
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn move_duplicate_rotate_and_delete_have_real_native_rows_and_independent_cells()->Result<()> {
        for (duplicate,turns,destination) in [(false,0,Some([35,18])),(true,0,Some([35,18])),(false,1,Some([37,18])),(false,0,None)] {
            let load=fixture();let edit=Edit{expected:[37,18,2,0xcc],destination,duplicate,quarter_turns:turns,height_delta:0};
            let plan=compile(&load,&[0;16],&[edit])?;let output=plan.apply(&load)?;
            let rows=field_elevation::parse_elevation_overrides(&output[0x10000..0x12000]);
            assert_eq!(rows.len(),if duplicate {2}else if destination.is_none() {0}else{1});
            if let Some(cell)=destination {let row=rows.iter().find(|r|[r.tile_x,r.tile_z]==cell).unwrap();
                let expected=if turns==1 {[-112,-112,-64,-64]}else{[-64,-112,-64,-112]};
                assert_eq!(codec::decode(row,0)?.map(|s|s.height),expected);}
            assert_eq!(output[0x4000..0x8000],load[0x4000..0x8000]);
            assert!(plan.apply(&{let mut stale=load.clone();stale[0]^=1;stale}).is_err());
        }Ok(())
    }
    #[test]
    fn floor_plans_refuse_colliding_keys_stale_sources_and_unknown_padding() {
        let load=fixture();let mut edit=Edit{expected:[37,18,2,0xcc],destination:Some([35,18]),duplicate:true,quarter_turns:0,height_delta:0};
        let mut occupied=load.clone();flag(&mut occupied,[35,18],true);assert!(compile(&occupied,&[0;16],&[edit.clone()]).is_err());
        let mut different_tiers=load.clone();different_tiers[0x4000+18*128+35]=1;
        assert!(compile(&different_tiers,&[0;16],&[edit.clone()]).is_err());
        let mut padding=load.clone();let header=field_map::detect(&padding[..0x12000]).unwrap().trigger_block().unwrap();padding[0x10000+header.end_offset]=7;
        assert!(compile(&padding,&[0;16],&[edit.clone()]).is_err());
        edit.expected[2]=3;assert!(compile(&load,&[0;16],&[edit]).is_err());
        assert!(Edit::translated([37,18,2,0xcc],[-320,64],false).is_err());
        assert_eq!(Edit::translated([37,18,2,0xcc],[-256,0],false).unwrap().destination,Some([35,18]));
    }
    #[test]
    fn floor_rows_survive_raw_img_sector_rewrite_and_native_reparse()->Result<()> {
        use legaia_disc_patch::disc::DiscPatcher;
        let load=fixture();let edit=Edit::translated([37,18,2,0xcc],[-256,0],true)?;
        let output=compile(&load,&[0;16],&[edit])?.apply(&load)?;
        let original=super::super::ps1_export::tests::map_disc(&load[..0x12000]);
        let before=DiscPatcher::open(original.clone())?;let mut disc=DiscPatcher::open(original.clone())?;
        disc.patch_prot_entry(0,0,&output[..0x12000])?;
        let changed=disc.image().to_vec();let read=DiscPatcher::open(changed.clone())?;
        assert_eq!(read.read_entry_footprint(0)?,output[..0x12000]);
        assert_eq!(read.read_entry_footprint(1)?,before.read_entry_footprint(1)?);
        let folder=std::env::current_dir()?.join("target").join(format!("native-floor-synthetic-{}-{}",std::process::id(),std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_nanos()));
        std::fs::create_dir(&folder)?;
        std::fs::write(folder.join("synthetic-source.img"),&original)?;std::fs::write(folder.join("synthetic-floor-copy.img"),&changed)?;
        let reread=DiscPatcher::open(std::fs::read(folder.join("synthetic-floor-copy.img"))?)?;
        let map=reread.read_entry_footprint(0)?;
        let rows=field_elevation::parse_elevation_overrides(&map[0x10000..0x12000]);
        assert_eq!(rows.len(),2);assert_eq!((rows[1].tile_x,rows[1].tile_z),(35,18));Ok(())
    }
    #[test]
    fn equal_tier_weights_preserve_floor_animation_without_baking_a_snapshot()->Result<()> {
        let mut load=fixture();
        for (cell,values) in [([37u8,18u8],[0,1,2,3]),([35,18],[3,1,0,2])] {
            let at=0x4000+cell[1] as usize*128+cell[0] as usize;
            for (d,tier) in [0,1,128,129].into_iter().zip(values) {load[at+d]=tier;}
        }
        let mut lut=[0;16];lut[..4].copy_from_slice(&[0,32,-64,128]);
        let plan=compile(&load,&lut,&[Edit::translated([37,18,2,0xcc],[-256,0],false)?])?;
        let output=plan.apply(&load)?;let row=field_elevation::parse_elevation_overrides(&output[0x10000..0x12000])[0];
        let original=field_elevation::ElevationOverride{tile_x:37,tile_z:18,coarse:2,quads:0xcc};
        for ladder in [[0,32,-64,128],[128,-32,256,-64]] {
            lut[..4].copy_from_slice(&ladder);
            assert_eq!(codec::decode(&original,mean(&load,&lut,[37,18]))?.map(|s|s.height),codec::decode(&row,mean(&output,&lut,[35,18]))?.map(|s|s.height));
        }Ok(())
    }
}
