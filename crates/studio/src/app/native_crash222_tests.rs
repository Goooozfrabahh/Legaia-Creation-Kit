use super::*;
use crate::app::native_asset_plan::{scene_registration as reg,texture_relocation};
use serde_json::json;
use crate::app::collision_recipes::digest;

const PARENT:&str=r"D:\AstraRimElmProject\ManualTests\Native_Format_Adapters_20261028_01\LCK_EXPERIMENTAL_MIXED_FORMATS_FIXED.img";
const RECOVERED:&str=r"D:\AstraRimElmProject\ManualTests\Runtime_Compatibility_20261029_02\LCK_EXPERIMENTAL_MIXED_RUNTIME_CANDIDATE.img";
const FAILED:&str=r"D:\AstraRimElmProject\222.img";

#[test]
#[ignore="exact historical wall-write reconstruction and full user collision recovery; no emulator"]
fn full_scene_owned_wall_relocation_candidate()->Result<()>{
    use crate::app::native_wall_journal::{self as walls,Journal,Reference};
    use crate::app::collision_recipes::ObjectId;
    let output=PathBuf::from(std::env::var_os("LCK_OWNED_WALL_IMG").context("new collision candidate")?);ensure!(!output.exists(),"Use a new IMG path");
    let root=Path::new(r"D:\AstraRimElmProject\ManualTests\RuntimeRecovery_20261030_01");let source=root.join("R7_FULL_SCENE_DELETED_RESOURCE_RETIREMENT.img");
    let baseline=r"D:\AstraRimElmProject\ManualTests\M2_StageE_20261027_01\LCK_EXPERIMENTAL_STAGE_E_town01_floor_x37_z18.img";
    let legacy_log=std::fs::read_to_string(r"D:\AstraRimElmProject\ManualTests\Native_Format_Adapters_20261028_01\LCK_EXPERIMENTAL_MIXED_FORMATS_FIXED_export_1793237710611489900\debug.txt")?;
    ensure!(legacy_log.contains(&digest(&std::fs::read(PARENT)?)),"Historical log/image hash mismatch");
    let start=legacy_log.find("LCK_SCENE_V1\t").context("Legacy manifest")?;let manifest=legacy_log[start..].lines().take_while(|l|!l.is_empty()).collect::<Vec<_>>().join("\n");
    let base_lib=crate::library::Library::open(Path::new(baseline))?;let base_scene=legaia_engine_world_views::scene_staging::assemble_field_scene(&base_lib.index,"town01").map_err(anyhow::Error::msg)?;
    let mut legacy_world=world_view::build_world(&base_scene,&base_lib.index);let old_doc=scene_instances::parse(&manifest,"town01",&legacy_world).map_err(anyhow::Error::msg)?;
    scene_instances::apply(&old_doc,&mut legacy_world,base_scene.placements.len(),&base_lib.index).map_err(anyhow::Error::msg)?;
    let parent=crate::library::Library::open(Path::new(PARENT))?;let scene=legaia_engine_world_views::scene_staging::assemble_field_scene(&parent.index,"town01").map_err(anyhow::Error::msg)?;
    let mut full_world=world_view::build_world(&scene,&parent.index);let full_text=std::fs::read_to_string(r"D:\AstraRimElmProject\ManualTests\Crash222_20261029_01\222-full-scene-preserved.lckscene")?;
    let full=scene_instances::parse(&full_text,"town01",&full_world).map_err(anyhow::Error::msg)?;scene_instances::apply(&full,&mut full_world,scene.placements.len(),&parent.index).map_err(anyhow::Error::msg)?;
    let entry=Scene::load(&parent.index,"town01")?.field_map_index(&parent.index).context("MAP")?as usize;
    let base=DiscPatcher::open(std::fs::read(baseline)?)?;let original=DiscPatcher::open(std::fs::read(&source)?)?;let parent_disc=DiscPatcher::open(std::fs::read(PARENT)?)?;
    let base_map=base.read_entry_footprint(entry)?;let parent_map=parent_disc.read_entry_footprint(entry)?;let source_map=original.read_entry_footprint(entry)?;
    ensure!(source_map==DiscPatcher::open(std::fs::read(FAILED)?)?.read_entry_footprint(entry)?,"Current full MAP differs from recorded full export");
    let binding=|text:&str,label:&str|->Result<(usize,(u8,u8))>{let line=text.lines().find(|l|l.contains(label)&&l.contains("Independent descriptor ")).context("Recorded native allocation missing")?;let tail=line.split_once("Independent descriptor ").unwrap().1;let id=tail.split(',').next().unwrap().parse::<usize>()?;let coords=tail.split_once("reference (").context("Recorded reference")?.1.split(')').next().unwrap().split(',').map(|s|s.trim().parse::<u8>()).collect::<std::result::Result<Vec<_>,_>>()?;ensure!(coords.len()==2,"Reference coordinate count");Ok((id,(coords[0],coords[1])))};
    let r7_bundle=original.read_entry_footprint(entry+3)?;let r7_pac=original.read_entry_footprint(entry+4)?;let r7_models=reg::native_model_pool(&r7_bundle,Some(&r7_pac));
    let witness=|map:&[u8],id:usize,cell:(u8,u8)|->Result<Reference>{ensure!(id_at(map,cell)?==id,"Historical instance reference does not match actual MAP");let rec=ObjectRecord::parse(map,id).context("Instance descriptor")?;let index=field_objects::pack_mesh_index(id as u16,&rec).context("Instance model")?;Ok(Reference{cell:[cell.0,cell.1],descriptor:id as u16,model_selector:index,descriptor_sha256:digest(&map[id*32..id*32+32]),model_sha256:digest(&r7_models.get(index as usize).context("Instance native model")?.bytes)})};
    let mut layers=vec![];let mut legacy_refs=std::collections::BTreeMap::new();
    for assignment in &old_doc.collision_recipes{let ObjectId::Created(uid)=assignment.owner else{bail!("Unexpected legacy native recipe")};let draw=legacy_world.draws.iter().find(|d|d.editor_uid==Some(uid)).context("Legacy collision draw")?;
        let mut owner=walls::serialized_owner(format!("legacy-import-{uid}"),&assignment.recipe,draw)?;let(id,cell)=binding(&legacy_log,&format!("IMPORT UID {uid} "))?;ensure!(id_at(&parent_map,cell)?==id,"Legacy reference changed");legacy_refs.insert(uid,(id,cell));
        if id_at(&source_map,cell)?==id{owner.native=Some(witness(&source_map,id,cell)?);}layers.push(owner);
    }
    Journal::reconstruct("town01".into(),entry as u32,&base_map,&parent_map,layers.iter().cloned().map(|mut o|{o.native=None;o}).collect())?;
    let full_log=std::fs::read_to_string(r"D:\AstraRimElmProject\222_export_1793316316530331500\debug.txt")?;
    for assignment in &full.collision_recipes{let ObjectId::Created(uid)=assignment.owner else{bail!("Unexpected full native recipe")};let draw=full_world.draws.iter().find(|d|d.editor_uid==Some(uid)).context("Full collision draw")?;let mut owner=walls::serialized_owner(format!("current-created-{uid}"),&assignment.recipe,draw)?;
        let label=if full.created.iter().any(|n|n.uid==uid){format!("DUPLICATE UID {uid} ")}else{format!("IMPORT UID {uid} ")};let(id,cell)=binding(&full_log,&label)?;owner.native=Some(witness(&source_map,id,cell)?);layers.push(owner);
    }
    let journal=Journal::reconstruct("town01".into(),entry as u32,&base_map,&source_map,layers)?;
    let current_lib=crate::library::Library::open(&source)?;let current_scene=legaia_engine_world_views::scene_staging::assemble_field_scene(&current_lib.index,"town01").map_err(anyhow::Error::msg)?;let current_world=world_view::build_world(&current_scene,&current_lib.index);
    let mut changes=vec![];let mut moved=vec![];
    for(uid,(id,cell))in legacy_refs{
        let source_key=scene.placements.iter().position(|p|p.cell==cell).map(|i|(false,i)).context("Historical native source")?;
        let label=format!("legacy-import-{uid}");
        if full.hidden.contains(&source_key){ensure!(id_at(&source_map,cell)?!=id,"Requested deletion still has native reference");changes.push((label,None));continue;}
        let changed=full.moves.iter().any(|(k,_)|*k==source_key)||full.rotations.iter().any(|(k,_)|*k==scene_instances::NameTarget::Native(source_key.0,source_key.1))||full.scales.iter().any(|(k,_)|*k==scene_instances::NameTarget::Native(source_key.0,source_key.1));
        if changed{let key=(false,current_scene.placements.iter().position(|p|p.cell==cell).context("Moved native reference")?);let recipe=current_world.automatic_collision_recipe(key).map_err(anyhow::Error::msg)?;let draw=current_world.at(key.0,key.1).context("Moved native draw")?;
            let mut owner=walls::serialized_owner(label.clone(),&recipe,draw)?;owner.native=Some(witness(&source_map,id,cell)?);moved.push(json!({"owner":label,"native_reference":owner.native,"new_center":draw.center,"authored_wall_cells":owner.blocked.len()}));changes.push((label,Some(owner)));
        }
    }
    let (map,next)=journal.revise(&source_map,&changes)?;
    let before_owners=journal.owners.iter().filter(|o|o.id.starts_with("current-created-")).collect::<Vec<_>>();let after_owners=next.owners.iter().filter(|o|o.id.starts_with("current-created-")).collect::<Vec<_>>();ensure!(before_owners==after_owners,"Duplicate/new import collision contribution changed");
    let mut removed=0;let mut added=0;for i in 0..map.len(){if(0x4000..0x8000).contains(&i){ensure!(map[i]&15==source_map[i]&15,"Floor nibble changed");removed+=(source_map[i]&!map[i]).count_ones();added+=(map[i]&!source_map[i]).count_ones();}else{ensure!(map[i]==source_map[i],"Unrelated native MAP byte changed at {i:#x}");}}
    ensure!(removed>0&&added>0,"Collision correction did not move serialized contributions");
    let mut disc=DiscPatcher::open(original.image().to_vec())?;disc.patch_prot_entry(entry,0,&map)?;for i in 0..disc.entry_count(){if i!=entry{ensure!(disc.read_entry_footprint(i)?==original.read_entry_footprint(i)?,"Unrelated native resource changed");}}
    let image_hash=digest(disc.image());let sidecar=json!({"schema":"lck-native-wall-journal-v1","image_sha256":image_hash,"baseline_image":baseline,"baseline_image_sha256":digest(base.image()),"baseline_map_sha256":digest(&base_map),"journal":next,"historical_parent":PARENT,"historical_parent_sha256":digest(parent_disc.image()),"historical_full_manifest_sha256":digest(full_text.as_bytes()),"moved":moved,"removed_wall_bits":removed,"added_wall_bits":added,"native_gameplay_verified":false});
    let log=format!("OWNED WALL RECOMPOSITION: exact original baseline plus all historical3 and current7 serialized owner layers reproduce entire source wall grid. Applied requested moved/deleted owner changes only; removed{removed}, added{added} bits. Surviving duplicate/import contributions unchanged. Floor low nibbles, MAP geometry/records, TMD/TIMs, SCUS, and all other native entries unchanged from user-verified R7. No native ownership inferred by proximity; generated replacement is authored mesh collision, not proof of original retail collider/floor ownership.");
    let plan=NativeExportPlan{dependencies:Default::default(),source:source.clone(),image:disc.into_image(),patched:vec![log.clone()],skipped:vec!["Native gameplay/collision verification pending; native floor/script ownership not claimed".into()],continuation_receipt:None,unrestricted_log:Some(log),imports:None,object_verification:vec![]};
    super::super::export(plan,&output)?;let file=std::fs::OpenOptions::new().write(true).create_new(true).open(output.with_extension("lck-collision.json"))?;serde_json::to_writer_pretty(file,&sidecar)?;println!("OWNED COLLISION: removed{removed}, added{added} serialized bits; seven duplicate/import owners retained exactly");Ok(())
}

