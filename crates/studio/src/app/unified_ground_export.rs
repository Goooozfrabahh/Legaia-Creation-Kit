//! U1B.2 - Compose native object, ground and collision edits
//! into one output IMG without writing over the source image.
//!
//! Native objects are compiled first, native walk-ground second,
//! and C1 collision-wall bits last.
//!
//! Only exact grid-aligned ground transformations are supported
//! in mixed mode. W2B approximate rasterization remains available
//! through the existing floor-only experimental exporter.

use anyhow::{Context,Result,ensure};
use std::{
    collections::BTreeSet,
    fs,
    path::{Path,PathBuf},
    time::{SystemTime,UNIX_EPOCH},
};

use legaia_disc_patch::disc::DiscPatcher;
use legaia_engine_scene::{
    scene::{Scene,ProtIndex},
    scene_assembly::AssembledScene,
};

use legaia_scene_formats::field_objects::CELL_WALK_VISIBLE;

use super::{
    collision_edit,
    form_bridge,
    ground_edit_ops,
    native_ground_delete,
    ps1_export,
    world_view,
};

type Cell=[u8;2];

struct StageFiles {
    paths:Vec<PathBuf>,
}

impl StageFiles {
    fn new(object:PathBuf,ground:PathBuf)->Self {
        Self {
            paths:vec![
                object.clone(),
                object.with_extension("lckbuilding"),
                object.with_extension("json"),
                ground.clone(),
                ground.with_extension("lck_w1_building"),
                ground.with_extension("u1a_collision_overlaying"),
                ground.with_extension("u1a_pre_collision_img"),
            ],
        }
    }
}

impl Drop for StageFiles {
    fn drop(&mut self){
        for path in &self.paths{
            let _=fs::remove_file(path);
        }
    }
}

fn map_from_image(
    image:&Path,
    entry:usize
)->Result<Vec<u8>>{
    let bytes=fs::read(image)
        .with_context(||format!("Read {}",image.display()))?;

    let disc=DiscPatcher::open(bytes)?;
    Ok(disc.read_entry_footprint(entry)?)
}

fn offset(cell:Cell)->usize{
    0x8000+(cell[1] as usize*128+cell[0] as usize)*2
}

/// A native floor writer may modify only grid words owned by the
/// exact floor plan. It may not overwrite placed-object records,
/// scripts, wall nibbles, unrelated grid entries or MAP metadata.
fn verify_ground_delta(
    before:&[u8],
    after:&[u8],
    owned:&BTreeSet<Cell>,
)->Result<()>{
    ensure!(
        before.len()==after.len()&&before.len()>=0x12000,
        "Native MAP footprint changed or is incomplete"
    );

    let isolated=native_ground_delete::validate_visual_descriptor_delta(
        before,after,owned
    )?;

    for (i,(&a,&b)) in before.iter().zip(after.iter()).enumerate(){
        if a==b {continue;}
        if i<0x4000 && isolated.contains(&(i/32)){
            continue;
        }

        ensure!(
            (0x8000..0x10000).contains(&i),
            "Ground writer changed a non-grid MAP byte at {i:#x}"
        );

        let word_index=(i-0x8000)/2;
        let cell=[
            (word_index%128) as u8,
            (word_index/128) as u8,
        ];

        ensure!(
            owned.contains(&cell),
            "Ground writer modified unrelated cell {:?}",
            cell
        );
    }

    // The only permitted effects are native floor record selection,
    // the walkable flag, and the existing grid's upper 0x8000 bit.
    // Other object/actor/decoration flags must remain untouched.
    for &cell in owned{
        let at=offset(cell);
        let before_word=u16::from_le_bytes(
            before[at..at+2].try_into()?
        );

        let after_word=u16::from_le_bytes(
            after[at..at+2].try_into()?
        );

        let allowed=0x01ffu16|CELL_WALK_VISIBLE|0x8000;

        ensure!(
            (before_word^after_word)&!allowed==0,
            "Ground writer would overwrite non-ground flags at {:?}",
            cell
        );
    }

    ensure!(
        before[0x4000..0x8000]==after[0x4000..0x8000],
        "Ground writer unexpectedly changed wall or height bytes"
    );

    Ok(())
}

