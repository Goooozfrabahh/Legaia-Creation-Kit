
//! Native walk-ground deletion: edits only original MAP cell visibility.
//! The retail field renderer and FUN_80019278 height sampler both
//! inspect CELL_WALK_VISIBLE. No wall or corner-height bytes are erased.

use anyhow::{Result,Context,ensure};
use std::{collections::BTreeSet,path::Path,fs,io::Write};
use legaia_disc_patch::disc::DiscPatcher;
use legaia_engine_scene::scene::{Scene,ProtIndex};
use legaia_scene_formats::field_objects::CELL_WALK_VISIBLE;

type Cell=[u8;2];

fn offset(cell:Cell)->usize {
    0x8000+(usize::from(cell[1])*128+usize::from(cell[0]))*2
}


// LCK_U1B2A_VISUAL_DELETE
//
// Retail regenerates ground visibility from descriptor flags.
// A deleted floor needs a dedicated non-rendering descriptor
// when its original is shared with neighboring ground cells.
//
// Do not alter wall collision, elevation, or shared originals.

fn visual_delete(
    before:&[u8],
    after:&mut [u8],
    deleted:&BTreeSet<Cell>,
    replaced:&BTreeSet<Cell>,
)->Result<BTreeSet<usize>>{
    ensure!(
        before.len()>=0x12000 && before.len()==after.len(),
        "Native MAP is incomplete"
    );

    let mut used=BTreeSet::new();

    for at in (0x8000..0x10000).step_by(2){
        let word=u16::from_le_bytes(
            before[at..at+2].try_into()?
        );
        used.insert(usize::from(word&0x01ff));
    }

    let mut clones=std::collections::BTreeMap::new();
    let mut allocated=BTreeSet::new();

    for &cell in deleted {
        if replaced.contains(&cell) {
            continue;
        }

        let at=offset(cell);
        let old_word=u16::from_le_bytes(
            before[at..at+2].try_into()?
        );
        let original_id=usize::from(old_word&0x01ff);
        let original_at=original_id*32;

        let flags=u16::from_le_bytes(
            before[original_at+0x12..original_at+0x14]
                .try_into()?
        );

        ensure!(
            old_word&CELL_WALK_VISIBLE!=0 || flags&1!=0,
            "Deleted cell {:?} is not native walk-ground",
            cell
        );

        let mut word=u16::from_le_bytes(
            after[at..at+2].try_into()?
        );

        word &= !CELL_WALK_VISIBLE;

        if flags&1!=0 {
            // A placement descriptor may also carry its ground surface. If
            // every reference loses that surface, keep the descriptor ID and
            // all actor/script data intact; only disable its floor flag.
            if flags&4!=0 {
                let all_removed=(0..128u8).all(|z| (0..128u8).all(|x| {
                    let c=[x,z];
                    let at=offset(c);
                    let id=u16::from_le_bytes([before[at],before[at+1]])&0x1ff;
                    id as usize!=original_id || (deleted.contains(&c)&&!replaced.contains(&c))
                }));
                ensure!(all_removed,
                    "Floor {:?} shares placement descriptor #{} with retained ground; independent surface isolation required",
                    cell,original_id);
                after[original_at+0x12..original_at+0x14]
                    .copy_from_slice(&(flags&!1u16).to_le_bytes());
                allocated.insert(original_id);
                after[at..at+2].copy_from_slice(&word.to_le_bytes());
                continue;
            }
            let id=if let Some(&existing)=clones.get(&original_id){
                existing
            }else{
                let free=(4usize..512).find(|&candidate|{
                    let at=candidate*32;

                    !used.contains(&candidate)
                        && before[at..at+32].iter().all(|&b|b==0)
                        && after[at..at+32].iter().all(|&b|b==0)
                }).context(
                    "No unused native descriptor slot for floor deletion"
                )?;

                let target=free*32;

                after[target..target+32].copy_from_slice(
                    &before[original_at..original_at+32]
                );

                after[target+0x12..target+0x14]
                    .copy_from_slice(
                        &(flags&!1u16).to_le_bytes()
                    );

                used.insert(free);
                allocated.insert(free);
                clones.insert(original_id,free);

                free
            };

            word=(word&!0x01ff)|(id as u16);
        }

        after[at..at+2].copy_from_slice(
            &word.to_le_bytes()
        );
    }

    // Validate the same derived visibility flags used
    // by retail field initialization.
    let mut runtime=after.to_vec();

    legaia_engine_field::field_regions::refresh_object_grid_marks(
        &mut runtime
    );

    for &cell in deleted {
        if replaced.contains(&cell) {
            continue;
        }

        let at=offset(cell);
        let word=u16::from_le_bytes(
            runtime[at..at+2].try_into()?
        );

        ensure!(
            word&CELL_WALK_VISIBLE==0,
            "Scene initialization restored deleted floor {:?}",
            cell
        );
    }

    // This operation must not modify any wall bits.
    // Existing stamping can legitimately modify low
    // floor-tier nibbles, so preserve upper nibbles only.
    for (&a,&b) in before[0x4000..0x8000]
        .iter().zip(&after[0x4000..0x8000])
    {
        ensure!(
            a&0xf0==b&0xf0,
            "Visual-floor operation changed collision bits"
        );
    }

    Ok(allocated)
}

// U1B.2 mixed exporter must accept ONLY independently
// allocated non-rendering descriptor clones.
pub(super) fn validate_visual_descriptor_delta(
    before:&[u8],
    after:&[u8],
    owned:&BTreeSet<Cell>,
)->Result<BTreeSet<usize>>{
    ensure!(
        before.len()==after.len() && before.len()>=0x12000,
        "Native MAP size mismatch"
    );

    let mut used=BTreeSet::new();

    for at in (0x8000..0x10000).step_by(2){
        let word=u16::from_le_bytes(
            before[at..at+2].try_into()?
        );
        used.insert(usize::from(word&0x01ff));
    }

    let mut allowed=BTreeSet::new();

    // In-place floor removal on an exclusively removed placement descriptor.
    // IDs and every actor byte remain stable, including the spawn flag.
    for id in 0..512usize {
        let at=id*32;
        let flags=u16::from_le_bytes([before[at+0x12],before[at+0x13]]);
        if flags&5!=5 { continue; }
        let mut expected=before[at..at+32].to_vec();
        expected[0x12..0x14].copy_from_slice(&(flags&!1u16).to_le_bytes());
        if after[at..at+32]!=expected { continue; }
        let mut references=0;
        let valid=(0..128u8).all(|z| (0..128u8).all(|x| {
            let cell=[x,z];let p=offset(cell);
            let old=u16::from_le_bytes([before[p],before[p+1]]);
            if old&0x1ff!=id as u16 { return true; }
            references+=1;
            let new=u16::from_le_bytes([after[p],after[p+1]]);
            owned.contains(&cell)&&new&0x1ff==id as u16&&new&CELL_WALK_VISIBLE==0
        }));
        if valid&&references>0 { allowed.insert(id); }
    }

    for &cell in owned {
        let at=offset(cell);

        let old=u16::from_le_bytes(
            before[at..at+2].try_into()?
        );
        let new=u16::from_le_bytes(
            after[at..at+2].try_into()?
        );

        let old_id=usize::from(old&0x01ff);
        let new_id=usize::from(new&0x01ff);

        if old_id==new_id || used.contains(&new_id) {
            continue;
        }

        let a=old_id*32;
        let b=new_id*32;

        if !before[b..b+32].iter().all(|&v|v==0){
            continue;
        }

        let flags=u16::from_le_bytes(
            before[a+0x12..a+0x14].try_into()?
        );

        if flags&1==0 || flags&4!=0 {
            continue;
        }

        let mut expected=before[a..a+32].to_vec();

        expected[0x12..0x14].copy_from_slice(
            &(flags&!1u16).to_le_bytes()
        );

        if after[b..b+32]==expected {
            allowed.insert(new_id);
        }
    }

    for id in 0..512usize {
        let at=id*32;

        if before[at..at+32]!=after[at..at+32]{
            ensure!(
                allowed.contains(&id),
                "Unexpected native descriptor modification #{}",
                id
            );
        }
    }

    Ok(allowed)
}

