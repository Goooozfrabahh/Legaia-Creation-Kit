//! Read-only evidence reporting; never authorizes MAN relocation or export.
use super::{collision_recipes, ps1_export::native_dependencies::NativeBindings, world_view::WorldModel};
use legaia_engine_scene::{scene::{ProtIndex, Scene},scene_assembly::AssembledScene};
use legaia_scene_formats::field_objects::ObjectRecord;
use super::native_script_dependencies::{scan_script, SceneInventory};
use std::fmt::Write;

pub(super) fn report(scene:&AssembledScene,world:&WorldModel,key:(bool,usize),index:&ProtIndex)->Result<String,String> {
    let source=world.collision.source(world,key).ok_or("Verified native MAP source unavailable for this selection")?;
    let id=&source.object;
    if id.scene!=scene.name {return Err("Source scene differs; donor dependency reporting is not available here".into());}
    let native=Scene::load(index,&id.scene).map_err(|e|e.to_string())?;
    let map=index.entry_bytes(id.map_entry).map_err(|e|e.to_string())?;
    if collision_recipes::digest(&map)!=source.map_sha256 {return Err("Source MAP changed".into());}
    let lba=index.entry_start_lba_retail(id.map_entry.try_into().map_err(|_|"MAP index outside native range")?)
        .ok_or("Native MAP load address unavailable")?;
    let load=index.prot_dat_raw_bytes(lba as u64*2048,0x14000).map_err(|e|e.to_string())?;
    if load.get(..0x12000)!=Some(map.as_slice()) {return Err("Physical MAP load differs from indexed MAP".into());}
    let binds=NativeBindings::parse(&load[0x10000..0x12000],&load[0x12000..]).map_err(|e|e.to_string())?;
    let rec=ObjectRecord::parse(&source.descriptor,0).ok_or("Descriptor unavailable")?;
    let anchor=(id.cell[0] as i32+rec.col_delta as i32,id.cell[1] as i32+rec.row_delta as i32);
    if !(0..128).contains(&anchor.0)||!(0..128).contains(&anchor.1) {return Err("Native anchor outside grid".into());}
    let grid=0x8000+(id.cell[1] as usize*128+id.cell[0] as usize)*2;
    let word=u16::from_le_bytes(map[grid..grid+2].try_into().unwrap());
    let mut out=String::new();
    writeln!(out,"{}",super::ps1_export::collision_ownership_diagnostic(world,key,&map)).unwrap();
    writeln!(out,"Native source: {} / MAP PROT {} / descriptor {}\nPlacement cell {:?}; binding anchor {:?}",id.scene,id.map_entry,id.descriptor_id,id.cell,anchor).unwrap();
    writeln!(out,"MAP SHA-256 {}\nDescriptor flags {:#06x}; mesh selector {}; grid word {word:#06x}",source.map_sha256,rec.flags,rec.pack_index_field).unwrap();
    let offset=legaia_scene_formats::field_objects::collision_footprint_offset(&rec);
    writeln!(out,"Verified record-local collider offset: X {} Z {}. Script runtime flags may change collision.\nWall ownership: UNKNOWN. Walkable surfaces: scene grid/LUT/kind-2 dependencies; mesh does not establish ownership.",offset.0,offset.1).unwrap();
    for (name,block) in [("MAP",&load[0x10000..0x12000]),("fallback",&load[0x12000..])] {
        for kind in 0..3 {
            let h=kind*4;let start=i16::from_le_bytes([block[h+2],block[h+3]]);let count=i16::from_le_bytes([block[h+4],block[h+5]]);
            if count<=0 {continue;}
            // NativeBindings already checked these complete row ranges.
            for (i,row) in block[start as usize..start as usize+count as usize*4].chunks_exact(4).enumerate() {
                if (row[0],row[1])==(id.cell[0],id.cell[1]) || (row[0] as i32,row[1] as i32)==anchor {
                    writeln!(out,"{name} kind-{kind} row at +{:#x}: {row:02x?}",start as usize+i*4).unwrap();
                }
            }
        }
    }
    let binding=binds.at((anchor.0 as u8,anchor.1 as u8));
    if let Some(binding)=binding {
        writeln!(out,"Resolved binding: {} block +{:#x}; flat MAN record {}; gate {} (gate is not a bind filter)",binding.block,binding.offset,binding.record,binding.gate).unwrap();
    } else {
        writeln!(out,"No kind-1 binding at native anchor. Stored bind marker and window spawn rules still require validation. No script-closure claim.").unwrap();
    }
    // An unbound descriptor can still sit in a rectangle painted by a scene
    // script. Always inventory the MAN; a missing bind is not a wall proof.
    let man=native.field_man_payload(index).map_err(|e|e.to_string())?.ok_or("Scene MAN unavailable; wall-writer coverage unresolved")?;
    if source.man_sha256.as_deref()!=Some(&collision_recipes::digest(&man)) {return Err("Source MAN changed".into());}
    let inventory=SceneInventory::read(&man)?;
    let target=binding.map(|b|b.record as usize);
    writeln!(out,"MAN SHA-256 {}; partition counts {:?}",inventory.man_sha256,inventory.partition_counts).unwrap();
    let mut incoming=Vec::new();let mut found=false;
    for item in &inventory.records {
            let (partition,record,flat,start,pc0,len)=(item.partition,item.record,item.flat,item.start,item.pc0,item.len);
            let body=&man[start..start+len];let scan=&item.scan;
            for (pc,destination) in &scan.targets {
                if Some(*destination as usize)==target {incoming.push(format!("P{partition}[{record}] flat {flat} +{pc:#x}"));}
            }
            for writer in &scan.wall_writes {
                let pc=writer.pc;let operation=writer.description();
                writeln!(out,"Observed scene wall writer: P{partition}[{record}] flat {flat}, MAN byte {:#x}, record +{pc:#x}: {operation}. Execution conditions unresolved. Writer is not exclusive ownership.",start+pc).unwrap();
            }
            if Some(flat)==target {
                found=true;
                writeln!(out,"Bound P{partition}[{record}] bytes {start:#x}..{:#x}; script +{pc0:#x}",start+len).unwrap();
                if partition==0 {writeln!(out,"Animation ID {} (native header); clip compatibility/lifetime not proven by this report",body[pc0-1]).unwrap();}
                writeln!(out,"Decoded outgoing context targets: {:?}\nDecoded wall-write instruction offsets: {:?}",scan.targets,scan.wall_writes).unwrap();
                if let Some(error)=&scan.error {writeln!(out,"INCOMPLETE: {error}").unwrap();}
                for instruction in &scan.instructions {writeln!(out,"{instruction}").unwrap();}
            }
    }
    for gap in &inventory.gaps {writeln!(out,"Scene coverage gap: {gap}").unwrap();}
    let incomplete=inventory.gaps.len();
    if target.is_some() && !found {writeln!(out,"UNRESOLVED: bound MAN record is absent or its header/span cannot be decoded.").unwrap();}
    writeln!(out,"Observed incoming context references: {incoming:?}\nIncomplete record scans: {incomplete}. Scans stop at first decode error without resynchronizing.\nThis is a linear instruction inventory, not a complete control-flow/dependency proof. Dynamic targets, helper calls, global state and resource closure remain unresolved. No MAN relocation is performed.").unwrap();
    Ok(out)
}

