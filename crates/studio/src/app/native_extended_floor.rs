//! Experimental, explicit floor-component extension. Never an implicit
//! mesh/wall/script ownership claim or a production six-object export waiver.
use anyhow::{Result,ensure,Context};
use legaia_code_hooks::extended_floor::{self,Row};
use legaia_scene_formats::field_map;
use super::collision_recipes::digest;

#[derive(Debug,serde::Serialize)]
pub struct Request {pub scene:String,pub source_tile:[u8;2],pub delta:[i32;2],pub duplicate:bool,pub height_delta:i16}

/// Headless experimental entry point. This deliberately exports floor-only
/// scope; mesh, wall and actor ownership are never inferred from a tile.
pub fn export(source:&std::path::Path,output:&std::path::Path,request:&Request)->Result<String> {
    use legaia_disc_patch::disc::DiscPatcher;
    use std::io::Write;
    ensure!(output.is_absolute() && !output.exists() && output.file_name().and_then(|x|x.to_str()).is_some_and(|x|x.contains("EXPERIMENTAL")),"Use a new absolute EXPERIMENTAL output IMG");
    let staging=output.with_extension("lckbuilding");let metadata=output.with_extension("json");
    ensure!(!staging.exists() && !metadata.exists(),"Experimental output/staging/receipt already exists");
    let original=std::fs::read(source)?;
    ensure!(digest(&original)=="750dbcbadb70362feb3468467bbe7efbf3d151a414caa428c66c368d4b0b73e6","Unsupported retail image profile; no engine patch applied");
    let library=crate::library::Library::open(source)?;
    let mut disc=DiscPatcher::open(original.clone())?;
    let exe=disc.read_named_file("SCUS_942.54").context("No native SCUS")?;
    ensure!(digest(&library.index.entry_bytes(897)?)=="0b39d892d783ffcc89dac06ee395540be0a7392c5cbc79fb9ba3442daccc09f6","Unknown native overlay profile");
    let scene=legaia_engine_scene::scene::Scene::load(&library.index,&request.scene)?;
    let entry=scene.field_map_index(&library.index).context("No native MAP")? as usize;
    let map=disc.read_entry_footprint(entry)?;
    let rows=legaia_engine_field::field_elevation::parse_elevation_overrides(&map[0x10000..]);
    let selected=rows.iter().filter(|r|[r.tile_x,r.tile_z]==request.source_tile).collect::<Vec<_>>();
    ensure!(selected.len()==1,"Select one unambiguous native primary floor component");
    let expected=super::native_elevation_codec::serialize(selected[0])?;
    let plan=translate(&map,&exe,expected,request.delta,request.duplicate,request.height_delta)?;plan.verify_source(&map,&exe)?;
    let mut expected_exe=exe;
    for edit in &plan.code {
        disc.patch_named_file("SCUS_942.54",edit.file_off as u64,&edit.bytes)?;
        expected_exe[edit.file_off..edit.file_off+edit.bytes.len()].copy_from_slice(&edit.bytes);
    }
    let encoded=extended_floor::encode(&extended_floor::decode(&plan.map[plan.table_offset..])?)?;
    disc.patch_prot_entry(entry,plan.table_offset as u64,&encoded)?;
    let read=DiscPatcher::open(disc.image().to_vec())?;
    ensure!(read.read_named_file("SCUS_942.54").as_deref()==Some(expected_exe.as_slice()) && read.read_entry_footprint(entry)?==plan.map,"Experimental native readback mismatch");
    let prior=DiscPatcher::open(original.clone())?;
    for other in 0..prior.entry_count(){if other!=entry {ensure!(read.read_entry(other)?==prior.read_entry(other)?,"Unrelated PROT resource {other} changed");}}
    let scus=library.iso_files.iter().find(|f|f.path.eq_ignore_ascii_case("SCUS_942.54")).context("Missing SCUS ISO extent")?;
    let mut allowed=std::collections::BTreeSet::new();
    for edit in &plan.code {for sector in edit.file_off/2048..=(edit.file_off+edit.bytes.len()-1)/2048 {allowed.insert(scus.lba as usize+sector);}}
    let map_lba=disc.entry_disc_lba(entry).context("Missing MAP extent")? as usize;
    for sector in plan.table_offset/2048..=(plan.table_offset+encoded.len()-1)/2048 {allowed.insert(map_lba+sector);}
    let mut changed_sectors=Vec::new();
    for (i,(before,after)) in original.chunks_exact(2352).zip(disc.image().chunks_exact(2352)).enumerate() {
        if before!=after {ensure!(allowed.contains(&i) && legaia_iso::write::mode2_form1_sector_is_valid(after),"Unexpected/invalid changed raw sector {i}");changed_sectors.push(i);}
    }
    let report=serde_json::json!({"schema":"lck-experimental-extended-floor-v1","scope":"explicit floor component only; geometry, walls, MAN and scripts unchanged","request":request,"source_sha256":digest(&original),"output_sha256":digest(disc.image()),"map_entry":entry,"table_offset":plan.table_offset,"encoded_bytes":encoded.len(),"code_va":extended_floor::CODE,"code_bytes":plan.code[0].bytes.len(),"changed_raw_sectors":changed_sectors,"cpu_regression":"retail regression available; not executed by this export command","gui_verified":false,"native_gameplay_verified":false});
    let mut file=std::fs::OpenOptions::new().write(true).create_new(true).open(&staging)?;
    file.write_all(disc.image())?;file.sync_all()?;drop(file);
    ensure!(digest(&std::fs::read(&staging)?)==report["output_sha256"].as_str().unwrap(),"Staged IMG hash mismatch");
    std::fs::hard_link(&staging,output).context("Publish experimental IMG without replacement")?;std::fs::remove_file(&staging)?;
    let mut receipt=std::fs::OpenOptions::new().write(true).create_new(true).open(metadata)?;receipt.write_all(&serde_json::to_vec_pretty(&report)?)?;receipt.sync_all()?;
    ensure!(digest(&std::fs::read(source)?)==report["source_sha256"].as_str().unwrap(),"Retail input changed");
    Ok(format!("Experimental floor-only IMG: {}. Native byte/sector readback passed; GUI/gameplay NOT verified.",output.display()))
}

