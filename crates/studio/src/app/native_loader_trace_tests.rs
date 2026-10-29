use super::*;
use legaia_code_hooks::mips_sim::Cpu;
use serde_json::json;

/// Executes ORIGINAL SCUS instructions for both walkers, the type-1/2/9/flag
/// dispatcher arms, registrar and pointer fixup. OS/heap/LZS/GPU are explicit
/// host services. This is bounded CPU simulation, not PS1 runtime/gameplay.
fn execute_original(exe:&[u8],bundle:&[u8],base:usize,pac:Option<&[u8]>)->Result<Vec<(usize,String)>>{
    let load=word(exe,0x18)?;let mut cpu=Cpu::new();cpu.load_delays=true;cpu.strict_alignment=true;
    for(lo,hi)in [(0x80010638u32,0x8001068c),(0x8001f05c,0x8001f69c),(0x80020224,0x80020308),(0x8002541c,0x800255b8),(0x800268dc,0x80026988),(0x80026b4c,0x80026bd8)]{
        let at=0x800+(lo-load)as usize;cpu.load(lo,exe.get(at..at+(hi-lo)as usize).context("SCUS routine missing")?);
    }
    ensure!(cpu.rd32(0x80026ba8)==0xac640000 && cpu.rd32(0x800202b4)==0x0c007c17,"Unsupported SCUS loader profile");
    let carrier=0x80400000;cpu.load(carrier,&bundle[base..]);cpu.wr32(0x8007b85c,carrier);cpu.wr32(0x8007b774,5);
    let mut ranges=vec![(carrier,bundle[base..].to_vec())];let mut heap=0x81000000u32;
    let mut records=vec![];let mut uploads=0;
    for phase in 0..2{
        cpu.r[29]=0x807ff000;cpu.r[28]=0x8007b318;cpu.r[31]=0x8ffffffc;
        if phase==0{cpu.r[4]=0;cpu.pc=0x80020224;}else{let mode=cpu.r[2]>>8;if mode==0{break;}cpu.r[4]=mode;cpu.pc=0x8002541c;}
        while cpu.pc!=0x8ffffffc{
            ensure!(cpu.steps<3_000_000,"Native loader simulation step budget at {:#x}",cpu.pc);
            let mut stub=None;
            match cpu.pc{
                0x8001f05c=>{let kind=(cpu.r[5]>>24)as u8;
                    // These dependency installers are NOT claimed verified here.
                    if !matches!(kind,0|1|2|9|10|15|20){stub=Some(if (3..=8).contains(&kind){1<<(kind-1)}else if kind==11{16}else{0});}
                },
                0x80017888=>{let size=cpu.r[5]as usize;ensure!(size<16*1024*1024,"Native allocation request too large");let p=heap;heap=heap.checked_add((size as u32+31)&!31).context("Synthetic heap overflow")?;ranges.push((p,vec![0;size]));stub=Some(p);},
                0x8001a55c=>{let n=cpu.r[4]as usize;let src=cpu.r[5];let dst=cpu.r[6];
                    let(start,data)=ranges.iter().find(|(s,b)|src>=*s&&((src-*s)as usize)<b.len()).context("LZS source mapping missing")?;
                    let raw=legaia_lzs::decompress(&data[(src-start)as usize..],n)?;cpu.load(dst,&raw);
                    let allocation=ranges.iter_mut().find(|(s,_)|*s==dst).context("LZS destination missing")?;allocation.1[..raw.len()].copy_from_slice(&raw);stub=Some(n as u32);
                },
                0x8001a8b0=>{let dst=cpu.r[4];let src=cpu.r[5];let n=cpu.r[6]as usize;let raw=(0..n).map(|i|cpu.rd8(src+i as u32)).collect::<Vec<_>>();cpu.load(dst,&raw);let allocation=ranges.iter_mut().find(|(s,_)|*s==dst).context("Copy destination missing")?;allocation.1[..n].copy_from_slice(&raw);stub=Some(dst);},
                0x800255b8=>{let raw=pac.context("Original walker requested missing PAC")?;cpu.load(carrier,raw);ranges[0].1=raw.to_vec();stub=Some(raw.len()as u32);},
                0x80026b4c=>{
                    let p=cpu.r[4];let slot=cpu.rd32(0x8007b774)as usize;
                    let(start,data)=ranges.iter().find(|(s,b)|p>=*s&&((p-*s)as usize)<b.len()).context("Registrar pointer outside allocation")?;
                    let at=(p-start)as usize;
                    let raw=if word(data,0)?==0x80000002{data.clone()}else{let n=word(data,0)?as usize;let i=(0..n).find(|i|word(data,4+i*4).ok().map(|w|w as usize*4)==Some(at)).context("Registrar did not use native pack offset")?;let end=if i+1<n{word(data,8+i*4)?as usize*4}else{data.len()};data[at..end].to_vec()};
                    ensure!(word(&raw,0)?==0x80000002,"Original registrar received non-TMD");records.push((slot,digest(&raw)));
                },
                0x800198e0=>{uploads+=1;ensure!(uploads<16384,"Upload loop budget");stub=Some(0);},
                0x80058104|0x8005fb84|0x800567a8|0x80017b94|0x8003de7c=>stub=Some(0),
                _=>{},
            }
            if let Some(v)=stub{cpu.r[2]=v;cpu.pc=cpu.r[31];}else{let instruction=cpu.rd32(cpu.pc);cpu.exec(instruction);}
        }
    }
    ensure!(cpu.rd32(0x8007b774)as usize==records.len()+5,"Original registrar final counter disagrees");
    for(slot,_)in &records{ensure!(cpu.rd32(0x8007c018+*slot as u32*4)!=0,"Original registrar failed pointer store");}
    Ok(records)
}