#[test]
#[ignore="exact user full scene, retire only deleted appended assets, no engine patch or emulator"]
fn full_scene_retired_import_storage_candidate()->Result<()>{
    let output=PathBuf::from(std::env::var_os("LCK_RETIRED_IMPORT_IMG").context("new candidate IMG")?);ensure!(!output.exists(),"New output required");
    let source=r"D:\AstraRimElmProject\ManualTests\Crash222_Matrix_20261029_02\E_FULL_USER_SCENE_RECOVERED_SOURCE.img";
    let baseline=r"D:\AstraRimElmProject\ManualTests\M2_StageE_20261027_01\LCK_EXPERIMENTAL_STAGE_E_town01_floor_x37_z18.img";
    let parent=crate::library::Library::open(Path::new(PARENT))?;let scene=legaia_engine_world_views::scene_staging::assemble_field_scene(&parent.index,"town01").map_err(anyhow::Error::msg)?;let world=world_view::build_world(&scene,&parent.index);
    let text=std::fs::read_to_string(r"D:\AstraRimElmProject\ManualTests\Crash222_20261029_01\222-full-scene-preserved.lckscene")?;let doc=scene_instances::parse(&text,"town01",&world).map_err(anyhow::Error::msg)?;
    let entry=Scene::load(&parent.index,"town01")?.field_map_index(&parent.index).context("MAP")?as usize;
    let old_parent=DiscPatcher::open(std::fs::read(PARENT)?)?;let pm=old_parent.read_entry_footprint(entry)?;
    let mut requested=std::collections::BTreeSet::new();for key in &doc.hidden{let c=super::source(&scene,*key)?;let id=id_at(&pm,c)?;let rec=ObjectRecord::parse(&pm,id).context("Deleted source record")?;requested.insert(field_objects::pack_mesh_index(id as u16,&rec).context("Deleted model")?as usize);}
    let base=DiscPatcher::open(std::fs::read(baseline)?)?;let original=DiscPatcher::open(std::fs::read(source)?)?;
    ensure!(base.read_named_file("SCUS_942.54")==original.read_named_file("SCUS_942.54"),"Baseline engine profile changed");
    for i in 0..base.entry_count(){if ![entry,entry+3,entry+4].contains(&i){ensure!(base.read_entry_footprint(i)?==original.read_entry_footprint(i)?,"Baseline dependency PROT {i} changed");}}
    let bb=base.read_entry_footprint(entry+3)?;let bp=base.read_entry_footprint(entry+4)?;let b=original.read_entry_footprint(entry+3)?;let p=original.read_entry_footprint(entry+4)?;let map=original.read_entry_footprint(entry)?;
    let limits=reg::Limits{max_models:1024,max_decoded:16*1024*1024,max_encoded:16*1024*1024};
    let retired=reg::prove_retirable_additions(&bb,Some(&bp),&b,Some(&p),&map,&requested,limits)?;ensure!(retired==requested&&!retired.is_empty(),"Deleted model still shared or unresolved");
    let(new_bundle,new_pac,saved)=reg::retire_model_storage(&b,Some(&p),&retired,limits)?;
    let lib=crate::library::Library::open(Path::new(source))?;let exe=lib.executable()?;let(old_base,_)=reg::native_descriptor_table(&b)?;let(new_base,_)=reg::native_descriptor_table(&new_bundle)?;
    let(old_ids,_)=crate::app::native_loader_trace::tests::execute_original_allocations(&exe,&b,old_base,Some(&p),true)?;
    let(new_ids,_)=crate::app::native_loader_trace::tests::execute_original_allocations(&exe,&new_bundle,new_base,new_pac.as_deref(),true)?;
    ensure!(old_ids.len()==new_ids.len(),"Selector count changed");for(i,(a,z))in old_ids.iter().zip(&new_ids).enumerate(){if !retired.contains(&i){ensure!(a==z,"Unrelated original-engine model identity changed at {i}");}}
    let mut disc=DiscPatcher::open(original.image().to_vec())?;disc.patch_prot_entry(entry+3,0,&new_bundle)?;if let Some(pac)=new_pac{disc.patch_prot_entry(entry+4,0,&pac)?;}
    for i in 0..disc.entry_count(){if ![entry+3,entry+4].contains(&i){ensure!(disc.read_entry_footprint(i)?==original.read_entry_footprint(i)?,"Candidate changed MAP/collision/floor/dependency {i}");}}
    let log=format!("DELETED IMPORT STORAGE RETIREMENT: baseline {baseline}; requested deleted model selectors {requested:?}, retired {retired:?}; {saved} persistent native bytes reclaimed. Original selectors retained with zero-object TMD tombstones; no unrelated model renumbered. All user MAP/collision/floor/transform data, SCUS and texture uploads unchanged. This does not repair stale collision ownership or prove gameplay. Native binding resources match baseline; dynamic/script behavior remains unverified.");
    let plan=NativeExportPlan{dependencies:Default::default(),source:source.into(),image:disc.into_image(),patched:vec![log.clone()],skipped:vec!["GAMEPLAY UNVERIFIED: exact user layout retained; stale collision ownership remains unresolved".into()],continuation_receipt:None,unrestricted_log:Some(log),imports:None,object_verification:vec![]};
    super::super::export(plan,&output)?;println!("RETIRED {retired:?}: {saved} persistent bytes saved; native identities/MAP/texture readback passed");Ok(())
}