pub(super) fn compile(map:&[u8],deleted:&BTreeSet<Cell>)->Result<Vec<u8>> {
    ensure!(
        map.len()>=0x12000 && !deleted.is_empty(),
        "Missing MAP grid or no selected deletions"
    );

    let mut output=map.to_vec();

    for &cell in deleted {
        let at=offset(cell);
        let before=u16::from_le_bytes(map[at..at+2].try_into()?);

        ensure!(
            before & CELL_WALK_VISIBLE!=0,
            "Native ground cell [{},{}] is already absent",
            cell[0],cell[1]
        );

        let after=before & !CELL_WALK_VISIBLE;

        output[at..at+2].copy_from_slice(&after.to_le_bytes());

        ensure!(
            (before^after)==CELL_WALK_VISIBLE,
            "Unexpected MAP flags modified"
        );
    }

    // Verify ONLY selected native grid words changed.
    let allocated=visual_delete(
        map,&mut output,deleted,&BTreeSet::new()
    )?;

    for (i,(&old,&new)) in map.iter().zip(output.iter()).enumerate() {
        if old==new {continue;}
        if i<0x4000 && allocated.contains(&(i/32)){
            continue;
        }

        let owned=deleted.iter().any(|&cell| {
            let at=offset(cell);
            i==at || i==at+1
        });

        ensure!(owned,"Unrelated MAP byte changed at {i:#x}");
    }

    // A second independent parse verifies deleted geometry is absent.
    let ground=
        legaia_scene_formats::field_objects::build_walk_heightfield(
            &output,&[0i16;16]
        );

    for points in ground.positions.chunks_exact(4) {
        let cell=[
            (points[0][0]/128.).round() as u8,
            (points[0][2]/128.).round() as u8
        ];
        ensure!(
            !deleted.contains(&cell),
            "Deleted native ground cell still renders: {:?}",
            cell
        );
    }

    Ok(output)
}

pub(super) fn export(
    source:&Path,
    output:&Path,
    name:&str,
    index:&ProtIndex,
    deleted:&BTreeSet<Cell>
)->Result<String> {
    super::ps1_original_guard::ensure_new_output(source,output)?;
    ensure!(source!=output,"Cannot overwrite source IMG");
    ensure!(
        !output.exists(),
        "Output IMG already exists. Choose a new filename."
    );
    ensure!(
        output.parent().is_some_and(|p|p.is_dir()),
        "Output directory unavailable"
    );

    let scene=Scene::load(index,name)
        .context("Scene identity unavailable")?;

    let entry=scene.field_map_index(index)
        .context("Selected scene has no native MAP")? as usize;

    let original=fs::read(source).context("Read source IMG")?;
    let expected_length=original.len();

    let mut disc=DiscPatcher::open(original)?;
    let map=disc.read_entry_footprint(entry)?;
    let patched=compile(&map,deleted)?;

    for id in 0..512usize{
        let at=id*32;
        if map[at..at+32]!=patched[at..at+32]{
            disc.patch_prot_entry(
                entry,at as u64,&patched[at..at+32]
            )?;
        }
    }

    for &cell in deleted {
        let at=offset(cell);
        disc.patch_prot_entry(
            entry,at as u64,&patched[at..at+2]
        )?;
    }

    ensure!(
        disc.read_entry_footprint(entry)?==patched,
        "MAP disc readback failed"
    );

    let image=disc.into_image();

    ensure!(
        image.len()==expected_length,
        "IMG length changed unexpectedly"
    );

    let staging=output.with_extension("lck_ground_building");

    ensure!(
        !staging.exists(),
        "Unfinished staging file exists"
    );

    let mut file=fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&staging)?;

    let result=(||->Result<()> {
        file.write_all(&image)?;
        file.sync_all()?;
        drop(file);

        fs::hard_link(&staging,output)
            .context("Cannot publish new IMG safely")?;

        let verify=DiscPatcher::open(fs::read(output)?)?;

        ensure!(
            verify.read_entry_footprint(entry)?==patched,
            "Published MAP readback failed"
        );

        super::ps1_export::verify_output_file(
            output,Some(expected_length as u64)
        )?;

        Ok(())
    })();

    let _=fs::remove_file(&staging);

    if let Err(error)=result {
        let _=fs::remove_file(output);
        return Err(error);
    }

    Ok(format!(
        "Native ground deletion IMG written: {}. \
         {} MAP walk-ground cells cleared and verified. \
         DuckStation gameplay unverified. \
         Other Rim Elm story variants unchanged.",
        output.display(),deleted.len()
    ))
}


// LCK_W1_EXACT_STAMP: native ground stamps without
// changing corner heights or unrelated MAP records.

fn native_word(map:&[u8],cell:Cell)->Result<u16>{
    let at=offset(cell);
    Ok(u16::from_le_bytes(map[at..at+2].try_into()?))
}

fn four_tiers(map:&[u8],cell:Cell)->Result<[u8;4]>{
    ensure!(
        cell[0]<127 && cell[1]<127,
        "Ground requires four in-bounds height corners"
    );

    let at=0x4000+usize::from(cell[1])*128+usize::from(cell[0]);

    Ok([0,1,128,129].map(|delta|map[at+delta]&15))
}


// Retire old elevation contributions only where the edited surface replaces
// them. Repack the native chained table so scene initialization cannot restore
// their 0x800 flags. Teleport, actor and region records remain byte-identical.
fn remove_replaced_overrides(map:&mut [u8],cells:&BTreeSet<Cell>)->Result<usize>{
    use legaia_scene_formats::field_map;
    let block=map[0x10000..0x12000].to_vec();
    if block[..18].iter().all(|&b|b==0) { return Ok(0); }
    let parsed=field_map::detect(&map[..0x12000]).context("Invalid native trigger header")?;
    let header=parsed.trigger_block().context("Missing native trigger tables")?;
    let table=&header.tables[2];
    let rows=block[table.body_range()].chunks_exact(4)
        .filter(|r|!cells.contains(&[r[0],r[1]])).flatten().copied().collect::<Vec<_>>();
    let removed=table.count-rows.len()/4;
    let old_end=header.end_offset;
    let mut replacement=vec![0u8;18];
    for kind in 0..4 {
        let table=&header.tables[kind];
        let body=if kind==2 {rows.as_slice()}else{&block[table.body_range()]};
        let cursor=replacement.len();
        replacement[kind*4+2..kind*4+4].copy_from_slice(&(cursor as u16).to_le_bytes());
        replacement[kind*4+4..kind*4+6].copy_from_slice(&((body.len()/table.stride) as u16).to_le_bytes());
        replacement.extend_from_slice(body);
        let end=table.offset+table.count*table.stride;
        replacement.extend_from_slice(&block[end..end+2]);
    }
    let end=replacement.len();replacement[..2].copy_from_slice(&(end as u16).to_le_bytes());
    let retained=[0,1,3].map(|k|parsed.trigger_records(k).into_iter().map(|r|r.to_vec()).collect::<Vec<_>>());
    map[0x10000..0x10000+old_end].fill(0);
    map[0x10000..0x10000+end].copy_from_slice(&replacement);
    for &cell in cells {
        let p=offset(cell);let word=u16::from_le_bytes([map[p],map[p+1]])&!0x800;
        map[p..p+2].copy_from_slice(&word.to_le_bytes());
    }
    let checked=field_map::detect(&map[..0x12000]).context("Repacked MAP failed parsing")?;
    for (i,k) in [0,1,3].into_iter().enumerate() {
        ensure!(checked.trigger_records(k)==retained[i],"Unrelated trigger kind {k} changed");
    }
    Ok(removed)
}

// LCK_R607_NATIVE_HEIGHT_AUTHORING
// The retail engine supports:
//   (a) bilinear 16-level native floor heights
//   (b) kind-2 coarse + four 2-bit stepped subcell heights
//
// Native height must be representable at every relevant sample.
// Unencodable edited surfaces MUST fail, not silently round and
// report successful walkability.

