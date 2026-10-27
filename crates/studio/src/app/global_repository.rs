//! Game-wide original scene resource index for LCK. A complete browse source,
//! not an assertion that every PS1 model is portable to every destination.
//! Never changes the loaded scene, IMG, or retail archive.
use std::{hash::{Hash, Hasher}, sync::{Arc,mpsc::Sender}};
use legaia_engine_scene::{scene::ProtIndex,scene_assembly::AssembledScene};
use crate::asset_catalog::Catalog;
use super::{EditorCatalogItem,world_view};

#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub(super) enum Kind { Placement(usize), Terrain(usize), Mesh(usize), Npc(usize) }

#[derive(Clone,Debug)]
pub(super) struct Asset {
    pub source:String,
    pub source_label:String,
    pub label:String,
    pub key:String,
    pub alias_key:Option<String>,
    pub kind:Kind,
    /// Hash of original exact PS1 TMD bytes, not a guessed semantic name.
    /// Any missing or malformed geometry is never offered as placeable.
    pub mesh_fingerprint:Option<u64>,
}
impl Asset {
    pub fn name_key(&self)->String {self.alias_key.clone().unwrap_or_else(||self.key.clone())}
    pub fn display_name(&self,names:&Catalog)->String {
        names.get(&self.name_key()).or_else(||names.get(&self.key)).cloned().unwrap_or_else(||self.label.clone())
    }
    pub fn name_record(&self,names:&Catalog)->crate::asset_catalog::Record {
        names.records.get(&self.name_key()).or_else(||names.records.get(&self.key)).cloned().unwrap_or_default()
    }
    /// Structural categories reflect PS1 source formats, NOT invented identities.
    pub fn group(&self)->usize {
        match self.kind {Kind::Placement(_)=>0,Kind::Terrain(_)=>1,
            Kind::Mesh(_)=>2,Kind::Npc(_)=>3}
    }
    pub fn kind_label(&self)->&'static str {
        match self.kind {Kind::Placement(_)=>"MAP object",Kind::Terrain(_)=>"terrain/decor",
            Kind::Mesh(_)=>"TMD mesh",Kind::Npc(_)=>"MAN NPC"}
    }
    pub fn matches(&self,term:&str, names:&Catalog)->bool {
        term.is_empty() || self.label.to_lowercase().contains(term) ||
        self.source.to_lowercase().contains(term) ||
        self.source_label.to_lowercase().contains(term) ||
        self.key.to_lowercase().contains(term) ||
        names.get(&self.key).is_some_and(|s|s.to_lowercase().contains(term)) ||
        names.search_text(&self.key).to_lowercase().contains(term) || names.search_text(&self.name_key()).to_lowercase().contains(term)
    }
}
#[derive(Debug)]
pub(super) struct Batch {pub assets:Vec<Asset>,pub failed:bool}
fn fingerprint(raw:&[u8])->Option<u64> {
    if raw.is_empty(){return None;}
    let mut h=std::collections::hash_map::DefaultHasher::new();
    raw.hash(&mut h);
    Some(h.finish())
}
fn mesh_hash(scene:&AssembledScene,index:usize)->Option<u64> {
    let bytes=&scene.res.tmds.get(index)?.raw;
    fingerprint(bytes)
}
fn catalog_one(index:&ProtIndex,name:&str)->Result<Vec<Asset>,String> {
    let scene=legaia_engine_world_views::scene_staging::assemble_field_scene(index,name)?;
    let display_source=crate::place_name(name).unwrap_or(name).to_owned();
    let visual_keys=super::naming::visual_keys(&scene);
    let mut found=Vec::with_capacity(scene.placements.len()+scene.terrain.len()+scene.env_tmds.len());
    for (i,d) in scene.placements.iter().enumerate() {
        found.push(Asset {
            source:name.to_string(),source_label:display_source.clone(),label:format!("Unidentified world object #{i:03} [TMD {} / x{} y{} z{}]",
                d.env_slot,d.world_x,d.world_y,d.world_z),
            key:format!("cell:{name}|placed:{i}|mesh:{}|anchor:{},{}|pos:{},{},{}",
                d.env_slot,d.anchor.0,d.anchor.1,d.world_x,d.world_y,d.world_z),
            alias_key:visual_keys.get(d.res_tmd).cloned().flatten(),kind:Kind::Placement(i),mesh_fingerprint:mesh_hash(&scene,d.res_tmd),
        });
    }
    for (i,d) in scene.terrain.iter().enumerate() {
        found.push(Asset {
            source:name.to_string(),source_label:display_source.clone(),label:format!("Terrain / decoration #{i:03} [TMD {}]",d.env_slot),
            key:format!("cell:{name}|terrain:{i}|mesh:{}|anchor:{},{}|pos:{},{},{}",
                d.env_slot,d.anchor.0,d.anchor.1,d.world_x,d.world_y,d.world_z),
            alias_key:visual_keys.get(d.res_tmd).cloned().flatten(),kind:Kind::Terrain(i),mesh_fingerprint:mesh_hash(&scene,d.res_tmd),
        });
    }
    for (slot,&res_tmd) in scene.env_tmds.iter().enumerate() {
        found.push(Asset {
            source:name.to_string(),source_label:display_source.clone(),label:format!("Reusable original TMD #{slot:03} / resource {res_tmd}"),
            key:format!("cell:{name}|mesh:{slot}|resource:{res_tmd}"),
            alias_key:visual_keys.get(res_tmd).cloned().flatten(),kind:Kind::Mesh(slot),mesh_fingerprint:mesh_hash(&scene,res_tmd),
        });
    }
    // Index original scene actors (including conditionally hidden ones) but
    // never manufacture their MAN scripting/animation in a foreign cell.
    let world=world_view::build_world(&scene,index);
    for npc in world.npcs.iter().filter(|n|!n.is_copy) {
        found.push(Asset {
            source:name.to_string(),source_label:display_source.clone(),
            label:format!("Field actor MAN #{} / TMD {} / ANM {}{}",
                npc.record,npc.model_index,npc.anim_id,
                if npc.conditional {" [conditional]"} else {""}),
            key:format!("cell:{name}|npc:man:{}",npc.record),
            alias_key:world.naming.assets.get(&(false,npc.key)).cloned(),kind:Kind::Npc(npc.record),mesh_fingerprint:None,
        });
    }
    Ok(found)
}

