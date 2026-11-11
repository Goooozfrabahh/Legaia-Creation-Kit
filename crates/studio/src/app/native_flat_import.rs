//! LCK F1.1: Experimental flat-floor height serialization.
//! Runs after the existing native resource/wall pipeline.
//! No gameplay verification is implied by MAP readback.

use anyhow::{Context, Result, ensure, bail};
use std::collections::{BTreeSet, HashMap};

use legaia_disc_patch::disc::DiscPatcher;
use legaia_engine_scene::{
    scene::{Scene, ProtIndex},
    scene_assembly::AssembledScene,
};
use legaia_scene_formats::{
    field_map,
    field_objects::CELL_WALK_VISIBLE,
};
use legaia_engine_field::{
    field_elevation,
    field_regions,
};

use super::{
    native_elevation_codec as codec,
    scene_instances::{NameTarget, SceneDocument},
    world_view::WorldModel,
};

const GRID: usize = 128;
const MAP_SIZE: usize = 0x12000;
const EPS: f32 = 0.02;

fn triangle_touches_cell(p:[[f32;3];3],cell:[u8;2])->bool{
    let x0=cell[0] as f32*128.0;
    let z0=cell[1] as f32*128.0;
    let x1=x0+128.0;
    let z1=z0+128.0;

    if p.iter().all(|v|v[0]<x0) ||
       p.iter().all(|v|v[0]>x1) ||
       p.iter().all(|v|v[2]<z0) ||
       p.iter().all(|v|v[2]>z1) {
        return false;
    }

    let cx=(x0+x1)*0.5;
    let cz=(z0+z1)*0.5;

    for i in 0..3 {
        let a=p[i];
        let b=p[(i+1)%3];

        let nx=b[2]-a[2];
        let nz=a[0]-b[0];

        let mut low=f32::INFINITY;
        let mut high=f32::NEG_INFINITY;

        for v in p {
            let dot=nx*v[0]+nz*v[2];
            low=low.min(dot);
            high=high.max(dot);
        }

        let center=nx*cx+nz*cz;
        let radius=64.0*(nx.abs()+nz.abs());

        if low>center+radius+0.001 ||
           high<center-radius-0.001 {
            return false;
        }
    }

    true
}

fn floor_cells(
    world:&WorldModel,
    uid:u64,
)->Result<Vec<([u8;2],i32)>>{
    let draw=world.draws.iter()
        .find(|d|d.editor_uid==Some(uid)&&!d.hidden)
        .context("Imported floor geometry unavailable")?;

    let model=world.model.as_ref()
        .context("Editor model unavailable")?;

    let mut cells=
        std::collections::BTreeMap::<[u8;2],i32>::new();

    for tri in model.indices.chunks_exact(3) {
        let ids=[
            tri[0] as usize,
            tri[1] as usize,
            tri[2] as usize
        ];

        if !ids.iter().all(|&id|
            id>=draw.vertex_start &&
            id<draw.vertex_end &&
            id<model.positions.len()
        ) {
            continue;
        }

        let p=[
            model.positions[ids[0]],
            model.positions[ids[1]],
            model.positions[ids[2]]
        ];

        if !p.iter().flat_map(|v|v.iter())
            .all(|v|v.is_finite()) {
            continue;
        }

        let minx=p.iter()
            .map(|v|v[0]).fold(f32::INFINITY,f32::min);

        let maxx=p.iter()
            .map(|v|v[0]).fold(f32::NEG_INFINITY,f32::max);

        let minz=p.iter()
            .map(|v|v[2]).fold(f32::INFINITY,f32::min);

        let maxz=p.iter()
            .map(|v|v[2]).fold(f32::NEG_INFINITY,f32::max);

        let x0=((minx/128.0).floor() as i32).max(0);
        let x1=((maxx/128.0).floor() as i32).min(126);
        let z0=((minz/128.0).floor() as i32).max(0);
        let z1=((maxz/128.0).floor() as i32).min(126);

        if x0>x1||z0>z1 {continue;}

        let height=((p[0][1]+p[1][1]+p[2][1])/3.0)
            .round() as i32;

        for z in z0..=z1 {
            for x in x0..=x1 {
                let cell=[x as u8,z as u8];

                if triangle_touches_cell(p,cell) {
                    cells.insert(cell,height.saturating_neg());
                }
            }
        }
    }

    if cells.is_empty() {
        // Best-effort fallback for tiny or irregular geometry.
        let c=draw.center;
        cells.insert(
            [
                ((c[0]/128.0).floor() as i32).clamp(0,126) as u8,
                ((c[2]/128.0).floor() as i32).clamp(0,126) as u8
            ],
            (c[1].round() as i32).saturating_neg()
        );
    }

    Ok(cells.into_iter().collect())
}