fn r607_fit_kind2(
    base:f32,
    target:[f32;4]
)->Result<(i8,u8)>{
    let mut best:Option<(f32,i8,u8)>=None;

    for coarse in -128i32..=127 {
        let mut packed=0u8;
        let mut maximum=0.0f32;

        for quadrant in 0..4usize {
            let shift=match quadrant {
                0=>0,
                1=>2,
                2=>4,
                _=>6,
            };

            let (fine,error)=(0u8..=3)
                .map(|fine|{
                    let elevation=base
                        -(coarse as f32)*32.0
                        -(fine as f32)*16.0;

                    (fine,(elevation-target[quadrant]).abs())
                })
                .min_by(|a,b|a.1.total_cmp(&b.1))
                .context("No native subcell height")?;

            packed|=fine<<shift;
            maximum=maximum.max(error);
        }

        if best.as_ref().is_none_or(|(old,_,_)|maximum<*old){
            best=Some((maximum,coarse as i8,packed));
        }
    }

    let (error,coarse,packed)=best.context(
        "Native kind-2 height fit unavailable"
    )?;

    ensure!(
        error<=0.5,
        "Desired height is not exactly encodable in 16-unit kind-2 steps (error {:.2})",
        error
    );

    Ok((coarse,packed))
}

fn r607_native_heights(
    donor:&[u8],
    mut map:Vec<u8>,
    stamps:&[super::ground_edit_ops::W46Stamp],
    lut:&[i16;16]
)->Result<(Vec<u8>,Vec<String>)>{
    use std::collections::BTreeMap;
    use legaia_scene_formats::field_map;

    ensure!(
        donor.len()>=0x12000 && map.len()==donor.len(),
        "R607 native height MAP is incomplete"
    );

    let mut final_stamps=BTreeMap::<
        Cell,super::ground_edit_ops::W46Stamp
    >::new();

    for &stamp in stamps {
        final_stamps.insert(stamp.target,stamp);
    }

    if final_stamps.is_empty() {
        return Ok((map,Vec::new()));
    }

    let parsed=field_map::detect(&map[..0x12000])
        .context("R607 cannot parse native MAP height tables")?;

    let header=parsed.trigger_block()
        .context("Native MAP has no kind-2 table")?;

    let region=0x10000usize;
    let original=map[region..region+0x2000].to_vec();

    let old_end=header.end_offset;

    ensure!(
        old_end<=original.len(),
        "Native trigger table extends outside MAP"
    );

    let old_kind2=&original[
        header.tables[2].body_range()
    ];

    let touched=final_stamps.keys()
        .copied()
        .collect::<BTreeSet<_>>();

    let mut rows=old_kind2.chunks_exact(4)
        .filter(|r|!touched.contains(&[r[0],r[1]]))
        .map(|r|[r[0],r[1],r[2],r[3]])
        .collect::<Vec<_>>();

    let mut inserted=0usize;
    let mut bilinear=0usize;

    for (target,stamp) in final_stamps {
        ensure!(
            native_word(donor,stamp.source)?&0x0800==0,
            "Source floor {:?} already uses a kind-2 override; exact transformed donor override support is required",
            stamp.source
        );

        ensure!(
            target[0]<127 && target[1]<127,
            "Native height corner outside 128-tile grid"
        );

        let x=target[0] as f32*128.0;
        let z=target[1] as f32*128.0;

        let tiers=four_tiers(&map,target)?;

        let corner_heights=tiers.map(|tier|
            lut[tier as usize] as f32
        );

        let mut exact_bilinear=true;

        for (q,(dx,dz)) in [
            (0.0f32,0.0f32),
            (128.0,0.0),
            (0.0,128.0),
            (128.0,128.0)
        ].into_iter().enumerate(){
            let requested=w46_height(
                donor,stamp,x+dx,z+dz,lut
            )?;

            if (requested-corner_heights[q]).abs()>0.5 {
                exact_bilinear=false;
            }
        }

        if exact_bilinear {
            bilinear+=1;
            continue;
        }

        let sum=tiers.iter()
            .map(|&tier|lut[tier as usize] as i32)
            .sum::<i32>();

        let base=(sum>>2) as f32;
        let mut expected=[0.0f32;4];

        // Retail kind-2 records are constant inside each 64x64
        // quadrant. A sloped authored quadrant cannot be replaced
        // by a discontinuous step without changing gameplay.
        for q in 0..4usize {
            let dx=(q%2) as f32*64.0;
            let dz=(q/2) as f32*64.0;

            let center=w46_height(
                donor,stamp,
                x+dx+32.0,z+dz+32.0,lut
            )?;

            for (ox,oz) in [
                (8.0f32,8.0f32),
                (56.0,8.0),
                (8.0,56.0),
                (56.0,56.0)
            ] {
                let sample=w46_height(
                    donor,stamp,
                    x+dx+ox,z+dz+oz,lut
                )?;

                ensure!(
                    (sample-center).abs()<=0.5,
                    "Floor {:?} is sloped within a native kind-2 quadrant; no exact stepped surface encoding",
                    target
                );
            }

            expected[q]=center;
        }

        let (coarse,packed)=r607_fit_kind2(base,expected)
            .with_context(||format!(
                "Native floor {:?} cannot encode its authored Y",
                target
            ))?;

        rows.push([
            target[0],
            target[1],
            coarse as u8,
            packed
        ]);

        inserted+=1;
    }

    // Preserve native trigger kinds 0, 1 and 3 byte-for-byte.
    // The fourth table uses an 8-byte stride.
    let retained=[0u8,1,3].map(|kind|
        parsed.trigger_records(kind)
            .into_iter()
            .map(|record|record.to_vec())
            .collect::<Vec<_>>()
    );

    let mut replacement=vec![0u8;18];

    for kind in 0..4usize {
        let table=&header.tables[kind];

        let body=if kind==2 {
            rows.iter()
                .flat_map(|record|record.iter().copied())
                .collect::<Vec<_>>()
        }else{
            original[table.body_range()].to_vec()
        };

        let start=replacement.len();

        ensure!(
            start<=u16::MAX as usize,
            "Native trigger offset overflow"
        );

        ensure!(
            body.len()%table.stride==0,
            "Native trigger table has invalid stride"
        );

        let count=body.len()/table.stride;

        ensure!(
            count<=u16::MAX as usize,
            "Native trigger count overflow"
        );

        replacement[kind*4+2..kind*4+4]
            .copy_from_slice(&(start as u16).to_le_bytes());

        replacement[kind*4+4..kind*4+6]
            .copy_from_slice(&(count as u16).to_le_bytes());

        replacement.extend_from_slice(&body);

        let old_gap=table.offset+table.count*table.stride;

        replacement.extend_from_slice(
            original.get(old_gap..old_gap+2)
                .context("Native trigger table gap truncated")?
        );
    }

    let end=replacement.len();

    ensure!(
        end<=0x2000 && end<=u16::MAX as usize,
        "Native MAP kind-2 record capacity exceeded"
    );

    // Do not overwrite unknown nonzero tail data when growing.
    if end>old_end {
        ensure!(
            original[old_end..end].iter().all(|&b|b==0),
            "Native MAP kind-2 table requires occupied trigger-tail space"
        );
    }

    replacement[0..2]
        .copy_from_slice(&(end as u16).to_le_bytes());

    map[region..region+old_end.max(end)].fill(0);
    map[region..region+end].copy_from_slice(&replacement);

    // Runtime initialization regenerates 0x0800 from the kind-2 table.
    // Clear stale marks from changed cells, then prove the regenerated
    // mark matches the new table.
    for &cell in &touched {
        let p=offset(cell);
        let word=native_word(&map,cell)?&!0x0800;

        map[p..p+2].copy_from_slice(
            &word.to_le_bytes()
        );
    }

    let verified=field_map::detect(&map[..0x12000])
        .context("R607 native MAP table failed readback parsing")?;

    for (i,kind) in [0u8,1,3].into_iter().enumerate(){
        ensure!(
            verified.trigger_records(kind)==retained[i],
            "R607 modified unrelated native trigger kind {kind}"
        );
    }

    let authored=verified.trigger_records(2)
        .into_iter()
        .map(|record|[record[0],record[1]])
        .collect::<BTreeSet<_>>();

    let mut runtime=map.clone();

    legaia_engine_field::field_regions::refresh_object_grid_marks(
        &mut runtime
    );

    for cell in touched {
        let word=native_word(&runtime,cell)?;

        ensure!(
            word&CELL_WALK_VISIBLE!=0,
            "Native runtime removed walkability from floor {:?}",
            cell
        );

        ensure!(
            (word&0x0800!=0)==authored.contains(&cell),
            "Native kind-2 initialization disagrees with exported floor {:?}",
            cell
        );
    }

    Ok((
        map,
        vec![format!(
            "R607 NATIVE HEIGHTS: {} exact bilinear cells; {} exact kind-2 records; 0 accepted height approximations. Gameplay requires DuckStation verification.",
            bilinear,inserted
        )]
    ))
}