#[test]
#[ignore="read-only original SCUS instruction execution against supplied native IMG"]
fn original_loader_mixed_identity()->Result<()>{
    let path=std::env::var_os("LCK_LOADER_IMG").context("LCK_LOADER_IMG required")?;
    let library=crate::library::Library::open(std::path::Path::new(&path))?;let exe=library.executable()?;
    let scene=legaia_engine_scene::scene::Scene::load(&library.index,"town01")?;let entry=scene.field_map_index(&library.index).context("MAP missing")?;
    let bundle=library.index.entry_bytes(entry+3)?;let pac=library.index.entry_bytes(entry+4)?;
    let(base,_)=crate::app::native_asset_plan::scene_registration::native_descriptor_table(&bundle)?;
    let trace=trace(&bundle,base,Some(&pac))?;let executed=execute_original(&exe,&bundle,base,Some(&pac))?;
    ensure!(executed.len()==trace.models.len(),"Original instruction registration count differs");
    for(model,(slot,hash))in trace.models.iter().zip(&executed){ensure!(model.global_slot==*slot&&model.sha256==*hash,"Original instructions disagree at selector {}",model.selector);}
    for m in &trace.models{if m.selector>=114{println!("ORIGINAL SCUS SIMULATION: selector {} global {} address {:#x} hash {} objects {} flags {} {} member {} object_tail {:?}",m.selector,m.global_slot,m.pointer_table_address,m.sha256,m.objects,m.native_flags,m.carrier,m.member,(0..m.objects as usize).map(|i|word(&m.bytes,12+28*i+24).unwrap()).collect::<Vec<_>>());}}
    if let Some(path)=std::env::var_os("LCK_LOADER_REPORT"){let file=std::fs::OpenOptions::new().write(true).create_new(true).open(path)?;serde_json::to_writer_pretty(file,&json!({"trace":trace,"original_instruction_identity_matches":true,"scus_sha256":digest(&exe),"verification_class":"automated original-instruction simulation with substituted OS/heap/LZS/GPU and non-model installers","render_verified":false,"gameplay_verified":false}))?;}
    Ok(())
}

