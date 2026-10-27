//! Opt-in real-disc acceptance test. Never runs in the normal unit suite.
//! Source is read-only; the caller supplies a new output directory explicitly.
use super::*;
use std::io::Write;

#[test]
#[ignore = "explicit read-only multi-town native asset dependency inventory"]
fn multiple_towns_and_categories_share_one_native_dependency_inventory() -> Result<()> {
    use super::super::native_asset_plan::{self,Category,InstanceId,ResourceKind};
    let source=PathBuf::from(std::env::var_os("LCK_P0_SOURCE").context("LCK_P0_SOURCE required")?);
    let lib=crate::library::Library::open(&source)?;
    let target=legaia_engine_world_views::scene_staging::assemble_field_scene(&lib.index,"town01").map_err(anyhow::Error::msg)?;
    let mut world=world_view::build_world(&target,&lib.index);
    let local=world.draws.iter().find(|d|!d.terrain && world.npc((false,d.index)).is_none()).map(|d|(false,d.index)).context("No local environmental draw")?;
    world.duplicate_native(local,target.placements.len(),940001,[0.0;3]).map_err(anyhow::Error::msg)?;
    let npc=world.npcs.iter().find(|n|n.drawable && world.at(false,n.key).is_some()).map(|n|n.key).context("No local actor draw")?;
    ensure!(world.set_offset(false,npc,[64.0,0.0,0.0]),"Cannot request actor movement");
    let mut uid=940002;
    for name in ["izumi","bylon"] {
        let donor=legaia_engine_world_views::scene_staging::assemble_field_scene(&lib.index,name).map_err(anyhow::Error::msg)?;
        for kind in ["placement","terrain","mesh"] {
            let count=match kind {"placement"=>donor.placements.len(),"terrain"=>donor.terrain.len(),_=>donor.env_tmds.len()};
            ensure!(count>0,"Representative {name} lacks {kind} category");
            world.insert_foreign(&donor,kind,0,uid,[4096.0,0.0,4096.0],&lib.index).map_err(anyhow::Error::msg)?;
            uid+=1;
        }
    }
    let saved=scene_instances::serialize(&target.name,&world);
    let first=native_asset_plan::collect(&target,&world,&lib.index)?;
    let second=native_asset_plan::collect(&target,&world,&lib.index)?;
    ensure!(first==second,"Native dependency inventory is nondeterministic");
    let categories=first.requests.values().map(|r|r.category).collect::<std::collections::BTreeSet<_>>();
    ensure!(categories.len()==4 && categories.contains(&Category::Actor),"Category coverage incomplete");
    for id in 940001..uid {ensure!(first.requests.contains_key(&InstanceId::Created(id)),"Requested instance omitted");}
    ensure!(first.resources.keys().any(|r|r.kind==ResourceKind::Tim),"Native TIM payload provenance absent");
    for foreign in &world.foreign {
        let request=&first.requests[&InstanceId::Created(foreign.uid)];
        ensure!(request.resources.iter().any(|id|id.kind==ResourceKind::Tmd && first.resources[id].payload.as_deref()==Some(foreign.raw_tmd.as_slice())),"Donor bytes lost");
    }
    ensure!(first.validate_routes().is_err(),"Unsupported terrain/raw mesh routes accepted");
    let mut unsupported=world.clone();unsupported.collision.recipes.clear();unsupported.collision.mesh_recipes.clear();
    let error=plan_export(&source,&target,&unsupported,&lib.index).err().context("Unsupported categories generated a writable plan")?;
    let text=format!("{error:#}");ensure!(text.contains("TerrainTile") && text.contains("RawMesh") && text.contains("No requested instance was omitted"),"Incomplete category rejection: {text}");
    let mut stale=world.clone();stale.foreign[0].raw_tmd[0]^=1;
    ensure!(native_asset_plan::collect(&target,&stale,&lib.index).is_err(),"Stale native donor bytes accepted");
    ensure!(scene_instances::serialize(&target.name,&world)==saved,"Inventory changed document");
    println!("{} requests across town01, izumi, bylon; {} immutable resources; categories {categories:?}; unsupported categories blocked, donor bytes retained, stale provenance rejected",first.requests.len(),first.resources.len());
    Ok(())
}

