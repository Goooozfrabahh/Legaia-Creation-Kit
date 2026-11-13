// LCK_R615_NATIVE128_RUNTIME_WINDOW_AND_UNIVERSAL_AUDIT
// Experimental retail region-latch authoring plus native MAP evidence.
// Never treats a serialized placement as proof of gameplay behavior.
use anyhow::{Context,Result,ensure};
use std::{collections::BTreeSet,fs,path::Path};
use legaia_disc_patch::disc::DiscPatcher;
use legaia_engine_field::field_regions::{RegionTable,RegionAttributes,refresh_region_attributes,window_rebuild_spawns};
use legaia_scene_formats::field_objects::{ObjectRecord,CELL_WALK_VISIBLE,CELL_VISIBLE};
use super::export_manifest::Manifest;

const BLOCK:usize=0x10000;
const BODY:usize=0x484;
const STRIDE:usize=8;
const RETAIL_COUNT:usize=14;
const MAIN:usize=5;
const MAIN_BYTES:[u8;5]=[9,60,45,0,0];
const FULL:[u8;8]=[0,128,128,0,0,0,0,0];
const TABLE_END:usize=BLOCK+0x800;
const GRID:usize=0x8000;
const WALL:usize=0x4000;
const CELLS:usize=128*128;

fn header(map:&[u8])->Result<(usize,usize)> {
    ensure!(map.len()>=0x12000,"R615: MAP entry shorter than 0x12000");
    let body=i16::from_le_bytes([map[BLOCK+14],map[BLOCK+15]]);
    let count=i16::from_le_bytes([map[BLOCK+16],map[BLOCK+17]]);
    ensure!(body>=0&&count>=0,"R615: negative region body/count");
    let (body,count)=(body as usize,count as usize);
    ensure!(BLOCK+body+count*STRIDE<=map.len(),"R615: region records outside MAP");
    Ok((body,count))
}

fn attributes(map:&[u8],x:i32,z:i32)->Result<(u32,RegionAttributes)> {
    let table=RegionTable::parse(&map[BLOCK..]).context("R615: invalid native region table")?;
    Ok(refresh_region_attributes(Some(&table),x,z,false))
}

// Native raw region records store Z as [z_max,z_min]; the latched scratchpad
// window becomes [x_min,z_min,x_max,z_max]. Swapping these here is crucial.
// Keep every retail record intact. Add one wide type-0 exterior window, then
// re-append the original non-exterior type-0/1 windows so original interiors
// and story/camera windows continue to take precedence on overlapping tiles.
// Additional type-0 mask bits can appear even inside original type-1 windows;
// this is an opt-in gameplay experiment, not a verified safe camera/script patch.
fn plan_wide(retail:&[u8],stage:&[u8])->Result<(Vec<u8>,usize)> {
    let (body,base_count)=header(retail)?;
    let (stage_body,stage_count)=header(stage)?;
    ensure!(body==BODY&&base_count==RETAIL_COUNT&&stage_body==BODY,
        "R615: only fingerprinted Rim Elm town01 region schema is supported");
    ensure!(stage_count>=RETAIL_COUNT,
        "R615: staged IMG has fewer regions than retail Rim Elm");
    let main=BLOCK+body+MAIN*STRIDE;
    ensure!(&retail[main..main+5]==&MAIN_BYTES[..],
        "R615: Rim Elm exterior region signature changed");
    ensure!(retail[BLOCK+body..BLOCK+body+RETAIL_COUNT*STRIDE]==
        stage[BLOCK+body..BLOCK+body+RETAIL_COUNT*STRIDE],
        "R615: original region records were modified");
    let mut extra=FULL.to_vec();
    let mut restored=0usize;
    for i in 0..RETAIL_COUNT {
        if i==MAIN {continue;}
        let at=BLOCK+body+i*STRIDE;
        let rec=&retail[at..at+STRIDE];
        if rec[4]<=1 {
            extra.extend_from_slice(rec);
            restored+=1;
        }
    }
    let final_count=stage_count+extra.len()/STRIDE;
    ensure!(final_count<256,"R615: region count beyond byte-sized runtime budget");
    let at=BLOCK+body+stage_count*STRIDE;
    let end=at+extra.len();
    ensure!(end<=TABLE_END,"R615: insufficient native region-table padding");
    ensure!(stage[at..end].iter().all(|&b|b==0),
        "R615: native region padding occupied; no unsafe overwrite");
    let mut out=stage.to_vec();
    out[at..end].copy_from_slice(&extra);
    out[BLOCK+16..BLOCK+18].copy_from_slice(&(final_count as i16).to_le_bytes());
    let exterior=RegionAttributes{box_bytes:[9,0,45,60],kind:0};
    let wide=RegionAttributes{box_bytes:[0,0,128,128],kind:0};
    for z in 0..128i32 {
        for x in 0..128i32 {
            let (old_mask,before)=attributes(retail,x,z)?;
            let (new_mask,after)=attributes(&out,x,z)?;
            // A tile with a specific retail interior/camera window must not
            // lose that last-latched window; its region mask is also preserved.
            if before!=exterior && before!=RegionAttributes::DEFAULT_FILL {
                ensure!(after==before && (new_mask&old_mask)==old_mask,
                    "R615: original interior/camera latch or existing region flags changed at ({x},{z})");
            } else {
                ensure!(after==wide && new_mask&1!=0,
                    "R615: wide exterior did not latch at ({x},{z})");
            }
        }
    }
    Ok((out,restored))
}