#[test]
#[ignore="read-only compare user-authored editor preview with exported native world geometry"]
fn mixed_visual_alignment()->Result<()>{
    let source=std::env::var_os("LCK_FOREIGN_SOURCE").context("source")?;let path=std::env::var_os("LCK_LOADER_IMG").context("IMG")?;
    let log=std::fs::read_to_string(std::env::var_os("LCK_FOREIGN_LOG").context("log")?)?;
    let start=log.find("LCK_SCENE_V1\t").context("manifest")?;let manifest=log[start..].lines().take_while(|l|!l.is_empty()).collect::<Vec<_>>().join("\n");
    let lib=crate::library::Library::open(std::path::Path::new(&source))?;
    let assembled=legaia_engine_world_views::scene_staging::assemble_field_scene(&lib.index,"town01").map_err(anyhow::Error::msg)?;
    let mut world=crate::app::world_view::build_world(&assembled,&lib.index);
    let doc=crate::app::scene_instances::parse(&manifest,"town01",&world).map_err(anyhow::Error::msg)?;
    crate::app::scene_instances::apply(&doc,&mut world,assembled.placements.len(),&lib.index).map_err(anyhow::Error::msg)?;
    let exported=crate::library::Library::open(std::path::Path::new(&path))?;
    let scene=legaia_engine_world_views::scene_staging::assemble_field_scene(&exported.index,"town01").map_err(anyhow::Error::msg)?;
    for foreign in &doc.foreign{
        let donor=legaia_engine_world_views::scene_staging::assemble_field_scene(&lib.index,&foreign.source).map_err(anyhow::Error::msg)?;
        let d=&donor.placements[foreign.record];
        let native_donor=legaia_engine_scene::scene::Scene::load(&lib.index,&foreign.source)?;let de=native_donor.field_map_index(&lib.index).unwrap();let db=lib.index.entry_bytes(de+3)?;let dp=lib.index.entry_bytes(de+4)?;
        let lim=crate::app::native_asset_plan::scene_registration::Limits{max_models:1024,max_decoded:16*1024*1024,max_encoded:16*1024*1024};
        let(_,dv)=crate::app::native_asset_plan::scene_registration::native_texture_sequence(&db,Some(&dp),lim)?;
        let raw=&donor.res.tmds[d.res_tmd].raw;
        let samples=crate::app::native_asset_plan::texture_relocation::texture_sample_changes(raw,&dv,raw,&donor.res.vram)?;
        println!("DONOR MATERIAL UID {}: {} / {} native-upload versus editor-assembly samples differ",foreign.uid,samples.1,samples.0);
        let draw=world.draws.iter().find(|d|d.editor_uid==Some(foreign.uid)).context("preview")?;
        let model=world.model.as_ref().unwrap();let before=&model.positions[draw.vertex_start..draw.vertex_end];
        let(selector,index)=scene.placements.iter().enumerate().find(|(_,p)|p.env_slot==113+foreign.uid as usize).map(|(i,p)|(p.env_slot,i)).context("exported draw")?;
        let(_,after)=crate::app::world_view::foreign_original_geometry(&scene,"placement",index).map_err(anyhow::Error::msg)?;
        ensure!(before.len()==after.len(),"Selected/native vertex stream differs");
        let max_error=before.iter().zip(&after).flat_map(|(a,b)|(0..3).map(move |i|(a[i]-b[i]).abs())).fold(0f32,f32::max);
        ensure!(max_error<1.5,"Native geometry differs beyond this benchmark's rounding bound: {max_error}");
        let bounds=|points:&[[f32;3]]|{let mut lo=[f32::INFINITY;3];let mut hi=[f32::NEG_INFINITY;3];for p in points{for a in 0..3{lo[a]=lo[a].min(p[a]);hi[a]=hi[a].max(p[a]);}}(lo,hi)};
        println!("VISUAL UID {} selector {} donor origin {:?} anchor {:?} anim {} radius {} preview {:?} native {:?} target {:?}",foreign.uid,selector,[d.world_x,d.world_y,d.world_z],d.anchor,d.anim_id,d.cull_radius,bounds(before),bounds(&after),foreign.center);
        let native=legaia_engine_scene::scene::Scene::load(&exported.index,"town01")?;
        let map=exported.index.entry_bytes(native.field_map_index(&exported.index).unwrap())?;let p=&scene.placements[index];let at=0x8000+(p.anchor.1 as usize*128+p.anchor.0 as usize)*2;
        println!("SPAWN UID {} reference {:?} anchor {:?} anchor word {:#x} destination anim {} bind {:?}",foreign.uid,p.cell,p.anchor,u16::from_le_bytes(map[at..at+2].try_into()?),p.anim_id,native.field_object_binds(&exported.index)?.and_then(|b|b.get(&p.anchor).copied()));
    }Ok(())
}

