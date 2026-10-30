use super::*;
use crate::app::native_asset_plan::{scene_registration as reg,texture_relocation};
use serde_json::json;

const PARENT:&str=r"D:\AstraRimElmProject\ManualTests\Native_Format_Adapters_20261028_01\LCK_EXPERIMENTAL_MIXED_FORMATS_FIXED.img";
const RECOVERED:&str=r"D:\AstraRimElmProject\ManualTests\Runtime_Compatibility_20261029_02\LCK_EXPERIMENTAL_MIXED_RUNTIME_CANDIDATE.img";
const FAILED:&str=r"D:\AstraRimElmProject\222.img";

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