pub(super) fn scan_disc(index:Arc<ProtIndex>, names:Vec<String>, tx:Sender<Batch>) {
    for name in names {
        let (assets,failed)=match catalog_one(&index,&name) {
            Ok(items)=>(items,false),Err(_)=>(Vec::new(),true),
        };
        if tx.send(Batch{assets,failed}).is_err(){break;}
    }
}

/// Only same-cell assets are local editor sources. Identical TMD bytes do not
/// establish identical TIM/CLUT contents, placement transforms or collision.
/// Foreign assets must retain their donor identity through insert_foreign;
/// verified native resource reuse belongs exclusively to export preparation.
pub(super) fn local_source(scene:&AssembledScene,asset:&Asset)->Option<EditorCatalogItem> {
    local_source_in(&scene.name, asset, scene.placements.len(), scene.terrain.len(), scene.env_tmds.len())
}

fn local_source_in(name:&str, asset:&Asset, placements:usize, terrain:usize, meshes:usize)
    ->Option<EditorCatalogItem> {
    if name != asset.source { return None; }
    match asset.kind {
        Kind::Placement(i)=>(i<placements).then_some(EditorCatalogItem::Draw(false,i)),
        Kind::Terrain(i)=>(i<terrain).then_some(EditorCatalogItem::Draw(true,i)),
        Kind::Mesh(i)=>(i<meshes).then_some(EditorCatalogItem::Mesh(i)),
        Kind::Npc(record)=>Some(EditorCatalogItem::Draw(false,
            world_view::NPC_KEY_BASE.checked_add(record)?)),
    }
}

/// Identifications are from data/gamedata/scenes.toml, cross-checked against
/// retail CDNAME and in-game scene banners. Not inferred from file prefixes.
/// All field scenes remain browsable, including areas not marked as towns.
pub(super) fn is_verified_town(source: &str) -> bool {
    matches!(source, "town01" | "town0b" | "town0c" | "izumi" | "bylon" | "retock" | "geremi" | "balden" | "conc" | "rayman" | "ropeway" | "station" | "retockin" | "balden2" | "rayman2" | "ropeway2" | "town0d" | "son" | "doman" | "bubu1" | "bubu2" | "town0e")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn foreign_geometry_fingerprint_never_substitutes_a_local_source() {
        let mut asset=Asset { source:"donor".into(), source_label:"donor".into(),
            label:"object".into(), key:"cell:donor|placed:0".into(),
            alias_key:None,kind:Kind::Placement(0), mesh_fingerprint:fingerprint(&[1,2,3]) };
        // Geometry equality cannot prove equality of the external TIM/CLUT,
        // native placement record or collision resource. Every foreign kind
        // must use the donor-aware path even with a valid matching fingerprint.
        for kind in [Kind::Placement(0),Kind::Terrain(0),Kind::Mesh(0),Kind::Npc(1)] {
            asset.kind=kind;
            assert!(local_source_in("destination",&asset,1,1,1).is_none());
            assert!(local_source_in("donor",&asset,1,1,1).is_some());
        }
        asset.kind=Kind::Placement(1);
        assert!(local_source_in("donor",&asset,1,1,1).is_none());
    }

    #[test]
    fn fingerprint_is_stable_and_empty_is_rejected(){
        assert_eq!(fingerprint(&[1,2,3]),fingerprint(&[1,2,3]));
        assert_ne!(fingerprint(&[1,2,3]),fingerprint(&[3,2,1]));
        assert_eq!(fingerprint(&[]),None);
    }
    #[test]
    fn labels_distinguish_provenance(){
        let a=Asset{source:"town01".into(),source_label:"Rim Elm".into(),label:"Unidentified world object #004".into(),
            key:"cell:town01|placed:4".into(),alias_key:None,kind:Kind::Placement(4),mesh_fingerprint:Some(5)};
        assert_eq!(a.kind_label(),"MAP object");
        assert_ne!(a.source,"road01");
        assert_eq!(a.group(),0);
    }
}