#[test]
#[ignore="entire-disc read-only native loader/catalog compatibility census; new JSON output directory required"]
fn entire_disc_runtime_compatibility_audit()->Result<()>{
    use std::collections::{BTreeMap,BTreeSet};
    let path=std::env::var_os("LCK_AUDIT_SOURCE").context("LCK_AUDIT_SOURCE required")?;
    let output=std::path::PathBuf::from(std::env::var_os("LCK_AUDIT_OUTPUT").context("LCK_AUDIT_OUTPUT required")?);
    ensure!(output.is_absolute()&&!output.exists(),"Use a NEW audit output directory");std::fs::create_dir(&output)?;
    let source_hash=digest(&std::fs::read(&path)?);let library=crate::library::Library::open(std::path::Path::new(&path))?;
    let exe=library.executable()?;let mut names=library.index.cdname_scene_names();names.sort();names.dedup();
    let mut summary=vec![];let mut families=BTreeMap::<String,Vec<String>>::new();let mut instruction_families=BTreeSet::new();
    for(i,name)in names.iter().enumerate(){
        let result=(||->Result<serde_json::Value>{
            let scene=legaia_engine_scene::scene::Scene::load(&library.index,name)?;
            if library.index.block_range(name).is_some_and(|(raw,_)|raw<2){
                return Ok(json!({"scene":name,"scope":"shared head define, not a native cell; legacy unshifted extraction window must not certify a field registry","loader":null,"native_runtime_verified":false}));
            }
            let inventory=crate::app::native_cell_inventory::inspect(&library.index,name)?;
            let mut loader=serde_json::Value::Null;
            if let Some(entry)=scene.field_map_index(&library.index){
                let bundle=library.index.entry_bytes(entry+3)?;let pac=library.index.entry_bytes(entry+4)?;
                match crate::app::native_asset_plan::scene_registration::native_descriptor_table(&bundle){
                    Ok((base,header))=>{
                        let family=format!("base={base:#x};types={:?}",header.used().iter().map(|d|d.type_byte).collect::<Vec<_>>());families.entry(family.clone()).or_default().push(name.clone());
                        match trace(&bundle,base,Some(&pac)){
                            Ok(trace)=>{
                                let bank=legaia_engine_scene::model_bank::SceneModelBank::build(&scene);let bank=bank.materialise(&scene);
                                let bank_identity=bank.len()==trace.models.len()&&bank.iter().zip(&trace.models).all(|(a,b)|a.as_ref().is_some_and(|a|a==&b.bytes));
                                // One ORIGINAL SCUS execution per distinct native loader family.
                                let instructions=if instruction_families.insert(family){match execute_original(&exe,&bundle,base,Some(&pac)){
                                    Ok(rows)=>json!({"status":"automated_simulation","identity_match":rows.len()==trace.models.len()&&rows.iter().zip(&trace.models).all(|((slot,h),m)|*slot==m.global_slot&&*h==m.sha256)}),
                                    Err(e)=>json!({"status":"unverified","error":format!("{e:#}")}),
                                }}else{json!({"status":"family_representative_only"})};
                                loader=json!({"independent_trace":trace,"editor_bank_identity_matches":bank_identity,"original_instructions":instructions});
                            },Err(e)=>loader=json!({"error":format!("{e:#}")}),
                        }
                    },Err(e)=>loader=json!({"error":format!("{e:#}")}),
                }
            }
            Ok(json!({"scene":name,"inventory":inventory,"loader":loader,"runtime_render_verified":false,"collision_interaction_verified":false}))
        })();
        let report=match result{Ok(report)=>{summary.push(json!({"scene":name,"report":format!("cell-{i:04}.json"),"status":"audited_with_explicit_gaps","loader_present":!report["loader"].is_null()}));report},Err(e)=>{let r=json!({"scene":name,"error":format!("{e:#}"),"status":"unresolved"});summary.push(r.clone());r}};
        std::fs::write(output.join(format!("cell-{i:04}.json")),serde_json::to_vec(&report)?)?;
        if i%10==0{println!("Compatibility audit {}/{}: {name}",i+1,names.len());}
    }
    let mut classes=BTreeMap::<String,usize>::new();let mut entries=vec![];
    for entry in library.index.entries(){let class=format!("{:?}",library.index.class_of(entry.index)?);*classes.entry(class.clone()).or_default()+=1;entries.push(json!({"entry":entry.index,"bytes":entry.size_bytes,"class":class}));}
    std::fs::write(output.join("compatibility.json"),serde_json::to_vec_pretty(&json!({"schema":1,"source_sha256":source_hash,"scus_sha256":digest(&exe),"named_blocks_attempted":names.len(),"cells":summary,"loader_families":families,"all_prot_entries":entries,"entry_classes":classes,
        "scope":"Every named CDNAME block and every PROT classification; known model registrations and existing dependency inventory. Unknown/non-field loader families explicitly unresolved.","universal_compatibility_verified":false,"native_runtime_verified":false,"render_verified":false,"collision_interaction_verified":false}))?)?;
    ensure!(source_hash==digest(&std::fs::read(&path)?),"Source image changed");println!("Audit retained explicit gaps in {}",output.display());Ok(())
}

