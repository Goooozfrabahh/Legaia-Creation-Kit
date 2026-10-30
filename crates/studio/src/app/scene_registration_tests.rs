use super::*;
use super::super::{ResourceKind,texture_relocation::{Planner,multi_tim::{Upload,UploadTrace},tests::{tim,model,register as resource}}};
fn budgets()->Limits {Limits{max_models:1024,max_decoded:16*1024*1024,max_encoded:16*1024*1024}}
#[test]
fn deleted_resource_retirement_preserves_selectors_shared_references_and_script_evidence(){
    let original=model(0,0,vec![(0,0),(1,0),(0,1)]);let added=model(0,0,vec![(0,0),(2,0),(0,2)]);
    let base=typed_bundle(&[(1,pack(&[tim(0,31)]).unwrap()),(2,pack(&[original.clone()]).unwrap()),(3,vec![1,2,3,4]),(6,vec![0;4])],0);
    let current=typed_bundle(&[(1,pack(&[tim(0,31)]).unwrap()),(2,pack(&[original.clone(),added.clone()]).unwrap()),(3,vec![1,2,3,4]),(6,vec![0;4])],0);
    let requested=BTreeSet::from([1]);let mut map=vec![0;0x12000];
    let eligible=prove_retirable_additions(&base,None,&current,None,&map,&requested,budgets()).unwrap();assert_eq!(eligible,requested);
    let fingerprints=resource_baseline(&base,None,budgets()).unwrap();let fingerprints:ResourceBaseline=serde_json::from_slice(&serde_json::to_vec(&fingerprints).unwrap()).unwrap();
    assert_eq!(prove_retirable_baseline(&fingerprints,&current,None,&map,&requested,budgets()).unwrap(),eligible,"Self-contained baseline release differs");
    let (out,_,saved)=retire_model_storage(&current,None,&eligible,budgets()).unwrap();let bank=native_model_pool(&out,None);assert_eq!(bank.len(),2);assert_eq!(bank[0].bytes,original);assert_eq!(bank[1].bytes.len(),12);assert_eq!(saved,added.len()-12);
    map[128+16..128+18].copy_from_slice(&1u16.to_le_bytes());map[128+18..128+20].copy_from_slice(&6u16.to_le_bytes());map[0x8000..0x8002].copy_from_slice(&4u16.to_le_bytes());
    assert!(prove_retirable_additions(&base,None,&current,None,&map,&requested,budgets()).unwrap().is_empty(),"Shared/alive model retained");
    assert!(prove_retirable_baseline(&fingerprints,&current,None,&map,&requested,budgets()).unwrap().is_empty(),"Persistent shared reference released");
    assert!(prove_retirable_additions(&base,None,&current,None,&map,&BTreeSet::from([0]),budgets()).is_err(),"Original bank never inferred owned");
    let changed=typed_bundle(&[(1,pack(&[tim(0,31)]).unwrap()),(2,pack(&[bank[0].bytes.clone(),added]).unwrap()),(3,vec![9,2,3,4]),(6,vec![0;4])],0);
    assert!(prove_retirable_additions(&base,None,&changed,None,&map,&requested,budgets()).is_err(),"Changed actor/script resources need closure");
    assert!(prove_retirable_baseline(&fingerprints,&changed,None,&map,&requested,budgets()).is_err(),"Persistent actor/script closure was bypassed");
}
#[test]
fn retired_storage_supports_pac_pack_and_single_tmd_without_shifting_later_selectors(){
    let a=model(0,0,vec![(0,0),(1,0),(0,1)]);let b=model(0,0,vec![(0,0),(2,0),(0,2)]);let c=model(0,0,vec![(0,0),(3,0),(0,3)]);
    let bundle=typed_bundle(&[(1,pack(&[tim(0,31)]).unwrap()),(2,pack(&[a.clone()]).unwrap()),(20,vec![0;4])],0x800);
    let packed=pack(&[b.clone()]).unwrap();let mut pac=table::encode_size_word(2,packed.len()as u32).to_le_bytes().to_vec();pac.extend(packed);pac.extend(table::encode_size_word(9,c.len()as u32).to_le_bytes());pac.extend(&c);pac.extend([0u8;16]);
    let(out,stream,saved)=retire_model_storage(&bundle,Some(&pac),&BTreeSet::from([1]),budgets()).unwrap();let bank=native_model_pool(&out,stream.as_deref());
    assert_eq!(bank.len(),3);assert_eq!(bank[0].bytes,a);assert_eq!(bank[1].bytes.len(),12);assert_eq!(bank[2].bytes,c);assert_eq!(saved,b.len()-12);assert_eq!(&out[..0x800],&bundle[..0x800]);
    let(out,stream,saved)=retire_model_storage(&bundle,Some(&pac),&BTreeSet::from([2]),budgets()).unwrap();let bank=native_model_pool(&out,stream.as_deref());assert_eq!(bank[1].bytes,b);assert_eq!(bank[2].bytes.len(),12);assert_eq!(saved,c.len()-12);
    assert_eq!(retire_model_storage(&bundle,Some(&pac),&BTreeSet::new(),budgets()).unwrap(),(bundle,Some(pac),0));
}
#[test]
fn small_native_staging_preserves_order_models_suffixes_and_is_idempotent(){
    let image_members=(0..24).map(|i|tim(0,i+1)).collect::<Vec<_>>();
    let geometry=model(0,0,vec![(0,0),(1,0),(0,1)]);
    let source=typed_bundle(&[(1,pack(&image_members).unwrap()),(2,pack(&[geometry.clone()]).unwrap()),(6,vec![0;4])],0);
    let lowered=lower_tim_lists(&source,budgets()).unwrap();let(_,table)=native_descriptor_table(&lowered).unwrap();
    assert_eq!(table.count,26);assert!(table.used()[..24].iter().all(|d|d.type_byte==0&&d.size as usize<=SINGLE_TIM_BUFFER));
    assert_eq!(lowered.len()%2048,0);let(before,vram_before)=native_texture_sequence(&source,None,budgets()).unwrap();let(after,vram_after)=native_texture_sequence(&lowered,None,budgets()).unwrap();
    assert_eq!(before,after);assert_eq!(vram_before.as_u16(),vram_after.as_u16());assert_eq!(native_model_pool(&lowered,None)[0].bytes,geometry);
    assert_eq!(lower_tim_lists(&lowered,budgets()).unwrap(),lowered);
    let mut oversized=image_members[0].clone();oversized.resize(SINGLE_TIM_BUFFER+4,0x57);
    let source=typed_bundle(&[(1,pack(&[oversized]).unwrap()),(2,pack(&[geometry]).unwrap())],0);
    assert_eq!(lower_tim_lists(&source,budgets()).unwrap(),source,"Do not silently truncate an unrepresentable native member suffix");
}
fn typed_bundle(records:&[(u8,Vec<u8>)],base:usize)->Vec<u8>{
    let mut out=vec![0x5a;base];out.resize(base+8+records.len()*8,0);out[base..base+4].copy_from_slice(&(records.len() as u32).to_le_bytes());
    out[base+4..base+8].copy_from_slice(&records.iter().map(|(_,r)|r.len() as u32).sum::<u32>().to_le_bytes());
    for(i,(kind,raw))in records.iter().enumerate(){let at=base+8+i*8;let offset=(out.len()-base) as u32;out[at..at+4].copy_from_slice(&table::encode_size_word(*kind,raw.len() as u32).to_le_bytes());out[at+4..at+8].copy_from_slice(&offset.to_le_bytes());
        if matches!(*kind,0x0a|0x14){out.extend_from_slice(raw);}else{out.extend_from_slice(&legaia_lzs::compress(raw));}}
    out
}
#[test]
fn native_small_embedded_and_extended_tables_preserve_registration_and_prefixes(){
    for base in [0,0x800,0x1000]{
        let (_,before,inventory,relocation,id)=fixture();
        let old=model(480<<6,2,vec![(0,0),(7,0),(0,7)]);
        let source=typed_bundle(&[(1,pack(&[tim(0,31)]).unwrap()),(2,pack(&[old.clone()]).unwrap()),(6,vec![0;4]),(7,vec![0;4])],base);
        assert_eq!(native_descriptor_table(&source).unwrap().0,base);
        let registered=super::register(&inventory,&source,&before,&relocation,budgets()).unwrap();
        assert_eq!(&registered.bytes()[..base],&source[..base]);assert_eq!(registered.selector(&id).unwrap(),1);
        assert_eq!(native_model_pool(registered.bytes(),None)[0].bytes,old);
    }
    let records=(0..8).map(|i|(if i==0{2}else if i==7{0x14}else{(i+2) as u8},if i==0{pack(&[model(0,0,vec![(0,0),(1,0),(0,1)])]).unwrap()}else{vec![0;4]})).collect::<Vec<_>>();
    let source=typed_bundle(&records,0);assert_eq!(table::detect(&source).unwrap().count,8);assert_eq!(native_model_pool(&source,None).len(),1);
}
#[test]
fn raw_palette_upload_words_are_decoded_and_serialized_without_fabricated_colors(){
    let colors=(0..256u16).flat_map(|v|v.to_le_bytes()).collect::<Vec<_>>();
    let mut record=vec![0;8];record.extend_from_slice(&524u32.to_le_bytes());for n in [0u16,504,256,1]{record.extend_from_slice(&n.to_le_bytes());}record.extend_from_slice(&colors);
    let list=pack(&[record]).unwrap();let normalized=native_tim_members(&list).unwrap();let native=legaia_tim::parse_native_upload(&normalized[0]).unwrap();assert_eq!(native.image.data,colors);
    let mut vram=Vram::new();upload_pack(&list,&mut vram).unwrap();assert_eq!(&vram.as_u16()[504*1024..504*1024+256],&(0..256u16).collect::<Vec<_>>());
    assert!(native_tim_members(&pack(&[vec![0;40]]).unwrap()).is_err());
}
#[test]
fn bare_tmd_and_streamed_model_carriers_keep_old_ids_and_append_new_models(){
    let (_,before,inventory,relocation,id)=fixture();let old=model(480<<6,2,vec![(0,0),(7,0),(0,7)]);
    let source=typed_bundle(&[(1,pack(&[tim(0,31)]).unwrap()),(9,old.clone())],0);
    let registered=super::register(&inventory,&source,&before,&relocation,budgets()).unwrap();assert_eq!(native_model_pool(registered.bytes(),None)[0].bytes,old);assert_eq!(registered.selector(&id).unwrap(),1);
    let source=typed_bundle(&[(1,pack(&[tim(0,31)]).unwrap()),(3,vec![0;4]),(6,vec![0;4]),(0x14,vec![0;4])],0);
    let packed=pack(&[old.clone()]).unwrap();let mut pac=table::encode_size_word(2,packed.len() as u32).to_le_bytes().to_vec();pac.extend_from_slice(&packed);pac.extend_from_slice(&[0;16]);
    let registered=register_scene(&inventory,&source,Some(&pac),&before,&relocation,budgets()).unwrap();let pool=native_model_pool(registered.bytes(),registered.pac_bytes());assert_eq!(pool.len(),2);assert_eq!(pool[0].bytes,old);assert_eq!(registered.selector(&id).unwrap(),1);
}
#[test]
fn mode_0a_bare_texture_pack_uses_its_native_reader_and_registration(){
    let (_,before,inventory,relocation,id)=fixture();let old=model(480<<6,2,vec![(0,0),(7,0),(0,7)]);
    let source=typed_bundle(&[(2,pack(&[old]).unwrap()),(3,vec![0;4]),(6,vec![0;4]),(0x0a,vec![0;4])],0);
    let pac=pack(&[tim(0,31)]).unwrap();let (_,actual)=native_texture_sequence(&source,Some(&pac),budgets()).unwrap();assert_eq!(crate::app::naming::material_key(&actual),crate::app::naming::material_key(&before));
    let registered=register_scene(&inventory,&source,Some(&pac),&before,&relocation,budgets()).unwrap();assert_eq!(registered.selector(&id).unwrap(),1);
    let (_,replay)=native_texture_sequence(registered.bytes(),registered.pac_bytes(),budgets()).unwrap();assert_eq!(crate::app::naming::material_key(&replay),crate::app::naming::material_key(&relocation.apply(&before).unwrap()));
}
fn bundle(payloads:&[Vec<u8>])->Vec<u8> {
    let mut out=vec![0;8+payloads.len()*8];out[..4].copy_from_slice(&(payloads.len() as u32).to_le_bytes());out[4..8].copy_from_slice(&(payloads.iter().map(|p|p.len() as u32).sum::<u32>()).to_le_bytes());
    for (i,payload) in payloads.iter().enumerate(){let at=8+i*8;let offset=out.len() as u32;out[at..at+4].copy_from_slice(&table::encode_size_word(i as u8+1,payload.len() as u32).to_le_bytes());out[at+4..at+8].copy_from_slice(&offset.to_le_bytes());out.extend_from_slice(&legaia_lzs::compress(payload));out.extend_from_slice(&[0xde,0xad,i as u8,0xbe]);}out
}
fn fixture()->(Vec<u8>,Vram,NativeAssetPlan,MultiPlan,ResourceId) {
    let old_tim=tim(0,31);let mut before=Vram::new();before.upload_tim(&legaia_tim::parse_strict(&old_tim).unwrap());
    let old_model=model(480<<6,2,vec![(0,0),(7,0),(0,7)]);
    let source=bundle(&[pack(&[old_tim]).unwrap(),pack(&[old_model]).unwrap(),b"KEEP_MAN_123".to_vec(),vec![4,4,0,0],vec![5,5,0,0],vec![0;4],vec![7,7,0,0]]);
    let (inventory,relocation,id)=incoming(&before);(source,before,inventory,relocation,id)
}
fn incoming(before:&Vram)->(NativeAssetPlan,MultiPlan,ResourceId) {
    let mut inventory=NativeAssetPlan::default();let raw=tim(0,992);let texture=resource(&mut inventory,ResourceKind::Tim,&raw);
    let id=resource(&mut inventory,ResourceKind::Tmd,&model(480<<6,2,vec![(0,0),(7,0),(0,7)]));
    let mut expected=Vram::new();expected.upload_tim(&legaia_tim::parse_strict(&raw).unwrap());
    let trace=UploadTrace::verify_replay(&inventory,&[Upload{sequence:Some(0),resource:texture,image:true,clut:true,merge_clut_zeros:false}],&expected,"Explicit fixture TIM consumer replay").unwrap();
    let relocation=Planner::new(before,&[]).unwrap().plan_multi(&inventory,&trace,&[id.clone()]).unwrap();(inventory,relocation,id)
}
fn streamed_fixture()->(Vec<u8>,Vec<u8>,Vram,NativeAssetPlan,MultiPlan,ResourceId) {
    let (source,before,inventory,relocation,id)=fixture();let (_,old)=slots(&source,budgets()).unwrap();
    let payloads=[(2,old[1].decoded.clone()),(3,old[2].decoded.clone()),(5,old[4].decoded.clone()),(6,old[5].decoded.clone()),(7,old[6].decoded.clone()),(0x14,b"OPAQUE_FLAG".to_vec())];
    let mut bundle=vec![0u8;56];bundle[..4].copy_from_slice(&6u32.to_le_bytes());bundle[4..8].copy_from_slice(&(payloads.iter().map(|(_,p)|p.len() as u32).sum::<u32>()).to_le_bytes());
    for (i,(kind,raw)) in payloads.iter().enumerate(){let offset=bundle.len() as u32;let at=8+i*8;bundle[at..at+4].copy_from_slice(&table::encode_size_word(*kind,raw.len() as u32).to_le_bytes());bundle[at+4..at+8].copy_from_slice(&offset.to_le_bytes());if *kind==0x14{bundle.extend_from_slice(raw);}else{bundle.extend_from_slice(&legaia_lzs::compress(raw));}}
    let mut pac=table::encode_size_word(1,old[0].decoded.len() as u32).to_le_bytes().to_vec();pac.extend_from_slice(&old[0].decoded);pac.extend_from_slice(&[0;16]);
    (bundle,pac,before,inventory,relocation,id)
}
#[test]
fn streamed_pac_registration_preserves_upload_order_flags_and_selectors() {
    let (bundle,pac,before,inventory,relocation,id)=streamed_fixture();
    assert!(super::register(&inventory,&bundle,&before,&relocation,budgets()).is_err());
    let result=register_scene(&inventory,&bundle,Some(&pac),&before,&relocation,budgets()).unwrap();
    result.validate_source(&bundle).unwrap();result.validate_pac_source(&pac).unwrap();assert_eq!(result.selector(&id).unwrap(),1);
    let (_,old)=slots_for_scene(&bundle,budgets(),true).unwrap();let (_,new)=slots_for_scene(result.bytes(),budgets(),true).unwrap();
    for i in 1..old.len(){assert_eq!(old[i].stored,new[i].stored);assert_eq!(old[i].decoded,new[i].decoded);}
    let output=result.pac_bytes().unwrap();assert_eq!(&output[..pac.len()-16],&pac[..pac.len()-16]);assert_eq!(&output[output.len()-16..],&pac[pac.len()-16..]);
    let mut replay=before.clone();pac_uploads(output,&mut replay).unwrap();assert_eq!(crate::app::naming::material_key(&replay),crate::app::naming::material_key(&relocation.apply(&before).unwrap()));
    let again=register_scene(&inventory,result.bytes(),Some(output),&before,&relocation,budgets()).unwrap();assert_eq!(again.bytes(),result.bytes());assert_eq!(again.pac_bytes(),Some(output));
    assert_eq!(table::mesh_pool(result.bytes())[0].bytes,table::mesh_pool(&bundle)[0].bytes);
    let mut changed=pac.clone();changed[4]^=1;assert!(result.validate_pac_source(&changed).is_err());
}

