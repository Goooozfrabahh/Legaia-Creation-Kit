//! Explicitly reviewed native wall contributions and deterministic reconstruction.
//! This primitive is NOT an export permission or recovered retail ownership.
//! Unclassified baseline bits remain protected. No descriptor, trigger, floor,
//! MAN, or unrelated reference is rewritten here.
use super::{collision_recipes::{digest, NativeObjectId, ObjectId, SourceProvenance},
    native_script_dependencies::SceneInventory, ps1_export::native_dependencies::NativeBindings};
use legaia_scene_formats::field_objects::{self, ObjectRecord};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

const MAP_LEN: usize = 0x12000;
const LOAD_LEN: usize = 0x14000;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Claim {
    pub cell: [u8; 2],
    /// True when terrain/another retained scene contribution also owns this bit.
    pub retained_scene: bool,
    pub owners: BTreeSet<NativeObjectId>,
    pub evidence_note: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Review {
    pub version: u32,
    pub scene: String,
    pub map_entry: u32,
    pub map_sha256: String,
    /// Includes the real physically adjacent trigger fallback load window.
    pub load_sha256: String,
    pub man_sha256: String,
    pub sources: Vec<SourceProvenance>,
    pub claims: Vec<Claim>,
}

pub(super) struct Edit {
    pub owner: NativeObjectId,
    /// Native world X/Z units. Exact 64-unit wall quantization is required.
    pub delta: [i32; 2],
    pub delete: bool,
}
pub(super) struct Copy {
    pub source: NativeObjectId,
    pub uid: u64,
    pub delta: [i32; 2],
}

/// Private output binds this reconstruction to the exact MAP, fallback and MAN.
/// Production NativeExportPlan does not accept it until an object/actor/surface
/// and complete script-dependency contract is implemented and validated.
pub(super) struct Reconstruction {
    load_sha256: String,
    man_sha256: String,
    output: Vec<u8>,
    pub contributions: BTreeMap<ObjectId, BTreeSet<[u8; 2]>>,
}

fn location([sx, sz]: [u8; 2]) -> (usize, u8) {
    (0x4000 + (sz as usize / 2)*128 + sx as usize / 2, 1 << (4 + (sx & 1) + 2*(sz & 1)))
}
fn shifted(cell: [u8; 2], delta: [i32; 2]) -> Result<[u8; 2], String> {
    let mut out = [0; 2];
    for k in 0..2 {
        if delta[k] % 64 != 0 {return Err("Wall contributions require exact 64-unit X/Z translation".into());}
        let value = (cell[k] as i32).checked_add(delta[k] / 64).ok_or("Wall translation overflow")?;
        out[k] = u8::try_from(value).map_err(|_| "Wall contribution moved outside native grid")?;
    }
    Ok(out)
}

impl Reconstruction {
    pub fn compile(review: &Review, load: &[u8], man: &[u8], edits: &[Edit], copies: &[Copy]) -> Result<Self, String> {
        if review.version != 1 || review.scene.is_empty() || review.scene.len() > 128 || review.scene.chars().any(char::is_control) {
            return Err("Invalid native ownership review identity/version".into());
        }
        if load.len() != LOAD_LEN || digest(load) != review.load_sha256 || digest(&load[..MAP_LEN]) != review.map_sha256 || digest(man) != review.man_sha256 {
            return Err("Ownership review MAP/fallback/MAN fingerprint changed".into());
        }
        if edits.is_empty() && copies.is_empty() {return Err("No reviewed wall edit requested".into());}
        if review.sources.len() > 16384 || review.claims.len() > 65536 {return Err("Ownership review exceeds native capacity".into());}
        let map = &load[..MAP_LEN];
        let bindings = NativeBindings::parse(&load[0x10000..MAP_LEN], &load[MAP_LEN..]).map_err(|e|e.to_string())?;
        let scripts = SceneInventory::read(man)?;
        if !scripts.gaps.is_empty() {return Err(format!("Script dependency inventory incomplete: {}", scripts.gaps.join("; ")));}
        let mut sources = BTreeMap::new();
        for source in &review.sources {
            let id = &source.object;
            if id.scene != review.scene || id.map_entry != review.map_entry || source.map_sha256 != review.map_sha256 || source.man_sha256.as_deref() != Some(review.man_sha256.as_str()) ||
                id.cell.iter().any(|&v|v >= 128) || !(4..512).contains(&id.descriptor_id) || sources.insert(id.clone(), source).is_some() {
                return Err("Ownership source is duplicated, stale or belongs to another scene/MAP".into());
            }
            let off = id.descriptor_id as usize * 32;
            let grid = 0x8000 + (id.cell[1] as usize*128 + id.cell[0] as usize)*2;
            if map[off..off+32] != source.descriptor || u16::from_le_bytes(map[grid..grid+2].try_into().unwrap()) & field_objects::OBJECT_INDEX_MASK != id.descriptor_id ||
                !ObjectRecord::parse(map, id.descriptor_id as usize).is_some_and(|r|r.is_placed()) {
                return Err("Reviewed native placement/descriptor does not match the baseline".into());
            }
        }
        let mut contributions: BTreeMap<ObjectId, BTreeSet<[u8; 2]>> = BTreeMap::new();
        let mut claimed = BTreeSet::new();
        for claim in &review.claims {
            let (off, bit) = location(claim.cell);
            if !claimed.insert(claim.cell) || map[off] & bit == 0 || (claim.owners.is_empty() && !claim.retained_scene) ||
                claim.evidence_note.trim().is_empty() || claim.evidence_note.len() > 1024 || claim.evidence_note.chars().any(char::is_control) {
                return Err("Invalid, duplicate or absent baseline wall claim/evidence".into());
            }
            for owner in &claim.owners {
                if !sources.contains_key(owner) {return Err("Wall claim has an unreviewed owner".into());}
                contributions.entry(ObjectId::Native(owner.clone())).or_default().insert(claim.cell);
            }
        }
        let original = contributions.clone();
        let mut changed_owners = BTreeSet::new();
        let mut affected = BTreeSet::new();
        for edit in edits {
            if !changed_owners.insert(edit.owner.clone()) || (edit.delete && edit.delta != [0,0]) {return Err("Conflicting reviewed owner edits".into());}
            let key = ObjectId::Native(edit.owner.clone());
            let cells = original.get(&key).ok_or("No reviewed wall contribution for this owner; absence is not a collision proof")?;
            Self::require_static_owner(&edit.owner, map, &bindings)?;
            affected.extend(cells.iter().copied());
            let next = if edit.delete {BTreeSet::new()} else {cells.iter().map(|&c|shifted(c, edit.delta)).collect::<Result<_,_>>()?};
            affected.extend(next.iter().copied()); contributions.insert(key, next);
        }
        for copy in copies {
            let key = ObjectId::Created(copy.uid);
            if copy.uid == 0 || contributions.contains_key(&key) {return Err("Duplicate or zero wall copy UID".into());}
            let cells = original.get(&ObjectId::Native(copy.source.clone())).ok_or("Wall copy has no reviewed native source")?;
            Self::require_static_owner(&copy.source, map, &bindings)?;
            let next: BTreeSet<_> = cells.iter().map(|&c|shifted(c, copy.delta)).collect::<Result<_,_>>()?;
            affected.extend(cells.iter().copied()); affected.extend(next.iter().copied()); contributions.insert(key, next);
        }
        for &cell in &affected {
            let writers = scripts.writers_at(cell);
            if !writers.is_empty() {return Err(format!("Wall sub-cell {cell:?} has unresolved script writers (flat MAN record, absolute MAN byte): {writers:?}"));}
        }
        let mut output = map.to_vec();
        // Clear only explicitly classified baseline contributions. Unclassified
        // bits, scene terrain and contributions of every retained owner survive.
        for claim in &review.claims {
            if !claim.retained_scene {let (off, bit) = location(claim.cell); output[off] &= !bit;}
        }
        for cells in contributions.values() {
            for &cell in cells {let (off, bit) = location(cell); output[off] |= bit;}
        }
        for i in 0..MAP_LEN {
            if (i < 0x4000 || i >= 0x8000) && output[i] != map[i] || output[i] & 15 != map[i] & 15 {
                return Err("Ownership reconstruction changed non-wall native data".into());
            }
        }
        Ok(Self {load_sha256: review.load_sha256.clone(), man_sha256: review.man_sha256.clone(), output, contributions})
    }

    fn require_static_owner(owner: &NativeObjectId, map: &[u8], bindings: &NativeBindings) -> Result<(), String> {
        let rec = ObjectRecord::parse(map, owner.descriptor_id as usize).ok_or("Missing native owner descriptor")?;
        let anchor = [owner.cell[0] as i32 + rec.col_delta as i32, owner.cell[1] as i32 + rec.row_delta as i32];
        if anchor.iter().any(|v|!(0..128).contains(v)) {return Err("Reviewed owner has an invalid native binding anchor".into());}
        for cell in [owner.cell, [anchor[0] as u8, anchor[1] as u8]] {
            let off = 0x8000 + (cell[1] as usize*128 + cell[0] as usize)*2;
            if let Some(bind) = bindings.at((cell[0],cell[1])) {
                return Err(format!("Owner binding unresolved: {} byte {:#x}, flat MAN record {}, gate {}", bind.block, bind.offset, bind.record, bind.gate));
            }
            if bindings.has_trigger((cell[0],cell[1])) || u16::from_le_bytes(map[off..off+2].try_into().unwrap()) & field_objects::CELL_BIND_OWNED != 0 {
                return Err("Owner trigger/bind-sweep dependency unresolved".into());
            }
        }
        Ok(())
    }
    pub fn apply(&self, load: &[u8], man: &[u8]) -> Result<Vec<u8>, String> {
        if load.len() != LOAD_LEN || digest(load) != self.load_sha256 || digest(man) != self.man_sha256 {
            return Err("Reconstruction source MAP/fallback/MAN changed".into());
        }
        Ok(self.output.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::native_script_dependencies::fixture_man;

    fn rebind(review: &mut Review, load: &[u8], man: &[u8]) {
        review.map_sha256 = digest(&load[..MAP_LEN]);
        review.load_sha256 = digest(load); review.man_sha256 = digest(man);
        for source in &mut review.sources {
            source.map_sha256 = review.map_sha256.clone(); source.man_sha256 = Some(review.man_sha256.clone());
            let off = source.object.descriptor_id as usize*32;
            source.descriptor.copy_from_slice(&load[off..off+32]);
        }
    }
    fn fixture(scripts: &[&[u8]]) -> (Review, Vec<u8>, Vec<u8>) {
        let man = fixture_man(scripts);
        let mut load = vec![0; LOAD_LEN]; load[0x4000..0x8000].fill(5);
        let mut sources = Vec::new();
        for (id, col) in [(20u16,20u8),(21,21)] {
            load[id as usize*32+0x12..id as usize*32+0x14].copy_from_slice(&0x16u16.to_le_bytes());
            let at = 0x8000 + (20*128 + col as usize)*2;
            load[at..at+2].copy_from_slice(&id.to_le_bytes());
            sources.push(SourceProvenance {object: NativeObjectId {scene: "synthetic".into(), map_entry: 1, cell: [col,20], descriptor_id: id},
                map_sha256: String::new(), man_sha256: None, descriptor: [0;32]});
        }
        let a = sources[0].object.clone(); let b = sources[1].object.clone();
        let claims = vec![
            Claim {cell:[10,10],retained_scene:false,owners:BTreeSet::from([a.clone()]),evidence_note:"Explicit synthetic exclusive contribution".into()},
            Claim {cell:[11,10],retained_scene:false,owners:BTreeSet::from([a.clone(),b]),evidence_note:"Explicit synthetic shared contribution".into()},
            Claim {cell:[12,10],retained_scene:true,owners:BTreeSet::from([a]),evidence_note:"Explicit synthetic scene contribution".into()}];
        for cell in [[10,10],[11,10],[12,10],[13,10]] {let (off,bit)=location(cell);load[off]|=bit;}
        let mut review = Review {version:1,scene:"synthetic".into(),map_entry:1,map_sha256:String::new(),load_sha256:String::new(),man_sha256:String::new(),sources,claims};
        rebind(&mut review,&load,&man); (review,load,man)
    }
    fn solid(map: &[u8], cell: [u8;2]) -> bool {let (off,bit)=location(cell);map[off]&bit!=0}
    fn delete(owner: &NativeObjectId) -> Edit {Edit {owner:owner.clone(),delta:[0,0],delete:true}}

    #[test]
    fn deletion_removes_only_exclusive_bits_and_keeps_shared_terrain_unknown_and_native_bytes() {
        let (review,load,man)=fixture(&[&[0x21]]);
        let owner=&review.sources[0].object;
        let plan=Reconstruction::compile(&review,&load,&man,&[delete(owner)],&[]).unwrap();
        let output=plan.apply(&load,&man).unwrap();
        assert!(!solid(&output,[10,10]));
        for cell in [[11,10],[12,10],[13,10]] {assert!(solid(&output,cell));}
        assert_eq!(&output[..0x4000],&load[..0x4000]);
        assert_eq!(&output[0x8000..],&load[0x8000..MAP_LEN]);
        assert!(output[0x4000..0x8000].iter().zip(&load[0x4000..0x8000]).all(|(a,b)|a&15==b&15));
        // Independently decode every original floor tier and grid reference.
        let before=legaia_engine_field::minigame_floor::FloorGrid::new(&load[..MAP_LEN]);
        let after=legaia_engine_field::minigame_floor::FloorGrid::new(&output);
        for z in 0..128 {for x in 0..128 {
            assert_eq!(before.height_index(x,z),after.height_index(x,z));
            assert_eq!(before.cell(x,z),after.cell(x,z));
        }}
        assert_eq!(plan.contributions[&ObjectId::Native(owner.clone())].len(),0);
    }
    #[test]
    fn multi_owner_deletion_and_move_copy_union_are_order_independent() {
        let (review,load,man)=fixture(&[&[0x21]]);let a=&review.sources[0].object;let b=&review.sources[1].object;
        let first=Reconstruction::compile(&review,&load,&man,&[delete(a),delete(b)],&[]).unwrap().apply(&load,&man).unwrap();
        let second=Reconstruction::compile(&review,&load,&man,&[delete(b),delete(a)],&[]).unwrap().apply(&load,&man).unwrap();
        assert_eq!(first,second);assert!(!solid(&first,[11,10]));assert!(solid(&first,[12,10]));
        let plan=Reconstruction::compile(&review,&load,&man,&[Edit{owner:a.clone(),delta:[64,0],delete:false}],
            &[Copy{source:a.clone(),uid:100,delta:[256,0]}]).unwrap();
        let output=plan.apply(&load,&man).unwrap();
        assert!(!solid(&output,[10,10]));
        for cell in [[11,10],[12,10],[13,10],[14,10],[15,10],[16,10]] {assert!(solid(&output,cell));}
        assert_eq!(plan.contributions[&ObjectId::Created(100)],BTreeSet::from([[14,10],[15,10],[16,10]]));
        assert_eq!(&output[0x8000..],&load[0x8000..MAP_LEN]);
    }
    #[test]
    fn copy_uses_reviewed_source_even_when_original_is_deleted_in_same_transaction() {
        let (review,load,man)=fixture(&[&[0x21]]);let a=&review.sources[0].object;
        let output=Reconstruction::compile(&review,&load,&man,&[delete(a)],&[Copy{source:a.clone(),uid:1,delta:[256,0]}]).unwrap().apply(&load,&man).unwrap();
        assert!(!solid(&output,[10,10]));assert!(solid(&output,[14,10]));assert!(solid(&output,[11,10]));
    }
    #[test]
    fn fingerprints_cover_map_descriptor_grid_man_fallback_and_prevent_accumulation() {
        let (review,load,man)=fixture(&[&[0x21]]);let a=&review.sources[0].object;
        let plan=Reconstruction::compile(&review,&load,&man,&[delete(a)],&[]).unwrap();
        for at in [20*32,0x4000,0x8000,0x10010,0x13fff] {
            let mut stale=load.clone();stale[at]^=1;
            assert!(plan.apply(&stale,&man).is_err());
            assert!(Reconstruction::compile(&review,&stale,&man,&[delete(a)],&[]).is_err());
        }
        let mut stale_man=man.clone();stale_man[2]^=1;assert!(plan.apply(&load,&stale_man).is_err());
        let mut accumulated=load.clone();accumulated[..MAP_LEN].copy_from_slice(&plan.apply(&load,&man).unwrap());
        assert!(plan.apply(&accumulated,&man).is_err());
        let mut wrong=review.clone();wrong.sources[0].object.map_entry+=1;
        assert!(Reconstruction::compile(&wrong,&load,&man,&[delete(a)],&[]).is_err());
    }
    #[test]
    fn review_roundtrip_is_explicit_and_invalid_partial_claims_or_transforms_fail() {
        let (review,load,man)=fixture(&[&[0x21]]);let a=&review.sources[0].object;
        let encoded=serde_json::to_vec(&review).unwrap();let restored:Review=serde_json::from_slice(&encoded).unwrap();
        assert!(Reconstruction::compile(&restored,&load,&man,&[delete(a)],&[]).is_ok());
        for delta in [[1,0],[-1024,0],[i32::MAX,0]] {
            assert!(Reconstruction::compile(&review,&load,&man,&[Edit{owner:a.clone(),delta,delete:false}],&[]).is_err());
        }
        let mut empty=review.clone();empty.claims.clear();assert!(Reconstruction::compile(&empty,&load,&man,&[delete(a)],&[]).is_err());
        let mut duplicate=review.clone();duplicate.claims.push(duplicate.claims[0].clone());
        assert!(Reconstruction::compile(&duplicate,&load,&man,&[delete(a)],&[]).is_err());
        let mut absent=review.clone();absent.claims[0].cell=[60,60];
        assert!(Reconstruction::compile(&absent,&load,&man,&[delete(a)],&[]).is_err());
        assert!(Reconstruction::compile(&review,&load,&man,&[delete(a),delete(a)],&[]).is_err());
    }
    #[test]
    fn conditional_clear_set_writers_block_affected_source_and_destination_bits() {
        let (review,load,man)=fixture(&[&[0x4c,0x70,5,4,5,4,0x4c,0x73,5,4,5,4,1,0x21]]);
        let error=Reconstruction::compile(&review,&load,&man,&[delete(&review.sources[0].object)],&[]).err().unwrap();
        assert!(error.contains("script writers"));
        let inventory=SceneInventory::read(&man).unwrap();assert_eq!(inventory.writers_at([10,10]).len(),2);
        let (review,load,man)=fixture(&[&[0x4c,0x71,7,4,7,4,0x21]]);
        let error=Reconstruction::compile(&review,&load,&man,&[],&[Copy{source:review.sources[0].object.clone(),uid:1,delta:[256,0]}]).err().unwrap();
        assert!(error.contains("script writers"));
        // A different sub-cell mask in the same native byte is not conflated.
        let (review,load,man)=fixture(&[&[0x4c,0x73,5,4,5,4,8,0x21]]);
        assert!(Reconstruction::compile(&review,&load,&man,&[delete(&review.sources[0].object)],&[]).is_ok());
    }
    #[test]
    fn decode_gaps_and_fallback_bindings_do_not_become_static_ownership_permission() {
        let (review,load,man)=fixture(&[&[0x4c,0x73]]);
        let error=Reconstruction::compile(&review,&load,&man,&[delete(&review.sources[0].object)],&[]).err().unwrap();
        assert!(error.contains("incomplete"));
        let (mut review,mut load,man)=fixture(&[&[0x21]]);
        let base=MAP_LEN;load[base+6..base+8].copy_from_slice(&18i16.to_le_bytes());load[base+8..base+10].copy_from_slice(&1i16.to_le_bytes());
        load[base+18..base+22].copy_from_slice(&[20,20,255,7]);rebind(&mut review,&load,&man);
        let error=Reconstruction::compile(&review,&load,&man,&[delete(&review.sources[0].object)],&[]).err().unwrap();
        assert!(error.contains("fallback")&&error.contains("255")&&error.contains("gate 7"));
    }
}