pub(super) fn patch_wide(path:&Path,entry:usize,retail:&[u8])->Result<String> {
    let original=fs::read(path).with_context(||format!("R615: read staged IMG {}",path.display()))?;
    let mut disc=DiscPatcher::open(original)?;
    let stage=disc.read_entry_footprint(entry)?;
    let (out,restored)=plan_wide(retail,&stage)?;
    let (body,old_count)=header(&stage)?;
    let (_,new_count)=header(&out)?;
    let first=BLOCK+body+old_count*STRIDE;
    let end=BLOCK+body+new_count*STRIDE;
    disc.patch_prot_entry(entry,first as u64,&out[first..end])?;
    disc.patch_prot_entry(entry,(BLOCK+16) as u64,&out[BLOCK+16..BLOCK+18])?;
    ensure!(disc.read_entry_footprint(entry)?==out,
        "R615: native MAP write/readback differs");
    fs::write(path,disc.into_image()).context("R615: save private staged IMG")?;
    let checked=DiscPatcher::open(fs::read(path)?)?.read_entry_footprint(entry)?;
    ensure!(checked==out,"R615: physical IMG readback failed");
    Ok(format!("R615 EXPERIMENTAL WIDE WINDOW: region records {} -> {}, one 128x128 type-0 exterior window, {} original non-exterior type-0/1 windows re-appended and checked across all tiles; no PS1 runtime code patched, gameplay unverified",old_count,new_count,restored))
}

// LCK V2.0.8.5 UNIFIED REGION PREVIEW
// A final type-0 region covers all native 128x128 MAP tiles.
// Does not install any MIPS runtime hooks.