#[test]
#[ignore = "explicit read-only retail allocation and additive collision contract validation"]
fn rim_elm_static_copy_additive_contract_stays_blocked() -> Result<()> {
    let source=PathBuf::from(std::env::var_os("LCK_P0_SOURCE").context("LCK_P0_SOURCE required")?);
    let lib=crate::library::Library::open(&source)?;
    let scene=legaia_engine_world_views::scene_staging::assemble_field_scene(&lib.index,"town01").map_err(anyhow::Error::msg)?;
    let mut world=world_view::build_world(&scene,&lib.index);
    let key=world.collision.sources.iter().find(|(_,s)|s.object.descriptor_id==230 && s.object.cell==[94,78])
        .map(|(k,_)|*k).context("Candidate absent")?;
    let provenance=world.collision.sources[&key].clone();
    let map=lib.index.entry_bytes(provenance.object.map_entry)?.as_ref().clone();
    let lba=lib.index.entry_start_lba_retail(provenance.object.map_entry.try_into()?).context("MAP LBA absent")?;
    let load=lib.index.prot_dat_raw_bytes(lba as u64*2048,0x14000)?;
    ensure!(load[..MAP_LEN]==map,"Physical and indexed MAP differ");
    let bindings=NativeBindings::parse(&load[0x10000..0x12000],&load[0x12000..])?;
    let p=&scene.placements[key.1];
    ensure!(p.env_slot==15 && (p.world_x,p.world_y,p.world_z)==(12096,0,10048),"Candidate native geometry/position changed");
    let report=super::super::native_dependency_report::report(&scene,&world,key,&lib.index).map_err(anyhow::Error::msg)?;
    ensure!(report.contains("No kind-1 binding"),"Candidate unexpectedly bound");
    let before=scene_instances::serialize(&scene.name,&world);
    let copied=world.duplicate_native(key,scene.placements.len(),930001,[64.0,0.0,0.0]).map_err(anyhow::Error::msg)?;
    let saved=scene_instances::serialize(&scene.name,&world);
    let draw=world.at(copied.0,copied.1).context("Copy absent")?;
    ensure!(draw.rotation==[0.0;3] && draw.scale==[1.0;3],"Unexpected transform");
    // These are isolated in-memory stages, not an alternate export path.
    // Keep the automatic recipe's MeshDerived provenance; do not fabricate review.
    let collision=world.compile_authored_collision(&scene.name,provenance.object.map_entry,&map).map_err(anyhow::Error::msg)?;
    let added_bits: u32=map.iter().zip(&collision).map(|(&a,&b)|(b&!a).count_ones()).sum();
    let changed_bytes=map.iter().zip(&collision).filter(|(a,b)|a!=b).count();
    let mut allocated=map.clone();
    let edits=[InsertEdit{uid:930001,cell:p.cell,source_position:(p.world_x,p.world_z),env_slot:p.env_slot,
        source_rotation:[p.rot_x,p.rot_y,p.rot_z],delta:[64.0,0.0,0.0],collision_bounds:original_collision_bounds(&world,key.1)}];
    let inserted=insert_copies_with_bindings(&mut allocated,&map,&edits,&[None;16],&bindings)?;
    ensure!(inserted.len()==1 && inserted[0].id!=230,"Copy did not allocate independent identity");
    let copy=&inserted[0];
    let parsed=field_objects::parse_placements(&allocated);
    for original in field_objects::parse_placements(&map) {ensure!(parsed.contains(&original),"Original placement changed");}
    let placed=parsed.iter().find(|p|p.obj_idx as usize==copy.id).context("Allocated copy failed readback")?;
    ensure!((placed.world_x,placed.world_z)==(12160,10048),"Copy position mismatch");
    ensure!(placed.pack_index==Some(15) && [placed.rot_x,placed.rot_y,placed.rot_z]==[p.rot_x,p.rot_y,p.rot_z],"Native model/orientation changed");
    ensure!((placed.world_x-(copy.col as i32*128+64)).abs()<=LCK_SAFE_ANCHOR_REACH &&
        (placed.world_z-(copy.row as i32*128+64)).abs()<=LCK_SAFE_ANCHOR_REACH,"Distant spawn anchor accepted");
    ensure!(!bindings.has_trigger((copy.col,copy.row)) && bindings.at((placed.anchor_col,placed.anchor_row)).is_none(),"Copy collides with native binding/trigger");
    let slot=copy.id*32;let grid=GRID_OFF+(copy.row as usize*128+copy.col as usize)*2;
    for i in 0..MAP_LEN {
        if !(slot..slot+32).contains(&i) && !(grid..grid+2).contains(&i) {ensure!(allocated[i]==map[i],"Allocation changed unrelated MAP byte {i:#x}");}
    }
    // Compose the isolated stages only in this fixture, then read the native
    // grid independently. This checks bytes, not playability or actor behavior.
    allocated[WALL_OFF..GRID_OFF].copy_from_slice(&collision[WALL_OFF..GRID_OFF]);
    let original=legaia_engine_field::minigame_floor::FloorGrid::new(&map);
    let output=legaia_engine_field::minigame_floor::FloorGrid::new(&allocated);
    for z in 0..128 {for x in 0..128 {
        ensure!(original.height_index(x,z)==output.height_index(x,z),"Floor tier changed");
        ensure!(output.wall_nibble(x,z)&original.wall_nibble(x,z)==original.wall_nibble(x,z),"Native wall removed");
    }}
    ensure!(allocated[0x10000..]==map[0x10000..],"Native scene bindings changed");
    ensure!(allocated[230*32..231*32]==map[230*32..231*32],"Original descriptor changed");
    let generated=super::super::collision_recipes::generated_for(&world,copied).context("Generated recipe absent")?;
    println!("Candidate: original (12096,0,10048), copy (12160,0,10048), mesh 15; descriptor {}, spawn cell ({},{}), collider {:?}; {} changed wall bytes / {} added bits; {:?} floor/steep/filtered faces, heuristic band {}. No runtime contract or IMG.",copy.id,copy.col,copy.row,copy.collider,changed_bytes,added_bits,generated.surface_counts(),generated.band_height);
    let error=plan_export(&source,&scene,&world,&lib.index).err().context("Unreviewed collision unexpectedly authorized")?;
    let diagnostic=format!("{error:#}");
    ensure!(diagnostic.contains("Copy collision intent: additive") && diagnostic.contains("Floor candidates are not compiled as walkable surfaces") && diagnostic.contains("native IMG export is not enabled"),"Missing precise contract rejection: {diagnostic}");
    ensure!(scene_instances::serialize(&scene.name,&world)==saved,"Read-only validation changed document");
    world.clear_created();ensure!(scene_instances::serialize(&scene.name,&world)==before,"Original state changed");
    Ok(())
}