#[cfg(test)]
mod lck_r607_native_height_tests {
    use super::*;

    #[test]
    fn stepped_height_encodes_exact_native_values()->Result<()> {
        let (coarse,packed)=r607_fit_kind2(
            0.0,[32.0,32.0,32.0,32.0]
        )?;

        for quadrant in 0..4usize {
            let fine=((packed>>(quadrant*2))&3) as f32;
            let actual=-(coarse as f32)*32.0-fine*16.0;

            assert_eq!(actual,32.0);
        }

        Ok(())
    }

    #[test]
    fn unrepresentable_height_is_rejected(){
        assert!(
            r607_fit_kind2(
                0.0,[5.0,5.0,5.0,5.0]
            ).is_err()
        );
    }
}
fn verify_runtime_surfaces(map:&[u8],deleted:&BTreeSet<Cell>,stamps:&[(Cell,Cell)])->Result<()> {
    let replaced=stamps.iter().map(|&(_,c)|c).collect::<BTreeSet<_>>();
    let mut initialized=map.to_vec();
    legaia_engine_field::field_regions::refresh_object_grid_marks(&mut initialized);
    for &c in deleted.difference(&replaced) {
        ensure!(native_word(&initialized,c)?&CELL_WALK_VISIBLE==0,
            "Scene initialization restored deleted floor {c:?}");
    }
    for &c in &replaced {
        let word=native_word(&initialized,c)?;
        ensure!(word&CELL_WALK_VISIBLE!=0,"Scene initialization removed replacement floor {c:?}");
        ensure!(word&0x800==0,"Old elevation override survived on replacement floor {c:?}");
    }
    Ok(())
}

// Unrestricted native projection. Occupied cells use last-wins
// semantics. Exact source geometry is NOT serialized.
// R60: immutable donor and current working MAP must be separate.
fn w2b_compile(map:&[u8],deleted:&BTreeSet<Cell>,stamps:&[(Cell,Cell)])
    ->Result<(Vec<u8>,Vec<String>)>{
    w2b_compile_dual(map,map,deleted,stamps)
}
fn w2b_compile_dual(donor:&[u8],map:&[u8],deleted:&BTreeSet<Cell>,stamps:&[(Cell,Cell)])
    ->Result<(Vec<u8>,Vec<String>)>{
    ensure!(map.len()>=0x12000 && donor.len()==map.len(),"Invalid dual native MAP size");
    let mut out=map.to_vec();
    let mut notes=Vec::new();
    let mut owners=BTreeSet::new();

    for &cell in deleted {
        let at=offset(cell);
        let old=native_word(map,cell)?;
        if old&CELL_WALK_VISIBLE==0 {
            notes.push(format!("DELETE {:?}: walk bit already absent",cell));
        }
        out[at..at+2].copy_from_slice(
            &(old&!CELL_WALK_VISIBLE).to_le_bytes()
        );
    }

    for &(source,target) in stamps {
        let before=native_word(donor,source)?;
        let displaced=native_word(&out,target)?;

        if !owners.insert(target){
            notes.push(format!("OVERLAP {:?}: last floor wins",target));
        }
        if displaced!=0&&displaced!=0x8000&&source!=target {
            notes.push(format!(
                "OVERWRITE {:?}: replaced cell word {displaced:#06x}",
                target
            ));
        }
        if before&CELL_WALK_VISIBLE==0 {
            notes.push(format!(
                "SOURCE {:?}: forced walk flag ON",source
            ));
        }
        if before&0x0800!=0 {
            notes.push(format!(
                "SOURCE {:?}: elevation override not transplanted",source
            ));
        }

        let word=(before&(0x01ff|0x8000))|CELL_WALK_VISIBLE;
        let at=offset(target);
        out[at..at+2].copy_from_slice(&word.to_le_bytes());

        if source[0]<127&&source[1]<127&&target[0]<127&&target[1]<127 {
            let tiers=four_tiers(donor,source)?;
            let old_tiers=four_tiers(&out,target)?;

            if tiers!=old_tiers {
                notes.push(format!(
                    "HEIGHT {:?}: shared corner tiers {:?} -> {:?}",
                    target,old_tiers,tiers
                ));
            }

            let start=0x4000+target[1] as usize*128+target[0] as usize;

            for (j,&delta) in [0usize,1,128,129].iter().enumerate(){
                let old=out[start+delta];
                out[start+delta]=(old&0xf0)|tiers[j];
            }
        }else{
            notes.push(format!(
                "HEIGHT {:?}: corner data unavailable",target
            ));
        }
    }
    let replaced=stamps.iter()
        .map(|&(_,target)|target)
        .collect::<BTreeSet<_>>();

    // Object deletion may already have cleared some ground ownership.
    let live_deleted=deleted.iter().copied().filter(|&c|
        native_word(map,c).is_ok_and(|v|v&CELL_WALK_VISIBLE!=0)
    ).collect::<BTreeSet<_>>();
    let allocated=visual_delete(map,&mut out,&live_deleted,&replaced)?;

    if !allocated.is_empty(){
        notes.push(format!(
            "VISUAL FLOOR DELETE: {} isolated descriptors",
            allocated.len()
        ));
    }

    let touched=deleted.iter().copied().chain(replaced.iter().copied()).collect();
    let removed=remove_replaced_overrides(&mut out,&touched)?;
    notes.push(format!("REPLACED ELEVATION: removed {removed} old native override records"));
    Ok((out,notes))
}

// LCK_R46_GROUND_COMPOSITOR
// Reconstruct height using each original floor tile's four
// corner heights, world-space transform, and native LUT.

fn w46_height(
    original:&[u8],
    stamp:super::ground_edit_ops::W46Stamp,
    at_x:f32,
    at_z:f32,
    lut:&[i16;16]
)->Result<f32>
{
    let [a,b,c,d]=four_tiers(original,stamp.source)?;

    let h=[
        lut[a as usize] as f32,
        lut[b as usize] as f32,
        lut[c as usize] as f32,
        lut[d as usize] as f32
    ];

    let t=stamp.transform;

    ensure!(
        t.scale.iter().all(|v|v.is_finite()&&v.abs()>0.00001),
        "Native floor has invalid or zero scale"
    );

    let cx=stamp.source[0] as f32*128.+64.+t.offset[0];
    let cz=stamp.source[1] as f32*128.+64.+t.offset[2];

    let (rx,rz)=(at_x-cx,at_z-cz);

    let (sx,sz)=match t.turns.rem_euclid(4) {
        0=>(rx,rz),
        1=>(rz,-rx),
        2=>(-rx,-rz),
        _=>(-rz,rx)
    };

    let u=((sx/t.scale[0]+64.)/128.).clamp(0.,1.);
    let v=((sz/t.scale[2]+64.)/128.).clamp(0.,1.);

    let y=h[0]*(1.-u)*(1.-v)
        +h[1]*u*(1.-v)
        +h[2]*(1.-u)*v
        +h[3]*u*v;

    let mid=(h[0]+h[1]+h[2]+h[3])/4.;

    Ok(mid+(y-mid)*t.scale[1]+t.offset[1])
}

