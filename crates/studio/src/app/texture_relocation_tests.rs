use super::*;
use legaia_tmd::{encode::{ModelObject,ModelGroup,ModelPrim,LEGAIA_OBJECT_SCALE},descriptor::PacketShape};

#[test]
fn unrestricted_export_respects_framebuffers_existing_textures_and_boot_words(){
    crate::app::ps1_export::unrestricted::with_test_scope(||{
        field_runtime_buffers_remain_reserved_even_without_tim_upload_coverage();
        let mut destination=Vram::new();destination.write_block(640,0,256,256,&vec![0;131072]);
        let mut planner=Planner::new(&destination,&field_runtime_reservations()).unwrap();
        let mut boot=Vram::new();boot.write_block(896,0,64,256,&vec![0;32768]);planner.reserve_written(&boot);
        let source=Rect{x:0,y:0,w:64,h:256};let target=planner.allocate(source,&vec![1;32768],0,None,&[]).unwrap();
        assert_eq!(target,Rect{x:960,y:0,w:64,h:256});
        assert!(!field_runtime_reservations().iter().any(|r|r.overlaps(target)));
        assert!(Rect{x:1000,y:511,w:256,h:1}.valid().is_err());
        assert!(planner.allocate(source,&vec![2;32768],0,Some([0,0]),&[]).is_err());
    });
}

#[test]
fn field_runtime_buffers_remain_reserved_even_without_tim_upload_coverage() {
    let mut planner=Planner::new(&Vram::new(),&field_runtime_reservations()).unwrap();
    for [x,y] in [[0,0],[319,479],[320,0],[639,511],[640,256],[655,511]] {assert!(planner.protected[y*1024+x]);}
    for [x,y] in [[0u16,0u16],[320,256],[640,256]] {
        assert!(planner.allocate(Rect{x,y,w:8,h:8},&[0;128],0,Some([x,y]),&[]).is_err());
    }
    let mut shared=Vram::new();shared.write_block(832,0,8,8,&[0;128]);planner.reserve_written(&shared);
    assert!(planner.allocate(Rect{x:832,y:0,w:8,h:8},&[0;128],0,Some([832,0]),&[]).is_err(),"zero-valued shared resources are still occupied");
}

#[test]
fn observed_transient_vram_data_blocks_reuse_after_it_clears() {
    let zero=vec![0u8;1024*512*2];let mut busy=zero.clone();busy[(64*1024+880)*2]=1;
    let observed=super::super::runtime_evidence::VramObservations::from_raw(&[("postload",&busy),("settled",&zero)]).unwrap();
    let mut planner=Planner::new(&Vram::new(),&[]).unwrap();planner.reserve_observations(&observed).unwrap();
    let source=Rect{x:880,y:64,w:16,h:64};let bytes=vec![0;2048];
    assert!(planner.allocate(source,&bytes,0,Some([880,64]),&[]).is_err());
    let moved=planner.allocate(source,&bytes,0,None,&[]).unwrap();assert!(!observed.intersects(moved).unwrap());
    assert!(planner.reserve_observations(&observed).is_err());
}