#[test]
fn streamed_flag_size_is_not_a_payload_extent_and_survives_repacking(){
    let (mut bundle,pac,before,inventory,relocation,id)=streamed_fixture();
    let table=table::detect(&bundle).unwrap();let i=table.descriptor_index(0x14).unwrap();let old=table.used()[i].size;
    let declared=1927u32;bundle[8+i*8..12+i*8].copy_from_slice(&table::encode_size_word(0x14,declared).to_le_bytes());
    let total=u32::from_le_bytes(bundle[4..8].try_into().unwrap())-old+declared;bundle[4..8].copy_from_slice(&total.to_le_bytes());
    let (_,slots)=slots_for_scene(&bundle,budgets(),true).unwrap();assert!(slots[i].stored.len()<declared as usize);
    let registered=register_scene(&inventory,&bundle,Some(&pac),&before,&relocation,budgets()).unwrap();
    let read=table::detect(registered.bytes()).unwrap();assert_eq!(read.used()[i].size,declared);assert_eq!(registered.selector(&id).unwrap(),1);
    let (_,after)=slots_for_scene(registered.bytes(),budgets(),true).unwrap();assert_eq!(after[i].stored,slots[i].stored);
}

#[test]
fn growing_pac_discards_obsolete_zero_padding_but_preserves_native_upload_prefix(){
    let (_,mut source,_,_,_,_)=streamed_fixture();
    let mut replay=Vram::new();let (end,_)=pac_uploads(&source,&mut replay).unwrap();
    source.resize((end+4).div_ceil(2048)*2048,0);
    let incoming=pack(&[tim(0,0x421)]).unwrap();
    let compiled=append_pac_uploads(&source,&incoming,&mut replay,1024*1024).unwrap();
    let mut check=Vram::new();let (new_end,tims)=pac_uploads(&compiled,&mut check).unwrap();
    assert_eq!(&compiled[..end],&source[..end]);assert_eq!(tims.len(),2);
    assert_eq!(compiled.len(),(new_end+4).max(source.len()).div_ceil(2048)*2048);
    assert!(compiled[new_end..].iter().all(|b|*b==0));
}
#[test]
fn streamed_pac_rejects_unresolved_contributors_malformed_chunks_and_limits() {
    let (bundle,pac,before,inventory,relocation,_)=streamed_fixture();let saved=before.clone();
    for bad in [pac[..pac.len()-16].to_vec(),{let mut p=pac.clone();p[3]=2;p},{let mut p=pac.clone();p[0]|=1;p},vec![0xff;8]] {
        assert!(register_scene(&inventory,&bundle,Some(&bad),&before,&relocation,budgets()).is_err());
    }
    assert!(register_scene(&inventory,&bundle,Some(&pac),&before,&relocation,Limits{max_encoded:pac.len(),..budgets()}).is_err());
    assert_eq!(crate::app::naming::material_key(&before),crate::app::naming::material_key(&saved));
}
#[test]
fn production_resource_compiler_patches_transactionally_and_preserves_every_other_entry() {
    let (bundle,pac,before,inventory,relocation,id)=streamed_fixture();
    // Minimal native archive with MAP/+1/+2/bundle/PAC/neighbor entries.
    let payloads=[vec![0x11;2048],vec![0x22;2048],vec![0x33;2048],bundle,pac,vec![0x66;2048]];
    let mut prot=vec![0u8;2048];prot[4..8].copy_from_slice(&(payloads.len() as u32).to_le_bytes());prot[8..12].copy_from_slice(&1u32.to_le_bytes());
    for (i,payload) in payloads.iter().enumerate(){let sector=(prot.len()/2048) as u32;prot[16+i*4..20+i*4].copy_from_slice(&sector.to_le_bytes());prot.extend_from_slice(payload);prot.resize(prot.len().div_ceil(2048)*2048,0);}
    let end=(prot.len()/2048) as u32;prot[16+payloads.len()*4..20+payloads.len()*4].copy_from_slice(&end.to_le_bytes());
    let disc=legaia_disc_patch::disc::DiscPatcher::open(legaia_disc_patch::disc::synth::synth_disc(&prot)).unwrap();let original=disc.image().to_vec();
    let (compiled,registered)=super::super::super::ps1_export::NativeExportPlan::compile_scene_resources(&disc,0,&inventory,&before,&relocation,budgets()).unwrap();
    assert_eq!(disc.image(),original);assert_eq!(disc.entry_count(),compiled.entry_count());
    for idx in [0,1,2,5]{assert_eq!(disc.read_entry_footprint(idx).unwrap(),compiled.read_entry_footprint(idx).unwrap());}
    let bytes=compiled.read_entry_footprint(3).unwrap();assert_eq!(table::mesh_pool(&bytes)[registered.selector(&id).unwrap() as usize].bytes,table::mesh_pool(registered.bytes())[1].bytes);
    let second=super::super::super::ps1_export::NativeExportPlan::compile_scene_resources(&disc,0,&inventory,&before,&relocation,budgets()).unwrap();assert_eq!(compiled.image(),second.0.image());
    assert!(super::super::super::ps1_export::NativeExportPlan::compile_scene_resources(&disc,0,&inventory,&before,&relocation,Limits{max_models:1,..budgets()}).is_err());assert_eq!(disc.image(),original);
    let mut opaque=legaia_disc_patch::disc::DiscPatcher::open(original.clone()).unwrap();opaque.patch_prot_entry(3,2047,&[0x77]).unwrap();
    let saved=opaque.image().to_vec();let grown=super::super::super::ps1_export::NativeExportPlan::compile_scene_resources(&opaque,0,&inventory,&before,&relocation,budgets()).unwrap();
    assert!(grown.0.read_entry_footprint(3).unwrap().len()>opaque.read_entry_footprint(3).unwrap().len());assert_eq!(opaque.image(),saved);
    assert_eq!(grown.0.read_entry_footprint(5).unwrap(),opaque.read_entry_footprint(5).unwrap());
}
#[test]
#[ignore = "explicit read-only town01 PAC packaging and native donor gates; no IMG output"]
fn real_town01_pac_packaging_and_native_donor_gates() -> Result<()> {
    let path=std::path::PathBuf::from(std::env::var_os("LCK_P0_SOURCE").context("LCK_P0_SOURCE required")?);
    let lib=crate::library::Library::open(&path)?;
    let destination=legaia_engine_scene::scene::Scene::load(&lib.index,"town01")?;
    let bundle=&destination.entries.iter().find(|e|e.idx==destination.start+3).context("Destination bundle")?.bytes;
    let pac=&destination.entries.iter().find(|e|e.idx==destination.start+4).context("Destination PAC")?.bytes;
    let mut before=Vram::new();pac_uploads(pac,&mut before)?;
    // A synthetic package isolates actual town01 native serialization from
    // separately unresolved donor upload/physics/runtime contracts.
    let (inventory,relocation,id)=incoming(&before);
    let original=legaia_disc_patch::disc::DiscPatcher::open(std::fs::read(&path)?)?;
    let (compiled,result)=super::super::super::ps1_export::NativeExportPlan::compile_scene_resources(&original,destination.start as usize,&inventory,&before,&relocation,budgets())?;
    println!("town01 native packaging (synthetic donor): selector {}, bundle {} -> {}, PAC {} -> {}; {}. Native sectors reparsed; all other PROT entries identical; in-memory only",result.selector(&id)?,bundle.len(),result.bytes().len(),pac.len(),result.pac_bytes().unwrap().len(),result.model_diagnostic());
    drop(compiled);
    let mut examined=0;
    let assembled_destination=legaia_engine_world_views::scene_staging::assemble_field_scene(&lib.index,"town01").map_err(anyhow::Error::msg)?;
    let mut editor=super::super::super::world_view::build_world(&assembled_destination,&lib.index);
    for name in ["bylon","izumi"] {
        let donor=legaia_engine_scene::scene::Scene::load(&lib.index,name)?;
        let source=&donor.entries.iter().find(|e|e.idx==donor.start+3).context("Donor bundle")?.bytes;
        let raw=table::decode_slot_by_type(source,1).context("Donor TIM list")?.map_err(anyhow::Error::msg)?;
        let mut inventory=NativeAssetPlan::default();let mut events=Vec::new();let mut expected=Vram::new();
        for (i,tim_bytes) in members(&raw)?.iter().enumerate(){let texture=resource(&mut inventory,ResourceKind::Tim,tim_bytes);let tim=legaia_tim::parse(tim_bytes)?;let clut=tim.clut.is_some();expected.upload_tim(&tim);events.push(Upload{sequence:Some(i as u32),resource:texture,image:true,clut,merge_clut_zeros:false});}
        let trace=UploadTrace::verify_native_replay(&inventory,&events,&expected,"Verified donor descriptor TIM-list member order; native uploader flag masks; passive CLUT walker")?;
        ensure!(table::decode_slot_by_type(source,6).context("Donor CLUT walker")?.map_err(anyhow::Error::msg)?==[0,0,0,0],"Active donor CLUT walker");
        let donor_map=lib.index.entry_bytes(donor.start)?;let pool=table::mesh_pool(source);
        let mut choices=field_objects::parse_placements(&donor_map);choices.retain(|p|p.anchor_cell&field_objects::CELL_BIND_OWNED==0);
        choices.sort_by_key(|p|{let r=field_objects::ObjectRecord::parse(&donor_map,p.obj_idx as usize).unwrap();field_objects::pack_mesh_index(p.obj_idx,&r).and_then(|i|pool.get(i as usize)).map(|m|m.bytes.len()).unwrap_or(usize::MAX)});
        let mut selected=None;let mut last_error=String::new();
        for placement in choices {
            let record=field_objects::ObjectRecord::parse(&donor_map,placement.obj_idx as usize).context("Donor record")?;
            let Some(index)=field_objects::pack_mesh_index(placement.obj_idx,&record) else{continue};let Some(model)=pool.get(index as usize) else{continue};
            let id=resource(&mut inventory,ResourceKind::Tmd,&model.bytes);
            let attempt=(||->Result<_>{let plan=Planner::new(&before,&[])?.plan_multi(&inventory,&trace,&[id.clone()])?;let result=register_scene(&inventory,bundle,Some(pac),&before,&plan,budgets())?;
                validate_stored_extent(result.bytes().len().div_ceil(2048)*2048,budgets())?;validate_stored_extent(result.pac_bytes().unwrap().len().div_ceil(2048)*2048,budgets())?;Ok((plan,result))})();
            match attempt {Ok((plan,result))=>{selected=Some((id,plan,result,placement,index));break;},Err(error)=>last_error=format!("{error:#}")}
        }
        let (id,relocation,result,placement,source_selector)=selected.with_context(||format!("No unbound donor placement has a fitting native resource closure: {last_error}"))?;
        ensure!(table::mesh_pool(result.bytes())[result.selector(&id)? as usize].bytes==relocation.package(&before)?.tmd_pack[8..],"Foreign native model readback");
        let source_flags=relocation.source_resources().iter().filter(|id|id.kind==ResourceKind::Tim).map(|id|u32::from_le_bytes(inventory.resources[id].payload.as_deref().unwrap()[4..8].try_into().unwrap())&!15).collect::<BTreeSet<_>>();
        for tim in members(&relocation.package(&before)?.tim_pack)?{ensure!(source_flags.contains(&(legaia_tim::parse_native_upload(&tim)?.flags&!15)),"Native source flag metadata lost");}
        let (compiled,registered)=super::super::super::ps1_export::NativeExportPlan::compile_scene_resources(&original,destination.start as usize,&inventory,&before,&relocation,budgets())?;
        let source_tiers=donor.field_floor_height_lut(&lib.index)?.context("Donor native floor ladder missing")?.map(|y|Some(-(y as i32)));
        let destination_tiers=destination.field_floor_height_lut(&lib.index)?.context("Destination native floor ladder missing")?.map(|y|Some(-(y as i32)));
        let delta=[12096.0-placement.world_x as f32,0.0,10048.0-placement.world_z as f32];
        let candidate=super::super::super::ps1_export::NativeExportPlan::compile_registered_copy(compiled,destination.start as usize,donor.start as usize,(placement.col,placement.row),73,&id,&registered,delta,&source_tiers,&destination_tiers,None,false)?;
        ensure!(candidate.uid==73&&candidate.selector==registered.selector(&id)?&&!candidate.image_bytes().is_empty(),"Combined candidate identity/readback failed");
        println!("{name} native object #{} model {} source cell ({},{}) world ({},{}) floor tier {:?} -> town01: independent descriptor {}, grid {:?}, selector {}, requested world (12096,10048), bundle {} -> {}, PAC {} -> {}; {}",placement.obj_idx,source_selector,placement.col,placement.row,placement.world_x,placement.world_z,placement.floor_nibble,candidate.descriptor,candidate.cell,candidate.selector,bundle.len(),result.bytes().len(),pac.len(),result.pac_bytes().unwrap().len(),candidate.unresolved());examined+=1;
        let assembled_donor=legaia_engine_world_views::scene_staging::assemble_field_scene(&lib.index,name).map_err(anyhow::Error::msg)?;
        let key=assembled_donor.placements.iter().position(|p|p.cell==(placement.col,placement.row)&&p.env_slot==source_selector as usize).context("Repository donor draw missing")?;
        let (_,points)=super::super::super::world_view::foreign_original_geometry(&assembled_donor,"placement",key).map_err(anyhow::Error::msg)?;
        let center=[0,1,2].map(|axis|{let lo=points.iter().map(|p|p[axis]).fold(f32::INFINITY,f32::min);let hi=points.iter().map(|p|p[axis]).fold(f32::NEG_INFINITY,f32::max);(lo+hi)*0.5+delta[axis]});
        editor.insert_foreign(&assembled_donor,"placement",key,100+examined as u64,center,&lib.index).map_err(anyhow::Error::msg)?;
    }
    ensure!(examined==2,"Both source-town fixtures required");
    let saved=super::super::super::scene_instances::serialize("town01",&editor);
    let mut restored=super::super::super::world_view::build_world(&assembled_destination,&lib.index);
    let doc=super::super::super::scene_instances::parse(&saved,"town01",&restored).map_err(anyhow::Error::msg)?;
    super::super::super::scene_instances::apply(&doc,&mut restored,assembled_destination.placements.len(),&lib.index).map_err(anyhow::Error::msg)?;
    let candidate=super::super::super::ps1_export::NativeExportPlan::compile_saved_imports(original,&assembled_destination,&restored,&lib.index,budgets(),&[])?;
    ensure!(candidate.placements.len()==2&&candidate.placements[0].1!=candidate.placements[1].1,"Persisted multi-source instances merged");
    ensure!(candidate.pending_collision_recipes>0,"Authored collision intent lost");
    let rejection=super::super::super::ps1_export::plan_export(&path,&assembled_destination,&restored,&lib.index).err().context("Normal export unexpectedly accepted unresolved collision")?;
    ensure!(rejection.to_string().contains("Authored collision recipes are in-memory only"),"Unexpected normal export rejection: {rejection:#}");
    println!("Saved/reopened multi-town World Objects: {:?}; {} pending collision recipes; normal export still rejects unresolved authored collision",candidate.placements,candidate.pending_collision_recipes);
    for diagnostic in &candidate.collision_diagnostics {println!("{diagnostic}");}
    ensure!(candidate.native_walls.is_none(),"Automatic mesh proposals were silently promoted");
    let baseline=candidate.native_map(destination.start as usize)?;
    // Explicit fixture authoring exercises native byte serialization, not a
    // claim that the human has reviewed these shapes or retail physics agrees.
    let mut reviewed=restored.clone();
    for assignment in restored.collision.recipes.iter().cloned() {
        let mut recipe=assignment.recipe;recipe.evidence=super::super::super::collision_recipes::Evidence::UserAuthored{note:"Explicit native regression fixture authoring; not verified retail collision or user approval".into()};
        let uid=match assignment.owner {super::super::super::collision_recipes::ObjectId::Created(uid)=>uid,_=>anyhow::bail!("Unexpected original recipe")};
        let draw=reviewed.draws.iter().find(|d|d.editor_uid==Some(uid)).context("Reviewed collision owner missing")?;
        let key=(draw.terrain,draw.index);reviewed.set_collision_recipe(key,recipe).map_err(anyhow::Error::msg)?;
    }
    let saved_reviewed=super::super::super::scene_instances::serialize("town01",&reviewed);
    let reviewed_doc=super::super::super::scene_instances::parse(&saved_reviewed,"town01",&restored).map_err(anyhow::Error::msg)?;
    super::super::super::scene_instances::apply(&reviewed_doc,&mut reviewed,assembled_destination.placements.len(),&lib.index).map_err(anyhow::Error::msg)?;
    let raw=legaia_disc_patch::disc::DiscPatcher::open(std::fs::read(&path)?)?;
    let observations=if let Some(dir)=std::env::var_os("LCK_RUNTIME_CAPTURE_DIR") {
        let dir=std::path::PathBuf::from(dir);let a=std::fs::read(dir.join("town01_postload_vram.bin"))?;let b=std::fs::read(dir.join("town01_settled_vram.bin"))?;
        let observed=super::super::runtime_evidence::VramObservations::from_raw(&[("postload",&a),("settled",&b)])?;
        println!("Capture-informed allocation: {}",observed.summary());Some(observed)
    }else{None};
    let authored=super::super::super::ps1_export::NativeExportPlan::compile_saved_imports_observed(raw,&assembled_destination,&reviewed,&lib.index,budgets(),&[],observations.as_ref())?;
    let plan=authored.native_walls.as_ref().with_context(||format!("Authored native collision failed: {:?}",authored.collision_diagnostics))?;
    ensure!(plan.owners==vec![101,102]&&plan.sources.len()==2&&authored.pending_collision_recipes==0,"Authored collision identities/provenance lost");
    let compiled=authored.native_map(destination.start as usize)?;
    ensure!(plan.apply(&baseline).map_err(anyhow::Error::msg)?==compiled,"Final MAP differs from the single compiled collision plan");
    for i in 0..baseline.len(){ensure!(baseline[i]==compiled[i]||((0x4000..0x8000).contains(&i)&&baseline[i]&15==compiled[i]&15&&baseline[i]&compiled[i]==baseline[i]),"Authored collision changed unrelated native data");}
    let before_floor=legaia_engine_field::minigame_floor::FloorGrid::new(&baseline);let after_floor=legaia_engine_field::minigame_floor::FloorGrid::new(&compiled);
    for z in 0..128 {for x in 0..128 {ensure!(before_floor.height_index(x,z)==after_floor.height_index(x,z)&&before_floor.cell(x,z)==after_floor.cell(x,z),"Native floor/grid reader found changed original data");}}
    println!("Real imported fixture authoring: {} added native wall bits / {} bytes, owners {:?}; native actor collision and walkable surfaces remain unverified; no IMG",plan.added_bits,plan.changed_bytes,plan.owners);
    let final_bundle=authored.native_map(destination.start as usize+3)?;let header=table::detect(&final_bundle).context("Final demand descriptor table")?;
    println!("Runtime demand (not a capacity certification): final model pool {} + five residents, decoded TMD pack {} bytes (original {}), decoded bundle total {} bytes",table::mesh_pool(&final_bundle).len(),header.used()[header.descriptor_index(2).unwrap()].size,table::detect(bundle).unwrap().used()[table::detect(bundle).unwrap().descriptor_index(2).unwrap()].size,header.total_decompressed_size);
    let original_uploads=pac_uploads(pac,&mut Vram::new())?.1;let final_pac=authored.native_map(destination.start as usize+4)?;
    for raw in pac_uploads(&final_pac,&mut Vram::new())?.1 {if !original_uploads.contains(&raw){let tim=legaia_tim::parse_native_upload(&raw)?;println!("Added native upload flags {:#010x}, image-word rect ({},{},{},{}); dynamic lifetime still requires runtime capture",tim.flags,tim.image.fb_x,tim.image.fb_y,tim.image.fb_w,tim.image.h);if let Some(observed)=&observations {ensure!(!observed.intersects(super::super::texture_relocation::Rect{x:tim.image.fb_x,y:tim.image.fb_y,w:tim.image.fb_w,h:tim.image.h})?,"New upload overlaps observed runtime activity");}}}
    if let Some(output)=std::env::var_os("LCK_VISUAL_EXPERIMENT_OUT") {
        let observed=observations.as_ref().context("Experimental visual IMG requires capture-informed exclusions")?;
        let ack=std::env::var("LCK_VISUAL_EXPERIMENT_ACK").context("Experimental visual acknowledgement missing")?;
        let mut visual=restored.clone();let mut uids=visual.foreign.iter().map(|f|f.uid).collect::<Vec<_>>();uids.sort();
        let outdoor=match std::env::var("LCK_VISUAL_EXPERIMENT_PLACEMENT").as_deref() {
            Ok("town01-outdoor-v1")=>true, Err(std::env::VarError::NotPresent)=>false,
            _=>anyhow::bail!("Unknown experimental placement fixture"),
        };
        let native_map=lib.index.entry_bytes(destination.start)?;
        let floor_lut=destination.field_floor_height_lut(&lib.index)?.context("Outdoor floor ladder missing")?;
        let recipes=visual.collision.recipes.clone();
        for (i,uid) in uids.into_iter().enumerate(){
            let draw=visual.draws.iter().find(|d|d.editor_uid==Some(uid)).context("Visual fixture owner missing")?;
            let key=(draw.terrain,draw.index);
            let offset=if outdoor {
                // Test placement only: native empty, wall-free tiles on the
                // outdoor approach west/north of Vahn's house (38,25).
                // Exact tile centres remove anchor/geometry displacement.
                let (col,row)=[(31usize,20usize),(32,20)][i];
                ensure!(native_map[0x4000+row*128+col]&0xf0==0&&native_map[0x8000+(row*128+col)*2..0x8000+(row*128+col)*2+2]==[0,0],"Outdoor target is no longer empty/wall-free");
                let source=&visual.collision.foreign_sources[&uid];
                ensure!((source.object.scene.as_str(),source.object.descriptor_id)==[("bylon",291),("izumi",36)][i],"Visual donor identity changed");
                let record=field_objects::ObjectRecord::parse(&source.descriptor,0).context("Visual donor descriptor missing")?;
                let cached=visual.foreign.iter().find(|f|f.uid==uid).context("Visual donor cache missing")?;
                let current_origin=[field_objects::world_x(source.object.cell[0],record.x_off) as f32+draw.original_center[0]-cached.donor_center[0],
                    field_objects::world_z(source.object.cell[1],record.z_off) as f32+draw.original_center[2]-cached.donor_center[2]];
                let low=visual.model.as_ref().context("Visual mesh missing")?.rest[draw.vertex_start..draw.vertex_end].iter().map(|p|p[1]).fold(f32::INFINITY,f32::min);
                let ground=floor_lut[(native_map[0x4000+row*128+col]&15) as usize] as f32;
                ensure!(low.is_finite(),"Visual mesh has no finite lower bound");
                // Preview geometry can include fractional coplanar lifts.
                // Round UP to a whole native unit; do not weaken quantization.
                let offset=[col as f32*128.0+64.0-current_origin[0],(ground+48.0-low).ceil(),row as f32*128.0+64.0-current_origin[1]];
                println!("Outdoor UID {uid}: target ({},{}), native ground Y {}, mesh bottom clearance {} (native-unit rounding), editor offset {:?}",col*128+64,row*128+64,-ground,low+offset[1]-ground,offset);
                offset
            }else{[i as f32*128.0,0.0,0.0]};
            ensure!(visual.set_offset(key.0,key.1,offset),"Visual fixture positioning failed");
        }
        ensure!(visual.collision.recipes==recipes,"Repositioning altered collision proposals");
        if outdoor {
            let disc=legaia_disc_patch::disc::DiscPatcher::open(std::fs::read(&path)?)?;
            let check=super::super::super::ps1_export::NativeExportPlan::compile_saved_imports_observed(disc,&assembled_destination,&visual,&lib.index,budgets(),&[],Some(observed))?;
            let map=check.native_map(destination.start as usize)?;
            // Ported retail window sweep, not a runtime visibility guarantee.
            // A small outdoor window must actually create both drawn actors.
            let spawns=legaia_engine_field::field_regions::window_rebuild_spawns(&map,(29,18,35,24),floor_lut.map(|y|-y));
            for (i,&(uid,id,cell,_)) in check.placements.iter().enumerate(){
                ensure!(cell==[(31,20),(32,20)][i],"Outdoor allocator did not use the exact intended empty tile");
                let spawn=spawns.iter().find(|s|s.descriptor as usize==id&&s.tile==cell).context("Outdoor copy absent from ported window sweep")?;
                ensure!(spawn.drawn(),"Outdoor copy has a no-draw template");
                println!("Outdoor UID {uid}: descriptor {id}, spawn {:?}, world {:?}, draw kind {}, footprint anchor {:?}",cell,spawn.world,spawn.template_kind,spawn.anchor);
            }
            ensure!(check.placements.len()==2&&check.native_walls.is_none()&&check.pending_collision_recipes==2,"Outdoor visual-only contract changed");
        }
        super::super::super::ps1_export::experimental::export_visual(&path,&std::path::PathBuf::from(&output),&assembled_destination,&visual,&lib.index,observed,&ack)?;
        println!("EXPERIMENTAL VISUAL ONLY IMG created: {}. Runtime/physics unverified; see manifest; normal exporter unchanged",std::path::PathBuf::from(output).display());
    }
    Ok(())
}
#[test]
fn global_model_boundary_199_does_not_certify_safety_and_200_rejects() {
    let below=ModelRegistrationDiagnostic::assess(199,1).unwrap();below.reject_known_collision().unwrap();
    assert!(below.summary().contains("199 existing; 1 proposed additional; 200 resulting; first proposed index 199"));
    assert!(below.summary().contains("not certified safe"));
    for (existing,additional) in [(200,1),(199,2),(201,0)] {
        let diagnostic=ModelRegistrationDiagnostic::assess(existing,additional).unwrap();
        let error=diagnostic.reject_known_collision().unwrap_err().to_string();
        assert!(error.contains("index 200")&&error.contains("0x8007C338"));
        assert!(error.contains(&format!("{existing} existing; {additional} proposed additional")));
    }
    ModelRegistrationDiagnostic::assess(200,0).unwrap().reject_known_collision().unwrap(); // reuse adds no slot
    assert!(ModelRegistrationDiagnostic::assess(usize::MAX,1).is_err());
}
#[test]
fn registration_counts_resident_slots_and_retains_stricter_limits() {
    let (source,before,inventory,relocation,id)=fixture();let (_,old)=slots(&source,budgets()).unwrap();
    let native=members(&old[1].decoded).unwrap()[0].clone();
    for scene_models in [194,195,196] {
        let mut payloads=old.iter().map(|s|s.decoded.clone()).collect::<Vec<_>>();payloads[1]=pack(&vec![native.clone();scene_models]).unwrap();
        let large=bundle(&payloads);let preserved=large.clone();
        let result=super::register(&inventory,&large,&before,&relocation,budgets());
        if scene_models==194 {
            let accepted=result.unwrap();assert_eq!(accepted.selector(&id).unwrap(),194); // global new index = 194+5 = 199
            assert!(accepted.model_diagnostic().contains("199 existing; 1 proposed additional"));
            assert!(accepted.model_diagnostic().contains("not certified safe"));
            assert!(super::register(&inventory,&large,&before,&relocation,Limits{max_models:194,..budgets()}).is_err());
            let reused=super::register(&inventory,accepted.bytes(),&before,&relocation,budgets()).unwrap();
            assert!(reused.model_diagnostic().contains("200 existing; 0 proposed additional"));
        } else {
            let error=result.err().unwrap().to_string();assert!(error.contains("Model registration reaches index 200"));
            assert!(error.contains(&format!("{} existing; 1 proposed additional",scene_models+5)));
        }
        assert_eq!(large,preserved);
    }
}
#[test]
fn plain_bundle_registration_resolves_native_models_and_preserves_other_slots() {
    let (source,before,inventory,relocation,id)=fixture();
    let result=super::super::super::ps1_export::NativeExportPlan::register_resources_fixture(&inventory,&source,&before,&relocation,budgets()).unwrap();
    result.validate_source(&source).unwrap();assert_eq!(result.selector(&id).unwrap(),1);
    let pool=table::mesh_pool(result.bytes());assert_eq!(pool.len(),2);assert_eq!(pool[0].bytes,table::mesh_pool(&source)[0].bytes);
    assert_eq!(field_objects::FIELD_ACTOR_PACK_BIAS+result.selector(&id).unwrap(),6);
    let (_,old)=slots(&source,budgets()).unwrap();let (_,new)=slots(result.bytes(),budgets()).unwrap();
    for old_slot in &old[2..7] {let new_slot=new.iter().find(|s|s.kind==old_slot.kind).unwrap();
        // Sector padding belongs to the final stored span; all original opaque
        // bytes must remain exact, and only zero alignment bytes may follow.
        assert!(new_slot.stored.starts_with(&old_slot.stored));assert!(new_slot.stored[old_slot.stored.len()..].iter().all(|b|*b==0));assert_eq!(old_slot.decoded,new_slot.decoded);}
    assert!(new.iter().any(|s|s.kind==0&&s.suffix==old[0].suffix));assert_eq!(old[1].suffix,new.iter().find(|s|s.kind==2).unwrap().suffix);
    let mut descriptor=[0u8;32];descriptor[18]=4;let updated=result.bind_record(&id,&descriptor).unwrap();
    for i in 0..32 {if i!=16&&i!=17 {assert_eq!(updated[i],descriptor[i]);}}
    let record=field_objects::ObjectRecord::parse(&updated,0).unwrap();let selector=field_objects::pack_mesh_index(4,&record).unwrap();
    assert_eq!(pool[selector as usize].bytes,relocation.package(&before).unwrap().tmd_pack[8..]);
    // Registering the same immutable package again keeps both original and
    // new selectors stable, without creating or merging placed instances.
    let again=super::register(&inventory,result.bytes(),&before,&relocation,budgets()).unwrap();assert_eq!(again.selector(&id).unwrap(),1);assert_eq!(again.bytes(),result.bytes());
    let mut stale=source.clone();stale[4]^=1;assert!(result.validate_source(&stale).is_err());
}
#[test]
fn existing_native_tim_metadata_flags_are_preserved_without_normalization() {
    let (source,before,inventory,relocation,_)=fixture();let (_,old)=slots(&source,budgets()).unwrap();
    let mut payloads=old.iter().map(|s|s.decoded.clone()).collect::<Vec<_>>();let mut tims=members(&payloads[0]).unwrap();
    let flags=u32::from_le_bytes(tims[0][4..8].try_into().unwrap())|0x80010000;tims[0][4..8].copy_from_slice(&flags.to_le_bytes());payloads[0]=pack(&tims).unwrap();
    let native=bundle(&payloads);let result=super::register(&inventory,&native,&before,&relocation,budgets()).unwrap();
    let (_,after)=slots(result.bytes(),budgets()).unwrap();assert_eq!(after[0].kind,0);assert_eq!(after[0].decoded,tims[0]);
}
#[test]
fn native_tim_member_suffixes_are_retained_but_not_uploaded() {
    let (source,before,inventory,relocation,_)=fixture();
    let (_,old)=slots(&source,budgets()).unwrap();
    let mut payloads=old.iter().map(|s|s.decoded.clone()).collect::<Vec<_>>();
    let mut tims=members(&payloads[0]).unwrap();
    tims[0].extend_from_slice(&[0xde,0xad,0xbe,0xef,0x10,0,0,0]);
    payloads[0]=pack(&tims).unwrap();
    let native=bundle(&payloads);
    let (_,replay)=native_texture_sequence(&native,None,budgets()).unwrap();
    assert_eq!(crate::app::naming::material_key(&replay),crate::app::naming::material_key(&before));
    let registered=super::register(&inventory,&native,&before,&relocation,budgets()).unwrap();
    let (_,after)=slots(registered.bytes(),budgets()).unwrap();
    assert_eq!(after[0].kind,0);assert_eq!(after[0].decoded,tims[0]);
    let mut pac=table::encode_size_word(0,tims[0].len() as u32).to_le_bytes().to_vec();
    pac.extend_from_slice(&tims[0]);pac.extend_from_slice(&[0;4]);
    let mut uploaded=Vram::new();let (_,uploads)=pac_uploads(&pac,&mut uploaded).unwrap();
    assert_eq!(uploads[0],tims[0][..legaia_tim::parse_native_upload(&tims[0]).unwrap().byte_extent()]);
    assert_eq!(crate::app::naming::material_key(&uploaded),crate::app::naming::material_key(&before));
}
#[test]
fn oversized_provisional_pac_selects_compressed_carrier_before_budget_check() {
    let (bundle,mut pac,before,inventory,relocation,id)=streamed_fixture();
    // Opaque bytes after the terminator must survive. The appended raw
    // candidate grows beyond the arena, while the compressed route fits.
    pac.resize(BUNDLE_ARENA/2048*2048,0);*pac.last_mut().unwrap()=0x5a;
    let result=register_scene(&inventory,&bundle,Some(&pac),&before,&relocation,budgets()).unwrap();
    assert_eq!(result.pac_bytes(),Some(pac.as_slice()));
    assert_eq!(result.selector(&id).unwrap(),1);
    let (_,header)=native_descriptor_table(result.bytes()).unwrap();
    assert!(header.descriptor_index(1).is_none());assert!(header.descriptor_index(0).is_some());
    let (_,actual)=native_texture_sequence(result.bytes(),result.pac_bytes(),budgets()).unwrap();
    assert_eq!(crate::app::naming::material_key(&actual),crate::app::naming::material_key(&relocation.apply(&before).unwrap()));
}
#[test]
fn invalid_boundaries_duplicates_dynamic_dependencies_and_limits_fail_closed() {
    let (source,before,inventory,relocation,_)=fixture();let saved=source.clone();
    for limits in [Limits{max_models:1,..budgets()},Limits{max_encoded:source.len(),..budgets()},Limits{max_decoded:table::detect(&source).unwrap().descriptor_size_sum() as usize,..budgets()}] {
        assert!(super::register(&inventory,&source,&before,&relocation,limits).is_err());assert_eq!(source,saved);
    }
    for bad in [
        {let mut b=source.clone();b[24+3]=2;b}, // duplicate TMD descriptor
        {let mut b=source.clone();b[12..16].copy_from_slice(&0u32.to_le_bytes());b},
        {let mut b=source.clone();b[20..24].copy_from_slice(&0xfffffff0u32.to_le_bytes());b},
        {let mut b=source.clone();b[56+3]=0x14;b},
        {let mut b=source.clone();let old=(u32::from_le_bytes(b[8..12].try_into().unwrap())&0xffffff) as usize;
            let total=u32::from_le_bytes(b[4..8].try_into().unwrap()) as usize-old+TIM_LIST_BUFFER+4;b[4..8].copy_from_slice(&(total as u32).to_le_bytes());b[8..12].copy_from_slice(&table::encode_size_word(1,(TIM_LIST_BUFFER+4) as u32).to_le_bytes());b},
        {let mut b=source.clone();b.resize(BUNDLE_ARENA+1,0);b},
        {let mut b=source.clone();b[4]^=1;b},source[..source.len()/2].to_vec(),
    ] {assert!(super::register(&inventory,&bad,&before,&relocation,budgets()).is_err());}
    let (_,old)=slots(&source,budgets()).unwrap();let mut payloads=old.iter().map(|s|s.decoded.clone()).collect::<Vec<_>>();payloads[5]=vec![1,0,0,0];
    assert!(super::register(&inventory,&bundle(&payloads),&before,&relocation,budgets()).is_err());
    let mut stale=inventory.clone();let id=relocation.source_resources().into_iter().next().unwrap();stale.resources.get_mut(&id).unwrap().payload=Some(std::sync::Arc::from([0u8;8]));
    assert!(super::register(&stale,&source,&before,&relocation,budgets()).is_err());
}
#[test]
#[ignore = "explicit read-only registration into an authentic plain native bundle"]
fn real_plain_bundle_registration_uses_descriptor_lookup() -> Result<()> {
    let path=std::path::PathBuf::from(std::env::var_os("LCK_P0_SOURCE").context("LCK_P0_SOURCE required")?);
    let lib=crate::library::Library::open(&path)?;let mut accepted=0;
    for name in ["town01","bylon","izumi"] {
        let scene=legaia_engine_scene::scene::Scene::load(&lib.index,name)?;
        let Some(legaia_engine_scene::scene_bundle::BundleSource::Plain{entry,..})=legaia_engine_scene::scene_bundle::find_bundle(&scene) else {continue};
        let source=entry.bytes.as_slice();
        ensure!(scene.entries.iter().filter(|e|!table::mesh_pool(&e.bytes).is_empty()).count()==1,"Multiple scene mesh-bundle contributions need global selector mapping");
        let (_,decoded)=match slots(source,budgets()){Ok(value)=>value,Err(error)=>{println!("{name}: explicit format rejection {error:#}");continue}};
        let texture_slot=decoded.iter().find(|s|s.kind==1).unwrap();let mut before=Vram::new();
        if let Err(error)=upload_pack(&texture_slot.decoded,&mut before){println!("{name}: native TIM-list adapter rejected {error:#}");continue;}
        let old=table::mesh_pool(source);let (inventory,relocation,id)=incoming(&before);
        let registered=super::super::super::ps1_export::NativeExportPlan::register_resources_fixture(&inventory,source,&before,&relocation,budgets())?;
        let selector=registered.selector(&id)?;let new=table::mesh_pool(registered.bytes());ensure!(new.len()>=old.len() && new[selector as usize].slot==1,"Native descriptor model lookup failed");
        for (a,b) in old.iter().zip(&new){ensure!(a.bytes==b.bytes && a.member==b.member,"Original registration changed");}
        println!("{name}: PROT {}, models {} -> {}, registered selector {}, field pool index {}, encoded bytes {} -> {}; in-memory only",entry.idx,old.len(),new.len(),selector,field_objects::FIELD_ACTOR_PACK_BIAS+selector,source.len(),registered.bytes().len());accepted+=1;
    }
    ensure!(accepted>0,"No actual plain bundle met verified adapter contract");Ok(())
}