#[test]
#[ignore = "explicit read-only retail naming identity and persistence validation"]
fn rim_elm_names_use_native_identity_and_verified_visual_aliases() -> Result<()> {
    let source=PathBuf::from(std::env::var_os("LCK_P0_SOURCE").context("LCK_P0_SOURCE required")?);
    let lib=crate::library::Library::open(&source)?;
    let scene=legaia_engine_world_views::scene_staging::assemble_field_scene(&lib.index,"town01").map_err(anyhow::Error::msg)?;
    let mut world=world_view::build_world(&scene,&lib.index);
    let unique=world.naming.native.values().collect::<std::collections::BTreeSet<_>>();
    ensure!(unique.len()==world.naming.native.len(),"Native naming identities are ambiguous");
    let missing=world.draws.iter().filter(|d|d.editor_uid.is_none()&&!world.naming.native.contains_key(&(d.terrain,d.index)))
        .map(|d|(d.terrain,d.index)).collect::<Vec<_>>();
    ensure!(missing.is_empty(),"Original Rim Elm instances lack stable naming identities: {missing:?}");
    let key=world.collision.sources.iter().find(|(_,s)|s.object.descriptor_id==230&&s.object.cell==[94,78]).map(|(k,_)|*k).context("Candidate absent")?;
    let canonical=world.base_name_key(key).context("Verified visual alias unavailable")?.to_owned();
    ensure!(canonical.starts_with("visual-v1:"),"Naming identity lacks visual provenance");
    ensure!(world.naming.native.contains_key(&key),"Stable native instance identity absent");
    world.set_display_name(key,Some("Rim Elm test lantern".into()));
    let copied=world.duplicate_native(key,scene.placements.len(),920001,[64.0,0.0,0.0]).map_err(anyhow::Error::msg)?;
    ensure!(world.at(copied.0,copied.1).and_then(|d|d.display_name.as_deref())==Some("Rim Elm test lantern"),"Duplicate did not retain its independent initial name");
    world.set_display_name(copied,Some("Independent test lantern".into()));
    let saved=scene_instances::serialize(&scene.name,&world);
    ensure!(saved.contains("NATIVE_NAME\tmap-instance-v1:town01:"),"Original name uses a transient list index");
    let doc=scene_instances::parse(&saved,&scene.name,&world).map_err(anyhow::Error::msg)?;
    scene_instances::apply(&doc,&mut world,scene.placements.len(),&lib.index).map_err(anyhow::Error::msg)?;
    ensure!(scene_instances::serialize(&scene.name,&world)==saved,"Native names failed to restore");
    ensure!(world.base_name_key(copied)==Some(canonical.as_str()),"Copy's verified base alias changed");
    let center=world.at(key.0,key.1).context("Source draw absent")?.original_center;
    let foreign=world.insert_foreign(&scene,"placement",key.1,920002,center,&lib.index).map_err(anyhow::Error::msg)?;
    ensure!(world.base_name_key(foreign)==Some(canonical.as_str()),"Imported donor did not retain the verified shared naming identity");
    world.set_display_name(foreign,Some("Imported test lantern".into()));
    let saved=scene_instances::serialize(&scene.name,&world);let doc=scene_instances::parse(&saved,&scene.name,&world).map_err(anyhow::Error::msg)?;
    scene_instances::apply(&doc,&mut world,scene.placements.len(),&lib.index).map_err(anyhow::Error::msg)?;
    ensure!(scene_instances::serialize(&scene.name,&world)==saved,"Imported instance name failed to restore");
    println!("Verified {} unique native naming identities; original, copy and imported naming roundtrip passed",world.naming.native.len());
    Ok(())
}

#[test]
#[ignore = "explicit read-only retail Ctrl+D and Properties integration"]
fn rim_elm_ctrl_d_opens_generated_collision_properties() -> Result<()> {
    use super::super::{App,Automation,NativeSceneSlot};
    use eframe::egui;
    let source=PathBuf::from(std::env::var_os("LCK_P0_SOURCE").context("LCK_P0_SOURCE required")?);
    let lib=std::sync::Arc::new(crate::library::Library::open(&source)?);
    let scene=legaia_engine_world_views::scene_staging::assemble_field_scene(&lib.index,"town01").map_err(anyhow::Error::msg)?;
    let world=world_view::build_world(&scene,&lib.index);
    let key=world.collision.sources.iter().find(|(_,s)|s.object.descriptor_id==230 && s.object.cell==[94,78])
        .map(|(key,_)|*key).context("Candidate absent")?;
    let mut app=App::new(crate::Studio::new(None),None,Automation::default());
    app.studio.library=Some(lib);app.native_selected=Some(key);
    app.native_scene=Some(("town01".into(),NativeSceneSlot::Ready(std::sync::Arc::new(scene),std::sync::Arc::new(world))));
    let ctx=egui::Context::default();
    let frame=|app:&mut App,events:Vec<egui::Event>,modifiers|ctx.run(egui::RawInput {
        screen_rect:Some(egui::Rect::from_min_size(egui::Pos2::ZERO,egui::vec2(1280.0,820.0))),events,modifiers,..Default::default()
    },|ctx|app.editor_workspace(ctx));
    let _=frame(&mut app,vec![],Default::default());let camera=app.native_view.cam;
    // Native egui integration sets both ctrl and the platform command alias.
    let command=egui::Modifiers{ctrl:true,command:true,..Default::default()};
    let _=frame(&mut app,vec![egui::Event::Key{key:egui::Key::D,physical_key:None,pressed:true,repeat:false,modifiers:command}],command);
    let copied=app.native_selected.context("Ctrl+D did not select a copy")?;
    ensure!(copied!=key,"Ctrl+D failed to create an independent draw; keyboard input {}, error {:?}",ctx.wants_keyboard_input(),app.studio.error);
    let Some((_,NativeSceneSlot::Ready(_,world)))=&app.native_scene else {bail!("Scene absent")};
    ensure!(super::super::collision_recipes::generated_for(world,copied).is_some(),"Ctrl+D copy has no generated recipe");
    let before=scene_instances::serialize("town01",world);
    app.editor_properties_target=Some(copied);
    let _=frame(&mut app,vec![],Default::default());let output=frame(&mut app,vec![],Default::default());
    fn contains(shape:&egui::Shape,needle:&str)->bool {match shape {
        egui::Shape::Text(t)=>t.galley.job.text.contains(needle),egui::Shape::Vec(shapes)=>shapes.iter().any(|s|contains(s,needle)),_=>false
    }}
    ensure!(output.shapes.iter().any(|s|contains(&s.shape,"Collision Properties (P1)")),"Actual Properties window omitted collision controls");
    ensure!(output.shapes.iter().any(|s|contains(&s.shape,"Saved generated collision:")),"Properties failed to identify the saved recipe");
    ensure!(app.native_view.cam==camera,"Properties/Ctrl+D changed camera");
    let Some((_,NativeSceneSlot::Ready(_,world)))=&app.native_scene else {bail!("Scene absent")};
    ensure!(scene_instances::serialize("town01",world)==before,"Properties mutated document");
    Ok(())
}

