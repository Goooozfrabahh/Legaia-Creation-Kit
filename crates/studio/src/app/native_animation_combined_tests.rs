// Diagnostic fixtures may name the user's cases; production writer has no donor exceptions.
fn combined_decode(bundle:&[u8],kind:u8)->Result<Vec<u8>> {
    let(base,h)=reg::native_descriptor_table(bundle)?;
    let d=h.used().iter().find(|d|d.type_byte==kind).context("Missing descriptor")?;
    legaia_lzs::decompress(&bundle[base+d.data_offset as usize..],d.size as usize)
}
fn combined_pack(items:&[Vec<u8>])->Vec<u8> {
    let mut out=vec![0;4+4*items.len()];out[..4].copy_from_slice(&(items.len()as u32).to_le_bytes());
    for(i,item)in items.iter().enumerate(){let off=(out.len()/4)as u32;out[4+4*i..8+4*i].copy_from_slice(&off.to_le_bytes());out.extend(item);while out.len()%4!=0{out.push(0);}}
    out
}
/// Same CPU carries original descriptor/PAC registration, pointer fixups, real
/// free-stack/list allocation, materialization, ANM binding and complete startup
/// through installer return. Heap/LZS/GPU/GTE/RNG services remain explicit stubs.
fn combined_construct(exe:&[u8],overlay:&[u8],bundle:&[u8],pac:&[u8],first_p1:usize)->Result<serde_json::Value> {
    let(base,_)=reg::native_descriptor_table(bundle)?;
    let(models,allocations,mut cpu)=super::super::native_loader_trace::tests::execute_original_machine(exe,bundle,base,Some(pac),true)?;
    let man=combined_decode(bundle,3)?;let mf=man_section::parse(&man).map_err(anyhow::Error::msg)?;
    let bank=player_anm::parse(&combined_decode(bundle,5)?)?;
    let load=u32::from_le_bytes(exe[0x18..0x1c].try_into()?);
    cpu.load(load,&exe[0x800..0x800+(0x8007b318-load)as usize]);cpu.load(0x801ce818,overlay);
    cpu.wr16(0x8007b6f8,5);
    let man_address=cpu.rd32(0x8007b898);let bank_address=cpu.rd32(0x8007b888);
    ensure!((0..man.len()).all(|i|cpu.rd8(man_address+i as u32)==man[i]),"Native MAN global not retained");
    let pointer_table=(0..models.len()+5).map(|i|cpu.rd32(0x8007c018+i as u32*4)).collect::<Vec<_>>();
    // Actual original full-pool setup, then the seven original list sentinels.
    for entry in std::iter::once(0x800203ec).chain(std::iter::repeat_n(0x80020424,7)) {
        cpu.r[29]=0x807ff000;cpu.r[31]=0x8ffffffc;cpu.pc=entry;let start=cpu.steps;
        while cpu.pc!=0x8ffffffc {ensure!(cpu.steps-start<10000,"Native pool setup budget");let w=cpu.rd32(cpu.pc);cpu.exec(w);}
        if entry==0x80020424 {let i=(142-cpu.rd32(0x8007c348))as usize-1;let offsets=[4,8,12,16,24,20,36];cpu.wr32(0x8007c348+offsets[i],cpu.r[2]);}
    }
    let pool_before=cpu.rd32(0x8007c348);
    let mut heap=allocations.iter().filter_map(|a|Some(a["address"].as_u64()?+a["size"].as_u64()?)).max().unwrap_or(0x81000000)as u32;
    heap=(heap+31)&!31;
    let mut actors=vec![];let mut snapshots=vec![];let mut ledger=vec![];
    for p in mf.actor_placements(&man).into_iter().filter(|a|a.index>=first_p1) {
        cpu.r[4]=p.index as u32;cpu.r[5]=mf.partitions[0].len()as u32;cpu.r[29]=0x807ff000;cpu.r[28]=0x8007b318;cpu.r[31]=0x8ffffffc;cpu.pc=0x8003a1e4;
        let start=cpu.steps;let mut actor=0;
        while cpu.pc!=0x8ffffffc {
            ensure!(cpu.steps-start<100000,"Combined installer stuck at {:#x}",cpu.pc);
            match cpu.pc {
                0x80017888=>{let size=cpu.r[5];ensure!(size<0x10000,"Unexpected construction extent");ledger.push(serde_json::json!({"p1":p.index,"bytes":size,"pointer":heap}));cpu.r[2]=heap;heap+=(size+31)&!31;cpu.pc=cpu.r[31];},
                0x8003d344=>{for i in 0..3{cpu.wr32(cpu.r[5]+i*4,0);}cpu.pc=cpu.r[31];},
                0x80056798=>{cpu.r[2]=0;cpu.pc=cpu.r[31];},
                0x8003a474=>{actor=cpu.r[16];let w=cpu.rd32(cpu.pc);cpu.exec(w);},
                pc=>{ensure!((load..0x8007b318).contains(&pc)||(0x801ce818..0x801ce818+overlay.len()as u32).contains(&pc),"Unmodelled combined helper {pc:#x}");let w=cpu.rd32(pc);cpu.exec(w);},
            }
        }
        ensure!(actor!=0&&cpu.rd16(actor+0x50)as usize==mf.partitions[0].len()+p.index,"Independent native actor identity differs");
        let global=p.model_index as usize+5;ensure!(cpu.rd16(actor+0x64)as usize==global,"Model selector differs");
        let table=cpu.rd32(actor+0x44);let tmd=cpu.rd32(0x8007c018+global as u32*4);
        ensure!(cpu.rd32(table)==cpu.rd32(tmd+8),"Object count differs");
        for i in 0..cpu.rd32(table){ensure!(cpu.rd32(table+4+i*4)==tmd+12+i*28,"Object pointer not native registered TMD");}
        ensure!(cpu.rd32(actor+0x4c)==bank_address+bank.record_offsets[p.anim_id as usize-1],"Native clip pointer differs");
        ensure!(cpu.rd16(actor+0x5e)==p.anim_id as u16&&cpu.rd16(actor+0x68)==0,"Clip binding/initial phase differs");
        // The native linked-list next/previous fields legitimately change.
        for (old,bytes) in &snapshots {ensure!((0x10..0xc0).map(|i|cpu.rd8(old+i)).collect::<Vec<_>>()==*bytes,"Later construction mutated earlier actor state");}
        snapshots.push((actor,(0x10..0xc0).map(|i|cpu.rd8(actor+i)).collect::<Vec<_>>()));
        actors.push(serde_json::json!({"p1":p.index,"pointer":actor,"global_model":global,"tmd_pointer":tmd,"object_table":table,"clip":p.anim_id,"clip_pointer":cpu.rd32(actor+0x4c),"context":cpu.rd16(actor+0x50),"final_pc":cpu.rd16(actor+0x9e)}));
    }
    ensure!(pointer_table==(0..models.len()+5).map(|i|cpu.rd32(0x8007c018+i as u32*4)).collect::<Vec<_>>(),"Construction overwrote model registration pointers");
    ensure!(pool_before-cpu.rd32(0x8007c348)==actors.len()as u32,"Native pool accounting differs");
    Ok(serde_json::json!({"models":models,"resource_allocations":allocations,"construction_allocations":ledger,"actors":actors,"pool_before":pool_before,"pool_after":cpu.rd32(0x8007c348),"man_address":man_address,"bank_address":bank_address,"model_table_preserved":true,"earlier_actor_state_preserved":true,"scope":"Original cumulative loader and complete imported-actor installers on same CPU with real free-stack/list allocation; synthetic heap/LZS/GPU/GTE/RNG, not full scene/gameplay"}))
}