fn block(out:&mut Vec<u8>,x:u16,y:u16,w:u16,h:u16,data:&[u8]) {
    out.extend_from_slice(&(12+data.len() as u32).to_le_bytes());
    for n in [x,y,w,h] {out.extend_from_slice(&n.to_le_bytes());}out.extend_from_slice(data);
}
pub(in crate::app::native_asset_plan) fn tim(depth:u8,colour:u16)->Vec<u8> {
    let mut out=Vec::new();out.extend_from_slice(&16u32.to_le_bytes());out.extend_from_slice(&(depth as u32|if depth<2{8}else{0}).to_le_bytes());
    if depth<2 {let w=if depth==0{16}else{256};let bytes=(0..w).flat_map(|i|if i==0{0u16}else{colour|if i%2==0{0x8000}else{0}}.to_le_bytes()).collect::<Vec<_>>();block(&mut out,0,480,w,1,&bytes);}
    let word=match depth {0=>0x3210u16,1=>0x0100,_=>colour|0x8000};
    let data=(0..64).flat_map(|_|word.to_le_bytes()).collect::<Vec<_>>();block(&mut out,128,0,8,8,&data);out
}
pub(in crate::app::native_asset_plan) fn model(cba:u16,tsb:u16,uvs:Vec<(u8,u8)>)->Vec<u8> {
    legaia_tmd::encode::encode(&[ModelObject{vertices:vec![[0,0,0],[1,0,0],[0,1,0]],scale:LEGAIA_OBJECT_SCALE,
        groups:vec![ModelGroup{shape:PacketShape::Gt3,semi_transparent:true,prims:vec![ModelPrim{vertices:vec![0,1,2],uvs,cba,tsb,colors:vec![[128,100,60],[40,70,90],[255,0,1]]}]},
            ModelGroup{shape:PacketShape::F3,semi_transparent:false,prims:vec![ModelPrim{vertices:vec![0,1,2],uvs:vec![],cba:0,tsb:0,colors:vec![[31,55,80]]}]}]}]).unwrap()
}
pub(in crate::app::native_asset_plan) fn register(plan:&mut NativeAssetPlan,kind:ResourceKind,raw:&[u8])->ResourceId {
    plan.register(kind,raw,Origin{scene:"synthetic-donor".into(),entry:Some(7),offset:Some(32)},false).unwrap()
}
fn fixture(depth:u8)->(NativeAssetPlan,ResourceId,ResourceId) {
    let mut inventory=NativeAssetPlan::default();let texture=register(&mut inventory,ResourceKind::Tim,&tim(depth,0x1f));
    let tmd=register(&mut inventory,ResourceKind::Tmd,&model(0x8000|(480<<6),0x8000|0x60|2|((depth as u16)<<7),vec![(0,0),(7,0),(0,7)]));
    (inventory,texture,tmd)
}
#[test]
fn indexed_and_direct_colour_roundtrip_only_native_reference_fields() {
    for depth in 0..3 {
        let (inventory,texture,tmd)=fixture(depth);let mut planner=Planner::new(&Vram::new(),&[]).unwrap();
        let placement=Placement{image:[384,256],clut:(depth<2).then_some([512,470])};
        let plan=planner.plan(&inventory,&texture,&[tmd.clone()],Some(placement)).unwrap();
        assert_eq!(plan.placement,placement);assert_eq!(plan.donor_tim,texture);assert!(!plan.origins.is_empty());
        let before=inventory.resources[&tmd].payload.as_ref().unwrap();let a=references(before).unwrap();let b=references(&plan.tmds[&tmd]).unwrap();
        assert_eq!(a[0].uvs,b[0].uvs);assert_eq!(b[0].tsb&31,6|16);assert_eq!(a[0].tsb&!31,b[0].tsb&!31);
        assert_eq!(b[0].cba,if depth<2 {0x8000|(470<<6)|32}else{a[0].cba});
        let allowed=BTreeSet::from([a[0].at+2,a[0].at+3,a[0].at+6,a[0].at+7]);
        for (i,(a,b)) in before.iter().zip(&plan.tmds[&tmd]).enumerate(){assert!(a==b||allowed.contains(&i));}
        let raw=inventory.resources[&texture].payload.as_ref().unwrap();let original=legaia_tim::parse_strict(raw).unwrap();let result=legaia_tim::parse_strict(&plan.tim).unwrap();
        assert_eq!(original.image.data,result.image.data);assert_eq!(original.flags,result.flags);
        assert_eq!(original.clut.as_ref().map(|c|&c.entries),result.clut.as_ref().map(|c|&c.entries));
        let header=if let Some(c)=&original.clut{8+12+c.entries.len()*2}else{8};
        let mut tim_fields=BTreeSet::from([header+4,header+5,header+6,header+7]);if depth<2{tim_fields.extend([12,13,14,15]);}
        assert!(raw.iter().zip(&plan.tim).enumerate().all(|(i,(a,b))|a==b||tim_fields.contains(&i)));
        assert!(plan.apply(&Vram::new()).is_ok());let mut stale=Vram::new();stale.write_block(900,400,1,1,&[0,0]);assert!(plan.apply(&stale).is_err());
    }
}
#[test]
fn destination_conflicts_palette_failures_and_reservations_are_transactional() {
    let (inventory,texture,tmd)=fixture(0);let mut vram=Vram::new();vram.write_block(384,256,1,1,&[0,0]);
    let mut planner=Planner::new(&vram,&[Rect{x:512,y:470,w:16,h:1}]).unwrap();let original=super::super::super::naming::material_key(&planner.vram);
    let blocked=Placement{image:[384,256],clut:Some([512,470])};assert!(planner.plan(&inventory,&texture,&[tmd.clone()],Some(blocked)).is_err());
    let palette_failure=Placement{image:[448,256],..blocked};assert!(planner.plan(&inventory,&texture,&[tmd.clone()],Some(palette_failure)).is_err());
    assert!(planner.blocks.is_empty());assert_eq!(super::super::super::naming::material_key(&planner.vram),original);
    let plan=planner.plan(&inventory,&texture,&[tmd],None).unwrap();assert_ne!(plan.placement,blocked);
    let mut exhausted=Planner::new(&Vram::new(),&[Rect{x:0,y:0,w:1024,h:512}]).unwrap();
    let tmd=plan.tmds.keys().next().unwrap().clone();assert!(exhausted.plan(&inventory,&texture,&[tmd],None).is_err());assert!(exhausted.blocks.is_empty());
}
#[test]
fn reuse_shares_pixels_but_different_palettes_keep_distinct_materials() {
    let (mut inventory,first,tmd)=fixture(0);let second=register(&mut inventory,ResourceKind::Tim,&tim(0,0x3e0));
    let mut planner=Planner::new(&Vram::new(),&[]).unwrap();
    let a=planner.plan(&inventory,&first,&[tmd.clone()],None).unwrap();let blocks=planner.blocks.len();
    let repeated=planner.plan(&inventory,&first,&[tmd.clone()],None).unwrap();assert_eq!(a.tim,repeated.tim);assert_eq!(a.tmds,repeated.tmds);assert_eq!(blocks,planner.blocks.len());
    let b=planner.plan(&inventory,&second,&[tmd.clone()],None).unwrap();assert_eq!(a.placement.image,b.placement.image);assert_ne!(a.placement.clut,b.placement.clut);assert_ne!(a.tmds[&tmd],b.tmds[&tmd]);
    assert_eq!(planner.blocks.len(),blocks+1);
    let mut independent=Planner::new(&Vram::new(),&[]).unwrap();let same=independent.plan(&inventory,&first,&[tmd],None).unwrap();assert_eq!(a.tim,same.tim);assert_eq!(a.tmds,same.tmds);
}
#[test]
fn malformed_unsupported_and_incomplete_packages_fail_closed() {
    let (mut inventory,texture,tmd)=fixture(0);
    let valid=inventory.resources[&texture].payload.as_ref().unwrap().to_vec();
    let valid_tmd=inventory.resources[&tmd].payload.as_ref().unwrap().to_vec();
    for raw in [vec![],valid[..valid.len()-1].to_vec(),{let mut b=valid.clone();b.push(0);b},{let mut b=tim(2,1);b[4]=3;b}] {
        let id=if raw.is_empty(){ResourceId{kind:ResourceKind::Tim,sha256:"absent".into()}}else{register(&mut inventory,ResourceKind::Tim,&raw)};
        assert!(Planner::new(&Vram::new(),&[]).unwrap().plan(&inventory,&id,&[tmd.clone()],None).is_err());
    }
    let refs=references(&valid_tmd).unwrap();
    for raw in [{let mut b=valid_tmd.clone();b[refs[0].at+6]|=0x80;b[refs[0].at+7]|=1;b}, // reserved GPU depth 3
        model(256/16|(480<<6),2,vec![(0,0),(7,0),(0,7)]), // external CLUT
        model(480<<6,2,vec![(0,0),(255,0),(0,7)]), // image outside TIM
        {let mut b=valid_tmd.clone();let t=legaia_tmd::parse(&b).unwrap();let g=legaia_tmd::legaia_prims::iter_groups(&b,t.objects[0].primitives_byte_offset,t.objects[0].primitives_byte_size).unwrap();b[g[0].header_offset+2..g[0].header_offset+4].copy_from_slice(&0xffffu16.to_le_bytes());b},
        {let mut b=valid_tmd.clone();b[32..36].copy_from_slice(&99u32.to_le_bytes());b}, // claimed primitive count
        {let mut b=valid_tmd.clone();b[12..16].copy_from_slice(&((refs[0].at-12) as u32).to_le_bytes());b}] { // geometry alias
        let id=register(&mut inventory,ResourceKind::Tmd,&raw);assert!(Planner::new(&Vram::new(),&[]).unwrap().plan(&inventory,&texture,&[id],None).is_err());
    }
    let mut stale=inventory.clone();stale.resources.get_mut(&texture).unwrap().payload=Some(std::sync::Arc::from([1u8,2,3]));
    assert!(Planner::new(&Vram::new(),&[]).unwrap().plan(&stale,&texture,&[tmd],None).is_err());
}