fn plan_unified(retail:&[u8],stage:&[u8])->Result<Vec<u8>> {
    let (old_body,old_count)=header(retail)?;
    let (body,count)=header(stage)?;

    ensure!(
        old_count==RETAIL_COUNT && count>=old_count,
        "v2.0.8.5: unexpected retail region count"
    );

    ensure!(
        old_body>=0x20 && body>=0x20 && count<255,
        "v2.0.8.5: invalid region layout"
    );

    let old_start=BLOCK+old_body;
    let start=BLOCK+body;
    let old_end=old_start+old_count*STRIDE;
    let protected_end=start+old_count*STRIDE;
    let append_at=start+count*STRIDE;
    let append_end=append_at+STRIDE;

    ensure!(
        old_end<=TABLE_END && append_end<=TABLE_END,
        "v2.0.8.5: region table capacity exceeded"
    );

    ensure!(
        &retail[old_start+MAIN*STRIDE..old_start+MAIN*STRIDE+5]
            ==&MAIN_BYTES[..],
        "v2.0.8.5: unexpected Rim Elm exterior signature"
    );

    ensure!(
        retail[old_start..old_end]==stage[start..protected_end],
        "v2.0.8.5: original region records changed"
    );

    ensure!(
        stage[append_at..append_end].iter().all(|&b|b==0),
        "v2.0.8.5: target region slot occupied"
    );

    let mut out=stage.to_vec();

    out[append_at..append_end].copy_from_slice(&FULL);

    out[BLOCK+16..BLOCK+18]
        .copy_from_slice(&((count+1) as i16).to_le_bytes());

    // Corrected: RegionTable::parse returns Option, not Result.
    let before=RegionTable::parse(&retail[BLOCK..])
        .context("v2.0.8.5: invalid retail region table")?;

    let after=RegionTable::parse(&out[BLOCK..])
        .context("v2.0.8.5: invalid unified region table")?;

    for z in 0..128i32 {
        for x in 0..128i32 {
            let (old_bits,_)=refresh_region_attributes(
                Some(&before),x,z,false
            );

            let (bits,attrs)=refresh_region_attributes(
                Some(&after),x,z,false
            );

            ensure!(
                attrs.box_bytes==[0,0,128,128] && attrs.kind==0,
                "v2.0.8.5: unified region did not latch at ({x},{z})"
            );

            ensure!(
                bits&old_bits==old_bits,
                "v2.0.8.5: original region bits lost at ({x},{z})"
            );
        }
    }

    Ok(out)
}

pub(super) fn patch_unified(
    path:&Path,
    entry:usize,
    retail:&[u8]
)->Result<String> {
    let source=fs::read(path)
        .context("v2.0.8.5: cannot read staged IMG")?;

    let mut disc=DiscPatcher::open(source)?;
    let stage=disc.read_entry_footprint(entry)?;
    let out=plan_unified(retail,&stage)?;

    let (body,count)=header(&stage)?;
    let at=BLOCK+body+count*STRIDE;

    disc.patch_prot_entry(
        entry,
        at as u64,
        &out[at..at+STRIDE]
    )?;

    disc.patch_prot_entry(
        entry,
        (BLOCK+16) as u64,
        &out[BLOCK+16..BLOCK+18]
    )?;

    ensure!(
        disc.read_entry_footprint(entry)?==out,
        "v2.0.8.5: staged MAP readback mismatch"
    );

    fs::write(path,disc.into_image())
        .context("v2.0.8.5: cannot save staged IMG")?;

    let written=DiscPatcher::open(fs::read(path)?)?
        .read_entry_footprint(entry)?;

    ensure!(
        written==out,
        "v2.0.8.5: physical IMG readback mismatch"
    );

    let spawns=window_rebuild_spawns(
        &out,
        (0,0,128,128),
        [0i16;16]
    ).len();

    Ok(format!(
        "v2.0.8.5 EXPERIMENTAL UNIFIED 128: 16384 native tiles have full region coverage; original region records preserved; {} native object spawn candidates; runtime streaming/camera/edge rows and gameplay NOT verified",
        spawns
    ))
}
fn word(map:&[u8],i:usize)->u16 {
    let at=GRID+i*2;
    u16::from_le_bytes([map[at],map[at+1]])
}