// Best-effort height encoder. No exact-height veto.
fn nearest_native_height(
    cell:[u8;2],
    mean:i32,
    requested:i32
)->(field_elevation::ElevationOverride,i32){
    let mut best_error=i64::MAX;
    let mut chosen=(0i8,0u8,mean);

    for coarse in -128i32..=127 {
        for step in 0i32..=3 {
            let actual=i64::from(mean)
                -i64::from(coarse)*32
                -i64::from(step)*16;

            let error=(actual-i64::from(requested)).abs();

            if error<best_error {
                best_error=error;
                chosen=(
                    coarse as i8,
                    step as u8,
                    actual.clamp(
                        i64::from(i32::MIN),
                        i64::from(i32::MAX)
                    ) as i32
                );
            }
        }
    }

    (
        field_elevation::ElevationOverride {
            tile_x:cell[0],
            tile_z:cell[1],
            coarse:chosen.0,
            quads:chosen.1*0x55
        },
        chosen.2
    )
}

fn object_word(map:&[u8],cell:[u8;2])->Result<u16> {
    let at=0x8000+
        (cell[1] as usize*GRID+cell[0] as usize)*2;

    Ok(u16::from_le_bytes(
        map.get(at..at+2)
            .context("Native object grid word absent")?
            .try_into()?
    ))
}

fn floor_mean(
    map:&[u8],
    lut:&[i32;16],
    cell:[u8;2],
)->i32 {
    let at=0x4000+
        cell[1] as usize*GRID+cell[0] as usize;

    [0usize,1,128,129].iter()
        .map(|&d|lut[(map[at+d]&15) as usize])
        .sum::<i32>() >> 2
}