#[test]
#[ignore = "explicit read-only unbound candidate dependency validation"]
fn unbound_candidate_reports_scene_wall_dependencies() -> Result<()> {
    let source=PathBuf::from(std::env::var_os("LCK_P0_SOURCE").context("LCK_P0_SOURCE required")?);
    let lib=crate::library::Library::open(&source)?;
    let scene=legaia_engine_world_views::scene_staging::assemble_field_scene(&lib.index,"town01").map_err(anyhow::Error::msg)?;
    let world=world_view::build_world(&scene,&lib.index);
    let before=scene_instances::serialize(&scene.name,&world);
    // Previously established compact unbound placement. Other uses of record
    // 230 are bound, so identify the source by cell as well as descriptor.
    let key=world.collision.sources.iter().find(|(_,s)|s.object.descriptor_id==230 && s.object.cell==[94,78])
        .map(|(key,_)|*key).context("Candidate native identity absent")?;
    let report=super::super::native_dependency_report::report(&scene,&world,key,&lib.index).map_err(anyhow::Error::msg)?;
    ensure!(report.contains("No kind-1 binding"),"Candidate unexpectedly bound");
    ensure!(report.contains("Incomplete record scans:"),"Unbound report skipped scene scripts");
    ensure!(report.contains("Wall ownership: UNKNOWN"),"Unproven collision ownership");
    ensure!(before==scene_instances::serialize(&scene.name,&world),"Read-only report mutated document");
    println!("{report}");
    let mut copy=world.clone();
    copy.duplicate_native(key,scene.placements.len(),910001,[64.0,0.0,0.0]).map_err(anyhow::Error::msg)?;
    let recipe=copy.collision.recipes.iter().find(|a|a.owner==super::super::collision_recipes::ObjectId::Created(910001))
        .context("Automatic collision recipe missing")?;
    ensure!(recipe.recipe.generated.is_some(),"Copy lacks mesh geometry definition");
    let generated=recipe.recipe.generated.as_ref().unwrap();
    println!("Automatic #230 collision: {} faces, {:?} floor/steep/filtered; {} transformed wall sub-cells",
        generated.triangles.len(),generated.surface_counts(),generated.wall_cells([64.0,0.0,0.0],[0.0;3],[1.0;3]).map_err(anyhow::Error::msg)?.len());
    let saved=scene_instances::serialize(&scene.name,&copy);
    let parsed=scene_instances::parse(&saved,&scene.name,&copy).map_err(anyhow::Error::msg)?;
    scene_instances::apply(&parsed,&mut copy,scene.placements.len(),&lib.index).map_err(anyhow::Error::msg)?;
    ensure!(scene_instances::serialize(&scene.name,&copy)==saved,"Generated collision did not roundtrip");
    let selected=copy.draws.iter().find(|d|d.editor_uid==Some(910001)).map(|d|(d.terrain,d.index)).context("Copy draw absent")?;
    let ctx=eframe::egui::Context::default();
    let _=ctx.run(Default::default(),|ctx|{
        eframe::egui::CentralPanel::default().show(ctx,|ui|{
            assert!(super::super::collision_recipes::show_editor(ui,&copy,selected,&lib.index).is_none());
        });
    });
    ensure!(scene_instances::serialize(&scene.name,&copy)==saved,"Opening visual review changed document");
    match plan_export(&source,&scene,&copy,&lib.index) {
        Ok(_)=>bail!("Unresolved candidate collision unexpectedly accepted"),
        Err(error)=>{
            ensure!(format!("{error:#}").contains("Unresolved collision ownership"),"Unexpected candidate rejection: {error:#}");
            ensure!(format!("{error:#}").contains("9 native wall bits"),"Missing exact native conflict count");
            println!("Candidate copy correctly blocked: {error:#}");
        }
    }
    // Exercise both a native-placement repository drop and a mesh-only donor
    // without writing a project or disc image.
    let mut imports=world.clone();
    let center=world.at(key.0,key.1).context("Candidate draw")?.original_center;
    imports.insert_foreign(&scene,"placement",key.1,910002,center,&lib.index).map_err(anyhow::Error::msg)?;
    let mesh=scene.placements[key.1].env_slot as usize;
    imports.insert_foreign(&scene,"mesh",mesh,910003,center,&lib.index).map_err(anyhow::Error::msg)?;
    ensure!(imports.collision.recipes.iter().any(|a|a.owner==super::super::collision_recipes::ObjectId::Created(910002)),"Donor placement collision missing");
    ensure!(imports.collision.mesh_recipes.contains_key(&910003),"Mesh-only donor collision missing");
    let saved=scene_instances::serialize(&scene.name,&imports);
    let parsed=scene_instances::parse(&saved,&scene.name,&imports).map_err(anyhow::Error::msg)?;
    scene_instances::apply(&parsed,&mut imports,scene.placements.len(),&lib.index).map_err(anyhow::Error::msg)?;
    ensure!(scene_instances::serialize(&scene.name,&imports)==saved,"Foreign collision did not roundtrip");
    let mut stale=parsed.clone();stale.mesh_collision[0].tmd_sha256="0".repeat(64);
    ensure!(scene_instances::apply(&stale,&mut imports,scene.placements.len(),&lib.index).is_err(),"Stale TMD collision accepted");
    ensure!(scene_instances::serialize(&scene.name,&imports)==saved,"Failed donor restore changed document");
    println!("Placement and mesh-only repository defaults and transactional restoration passed");
    Ok(())
}