#[test]
#[ignore="combined native registration/construction and bounded diagnostic IMG matrix; no game launch"]
fn stage_b2b_combined_interaction_matrix()->Result<()> {
    let original=std::path::PathBuf::from(std::env::var_os("LCK_COMBINED_INPUT").context("A-E directory")?);
    let output=std::path::PathBuf::from(std::env::var_os("LCK_COMBINED_OUTPUT").context("new directory")?);
    ensure!(!output.exists(),"New matrix directory required");std::fs::create_dir_all(&output)?;
    let report:serde_json::Value=serde_json::from_slice(&std::fs::read(original.join("ISOLATION_REPORT.json"))?)?;
    let mut cases=vec![];let mut inputs=vec![];
    for index in [1,2,4] {let path=report["cases"][index]["path"].as_str().context("IMG path")?;let lib=crate::library::Library::open(std::path::Path::new(path))?;let exe=lib.executable()?;let bundle=lib.index.entry_bytes(4)?;let pac=lib.index.entry_bytes(5)?;
        ensure!(digest(&std::fs::read(path)?)==report["cases"][index]["sha256"].as_str().unwrap(),"Original A-E changed");
        cases.push(serde_json::json!({"case":report["cases"][index]["case"],"path":path,"native":combined_construct(&exe,&lib.index.entry_bytes(897)?,&bundle,&pac,53)?}));inputs.push((path.to_owned(),bundle,pac,exe,lib.index.entry_bytes(897)?));}
    let (epath,eb,ep,exe,overlay)=&inputs[2];let em=combined_decode(eb,3)?;let bank=combined_decode(eb,5)?;
    let mut variants=vec![];
    let mut reversed=em.clone();let mf=man_section::parse(&em).map_err(anyhow::Error::msg)?;
    let mut rawmodels=legaia_container_formats::pack::extract_pack(&combined_decode(eb,2)?)?.into_iter().map(<[u8]>::to_vec).collect::<Vec<_>>();
    ensure!(rawmodels.len()==116,"Expected E resource cohort");rawmodels.swap(114,115);
    for p in mf.actor_placements(&em).into_iter().filter(|p|p.index>=53){reversed[p.record_offset+1]=if p.model_index==114{115}else{114};}
    variants.push(("F_REVERSE_MODELS","Reverse only the two imported model registrations and matching actor selectors; ANM order unchanged",BTreeMap::from([(2,combined_pack(&rawmodels)),(3,reversed)])));
    let bm=combined_decode(&inputs[0].1,3)?;
    variants.push(("G_BOTH_RESOURCES_BALDEN_ONLY","E model/ANM/TIM resources, only Balden instantiated",BTreeMap::from([(3,bm)])));
    let mut cm=combined_decode(&inputs[1].1,3)?;let c=man_section::parse(&cm).map_err(anyhow::Error::msg)?.actor_placement(&cm,53).context("C actor")?;
    cm[c.record_offset+1]=115;cm[c.record_offset+2]=71;cm[c.record_offset+7]=71;
    variants.push(("H_BOTH_RESOURCES_BUBU_ONLY","E resources, only Bubu1 instantiated at global model slot 120 / ANM 71",BTreeMap::from([(3,cm)])));
    let mut twins=em.clone();let p=mf.actor_placement(&em,54).context("Second actor")?;twins[p.record_offset+1]=114;twins[p.record_offset+2]=70;twins[p.record_offset+7]=70;
    variants.push(("I_TWO_BALDEN_E_RESOURCES","E resources and two live actors, both deliberately bound to Balden model/clip; diagnostic appearance substitution only",BTreeMap::from([(3,twins)])));
    let retail=crate::library::Library::open(std::path::Path::new(&std::env::var_os("LCK_AUDIT_SOURCE").context("retail")?))?;
    variants.push(("M_BOTH_RESOURCES_NO_IMPORT_ACTORS","E resources with retail MAN; distinguishes resource loading from imported actor construction",BTreeMap::from([(3,combined_decode(&retail.index.entry_bytes(4)?,3)?)])));
    for(name,hypothesis,replacements)in variants {
        let mut disc=legaia_disc_patch::disc::DiscPatcher::open(std::fs::read(epath)?)?;
        let b=rewrite_bundle(eb,&replacements,404480)?;let mut padded=b.clone();padded.resize(b.len().div_ceil(2048)*2048,0);
        if padded.len()>eb.len(){disc.grow_prot_entries(&BTreeMap::from([(4,padded)]))?;}else{padded.resize(eb.len(),0);disc.patch_prot_entry(4,0,&padded)?;}
        let finalb=disc.read_entry_footprint(4)?;let finalp=disc.read_entry_footprint(5)?;
        ensure!(finalp.as_slice()==ep.as_slice(),"Diagnostic changed ordered TIM/PAC resources");
        ensure!(disc.read_entry_footprint(1)?.as_slice()==retail.index.entry_bytes(1)?.as_slice(),"Diagnostic changed MAP/collision");
        ensure!(combined_decode(&finalb,5)?==bank,"Diagnostic changed ANM bytes");
        let native=combined_construct(exe,overlay,&finalb,&finalp,53)?;
        if name=="H_BOTH_RESOURCES_BUBU_ONLY" {ensure!(native["actors"][0]["global_model"]==120&&native["actors"][0]["clip"]==71,"H must construct Bubu from E's second model/clip, not C's original selector");}
        let path=output.join(format!("LCK_EXPERIMENTAL_{name}.img"));std::fs::write(&path,disc.image())?;
        cases.push(serde_json::json!({"case":name,"hypothesis":hypothesis,"path":path,"sha256":digest(disc.image()),"native":native,"gameplay_verified":false}));
    }
    std::fs::write(output.join("COMBINED_REPORT.json"),serde_json::to_vec_pretty(&serde_json::json!({"parent":"e5eefff0434d1856c443788c76224444ea9504f3","user_results":{"A":"LOADS","B":"LOADS","C":"LOADS","D":"FREEZES","E":"FREEZES"},"writer_fix_applied":false,"cases":cases}))?)?;
    Ok(())
}