fn write_flat_map(
    original:&[u8],
    loaded:&[u8],
    lut:&[i32;16],
    cell:[u8;2],
    native_y:i32,
    _imported_anchor:Option<[u8;2]>,
)->Result<Vec<u8>>{
    ensure!(
        original.len()==MAP_SIZE && loaded.len()>=0x14000,
        "Native MAP data incomplete"
    );

    // LCK_EMPTY_GROUND_OWNER_FIX_V1
    // Native initialization derives 0x1000 from the cell's
    // owning descriptor. A bare grid flag is not enough.
    let mut working=original.to_vec();
    let old_word=object_word(original,cell)?;
    let old_owner=(old_word&0x01ff) as usize;

    // Count how many map cells reference each descriptor.
    // Never change a shared descriptor just to add a floor.
    let mut references=[0usize;512];

    for bytes in original[0x8000..0x10000]
        .chunks_exact(2)
    {
        let word=u16::from_le_bytes([
            bytes[0],bytes[1]
        ]);

        let id=(word&0x01ff) as usize;
        references[id]+=1;
    }

    let owner=if old_owner>=4 &&
        old_owner<512 &&
        references[old_owner]==1
    {
        // The existing cell has its own independent
        // native descriptor. Preserve that identity.
        old_owner
    }else{
        // Empty cells and shared descriptors need a
        // separately owned record. This prevents a
        // floor edit changing unrelated objects.
        let free=(4..512usize).find(|&candidate|{
            references[candidate]==0 &&
            working[candidate*32..candidate*32+32]
                .iter().all(|&value|value==0)
        }).context(
            "No unused native descriptor slot available for new ground"
        )?;

        let dst=free*32;

        if old_owner>=4 && old_owner<512 {
            // Shared descriptor: deep-copy its 32-byte
            // native record so the donor remains intact.
            let src=old_owner*32;

            let original_record=
                working[src..src+32].to_vec();

            working[dst..dst+32]
                .copy_from_slice(&original_record);
        }

        // Change ONLY this target cell's descriptor ID.
        // Keep any other native grid bits.
        let at=0x8000+
            (cell[1] as usize*GRID+cell[0] as usize)*2;

        let new_word=
            (old_word&!0x01ff)|(free as u16);

        working[at..at+2]
            .copy_from_slice(&new_word.to_le_bytes());

        free
    };

    let descriptor=owner*32;
    let flags_at=descriptor+0x12;
    let count_at=descriptor+0x16;

    let flags=u16::from_le_bytes(
        working[flags_at..flags_at+2].try_into()?
    );

    // Descriptor flag bit zero generates CELL_WALK_VISIBLE
    // during the native scene-initialization pass.
    working[flags_at..flags_at+2]
        .copy_from_slice(&(flags|1).to_le_bytes());

    // A zero countdown clears the ownership flag on boot.
    // Set a nonzero value without replacing an existing one.
    if working[count_at..count_at+2]==[0,0] {
        working[count_at..count_at+2]
            .copy_from_slice(&1u16.to_le_bytes());
    }

    let before=field_map::detect(&working)
        .context("Native MAP trigger structure invalid")?;

    let header=before.trigger_block()
        .context("Native trigger block unavailable")?;

    let mean=floor_mean(original,lut,cell);

    let (row,encoded_y)=nearest_native_height(
        cell,mean,native_y
    );

    let bytes=codec::serialize(&row)?;
    let block=&original[0x10000..0x12000];

    let mut replacement=vec![0u8;18];
    let mut cursor=18usize;

    for kind in 0..4 {
        let table=&header.tables[kind];
        let mut body=block[table.body_range()].to_vec();

        if kind==2 {
            ensure!(
                table.stride==4,
                "Native elevation record size invalid"
            );

            // Existing height rows may be replaced.
            // Same-cell overlaps use last-write-wins.
            body=body.chunks_exact(4)
                .filter(|r|
                    r[0]!=cell[0]||r[1]!=cell[1]
                )
                .flat_map(|r|r.iter().copied())
                .collect();

            body.extend_from_slice(&bytes);
        }

        ensure!(
            table.stride>0 && body.len()%table.stride==0,
            "Native trigger alignment invalid"
        );

        let count=body.len()/table.stride;

        ensure!(
            count<=i16::MAX as usize &&
            cursor<=u16::MAX as usize,
            "Native trigger storage capacity exceeded"
        );

        replacement[kind*4+2..kind*4+4]
            .copy_from_slice(&(cursor as u16).to_le_bytes());

        replacement[kind*4+4..kind*4+6]
            .copy_from_slice(&(count as u16).to_le_bytes());

        replacement.extend_from_slice(&body);

        let end=table.offset+table.count*table.stride;
        replacement.extend_from_slice(&block[end..end+2]);

        cursor=replacement.len();
    }

    ensure!(
        cursor<=0x2000,
        "Native MAP trigger storage exhausted"
    );

    replacement[..2]
        .copy_from_slice(&(cursor as u16).to_le_bytes());

    let mut output=working.clone();
    let cleared=cursor.max(header.end_offset);

    output[0x10000..0x10000+cleared].fill(0);

    output[0x10000..0x10000+cursor]
        .copy_from_slice(&replacement);

    let at=0x8000+
        (cell[1] as usize*GRID+cell[0] as usize)*2;

    output[at..at+2].copy_from_slice(
        &(object_word(&working,cell)?|CELL_WALK_VISIBLE|0x0800)
            .to_le_bytes()
    );

    field_map::detect(&output)
        .context("Written native MAP could not be decoded")?;

    let saved=field_elevation::parse_elevation_overrides(
        &output[0x10000..0x12000]
    );

    let matched=saved.iter()
        .find(|r|r.tile_x==cell[0]&&r.tile_z==cell[1])
        .context("Native height row missing after writing")?;

    ensure!(
        codec::decode(
            matched,floor_mean(&output,lut,cell)
        )?.iter().all(|v|v.height==encoded_y),
        "Actual height row readback mismatch"
    );

    Ok(output)
}