#[test]
#[ignore="full preserved scene allocation/free/caller trace; create-only IMG, user operates emulator"]
fn full_scene_heap_caller_trace()->Result<()>{
    let output=PathBuf::from(std::env::var_os("LCK_HEAP_TRACE_IMG").context("new trace IMG")?);ensure!(!output.exists(),"New path required");
    let source=r"D:\AstraRimElmProject\ManualTests\Crash222_Matrix_20261029_02\E_FULL_USER_SCENE_RECOVERED_SOURCE.img";
    let lib=crate::library::Library::open(Path::new(source))?;let exe=lib.executable()?;let edits=legaia_code_hooks::native_load_trace::allocation_plan(&exe)?;
    let original=DiscPatcher::open(std::fs::read(source)?)?;let mut disc=DiscPatcher::open(original.image().to_vec())?;
    let mut patched=exe.clone();for edit in edits{ensure!(edit.prot_index.is_none(),"Only SCUS instrumentation");patched[edit.file_off..edit.file_off+edit.bytes.len()].copy_from_slice(&edit.bytes);disc.patch_named_file("SCUS_942.54",edit.file_off as u64,&edit.bytes)?;}
    ensure!(disc.read_named_file("SCUS_942.54").context("SCUS")?==patched,"SCUS trace readback");
    for i in 0..disc.entry_count(){ensure!(original.read_entry_footprint(i)?==disc.read_entry_footprint(i)?,"Heap tracing changed PROT entry {i}");}
    let plan=NativeExportPlan{dependencies:Default::default(),source:source.into(),image:disc.into_image(),patched:vec!["DIAGNOSTIC: original heap allocate/free calls report sizes, returned pointers and original caller RA; all native data unchanged".into()],skipped:vec!["NOT A FIX: no heap expansion or suppression of native allocation failure".into()],continuation_receipt:None,unrestricted_log:Some("LCK HEAP = requested size, returned pointer, original caller return address. LCK FREE = released pointer, return value, original caller return address. All user scene data identical to E. No host emulator launched.".into()),imports:None,object_verification:vec![]};
    super::super::export(plan,&output)?;Ok(())
}