/// After C1 collision application, all ground tiers, native
/// object references, model records and other MAP contents must
/// still equal the pre-collision image.
fn verify_collision_delta(
    before:&[u8],
    after:&[u8],
)->Result<()>{
    ensure!(
        before.len()==after.len()&&before.len()>=0x12000,
        "Collision overlay changed MAP footprint"
    );

    for (i,(&a,&b)) in before.iter().zip(after.iter()).enumerate(){
        if a==b {continue;}

        ensure!(
            (0x4000..0x8000).contains(&i),
            "Collision overlay changed a non-wall MAP byte at {i:#x}"
        );

        ensure!(
            a&0x0f==b&0x0f,
            "Collision overlay modified floor tier at {i:#x}"
        );
    }

    Ok(())
}

fn validate_ground(
    floors:&ground_edit_ops::State
)->Result<(BTreeSet<Cell>,Vec<(Cell,Cell)>)>{
    // A quarter-turn changes visible UV orientation, which the
    // retail floor-grid stamp currently does not serialize.
    for transform in floors.originals.values()
        .chain(floors.copies.iter().map(|c|&c.transform))
    {
        ensure!(
            transform.turns.rem_euclid(4)==0,
            "U1B.2 cannot yet export rotated floor textures in a mixed IMG"
        );
    }

    ground_edit_ops::native_128_plan(floors)
        .map_err(anyhow::Error::msg)
}

