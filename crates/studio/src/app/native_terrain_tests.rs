//! Automated native-record tests. No GUI or emulator is launched.
use super::*;

fn fixture()->(Vec<u8>,MoveEdit) {
    let mut map=vec![0;MAP_LEN];
    let id=162usize;let cell=(41u8,22u8);
    map[id*32+0x10..id*32+0x12].copy_from_slice(&10u16.to_le_bytes());
    map[id*32+0x12..id*32+0x14].copy_from_slice(&0x812u16.to_le_bytes());
    map[id*32+10..id*32+12].copy_from_slice(&1024u16.to_le_bytes());
    map[id*32+0x14..id*32+0x18].copy_from_slice(&[10,27,0x80,0x7d]);
    map[id*32+0x1e]=3;
    for (col,row) in [cell,(45,22)] {
        let off=GRID_OFF+(row as usize*128+col as usize)*2;
        map[off..off+2].copy_from_slice(&(id as u16|0x3000).to_le_bytes());
        map[WALL_OFF+row as usize*128+col as usize]=6;
    }
    (map,MoveEdit{cell,source_position:(5312,2880),env_slot:10,
        source_rotation:[0,1024,0],delta:[-320.0,32.0,64.0],
        collision_bounds:Some([5248,5376,2816,2944])})
}

fn apply(map:&[u8],patches:Vec<(usize,Vec<u8>)>)->Vec<u8> {
    let mut next=map.to_vec();for (offset,bytes) in patches {next[offset..offset+bytes.len()].copy_from_slice(&bytes);}next
}

#[test]
fn terrain_translation_exports_and_reparses_a_synthetic_raw_img() -> Result<()> {
    let (map,edit)=fixture();let image=super::tests::map_disc(&map);
    let directory=std::env::current_dir()?.join("target").join(format!("m2-terrain-synthetic-{}-{}",std::process::id(),std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_nanos()));
    std::fs::create_dir(&directory)?;
    let source=directory.join("synthetic-source.img");std::fs::write(&source,&image)?;
    let original=DiscPatcher::open(image.clone())?;
    let batch=ExportEdits{terrain_moves:vec![edit.clone()],..Default::default()};
    let make=||compile_plan(source.clone(),DiscPatcher::open(image.clone())?,image.len(),0,&batch);
    let first=make()?;ensure!(first.image==make()?.image,"Nondeterministic terrain export");
    let output=directory.join("synthetic-terrain-translated.img");export(first,&output)?;
    let read=DiscPatcher::open(std::fs::read(&output)?)?;
    let result=read.read_entry_footprint(0)?;
    let tiles=field_objects::parse_terrain_tiles(&result);
    let selected=tiles.iter().find(|p|(p.col,p.row)==edit.cell).context("Missing serialized terrain")?;
    ensure!((selected.world_x,selected.world_z,selected.y_off)==(4992,2944,-32),"Translation failed readback");
    ensure!(result[WALL_OFF..GRID_OFF]==map[WALL_OFF..GRID_OFF] && result[0x10000..]==map[0x10000..],"Ground or trigger records changed");
    ensure!(read.read_entry_footprint(1)?==original.read_entry_footprint(1)?,"Unrelated native entry changed");
    ensure!(std::fs::read(&source)?==image,"Source image changed");
    ensure!(export(make()?,&output).is_err(),"Existing output was overwritten");
    Ok(())
}

#[test]
fn terrain_translation_detaches_one_reference_and_preserves_ground_and_resources() {
    let (map,edit)=fixture();
    let next=apply(&map,map_patches(&map,&ExportEdits{terrain_moves:vec![edit.clone()],..Default::default()}).unwrap());
    let tiles=field_objects::parse_terrain_tiles(&next);
    let moved=tiles.iter().find(|p|(p.col,p.row)==edit.cell).unwrap();
    assert_ne!(moved.obj_idx,162);
    assert_eq!((moved.world_x,moved.world_z,moved.y_off),(4992,2944,-32));
    let retained=tiles.iter().find(|p|(p.col,p.row)==(45,22)).unwrap();
    assert_eq!((retained.obj_idx,retained.world_x,retained.world_z,retained.y_off),(162,5824,2880,0));
    assert_eq!(&next[162*32..163*32],&map[162*32..163*32]);
    let at=moved.obj_idx as usize*32;
    assert_eq!(&next[at+6..at+32],&map[162*32+6..163*32]);
    assert_eq!(&next[WALL_OFF..GRID_OFF],&map[WALL_OFF..GRID_OFF]);
    assert_eq!(&next[0x10000..],&map[0x10000..]);
    let off=GRID_OFF+(22*128+41)*2;
    assert_eq!(u16::from_le_bytes(next[off..off+2].try_into().unwrap())&!511,0x3000);
    let ground_before=field_objects::build_walk_heightfield(&map,&[0;16]);
    let ground_after=field_objects::build_walk_heightfield(&next,&[0;16]);
    assert_eq!(ground_before.positions,ground_after.positions);
}

#[test]
fn terrain_translation_rejects_collision_without_ownership_and_bad_cull_reach() {
    let (mut map,edit)=fixture();
    let original=map.clone();
    let (off,bit)=wall_offset(82,46).unwrap();map[off]|=bit;
    let error=map_patches(&map,&ExportEdits{terrain_moves:vec![edit.clone()],..Default::default()}).unwrap_err();
    assert!(error.to_string().contains("Unresolved collision ownership"));
    let mut distant=edit.clone();distant.delta[0]=-385.0;
    assert!(map_patches(&original,&ExportEdits{terrain_moves:vec![distant],..Default::default()}).unwrap_err().to_string().contains("cull radius"));
    let mut fractional=edit;fractional.delta[0]=0.5;
    assert!(map_patches(&original,&ExportEdits{terrain_moves:vec![fractional],..Default::default()}).is_err());
}

