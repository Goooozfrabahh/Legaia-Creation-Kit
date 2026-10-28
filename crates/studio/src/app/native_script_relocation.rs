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

pub(super) struct WallTranslation {pub instruction_offset:usize,pub delta_tiles:[i16;2]}
pub(super) struct WallRelocation {source_sha256:String,output:Vec<u8>,pub changed:Vec<usize>}
impl WallRelocation {
    pub fn apply(&self,source:&[u8])->Result<Vec<u8>> {ensure!(digest(source)==self.source_sha256,"Wall script source changed");Ok(self.output.clone())}
}
/// Relocate explicitly selected native wall-paint instructions without changing
/// script widths, branches, context references or execution conditions. This is
/// NOT proof those instructions exclusively own any stored MAP wall bit.
pub(super) fn compile_wall_translations(man:&[u8],edits:&[WallTranslation])->Result<WallRelocation> {
    let inventory=SceneInventory::read(man).map_err(anyhow::Error::msg)?;
    ensure!(inventory.gaps.is_empty() && !edits.is_empty(),"Incomplete/empty native wall-script request");
    let mut output=man.to_vec();let mut allowed=BTreeSet::new();let mut changed=Vec::new();
    for edit in edits {
        ensure!(edit.delta_tiles!=[0;2] && !changed.contains(&edit.instruction_offset),"Repeated/empty wall-script translation");
        let record=inventory.records.iter().find(|r|edit.instruction_offset>=r.start+r.pc0 && edit.instruction_offset<r.start+r.len).context("Wall instruction is outside a native script")?;
        let local=edit.instruction_offset-record.start;
        let wall=record.scan.wall_writes.iter().find(|w|w.pc==local).context("Selected byte is not a decoded native wall-paint boundary")?;
        ensure!(wall.sub<=3,"Unknown native wall-paint suboperation");
        let instruction=LinearWalker::new(&man[record.start..record.start+record.len],record.pc0).find_map(|r|r.ok().filter(|i|i.pc==local)).context("Missing wall instruction")?;
        let operand=edit.instruction_offset+instruction.size-if wall.sub>=2 {5}else{4};
        let old=[wall.x0,wall.z0,wall.x1,wall.z1];
        ensure!(man[operand..operand+4]==old,"Native wall operand layout differs");
        for axis in 0..4 {
            let v=i32::from(old[axis])+i32::from(edit.delta_tiles[axis%2]);
            ensure!((0..128).contains(&v),"Wall-paint translation exceeds native MAP bounds");
            output[operand+axis]=v as u8;allowed.insert(operand+axis);
        }
        changed.push(edit.instruction_offset);
    }
    let read=SceneInventory::read(&output).map_err(anyhow::Error::msg)?;
    ensure!(read.gaps.is_empty() && read.partition_counts==inventory.partition_counts,"Wall translation changed script structure");
    for edit in edits {
        let old_record=inventory.records.iter().find(|r|r.scan.wall_writes.iter().any(|w|r.start+w.pc==edit.instruction_offset)).unwrap();
        let new_record=read.records.iter().find(|r|r.flat==old_record.flat).unwrap();
        let old=old_record.scan.wall_writes.iter().find(|w|old_record.start+w.pc==edit.instruction_offset).unwrap();
        let new=new_record.scan.wall_writes.iter().find(|w|new_record.start+w.pc==edit.instruction_offset).context("Wall instruction disappeared")?;
        // Verify exact native sub-cell contributions, including row bias,
        // masks and wrapping bounds. No bounding-box ownership inference.
        let mut before=BTreeSet::new();let mut after=BTreeSet::new();
        for z in 0..256 {for x in 0..256 {
            if old.touches([x as u8,z as u8]) {let tx=x+edit.delta_tiles[0] as i32*2;let tz=z+edit.delta_tiles[1] as i32*2;
                ensure!((0..256).contains(&tx)&&(0..256).contains(&tz),"Native wall contribution would leave the grid");before.insert([tx,tz]);}
            if new.touches([x as u8,z as u8]) {after.insert([x,z]);}
        }}
        ensure!(!before.is_empty() && before==after && old.sub==new.sub && old.mask==new.mask,"Native wall effect translation/readback differs");
    }
    for (i,(&old,&new)) in man.iter().zip(&output).enumerate(){ensure!(old==new||allowed.contains(&i),"Unrelated MAN byte changed");}
    Ok(WallRelocation{source_sha256:digest(man),output,changed})
}
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
    fn wall_paint_dependencies_translate_exact_native_effects_without_rewriting_control_flow()->Result<()> {
        let man=fixture_man(&[&[0x4c,0x70,3,4,4,5,0x4c,0x73,3,4,4,5,5,0x21]]);
        let inventory=SceneInventory::read(&man).unwrap();let record=&inventory.records[0];
        let edits=record.scan.wall_writes.iter().map(|w|WallTranslation{instruction_offset:record.start+w.pc,delta_tiles:[2,-1]}).collect::<Vec<_>>();
        let plan=compile_wall_translations(&man,&edits)?;assert_eq!(plan.changed.len(),2);
        let output=plan.apply(&man)?;assert_eq!(output.len(),man.len());
        assert!(compile_wall_translations(&man,&[WallTranslation{instruction_offset:edits[0].instruction_offset+1,delta_tiles:[1,0]}]).is_err());
        assert!(compile_wall_translations(&man,&[WallTranslation{instruction_offset:edits[0].instruction_offset,delta_tiles:[-10,0]}]).is_err());
        assert!(compile_wall_translations(&man,&[WallTranslation{instruction_offset:edits[0].instruction_offset,delta_tiles:[i16::MAX,0]}]).is_err());
        let mut stale=man;stale[1]=1;assert!(plan.apply(&stale).is_err());Ok(())
    }
    #[test]
    #[ignore="explicit read-only retail MAN dependency serializer regression; no IMG/GUI"]
    fn retail_native_wall_script_translation_readback()->Result<()> {
        let path=std::path::PathBuf::from(std::env::var_os("LCK_RETAIL_SOURCE").context("LCK_RETAIL_SOURCE required")?);
        let library=crate::library::Library::open(&path)?;
        let scene=legaia_engine_scene::scene::Scene::load(&library.index,"town01")?;
        let man=scene.field_man_payload(&library.index)?.context("No native MAN")?;
        let inventory=SceneInventory::read(&man).map_err(anyhow::Error::msg)?;
        ensure!(inventory.gaps.is_empty(),"Retail wall writer coverage incomplete");
        let mut accepted=0;let mut rejected=0;
        for record in &inventory.records {for wall in &record.scan.wall_writes {
            let offset=record.start+wall.pc;
            let candidate=[[1,0],[-1,0],[0,1],[0,-1]].into_iter().find_map(|delta_tiles|compile_wall_translations(&man,&[WallTranslation{instruction_offset:offset,delta_tiles}]).ok());
            if let Some(plan)=candidate {let rewritten=plan.apply(&man)?;ensure!(rewritten.len()==man.len(),"Native MAN allocation changed");accepted+=1;
                println!("Native wall instruction MAN {offset:#x}, flat {}: same-width coordinate/effect readback passed; object ownership NOT established",record.flat);
            }else{rejected+=1;}
        }}
        ensure!(accepted>0,"No real native wall instruction was safely relocated");
        println!("Retail MAN: {accepted} wall instructions passed relocation readback; {rejected} rejected. No MAP wall bit or exported IMG changed.");Ok(())
    }
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