#[test]
#[ignore = "explicit read-only retail selection report validation"]
fn selected_world_object_report_preserves_native_provenance() -> Result<()> {
    let source=PathBuf::from(std::env::var_os("LCK_P0_SOURCE").context("LCK_P0_SOURCE required")?);
    let lib=crate::library::Library::open(&source)?;
    let scene=legaia_engine_world_views::scene_staging::assemble_field_scene(&lib.index,"town01").map_err(anyhow::Error::msg)?;
    let world=world_view::build_world(&scene,&lib.index);
    let key=world.collision.sources.iter().find(|(_,source)|source.object.descriptor_id==137)
        .map(|(key,_)|*key).context("Native source identity absent")?;
    let before=scene_instances::serialize(&scene.name,&world);
    let report=super::super::native_dependency_report::report(&scene,&world,key,&lib.index).map_err(anyhow::Error::msg)?;
    ensure!(report.contains("flat MAN record 0"),"Binding report missing native context");
    ensure!(report.contains("Animation ID 1"),"Native ANM evidence absent");
    ensure!(report.contains("Wall ownership: UNKNOWN"),"Report claims unproven ownership");
    ensure!(scene_instances::serialize(&scene.name,&world)==before,"Report changed document");
    println!("Read-only selection report passed; {} bytes; source identities={}",report.len(),world.collision.sources.len());
    println!("{report}");
    Ok(())
}

#[test]
#[ignore = "explicit read-only retail binding investigation"]
fn inspect_rim_elm_binding_dependencies() -> Result<()> {
    let source=PathBuf::from(std::env::var_os("LCK_P0_SOURCE").context("LCK_P0_SOURCE required")?);
    let dir=PathBuf::from(std::env::var_os("LCK_P0_OUTPUT_DIR").context("LCK_P0_OUTPUT_DIR required")?);
    let mut log=std::fs::OpenOptions::new().write(true).create_new(true).open(dir.join("binding-dependencies.txt"))?;
    let disc=DiscPatcher::open(std::fs::read(source)?)?;
    for entry in [1,10,19] {
        let load=disc.read_disc_sectors(disc.entry_disc_lba(entry).context("MAP address")?,0x28)?;
        let bindings=NativeBindings::parse(&load[0x10000..0x12000],&load[0x12000..])?;
        let (man_entry,_,_,_,man)=find_native_man(&disc,entry)?;
        let mf=man_section::parse(&man)?;
        writeln!(log,"MAP={entry} MAN={man_entry} partitions={:?}",mf.header.partition_counts)?;
        for p in field_objects::parse_placements(&load) {
            let Some(bind)=bindings.at((p.anchor_col,p.anchor_row)) else {continue};
            writeln!(log,"record={} cell=({},{}) anchor=({},{}) binding={bind:?}",p.obj_idx,p.col,p.row,p.anchor_col,p.anchor_row)?;
            let flat=bind.record as usize;
            if flat>=mf.partitions[0].len() {writeln!(log,"  non-P0 flat record: unresolved header semantics")?;continue;}
            let start=mf.data_region_offset+mf.partitions[0][flat] as usize;
            let end=man_actor_record_end(&mf,man.len(),start);
            let bytes=man.get(start..end).context("MAN record range")?;
            let pc0=2+2*(*bytes.first().context("MAN header")? as usize);
            ensure!(pc0<bytes.len(),"Invalid P0 header");
            writeln!(log,"  animation={} MAN_range={start:#x}..{end:#x} pc0={pc0}",bytes[pc0-1])?;
            // Detailed case study uses the native record, never changes an
            // export policy. Stop decoding on first error: no byte resync can
            // establish dependency completeness.
            if p.obj_idx==137 {
                for instruction in legaia_scene_formats::field_disasm::LinearWalker::new(bytes,pc0) {
                    writeln!(log,"    {instruction:?}")?;
                    if instruction.is_err() {break;}
                }
            }
        }
    }
    Ok(())
}