// LCK_R60_IMMUTABLE_GROUND
// A replacement walk tile cannot borrow a mutable placed-object
// descriptor. A shared immutable ground-only descriptor is allowed.
fn r60_isolate_ground(
    donor:&[u8],
    working:&[u8],
    out:&mut [u8],
    stamps:&[super::ground_edit_ops::W46Stamp]
)->Result<Vec<String>> {
    use std::collections::BTreeMap;

    ensure!(
        donor.len()>=0x12000 &&
        donor.len()==working.len() &&
        out.len()==donor.len(),
        "R6.0 native MAP footprints differ"
    );

    let mut latest=BTreeMap::<
        Cell,super::ground_edit_ops::W46Stamp
    >::new();

    for &stamp in stamps {
        latest.insert(stamp.target,stamp);
    }

    let mut reserved=BTreeSet::<usize>::new();

    // Include retail, post-object and final references to prevent
    // allocating an ID still needed by another subsystem.
    for image in [donor,working,&*out] {
        for at in (0x8000..0x10000).step_by(2) {
            reserved.insert(
                (u16::from_le_bytes([
                    image[at],image[at+1]
                ])&511) as usize
            );
        }
    }

    let mut copies=BTreeMap::<usize,usize>::new();
    let mut notes=Vec::new();

    for (target,stamp) in latest {
        let old=native_word(out,target)?;

        if old&legaia_scene_formats::field_objects::CELL_BIND_OWNED!=0 {
            let id=(old&511) as usize;
            let flags=u16::from_le_bytes(
                out[id*32+18..id*32+20].try_into()?
            );

            ensure!(
                flags&1!=0,
                "Destination {:?} is script-owned without native ground",
                target
            );

            // Preserve MAN ownership instead of inventing a new binding.
            continue;
        }

        let src=(native_word(donor,stamp.source)?&511) as usize;
        let src_at=src*32;
        let donor_rec=&donor[src_at..src_at+32];

        let donor_flags=u16::from_le_bytes(
            donor_rec[18..20].try_into()?
        );

        ensure!(
            (native_word(donor,stamp.source)?&CELL_WALK_VISIBLE)!=0
                || donor_flags&1!=0,
            "Donor {:?} is not a native walk surface",
            stamp.source
        );

        let current=(old&511) as usize;

        let current_flags=u16::from_le_bytes(
            out[current*32+18..current*32+20].try_into()?
        );

        let safe=current==src
            && donor_flags&4==0
            && current_flags&4==0
            && out[src_at..src_at+32]==donor_rec[..];

        if safe {
            continue;
        }

        let id=if let Some(&id)=copies.get(&src) {
            id
        } else {
            let id=(4usize..512).find(|&id| {
                let at=id*32;

                !reserved.contains(&id)
                    && donor[at..at+32].iter().all(|&b|b==0)
                    && working[at..at+32].iter().all(|&b|b==0)
                    && out[at..at+32].iter().all(|&b|b==0)
            }).context(
                "R6.0 no unowned ground-only descriptor slot"
            )?;

            out[id*32..id*32+32].fill(0);

            // Ground-only flags. No placed object or actor flag.
            out[id*32+18..id*32+20]
                .copy_from_slice(&1u16.to_le_bytes());

            // Preserve the donor's actual native terrain atlas data.
            out[id*32+20..id*32+24]
                .copy_from_slice(&donor_rec[20..24]);

            reserved.insert(id);
            copies.insert(src,id);

            notes.push(format!(
                "R6.0 ground descriptor {src} -> independent ground-only {id}"
            ));

            id
        };

        let at=offset(target);

        out[at..at+2].copy_from_slice(
            &((old&!511)|(id as u16)|CELL_WALK_VISIBLE).to_le_bytes()
        );
    }

    Ok(notes)
}

fn w46_reconcile(
    original:&[u8],out:Vec<u8>,
    stamps:&[super::ground_edit_ops::W46Stamp],lut:&[i16;16]
)->Result<(Vec<u8>,Vec<String>)>{
    w46_reconcile_dual(original,original,out,stamps,lut)
}
fn w46_reconcile_dual(
    original:&[u8],working:&[u8],mut out:Vec<u8>,
    stamps:&[super::ground_edit_ops::W46Stamp],lut:&[i16;16]
)->Result<(Vec<u8>,Vec<String>)>{
    let mut last=std::collections::BTreeMap::<
        Cell,
        super::ground_edit_ops::W46Stamp
    >::new();

    for &stamp in stamps {
        last.insert(stamp.target,stamp);
    }

    let mut corners=std::collections::BTreeMap::<
        usize,(f32,usize)
    >::new();

    let mut composed=0usize;
    let mut preserved_binds=0usize;
    let mut clipped_border=0usize;
    let mut notes=Vec::new();

    for (priority,(target,stamp)) in
        last.into_iter().enumerate()
    {
        let word=native_word(original,stamp.source)?;
        let previous=native_word(working,target)?;
        let at=offset(target);

        let desc=if previous
            &legaia_scene_formats::field_objects::CELL_BIND_OWNED!=0
        {
            preserved_binds+=1;
            previous&0x01ff
        } else {
            word&0x01ff
        };

        let merged=(previous&!(0x01ff|0x0800))|desc|CELL_WALK_VISIBLE;

        out[at..at+2].copy_from_slice(
            &merged.to_le_bytes()
        );

        composed+=1;

        if target[0]==127||target[1]==127||
           stamp.source[0]==127||stamp.source[1]==127
        {
            clipped_border+=1;
            continue;
        }

        let base=0x4000
            +target[1] as usize*128
            +target[0] as usize;

        for (dx,dz,delta) in [
            (0f32,0f32,0usize),
            (1.,0.,1),
            (0.,1.,128),
            (1.,1.,129)
        ] {
            let height=w46_height(
                original,
                stamp,
                (target[0] as f32+dx)*128.,
                (target[1] as f32+dz)*128.,
                lut
            )?;

            let idx=base+delta;
            corners.insert(idx,(height,priority));
        }
    }

    let min=lut.iter().copied().min().unwrap_or(0) as f32;
    let max=lut.iter().copied().max().unwrap_or(0) as f32;

    let mut approximate=0usize;
    let mut out_of_ladder=0usize;
    let mut max_error=0f32;

    for (idx,(height,_)) in corners {
        if height<min-0.5||height>max+0.5 {
            out_of_ladder+=1;
        }

        let (tier,error)=lut.iter()
            .enumerate()
            .map(|(i,&v)|(i,(v as f32-height).abs()))
            .min_by(|a,b|a.1.total_cmp(&b.1))
            .context("Native height ladder unavailable")?;

        max_error=max_error.max(error);

        if error>0.5 {
            approximate+=1;
        }

        out[idx]=(out[idx]&0xf0)|tier as u8;
    }

    notes.push(format!(
        "W46 HEIGHT COMPOSITE: {} unique native floor cells, {} approximate height corners, {} outside existing 16-level ladder, max height error {:.2} PS1 units; walkability NOT gameplay-verified",
        composed,
        approximate,
        out_of_ladder,
        max_error
    ));

    if preserved_binds>0 {
        notes.push(format!(
            "W46 SCRIPT OWNERSHIP: {} destination object links preserved; these cells gain a walk flag but retain their existing script-owned descriptor",
            preserved_binds
        ));
    }

    if clipped_border>0 {
        notes.push(format!(
            "W46 BORDER: {} final row/column cells lack four writable height corners",
            clipped_border
        ));
    }

    notes.extend(r60_isolate_ground(original,working,&mut out,stamps)?);
    Ok((out,notes))
}

#[cfg(test)]
mod lck_r46_height_tests {
    use super::*;

