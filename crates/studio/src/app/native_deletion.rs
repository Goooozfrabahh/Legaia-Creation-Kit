//! Per-instance deletion dependencies. Spatial wall samples are conflicts to
//! resolve, never ownership metadata or permission to erase native wall bits.
use super::*;

#[derive(Debug)]
pub(super) struct DeletionDependency {
    pub descriptor:usize,
    pub cell:(u8,u8),
    pub footprint_anchor:(u8,u8),
    pub model_selector:u16,
    pub diagnostic_bounds:Option<[i32;4]>,
    pub unresolved_wall_bits:Vec<(i32,i32)>,
    pub grid_word:u16,
    pub replacement_word:u16,
}

pub(super) fn analyze(map:&[u8],edit:&MoveEdit,bindings:&NativeBindings,proof:Option<&insertion_provenance::VerifiedInsertions>)->Result<DeletionDependency> {
    ensure!(map.len()==MAP_LEN,"Native deletion requires the complete source MAP");
    let (id,record)=record_at(map,edit.cell,edit.source_position,edit.env_slot,edit.source_rotation)?;
    ensure!(id>3,"Native actor/player record #{id} requires a MAN/actor deletion writer");
    let anchor=(edit.cell.0 as i32+record.col_delta as i32,edit.cell.1 as i32+record.row_delta as i32);
    ensure!((0..128).contains(&anchor.0)&&(0..128).contains(&anchor.1),"Deletion record #{id} has an invalid native footprint anchor");
    let anchor=(anchor.0 as u8,anchor.1 as u8);
    for cell in [edit.cell,anchor] {
        if let Some(binding)=bindings.at(cell) {
            bail!("Deletion record #{id} at {:?}: {} binding byte {:#x}, MAN flat record {}, gate {} at {:?}; script/animation dependencies unresolved",edit.cell,binding.block,binding.offset,binding.record,binding.gate,cell);
        }
        ensure!(!bindings.has_trigger(cell),"Deletion record #{id} at {:?}: native teleport/elevation/trigger dependency at {:?}; no trigger removed",edit.cell,cell);
    }
    ensure!(!anchor_has_bind(map,edit.cell,&record),"Deletion record #{id} at {:?} is owned by the native bind sweep; context relocation/removal unresolved",edit.cell);
    let off=GRID_OFF+(edit.cell.1 as usize*GRID_DIM+edit.cell.0 as usize)*2;
    let grid_word=u16::from_le_bytes(map[off..off+2].try_into()?);
    // The verified field terrain emitters skip FLAG_PLACED records (see
    // scene_assembly.rs / FUN_801F69D8, FUN_801F7088). Their visibility bits do
    // not imply a separate terrain object at this reference. Remove this
    // object's index and draw gates, retaining slope/ground ordering and all
    // other cell bits. In particular 0x800 influences native floor sampling.
    ensure!(!ObjectRecord::parse(map,0).context("Native sentinel missing")?.is_placed(),
        "Deletion cannot use descriptor-0 sentinel: source has an active placed record there");
    let replacement_word=grid_word&!(field_objects::OBJECT_INDEX_MASK|field_objects::CELL_VISIBLE|field_objects::CELL_WALK_VISIBLE);
    let verified=proof.is_some_and(|p|p.covers(map,edit));
    let bounds=edit.collision_bounds;
    if !verified {
        let bounds=bounds.with_context(||format!("Deletion record #{id} at {:?}: original mesh diagnostic region unavailable; local MAP collision dependencies cannot be assessed. Unrelated walls elsewhere are not a deletion veto",edit.cell))?;
        ensure!(bounds[0]<=bounds[1]&&bounds[2]<=bounds[3]&&bounds.iter().all(|v|v.abs()<=32768),"Deletion record #{id} has invalid source geometry diagnostics");
    }
    Ok(DeletionDependency{descriptor:id,cell:edit.cell,footprint_anchor:anchor,
        model_selector:field_objects::pack_mesh_index(id as u16,&record).context("Native deletion model selector unavailable")?,
        diagnostic_bounds:bounds,unresolved_wall_bits:if verified {Vec::new()}else{source_stencil(map,bounds)},grid_word,replacement_word})
}

impl DeletionDependency {
    pub fn validate(&self)->Result<()> {
        let examples=self.unresolved_wall_bits.iter().take(12).copied().collect::<Vec<_>>();
        ensure!(self.unresolved_wall_bits.is_empty(),
            "Deletion record #{} model {} at {:?}, footprint anchor {:?}, grid {:#06x}: {} spatially relevant native MAP wall bits have unresolved ownership in source mesh diagnostic region {:?}; sub-cell examples {:?}. These bits are not assigned to the object. No walls erased and no successful partial IMG produced. Explicit ownership/reconstruction is required for this object; distant walls are preserved without blocking it",
            self.descriptor,self.model_selector,self.cell,self.footprint_anchor,self.grid_word,self.unresolved_wall_bits.len(),self.diagnostic_bounds,examples);
        Ok(())
    }
}
