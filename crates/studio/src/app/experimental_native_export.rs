//! Explicit visual-only experiment. Never constructs an accepted production
//! NativeExportPlan and never runs automatically when production preflight fails.
use super::*;
use super::super::native_asset_plan::{scene_registration::{self,Limits},runtime_evidence::VramObservations};
use super::super::collision_recipes::{self,Evidence,ObjectId};

pub(in crate::app) const ACK:&str="VISUAL_ONLY_UNVERIFIED_PHYSICS_AND_RUNTIME";

pub(in crate::app) fn export_visual(
    source:&Path,output:&Path,scene:&AssembledScene,world:&WorldModel,index:&ProtIndex,
    observations:&VramObservations,ack:&str,
)->Result<()> {
    ensure!(ack==ACK,"Explicit visual-only/runtime-and-physics acknowledgement required");
    ensure!(output.is_absolute()&&output.file_name().and_then(|p|p.to_str()).is_some_and(|n|n.contains("EXPERIMENTAL_VISUAL_ONLY")&&n.ends_with(".img")),"Experimental IMG requires an absolute, explicitly labelled new filename");
    let parent=output.parent().context("Experimental output parent missing")?;
    ensure!(!parent.exists()&&parent.parent().is_some_and(|p|p.is_dir()),"Experimental artifact directory must be NEW under an existing directory");
    ensure!(observations.sources.len()>=2,"At least two bounded VRAM observations required; these do not certify freedom");
    let text=scene_instances::serialize(&scene.name,world);
    let doc=scene_instances::parse(&text,&scene.name,world).map_err(anyhow::Error::msg)?;
    ensure!(!doc.foreign.is_empty()&&doc.mesh_collision.is_empty()&&doc.collision_recipes.len()==doc.foreign.len(),"Visual experiment requires retained native-source collision proposals for every foreign instance");
    for foreign in &doc.foreign {
        ensure!(doc.collision_recipes.iter().any(|a|a.owner==ObjectId::Created(foreign.uid)&&matches!(a.recipe.evidence,Evidence::MeshDerived{..})),"Visual experiment must not bypass reviewed or other authored collision intent");
    }
    let source=source.canonicalize()?;let source_bytes=std::fs::read(&source)?;
    let source_hash=collision_recipes::digest(&source_bytes);
    let disc=DiscPatcher::open(source_bytes)?;validate_source_archive(&disc,index)?;
    let native=Scene::load(index,&scene.name)?;let map_entry=native.field_map_index(index).context("No native destination MAP")? as usize;
    let original_map=disc.read_entry_footprint(map_entry)?;
    // Structural ceilings only: whole PS1 RAM is NOT available heap headroom.
    // Normal production acceptance is intentionally absent from this path.
    let limits=Limits{max_models:scene_registration::FIRST_KNOWN_MODEL_CONFLICT-field_objects::FIELD_ACTOR_PACK_BIAS as usize,max_decoded:2*1024*1024,max_encoded:0x62c00};
    let old_bundle=disc.read_entry_footprint(map_entry+3)?;
    let old_pac=if scene_asset_table::streams_scene_pac(&old_bundle){Some(disc.read_entry_footprint(map_entry+4)?)}else{None};
    let (old_textures,_)=scene_registration::native_texture_sequence(&old_bundle,old_pac.as_deref(),limits)?;
    let candidate=NativeExportPlan::compile_saved_imports_observed(disc,scene,world,index,limits,&[],Some(observations))?;
    ensure!(candidate.native_walls.is_none()&&candidate.pending_collision_recipes==doc.collision_recipes.len(),"Visual experiment cannot hide compiled or lost collision intent");
    let map=candidate.native_map(map_entry)?;
    ensure!(map[WALL_OFF..GRID_OFF]==original_map[WALL_OFF..GRID_OFF]&&map[0x10000..]==original_map[0x10000..],"Visual experiment changed original walls/floors/bindings");
    let bundle=candidate.native_map(map_entry+3)?;
    let pac=if scene_asset_table::streams_scene_pac(&bundle){Some(candidate.native_map(map_entry+4)?)}else{None};
    let (textures,_)=scene_registration::native_texture_sequence(&bundle,pac.as_deref(),limits)?;
    let mut uploads=Vec::new();
    for raw in textures.iter().filter(|raw|!old_textures.contains(raw)) {
        let tim=legaia_tim::parse_native_upload(raw)?;
        let rect=super::super::native_asset_plan::texture_relocation::Rect{x:tim.image.fb_x,y:tim.image.fb_y,w:tim.image.fb_w,h:tim.image.h};
        ensure!(!observations.intersects(rect)?,"Experimental new upload overlaps observed runtime activity");
        uploads.push(serde_json::json!({"image_word_rect":[rect.x,rect.y,rect.w,rect.h],"flags":tim.flags,"sha256":collision_recipes::digest(raw)}));
    }
    let mut instances=Vec::new();let lut=native.field_floor_height_lut(index)?.context("Native floor ladder missing")?;
    for &(uid,id,cell,selector) in &candidate.placements {
        let record=ObjectRecord::parse(&map,id).context("Experimental descriptor missing")?;
        let tier=(map[0x4000+cell.1 as usize*128+cell.0 as usize]&15) as usize;
        let foreign=world.foreign.iter().find(|f|f.uid==uid).context("Experimental source identity lost")?;
        instances.push(serde_json::json!({"uid":uid,"source_cell":foreign.source,"source_draw":foreign.record,"native_source":world.collision.foreign_sources.get(&uid),"descriptor":id,"spawn_cell":[cell.0,cell.1],"selector":selector,
            "psx_world_position":[field_objects::world_x(cell.0,record.x_off),-(lut[tier] as i32)+record.y_off as i32,field_objects::world_z(cell.1,record.z_off)],
            "record_local_collision_bytes":"copied from real donor; full physical contract unverified"}));
    }
    let image_hash=collision_recipes::digest(candidate.image_bytes());
    let manifest=serde_json::json!({"schema":"lck-experimental-visual-v1","visual_only":true,"production_export_approved":false,"playability_verified":false,
        "source_image":source,"source_sha256":source_hash,"output_image":output,"output_sha256":image_hash,"destination":scene.name,"map_entry":map_entry,
        "instances":instances,"new_uploads":uploads,"observation_sources":observations.sources,"observation_scope":observations.summary(),
        "pending_collision_recipes":doc.collision_recipes.len(),"native_wall_and_floor_grid_unchanged":true,
        "unverified_requirements":["heap allocation-time headroom, fragmentation, actor and primitive allocations","registry extent below known index-200 conflict","complete dynamic/shared VRAM lifetime","native actor collider class/extents/enabled state","walkable surfaces and conditional script/animation behavior"],
        "explicit_visual_scope":"Collision proposals are preserved in the sidecar scene but are NOT approved or compiled as physical collision. This is an opt-in rendering/registration experiment, not a functional-object or production-export claim.",
        "sidecar_scene_source":"Open the sidecar against the original source image to review pending recipes; the exported IMG already contains native instances and has a different native layout.",
        "test_instructions":"Cold boot this new IMG. Do not restore a captured full save state, which would restore old RAM/resources. Enter the selected cell through its native loader. Do not use this image for normal play or save progression.",
        "diagnostics":candidate.collision_diagnostics});
    std::fs::create_dir(parent)?;
    let part=output.with_extension("img.part");
    use std::io::Write;
    let mut file=std::fs::OpenOptions::new().write(true).create_new(true).open(&part)?;
    file.write_all(candidate.image_bytes())?;file.sync_all()?;drop(file);
    let disk=std::fs::read(&part)?;ensure!(collision_recipes::digest(&disk)==image_hash,"Experimental on-disk image hash mismatch");
    let check=DiscPatcher::open(disk)?;
    ensure!(check.read_entry_footprint(map_entry)?==map&&check.read_entry_footprint(map_entry+3)?==bundle,"Experimental native image readback mismatch");
    if let Some(expected)=&pac {ensure!(check.read_entry_footprint(map_entry+4)?==*expected,"Experimental PAC readback mismatch");}
    for (path,bytes) in [(output.with_extension("lckscene"),text.into_bytes()),(output.with_extension("json"),serde_json::to_vec_pretty(&manifest)?)] {
        let mut f=std::fs::OpenOptions::new().write(true).create_new(true).open(path)?;f.write_all(&bytes)?;f.sync_all()?;
    }
    // Create-only publication: a hard link cannot overwrite an existing file.
    // No guest/executable patch or accepted production plan is involved.
    std::fs::hard_link(&part,output)?;std::fs::remove_file(&part)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn visual_experiment_requires_ack_before_reading_or_writing_any_image() {
        let (scene,world)=super::super::tests::actor_fixture();let index=ProtIndex::from_bytes(legaia_disc_patch::disc::synth::synth_prot(&[]),None).unwrap();
        let bytes=vec![0;1024*512*2];let observed=VramObservations::from_raw(&[("fixture",&bytes)]).unwrap();
        let error=export_visual(Path::new("missing.img"),Path::new("missing.img"),&scene,&world,&index,&observed,"").unwrap_err();
        assert!(error.to_string().contains("acknowledgement"));
    }
}
