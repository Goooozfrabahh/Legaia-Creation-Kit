use super::*;

fn fixture()->(Vec<u8>,Vec<u8>) {
    let mut map=vec![0u8;0x12000];
    map[11*32+0x10..11*32+0x12].copy_from_slice(&7u16.to_le_bytes());
    map[11*32+0x12..11*32+0x14].copy_from_slice(&0x12u16.to_le_bytes());
    for (col,row,word) in [(7,8,0x200b_u16),(9,8,0x100b_u16)] {
        let at=0x8000+(row*128+col)*2;map[at..at+2].copy_from_slice(&word.to_le_bytes());
    }
    // A kind-2 dependency on the decoration reference.
    map[0x1000a..0x1000c].copy_from_slice(&18i16.to_le_bytes());
    map[0x1000c..0x1000e].copy_from_slice(&1i16.to_le_bytes());
    map[0x10012..0x10016].copy_from_slice(&[7,8,2,0xcc]);
    let mut load=map.clone();load.resize(0x14000,0);(map,load)
}
#[test]
fn invisible_ground_references_prevent_exclusive_descriptor_claims() ->Result<()> {
    let (map,load)=fixture();let mut out=CellInventory::default();
    map_records(&mut out,123,&map,&load)?;out.validate()?;
    let object=&out.records["object:123:7:8"];
    assert!(object.dependencies.iter().any(|e|e.target=="descriptor:11" && e.relationship==Relationship::SharedReference));
    assert_eq!(out.records["descriptor:11"].fields["reference_count"],2);
    assert!(object.dependencies.iter().any(|e|e.target=="walls" && e.relationship==Relationship::Unresolved));
    assert!(object.dependencies.iter().any(|e|e.target=="trigger:primary:2:0" && e.relationship==Relationship::References));
    assert!(!out.export_authorized);Ok(())
}
#[test]
fn native_identity_uses_cell_and_container_not_descriptor_alone() ->Result<()> {
    let (mut map,mut load)=fixture();let at=0x8000+(8*128+9)*2;
    map[at..at+2].copy_from_slice(&0x200bu16.to_le_bytes());load[..0x12000].copy_from_slice(&map);
    let mut out=CellInventory::default();map_records(&mut out,123,&map,&load)?;
    assert!(out.records.contains_key("object:123:7:8") && out.records.contains_key("object:123:9:8"));
    assert_eq!(out.records["object:123:7:8"].fields["descriptor"],out.records["object:123:9:8"].fields["descriptor"]);
    assert_ne!(out.records["object:123:7:8"].address,out.records["object:123:9:8"].address);Ok(())
}
#[test]
fn physical_fallback_and_malformed_tables_do_not_become_no_dependencies() {
    let (map,mut load)=fixture();load[0x12006..0x12008].copy_from_slice(&0x7fffi16.to_le_bytes());
    load[0x12008..0x1200a].copy_from_slice(&1i16.to_le_bytes());
    assert!(map_records(&mut CellInventory::default(),123,&map,&load).is_err());
    let (_,mut load)=fixture();load[0]^=1;
    assert!(map_records(&mut CellInventory::default(),123,&map,&load).is_err());
}
#[test]
fn storage_disjointness_does_not_claim_runtime_independence() {
    let a=addr("native-map",0,32);let b=addr("native-map",32,32);
    assert_eq!(storage_relation(&a,&b),Relationship::Unrelated);
    assert_eq!(storage_relation(&a,&addr("native-map",16,32)),Relationship::SharedReference);
    assert_eq!(storage_relation(&a,&addr("another-map",0,32)),Relationship::Unresolved);
    assert_eq!(storage_relation(&addr("native-map",usize::MAX,2),&a),Relationship::Unresolved);
}
#[test]
fn floor_and_actor_script_effects_retain_exact_native_instruction_bytes() {
    let mut script=vec![0x4c,0x9e];script.extend_from_slice(&[0;32]);script.extend_from_slice(&[0x23,4,5,0x21]);
    let scan=super::super::native_script_dependencies::scan_script(&script,0);
    assert!(scan.error.is_none());assert_eq!(scan.effects.len(),2);
    assert_eq!((scan.effects[0].kind,scan.effects[0].pc,scan.effects[0].size),("FloorLadderOperation",0,34));
    assert_eq!(scan.effects[0].raw,&script[..34]);
    assert_eq!((scan.effects[1].kind,scan.effects[1].pc),("ActorSpatialOperation",34));
}
#[test]
fn native_player_and_system_channels_are_shared_contexts_not_man_record_indices()->Result<()> {
    let mut cell=CellInventory::default();
    let man=super::super::native_script_dependencies::fixture_man(&[&[0xa1,0xf8,0xa1,0xfb,0x21]]);
    man_records(&mut cell,&man)?;cell.validate()?;
    let dependencies=&cell.records["man:0"].dependencies;
    assert!(dependencies.iter().any(|d|d.target=="context:player"&&d.relationship==Relationship::Affects));
    assert!(dependencies.iter().any(|d|d.target=="context:system"&&d.relationship==Relationship::Affects));
    assert!(!cell.records.keys().any(|id|id=="unresolved-context:man:248"||id=="unresolved-context:man:251"));Ok(())
}
#[test]
fn all_operations_keep_shared_descriptor_and_unknown_physics_requirements()->Result<()> {
    let (map,load)=fixture();let mut out=CellInventory::default();map_records(&mut out,456,&map,&load)?;
    let selected=BTreeSet::from(["object:456:7:8".into()]);
    for op in [Operation::Move,Operation::Rotate,Operation::Scale,Operation::Duplicate,Operation::Delete,Operation::CrossCellImport] {
        let report=out.assess(&selected,op)?;
        assert!(report.shared_references.contains("descriptor:11"));
        assert!(report.dependency_closure.contains("trigger:primary:2:0"));
        assert!(!report.unresolved_relationships.is_empty());assert!(!report.required_capabilities.is_empty());
        assert!(!report.export_authorized);
    }Ok(())
}
#[test]
fn dependency_closure_retains_incoming_scripts_and_does_not_conflate_them_with_ownership()->Result<()> {
    let mut out=CellInventory::default();
    let man=super::super::native_script_dependencies::fixture_man(&[&[0x21],&[0xa1,0,0x21]]);
    man_records(&mut out,&man)?;
    let closure=out.closure(&BTreeSet::from(["man:0".into()]))?;
    assert!(closure.contains("man:1"));
    assert!(out.records["man:1"].dependencies.iter().any(|e|e.target=="man:0"&&e.relationship==Relationship::Affects));
    assert!(out.closure(&BTreeSet::from(["missing".into()])).is_err());out.validate()?;Ok(())
}