#[test]
#[ignore = "requires explicit retail source and fresh controlled output directory"]
fn classify_rim_elm_native_constraints() -> Result<()> {
    let source=PathBuf::from(std::env::var_os("LCK_P0_SOURCE").context("LCK_P0_SOURCE required")?);
    let dir=PathBuf::from(std::env::var_os("LCK_P0_OUTPUT_DIR").context("LCK_P0_OUTPUT_DIR required")?);
    let mut log=std::fs::OpenOptions::new().write(true).create_new(true).open(dir.join("placement-constraints.txt"))?;
    let lib=crate::library::Library::open(&source)?;
    let scene=legaia_engine_world_views::scene_staging::assemble_field_scene(&lib.index,"town01").map_err(anyhow::Error::msg)?;
    let world=world_view::build_world(&scene,&lib.index);
    let disc=DiscPatcher::open(std::fs::read(&source)?)?;
    for entry in [1,10,19] {
        let map=disc.read_entry_footprint(entry)?;
        let mut refs=[0usize;512];
        for word in map[GRID_OFF..GRID_OFF+32768].chunks_exact(2) { refs[(u16::from_le_bytes([word[0],word[1]])&511) as usize]+=1; }
        let free=(4..512).filter(|&id|refs[id]==0 && map[id*32..id*32+32].iter().all(|b|*b==0)).count();
        writeln!(log,"MAP {entry}: blank unused records={free}")?;
        for (i,p) in scene.placements.iter().enumerate() {
            let (id,r)=record_at(&map,p.cell,(p.world_x,p.world_z),p.env_slot,[p.rot_x,p.rot_y,p.rot_z])?;
            let bounds=original_collision_bounds(&world,i);
            let walls=require_no_unowned_walls(&map,bounds).err().map(|e|e.to_string());
            writeln!(log,"placement={i} record={id} cell={:?} xyz=({},{},{}) mesh={} flags={:#x} bind={} refs={} bounds={bounds:?} wall_constraint={walls:?} offset=({},{}) collider=({},{})",p.cell,p.world_x,p.world_y,p.world_z,p.env_slot,r.flags,anchor_has_bind(&map,p.cell,&r),refs[id],r.x_off,r.z_off,source_collision(&r,p.cell.0,p.cell.1).0,source_collision(&r,p.cell.0,p.cell.1).1)?;
            let mov=MoveEdit{collision_bounds:bounds,cell:p.cell,source_position:(p.world_x,p.world_z),env_slot:p.env_slot,source_rotation:[p.rot_x,p.rot_y,p.rot_z],delta:[64.0,0.0,0.0]};
            let copy=InsertEdit{collision_bounds:bounds,uid:900000+i as u64,cell:p.cell,source_position:mov.source_position,env_slot:p.env_slot,source_rotation:mov.source_rotation,delta:mov.delta};
            // Diagnose allocation separately in memory. This never authorizes
            // export and never bypasses full collision validation on disk.
            let mut moved=map.clone();
            let allocation=move_originals(&mut moved,&[mov.clone()]);
            if allocation.is_ok() {
                let parsed=field_objects::parse_placements(&moved);
                ensure!(parsed.iter().any(|v|(v.col,v.row)==p.cell && v.world_x==p.world_x+64 && v.world_z==p.world_z),"Moved reference readback mismatch");
                ensure!(moved[WALL_OFF..GRID_OFF]==map[WALL_OFF..GRID_OFF],"Allocation changed collision");
                for other in field_objects::parse_placements(&map) {
                    if (other.col,other.row)==p.cell {continue;}
                    ensure!(parsed.contains(&other),"Allocation altered unrelated placement");
                }
            }
            let mut copied=map.clone();
            let copies=insert_copies(&mut copied,&map,&[copy],&[None;16]);
            if copies.is_ok() {
                ensure!(copied[id*32..id*32+32]==map[id*32..id*32+32],"Copy changed original descriptor");
                let parsed=field_objects::parse_placements(&copied);
                for original in field_objects::parse_placements(&map) {ensure!(parsed.contains(&original),"Copy altered original placement");}
            }
            writeln!(log,"  move_allocation={allocation:?}; copy_allocation={copies:?}; full_move={:?}",map_patches(&map,&ExportEdits{moves:vec![mov],..Default::default()}).map(|p|p.len()))?;
        }
    }
    Ok(())
}