#[test]
#[ignore="cross-cell native resource families, cumulative different donors, duplicate references and rotation; memory-only fixtures"]
fn cross_cell_family_regressions()->Result<()>{
    use crate::app::{native_asset_plan::{self as assets,scene_registration as reg,texture_relocation::{Planner,multi_tim::{Upload,UploadTrace}}},ps1_export::unrestricted};
    use legaia_scene_formats::field_objects::{self,ObjectRecord};
    let path=std::env::var_os("LCK_AUDIT_SOURCE").context("source")?;
    let folder=std::path::PathBuf::from(std::env::var_os("LCK_AUDIT_OUTPUT").context("audit output")?);
    let census:serde_json::Value=serde_json::from_slice(&std::fs::read(folder.join("compatibility.json"))?)?;
    let library=crate::library::Library::open(std::path::Path::new(&path))?;
    let representatives=census["loader_families"].as_object().context("families")?.values().filter_map(|v|v[0].as_str()).collect::<Vec<_>>();
    let limits=reg::Limits{max_models:1024,max_decoded:16*1024*1024,max_encoded:16*1024*1024};let mut cases=vec![];
    for(destination_index,&destination)in representatives.iter().enumerate(){
        let result=(||->Result<serde_json::Value>{
            let scene=legaia_engine_scene::scene::Scene::load(&library.index,destination)?;let map_entry=scene.field_map_index(&library.index).context("No field MAP")?;
            let mut map=library.index.entry_bytes(map_entry)?.as_ref().clone();ensure!(map.len()==0x12000,"Not a field MAP");
            let mut bundle=library.index.entry_bytes(map_entry+3)?.as_ref().clone();let mut pac=if reg::native_stream_mode(&bundle).is_some(){Some(library.index.entry_bytes(map_entry+4)?.as_ref().clone())}else{None};
            let(base,_)=reg::native_descriptor_table(&bundle)?;let original=trace(&bundle,base,pac.as_deref())?;
            let(_,mut vram)=reg::native_texture_sequence(&bundle,pac.as_deref(),limits)?;let mut planner=Planner::new(&vram,&crate::app::native_asset_plan::texture_relocation::field_runtime_reservations())?;
            let player=library.index.entry_bytes(legaia_actor_formats::field_char_textures::PROT_ENTRY_INDEX)?;let player=legaia_actor_formats::field_char_textures::parse(&player)?;
            let mut resident=legaia_tim::Vram::new();player.upload_to_vram(&mut resident,false);planner.reserve_written(&resident);
            let mut witnesses=vec![];let mut cells=vec![];
            for step in 1..=2{
                let donor_name=representatives[(destination_index+step)%representatives.len()];
                let donor=legaia_engine_scene::scene::Scene::load(&library.index,donor_name)?;let donor_map_entry=donor.field_map_index(&library.index).context("No donor MAP")?;
                let donor_map=library.index.entry_bytes(donor_map_entry)?;let donor_bundle=library.index.entry_bytes(donor_map_entry+3)?;let donor_pac=if reg::native_stream_mode(&donor_bundle).is_some(){Some(library.index.entry_bytes(donor_map_entry+4)?)}else{None};
                let(base,_)=reg::native_descriptor_table(&donor_bundle)?;let donor_trace=trace(&donor_bundle,base,donor_pac.as_deref().map(|p|p.as_slice()))?;
                let placed=field_objects::parse_placements(&donor_map).into_iter().find(|p|p.pack_index.is_some_and(|i|donor_trace.models.get(i as usize).is_some())).context("No donor placed model")?;
                let model=&donor_trace.models[placed.pack_index.unwrap()as usize];let mut inventory=assets::NativeAssetPlan::default();let origin=assets::Origin{scene:donor_name.into(),entry:None,offset:None};
                let id=inventory.register(assets::ResourceKind::Tmd,&model.bytes,origin.clone(),false)?;
                let(tims,expected)=reg::native_texture_sequence(&donor_bundle,donor_pac.as_deref().map(|p|p.as_slice()),limits)?;let mut events=vec![];
                for(i,raw)in tims.iter().enumerate(){let t=legaia_tim::parse_native_upload(raw)?;let resource=inventory.register(assets::ResourceKind::Tim,raw,origin.clone(),false)?;events.push(Upload{sequence:Some(i as u32),resource,image:true,clut:t.clut.is_some(),merge_clut_zeros:false});}
                let upload=UploadTrace::verify_native_replay(&inventory,&events,&expected,"cross-family native trace")?;let relocation=planner.plan_multi(&inventory,&upload,&[id.clone()])?;
                let registered=reg::register_scene(&inventory,&bundle,pac.as_deref(),&vram,&relocation,limits)?;let selector=registered.selector(&id)?;
                let(_,cell)=unrestricted::copy_record(&mut map,&donor_map,(placed.col,placed.row),[0.;3],&[Some(0);16],Some(&[Some(0);16]))?;
                let at=0x8000+(cell.1 as usize*128+cell.0 as usize)*2;let record=(u16::from_le_bytes(map[at..at+2].try_into()?)&511)as usize;
                let raw:[u8;32]=map[record*32..record*32+32].try_into()?;map[record*32..record*32+32].copy_from_slice(&registered.bind_record(&id,&raw)?);
                bundle=registered.bytes().to_vec();if let Some(p)=registered.pac_bytes(){pac=Some(p.to_vec());}vram=relocation.apply(&vram)?;
                let(base,_)=reg::native_descriptor_table(&bundle)?;let audit=trace(&bundle,base,pac.as_deref())?;

                witnesses.push((selector,audit.models[selector as usize].sha256.clone()));cells.push(cell);
            }
            let snapshot=map.clone();let(_,duplicate)=unrestricted::copy_record(&mut map,&snapshot,cells[0],[32.,0.,32.],&[Some(0);16],None)?;
            // Exercise the actual MAP yaw writer independently of resource slots.
            unrestricted::yaw(&mut map,duplicate,[0.,std::f32::consts::FRAC_PI_2,0.],[0.;3])?;
            let di=(word(&map,0x8000+(duplicate.1 as usize*128+duplicate.0 as usize)*2)?&511)as usize;let duplicate_record=ObjectRecord::parse(&map,di).context("duplicate")?;
            ensure!(duplicate_record.pack_index_field==witnesses[0].0&&duplicate_record.rot_y&4095==1024,"Duplicate identity/yaw not persistent");
            let(base,_)=reg::native_descriptor_table(&bundle)?;let audit=trace(&bundle,base,pac.as_deref())?;
            ensure!(audit.models.iter().take(original.models.len()).zip(&original.models).all(|(a,b)|a.sha256==b.sha256),"Existing registry shifted");
            for(selector,hash)in &witnesses{ensure!(audit.models[*selector as usize].sha256==*hash,"Later different donor shifted earlier identity");}
            Ok(json!({"destination":destination,"status":"automated_resource_map_contract_passed","imports":witnesses,"independent_duplicate_reference":duplicate,"yaw_ticks":duplicate_record.rot_y,"native_runtime_verified":false,"render_verified":false,"scene_behavior_dependencies_verified":false}))
        })();
        let case=match result{Ok(v)=>v,Err(e)=>json!({"destination":destination,"status":"explicit_capability_gap","error":format!("{e:#}"),"runtime_verified":false})};println!("CROSS FAMILY {case}");cases.push(case);
    }
    ensure!(cases.iter().any(|c|c["status"]=="automated_resource_map_contract_passed"),"No resource family exercised successfully");
    std::fs::write(folder.join("cross-cell-families.json"),serde_json::to_vec_pretty(&cases)?)?;Ok(())
}