#[test]
fn terrain_translation_cannot_reinterpret_a_placement_or_overwrite_shared_resources() {
    let (mut map,edit)=fixture();
    map[162*32+0x12]|=field_objects::FLAG_PLACED as u8;
    assert!(map_patches(&map,&ExportEdits{terrain_moves:vec![edit.clone()],..Default::default()}).unwrap_err().to_string().contains("not a native terrain"));
    let (mut map,edit)=fixture();
    for id in 4..512 {if id!=162 {map[id*32]=1;}}
    assert!(map_patches(&map,&ExportEdits{terrain_moves:vec![edit],..Default::default()}).unwrap_err().to_string().contains("blank and unreferenced"));
}

#[test]
fn terrain_translation_preserves_and_reports_native_elevation_dependency() {
    let (mut map,edit)=fixture();
    // Genuine trigger-table layout: kind 2 header +0xa/+0xc, four-byte rows.
    map[0x1000a..0x1000c].copy_from_slice(&18i16.to_le_bytes());
    map[0x1000c..0x1000e].copy_from_slice(&1i16.to_le_bytes());
    map[0x10012..0x10016].copy_from_slice(&[edit.cell.0,edit.cell.1,2,0xcc]);
    let saved=map.clone();
    let error=map_patches(&map,&ExportEdits{terrain_moves:vec![edit],..Default::default()}).unwrap_err();
    assert!(error.to_string().contains("MAP kind-2 byte 0x12, payload [02, cc]"));
    assert_eq!(map,saved);
}

#[test]
#[ignore="explicit read-only exact M2 source investigation; no output IMG or GUI"]
fn exact_six_request_batch_serializes_terrain_but_retains_dependency_blocks()->Result<()> {
    let path=PathBuf::from(std::env::var_os("LCK_M2_SOURCE").context("LCK_M2_SOURCE required")?);
    let source_hash=super::super::collision_recipes::digest(&std::fs::read(&path)?);
    ensure!(source_hash=="3d2f0fd87192d3e24eedd51deb6da19051d30519ece3297d1de0948ab403f82f","Exact six-request source hash differs");
    let library=crate::library::Library::open(&path)?;
    let scene=legaia_engine_world_views::scene_staging::assemble_field_scene(&library.index,"town01").map_err(anyhow::Error::msg)?;
    let mut world=world_view::build_world(&scene,&library.index);
    let text="LCK_SCENE_V1\ttown01\nMOVE\tterrain\t36\t-320\t0\t64\nMOVE\tterrain\t37\t-320\t0\t64\nMOVE\tterrain\t43\t-320\t0\t64\nMOVE\tplacement\t12\t-320\t0\t64\nMOVE\tplacement\t13\t-320\t0\t64\nMOVE\tplacement\t14\t-320\t0\t64\n";
    let doc=scene_instances::parse(text,"town01",&world).map_err(anyhow::Error::msg)?;
    scene_instances::apply(&doc,&mut world,scene.placements.len(),&library.index).map_err(anyhow::Error::msg)?;
    let inventory=super::super::native_asset_plan::collect(&scene,&world,&library.index)?;
    ensure!(inventory.requests.len()==6,"Dropped request");inventory.validate_routes()?;
    let edits=prepare(&scene,&world)?;
    ensure!(edits.moves.len()==3 && edits.terrain_moves.len()==3,"Dropped native edit");
    let native=Scene::load(&library.index,"town01")?;
    let entry=native.field_map_index(&library.index).context("No MAP")?;
    let map=library.index.entry_bytes(entry)?;
    let mut record_probe=map.as_ref().clone();
    // Isolated descriptor-layer verification only. This is deliberately NOT
    // passed off as a valid physics plan or emitted as an IMG.
    move_records(&mut record_probe,&edits.terrain_moves,true)?;
    let before=field_objects::parse_terrain_tiles(&map);
    let after=field_objects::parse_terrain_tiles(&record_probe);
    for old in before.iter().filter(|p|!ObjectRecord::parse(&map,p.obj_idx as usize).unwrap().is_placed()) {
        let new=after.iter().find(|p|(p.col,p.row)==(old.col,old.row)).context("Terrain reference disappeared")?;
        let selected=edits.terrain_moves.iter().any(|e|e.cell==(old.col,old.row));
        let delta=if selected {(-320,64)} else {(0,0)};
        ensure!((new.world_x,new.world_z,new.y_off)==(old.world_x+delta.0,old.world_z+delta.1,old.y_off),"Unexpected terrain position change");
        ensure!(new.pack_index==old.pack_index,"Terrain mesh identity changed");
    }
    ensure!(map[WALL_OFF..GRID_OFF]==record_probe[WALL_OFF..GRID_OFF] && map[0x10000..]==record_probe[0x10000..],"Floor/wall/trigger mutation");
    for edit in &edits.terrain_moves {
        let error=map_patches(&map,&ExportEdits{terrain_moves:vec![edit.clone()],..Default::default()}).expect_err("Unknown terrain wall ownership must block");
        println!("Terrain {:?}: {error:#}",edit.cell);
    }
    let error=plan_export(&path,&scene,&world,&library.index).err().context("Six-request export unexpectedly allowed unresolved dependencies")?;
    let message=format!("{error:#}");
    ensure!(!message.contains("Unsupported native asset categories"),"Terrain route still missing");
    ensure!(message.contains("Native binding unresolved for record #137"),"Unexpected blocker: {message}");
    println!("Exact six-request batch: {message}");
    ensure!(super::super::collision_recipes::digest(&std::fs::read(&path)?)==source_hash,"Source IMG changed");
    Ok(())
}