#[test]
fn equal_byte_counts_with_different_rectangles_do_not_alias() {
    let (mut inventory,first,_)=fixture(0);
    let tmd=register(&mut inventory,ResourceKind::Tmd,&model(480<<6,2,vec![(0,0),(3,0),(0,3)]));
    let mut raw=inventory.resources[&first].payload.as_ref().unwrap().to_vec();
    let header=8+12+16*2;raw[header+8..header+10].copy_from_slice(&16u16.to_le_bytes());raw[header+10..header+12].copy_from_slice(&4u16.to_le_bytes());
    let second=register(&mut inventory,ResourceKind::Tim,&raw);let mut planner=Planner::new(&Vram::new(),&[]).unwrap();
    let a=planner.plan(&inventory,&first,&[tmd.clone()],None).unwrap();let b=planner.plan(&inventory,&second,&[tmd],None).unwrap();
    assert_ne!(a.placement.image,b.placement.image);assert_eq!(a.placement.clut,b.placement.clut);
    let parsed=legaia_tim::parse_strict(&b.tim).unwrap();
    for y in 0..4usize {for x in 0..16usize {let i=(y*16+x)*2;assert_eq!(b.output.pixel(parsed.image.fb_x as usize+x,parsed.image.fb_y as usize+y),u16::from_le_bytes([parsed.image.data[i],parsed.image.data[i+1]]));}}
}