#[test]
#[ignore="compare actual field-character resident footprint with old and candidate native uploads"]
fn resident_character_integrity()->Result<()>{
    use crate::app::native_asset_plan::scene_registration as reg;
    let source=std::env::var_os("LCK_FOREIGN_SOURCE").context("source")?;
    let old=std::env::var_os("LCK_RESIDENT_OLD_IMG").context("old IMG")?;
    let new=std::env::var_os("LCK_LOADER_IMG").context("candidate IMG")?;
    let library=crate::library::Library::open(std::path::Path::new(&source))?;
    let raw=library.index.entry_bytes(legaia_actor_formats::field_char_textures::PROT_ENTRY_INDEX)?;
    let textures=legaia_actor_formats::field_char_textures::parse(&raw)?;let mut resident=legaia_tim::Vram::new();textures.upload_to_vram(&mut resident,false);
    let lim=reg::Limits{max_models:1024,max_decoded:16*1024*1024,max_encoded:16*1024*1024};
    let uploads=|path:&std::path::Path|->Result<legaia_tim::Vram>{let lib=crate::library::Library::open(path)?;let scene=legaia_engine_scene::scene::Scene::load(&lib.index,"town01")?;let entry=scene.field_map_index(&lib.index).context("map")?;let b=lib.index.entry_bytes(entry+3)?;let p=lib.index.entry_bytes(entry+4)?;Ok(reg::native_texture_sequence(&b,Some(&p),lim)?.1)};
    let before=uploads(std::path::Path::new(&source))?;let previous=uploads(std::path::Path::new(&old))?;let candidate=uploads(std::path::Path::new(&new))?;
    let differences=|v:&legaia_tim::Vram|{(0..1024*512).filter(|&i|resident.is_written(i%1024,i/1024)&&v.is_written(i%1024,i/1024)&&(!before.is_written(i%1024,i/1024)||v.as_u16()[i]!=before.as_u16()[i])).count()};
    let old_damage=differences(&previous);let new_damage=differences(&candidate);
    ensure!(old_damage>0&&new_damage==0,"Character residency not protected: old {old_damage}, new {new_damage}");
    println!("RESIDENT INTEGRITY: {} field-character TIMs; old export overwrites {old_damage} resident words, corrected export overwrites {new_damage}. Full character image/CLUT/alternate-face footprints, not a gameplay test.",textures.textures.len());Ok(())
}