#[test]
#[ignore="reversible full-scene BIOS TTY trace; create-only IMG, user launches emulator"]
fn full_scene_native_tty_trace()->Result<()>{
    let output=PathBuf::from(std::env::var_os("LCK_TTY_TRACE_IMG").context("new output IMG")?);ensure!(!output.exists(),"New output required");
    let source=r"D:\AstraRimElmProject\ManualTests\Crash222_Matrix_20261029_02\E_FULL_USER_SCENE_RECOVERED_SOURCE.img";
    let lib=crate::library::Library::open(Path::new(source))?;let exe=lib.executable()?;let edits=legaia_code_hooks::native_load_trace::plan(&exe)?;let mut disc=DiscPatcher::open(std::fs::read(source)?)?;
    for edit in &edits{ensure!(edit.prot_index.is_none(),"Diagnostic must only patch SCUS");disc.patch_named_file("SCUS_942.54",edit.file_off as u64,&edit.bytes)?;}
    let mut patched=exe.clone();for edit in &edits{patched[edit.file_off..edit.file_off+edit.bytes.len()].copy_from_slice(&edit.bytes);}
    ensure!(disc.read_named_file("SCUS_942.54").context("SCUS")?==patched,"Diagnostic SCUS readback mismatch");
    let scene=Scene::load(&lib.index,"town01")?;let entry=scene.field_map_index(&lib.index).context("MAP")? as usize;
    let old=DiscPatcher::open(std::fs::read(source)?)?;for i in 0..disc.entry_count(){ensure!(old.read_entry_footprint(i)?==disc.read_entry_footprint(i)?,"TTY trace changed native data entry {i}");}
    let b=disc.read_entry_footprint(entry+3)?;let p=disc.read_entry_footprint(entry+4)?;let(base,_)=reg::native_descriptor_table(&b)?;
    let witness=crate::app::native_loader_trace::tests::execute_original_allocations(&exe,&b,base,Some(&p),true)?;
    let traced=crate::app::native_loader_trace::tests::execute_original_allocations(&patched,&b,base,Some(&p),true)?;
    ensure!(witness==traced,"Tracing changed native model identities/allocation chronology");
    let plan=NativeExportPlan{dependencies:Default::default(),source:source.into(),image:disc.into_image(),patched:vec!["DIAGNOSTIC ONLY: BIOS TTY begin/end around native resource dispatch and TIM upload; all native data entries identical to failing E".into()],skipped:vec!["NOT A CRASH FIX: native gameplay unverified; requires manual BIOS TTY/file logging to observe progress".into()],continuation_receipt:None,unrestricted_log:Some("TRACE PROFILE: LCK LOAD BEGIN/DONE = type_size, source/result, malloc-error accumulator; LCK GPU BEGIN = TIM flags, TIM pointer, malloc-error accumulator. Native functions still execute. All user edits and Stage E floor hook retained. No host emulator launched.".into()),imports:None,object_verification:vec![]};
    let result=super::super::export(plan,&output)?;println!("Trace IMG {}: all PROT bytes unchanged; original instruction model/allocator chronology identical; NOT runtime verified",result.path.display());Ok(())
}

