//! Run captured requests through the original allocator, not an unlimited
//! synthetic allocator. A historical RAM fixture is NOT the current live heap.
use super::*;

fn heap_call(cpu:&mut Cpu,entry:u32,a0:u32,a1:u32)->Result<u32>{
    cpu.r[4]=a0;cpu.r[5]=a1;cpu.r[28]=0x8007b318;cpu.r[29]=0x801ff000;
    cpu.r[31]=0x8ffffffc;cpu.pc=entry;let start=cpu.steps;
    while cpu.pc!=0x8ffffffc {
        ensure!(cpu.steps-start<100000,"Allocator step budget at {:#x}",cpu.pc);
        if cpu.pc==0x800567a8{cpu.r[2]=0;cpu.pc=cpu.r[31];continue;}
        ensure!((0x80010000..0x8007b318).contains(&cpu.pc),"Allocator left original SCUS at {:#x}",cpu.pc);
        cpu.exec(cpu.rd32(cpu.pc));
    }Ok(cpu.r[2])
}
fn free_ring(cpu:&Cpu)->Result<(usize,usize)>{
    let desc=cpu.rd32(0x8007bb58);ensure!((0x80000000..0x80200000).contains(&desc),"Invalid captured heap descriptor");
    let top=cpu.rd32(desc);let sentinel=top+12;let mut p=cpu.rd32(top+16);let(mut total,mut largest,mut n)=(0,0,0);
    while p!=sentinel {ensure!((0x80000000..0x801ffff4).contains(&p)&&n<4096,"Corrupt captured free ring");let size=cpu.rd32(p+8)as usize;total+=size;largest=largest.max(size);p=cpu.rd32(p+4);n+=1;}
    Ok((total,largest))
}

#[test]
#[ignore="original-allocator forecast through observed failure with R7 resource sizes; no gameplay claim"]
fn replay_retired_resource_heap()->Result<()>{
    use crate::app::native_asset_plan::scene_registration as reg;
    let root=std::path::Path::new(r"D:\AstraRimElmProject\ManualTests\RuntimeRecovery_20261030_01");
    let output=std::path::PathBuf::from(std::env::var_os("LCK_R7_HEAP_REPORT").context("new report")?);ensure!(!output.exists(),"New report required");
    let old=crate::library::Library::open(std::path::Path::new(r"D:\AstraRimElmProject\ManualTests\Crash222_Matrix_20261029_02\E_FULL_USER_SCENE_RECOVERED_SOURCE.img"))?;
    let new=crate::library::Library::open(&root.join("R7_FULL_SCENE_DELETED_RESOURCE_RETIREMENT.img"))?;
    let entry=legaia_engine_scene::scene::Scene::load(&old.index,"town01")?.field_map_index(&old.index).context("MAP")?;let exe=old.executable()?;
    let mut plans=vec![];for lib in [&old,&new]{let b=lib.index.entry_bytes(entry+3)?;let p=lib.index.entry_bytes(entry+4)?;let(base,_)=reg::native_descriptor_table(&b)?;let(_,a)=execute_original_allocations(&exe,&b,base,Some(&p),true)?;plans.push(a.into_iter().filter(|e|e["action"]=="allocate").collect::<Vec<_>>());}
    ensure!(plans[0].len()==plans[1].len(),"Allocation structure changed");let changes=plans[0].iter().zip(&plans[1]).filter_map(|(a,b)|{assert_eq!(a["caller"],b["caller"]);(a["size"]!=b["size"]).then_some((a["caller"].as_u64().unwrap()+8,a["size"].as_u64().unwrap(),b["size"].as_u64().unwrap()))}).collect::<Vec<_>>();
    ensure!(changes.len()==1&&changes[0].1>changes[0].2,"Expected one smaller persistent resource allocation");
    let input:serde_json::Value=serde_json::from_slice(&std::fs::read(root.join("R6-user-freeze-analysis.json"))?)?;let log=std::fs::read_to_string(root.join("R6-user-freeze.log"))?;let start=log.lines().position(|l|l.contains("map name town01")).context("Observed scene")?+1;
    let mut cpu=Cpu::new();cpu.load_delays=true;cpu.strict_alignment=true;for(lo,hi)in [(0x80017888u32,0x800178f0),(0x80017b94,0x80017bec),(0x8002b3d4,0x8002b688)]{let at=0x800+(lo-0x80010000)as usize;cpu.load(lo,&exe[at..at+(hi-lo)as usize]);}cpu.r[6]=0x134800;heap_call(&mut cpu,0x8002b3d4,2,0x80091800)?;
    let mut pointers=std::collections::BTreeMap::new();let mut rows=vec![];let mut substitutions=0;
    for e in input["heap_events"].as_array().context("events")?.iter().filter(|e|e["line"].as_u64().unwrap_or(0)>=start as u64){let first=e["first_word"].as_u64().context("word")?;let caller=e["caller_return_address"].as_u64().context("caller")?;
        if e["action"]=="HEAP"{let mut size=first;for(c,a,b)in &changes{if caller==*c&&size==*a{size=*b;substitutions+=1;}}
            let actual=heap_call(&mut cpu,0x80017888,0,size as u32)?;ensure!(actual!=0,"Candidate still fails allocation {size} at {caller:#x}");let observed=e["second_word"].as_u64().context("pointer")?;
            if observed!=0{pointers.insert(observed,actual);}rows.push(json!({"line":e["line"],"caller_ra":caller,"original_bytes":first,"candidate_bytes":size,"returned":actual,"original_failed":observed==0,"free_after":free_ring(&cpu)?.0}));
        }else{let actual=pointers.remove(&first).context("Free without observed allocation")?;heap_call(&mut cpu,0x80017b94,actual,0)?;}
    }
    ensure!(substitutions==1,"Native size change did not map uniquely to live request");let free=free_ring(&cpu)?;
    std::fs::write(output,serde_json::to_vec_pretty(&json!({"schema":1,"candidate":"R7","original_live_sequence_reproduced_separately":true,"native_request_size_changes":changes,"all_allocations_through_observed_failure_succeed":true,"remaining_free":free.0,"largest_free":free.1,"events":rows,"native_gameplay_verified":false,"limitation":"Forecast uses observed R6 control flow through failure; allocations or behavior after that point require manual gameplay verification"}))?)?;Ok(())
}