#[test]
fn all_baked_textured_triangle_and_quad_shapes_rewrite_without_attribute_changes() {
    let (mut inventory,texture,_)=fixture(0);
    let groups=[PacketShape::Ft3,PacketShape::Ft4,PacketShape::Gt3,PacketShape::Gt4].into_iter().map(|shape| {
        let quad=matches!(shape,PacketShape::Ft4|PacketShape::Gt4);let gouraud=matches!(shape,PacketShape::Gt3|PacketShape::Gt4);let n=if quad{4}else{3};
        ModelGroup{shape,semi_transparent:true,prims:vec![ModelPrim{vertices:(0..n).collect(),uvs:vec![(0,0),(7,0),(0,7),(7,7)][..n as usize].to_vec(),cba:480<<6,tsb:0x60|2,colors:vec![[110,95,85];if gouraud{n as usize}else{1}]}]}
    }).collect();
    let raw=legaia_tmd::encode::encode(&[ModelObject{vertices:vec![[0,0,0],[1,0,0],[0,1,0],[1,1,0]],groups,scale:LEGAIA_OBJECT_SCALE}]).unwrap();
    let id=register(&mut inventory,ResourceKind::Tmd,&raw);let plan=Planner::new(&Vram::new(),&[]).unwrap().plan(&inventory,&texture,&[id.clone()],None).unwrap();
    assert_eq!(references(&plan.tmds[&id]).unwrap().len(),4);
}

