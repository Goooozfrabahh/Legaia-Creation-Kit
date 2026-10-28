//! Same-width native context reference relocation. Does not allocate actors,
//! bind MAP triggers or establish script/physics ownership. No IMG permission.
use std::collections::{BTreeMap,BTreeSet};
use anyhow::{Result,ensure,Context};
use legaia_scene_formats::field_disasm::{self,InsnInfo,LinearWalker,MenuCtrlKind};
use super::{collision_recipes::digest,native_script_dependencies::SceneInventory};

#[derive(Clone,Debug)]
pub(super) struct Reference {pub record:usize,pub offset:usize,pub target:u8,pub spawn:bool}
pub(super) struct Relocation {source_sha256:String,output:Vec<u8>,pub changed:Vec<Reference>}
impl Relocation {
    pub fn apply(&self,source:&[u8])->Result<Vec<u8>> {
        ensure!(digest(source)==self.source_sha256,"MAN reference plan source changed");Ok(self.output.clone())
    }
}
fn shared(id:u8)->bool {matches!(id,0xf8|0xfb)}
/// Resolve the static context reference domain only. Dynamic helpers, actor
/// bindings, movement streams and scene transitions remain separate contracts.
pub(super) fn references(man:&[u8])->Result<Vec<Reference>> {
    let inventory=SceneInventory::read(man).map_err(anyhow::Error::msg)?;
    ensure!(inventory.gaps.is_empty(),"MAN decoding incomplete; native references cannot be safely rewritten");
    let mut found=vec![];
    for record in &inventory.records {
        let body=&man[record.start..record.start+record.len];
        for decoded in LinearWalker::new(body,record.pc0) {
            let insn=decoded.map_err(|e|anyhow::anyhow!("Native instruction decode: {e:?}"))?;
            if let Some(target)=insn.extended {
                found.push(Reference{record:record.flat,offset:record.start+insn.pc+1,target,spawn:false});
            }
            if let InsnInfo::SpawnRecord{global_index}=insn.info {
                found.push(Reference{record:record.flat,offset:record.start+insn.pc+if insn.extended.is_some(){2}else{1},target:global_index,spawn:true});
            }
        }
    }Ok(found)
}
pub(super) fn compile(man:&[u8],remap:&BTreeMap<u8,u8>)->Result<Relocation> {
    ensure!(!remap.is_empty(),"No native reference remap requested");
    let inventory=SceneInventory::read(man).map_err(anyhow::Error::msg)?;
    ensure!(inventory.gaps.is_empty(),"MAN decode gaps prevent reference relocation");
    let total:usize=inventory.partition_counts.iter().map(|&n|n.max(0) as usize).sum();
    let mut destinations=BTreeSet::new();
    for (&from,&to) in remap {
        ensure!(!shared(from)&&!shared(to),"Native player/system contexts cannot become independent object scripts");
        ensure!((from as usize)<total&&(to as usize)<total&&from!=to,"Context remap must reference independently allocated existing MAN identities");
        ensure!(destinations.insert(to),"Multiple original contexts would collapse into one native identity");
    }
    let mut output=man.to_vec();let mut changed=vec![];let refs=references(man)?;
    for reference in refs {
        let Some(&to)=remap.get(&reference.target) else {continue};
        let record=inventory.records.iter().find(|r|r.flat==reference.record).context("MAN source record missing")?;
        if reference.spawn {
            let target=inventory.records.iter().find(|r|r.flat==to as usize).context("Spawn target missing")?;
            ensure!(target.partition==2,"SPAWN_RECORD target must remain a partition-2 script");
        } else {
            let body=&man[record.start..record.start+record.len];
            let instruction=field_disasm::decode(body,reference.offset-record.start-1).map_err(|e|anyhow::anyhow!("Reference boundary: {e:?}"))?;
            // These typed dispatchers consume the context prefix separately.
            // System-flag extended forms alias that byte as flag data; opaque
            // helper families need their own native operand contract.
            ensure!(matches!(instruction.info,InsnInfo::Nop|InsnInfo::LFlag{..}|InsnInfo::CFlag{..}|InsnInfo::MoveTo{..}|InsnInfo::ExecMove{..}
                |InsnInfo::MenuCtrl{kind:MenuCtrlKind::HighNibble{outer:10,sub:0|1,..},..}
                |InsnInfo::MenuCtrl{kind:MenuCtrlKind::Nibble8{sub:12,..},..}
                |InsnInfo::MenuCtrl{kind:MenuCtrlKind::Nibble4{sub:1,..},..}),
                "Context reference at MAN {:#x} has unclosed helper/operand alias semantics ({:?}); no reference bytes changed",reference.offset,instruction.info);
        }
        output[reference.offset]=to;changed.push(reference);
    }
    ensure!(!changed.is_empty(),"Requested context remap had no supported native references");
    let read=SceneInventory::read(&output).map_err(anyhow::Error::msg)?;
    ensure!(read.gaps.is_empty()&&read.partition_counts==inventory.partition_counts,"Reference rewrite changed MAN structure");
    let allowed=changed.iter().map(|r|r.offset).collect::<BTreeSet<_>>();
    for (i,(&a,&b)) in man.iter().zip(&output).enumerate() {ensure!(a==b||allowed.contains(&i),"Unrelated MAN byte changed");}
    let reparsed=references(&output)?;
    for edit in &changed {
        ensure!(reparsed.iter().any(|r|r.offset==edit.offset&&r.target==remap[&edit.target]&&r.spawn==edit.spawn),"Native reference failed readback");
    }
    Ok(Relocation{source_sha256:digest(man),output,changed})
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::native_script_dependencies::fixture_man;
    #[test]
    fn context_references_relocate_without_changing_branches_positions_or_other_records()->Result<()> {
        let man=fixture_man(&[&[0xa3,0,12,24,0xa1,0,0x21],&[0x23,12,24,0x21]]);
        let plan=compile(&man,&BTreeMap::from([(0,1)]))?;let output=plan.apply(&man)?;
        assert_eq!(plan.changed.len(),2);
        let old=SceneInventory::read(&man).unwrap();let second=&old.records[1];
        assert_eq!(&output[second.start..second.start+second.len],&man[second.start..second.start+second.len]);
        assert_eq!(references(&output)?.iter().map(|r|r.target).collect::<Vec<_>>(),vec![1,1]);
        let mut stale=man;stale[2]^=1;assert!(plan.apply(&stale).is_err());Ok(())
    }
    #[test]
    fn remap_never_reinterprets_shared_contexts_flag_aliases_or_undecodable_scripts() {
        let man=fixture_man(&[&[0xa1,0,0x21],&[0x21]]);
        assert!(compile(&man,&BTreeMap::from([(0,0xf8)])).is_err());
        let flags=fixture_man(&[&[0xd0,0,0,0x21],&[0x21]]);
        assert!(SceneInventory::read(&flags).unwrap().gaps.is_empty());
        assert!(compile(&flags,&BTreeMap::from([(0,1)])).err().unwrap().to_string().contains("alias semantics"));
        let gap=fixture_man(&[&[0x4c,0x73],&[0x21]]);assert!(compile(&gap,&BTreeMap::from([(0,1)])).is_err());
    }
    #[test]
    fn native_context_guards_and_actor_ramps_keep_branch_and_value_operands()->Result<()> {
        // Native 4C A0 flag guard, 4C 8C field-68 guard and nibble-4 sub-1.
        let script=[0xcc,0,0xa0,0,5,0,0xcc,0,0x8c,0,0,0xcc,0,0x41,16,0,0,0,0x21];
        let man=fixture_man(&[&script,&[0x21]]);
        let plan=compile(&man,&BTreeMap::from([(0,1)]))?;assert_eq!(plan.changed.len(),3);
        let output=plan.apply(&man)?;let old=SceneInventory::read(&man).unwrap();let new=SceneInventory::read(&output).unwrap();
        let a=&old.records[0];let b=&new.records[0];
        let original=LinearWalker::new(&man[a.start..a.start+a.len],a.pc0).map(|i|format!("{:?}",i.unwrap().info)).collect::<Vec<_>>();
        let rewritten=LinearWalker::new(&output[b.start..b.start+b.len],b.pc0).map(|i|format!("{:?}",i.unwrap().info)).collect::<Vec<_>>();
        assert_eq!(original,rewritten);Ok(())
    }
    #[test]
    #[ignore="explicit native house binding/spatial/transition study; no IMG writes or GUI"]
    fn original_house_references_and_transitions()->Result<()> {
        let path=std::path::PathBuf::from(std::env::var_os("LCK_M2_SOURCE").context("Six-request native source required")?);
        let library=crate::library::Library::open(&path)?;
        let scene=legaia_engine_scene::scene::Scene::load(&library.index,"town01")?;
        let man=scene.field_man_payload(&library.index)?.context("No MAN")?;
        let inventory=SceneInventory::read(&man).map_err(anyhow::Error::msg)?;
        let mut relevant=BTreeSet::from([0usize]);
        for reference in references(&man)?.iter().filter(|r|r.target==0&&!r.spawn) {relevant.insert(reference.record);}
        for record in inventory.records.iter().filter(|r|relevant.contains(&r.flat)) {
            println!("Bound/incoming MAN {} P{}[{}] bytes {:#x}..{:#x}",record.flat,record.partition,record.record,record.start,record.start+record.len);
            for decoded in LinearWalker::new(&man[record.start..record.start+record.len],record.pc0) {
                let insn=decoded.map_err(|e|anyhow::anyhow!("Decode {e:?}"))?;
                if insn.extended==Some(0) || matches!(insn.info,InsnInfo::SceneChange{..}|InsnInfo::MoveTo{..}|InsnInfo::ExecMove{..}|InsnInfo::WarpOrInteract{..}) {
                    println!("MAN {:#x} ctx {:?}: {:?}; destination {:?}",record.start+insn.pc,insn.extended,insn.info,field_disasm::scene_change_name(&man[record.start..record.start+record.len],&insn));
                }
            }
        }
        ensure!([0,35,92,95,104].iter().all(|i|relevant.contains(i)),"Expected native incoming references were missed");
        // Demonstrates a same-width operand writer, NOT a new house identity:
        // changing references without allocating/rebinding the actor is unsafe.
        let changed=compile(&man,&BTreeMap::from([(0,1)]));
        println!("House context remap format eligibility: {}",match changed {Ok(p)=>format!("{} reference operands supported; NOT a bound-object export authorization",p.changed.len()),Err(e)=>format!("blocked: {e:#}")});
        let entry=scene.field_map_index(&library.index).context("No native MAP")?;
        let map=library.index.entry_bytes(entry)?;
        let go=0x8000+(25*128+38)*2;let descriptor=(u16::from_le_bytes(map[go..go+2].try_into()?)&511) as usize;
        let record=legaia_scene_formats::field_objects::ObjectRecord::parse(&map,descriptor).context("No house descriptor")?;
        let anchor=[(38i32+record.col_delta as i32) as u8,(25i32+record.row_delta as i32) as u8];
        let lba=library.index.entry_start_lba_retail(entry as u16).context("No MAP LBA")?;
        let load=library.index.prot_dat_raw_bytes(lba as u64*2048,0x14000)?;
        let mut teleports=vec![];
        for (label,bytes) in [("primary",&load[0x10000..0x12000]),("fallback",&load[0x12000..])] {
            for row in legaia_engine_field::field_regions::parse_intra_scene_teleports(bytes) {teleports.push((label,row));}
        }
        for (label,row) in teleports.iter().filter(|(_,r)|[r.tile_x,r.tile_z]==anchor || r.dest_tile()==(anchor[0],anchor[1])) {
            println!("Native house-anchor teleport {label}: {row:?}, landing world {:?}, arrival lookup {:?}",row.dest_world(),row.dest_tile());
        }
        println!("House MAP descriptor {descriptor}, native bind anchor {anchor:?}; co-coordinate triggers are references, not exclusive ownership");
        Ok(())
    }
}