#[test]
#[ignore="replay exact user R6 allocation results through original heap; no emulator"]
fn replay_user_r6_heap_failure()->Result<()>{
    let root=std::path::Path::new(r"D:\AstraRimElmProject\ManualTests\RuntimeRecovery_20261030_01");
    let output=std::path::PathBuf::from(std::env::var_os("LCK_R6_REPLAY_REPORT").context("new report")?);ensure!(!output.exists(),"New evidence path required");
    let log=std::fs::read_to_string(root.join("R6-user-freeze.log"))?;
    let start=log.lines().position(|l|l.contains("map name town01")).context("Observed town01 transition")?+1;
    let input:serde_json::Value=serde_json::from_slice(&std::fs::read(root.join("R6-user-freeze-analysis.json"))?)?;
    let exe=std::fs::read(r"D:\AstraRimElmProject\assets\SCUS_942.54")?;let mut cpu=Cpu::new();cpu.load_delays=true;cpu.strict_alignment=true;
    for(lo,hi)in [(0x80017888u32,0x800178f0),(0x80017b94,0x80017bec),(0x8002b3d4,0x8002b688)]{let at=0x800+(lo-0x80010000)as usize;cpu.load(lo,&exe[at..at+(hi-lo)as usize]);}
    cpu.r[6]=0x134800;heap_call(&mut cpu,0x8002b3d4,2,0x80091800)?;
    let mut rows=vec![];let mut failures=0;
    for e in input["heap_events"].as_array().context("heap events")?.iter().filter(|e|e["line"].as_u64().unwrap_or(0)>=start as u64){
        let before=free_ring(&cpu)?;let first=e["first_word"].as_u64().context("first word")?as u32;
        if e["action"]=="HEAP"{
            let actual=heap_call(&mut cpu,0x80017888,0,first)?;let observed=e["second_word"].as_u64().context("observed result")?as u32;
            ensure!(actual==observed,"Original allocator replay differs at log line {}: actual {actual:#x}, observed {observed:#x}",e["line"]);
            if actual==0{failures+=1;ensure!(first==8368&&e["caller_return_address"].as_u64()==Some(0x8001f71c),"Different native failure");}
            rows.push(json!({"line":e["line"],"action":"allocate","bytes":first,"returned":actual,"caller_ra":e["caller_return_address"],"free_before":before.0,"largest_before":before.1,"free_after":free_ring(&cpu)?.0,"largest_after":free_ring(&cpu)?.1}));
        }else{heap_call(&mut cpu,0x80017b94,first,0)?;rows.push(json!({"line":e["line"],"action":"free","pointer":first,"free_after":free_ring(&cpu)?.0,"largest_after":free_ring(&cpu)?.1}));}
    }
    ensure!(failures==1,"Expected exact observed failure");
    let lib=crate::library::Library::open(std::path::Path::new(r"D:\AstraRimElmProject\ManualTests\Crash222_Matrix_20261029_02\E_FULL_USER_SCENE_RECOVERED_SOURCE.img"))?;
    let entry=legaia_engine_scene::scene::Scene::load(&lib.index,"town01")?.field_map_index(&lib.index).context("MAP")?;
    let b=lib.index.entry_bytes(entry+3)?;let p=lib.index.entry_bytes(entry+4)?;let models=crate::app::native_asset_plan::scene_registration::native_model_pool(&b,Some(&p));
    let mut audit=vec![];
    for(i,m)in models.iter().enumerate(){let t=legaia_tmd::parse(&m.bytes)?;let used=12+t.objects.len()*28+t.stats().total_bytes_consumed;
        let normal_unique=t.objects.iter().map(|o|o.normals.iter().map(|v|(v.x,v.y,v.z,v._pad)).collect::<std::collections::BTreeSet<_>>().len()).sum::<usize>();
        audit.push(json!({"selector":i,"stored":m.bytes.len(),"structural_sections_sum":used,"normals":t.stats().total_normals,"unique_normals_per_group":normal_unique,"sha256":digest(&m.bytes)}));}
    std::fs::write(output,serde_json::to_vec_pretty(&json!({"schema":1,"evidence":"original allocator instructions reproduce EVERY logged allocation address and the observed failure","all_observed_addresses_match":true,"heap_bytes":0x134800,"events":rows,"models":audit,"native_gameplay_fixed":false}))?)?;Ok(())
}