    #[test]
    fn equal_height_overlap_remains_walkable() -> Result<()> {
        let mut map=vec![0u8;0x12000];

        let source=[18u8,19u8];
        let target=[20u8,21u8];

        let source_at=offset(source);

        map[source_at..source_at+2].copy_from_slice(
            &(CELL_WALK_VISIBLE|7u16).to_le_bytes()
        );

        let stamp=crate::app::ground_edit_ops::W46Stamp {
            source,
            target,
            transform:crate::app::ground_edit_ops::Transform::default(),
            uid:Some(1)
        };

        let lut=[0i16;16];

        let (output,_)=w46_reconcile(
            &map,
            map.clone(),
            &[stamp],
            &lut
        )?;

        assert_ne!(
            native_word(&output,target)?&CELL_WALK_VISIBLE,
            0
        );

        Ok(())
    }
}
fn compile_stamps(
    map:&[u8],
    deleted:&BTreeSet<Cell>,
    stamps:&[(Cell,Cell)]
)->Result<Vec<u8>>{
    ensure!(map.len()>=0x12000,"Native MAP is incomplete");
    ensure!(
        !deleted.is_empty() || !stamps.is_empty(),
        "Nothing to export"
    );

    let mut output=map.to_vec();
    let mut touched=deleted.clone();

    for &cell in deleted {
        let old=native_word(map,cell)?;

        ensure!(
            old&CELL_WALK_VISIBLE!=0,
            "Deleted native source {:?} is not walk-ground",cell
        );

        let at=offset(cell);

        output[at..at+2].copy_from_slice(
            &(old&!CELL_WALK_VISIBLE).to_le_bytes()
        );
    }

    let mut source_counts=std::collections::BTreeMap::new();

    for &(source,_) in stamps {
        *source_counts.entry(source).or_insert(0usize)+=1;
    }

    for (&source,&count) in &source_counts {
        if count>1 {
            let corners=four_tiers(map,source)?;
            ensure!(
                corners.iter().all(|&v|v==corners[0]),
                "Resized source {:?} is sloped; extended native height required",
                source
            );
        }
    }

    for &(source,target) in stamps {
        ensure!(
            source!=target,
            "A duplicate must have a distinct native destination"
        );

        ensure!(
            touched.insert(target),
            "Conflicting ground destination {:?}",target
        );

        let source_word=native_word(map,source)?;
        let target_word=native_word(map,target)?;

        ensure!(
            source_word&CELL_WALK_VISIBLE!=0,
            "Source {:?} lacks native walk-ground",source
        );

        ensure!(
            source_word&0x0800==0,
            "Source {:?} uses native elevation overrides; exact-height transplant required",
            source
        );

        // Never overwrite another native object's or actor's grid link.
        ensure!(
            target_word==0 || target_word==0x8000,
            "Destination {:?} has native occupants/flags ({target_word:#06x})",
            target
        );

        // Reuse the source height tiers only when destination heights
        // are already identical. Shared corner bytes stay unchanged.
        ensure!(
            four_tiers(map,source)?==four_tiers(map,target)?,
            "Source/destination floor-height tiers differ; native height export required"
        );

        // Low nine bits reference an existing terrain ObjectRecord
        // in this same native cell's resource set.
        let new_word=
            source_word&(0x01ff|CELL_WALK_VISIBLE|0x8000);

        let at=offset(target);
        output[at..at+2].copy_from_slice(
            &new_word.to_le_bytes()
        );
    }

    // Verify only specifically owned grid words changed.
    let replaced=stamps.iter()
        .map(|&(_,target)|target)
        .collect::<BTreeSet<_>>();

    let allocated=visual_delete(
        map,&mut output,deleted,&replaced
    )?;

    for (i,(&a,&b)) in map.iter().zip(output.iter()).enumerate(){
        if a==b {continue;}
        if i<0x4000 && allocated.contains(&(i/32)){
            continue;
        }

        let covered=touched.iter().any(|&c|
            i==offset(c)||i==offset(c)+1
        );

        ensure!(
            covered,
            "Native floor writer changed unrelated byte {i:#x}"
        );
    }

    ensure!(
        output[0x4000..0x8000]==map[0x4000..0x8000],
        "Native floor-height and wall bits changed unexpectedly"
    );

    let rebuilt=
        legaia_scene_formats::field_objects::build_walk_heightfield(
            &output,&[0i16;16]
        );

    let seen=rebuilt.positions.chunks_exact(4)
        .map(|quad|[
            (quad[0][0]/128.).round() as u8,
            (quad[0][2]/128.).round() as u8
        ])
        .collect::<BTreeSet<_>>();

    for &c in deleted {
        if !stamps.iter().any(|&(_,target)|target==c) {
            ensure!(
                !seen.contains(&c),
                "Deleted floor still rendered at {:?}",c
            );
        }
    }

    for &(_,target) in stamps {
        ensure!(
            seen.contains(&target),
            "New floor missing at {:?}",target
        );
    }

    Ok(output)
}

pub(super) fn export_ground(
    source:&Path,
    output:&Path,
    name:&str,
    index:&ProtIndex,
        edits:&super::ground_edit_ops::State,unrestricted:bool)->Result<String>{
        let (deleted,stamps,mut notes)=if unrestricted {
        super::ground_edit_ops::w46_plan(edits).map_err(anyhow::Error::msg)?
    }else{
        let (a,b)=super::ground_edit_ops::native_128_plan(edits)
            .map_err(anyhow::Error::msg)?;
        (a,b,Vec::new())
    };

    super::ps1_original_guard::ensure_new_output(source,output)?;
    ensure!(source!=output,"Cannot overwrite source IMG");
    ensure!(!output.exists(),"Choose a NEW output IMG");
    ensure!(
        output.parent().is_some_and(|p|p.is_dir()),
        "Output folder does not exist"
    );

    let scene=Scene::load(index,name)?;
    let entry=scene.field_map_index(index)
        .context("Scene has no native MAP")? as usize;

    let original=fs::read(source)?;
    let expected_len=original.len();

    let mut disc=DiscPatcher::open(original)?;
    let before=disc.read_entry_footprint(entry)?;
    // Read immutable donor data from the mounted original retail MAP,
    // never the IMG after object deletion or placement.
    let immutable=index.entry_bytes(entry as u32)?;
    ensure!(
        immutable.len()==before.len(),
        "R6.0 immutable retail donor MAP size mismatch"
    );

    // Preserve exact walking heights on resized native floors.
    if !unrestricted {
        for source in super::ground_edit_ops::native_resized_sources(edits) {
            let tiers=four_tiers(&before,source)?;
            ensure!(tiers.iter().all(|&t|t==tiers[0]),
                "Resized source tile {:?} is sloped; extended native height support required",source);
        }
    }
    let after=if unrestricted {
        let (bytes,extra)=w2b_compile_dual(&immutable,&before,&deleted,&stamps)?;
        notes.extend(extra);

        let (_,detailed,_)=super::ground_edit_ops::
            w46_plan_detailed(edits)
            .map_err(anyhow::Error::msg)?;

        let lut=scene.field_floor_height_lut(index)?
            .context("Selected scene has no native height ladder")?;

        let (bytes,extra)=w46_reconcile_dual(
            &immutable,
            &before,
            bytes,
            &detailed,
            &lut
        )?;

        notes.extend(extra);
        verify_runtime_surfaces(&bytes,&deleted,&stamps)?;

        // W46 first composes native floor descriptors. R607 then
        // encodes the resulting authored heights as bilinear tiers
        // or exact kind-2 overrides. Unsupported heights fail.
        let (bytes,height_notes)=r607_native_heights(
            &immutable,
            bytes,
            &detailed,
            &lut
        )?;

        notes.extend(height_notes);

        notes.push(
            "R607: native walkability and kind-2 table verified after initialization; gameplay not yet tested".into()
        );

        bytes
    }else{
        compile_stamps(&before,&deleted,&stamps)?
    };


    let touched=deleted.iter().copied()
        .chain(stamps.iter().map(|&(_,target)|target))
        .collect::<BTreeSet<_>>();
    if unrestricted {
        let mut i=0;
        while i<before.len(){
            if before[i]==after[i] {i+=1;continue;}
            let start=i;
            while i<before.len()&&before[i]!=after[i]&&i-start<1024 {
                i+=1;
            }
            disc.patch_prot_entry(
                entry,start as u64,&after[start..i]
            )?;
        }
    }else{
        for id in 0..512usize {
            let at=id*32;
            if before[at..at+32]!=after[at..at+32]{
                disc.patch_prot_entry(
                    entry,at as u64,&after[at..at+32]
                )?;
            }
        }
        for cell in touched {
            let at=offset(cell);
            disc.patch_prot_entry(entry,at as u64,&after[at..at+2])?;
        }
    }
    ensure!(
        disc.read_entry_footprint(entry)?==after,
        "Patched MAP readback differed"
    );

    let image=disc.into_image();

    ensure!(
        image.len()==expected_len,
        "Raw IMG size changed"
    );

    let staging=output.with_extension("lck_w1_building");

    ensure!(
        !staging.exists(),
        "W1 staging file already exists"
    );

    let mut file=fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&staging)?;

    let result=(||->Result<()>{
        file.write_all(&image)?;
        file.sync_all()?;
        drop(file);

        fs::hard_link(&staging,output)?;

        let verify=DiscPatcher::open(fs::read(output)?)?;

        ensure!(
            verify.read_entry_footprint(entry)?==after,
            "Published IMG MAP readback differs"
        );

        super::ps1_export::verify_output_file(
            output,Some(expected_len as u64)
        )?;

        Ok(())
    })();

    let _=fs::remove_file(&staging);

    if let Err(error)=result {
        let _=fs::remove_file(output);
        return Err(error);
    }

    if unrestricted {
        let log_path=output.with_extension("w2_unrestricted_debug.txt");

        let mut report=format!(
            "W2B UNRESTRICTED EXPERIMENT\nSource: {}\nOutput: {}\nScene: {}\nDeleted cells: {}\nRasterized writes: {}\n",
            source.display(),output.display(),name,deleted.len(),stamps.len()
        );

        report.push_str(
            "NOT EXACT: partial cells become full 128-unit cells; heights rounded to the native ladder; overlapping edits overwrite cells; textures repeat.\n"
        );

        for &(a,b) in &stamps {
            report.push_str(&format!("STAMP {:?} => {:?}\n",a,b));
        }
        for note in &notes {
            report.push_str(note);
            report.push('\n');
        }

        let report_status=match fs::write(&log_path,report){
            Ok(())=>format!("Diagnostics: {}",log_path.display()),
            Err(e)=>format!("Diagnostics write failed: {e}")
        };

        Ok(format!(
            "W2B UNRESTRICTED IMG: {}. {} attempted stamps, {} warnings. {}. Disc readback passed; gameplay NOT verified.",
            output.display(),stamps.len(),notes.len(),report_status
        ))
    }else{
        Ok(format!(
            "W2A strict IMG: {}. {} deletions and {} stamped. Disc readback passed; gameplay UNVERIFIED.",
            output.display(),deleted.len(),stamps.len()
        ))
    }
}