pub(super) fn audit(retail:&[u8],stage:&[u8],manifest:&Manifest,wide_mode:bool)->Result<String> {
    ensure!(retail.len()>=0x12000&&stage.len()>=0x12000,
        "R615: native MAP input truncated");
    let (_,retail_regions)=header(retail)?;
    let (_,staged_regions)=header(stage)?;
    let mut covered=0usize;
    let mut original_covered=0usize;
    let mut changed_outside=0usize;
    let mut walk_now_outside=0usize;
    let mut walk_added_outside=0usize;
    let mut visible_now_outside=0usize;
    let mut visible_added_outside=0usize;
    let mut native_placements_outside=0usize;
    let mut changed_place_links_outside=0usize;
    let mut wall_changed_outside=0usize;
    let mut height_changed_outside=0usize;
    let mut originally_uncovered_now_covered=0usize;
    let mut referenced=BTreeSet::<usize>::new();
    let mut modified_desc=BTreeSet::<usize>::new();
    let mut errors=0usize;
    for id in 0..512usize {
        let at=id*32;
        if retail[at..at+32]!=stage[at..at+32] {modified_desc.insert(id);}
    }
    for i in 0..CELLS {
        let x=(i%128) as i32;
        let z=(i/128) as i32;
        let (retail_mask,_)=attributes(retail,x,z)?;
        let (staged_mask,_)=attributes(stage,x,z)?;
        let old_region=retail_mask&3!=0;
        let new_region=staged_mask&3!=0;
        if old_region {original_covered+=1;}
        if new_region {covered+=1;}
        if !old_region&&new_region {originally_uncovered_now_covered+=1;}
        let a=word(retail,i);
        let b=word(stage,i);
        let id=(b&0x1ff) as usize;
        referenced.insert(id);
        let old_wall=retail[WALL+i];
        let new_wall=stage[WALL+i];
        if !old_region {
            if a!=b || old_wall!=new_wall {changed_outside+=1;}
            if b&CELL_WALK_VISIBLE!=0 {walk_now_outside+=1;}
            if a&CELL_WALK_VISIBLE==0 && b&CELL_WALK_VISIBLE!=0 {walk_added_outside+=1;}
            if b&CELL_VISIBLE!=0 {visible_now_outside+=1;}
            if a&CELL_VISIBLE==0 && b&CELL_VISIBLE!=0 {visible_added_outside+=1;}
            if (old_wall^new_wall)&0xf0!=0 {wall_changed_outside+=1;}
            if (old_wall^new_wall)&0x0f!=0 {height_changed_outside+=1;}
            if let Some(r)=ObjectRecord::parse(stage,id) {
                if r.is_placed() {
                    native_placements_outside+=1;
                    if a!=b || retail[id*32..id*32+32]!=stage[id*32..id*32+32] {
                        changed_place_links_outside+=1;
                    }
                }
            }else{errors+=1;}
        }
    }
    ensure!(errors==0,"R615: staged MAP has invalid native descriptor references");
    let sample=[30i32,30i32];
    let (_,retail_attrs)=attributes(retail,sample[0],sample[1])?;
    let (_,stage_attrs)=attributes(stage,sample[0],sample[1])?;
    let rect=stage_attrs.box_bytes;
    let resident_spawns=window_rebuild_spawns(
        stage,(rect[0],rect[1],rect[2],rect[3]),[0i16;16]
    );
    let actions=manifest.objects.iter().fold(std::collections::BTreeMap::<String,usize>::new(),|mut v,obj|{
        *v.entry(obj.action.clone()).or_insert(0)+=1;
        v
    });
    let actions=actions.iter().map(|(k,n)|format!("{k}={n}")).collect::<Vec<_>>().join(", ");
    Ok(format!(concat!(
        "LCK R6.1.5 NATIVE 128 EXPORT AND WINDOW AUDIT\n",
        "Scene: {}\n",
        "Wide exterior window requested: {}\n",
        "Original region records: {}\nFinal region records: {}\n",
        "Original type-0/1 region-covered tiles: {}\n",
        "Final type-0/1 region-covered tiles: {} / 16384\n",
        "Newly region-covered tiles: {}\n",
        "Edited grid/wall tiles outside original windows: {}\n",
        "Walk-flagged tiles outside original windows (CORRECT 0x1000): {}\n",
        "New walk flags outside original windows (0x1000): {}\n",
        "Mesh-draw-flagged tiles outside original windows (0x2000): {}\n",
        "New mesh draw flags outside original windows: {}\n",
        "Native placed-object grid links outside original windows: {}\n",
        "Changed/new placed-object links outside original windows: {}\n",
        "Wall-high-nibble changes outside original windows: {}\n",
        "Floor-height-low-nibble changes outside original windows: {}\n",
        "Referenced 9-bit descriptor IDs: {} / 512\n",
        "Changed descriptor records: {}\n",
        "Sample scene-window tile: ({},{})\n",
        "Retail latched rectangle (x0,z0,x1,z1): {:?}\n",
        "Final latched rectangle (x0,z0,x1,z1): {:?}\n",
        "Resident static-object spawn candidates in final sample window: {}\n",
        "Requested editor objects (manifest): {}\n",
        "Requested editor object actions: {}\n",
        "Requested floor edits: {}\nRequested native wall cells: {}\n",
        "Unencoded vertical collision previews: {}\n",
        "NOTE: The original R6.1.3 walk-flag audit may test 0x0800 (elevation override), not 0x1000. Use these numbers.\n",
        "NOTE: MAP flags/geometry, region-latched windows, and spawn candidates are serialization evidence, NOT DuckStation renderer/physics verification.\n",
        "NOTE: NPC MAN actors, cross-cell resource uploads, scripts, warps and camera data require independent runtime verification.\n"
    ),manifest.scene,wide_mode,retail_regions,staged_regions,
    original_covered,covered,originally_uncovered_now_covered,
    changed_outside,walk_now_outside,walk_added_outside,
    visible_now_outside,visible_added_outside,
    native_placements_outside,changed_place_links_outside,
    wall_changed_outside,height_changed_outside,
    referenced.len(),modified_desc.len(),sample[0],sample[1],
    retail_attrs.box_bytes,stage_attrs.box_bytes,resident_spawns.len(),
    manifest.objects.len(),actions,manifest.ground.len(),manifest.walls.len(),
    manifest.vertical_previews.len()))
}