pub(super) struct Plan {pub map:Vec<u8>,pub code:Vec<legaia_code_hooks::arenas::Edit>,pub table_offset:usize,source_map:String,source_exe:String}
impl Plan {
    pub fn verify_source(&self,map:&[u8],exe:&[u8])->Result<()> {
        ensure!(digest(map)==self.source_map && digest(exe)==self.source_exe,"Extended floor plan source changed");Ok(())
    }
}

/// Translate an explicitly selected native kind-2 surface by 64-unit steps.
/// Its old row remains as the immutable native sampling source. Sparse mode-2
/// redirects reveal the original bilinear base in the vacated subcells.
/// Destination subcells retain their baseline surface everywhere else.
pub(super) fn translate(map:&[u8],exe:&[u8],expected:[u8;4],delta:[i32;2],duplicate:bool,height_delta:i16)->Result<Plan> {
    ensure!(digest(exe)=="292256e2e66db42727f613406785e444254d3f699569e611f65fcf1c6d2f3482","Unknown experimental engine profile");
    compile_translation(map,exe,expected,delta,duplicate,height_delta)
}

fn compile_translation(map:&[u8],exe:&[u8],expected:[u8;4],delta:[i32;2],duplicate:bool,height_delta:i16)->Result<Plan> {
    ensure!(map.len()==0x12000 && expected[0]<128 && expected[1]<128,"Invalid native floor source MAP");
    ensure!(delta.iter().all(|v|v%64==0) && delta!=[0;2],"Extended floor translation requires a nonzero 64-unit displacement");
    let parsed=field_map::detect(map).context("Invalid native MAP trigger tables")?;
    let header=parsed.trigger_block().context("No primary floor table")?;
    let table=extended_floor::table_offset(map)?;
    ensure!(table>=0x10000+header.end_offset,"Native triggers overlap extension storage");
    let source_rows=legaia_engine_field::field_elevation::parse_elevation_overrides(&map[0x10000..]);
    ensure!(source_rows.iter().filter(|r|r.tile_x==expected[0] && r.tile_z==expected[1]).count()==1,"Missing/duplicate native floor source");
    let source=source_rows.iter().find(|r|r.tile_x==expected[0] && r.tile_z==expected[1]).unwrap();
    ensure!(super::native_elevation_codec::serialize(source)?==expected,"Native floor source changed");
    let go=0x8000+(expected[1] as usize*128+expected[0] as usize)*2;
    ensure!(u16::from_le_bytes(map[go..go+2].try_into()?)&0x1800==0x800,"Source is not an active ordinary kind-2 surface");
    let mut rows=std::collections::BTreeMap::new();
    for z in 0..2 {for x in 0..2 {
        let old=[expected[0] as i32*2+x,expected[1] as i32*2+z];
        if !duplicate {rows.insert([old[0] as u8,old[1] as u8],Row{destination:[old[0] as u8,old[1] as u8],mode:2,delta:[0;2],height_delta:0});}
    }}
    for z in 0..2 {for x in 0..2 {
        let target=[expected[0] as i32*2+x+delta[0]/64,expected[1] as i32*2+z+delta[1]/64];
        ensure!(target.iter().all(|v|(0..256).contains(v)),"Extended floor destination outside scene");
        let key=[target[0] as u8,target[1] as u8];
        let tile=[key[0]/2,key[1]/2];let grid=0x8000+(tile[1] as usize*128+tile[0] as usize)*2;
        // 0x1000 changes actor surface flags, not the height sampler. Retail
        // selects kind-2 solely with 0x0800 at 80019384. Preserve all stored
        // bits and baseline behavior outside the explicit redirected subcell.
        ensure!(u16::from_le_bytes(map[grid..grid+2].try_into()?)&0x800==0 || tile==[expected[0],expected[1]],"Destination already has an unrelated native kind-2 floor contribution");
        rows.insert(key,Row{destination:key,mode:1,delta:[i16::try_from(-delta[0])?,i16::try_from(-delta[1])?],height_delta});
    }}
    let encoded=extended_floor::encode(&rows.into_values().collect::<Vec<_>>())?;
    ensure!(table+encoded.len()<=map.len() && map[table..table+encoded.len()].iter().all(|&b|b==0),"Native extension storage occupied at {table:#x}; no guessed allocation");
    let mut output=map.to_vec();output[table..table+encoded.len()].copy_from_slice(&encoded);
    ensure!(extended_floor::encode(&extended_floor::decode(&output[table..])?)?==encoded,"Extended native readback failed");
    ensure!(output[..table]==map[..table],"Original native records changed");
    let code=extended_floor::plan(exe)?;
    Ok(Plan{map:output,code,table_offset:table,source_map:digest(map),source_exe:digest(exe)})
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use legaia_code_hooks::mips_sim::Cpu;
    use legaia_disc_patch::disc::DiscPatcher;
    const MAP:u32=0x80100000;
    const ACTOR:u32=0x801b0000;
    const STACK:u32=0x801bff00;
    const STOP:u32=0x80000080;
    fn executable_fixture()->Vec<u8> {
        let mut exe=vec![0;0x71000];exe[..8].copy_from_slice(b"PS-X EXE");exe[0x18..0x1c].copy_from_slice(&0x80010000u32.to_le_bytes());
        for (va,words) in [(0x80019278u32,[0x27bdffd8u32,0x00803821]),(0x800192fc,[0x94840000,0x04600011]),(0x80019588,[0x8fbf0024,0x8fb40020])] {
            let at=(va-0x80010000) as usize+0x800;for (i,w) in words.iter().enumerate(){exe[at+i*4..at+i*4+4].copy_from_slice(&w.to_le_bytes());}
        }exe
    }
    #[test]
    fn extended_floor_move_copy_overlap_and_storage_guards()->Result<()> {
        let map=super::super::native_floor_patch::fixture()[..0x12000].to_vec();let exe=executable_fixture();
        for duplicate in [false,true] {
            let plan=compile_translation(&map,&exe,[37,18,2,0xcc],[-320,64],duplicate,0)?;
            let rows=extended_floor::decode(&plan.map[plan.table_offset..])?;
            assert_eq!(rows.len(),if duplicate {4}else{8});
            assert_eq!(&plan.map[..plan.table_offset],&map[..plan.table_offset]);
            plan.verify_source(&map,&exe)?;
            let mut stale=map.clone();stale[1]=1;assert!(plan.verify_source(&stale,&exe).is_err());
            let mut occupied=map.clone();occupied[plan.table_offset]=1;
            assert!(compile_translation(&occupied,&exe,[37,18,2,0xcc],[-320,64],duplicate,0).is_err());
        }
        let overlap=compile_translation(&map,&exe,[37,18,2,0xcc],[64,0],false,0)?;
        assert_eq!(extended_floor::decode(&overlap.map[overlap.table_offset..])?.len(),6);
        assert!(compile_translation(&map,&exe,[37,18,2,0xcc],[1,0],false,0).is_err());
        let mut bad_code=exe.clone();let at=(extended_floor::CODE-0x80010000) as usize+0x800;bad_code[at]=1;
        assert!(compile_translation(&map,&bad_code,[37,18,2,0xcc],[-320,64],false,0).is_err());
        assert!(translate(&map,&exe,[37,18,2,0xcc],[-320,64],false,0).is_err());Ok(())
    }
    fn machine(exe:&[u8],overlay:&[u8],load:&[u8],lut:[i16;16],delays:bool)->Cpu {
        let mut cpu=Cpu::new();cpu.load_delays=delays;cpu.strict_alignment=true;
        let base=u32::from_le_bytes(exe[0x18..0x1c].try_into().unwrap());
        cpu.load(base,&exe[0x800..]);cpu.load(0x801ce818,overlay);cpu.load(MAP,load);
        cpu.wr32(0x1f8003ec,MAP);
        for (i,v) in lut.iter().enumerate(){cpu.wr16(0x1f80035c+i as u32*2,(-*v) as u16);}
        cpu
    }
    fn query(cpu:&mut Cpu,point:[u16;2],flags:u32)->(i32,u32,usize) {
        cpu.r=[0;32];cpu.r[4]=ACTOR;cpu.r[28]=0x8007b318;cpu.r[29]=STACK;cpu.r[31]=STOP;
        for i in 16..=23 {cpu.r[i]=0x12340000+i as u32;}
        cpu.wr32(ACTOR+0x10,flags);cpu.wr16(ACTOR+0x14,point[0]);cpu.wr16(ACTOR+0x18,point[1]);
        cpu.pc=extended_floor::ENTRY;cpu.steps=0;cpu.run_until(&[STOP]);
        assert_eq!(cpu.r[29],STACK);for i in 16..=23 {assert_eq!(cpu.r[i],0x12340000+i as u32,"callee-saved register {i}");}
        assert_eq!([cpu.rd16(ACTOR+0x14),cpu.rd16(ACTOR+0x18)],point,"Native actor position mutated");
        (cpu.r[2] as i32,cpu.rd32(ACTOR+0x10),cpu.steps)
    }
    #[test]
    #[ignore="explicit protected retail CPU execution and optional create-only experimental IMG; no GUI"]
    fn retail_extended_staircase_cpu_and_experimental_img()->Result<()> {
        let path=PathBuf::from(std::env::var_os("LCK_RETAIL_SOURCE").context("LCK_RETAIL_SOURCE required")?);
        let original=std::fs::read(&path)?;
        ensure!(digest(&original)=="750dbcbadb70362feb3468467bbe7efbf3d151a414caa428c66c368d4b0b73e6","Unknown retail image");
        let library=crate::library::Library::open(&path)?;
        let disc=DiscPatcher::open(original.clone())?;
        let exe=disc.read_named_file("SCUS_942.54").context("No native SCUS")?;
        let overlay=library.index.entry_bytes(897)?;
        ensure!(digest(&overlay)=="0b39d892d783ffcc89dac06ee395540be0a7392c5cbc79fb9ba3442daccc09f6","Unknown native field overlay");
        let scene=legaia_engine_scene::scene::Scene::load(&library.index,"town01")?;
        let entry=scene.field_map_index(&library.index).context("No native MAP")? as usize;
        let map=disc.read_entry_footprint(entry)?;
        let load=disc.read_disc_sectors(disc.entry_disc_lba(entry).context("No MAP address")?,0x28)?;
        let lut=scene.field_floor_height_lut(&library.index)?.context("No native floor ladder")?;
        let expected=[37,18,2,0xcc];let delta=[-320,64];
        let plan=translate(&map,&exe,expected,delta,false,0)?;plan.verify_source(&map,&exe)?;
        let rows=extended_floor::decode(&plan.map[plan.table_offset..])?;
        ensure!(rows.len()==8,"Four moved plus four vacated native subcells required");
        let mut patched_exe=exe.clone();for e in &plan.code {patched_exe[e.file_off..e.file_off+e.bytes.len()].copy_from_slice(&e.bytes);}
        let mut patched_load=load.clone();patched_load[..0x12000].copy_from_slice(&plan.map);
        if let Some(image)=std::env::var_os("LCK_STAGE_E_IMAGE") {
            let image=std::path::PathBuf::from(image);let on_disc=DiscPatcher::open(std::fs::read(&image)?)?;
            let written_exe=on_disc.read_named_file("SCUS_942.54").context("Patched SCUS missing")?;
            let written_load=on_disc.read_disc_sectors(on_disc.entry_disc_lba(entry).context("Patched MAP missing")?,0x28)?;
            ensure!(written_exe==patched_exe && written_load==patched_load,"On-disk experimental engine/data differ from validated plan");
            patched_exe=written_exe;patched_load=written_load;
            let reopened=crate::library::Library::open(&image)?;
            let assembled=legaia_engine_world_views::scene_staging::assemble_field_scene(&reopened.index,"town01").map_err(anyhow::Error::msg)?;
            ensure!(!assembled.placements.is_empty(),"Written native scene cannot reassemble");
            println!("On-disk experimental IMG reparsed and scene reassembled; subsequent CPU queries execute its extracted SCUS/MAP bytes: {}",image.display());
        }
        let mut bilinear=load.clone();let go=0x8000+(18*128+37)*2;
        let word=u16::from_le_bytes(bilinear[go..go+2].try_into()?);bilinear[go..go+2].copy_from_slice(&(word&!0x1800).to_le_bytes());
        let mut count=0;let mut peak=0;
        for ladder_changed in [false,true] {
        let mut lut=lut;if ladder_changed {lut[1]=lut[1].checked_add(32).context("Test ladder overflow")?;}
        for delays in [false,true] {
            let mut retail=machine(&exe,&overlay,&load,lut,delays);
            let mut base=machine(&exe,&overlay,&bilinear,lut,delays);
            let mut engine=machine(&patched_exe,&overlay,&patched_load,lut,delays);
            // Every edge and several interior samples of each 64-unit cell,
            // including both sides of the formerly unrepresentable jump.
            for z in [18*128-1,18*128,18*128+1,18*128+31,18*128+63,18*128+64,18*128+65,18*128+96,19*128-1,19*128,19*128+1,19*128+63,19*128+64,20*128-1,20*128] {
                for x in [34*128-1,34*128,34*128+63,34*128+64,35*128-1,35*128,35*128+1,35*128+63,35*128+64,36*128,37*128-1,37*128,37*128+63,37*128+64,38*128-1,38*128] {
                    for flags in [0,0x800000,0x80000000,0x40800004] {
                        let point=[x as u16,z as u16];let sub=[(x/64) as u8,(z/64) as u8];
                        let row=rows.iter().find(|r|r.destination==sub);
                        let want=if let Some(r)=row {
                            if r.mode==1 {query(&mut retail,[(x+r.delta[0] as i32) as u16,(z+r.delta[1] as i32) as u16],flags)}else{query(&mut base,point,flags)}
                        }else{query(&mut retail,point,flags)};
                        let got=query(&mut engine,point,flags);
                        ensure!((got.0,got.1)==(want.0,want.1),"Patched floor mismatch at {point:?}, load delays {delays}: {got:?} vs {want:?}");
                        peak=peak.max(got.2);count+=1;
                    }
                }
            }
            for i in 0..patched_load.len(){ensure!(engine.rd8(MAP+i as u32)==patched_load[i],"Runtime MAP mutated by sampler");}
        }
        }
        println!("Extended sampler: {count} actual native-data queries passed with original/changed live ladder and with/without R3000 integer load delays; peak {peak} modeled instructions. No GUI/gameplay verification.");
        let calls:Vec<_>=exe[0x800..].chunks_exact(4).enumerate().filter(|(_,b)|u32::from_le_bytes((*b).try_into().unwrap())==legaia_code_hooks::mips::jal(extended_floor::ENTRY)).map(|(i,_)|0x80010000+i as u32*4).collect();
        println!("SCUS direct floor callers: {calls:x?}; code {} bytes, MAP extension {} bytes, no heap/new load sectors",plan.code[0].bytes.len(),8+rows.len()*12);
        for name in ["retona","vell"] {
            let native=legaia_engine_scene::scene::Scene::load(&library.index,name)?;
            let map_id=native.field_map_index(&library.index).context("Representative cell has no MAP")? as usize;
            let other=disc.read_disc_sectors(disc.entry_disc_lba(map_id).context("No other MAP address")?,0x28)?;
            let heights=native.field_floor_height_lut(&library.index)?.context("No representative floor ladder")?;
            let mut retail=machine(&exe,&overlay,&other,heights,true);
            let mut patched=machine(&patched_exe,&overlay,&other,heights,true);
            for i in 0..96 {let point=[((i*167+63)%16384) as u16,((i*293+31)%16384) as u16];
                let flags=if i%2==0 {0}else{0x80000000};
                let before=query(&mut retail,point,flags);let after=query(&mut patched,point,flags);
                ensure!((before.0,before.1)==(after.0,after.1),"Unmodified cell {name} floor behavior changed at {point:?}");
            }
            println!("Unmodified native cell {name}: 96 CPU floor/actor-flag comparisons passed with strict alignment and R3000 load delays");
        }
        if let Some(output)=std::env::var_os("LCK_STAGE_E_OUTPUT") {
            let output=PathBuf::from(output);
            let message=export(&path,&output,&Request{scene:"town01".into(),source_tile:[37,18],delta,duplicate:false,height_delta:0})?;
            let read=DiscPatcher::open(std::fs::read(&output)?)?;
            ensure!(read.read_named_file("SCUS_942.54").as_deref()==Some(patched_exe.as_slice()),"On-disk SCUS mismatch");
            ensure!(read.read_entry_footprint(entry)?==plan.map,"On-disk floor MAP mismatch");
            println!("{message}");
            let reopened=crate::library::Library::open(&output)?;
            let assembled=legaia_engine_world_views::scene_staging::assemble_field_scene(&reopened.index,"town01").map_err(anyhow::Error::msg)?;
            ensure!(!assembled.placements.is_empty(),"Experimental IMG scene failed native reassembly");
            println!("Experimental floor-only IMG: {}; unchanged visible geometry/walls/MAN; not the six-object export",output.display());
        }
        ensure!(digest(&std::fs::read(path)?)=="750dbcbadb70362feb3468467bbe7efbf3d151a414caa428c66c368d4b0b73e6","Retail source changed");Ok(())
    }
}