#[test]
#[ignore = "requires explicit retail source and fresh controlled output directory"]
fn controlled_rim_elm_export() -> Result<()> {
    let source=PathBuf::from(std::env::var_os("LCK_P0_SOURCE").context("LCK_P0_SOURCE required")?);
    let dir=PathBuf::from(std::env::var_os("LCK_P0_OUTPUT_DIR").context("LCK_P0_OUTPUT_DIR required")?);
    let output=dir.join("RimElm_P0_translation.img");
    let rejected_output=dir.join("unsupported_edit_must_not_exist.img");
    ensure!(!output.exists() && !rejected_output.exists(),"Fresh output paths required");
    let mut log=std::fs::OpenOptions::new().write(true).create_new(true).open(dir.join("native-verification.txt"))?;
    let outcome=(||->Result<()> {
        let lib=crate::library::Library::open(&source)?;
        let scene=legaia_engine_world_views::scene_staging::assemble_field_scene(&lib.index,"town01")
            .map_err(anyhow::Error::msg)?;
        let mut world=world_view::build_world(&scene,&lib.index);
        let original=DiscPatcher::open(std::fs::read(&source)?)?;
        writeln!(log,"Scene={} placements={} PROT entries={}",scene.name,scene.placements.len(),original.entry_count())?;
        let mut selected=None;
        let mut failures=std::collections::BTreeMap::<String,usize>::new();
        for (i,p) in scene.placements.iter().enumerate() {
            if world.at(false,i).is_none() {continue;}
            let mov=MoveEdit {collision_bounds:original_collision_bounds(&world,i),cell:p.cell,
                source_position:(p.world_x,p.world_z),env_slot:p.env_slot,
                source_rotation:[p.rot_x,p.rot_y,p.rot_z],delta:[64.0,0.0,0.0]};
            let edits=ExportEdits{moves:vec![mov],..Default::default()};
            let supported=[1,10,19].into_iter().try_for_each(|entry| {
                map_patches(&original.read_entry_footprint(entry)?,&edits).map(|_|())
            });
            match supported {
                Ok(())=>{selected=Some(i);break;},
                Err(error)=>{*failures.entry(format!("{error:#}")).or_default()+=1;},
            }
        }
        writeln!(log,"Candidate rejection reasons: {failures:#?}")?;
        let i=selected.context("No currently supported +64 X translation across the principal Rim Elm maps")?;
        let p=&scene.placements[i];
        ensure!(world.set_offset(false,i,[64.0,0.0,0.0]),"Could not set independent object offset");
        writeln!(log,"Selected placement={i} cell={:?} env_slot={} resource={} native_xyz=({},{},{}) delta_view=(64,0,0)",p.cell,p.env_slot,p.res_tmd,p.world_x,p.world_y,p.world_z)?;
        std::fs::write(dir.join("supported_translation.lckscene"),scene_instances::serialize(&scene.name,&world))?;
        let plan=plan_export(&source,&scene,&world,&lib.index)?;
        writeln!(log,"Full preflight: {}",plan.summary())?;
        let predicted=plan.image.clone();
        let done=export(plan,&output)?;
        let written=std::fs::read(&done.path)?;
        ensure!(written==predicted,"Written image differs from the exact preflight plan");
        drop(predicted);
        ensure!(written.len()==original.image().len(),"Image size changed");
        let mut changed_sectors=Vec::new();
        for (lba,(before,after)) in original.image().chunks_exact(2352).zip(written.chunks_exact(2352)).enumerate() {
            if before==after {continue;}
            ensure!(legaia_iso::write::mode2_form1_sector_is_valid(after),"Invalid EDC/ECC in changed sector {lba}");
            ensure!(before[..24]==after[..24],"Sector header/subheader changed at {lba}");
            changed_sectors.push(lba);
        }
        ensure!(!changed_sectors.is_empty(),"No actual image changes");
        let edited=DiscPatcher::open(written)?;
        let edits=prepare_cross_cell(&scene,&world,&lib.index)?;
        let mut changed_entries=Vec::new();
        let mut allowed_sectors=HashSet::new();
        for entry in 0..original.entry_count() {
            let before=original.read_entry_footprint(entry)?;
            let after=edited.read_entry_footprint(entry)?;
            if before==after {continue;}
            ensure!(RIM_ELM_MAPS.iter().any(|(_,e)|*e==entry),"Unrelated PROT resource {entry} changed");
            let patches=map_patches(&before,&edits)?;
            let mut expected=before.clone();
            let lba=original.entry_disc_lba(entry).context("Missing native entry LBA")? as usize;
            for (offset,bytes) in patches {
                expected[offset..offset+bytes.len()].copy_from_slice(&bytes);
                for sector in offset/2048..=(offset+bytes.len()-1)/2048 {allowed_sectors.insert(lba+sector);}
            }
            ensure!(expected==after,"MAP {entry} includes unplanned bytes");
            ensure!(before[WALL_OFF..]==after[WALL_OFF..],"Wall/floor/grid/tail changed in MAP {entry}");
            let (id,old_record)=record_at(&before,p.cell,(p.world_x,p.world_z),p.env_slot,[p.rot_x,p.rot_y,p.rot_z])?;
            let new_record=ObjectRecord::parse(&after,id).context("Native record readback")?;
            ensure!(new_record.x_off as i32==old_record.x_off as i32+64,"Native X did not translate");
            let parsed=field_objects::parse_placements(&after);
            let moved=parsed.iter().find(|v|v.obj_idx as usize==id && (v.col,v.row)==p.cell).context("Moved spawn reference missing")?;
            ensure!(moved.world_x==p.world_x+64 && moved.world_z==p.world_z,"Native parser position mismatch");
            writeln!(log,"MAP {entry}: record={id} world=({},{},{}) -> ({},{},{}) collider=({},{})",p.world_x,p.world_y,p.world_z,moved.world_x,p.world_y,moved.world_z,moved.collider_x,moved.collider_z)?;
            changed_entries.push(entry);
        }
        ensure!([1,10,19].iter().all(|e|changed_entries.contains(e)),"Principal Rim Elm states missing");
        ensure!(changed_sectors.iter().all(|lba|allowed_sectors.contains(lba)),"Non-MAP sectors changed");
        writeln!(log,"Readback PASS: changed_entries={changed_entries:?}; changed_sectors={changed_sectors:?}; all other raw sectors and native resources identical; changed EDC/ECC valid")?;
        ensure!(world.set_offset(false,i,[0.0;3]),"Reset test translation");
        // A real original object with nearby wall bits supplies the negative case.
        let blocked=scene.placements.iter().enumerate().find(|(n,_)|
            !source_stencil(&original.read_entry_footprint(1).unwrap(),original_collision_bounds(&world,*n)).is_empty())
            .map(|(n,_)|n).context("No native collision conflict candidate")?;
        world.duplicate_native((false,blocked),scene.placements.len(),900001,[64.0,0.0,0.0])
            .map_err(anyhow::Error::msg)?;
        std::fs::write(dir.join("unsupported_duplicate.lckscene"),scene_instances::serialize(&scene.name,&world))?;
        match plan_export(&source,&scene,&world,&lib.index) {
            Ok(_)=>bail!("Unsupported collision duplicate unexpectedly passed preflight"),
            Err(error)=>writeln!(log,"Negative preflight PASS: placement={blocked}, UID=900001, {error:#}")?,
        }
        ensure!(!rejected_output.exists(),"Rejected edit produced an image");
        writeln!(log,"Supported output={}\nRejected output absent={}\nRuntime not verified by this automated test.",output.display(),rejected_output.display())?;
        Ok(())
    })();
    writeln!(log,"RESULT: {}",match &outcome {Ok(())=>"PASS".into(),Err(e)=>format!("FAIL: {e:#}")})?;
    log.sync_all()?;
    outcome
}