#[test]
#[ignore="explicit original-disc representative formats and six-request dependency benchmark; no IMG writes"]
fn representative_native_graphs_and_six_request_dependency_benchmark()->Result<()> {
    use std::path::PathBuf;
    let source=PathBuf::from(std::env::var_os("LCK_NATIVE_INVENTORY_SOURCE").context("Original inventory source required")?);
    let library=crate::library::Library::open(&source)?;
    // Test fixtures cover ordinary field, compressed bank, streaming MAN and
    // kingdom layouts. Production analysis contains no scene/ID exceptions.
    for name in ["town01","koin3","rikuroa","map01"] {
        let cell=inspect(&library.index,name)?;cell.validate()?;
        ensure!(cell.records.contains_key("map"),"{name}: MAP disappeared");
        let references=cell.records.values().filter(|r|r.kind=="MapDescriptor").map(|r|r.fields["reference_count"].as_u64().unwrap()).sum::<u64>();
        ensure!(references==16384,"{name}: native references omitted");
        println!("Representative {name}: {} records, {} explicit gaps",cell.records.len(),cell.gaps.len());
    }
    let benchmark=PathBuf::from(std::env::var_os("LCK_M2_SOURCE").context("Six-request fixture source required")?);
    let library=crate::library::Library::open(&benchmark)?;
    let cell=inspect(&library.index,"town01")?;
    let selected=[(37,18),(41,18),(41,22),(31,20),(32,20),(38,25)].into_iter().map(|(x,z)|format!("object:1:{x}:{z}")).collect::<BTreeSet<_>>();
    let report=cell.assess(&selected,Operation::Move)?;
    ensure!(report.selected.len()==6,"Dropped benchmark request");
    ensure!(report.shared_references.contains("descriptor:162"),"Shared terrain reference was missed");
    for target in ["man:0","man:35","man:92","man:95","man:104"] {ensure!(report.dependency_closure.contains(target),"Incoming house context dependency {target} omitted");}
    let stairs=report.dependency_closure.iter().filter_map(|id|cell.records.get(id)).filter(|r|r.kind=="ElevationOverride").any(|r|r.fields["raw"]==json!([37,18,2,204]));
    ensure!(stairs,"Native staircase record omitted");
    ensure!(!report.unresolved_relationships.is_empty()&&!report.export_authorized,"Unresolved ownership became export permission");
    println!("Six-request graph: {} closure records, {} shared references, {} explicit unresolved relationships; export remains unauthorized",report.dependency_closure.len(),report.shared_references.len(),report.unresolved_relationships.len());
    // Repartition the actual staircase samples against actual retained native
    // destination floors. This is a representability probe, not ownership or
    // permission to change any surface.
    use super::super::native_elevation_codec::{self as heights,Destination};
    use legaia_engine_field::field_elevation::{self,ElevationOverride};
    let native=Scene::load(&library.index,"town01")?;
    let entry=native.field_map_index(&library.index).context("No benchmark MAP")?;
    let map=library.index.entry_bytes(entry)?;
    let lba=library.index.entry_start_lba_retail(entry as u16).context("No MAP LBA")?;
    let load=library.index.prot_dat_raw_bytes(lba as u64*2048,0x14000)?;
    let lut=native.field_floor_height_lut(&library.index)?.context("No native height LUT")?;
    let mut overrides=field_elevation::parse_elevation_overrides(&load[0x10000..0x12000]);
    overrides.extend(field_elevation::parse_elevation_overrides(&load[0x12000..]));
    let corners=|cell:[u8;2]|{
        let at=0x4000+cell[1] as usize*128+cell[0] as usize;
        [0,1,128,129].map(|offset|-(lut[(map[at+offset]&15) as usize] as i32))
    };
    let mut background=BTreeMap::new();
    for tile in [[34,18],[35,18],[34,19],[35,19]] {
        let values=corners(tile);let mean=values.iter().sum::<i32>()>>2;
        let at=0x8000+(tile[1] as usize*128+tile[0] as usize)*2;
        let word=u16::from_le_bytes(map[at..at+2].try_into()?);
        let step=overrides.iter().find(|r|[r.tile_x,r.tile_z]==tile);
        let has_step=word&0x800!=0;
        let retained=if has_step {heights::decode(&step.copied().unwrap_or(ElevationOverride{tile_x:tile[0],tile_z:tile[1],coarse:0,quads:0}),mean)?.map(|s|s.height)}else{
            std::array::from_fn(|i|{
                let wx=32+(i%2) as i64*64;let wz=32+(i/2) as i64*64;
                let [a,b,c,d]=values.map(i64::from);
                (((b*wx+a*(128-wx))*(128-wz)+c*(128-wx)*wz+d*wx*wz)/16384) as i32
            })
        };
        let frame=Destination{floor_mean:mean,retained,piecewise_constant:has_step||values.iter().all(|v|*v==values[0])};
        println!("Native retained destination {tile:?}: {frame:?}");background.insert(tile,frame);
    }
    let source=overrides.iter().find(|r|(r.tile_x,r.tile_z)==(37,18)).context("Source height record missing")?;
    let samples=heights::decode(source,corners([37,18]).iter().sum::<i32>()>>2)?;
    println!("Native staircase samples: {samples:?}");
    let height_probe=heights::repartition(&samples,[-5,1],&background);
    println!("Exact staircase repartition representability: {height_probe:?}");
    ensure!(height_probe.is_err(),"Fixture expectations changed: review destination floor representation");
    let upper_probe=heights::repartition(&samples[..2],[-5,1],&background).unwrap_err();
    println!("Native upper destination step-span probe: {upper_probe:#}");
    ensure!(upper_probe.to_string().contains("span 64"),"Upper destination representation changed");
    if let Some(output)=std::env::var_os("LCK_NATIVE_BENCHMARK_REPORT") {
        let path=PathBuf::from(output);ensure!(path.is_absolute()&&!path.exists(),"Use a NEW benchmark report path");
        let file=std::fs::OpenOptions::new().write(true).create_new(true).open(path)?;serde_json::to_writer_pretty(file,&report)?;
    }
    Ok(())
}