/// A staged, transactional mixed export. Existing output files
/// and the source image are never replaced.
pub(super) fn execute(
    source:&Path,
    output:&Path,
    scene:&AssembledScene,
    world:&world_view::WorldModel,
    index:&ProtIndex,
    floors:&ground_edit_ops::State,
    collision:&collision_edit::State,
    object_edits:bool,
    unrestricted:bool,
    mut session:Option<form_bridge::Session>,
)->Result<ps1_export::ExportResult>{
    ensure!(
        source!=output,
        "Cannot overwrite the source IMG"
    );
    ensure!(
        !output.exists(),
        "Choose a NEW output IMG filename"
    );
    ensure!(
        output.parent().is_some_and(|p|p.is_dir()),
        "Output directory unavailable"
    );
    ensure!(floors.has_edits(),"No native ground edits requested");
    ensure!(
        floors.scene.is_empty()||floors.scene==scene.name,
        "Ground edits belong to a different scene"
    );

    // U1B.2 currently supports exact 128-unit native ground
    // changes only. No arbitrary UV/height approximations.
    let (removed,stamps)=validate_ground(floors)?;

    let mut touched=removed.clone();

    for &(_,destination) in &stamps {
        touched.insert(destination);
    }

    let stamp=SystemTime::now()
        .duration_since(UNIX_EPOCH)?
        .as_nanos();

    let stem=output.file_stem()
        .and_then(|s|s.to_str())
        .unwrap_or("LCK");

    let dir=output.parent().context("Missing output folder")?;

    let stage_object=dir.join(format!(
        "LCK_EXPERIMENTAL_U1B2_{stem}_{}_{}__object.img",
        std::process::id(),stamp
    ));

    let stage_ground=dir.join(format!(
        "LCK_EXPERIMENTAL_U1B2_{stem}_{}_{}__ground.img",
        std::process::id(),stamp
    ));

    ensure!(
        !stage_object.exists()&&!stage_ground.exists(),
        "U1B.2 staging file already exists"
    );

    let _cleanup=StageFiles::new(
        stage_object.clone(),
        stage_ground.clone()
    );

    let base=if object_edits{
        // Native object and resource edits are compiled from
        // the original source before ground changes are applied.
        let plan=if let Some(project)=session.as_mut(){
            ps1_export::prepare_project(
                source,scene,project,index,unrestricted
            )?
        }else if unrestricted {
            ps1_export::unrestricted::prepare(
                source,scene,world,index
            )?
        }else{
            ps1_export::plan_export(
                source,scene,world,index
            )?
        };

        let result=ps1_export::export(
            plan,&stage_object
        )?;

        Some(result)
    }else{
        None
    };

    // Continuation receipts depend on the exact IMG hash and
    // cannot simply be copied after another patch changes it.
    ensure!(
        !stage_object.with_extension("json").exists(),
        "Object export requires a continuation receipt. U1B.2 cannot yet regenerate it after ground edits. No final IMG published."
    );

    let base_path=base.as_ref()
        .map(|r|r.path.as_path())
        .unwrap_or(source);

    let native=Scene::load(index,&scene.name)?;
    let entry=native.field_map_index(index)
        .context("Selected scene has no native MAP")?
        as usize;

    let before_ground=map_from_image(base_path,entry)?;

    // Use the existing tested native-ground writer. Its exact
    // mode refuses unsupported overlaps and floor geometry.
    let ground_note=native_ground_delete::export_ground(
        base_path,
        &stage_ground,
        &scene.name,
        index,
        floors,
        false,
    )?;

    let after_ground=map_from_image(&stage_ground,entry)?;

    verify_ground_delta(
        &before_ground,
        &after_ground,
        &touched,
    )?;

    let collision_note=if collision.has_edits(){
        let note=collision.overlay_existing(
            &stage_ground,
            &scene.name,
            index
        )?;

        let after_collision=map_from_image(
            &stage_ground,entry
        )?;

        verify_collision_delta(
            &after_ground,
            &after_collision
        )?;

        Some(note)
    }else{
        None
    };

    let expected_size=fs::metadata(source)?.len();

    ps1_export::verify_output_file(
        &stage_ground,
        Some(expected_size)
    )?;

    // Publish only after object, ground and collision
    // serialization have all passed their checks.
    fs::hard_link(&stage_ground,output)
        .context("Publish verified U1B.2 IMG")?;

    let published=(||->Result<()>{
        ps1_export::verify_output_file(
            output,Some(expected_size)
        )?;

        ensure!(
            map_from_image(output,entry)?
                ==map_from_image(&stage_ground,entry)?,
            "Published U1B.2 MAP differs from staged MAP"
        );

        Ok(())
    })();

    if let Err(error)=published{
        let _=fs::remove_file(output);
        return Err(error);
    }

    let mut patched=base.as_ref()
        .map(|r|r.patched.clone())
        .unwrap_or_default();

    let skipped=base.as_ref()
        .map(|r|r.skipped.clone())
        .unwrap_or_default();

    let imports=base.as_ref()
        .and_then(|r|r.imports);

    patched.push(format!(
        "U1B.2 native ground: {} removed source cells; {} destination stamps; exact grid writer",
        removed.len(),stamps.len()
    ));

    patched.push(ground_note);

    if let Some(note)=collision_note{
        patched.push(note);
    }

    patched.push(
        "U1B.2 mixed MAP readback verified; DuckStation gameplay NOT verified".into()
    );

    Ok(ps1_export::ExportResult{
        path:output.to_path_buf(),
        patched,
        skipped,
        imports,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_ground_changes_are_owned(){
        let before=vec![0u8;0x12000];
        let mut after=before.clone();

        let cell=[3u8,4u8];
        let at=offset(cell);

        // Add exactly one walkable-ground flag.
        after[at+1]=0x10;

        let mut owned=BTreeSet::new();
        owned.insert(cell);

        verify_ground_delta(&before,&after,&owned).unwrap();

        // Unrelated object/record edits must be rejected.
        after[32]=1;
        assert!(
            verify_ground_delta(&before,&after,&owned).is_err()
        );
    }

    #[test]
    fn ground_writer_cannot_erase_wall_bits(){
        let mut before=vec![0u8;0x12000];
        before[0x4000]=0xa7;

        let mut after=before.clone();
        after[0x4000]=0x07;

        let mut owned=BTreeSet::new();
        owned.insert([1,2]);

        assert!(
            verify_ground_delta(&before,&after,&owned).is_err()
        );
    }

    #[test]
    fn collision_overlay_preserves_ground_tier(){
        let mut before=vec![0u8;0x12000];
        before[0x4000]=0x07;

        let mut after=before.clone();
        after[0x4000]=0x27;

        verify_collision_delta(&before,&after).unwrap();

        after[0x4000]=0x28;

        assert!(
            verify_collision_delta(&before,&after).is_err()
        );
    }

    #[test]
    fn collision_overlay_cannot_restore_object(){
        let before=vec![0u8;0x12000];
        let mut after=before.clone();

        after[0x8000]=1;

        assert!(
            verify_collision_delta(&before,&after).is_err()
        );
    }
}
