use super::*;
use super::super::tests::{tim,model,register};
fn fixture(shared_palette:bool)->(NativeAssetPlan,Vec<Upload>,ResourceId,Vram) {
    let mut inventory=NativeAssetPlan::default();let first=register(&mut inventory,ResourceKind::Tim,&tim(0,31));
    let mut raw=tim(0,992);let header=8+12+32;raw[header+4..header+6].copy_from_slice(&256u16.to_le_bytes());
    raw[header+12..].fill(0x54); // Distinct indexed image, not just a different palette.
    raw[12..14].copy_from_slice(&if shared_palette{0u16}else{32u16}.to_le_bytes());
    let second=inventory.register(ResourceKind::Tim,&raw,Origin{scene:"another-town".into(),entry:Some(19),offset:Some(0)},false).unwrap();
    let mut a=legaia_tmd::encode::decode_model(&legaia_tmd::parse(&model(480<<6,2,vec![(0,0),(7,0),(0,7)])).unwrap(),&model(480<<6,2,vec![(0,0),(7,0),(0,7)])).unwrap();
    let raw_b=model((480<<6)|if shared_palette{0}else{2},4,vec![(0,0),(7,0),(0,7)]);
    a.extend(legaia_tmd::encode::decode_model(&legaia_tmd::parse(&raw_b).unwrap(),&raw_b).unwrap());
    let mesh=register(&mut inventory,ResourceKind::Tmd,&legaia_tmd::encode::encode(&a).unwrap());
    let events=vec![Upload{sequence:Some(0),resource:first,image:true,clut:true,merge_clut_zeros:false},
        Upload{sequence:Some(1),resource:second,image:true,clut:!shared_palette,merge_clut_zeros:false}];
    let expected=replay(&inventory,&events);(inventory,events,mesh,expected)
}
fn replay(inventory:&NativeAssetPlan,events:&[Upload])->Vram {
    let mut vram=Vram::new();for event in events {let tim=legaia_tim::parse_strict(payload(inventory,&event.resource,ResourceKind::Tim).unwrap()).unwrap();vram.upload_tim_partial_opts(&tim,event.image,event.clut,event.merge_clut_zeros);}vram
}
#[test]
fn multiple_images_shared_or_separate_palettes_package_and_reuse() {
    for shared in [false,true] {
        let (inventory,events,mesh,expected)=fixture(shared);
        let trace=UploadTrace::verify_replay(&inventory,&events,&expected,"Synthetic explicit DMA sequence").unwrap();
        let mut planner=Planner::new(&Vram::new(),&[]).unwrap();let plan=planner.plan_multi(&inventory,&trace,&[mesh.clone()]).unwrap();
        assert_eq!(plan.evidence,"Synthetic explicit DMA sequence");
        assert_eq!(plan.uploads.len(),if shared{3}else{4});assert!(plan.uploads.iter().any(|u|u.origins.iter().any(|o|o.scene=="another-town")));
        assert!(plan.uploads.iter().all(|u|events.iter().any(|e|e.resource==u.source)));
        let blocks=planner.blocks.len();let repeated=planner.plan_multi(&inventory,&trace,&[mesh.clone()]).unwrap();assert_eq!(planner.blocks.len(),blocks);assert_eq!(plan.tmds,repeated.tmds);
        let package=plan.package(&Vram::new()).unwrap();assert_eq!(package.model_slots[&mesh],0);
        let images=legaia_container_formats::pack::extract_pack(&package.tim_pack).unwrap();assert_eq!(images.len(),plan.uploads.len());
        let models=legaia_container_formats::pack::extract_pack(&package.tmd_pack).unwrap();assert_eq!(&models[0][..plan.tmds[&mesh].len()],plan.tmds[&mesh]);
        let mut readback=Vram::new();for raw in images {readback.upload_tim(&legaia_tim::parse_strict(raw).unwrap());}
        assert_eq!(super::super::super::super::naming::material_key(&readback),super::super::super::super::naming::material_key(&plan.apply(&Vram::new()).unwrap()));
        let mut changed=Vram::new();changed.write_block(900,400,1,1,&[0,0]);assert!(plan.package(&changed).is_err());
        let original=payload(&inventory,&mesh,ResourceKind::Tmd).unwrap();let before=references(original).unwrap();let after=references(&plan.tmds[&mesh]).unwrap();
        assert_eq!(before.len(),after.len());for (a,b) in before.iter().zip(after){assert_eq!(a.uvs,b.uvs);assert_eq!(a.tsb&!31,b.tsb&!31);}
        // Generated native packs can be decoded back into the same dependency
        // path, including palettes supplied by ordinary 16-bit image uploads.
        let mut packed=NativeAssetPlan::default();let mut upload_events=Vec::new();
        for (i,raw) in legaia_container_formats::pack::extract_pack(&package.tim_pack).unwrap().iter().enumerate() {
            let n=legaia_tim::parse_strict(raw).unwrap().byte_extent();let resource=register(&mut packed,ResourceKind::Tim,&raw[..n]);
            upload_events.push(Upload{sequence:Some(i as u32),resource,image:true,clut:false,merge_clut_zeros:false});
        }
        let relocated=register(&mut packed,ResourceKind::Tmd,&plan.tmds[&mesh]);
        let trace=UploadTrace::verify_replay(&packed,&upload_events,&readback,"Native package parse/replay fixture").unwrap();
        assert!(Planner::new(&Vram::new(),&[]).unwrap().plan_multi(&packed,&trace,&[relocated]).is_ok());
    }
}
#[test]
fn ordering_overwrites_and_unknown_order_are_explicit() {
    let (mut inventory,mut events,mesh,_)=fixture(true);
    let mut raw=tim(0,0x7c00);let header=8+12+32;raw[header+4..header+6].copy_from_slice(&256u16.to_le_bytes());
    let last=register(&mut inventory,ResourceKind::Tim,&raw);events.push(Upload{sequence:Some(2),resource:last.clone(),image:true,clut:false,merge_clut_zeros:false});
    let expected=replay(&inventory,&events);let trace=UploadTrace::verify_replay(&inventory,&events,&expected,"Ordered overwrite fixture").unwrap();
    let plan=Planner::new(&Vram::new(),&[]).unwrap().plan_multi(&inventory,&trace,&[mesh.clone()]).unwrap();assert!(plan.uploads.iter().any(|u|u.source==last));
    let mut wrong=events.clone();wrong[0].sequence=None;assert!(UploadTrace::verify_replay(&inventory,&wrong,&expected,"Scanner order").is_err());
    // Identical writes commute and cannot reveal chronology. Make an overwrite differ, then
    // verify whole-reference replay detects that order instead of relying on hashes.
    let id=&events[2].resource;let mut changed=payload(&inventory,id,ResourceKind::Tim).unwrap().to_vec();let end=changed.len();changed[end-128..].fill(0x11);
    events[2].resource=register(&mut inventory,ResourceKind::Tim,&changed);
    let expected=replay(&inventory,&events);let mut reversed=events.clone();reversed.swap(1,2);for (i,e) in reversed.iter_mut().enumerate(){e.sequence=Some(i as u32);}
    assert!(UploadTrace::verify_replay(&inventory,&reversed,&expected,"Reversed noncommuting writes").is_err());
}