#[test]
#[ignore="original field-overlay spawn-gate instruction simulation; actor allocation substituted, no rendering"]
fn original_window_spawn_gates()->Result<()>{
    let path=std::env::var_os("LCK_LOADER_IMG").context("IMG")?;let library=crate::library::Library::open(std::path::Path::new(&path))?;
    let overlay=library.index.entry_bytes(897)?;let scene=legaia_engine_scene::scene::Scene::load(&library.index,"town01")?;let map=library.index.entry_bytes(scene.field_map_index(&library.index).context("MAP")?)?;
    let mut cpu=Cpu::new();cpu.load_delays=true;cpu.strict_alignment=true;
    cpu.load(0x801ce818,&overlay);ensure!(cpu.rd32(0x801d7ccc)==0x30420400,"Unrecognized field spawn-gate profile");
    cpu.load(0x80400000,&map);cpu.wr32(0x1f8003ec,0x80400000);cpu.load(0x1f800384,&[0,0,128,128]);
    cpu.wr32(0x8007c36c,0x80500000);cpu.wr32(0x80500000,0);
    cpu.pc=0x801d7b50;cpu.r[29]=0x807ff000;cpu.r[31]=0x8ffffffc;let mut spawned=vec![];
    while cpu.pc!=0x8ffffffc{
        ensure!(cpu.steps<3_000_000,"Window gate simulation budget exceeded");
        if cpu.pc==0x80024c88{
            let id=cpu.rd16(cpu.r[5]+4)as usize;spawned.push(id);
            cpu.r[2]=0x80600000+spawned.len()as u32*0x100;cpu.pc=cpu.r[31];
        }else{let instruction=cpu.rd32(cpu.pc);cpu.exec(instruction);}
    }
    let mut imported=0;
    for id in spawned{let selector=word(&map,id*32+16)?&0xffff;if (114..=116).contains(&selector){imported+=1;println!("ORIGINAL WINDOW GATES: descriptor {id} selector {selector} reaches actor constructor with full [0,128) region; actual live region/render/animation NOT verified");}}
    ensure!(imported==3,"One or more requested native objects fail original window gates even in full region");Ok(())
}