pub(super) fn show(ui:&mut eframe::egui::Ui,scene:&AssembledScene,world:&WorldModel,key:(bool,usize),index:&ProtIndex) {
    use eframe::egui;
    let source=world.collision.source(world,key);
    let id=egui::Id::new(("lck_native_dependency_report",&scene.name,key,source.map(|s|(&s.map_sha256,&s.man_sha256))));
    if ui.button("Read native dependencies").clicked() {
        let text=report(scene,world,key,index).unwrap_or_else(|error|format!("Dependency report unavailable: {error}"));
        ui.ctx().data_mut(|data|data.insert_temp(id,text));
    }
    if let Some(text)=ui.ctx().data_mut(|data|data.get_temp::<String>(id)) {
        egui::ScrollArea::vertical().id_salt(id).max_height(240.0).show(ui,|ui|{ui.monospace(text);});
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stops_on_unknown_instruction_without_claiming_later_dependencies() {
        let scan=scan_script(&[0x21,0x4c,0xb0,0xcc,7,0x21],0);
        assert!(scan.error.is_some());assert!(scan.targets.is_empty());
        assert_eq!(scan.instructions.len(),1);
    }
    #[test]
    fn reports_context_reference_on_instruction_boundary() {
        let scan=scan_script(&[0xa1,7,0x21],0);
        assert!(scan.error.is_none());assert_eq!(scan.targets,vec![(0,7)]);
    }
    #[test]
    fn wall_writer_inventory_keeps_clear_and_set_instruction_boundaries() {
        let scan=scan_script(&[0x4c,0x70,1,2,3,4,0x4c,0x73,1,2,3,4,5,0x21],0);
        assert!(scan.error.is_none());
        assert_eq!(scan.wall_writes.iter().map(|w|w.pc).collect::<Vec<_>>(),vec![0,6]);
        assert!(scan.wall_writes[0].description().contains("sub: 0"));
        assert!(scan.wall_writes[1].description().contains("mask: 5"));
        assert!(scan.wall_writes[1].description().contains("X [1,4) Z [3,6)"));
        let edge=super::super::native_script_dependencies::WallWrite{pc:0,sub:3,x0:255,z0:255,x1:255,z1:255,mask:255}.description();
        assert!(edge.contains("X [255,0) Z [0,1)"));
        let truncated=scan_script(&[0x4c,0x73,1,2,3,4],0);
        assert!(truncated.error.is_some());assert!(truncated.wall_writes.is_empty());
    }
}
