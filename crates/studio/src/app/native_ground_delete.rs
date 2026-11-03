
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
            ensure!(
                flags&0x0004==0,
                "Tile {:?} references a placed object. \
                 Actor-aware floor isolation is not implemented.",
                cell
            );

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


// Unrestricted native projection. Occupied cells use last-wins
// semantics. Exact source geometry is NOT serialized.
fn w2b_compile(map:&[u8],deleted:&BTreeSet<Cell>,stamps:&[(Cell,Cell)])
    ->Result<(Vec<u8>,Vec<String>)>{

    ensure!(map.len()>=0x12000,"Invalid native MAP size");
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
        let before=native_word(map,source)?;
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
            let tiers=four_tiers(map,source)?;
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

    let allocated=visual_delete(
        map,&mut out,deleted,&replaced
    )?;

    if !allocated.is_empty(){
        notes.push(format!(
            "VISUAL FLOOR DELETE: {} isolated descriptors",
            allocated.len()
        ));
    }

    Ok((out,notes))
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
        super::ground_edit_ops::w2b_plan(edits).map_err(anyhow::Error::msg)?
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

    // Preserve exact walking heights on resized native floors.
    if !unrestricted {
        for source in super::ground_edit_ops::native_resized_sources(edits) {
            let tiers=four_tiers(&before,source)?;
            ensure!(tiers.iter().all(|&t|t==tiers[0]),
                "Resized source tile {:?} is sloped; extended native height support required",source);
        }
    }
    let after=if unrestricted {
        let (bytes,extra)=w2b_compile(&before,&deleted,&stamps)?;
        notes.extend(extra);
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
            "NOT EXACT: partial cells become full 128-unit cells; Y offsets ignored; overlapping edits overwrite cells; textures repeat.\n"
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
    fn visual_delete_rejects_placed_actor_dependency()->Result<()>{
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

        assert!(
            compile(&map,&BTreeSet::from([cell])).is_err()
        );

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