#[test]
#[ignore="read-only actual World Objects catalog census across every named source; no GUI"]
fn world_objects_catalog_compatibility()->Result<()>{
    use crate::app::global_repository::{self,Kind};
    let source=std::env::var_os("LCK_AUDIT_SOURCE").context("source")?;let folder=std::path::PathBuf::from(std::env::var_os("LCK_AUDIT_OUTPUT").context("output")?);
    let library=crate::library::Library::open(std::path::Path::new(&source))?;let mut names=library.index.cdname_scene_names();names.sort();names.dedup();let mut cells=vec![];let mut total=0;
    for(i,name)in names.iter().enumerate(){
        let result=global_repository::catalog_one(&library.index,name);
        let cell=match result{Ok(assets)=>{
            total+=assets.len();let rows=assets.iter().map(|a|json!({"source":a.source,"key":a.key,"label":a.label,"category":a.kind_label(),"catalog_fingerprint_u64":a.mesh_fingerprint,
                "native_runtime_verified":false,"render_verified":false,"collision_interaction_verified":false,
                "required_native_adapter":match a.kind{Kind::Placement(_)=>"MAP + native registration + donor binding/animation/scripts + physics closure",Kind::Terrain(_)=>"decoration or terrain MAP + native geometry + floor sampler/physics closure",Kind::Mesh(_)=>"exact TMD/TIM registration plus independent placement allocation; raw mesh alone has no authored behavior",Kind::Npc(_)=>"MAN actor, model/ANM references, script closure, interaction/spawn state"}})).collect::<Vec<_>>();
            json!({"scene":name,"assets":rows,"source_has_legacy_head_alias":library.index.block_range(name).is_some_and(|(raw,_)|raw<2)})
        },Err(error)=>json!({"scene":name,"error":error})};
        cells.push(cell);if i%10==0{println!("Actual World Objects catalog {}/{}; {} assets so far",i+1,names.len(),total);}
    }
    let file=std::fs::OpenOptions::new().write(true).create_new(true).open(folder.join("world-objects-catalog.json"))?;
    serde_json::to_writer(file,&json!({"schema":1,"cataloged_assets":total,"named_sources_attempted":names.len(),"cells":cells,"resource_and_script_evidence":"cell-*.json and compatibility.json in this directory; content identity is not runtime registration or behavior","universal_compatibility_verified":false}))?;
    ensure!(total>0,"Empty catalog");println!("Catalog compatibility: {total} actual repository assets indexed with verification gaps");Ok(())
}