#[test]
#[ignore="historical user RAM fixture and exact IMGs; original allocator only, no live emulator"]
fn original_heap_recovery_matrix()->Result<()>{
    let output=std::path::PathBuf::from(std::env::var_os("LCK_HEAP_RECOVERY_REPORT").context("new report path")?);
    ensure!(!output.exists(),"Refuse to replace prior evidence");
    let root=r"D:\AstraRimElmProject\ManualTests\Crash222_Matrix_20261029_02";
    let cases=vec![("retail",r"D:\Emulators\Games\Legend of Legaia\LegendOfLegaia.img".to_string()),("parent",r"D:\AstraRimElmProject\ManualTests\Native_Format_Adapters_20261028_01\LCK_EXPERIMENTAL_MIXED_FORMATS_FIXED.img".into()),("222",r"D:\AstraRimElmProject\222.img".into()),("A",format!(r"{root}\A_222_EXACT_MAP_SMALL_STAGING.img")),("B",format!(r"{root}\B_NATIVE_EDITS_ONLY.img")),("C",format!(r"{root}\C_FOREIGN_IMPORTS_ONLY.img")),("D",format!(r"{root}\D_ALL_GEOMETRY_NO_COLLISION.img")),("E",format!(r"{root}\E_FULL_USER_SCENE_RECOVERED_SOURCE.img"))];
    let mut rows=vec![];
    for(name,path)in cases {
        let lib=crate::library::Library::open(std::path::Path::new(&path))?;let exe=lib.executable()?;let scene=legaia_engine_scene::scene::Scene::load(&lib.index,"town01")?;let entry=scene.field_map_index(&lib.index).context("MAP")?;
        let b=lib.index.entry_bytes(entry+3)?;let p=lib.index.entry_bytes(entry+4)?;let(base,_)=crate::app::native_asset_plan::scene_registration::native_descriptor_table(&b)?;
        let(_,requests)=execute_original_allocations(&exe,&b,base,Some(&p),true)?;
        for capture in ["town01_arena_return_ram.bin","town01_preload_ram.bin"] {
            let ram=std::fs::read(format!(r"D:\AstraRimElmProject\Backups\LCK_Run4_CaptureFixtures_20261027\{capture}"))?;
            ensure!(ram.len()==0x200000,"Raw RAM fixture size");let mut cpu=Cpu::new();cpu.load_delays=true;cpu.strict_alignment=true;cpu.load(0x80000000,&ram);
            // Allocator code must match the source IMG profile; only the heap
            // state comes from the historical snapshot. Never run its other code.
            let load=word(&exe,0x18)?;
            // Do not reload initialized SCUS globals over captured heap state.
            for(lo,hi)in [(0x80017888u32,0x800178f0),(0x80017b94,0x80017bec),(0x8002b468,0x8002b688)]{let at=0x800+(lo-load)as usize;cpu.load(lo,&exe[at..at+(hi-lo)as usize]);}
            let before=free_ring(&cpu)?;let mut addresses=std::collections::BTreeMap::new();let mut ledger=vec![];let mut failed=false;let mut min_largest=before.1;
            for r in &requests {
                if r["action"]=="allocate" {let size=r["size"].as_u64().context("request size")? as u32;let p=heap_call(&mut cpu,0x80017888,0,size)?;let remaining=free_ring(&cpu)?;min_largest=min_largest.min(remaining.1);
                    ledger.push(json!({"action":"allocate","bytes":size,"returned":p,"native_caller":r["caller"],"largest_free_after":remaining.1}));
                    if p==0{failed=true;break;}ensure!(p>=0x80091800&&p.checked_add(size).is_some_and(|e|e<=0x801c6000),"Native allocator escaped 2 MiB heap arena");addresses.insert(r["address"].as_u64().unwrap(),p);
                }else{let key=r["address"].as_u64().context("free address")?;let p=*addresses.get(&key).context("Free without preceding allocation")?;heap_call(&mut cpu,0x80017b94,p,0)?;ledger.push(json!({"action":"free","address":p}));}
            }
            println!("ORIGINAL HEAP {name}/{capture}: failed {failed}, initial largest {}, minimum largest {min_largest}",before.1);
            rows.push(json!({"case":name,"image":path,"capture":capture,"capture_sha256":digest(&ram),"initial_free":before.0,"initial_largest":before.1,"allocation_failed":failed,"minimum_largest":min_largest,"ledger":ledger,"current_gameplay_verified":false,"scope":"Original allocator on historical heap state, replaying dispatcher request order only; excludes intervening engine allocations and actor/render allocations"}));
        }
    }
    std::fs::write(output,serde_json::to_vec_pretty(&rows)?)?;Ok(())
}