#[test]
#[ignore = "explicit controlled retail NPC translation verification"]
fn controlled_rim_elm_npc_export() -> Result<()> {
    let source=PathBuf::from(std::env::var_os("LCK_P0_SOURCE").context("source required")?);
    let dir=PathBuf::from(std::env::var_os("LCK_P0_OUTPUT_DIR").context("output directory required")?);
    let output=dir.join("RimElm_P0_NPC_translation.img");
    ensure!(!output.exists(),"Output exists");
    let mut log=std::fs::OpenOptions::new().write(true).create_new(true).open(dir.join("npc-verification.txt"))?;
    let result=(||->Result<()> {
        let lib=crate::library::Library::open(&source)?;
        let scene=legaia_engine_world_views::scene_staging::assemble_field_scene(&lib.index,"town01").map_err(anyhow::Error::msg)?;
        let mut world=world_view::build_world(&scene,&lib.index);
        let original=DiscPatcher::open(std::fs::read(&source)?)?;
        let candidates=world.npcs.iter().filter(|n|!n.is_copy && n.drawable && !n.conditional)
            .map(|n|(n.key,n.record,n.psx_position,n.kind)).collect::<Vec<_>>();
        writeln!(log,"Original unconditional drawable MAN actors: {candidates:?}")?;
        let mut selected=None;
        for (key,record,pos,kind) in candidates {
            if !world.set_offset(false,key,[64.0,0.0,0.0]) {continue;}
            match plan_export(&source,&scene,&world,&lib.index) {
                Ok(plan)=>{selected=Some((key,record,pos,kind,plan));break;},
                Err(error)=>writeln!(log,"Actor {record} rejected: {error:#}")?,
            }
            ensure!(world.set_offset(false,key,[0.0;3]),"Reset candidate failed");
        }
        let (key,record,pos,kind,plan)=selected.context("No supported original actor translation")?;
        writeln!(log,"Selected NPC key={key} MAN={record} kind={kind} PSX={pos:?}; delta_view=[64,0,0]\nFull preflight: {}",plan.summary())?;
        std::fs::write(dir.join("supported_npc_translation.lckscene"),scene_instances::serialize(&scene.name,&world))?;
        let predicted=plan.image.clone();
        export(plan,&output)?;
        let bytes=std::fs::read(&output)?;
        ensure!(bytes==predicted,"Image differs from preflight bytes"); drop(predicted);
        let mut changed_sectors=Vec::new();
        for (lba,(before,after)) in original.image().chunks_exact(2352).zip(bytes.chunks_exact(2352)).enumerate() {
            if before!=after {
                ensure!(legaia_iso::write::mode2_form1_sector_is_valid(after),"EDC/ECC invalid at {lba}");
                ensure!(before[..24]==after[..24],"Sector header changed");
                changed_sectors.push(lba);
            }
        }
        ensure!(bytes.len()==original.image().len() && !changed_sectors.is_empty(),"Image size/difference failure");
        let edited=DiscPatcher::open(bytes)?;
        let (entry,offset,budget,_,man)=find_native_man(&original,1)?;
        let edits=prepare(&scene,&world)?;
        let expected=compile_man_npcs(&man,&edits.npcs)?;
        let (_,_,_,_,readback)=find_native_man(&edited,1)?;
        ensure!(readback==expected,"Native MAN readback mismatch");
        let (header,_)=man_actor_header(&man,record)?;
        let (next_header,_)=man_actor_header(&readback,record)?;
        ensure!(header==next_header,"Actor record moved unexpectedly");
        let mut changed_entries=Vec::new();
        for i in 0..original.entry_count() {
            let before=original.read_entry_footprint(i)?;
            let after=edited.read_entry_footprint(i)?;
            if before!=after {
                ensure!(i==entry,"Unrelated resource {i} changed");
                ensure!(before[..offset]==after[..offset] && before[offset+budget..]==after[offset+budget..],"Bytes outside MAN compression budget changed");
                changed_entries.push(i);
            }
        }
        ensure!(changed_entries==vec![entry],"Expected one MAN resource change");
        let start=original.entry_disc_lba(entry).unwrap() as usize;
        ensure!(changed_sectors.iter().all(|&s|s>=start+offset/2048 && s<=start+(offset+budget-1)/2048),"Unexpected raw sectors changed");
        let logical_changes=man.iter().zip(&readback).enumerate().filter_map(|(o,(a,b))|(a!=b).then_some((o,*a,*b))).collect::<Vec<_>>();
        writeln!(log,"Native readback PASS: MAN PROT={entry}, compressed_offset={offset}, budget={budget}, logical_byte_changes={logical_changes:?}")?;
        writeln!(log,"Integrity PASS: {changed_sectors:?} changed sectors; EDC/ECC valid. All other raw sectors and all other {} PROT entries byte-identical. All MAP/TMD/TIM/CLUT resources unchanged.",original.entry_count()-1)?;
        ensure!(world.set_offset(false,key,[0.0;3]),"Reset NPC failed");
        let blocked=scene.placements.iter().enumerate().find(|(n,_)|
            !source_stencil(&original.read_entry_footprint(1).unwrap(),original_collision_bounds(&world,*n)).is_empty())
            .map(|(n,_)|n).context("No collision-bearing original")?;
        world.duplicate_native((false,blocked),scene.placements.len(),900001,[64.0,0.0,0.0]).map_err(anyhow::Error::msg)?;
        std::fs::write(dir.join("unsupported_duplicate.lckscene"),scene_instances::serialize(&scene.name,&world))?;
        match plan_export(&source,&scene,&world,&lib.index) {
            Ok(_)=>bail!("Unsupported duplicate unexpectedly passed"),
            Err(e)=>writeln!(log,"Negative preflight PASS: original placement={blocked}, UID=900001: {e:#}")?,
        }
        ensure!(!dir.join("unsupported_edit_must_not_exist.img").exists(),"Rejected edit generated an image");
        writeln!(log,"Output={}\nRuntime behavior remains unverified.",output.display())?;
        Ok(())
    })();
    writeln!(log,"RESULT: {}",match &result {Ok(())=>"PASS".into(),Err(e)=>format!("FAIL: {e:#}")})?;
    log.sync_all()?;
    result
}