#[test]
#[ignore = "explicit read-only cross-town native resource package fixture"]
fn cross_town_native_payloads_form_replay_checked_packages() -> Result<()> {
    let source=std::path::PathBuf::from(std::env::var_os("LCK_P0_SOURCE").context("LCK_P0_SOURCE required")?);
    let lib=crate::library::Library::open(&source)?;let mut planner=Planner::new(&Vram::new(),&[])?;
    for name in ["bylon","izumi"] {
        let scene=legaia_engine_scene::scene::Scene::load(&lib.index,name)?;let mut selected=None;
        for location in legaia_engine_scene::scene_bundle::scene_tim_layout(&scene)? {
            let entry=scene.entries.iter().find(|e|e.idx==location.entry_idx).context("TIM entry missing")?;
            let raw=&entry.bytes[location.offset..location.offset+location.byte_len];let tim=legaia_tim::parse_strict(raw)?;
            if let Ok((image,Some(clut),depth))=rectangles(&tim) {
                let divisor=if depth==0{4}else{2};
                if depth<2 && image.w>=2 && image.h>=2 && image.x%64*divisor<255 && image.y%256<255 && clut.w>=if depth==0{16}else{256} {
                    selected=Some((raw.to_vec(),tim,location));break;
                }
            }
        }
        let (raw,tim,location)=selected.context("No bounded native TIM package")?;
        let mut inventory=NativeAssetPlan::default();let texture=inventory.register(ResourceKind::Tim,&raw,Origin{scene:name.into(),entry:Some(location.entry_idx),offset:Some(location.offset)},false)?;
        let depth=if tim.mode==PixelMode::Bpp4{0}else{1};let divisor=if depth==0{4}else{2};
        let u=(tim.image.fb_x%64*divisor) as u8;let v=(tim.image.fb_y%256) as u8;let c=tim.clut.as_ref().unwrap();
        let mesh=register(&mut inventory,ResourceKind::Tmd,&model(c.fb_x/16|c.fb_y<<6,tim.image.fb_x/64|(tim.image.fb_y/256)<<4|depth<<7,vec![(u,v),(u+1,v),(u,v+1)]));
        let events=[Upload{sequence:Some(0),resource:texture,image:true,clut:true,merge_clut_zeros:false}];
        let expected=replay(&inventory,&events);let trace=UploadTrace::verify_replay(&inventory,&events,&expected,"Single native TIM consumer replay; no scene-wide chronology claim")?;
        let before=planner.vram.clone();let plan=planner.plan_multi(&inventory,&trace,&[mesh])?;let package=plan.package(&before)?;
        println!("{name}: native TIM {} +{:#x}; {} upload blocks, TIM pack {} bytes, TMD pack {} bytes; synthetic native primitive references; no IMG",location.entry_idx,location.offset,plan.uploads.len(),package.tim_pack.len(),package.tmd_pack.len());
    }Ok(())
}
#[test]
fn external_palette_fragmentation_and_failure_rollback() {
    let (mut inventory,events,mesh,expected)=fixture(false);
    let trace=UploadTrace::verify_replay(&inventory,&events,&expected,"Explicit fixture").unwrap();
    let mut stale=inventory.clone();stale.resources.get_mut(&events[0].resource).unwrap().payload=Some(std::sync::Arc::from([0u8;8]));
    let mut unchanged=Planner::new(&Vram::new(),&[]).unwrap();assert!(unchanged.plan_multi(&stale,&trace,&[mesh.clone()]).is_err());assert!(unchanged.blocks.is_empty());
    let mut planner=Planner::new(&Vram::new(),&[Rect{x:0,y:0,w:1024,h:512}]).unwrap();
    assert!(planner.plan_multi(&inventory,&trace,&[mesh.clone()]).is_err());assert!(planner.blocks.is_empty());
    let mut missing=events.clone();for e in &mut missing{e.clut=false;}
    let expected=replay(&inventory,&missing);let trace=UploadTrace::verify_replay(&inventory,&missing,&expected,"Missing palette fixture").unwrap();
    assert!(Planner::new(&Vram::new(),&[]).unwrap().plan_multi(&inventory,&trace,&[mesh.clone()]).is_err());
    let mut overwrite=tim(0,1);let h=8+12+32;overwrite[h+4..h+6].copy_from_slice(&129u16.to_le_bytes());overwrite[h+8..h+10].copy_from_slice(&1u16.to_le_bytes());
    overwrite[h..h+4].copy_from_slice(&28u32.to_le_bytes());overwrite.truncate(h+28);
    let id=register(&mut inventory,ResourceKind::Tim,&overwrite);let mut fragmented=events;fragmented.push(Upload{sequence:Some(2),resource:id,image:true,clut:false,merge_clut_zeros:false});
    let expected=replay(&inventory,&fragmented);let trace=UploadTrace::verify_replay(&inventory,&fragmented,&expected,"Partial image overwrite").unwrap();
    let mut planner=Planner::new(&Vram::new(),&[]).unwrap();assert!(planner.plan_multi(&inventory,&trace,&[mesh]).is_err());assert!(planner.blocks.is_empty());
}