#[test]
#[ignore="exact new-import runtime isolation; create-only outputs, user authored positions, no emulator"]
fn runtime_new_import_isolation()->Result<()>{
    let folder=PathBuf::from(std::env::var_os("LCK_NEW_IMPORT_MATRIX").context("new isolation directory")?);
    ensure!(!folder.exists(),"Use a new directory");std::fs::create_dir(&folder)?;
    let source=crate::library::Library::open(Path::new(RECOVERED))?;let scene=legaia_engine_world_views::scene_staging::assemble_field_scene(&source.index,"town01").map_err(anyhow::Error::msg)?;
    let world=world_view::build_world(&scene,&source.index);let text=std::fs::read_to_string(r"D:\AstraRimElmProject\ManualTests\Crash222_20261029_01\222-full-scene-preserved.lckscene")?;
    let full=scene_instances::parse(&text,"town01",&world).map_err(anyhow::Error::msg)?;
    let entry=Scene::load(&source.index,"town01")?.field_map_index(&source.index).context("MAP")? as usize;
    let e=r"D:\AstraRimElmProject\ManualTests\Crash222_Matrix_20261029_02\E_FULL_USER_SCENE_RECOVERED_SOURCE.img";
    let original=DiscPatcher::open(std::fs::read(RECOVERED)?)?;let exact_map=original.read_entry_footprint(entry)?;
    let mut disc=DiscPatcher::open(std::fs::read(e)?)?;disc.patch_prot_entry(entry,0,&exact_map)?;
    ensure!(disc.read_entry_footprint(entry)?==exact_map,"Resource-only diagnostic changed parent MAP");
    let plan=NativeExportPlan{dependencies:Default::default(),source:e.into(),image:disc.into_image(),patched:vec!["DIAGNOSTIC: E resource bank retained; full parent MAP restored exactly. No new instances spawned.".into()],skipped:vec!["PARTIAL DIAGNOSTIC: no new user edits in MAP; resources only, not a completed scene export".into()],continuation_receipt:None,unrestricted_log:Some("RESOURCE-ONLY DIAGNOSTIC. Exact corrected parent MAP; E's 119-model bank and native texture uploads retained. No new geometry/collision arrangement. Gameplay unverified.".into()),imports:None,object_verification:vec![]};
    let result=super::super::export(plan,&folder.join("R1_RESOURCES_ONLY_PARENT_MAP.img"))?;let mut rows=vec![json!({"case":"R1","path":result.path,"parent_map_identical":true,"gameplay_verified":false})];
    for(label,uids)in [("R2_GEREMI_FIRST",vec![5]),("R3_KOIN4_ONLY",vec![7]),("R4_GEREMI_BOTH",vec![5,6])]{
        let mut doc=full.clone();doc.foreign.retain(|f|uids.contains(&f.uid));doc.created.clear();doc.hidden.clear();doc.moves.clear();doc.rotations.retain(|(t,_)|matches!(t,scene_instances::NameTarget::Created(uid) if uids.contains(uid)));doc.collision_recipes.clear();doc.mesh_collision.clear();
        let mut world=world_view::build_world(&scene,&source.index);scene_instances::apply(&doc,&mut world,scene.placements.len(),&source.index).map_err(anyhow::Error::msg)?;
        let plan=prepare(Path::new(RECOVERED),&scene,&world,&source.index)?;ensure!(plan.imports.is_some_and(|r|r.written==uids.len()),"Import isolation writer failed");ensure!(plan.unrestricted_log.as_ref().unwrap().lines().all(|l|!l.starts_with("FAILED ")),"Isolation includes writer failures");
        let done=super::super::export(plan,&folder.join(format!("{label}.img")))?;rows.push(json!({"case":label,"path":done.path,"new_user_uids":uids,"collision_recipes":0,"gameplay_verified":false}));
    }
    let lib=crate::library::Library::open(Path::new(e))?;let b=lib.index.entry_bytes(entry as u32+3)?;let p=lib.index.entry_bytes(entry as u32+4)?;
    let models=reg::native_model_pool(&b,Some(&p));let headers=models.iter().enumerate().filter(|(i,_)|*i>=117).map(|(i,m)|json!({"selector":i,"sha256":digest(&m.bytes),"bytes":m.bytes.len(),"header_words":m.bytes[..40.min(m.bytes.len())].chunks_exact(4).map(|b|u32::from_le_bytes(b.try_into().unwrap())).collect::<Vec<_>>()})).collect::<Vec<_>>();
    rows.push(json!({"new_native_models":headers,"scope":"Exact manifest subsets for runtime isolation; no inferred collision ownership"}));
    std::fs::write(folder.join("isolation.json"),serde_json::to_vec_pretty(&rows)?)?;Ok(())
}