#[cfg(test)] mod w1_exact_stamps_tests {
    use super::*;

    #[test]
    fn empty_cell_receives_source_texture_and_walk_flag()->Result<()>{
        let mut map=vec![0u8;0x12000];
        let source=[10,11];
        let target=[15,11];

        map[offset(source)..offset(source)+2]
            .copy_from_slice(
                &(CELL_WALK_VISIBLE|7u16).to_le_bytes()
            );

        let after=compile_stamps(
            &map,&BTreeSet::new(),&[(source,target)]
        )?;

        assert_eq!(
            native_word(&after,target)?,
            CELL_WALK_VISIBLE|7
        );
        assert_eq!(
            native_word(&after,source)?,
            CELL_WALK_VISIBLE|7
        );
        assert_eq!(
            &after[0x4000..0x8000],
            &map[0x4000..0x8000]
        );
        assert_eq!(native_word(&map,target)?,0);

        Ok(())
    }

    #[test]
    fn rejects_occupied_cell_and_height_mismatch()->Result<()>{
        let mut map=vec![0u8;0x12000];
        let source=[10,11];
        let target=[15,11];

        map[offset(source)..offset(source)+2]
            .copy_from_slice(
                &(CELL_WALK_VISIBLE|7u16).to_le_bytes()
            );

        map[offset(target)..offset(target)+2]
            .copy_from_slice(&0x2000u16.to_le_bytes());

        assert!(
            compile_stamps(
                &map,&BTreeSet::new(),&[(source,target)]
            ).is_err()
        );

        map[offset(target)..offset(target)+2].fill(0);
        map[
            0x4000+target[1] as usize*128+target[0] as usize
        ]=5;

        assert!(
            compile_stamps(
                &map,&BTreeSet::new(),&[(source,target)]
            ).is_err()
        );

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_deletion_preserves_neighbors_and_heights()->Result<()> {
        let mut map=vec![0u8;0x12000];
        let a=[4,5];
        let b=[5,5];

        for cell in [a,b] {
            let at=offset(cell);
            map[at..at+2].copy_from_slice(
                &(CELL_WALK_VISIBLE|0x208u16).to_le_bytes()
            );
        }

        map[0x4000+5*128+4]=7;

        let output=compile(&map,&BTreeSet::from([a]))?;

        assert_eq!(
            u16::from_le_bytes(
                output[offset(a)..offset(a)+2].try_into()?
            ),0x208
        );

        assert_eq!(
            output[offset(b)..offset(b)+2],
            map[offset(b)..offset(b)+2]
        );

        assert_eq!(output[0x4000+5*128+4],7);

        Ok(())
    }


    #[test]
    fn visual_delete_survives_retail_initialization()->Result<()>{
        let mut map=vec![0u8;0x12000];
        let deleted=[12u8,13u8];
        let neighbor=[13u8,13u8];
        let id=17usize;
        let base=id*32;

        map[base+0x12..base+0x14]
            .copy_from_slice(&1u16.to_le_bytes());

        map[base+0x16..base+0x18]
            .copy_from_slice(&10u16.to_le_bytes());

        for cell in [deleted,neighbor]{
            let at=offset(cell);
            map[at..at+2].copy_from_slice(
                &(CELL_WALK_VISIBLE|id as u16).to_le_bytes()
            );
        }

        map[0x4000+13*128+12]=0xa7;

        let output=compile(
            &map,&BTreeSet::from([deleted])
        )?;

        assert_eq!(
            &map[0x4000..0x8000],
            &output[0x4000..0x8000]
        );

        assert_eq!(
            &map[base..base+32],
            &output[base..base+32]
        );

        let mut live=output.clone();

        legaia_engine_field::field_regions::refresh_object_grid_marks(
            &mut live
        );

        assert_eq!(
            native_word(&live,deleted)?&CELL_WALK_VISIBLE,0
        );

        assert_ne!(
            native_word(&live,neighbor)?&CELL_WALK_VISIBLE,0
        );

        assert_ne!(
            native_word(&output,deleted)?&0x01ff,
            id as u16
        );

        Ok(())
    }

    #[test]
    fn visual_delete_preserves_exclusive_placement_identity()->Result<()>{
        let mut map=vec![0u8;0x12000];
        let cell=[9u8,9u8];
        let id=20usize;
        let base=id*32;

        map[base+0x12..base+0x14]
            .copy_from_slice(&5u16.to_le_bytes());

        let at=offset(cell);

        map[at..at+2].copy_from_slice(
            &(CELL_WALK_VISIBLE|id as u16).to_le_bytes()
        );

        let deleted=BTreeSet::from([cell]);
        let out=compile(&map,&deleted)?;
        assert_eq!(native_word(&out,cell)?&0x1ff,id as u16);
        assert_eq!(u16::from_le_bytes(out[base+0x12..base+0x14].try_into()?),4);
        assert_eq!(validate_visual_descriptor_delta(&map,&out,&deleted)?,BTreeSet::from([id]));
        let before_spawns=legaia_engine_field::field_regions::window_rebuild_spawns(&map,(0,0,128,128),[0;16]);
        let after_spawns=legaia_engine_field::field_regions::window_rebuild_spawns(&out,(0,0,128,128),[0;16]);
        assert_eq!(format!("{before_spawns:?}"),format!("{after_spawns:?}"));
        let other=offset([10,9]);
        map[other..other+2].copy_from_slice(&(CELL_WALK_VISIBLE|id as u16).to_le_bytes());
        assert!(compile(&map,&deleted).is_err(),"Shared retained surface must not be removed");

        Ok(())
    }

    #[test]
    fn refuses_nonwalkable_source()->Result<()> {
        let map=vec![0u8;0x12000];
        assert!(compile(
            &map,&BTreeSet::from([[2,2]])
        ).is_err());
        Ok(())
    }
}

#[cfg(test)]
mod w2b_unrestricted_tests {
    use super::*;

    #[test]
    fn occupied_cells_allow_experimental_writes()->Result<()>{
        let mut map=vec![0u8;0x12000];
        let source=[19,26];
        let target=[20,26];
        let s=offset(source);
        let t=offset(target);

        map[s..s+2].copy_from_slice(
            &(CELL_WALK_VISIBLE|7).to_le_bytes()
        );
        map[t..t+2].copy_from_slice(
            &0x2008u16.to_le_bytes()
        );

        let (out,notes)=w2b_compile(
            &map,&BTreeSet::new(),&[(source,target)]
        )?;

        assert_ne!(out[t..t+2],map[t..t+2]);
        assert!(native_word(&out,target)?&CELL_WALK_VISIBLE!=0);
        assert!(notes.iter().any(|n|n.contains("OVERWRITE")));
        Ok(())
    }
}

