//! Editor-only identity adapters and display-name resolution. Never used to
//! resolve donors, allocate native records or modify PS1 resources.
use std::collections::{BTreeMap,HashMap};
use sha2::{Digest,Sha256};
use legaia_engine_scene::{scene::{ProtIndex,Scene},scene_assembly::AssembledScene};
use super::{collision_recipes::digest,world_view::{WorldModel,NpcInfo}};

#[derive(Clone,Default)]
pub(super) struct Inventory {
    pub native:HashMap<(bool,usize),String>,
    pub assets:HashMap<(bool,usize),String>,
    pub meshes:Vec<Option<String>>,
    pub foreign:BTreeMap<u64,String>,
    pub foreign_legacy:BTreeMap<u64,String>,
}

pub(super) fn visual_key(tmd:&[u8],vram_hash:&str)->Option<String> {
    (!tmd.is_empty()).then(||format!("visual-v1:{}:{vram_hash}",digest(tmd)))
}
pub(super) fn material_key(vram:&legaia_tim::Vram)->String {
    let mut hash=Sha256::new();hash.update(vram.as_bytes());
    // An uploaded transparent zero is not an absent texture. The renderer
    // also consults upload coverage, so include that in verified identity.
    let mut coverage=Vec::with_capacity(legaia_tim::VRAM_PIXELS/8);
    for start in (0..legaia_tim::VRAM_PIXELS).step_by(8) {
        let mut byte=0u8;for bit in 0..8 {let i=start+bit;if vram.is_written(i%legaia_tim::VRAM_WIDTH,i/legaia_tim::VRAM_WIDTH){byte|=1<<bit;}}
        coverage.push(byte);
    }
    hash.update(&coverage);format!("{:x}",hash.finalize())
}
pub(super) fn visual_keys(scene:&AssembledScene)->Vec<Option<String>> {
    let material=material_key(&scene.res.vram);
    scene.res.tmds.iter().map(|t|visual_key(&t.raw,&material)).collect()
}

pub(super) fn inventory(scene:&AssembledScene,index:&ProtIndex,npcs:&[NpcInfo])->Inventory {
    let visuals=visual_keys(scene);
    let mut names=Inventory{meshes:scene.env_tmds.iter().map(|&i|visuals.get(i).cloned().flatten()).collect(),..Default::default()};
    let native=Scene::load(index,&scene.name).ok();
    let map=native.as_ref().and_then(|s|s.field_map_index(index)).and_then(|id|index.entry_bytes(id).ok()).filter(|bytes|bytes.len()==0x12000);
    let map_hash=map.as_ref().map(|m|digest(m));
    for (terrain,draws) in [(false,&scene.placements),(true,&scene.terrain)] {
        for (i,d) in draws.iter().enumerate() {
            if let Some(asset)=visuals.get(d.res_tmd).cloned().flatten() {names.assets.insert((terrain,i),asset);}
            if let (Some(map),Some(hash))=(&map,&map_hash) {
                let (x,z)=d.cell;let offset=0x8000+(z as usize*128+x as usize)*2;
                if x<128 && z<128 && offset+2<=map.len() {
                    let record=u16::from_le_bytes([map[offset],map[offset+1]])&511;
                    names.native.insert((terrain,i),format!("map-instance-v1:{}:{hash}:{}:{x}:{z}:{record}",scene.name,if terrain{"terrain"}else{"placement"}));
                }
            }
        }
    }
    if let Some(man)=native.as_ref().and_then(|s|s.field_man_payload(index).ok().flatten()) {
        let hash=digest(&man);
        for npc in npcs.iter().filter(|n|!n.is_copy) {
            let id=format!("man-instance-v1:{}:{hash}:{}",scene.name,npc.record);
            names.native.insert((false,npc.key),id.clone());
            names.assets.insert((false,npc.key),format!("npc-asset-v1:{id}"));
        }
    }
    let mut counts=HashMap::new();for id in names.native.values(){*counts.entry(id.clone()).or_insert(0usize)+=1;}
    names.native.retain(|_,id|counts.get(id)==Some(&1));
    names
}

impl WorldModel {
    pub fn base_name_key(&self,key:(bool,usize))->Option<&str> {
        let draw=self.at(key.0,key.1)?;
        if let Some(uid)=draw.editor_uid {if let Some(key)=self.naming.foreign.get(&uid){return Some(key);}}
        self.naming.assets.get(&draw.source_ref.unwrap_or(key)).map(String::as_str)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn material_identity_distinguishes_uploaded_zero_from_missing_texture() {
        let empty=legaia_tim::Vram::new();let mut uploaded=empty.clone();uploaded.write_block(0,0,1,1,&[0,0]);
        assert_eq!(empty.as_bytes(),uploaded.as_bytes());assert_ne!(material_key(&empty),material_key(&uploaded));
    }
    #[test]
    fn shared_names_require_exact_geometry_and_materials() {
        let key=visual_key(&[1,2,3],&digest(&[4,5])).unwrap();
        assert_eq!(Some(key.clone()),visual_key(&[1,2,3],&digest(&[4,5])));
        assert_ne!(Some(key.clone()),visual_key(&[1,2,3],&digest(&[4,6])));
        assert_ne!(Some(key),visual_key(&[1,2,4],&digest(&[4,5])));
        assert!(visual_key(&[],&digest(&[])).is_none());
    }
    #[test]
    fn aliases_survive_restart_and_share_only_verified_cross_cell_identity() {
        use crate::asset_catalog::{Catalog,Record};
        use super::super::global_repository::{Asset,Kind};
        let dir=tempfile::tempdir().unwrap();let path=dir.path().join("aliases.tsv");let legacy=dir.path().join("legacy.tsv");
        let key=visual_key(&[1,2,3],&digest(&[8,9])).unwrap();
        let a=Asset{source:"town01".into(),source_label:"Rim Elm".into(),label:"Native A".into(),key:"native:a".into(),alias_key:Some(key.clone()),kind:Kind::Placement(7),mesh_fingerprint:Some(3)};
        let mut b=a.clone();b.source="town02".into();b.key="native:b".into();b.kind=Kind::Mesh(22);
        let mut names=Catalog::load(&path,&legacy).unwrap();
        names.records.insert(key.clone(),Record{name:"Stone lantern".into(),aliases:"lamp".into(),..Default::default()});names.save(&path).unwrap();
        let mut restarted=Catalog::load(&path,&legacy).unwrap();
        assert_eq!(a.display_name(&restarted),"Stone lantern");assert_eq!(b.display_name(&restarted),"Stone lantern");assert!(b.matches("lamp",&restarted));
        let mut different=b.clone();different.alias_key=visual_key(&[1,2,3],&digest(&[8,10]));assert_eq!(different.display_name(&restarted),"Native A");
        restarted.records.remove(&key);restarted.save(&path).unwrap();
        assert_eq!(a.display_name(&Catalog::load(&path,&legacy).unwrap()),"Native A");
        assert_eq!(a.source,"town01");assert_eq!(a.key,"native:a");assert_eq!(b.source,"town02");
    }
}