#[test]
fn independent_trace_single_tim_status_matches_original_zero_return(){
    let mut t=Trace{models:vec![],dispatch:vec![],stream_mode:0,gaps:vec![],evidence_class:"test"};
    assert_eq!(dispatch(&mut t,0,&[],"single".into()).unwrap(),0);
    // Native 8001F150 sets s4 bit 0 for the list, even an empty one.
    assert_eq!(dispatch(&mut t,1,&[],"list".into()).unwrap(),1);
}

#[test]
#[ignore="original SCUS LZS instructions with output canaries; no emulator"]
fn original_decoder_recovery_bank()->Result<()>{
    let path=std::env::var_os("LCK_DECODER_IMG").context("IMG")?;let lib=crate::library::Library::open(std::path::Path::new(&path))?;let exe=lib.executable()?;
    let scene=legaia_engine_scene::scene::Scene::load(&lib.index,"town01")?;let entry=scene.field_map_index(&lib.index).context("MAP")?;
    let b=lib.index.entry_bytes(entry+3)?;let(base,header)=crate::app::native_asset_plan::scene_registration::native_descriptor_table(&b)?;let load=word(&exe,0x18)?;let mut rows=vec![];
    for (i,d) in header.used().iter().enumerate(){if matches!(d.type_byte,10|15|20){continue;}let src=&b[base+d.data_offset as usize..];let expected=legaia_lzs::decompress(src,d.size as usize)?;
        let mut cpu=Cpu::new();cpu.load_delays=true;cpu.strict_alignment=true;let at=0x800+(0x8001a55c-load)as usize;cpu.load(0x8001a55c,&exe[at..at+328]);cpu.load(0x80400000,src);cpu.load(0x81000000,&vec![0xa5;expected.len()+32]);
        cpu.r[4]=d.size;cpu.r[5]=0x80400000;cpu.r[6]=0x81000000;cpu.r[29]=0x807ff000;cpu.r[31]=0x8ffffffc;cpu.pc=0x8001a55c;
        while cpu.pc!=0x8ffffffc {ensure!(cpu.steps<40_000_000,"Native LZS budget");match cpu.pc{
            0x80017888=>{ensure!(cpu.r[5]==0x1014,"Unexpected original decoder scratch allocation");cpu.r[2]=0x82000000;cpu.pc=cpu.r[31];},
            0x80017b94=>{cpu.r[2]=0;cpu.pc=cpu.r[31];},
            p if (0x8001a55c..0x8001a6a4).contains(&p)=>cpu.exec(cpu.rd32(p)),
            p=>anyhow::bail!("Unsubstituted original decoder call {p:#x}"),
        }}
        ensure!((0..expected.len()).all(|j|cpu.rd8(0x81000000+j as u32)==expected[j]),"Original decoder differs at descriptor {i}");
        let overrun=(0..32).take_while(|j|cpu.rd8(0x81000000+expected.len()as u32+*j)!=0xa5).count();
        println!("ORIGINAL DECODE {i} type {} size {} overrun {overrun}",d.type_byte,d.size);ensure!(overrun==0,"Original decoder overruns descriptor {i} by {overrun} bytes");
        rows.push(json!({"descriptor":i,"type":d.type_byte,"bytes":d.size,"original_decoder_matches":true,"overrun":overrun,"scratch_bytes":4116}));
    }
    if let Some(path)=std::env::var_os("LCK_DECODER_REPORT"){let file=std::fs::OpenOptions::new().write(true).create_new(true).open(path)?;serde_json::to_writer_pretty(file,&rows)?;}Ok(())
}
