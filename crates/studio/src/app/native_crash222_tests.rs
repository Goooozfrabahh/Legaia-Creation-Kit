use super::*;
use crate::app::native_asset_plan::{scene_registration as reg,texture_relocation};
use serde_json::json;
use crate::app::collision_recipes::digest;

const PARENT:&str=r"D:\AstraRimElmProject\ManualTests\Native_Format_Adapters_20261028_01\LCK_EXPERIMENTAL_MIXED_FORMATS_FIXED.img";
const RECOVERED:&str=r"D:\AstraRimElmProject\ManualTests\Runtime_Compatibility_20261029_02\LCK_EXPERIMENTAL_MIXED_RUNTIME_CANDIDATE.img";
const FAILED:&str=r"D:\AstraRimElmProject\222.img";

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