pub(super) fn apply_imports(
    disc:&mut DiscPatcher,
    entry:usize,
    scene:&AssembledScene,
    world:&WorldModel,
    index:&ProtIndex,
    doc:&SceneDocument,
    targets:&HashMap<NameTarget,(u8,u8)>,
    log:&mut Vec<String>,
    reports:&mut Vec<serde_json::Value>,
){
    for foreign in &doc.foreign {
        if super::floor_tiles::decode_floor_kind(
            &foreign.kind
        ).is_none(){
            continue;
        }

        let uid=foreign.uid;

        let result=(||->Result<(Vec<String>,Vec<String>)>{
            let anchor=*targets
                .get(&NameTarget::Created(uid))
                .context("Visual native placement not committed")?;

            let samples=floor_cells(world,uid)?;

            let native=Scene::load(index,&scene.name)?;
            let lut=native.field_floor_height_lut(index)?
                .context("Native floor lookup unavailable")?
                .map(|v|-(v as i32));

            let lba=disc.entry_disc_lba(entry)
                .context("Native MAP disc address unavailable")?;

            let loaded=disc.read_disc_sectors(lba,0x28)?;
            let mut map=disc.read_entry_footprint(entry)?;

            let mut written=Vec::new();
            let mut failed=Vec::new();

            for (cell,wanted) in samples {
                let mean=floor_mean(&map,&lut,cell);
                let (_,actual)=nearest_native_height(
                    cell,mean,wanted
                );

                match write_flat_map(
                    &map,&loaded,&lut,cell,wanted
                    ,Some([anchor.0,anchor.1])
                ){
                    Ok(updated)=>{
                        map=updated;

                        let mut initialized=map.clone();

                        field_regions::refresh_object_grid_marks(
                            &mut initialized
                        );

                        let survives=object_word(
                            &initialized,cell
                        ).map(|v|
                            v&CELL_WALK_VISIBLE!=0
                        ).unwrap_or(false);

                        written.push(format!(
                            "cell {cell:?}: requested Y {wanted}; encoded Y {actual}; difference {}; walk flag survives initialization {survives}",
                            i64::from(actual)-i64::from(wanted)
                        ));
                    }

                    Err(error)=>{
                        failed.push(format!(
                            "cell {cell:?}: {error:#}"
                        ));
                    }
                }
            }

            if !written.is_empty() {
                let mut candidate=DiscPatcher::open(
                    disc.image().to_vec()
                )?;

                candidate.patch_prot_entry(
                    entry,0,&map
                )?;

                ensure!(
                    candidate.read_entry_footprint(entry)?==map,
                    "IMG MAP readback failed"
                );

                *disc=candidate;
            }

            let _=anchor;
            Ok((written,failed))
        })();

        match result {
            Ok((written,failed))=>{
                log.push(format!(
                    "BEST-EFFORT FLOOR UID {uid}: {} native cells written; {} failed; walkability unverified",
                    written.len(),failed.len()
                ));

                for line in &written {
                    log.push(format!(
                        "FLOOR UID {uid} WRITTEN: {line}"
                    ));
                }

                for line in &failed {
                    log.push(format!(
                        "FLOOR UID {uid} UNWRITTEN: {line}"
                    ));
                }

                if let Some(report)=reports.iter_mut()
                    .find(|r|r["uid"].as_u64()==Some(uid)){
                    report["native_floor"]=serde_json::json!({
                        "encoded":!written.is_empty(),
                        "written_cells":written,
                        "failed_cells":failed,
                        "gameplay_verified":false
                    });
                }
            }

            Err(error)=>{
                log.push(format!(
                    "FLOOR UID {uid}: walkability write unavailable: {error:#}; visual import retained"
                ));

                if let Some(report)=reports.iter_mut()
                    .find(|r|r["uid"].as_u64()==Some(uid)){
                    report["native_floor"]=serde_json::json!({
                        "encoded":false,
                        "reason":format!("{error:#}"),
                        "gameplay_verified":false
                    });
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn example_map()->Vec<u8> {
        let mut map=
            super::super::native_floor_patch::fixture()
                [..MAP_SIZE].to_vec();

        let descriptor=4usize*32;

        map[descriptor+0x12..descriptor+0x14]
            .copy_from_slice(&1u16.to_le_bytes());

        map[descriptor+0x16..descriptor+0x18]
            .copy_from_slice(&1u16.to_le_bytes());

        let at=0x8000+(18*128+35)*2;

        map[at..at+2].copy_from_slice(
            &(CELL_WALK_VISIBLE|4).to_le_bytes()
        );

        map
    }

    #[test]
    fn flat_height_is_written_without_changing_walls()->Result<()> {
        let original=example_map();

        let mut loaded=original.clone();
        loaded.resize(0x14000,0);

        let new=write_flat_map(
            &original,&loaded,&[0;16],[35,18],-128,None
        )?;

        assert_eq!(
            &new[0x4000..0x8000],
            &original[0x4000..0x8000]
        );

        let rows=field_elevation::parse_elevation_overrides(
            &new[0x10000..0x12000]
        );

        assert_eq!(rows.len(),2);

        let row=rows.iter()
            .find(|r|r.tile_x==35 && r.tile_z==18)
            .context("New floor row missing")?;

        assert!(
            codec::decode(row,0)?
                .iter().all(|s|s.height==-128)
        );

        Ok(())
    }

    #[test]
    fn new_empty_ground_cell_survives_native_initialization()->Result<()> {
        let mut original=example_map();
        let cell=[40u8,20u8];
        let at=0x8000+(cell[1] as usize*GRID+cell[0] as usize)*2;

        original[at..at+2].copy_from_slice(&0u16.to_le_bytes());

        let mut loaded=original.clone();
        loaded.resize(0x14000,0);

        let output=write_flat_map(&original,&loaded,&[0;16],cell,-128,None)?;
        let mut initialized=output.clone();

        field_regions::refresh_object_grid_marks(&mut initialized);

        let word=object_word(&initialized,cell)?;

        ensure!(word&0x1800==0x1800,"Native floor bits were lost on boot");
        ensure!((word&0x01ff)>=4,"Independent ground descriptor missing");

        Ok(())
    }
}
#[cfg(test)]
mod unrestricted_height_fast_tests {
    use super::*;

    #[test]
    fn non_exact_heights_get_native_attempts()->Result<()> {
        for requested in [
            -100000,-5000,-129,-1,0,1,15,16,
            257,5000,100000
        ] {
            let (row,actual)=nearest_native_height(
                [35,18],0,requested
            );

            let decoded=codec::decode(&row,0)?;

            assert!(
                decoded.iter().all(|sample|
                    sample.height==actual
                )
            );
        }

        assert_eq!(
            nearest_native_height([35,18],0,257).1,
            256
        );

        Ok(())
    }

    #[test]
    fn narrow_rotated_floor_touches_native_cell() {
        let triangle=[
            [250.0,0.0,250.0],
            [390.0,0.0,255.0],
            [250.0,0.0,257.0]
        ];

        assert!(
            triangle_touches_cell(
                triangle,[2,1]
            )
        );
    }
}