#[cfg(test)]mod tests {
    use super::*;
    fn fixture()->Vec<u8> {
        let mut m=vec![0u8;0x12000];
        m[BLOCK+14..BLOCK+16].copy_from_slice(&(BODY as i16).to_le_bytes());
        m[BLOCK+16..BLOCK+18].copy_from_slice(&(RETAIL_COUNT as i16).to_le_bytes());
        let at=BLOCK+BODY+MAIN*STRIDE;
        m[at..at+8].copy_from_slice(&[9,60,45,0,0,0,0,0]);
        m
    }
    #[test]fn wide_window_preserves_specific_interior_latches()->Result<()> {
        let mut base=fixture();
        let at=BLOCK+BODY+7*STRIDE;
        base[at..at+8].copy_from_slice(&[60,35,72,45,1,0,0,0]);
        let mut stage=base.clone();
        stage[BLOCK+BODY+RETAIL_COUNT*STRIDE..BLOCK+BODY+(RETAIL_COUNT+1)*STRIDE]
            .copy_from_slice(&[64,64,96,32,0,0,0,0]);
        stage[BLOCK+16..BLOCK+18].copy_from_slice(&15i16.to_le_bytes());
        let (out,restored)=plan_wide(&base,&stage)?;
        assert!(restored>=1);
        assert_eq!(attributes(&base,65,39)?.1,attributes(&out,65,39)?.1);
        assert_eq!(attributes(&out,10,10)?.1.box_bytes,[0,0,128,128]);
        assert_eq!(attributes(&out,100,100)?.1.box_bytes,[0,0,128,128]);
        assert_eq!(&out[BLOCK+BODY..BLOCK+BODY+14*STRIDE],
            &base[BLOCK+BODY..BLOCK+BODY+14*STRIDE]);
        Ok(())
    }
    #[test]fn a_new_native_region_latches_an_uninverted_object_window()->Result<()> {
        let mut map=fixture();
        let at=BLOCK+BODY+RETAIL_COUNT*STRIDE;
        map[at..at+8].copy_from_slice(&[64,64,96,32,0,0,0,0]);
        map[BLOCK+16..BLOCK+18].copy_from_slice(&15i16.to_le_bytes());
        assert_eq!(attributes(&map,70,40)?.1.box_bytes,[64,32,96,64]);
        Ok(())
    }
    #[test]fn occupied_region_padding_rejected()->Result<()> {
        let base=fixture();
        let mut stage=base.clone();
        stage[BLOCK+BODY+14*STRIDE]=0x7f;
        assert!(plan_wide(&base,&stage).is_err());
        Ok(())
    }
    #[test]fn changed_retail_window_rejected()->Result<()> {
        let base=fixture();
        let mut stage=base.clone();
        stage[BLOCK+BODY+MAIN*STRIDE]=42;
        assert!(plan_wide(&base,&stage).is_err());
        Ok(())
    }
}