#[test]
#[ignore = "explicit read-only native TIM payload and primitive-decoder fixture"]
fn native_tim_payload_relocation_uses_real_pixels_without_export() -> Result<()> {
    let source=std::path::PathBuf::from(std::env::var_os("LCK_P0_SOURCE").context("LCK_P0_SOURCE required")?);
    let lib=crate::library::Library::open(&source)?;let scene=legaia_engine_scene::scene::Scene::load(&lib.index,"bylon")?;
    let mut chosen=None;
    for location in legaia_engine_scene::scene_bundle::scene_tim_layout(&scene)? {
        let entry=scene.entries.iter().find(|e|e.idx==location.entry_idx).context("TIM entry absent")?;
        let raw=&entry.bytes[location.offset..location.offset+location.byte_len];let tim=legaia_tim::parse_strict(raw)?;
        if let Ok((image,Some(clut),depth))=rectangles(&tim) {
            if image.x%64==0 && image.y%256==0 && image.w<=256 && image.h<=256 && image.w>=2 && image.h>=2 && clut.w<=256 && clut.h<=16 && clut.w>=if depth==0{16}else{256} {chosen=Some((raw.to_vec(),tim,location));break;}
        }
    }
    let (raw,tim,location)=chosen.context("No native TIM inside bounded package contract")?;
    let depth=if tim.mode==PixelMode::Bpp4{0}else{1};let clut=tim.clut.as_ref().unwrap();
    let mesh=model((clut.fb_x/16)|(clut.fb_y<<6),(tim.image.fb_x/64)|((tim.image.fb_y/256)<<4)|(depth<<7),vec![(0,0),(1,0),(0,1)]);
    let mut inventory=NativeAssetPlan::default();let id=inventory.register(ResourceKind::Tim,&raw,Origin{scene:scene.name.clone(),entry:Some(location.entry_idx),offset:Some(location.offset)},false)?;
    let tmd=register(&mut inventory,ResourceKind::Tmd,&mesh);let mut planner=Planner::new(&Vram::new(),&[])?;
    let plan=planner.plan(&inventory,&id,&[tmd],None)?;
    ensure!(legaia_tim::parse_strict(&plan.tim)?.image.data==tim.image.data,"Native pixels changed");
    // Also exercise complete real TMD walks without ever truncating unknown
    // groups into a fabricated portable mesh. Rejections remain explicit.
    let assembled=legaia_engine_world_views::scene_staging::assemble_field_scene(&lib.index,"bylon").map_err(anyhow::Error::msg)?;
    let tested=assembled.res.tmds.iter().take(16).map(|t|references(&t.raw)).collect::<Vec<_>>();
    ensure!(tested.iter().any(Result::is_ok),"No real native TMD descriptor walk succeeded");
    println!("Native TIM PROT {} +{:#x}: {:?}, {}x{} words; relocation {:?}; real TMD walks: {} accepted / {} explicitly rejected; no IMG",location.entry_idx,location.offset,tim.mode,tim.image.fb_w,tim.image.h,plan.placement,tested.iter().filter(|r|r.is_ok()).count(),tested.iter().filter(|r|r.is_err()).count());
    Ok(())
}