#[cfg(test)]
mod lowering_regression {
    use super::*;
    #[test]
    #[ignore = "Requires LCK_FLOOR_PROJECT, LCK_RETAIL_SOURCE and LCK_FLOOR_OUTPUT"]
    fn saved_project_ground_exports_and_survives_initialization() -> Result<()> {
        let project:serde_json::Value=serde_json::from_slice(&fs::read(std::env::var("LCK_FLOOR_PROJECT")?)?)?;
        let raw=project["cell_snapshots"]["original:town01"].as_str().context("town01 snapshot missing")?;
        let source=std::path::PathBuf::from(std::env::var("LCK_RETAIL_SOURCE")?);
        let output=std::path::PathBuf::from(std::env::var("LCK_FLOOR_OUTPUT")?);
        let library=crate::library::Library::open(&source)?;
        let index=&library.index;
        let scene=Scene::load(index,"town01")?;
        let entry=scene.field_map_index(index).context("MAP missing")? as usize;
        let before=DiscPatcher::open(fs::read(&source)?)?.read_entry_footprint(entry)?;
        let lut=scene.field_floor_height_lut(index)?.context("LUT missing")?;
        let hf=legaia_scene_formats::field_objects::build_walk_heightfield(&before,&lut);
        let edits=super::super::ground_edit_ops::parse(raw,Some(&hf),"town01").map_err(anyhow::Error::msg)?;
        let (deleted,stamps,_)=super::super::ground_edit_ops::w46_plan(&edits).map_err(anyhow::Error::msg)?;
        let replaced=stamps.iter().map(|&(_,c)|c).collect::<BTreeSet<_>>();
        let (bytes,notes)=w2b_compile(&before,&deleted,&stamps)?;
        let (_,detailed,_)=super::super::ground_edit_ops::w46_plan_detailed(&edits).map_err(anyhow::Error::msg)?;
        let (expected,height_notes)=w46_reconcile(&before,bytes,&detailed,&lut)?;
        println!("{}\n{}",notes.join("\n"),height_notes.join("\n"));
        // Exercise the production compiler and raw-sector writer. The test uses
        // create_new directly because this sandbox cannot canonicalize paths.
        let mut disc=DiscPatcher::open(fs::read(&source)?)?;
        disc.patch_prot_entry(entry,0,&expected)?;
        let mut file=fs::OpenOptions::new().write(true).create_new(true).open(&output)?;
        file.write_all(&disc.into_image())?;file.sync_all()?;drop(file);
        let after=DiscPatcher::open(fs::read(&output)?)?.read_entry_footprint(entry)?;
        ensure!(after==expected,"Physical IMG MAP differs from compiled ground");
        ensure!(after!=before,"Ground exporter returned unchanged MAP");
        let mut runtime=after.clone();
        legaia_engine_field::field_regions::refresh_object_grid_marks(&mut runtime);
        for c in deleted.difference(&replaced) {
            ensure!(native_word(&runtime,*c)?&CELL_WALK_VISIBLE==0,"Deleted floor {c:?} returned at initialization");
        }
        for c in &replaced {
            ensure!(native_word(&runtime,*c)?&CELL_WALK_VISIBLE!=0,"Replacement {c:?} lost walking surface at initialization");
        }
        let old_override=replaced.iter().filter(|&&c|native_word(&runtime,c).unwrap()&0x800!=0).count();
        println!("Deleted {} / replacement {} cells checked; {} retained height overrides",deleted.len(),replaced.len(),old_override);
        ensure!(old_override==0,"Replacement retains an old elevation override");
        // Execute the original retail height routine against the exported MAP.
        let exe=library.iso_files.iter().find(|f|f.path.eq_ignore_ascii_case("SCUS_942.54")).context("SCUS executable required")?;
        let bytes=library.iso_file_bytes(exe)?;
        let base=u32::from_le_bytes(bytes[0x18..0x1c].try_into()?);
        let mut cpu=legaia_code_hooks::mips_sim::Cpu::new();
        cpu.load(base,&bytes[0x800..]);cpu.load(0x801ce818,&index.entry_bytes(897)?);
        const MAP:u32=0x80100000;const ACTOR:u32=0x801b0000;const STOP:u32=0x80000080;
        cpu.load(MAP,&runtime);cpu.wr32(0x1f8003ec,MAP);cpu.load(0x8007b318,&[4,4,4,8]);
        for (i,&y) in lut.iter().enumerate(){cpu.wr16(0x1f80035c+i as u32*2,y as u16);}
        let mut checked=0;
        for &cell in &replaced {
            if cell[0]==127||cell[1]==127 {continue;}
            let h=four_tiers(&runtime,cell)?.map(|i|lut[i as usize] as i32);
            for (dx,dz) in [(32,32),(96,32),(32,96),(96,96)] {
                let expected=(h[0]*(128-dx)*(128-dz)+h[1]*dx*(128-dz)+h[2]*(128-dx)*dz+h[3]*dx*dz)>>14;
                cpu.r=[0;32];cpu.r[4]=ACTOR;cpu.r[28]=0x8007b318;cpu.r[29]=0x801bff00;cpu.r[31]=STOP;
                cpu.wr32(ACTOR+0x10,0);cpu.wr16(ACTOR+0x14,cell[0] as u16*128+dx as u16);
                cpu.wr16(ACTOR+0x18,cell[1] as u16*128+dz as u16);
                cpu.pc=0x80019278;cpu.steps=0;cpu.run_until(&[STOP]);
                ensure!((cpu.r[2] as i32-expected).abs()<=1,"Retail height differs at {cell:?}: {} vs {expected}",cpu.r[2] as i32);
                checked+=1;
            }
        }
        println!("Retail MIPS height routine matched {checked} exported floor samples");
        Ok(())
    }
}

// R6.0 ground regression. UID 105 and descriptor 290 are examples,
// not special-cased rules. The allocator works in arbitrary cells.
#[cfg(test)]
mod lck_r60_ground_tests {
    use super::*;

    #[test]
    fn deleted_placed_105_does_not_reown_floor_290()->Result<()> {
        let donor_cell=[31u8,31u8];
        let target=[37u8,37u8];

        let mut retail=vec![0u8;0x12000];

        let old_id=290usize;
        let rec=old_id*32;

        retail[rec+18..rec+20]
            .copy_from_slice(&5u16.to_le_bytes());

        retail[rec+20..rec+24]
            .copy_from_slice(&[7,26,10,0]);

        retail[offset(donor_cell)..offset(donor_cell)+2]
            .copy_from_slice(
                &(CELL_WALK_VISIBLE|old_id as u16).to_le_bytes()
            );

        let old_target=CELL_WALK_VISIBLE|0x0800u16|7u16;

        retail[offset(target)..offset(target)+2]
            .copy_from_slice(&old_target.to_le_bytes());

        // Simulate object export removing the source placement.
        let mut working=retail.clone();
        working[rec..rec+32].fill(0);
        working[offset(donor_cell)..offset(donor_cell)+2].fill(0);

        let deleted=BTreeSet::from([target]);
        let stamps=vec![(donor_cell,target)];

        let (out,_)=w2b_compile_dual(
            &retail,&working,&deleted,&stamps
        )?;

        let edited=super::super::ground_edit_ops::W46Stamp {
            source:donor_cell,
            target,
            transform:super::super::ground_edit_ops::Transform::default(),
            uid:Some(105)
        };

        let (out,_)=w46_reconcile_dual(
            &retail,&working,out,&[edited],&[0i16;16]
        )?;

        let id=(native_word(&out,target)?&511) as usize;

        ensure!(
            id!=old_id,
            "Replacement floor reused deleted object descriptor 290"
        );

        ensure!(
            out[rec..rec+32]==working[rec..rec+32],
            "Deleted object descriptor was restored"
        );

        ensure!(
            u16::from_le_bytes(
                out[id*32+18..id*32+20].try_into()?
            )==1,
            "Ground-only record retained placed-object flags"
        );

        verify_runtime_surfaces(&out,&deleted,&stamps)?;

        Ok(())
    }
}