#[test]
fn equal_palettes_share_native_uploads_without_losing_source_provenance() {
    let (mut inventory,mut events,mesh,_)=fixture(false);
    let first=payload(&inventory,&events[0].resource,ResourceKind::Tim).unwrap().to_vec();
    let mut second=payload(&inventory,&events[1].resource,ResourceKind::Tim).unwrap().to_vec();second[20..52].copy_from_slice(&first[20..52]);
    events[1].resource=register(&mut inventory,ResourceKind::Tim,&second);
    let expected=replay(&inventory,&events);let trace=UploadTrace::verify_replay(&inventory,&events,&expected,"Equal palette fixture").unwrap();
    let mut planner=Planner::new(&Vram::new(),&[]).unwrap();let plan=planner.plan_multi(&inventory,&trace,&[mesh]).unwrap();
    assert_eq!(plan.uploads.len(),4);assert_eq!(planner.blocks.len(),3);
    let package=plan.package(&Vram::new()).unwrap();assert_eq!(legaia_container_formats::pack::parse_pack(&package.tim_pack).unwrap().len(),3);
    assert!(events.iter().all(|e|plan.uploads.iter().any(|u|u.source==e.resource)));
}
#[test]
fn linear_multirow_clut_and_composite_merge_semantics() {
    let (mut inventory,mut events,_,_)=fixture(false);
    let mut raw=tim(0,31);raw[16..18].copy_from_slice(&8u16.to_le_bytes());raw[18..20].copy_from_slice(&2u16.to_le_bytes());
    // Same 16 palette words, stored as 8x2 but consumed linearly as 16x1.
    let id=register(&mut inventory,ResourceKind::Tim,&raw);events=vec![Upload{sequence:Some(0),resource:id.clone(),image:true,clut:true,merge_clut_zeros:false}];
    let expected=replay(&inventory,&events);let trace=UploadTrace::verify_replay(&inventory,&events,&expected,"Verified consumer linear CLUT fixture").unwrap();
    let mesh=register(&mut inventory,ResourceKind::Tmd,&model(480<<6,2,vec![(0,0),(7,0),(0,7)]));
    assert!(Planner::new(&Vram::new(),&[]).unwrap().plan_multi(&inventory,&trace,&[mesh.clone()]).is_ok());
    assert!(Planner::new(&Vram::new(),&[]).unwrap().plan(&inventory,&id,&[mesh.clone()],None).is_ok());
    let next=register(&mut inventory,ResourceKind::Tim,&tim(0,992));events.push(Upload{sequence:Some(1),resource:next,image:false,clut:true,merge_clut_zeros:true});
    let expected=replay(&inventory,&events);let trace=UploadTrace::verify_replay(&inventory,&events,&expected,"Known merge-zero trace").unwrap();
    assert!(Planner::new(&Vram::new(),&[]).unwrap().plan_multi(&inventory,&trace,&[mesh]).is_err());
}