#[test]
#[ignore="exact preserved 222 scene and incremental native diagnostics; create-only artifacts, no emulator"]
fn exact_222_incremental_scene_regression()->Result<()>{
    let folder=PathBuf::from(std::env::var_os("LCK_222_MATRIX").context("new matrix directory")?);
    ensure!(!folder.exists(),"Use a new diagnostic directory");std::fs::create_dir(&folder)?;
    let original=crate::library::Library::open(Path::new(PARENT))?;
    let recovered=crate::library::Library::open(Path::new(RECOVERED))?;
    let entry=Scene::load(&original.index,"town01")?.field_map_index(&original.index).context("MAP")? as usize;
    // Recovery is permitted only after exact MAP identity, immutable model
    // geometry/primitive identity and material sample equivalence are proven.
    // This is source migration, not ordinal guessing or a replacement layout.
    ensure!(original.index.entry_bytes(entry as u32)?==recovered.index.entry_bytes(entry as u32)?,"Recovery parent changes native object identity/placement tables");
    let limits=reg::Limits{max_models:1024,max_decoded:16*1024*1024,max_encoded:16*1024*1024};
    let mut banks=vec![];
    for lib in [&original,&recovered]{let b=lib.index.entry_bytes(entry as u32+3)?;let p=lib.index.entry_bytes(entry as u32+4)?;banks.push((reg::native_model_pool(&b,Some(&p)),reg::native_texture_sequence(&b,Some(&p),limits)?.1));}
    ensure!(banks[0].0.len()==banks[1].0.len(),"Recovery changes selector count");
    let mut samples=0;for(a,b)in banks[0].0.iter().zip(&banks[1].0){if a.bytes==b.bytes{continue;}ensure!(texture_relocation::texture_only_model_change(&a.bytes,&b.bytes)?,"Recovery changes native geometry/primitive bytes");let(n,changed)=texture_relocation::texture_sample_changes(&a.bytes,&banks[0].1,&b.bytes,&banks[1].1)?;ensure!(changed==0,"Recovery changes intended material colors");samples+=n;}
    let text=std::fs::read_to_string(r"D:\AstraRimElmProject\ManualTests\Crash222_20261029_01\222-full-scene-preserved.lckscene")?;
    let assembled=legaia_engine_world_views::scene_staging::assemble_field_scene(&original.index,"town01").map_err(anyhow::Error::msg)?;
    let world=world_view::build_world(&assembled,&original.index);
    let full=scene_instances::parse(&text,"town01",&world).map_err(anyhow::Error::msg)?;
    ensure!(full.foreign.len()==3&&full.created.len()==4&&full.hidden.len()==2&&full.collision_recipes.len()==7,"Precise user manifest coverage changed");
    let failed_disc=DiscPatcher::open(std::fs::read(FAILED)?)?;let exact_map=failed_disc.read_entry_footprint(entry)?;
    let mut results=vec![json!({"recovery_parent_map_identical":true,"all_model_geometry_bytes_identical":true,"all_material_samples_identical":samples,"source_story_state_unchanged":true})];
    // Exact user's full MAP and resources, only native texture staging changed.
    let b=failed_disc.read_entry_footprint(entry+3)?;let small=reg::lower_tim_lists(&b,limits)?;
    let mut lowered=DiscPatcher::open(failed_disc.image().to_vec())?;
    if small.len()as u64>lowered.entry_footprint(entry+3).unwrap(){lowered.grow_prot_entries(&std::collections::BTreeMap::from([(entry+3,small)]))?;}else{lowered.patch_prot_entry(entry+3,0,&small)?;}
    ensure!(lowered.read_entry_footprint(entry)?==exact_map,"Staging-only diagnostic altered user MAP");
    let plan=NativeExportPlan{dependencies:Default::default(),source:FAILED.into(),image:lowered.into_image(),patched:vec!["Native TIM staging only; all user MAP and model identities retained".into()],skipped:vec!["MISSING NATIVE DEPENDENCY: inherited character atlas conflicts retained to isolate staging".into()],continuation_receipt:None,unrestricted_log:Some("DIAGNOSTIC: exact 222 MAP and model geometry retained; only compressed TIM_LIST lowering applied. Inherited unsafe VRAM uploads retained deliberately for isolation. NOT GAMEPLAY VERIFIED.".into()),imports:None,object_verification:vec![]};
    let path=folder.join("A_222_EXACT_MAP_SMALL_STAGING.img");let done=super::super::export(plan,&path)?;results.push(json!({"case":"A","path":done.path,"full_user_map_identical":true,"inherited_vram_conflicts_retained":true,"gameplay_verified":false}));
    for(label,recover,kind)in [("B_NATIVE_EDITS_ONLY",false,0),("C_FOREIGN_IMPORTS_ONLY",false,1),("D_ALL_GEOMETRY_NO_COLLISION",false,2),("E_FULL_USER_SCENE_RECOVERED_SOURCE",true,3)]{
        let lib=if recover{&recovered}else{&original};let source=if recover{RECOVERED}else{PARENT};
        let mut doc=full.clone();
        if kind==0{doc.foreign.clear();doc.rotations.retain(|(t,_)|!matches!(t,scene_instances::NameTarget::Created(5..=7)));doc.collision_recipes.clear();doc.mesh_collision.clear();}
        if kind==1{doc.created.clear();doc.moves.clear();doc.hidden.clear();doc.rotations.retain(|(t,_)|matches!(t,scene_instances::NameTarget::Created(5..=7)));doc.collision_recipes.clear();doc.mesh_collision.clear();}
        if kind==2{doc.collision_recipes.clear();doc.mesh_collision.clear();}
        let scene=legaia_engine_world_views::scene_staging::assemble_field_scene(&lib.index,"town01").map_err(anyhow::Error::msg)?;let mut world=world_view::build_world(&scene,&lib.index);
        scene_instances::apply(&doc,&mut world,scene.placements.len(),&lib.index).map_err(anyhow::Error::msg)?;
        let before=scene_instances::serialize("town01",&world);let plan=prepare(Path::new(source),&scene,&world,&lib.index)?;
        ensure!(scene_instances::serialize("town01",&world)==before,"Diagnostic export changed editor document");
        ensure!(plan.unrestricted_log.as_ref().unwrap().lines().all(|l|!l.starts_with("FAILED ")),"Actual writer failure in {label}: {:?}",plan.skipped);
        let parsed=DiscPatcher::open(plan.image.clone())?;let map=parsed.read_entry_footprint(entry)?;
        if kind==3{ensure!(map==exact_map,"Recovered full scene did not preserve exact user MAP operations");ensure!(plan.imports.is_some_and(|i|i.requested==3&&i.written==3),"Full import coverage");ensure!(plan.skipped.is_empty(),"Recovered source still reports dependency failures: {:?}",plan.skipped);}
        let bundle=parsed.read_entry_footprint(entry+3)?;let(_,head)=reg::native_descriptor_table(&bundle)?;ensure!(head.used().iter().all(|d|d.type_byte!=1),"Large transient TIM_LIST remains");
        std::fs::write(folder.join(format!("{label}.lckscene")),before)?;
        let path=folder.join(format!("{label}.img"));let done=super::super::export(plan,&path)?;
        results.push(json!({"case":label,"path":done.path,"source":source,"native_duplicates":doc.created.len(),"foreign_imports":doc.foreign.len(),"hidden":doc.hidden.len(),"collision_recipes":doc.collision_recipes.len(),"all_user_map_bytes_identical":map==exact_map,"no_460800_texture_staging":true,"img_readback_verified":true,"native_gameplay_verified":false}));println!("EXACT 222 MATRIX {label} passed native readback; gameplay unverified");
    }
    std::fs::write(folder.join("matrix.json"),serde_json::to_vec_pretty(&results)?)?;Ok(())
}