#[cfg(test)]
mod v2085_unified_region_tests {
    use super::*;

    fn fixture(body:usize)->Vec<u8> {
        let mut map=vec![0u8;0x12000];

        map[BLOCK+14..BLOCK+16]
            .copy_from_slice(&(body as i16).to_le_bytes());

        map[BLOCK+16..BLOCK+18]
            .copy_from_slice(&(RETAIL_COUNT as i16).to_le_bytes());

        let p=BLOCK+body+MAIN*STRIDE;

        map[p..p+STRIDE]
            .copy_from_slice(&[9,60,45,0,0,0,0,0]);

        let q=BLOCK+body+7*STRIDE;

        map[q..q+STRIDE]
            .copy_from_slice(&[60,35,72,45,1,0,0,0]);
        map
    }

    #[test]
    fn every_tile_latches_full_region_and_keeps_original_bits()->Result<()> {
        let original=fixture(BODY);
        let out=plan_unified(&original,&original)?;

        for [x,z] in [[0,0],[127,127],[65,39],[30,30]] {
            assert_eq!(
                attributes(&out,x,z)?.1.box_bytes,
                [0,0,128,128]
            );
        }

        assert_eq!(
            &original[BLOCK+BODY..BLOCK+BODY+14*STRIDE],
            &out[BLOCK+BODY..BLOCK+BODY+14*STRIDE]
        );

        assert_eq!(header(&out)?.1,15);

        assert_ne!(
            attributes(&original,65,39)?.1.box_bytes,
            attributes(&out,65,39)?.1.box_bytes
        );

        Ok(())
    }

    #[test]
    fn relocated_export_region_table_is_preserved()->Result<()> {
        let original=fixture(BODY);
        let relocated=0x588usize;
        let mut stage=original.clone();

        stage[BLOCK+14..BLOCK+16]
            .copy_from_slice(&(relocated as i16).to_le_bytes());

        stage[BLOCK+relocated..BLOCK+relocated+14*STRIDE]
            .copy_from_slice(
                &original[BLOCK+BODY..BLOCK+BODY+14*STRIDE]
            );

        let out=plan_unified(&original,&stage)?;

        assert_eq!(header(&out)?,(relocated,15));

        assert_eq!(
            attributes(&out,127,127)?.1.box_bytes,
            [0,0,128,128]
        );

        assert_eq!(
            &stage[BLOCK+relocated..BLOCK+relocated+14*STRIDE],
            &out[BLOCK+relocated..BLOCK+relocated+14*STRIDE]
        );

        Ok(())
    }

    #[test]
    fn occupied_padding_and_mutated_records_are_rejected()->Result<()> {
        let old=fixture(BODY);
        let mut occupied=old.clone();

        occupied[BLOCK+BODY+RETAIL_COUNT*STRIDE]=0x55;

        assert!(plan_unified(&old,&occupied).is_err());

        let mut corrupted=old.clone();
        corrupted[BLOCK+BODY+7*STRIDE]=17;

        assert!(plan_unified(&old,&corrupted).is_err());

        Ok(())
    }
}