/// Runs ONLY a test harness. Never starts the LCK GUI, renderer or emulator.
/// Reports are create-only; geometry/texture payloads are not copied.
#[test]
#[ignore="explicit game-wide read-only original-disc inventory; writes new JSON reports only"]
fn original_disc_game_wide_native_inventory()->Result<()> {
    use std::{path::PathBuf,io::Write};
    let source=PathBuf::from(std::env::var_os("LCK_NATIVE_INVENTORY_SOURCE").context("LCK_NATIVE_INVENTORY_SOURCE required")?);
    let destination=PathBuf::from(std::env::var_os("LCK_NATIVE_INVENTORY_OUTPUT").context("LCK_NATIVE_INVENTORY_OUTPUT required")?);
    ensure!(destination.is_absolute()&&!destination.exists(),"Inventory requires a NEW absolute output directory");
    let source_hash=digest(&std::fs::read(&source)?);
    let library=crate::library::Library::open(&source)?;
    let mut names=library.index.cdname_scene_names();names.sort();names.dedup();
    ensure!(!names.is_empty(),"No CDNAME inventory available");
    std::fs::create_dir(&destination)?;
    let mut summaries=vec![];let mut resource_users=BTreeMap::<String,BTreeSet<String>>::new();
    let mut covered=BTreeSet::new();
    for (i,name) in names.iter().enumerate() {
        let result=inspect(&library.index,name);
        let report=match result {
            Ok(cell)=>{
                let mut counts=BTreeMap::<String,usize>::new();
                for record in cell.records.values() {
                    *counts.entry(record.kind.clone()).or_default()+=1;
                    if matches!(record.kind.as_str(),"NativeModel"|"TimUpload") {
                        if let Some(hash)=record.fields["sha256"].as_str() {
                            resource_users.entry(format!("{}:{hash}",record.kind)).or_default().insert(name.clone());
                        }
                    }
                }
                for entry in cell.entry_range[0]..cell.entry_range[1] {covered.insert(entry);}
                summaries.push(json!({"scene":name,"report":format!("cell-{i:04}.json"),"counts":counts,"gaps":cell.gaps.len(),"map":cell.records.contains_key("map"),"status":"inventoried_with_explicit_gaps"}));
                serde_json::to_value(cell)?
            },
            Err(error)=>{
                summaries.push(json!({"scene":name,"report":format!("cell-{i:04}.json"),"status":"failed","error":format!("{error:#}")}));
                json!({"schema":1,"scene":name,"error":format!("{error:#}"),"export_authorized":false})
            }
        };
        let mut file=std::fs::OpenOptions::new().create_new(true).write(true).open(destination.join(format!("cell-{i:04}.json")))?;
        serde_json::to_writer(&mut file,&report)?;file.flush()?;
        if i%25==0 {println!("Inventoried {}/{} CDNAME blocks; latest {name}",i+1,names.len());}
    }
    let entries=library.index.entries().iter().map(|entry|json!({"entry":entry.index,"size":entry.size_bytes,"lba":entry.start_lba,"in_inventoried_cdname_block":covered.contains(&entry.index)})).collect::<Vec<_>>();
    let summary=json!({"schema":1,"source_sha256":source_hash,"cdname_blocks":names.len(),"cells":summaries,"prot_entries":entries,
        "resource_content_users":resource_users,"all_named_blocks_attempted":true,"unknown_layouts_retained":true,
        "universal_export_verified":false,"gui_verified":false,"native_gameplay_verified":false,"export_authorized":false});
    let file=std::fs::OpenOptions::new().create_new(true).write(true).open(destination.join("inventory.json"))?;
    serde_json::to_writer_pretty(file,&summary)?;
    ensure!(digest(&std::fs::read(&source)?)==source_hash,"Source disc changed");
    println!("Finished {} named blocks. Source unchanged. Reports: {}",names.len(),destination.display());Ok(())
}
