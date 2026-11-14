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
    pub floor_faces:Vec<super::floor_tiles::Face>,
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
        match self.kind {Kind::Placement(_)=>"MAP Base Form",Kind::Terrain(_)=>"terrain Base Form",
            Kind::Mesh(_)=>"TMD Base Form",Kind::Npc(_)=>"actor Base Form"}
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
pub(super) struct Batch {
    pub assets:Vec<Asset>,
    pub floors:Vec<super::native_floor_repository::Tile>,
    pub failed:bool,
}
fn fingerprint(raw:&[u8])->Option<u64> {
    if raw.is_empty(){return None;}
    let mut h=std::collections::hash_map::DefaultHasher::new();
    raw.hash(&mut h);
    Some(h.finish())
}
// LCK_R646D_VISUAL_FINGERPRINT
// Browser-only identity. No native resource or exporter changes.
fn r646d_visual_signature(
    model:&crate::raster::Model,
    vram:&legaia_tim::Vram,
)->u64 {
    let mut h=std::collections::hash_map::DefaultHasher::new();
    "LCK-VISUAL-V4".hash(&mut h);

    model.indices.hash(&mut h);
    model.rest.len().hash(&mut h);

    for p in &model.rest {
        for axis in 0..3 {
            (p[axis]-model.center[axis]).to_bits().hash(&mut h);
        }
    }

    model.textured.hash(&mut h);
    model.modulation.hash(&mut h);
    model.fill.hash(&mut h);

    for tri in model.indices.chunks_exact(3) {
        let ids=[
            tri[0] as usize,
            tri[1] as usize,
            tri[2] as usize
        ];

        if !ids.iter().all(|&i|
            model.textured.get(i)==Some(&1)
        ) {
            continue;
        }

        let Some([cba,tsb])=
            model.cba_tsb.get(ids[0]).copied()
        else {continue};

        let Some(a)=model.uvs.get(ids[0]).copied()
        else {continue};
        let Some(b)=model.uvs.get(ids[1]).copied()
        else {continue};
        let Some(c)=model.uvs.get(ids[2]).copied()
        else {continue};

        let uvs=[
            (a[0],a[1]),
            (b[0],b[1]),
            (c[0],c[1])
        ];

        // PrimTextureStatus::ok() is the confirmed enum API.
        vram.prim_texture_status(cba,tsb,&uvs)
            .ok().hash(&mut h);

        (tsb & 0x60).hash(&mut h);

        let a=(i32::from(a[0]),i32::from(a[1]));
        let b=(i32::from(b[0]),i32::from(b[1]));
        let c=(i32::from(c[0]),i32::from(c[1]));

        let min_u=a.0.min(b.0).min(c.0);
        let max_u=a.0.max(b.0).max(c.0);
        let min_v=a.1.min(b.1).min(c.1);
        let max_v=a.1.max(b.1).max(c.1);

        for p in [a,b,c] {
            (p.0-min_u).hash(&mut h);
            (p.1-min_v).hash(&mut h);
        }

        let edge=|x:(i32,i32),y:(i32,i32),p:(i32,i32)|{
            (y.0-x.0)*(p.1-x.1)-
            (y.1-x.1)*(p.0-x.0)
        };

        let area=edge(a,b,c);

        if area==0 {continue;}

        for v in min_v..=max_v {
            for u in min_u..=max_u {
                let p=(u,v);
                let x=edge(a,b,p);
                let y=edge(b,c,p);
                let z=edge(c,a,p);

                let inside=
                    (x>=0 && y>=0 && z>=0) ||
                    (x<=0 && y<=0 && z<=0);

                if !inside {continue;}

                (u-min_u).hash(&mut h);
                (v-min_v).hash(&mut h);

                crate::raster::texel(
                    vram,cba,tsb & 0x1FF,
                    u as usize,v as usize
                ).hash(&mut h);
            }
        }
    }

    h.finish()
}

fn mesh_hash(scene:&AssembledScene,index:usize)->Option<u64> {
    let bytes=&scene.res.tmds.get(index)?.raw;
    let model=crate::raster::Model::from_tmd(bytes)?;
    Some(r646d_visual_signature(&model,&scene.res.vram))
}
pub(super) fn catalog_one_with_floors(
    index:&ProtIndex,name:&str
)->Result<
    (Vec<Asset>,Vec<super::native_floor_repository::Tile>),
    String
> {
    let scene=legaia_engine_world_views::scene_staging::assemble_field_scene(index,name)?;
    let native_floors=super::native_floor_repository::index_scene(&scene);
    let display_source=crate::place_name(name).unwrap_or(name).to_owned();
    let visual_keys=super::naming::visual_keys(&scene);
    let descriptions=super::descriptive_names::for_scene(&scene,index,&visual_keys);
    let description=|resource:usize|visual_keys.get(resource).and_then(Option::as_ref)
        .and_then(|key|descriptions.get(key)).cloned().unwrap_or_else(||"Scene model".into());
    let mut found=Vec::with_capacity(scene.placements.len()+scene.terrain.len()+scene.env_tmds.len());

    // Reuse signatures for all placements of the same resource.
    let mut r646d_cache=
        std::collections::HashMap::<usize,Option<u64>>::new();

    let mut r646d_hash=|resource:usize|{
        *r646d_cache.entry(resource).or_insert_with(||
            mesh_hash(&scene,resource)
        )
    };
    for (i,d) in scene.placements.iter().enumerate() {
        found.push(Asset {
            source:name.to_string(),source_label:display_source.clone(),label:description(d.res_tmd),
            key:format!("cell:{name}|placed:{i}|mesh:{}|anchor:{},{}|pos:{},{},{}",
                d.env_slot,d.anchor.0,d.anchor.1,d.world_x,d.world_y,d.world_z),
            alias_key:visual_keys.get(d.res_tmd).cloned().flatten(),kind:Kind::Placement(i),mesh_fingerprint:r646d_hash(d.res_tmd),floor_faces:Vec::new(),
        });
    }
    for (i,d) in scene.terrain.iter().enumerate() {
        found.push(Asset {
            source:name.to_string(),source_label:display_source.clone(),label:description(d.res_tmd),
            key:format!("cell:{name}|terrain:{i}|mesh:{}|anchor:{},{}|pos:{},{},{}",
                d.env_slot,d.anchor.0,d.anchor.1,d.world_x,d.world_y,d.world_z),
            alias_key:visual_keys.get(d.res_tmd).cloned().flatten(),kind:Kind::Terrain(i),mesh_fingerprint:r646d_hash(d.res_tmd),floor_faces:Vec::new(),
        });
    }
    for (slot,&res_tmd) in scene.env_tmds.iter().enumerate() {
        found.push(Asset {
            source:name.to_string(),source_label:display_source.clone(),label:description(res_tmd),
            key:format!("cell:{name}|mesh:{slot}|resource:{res_tmd}"),
            alias_key:visual_keys.get(res_tmd).cloned().flatten(),kind:Kind::Mesh(slot),mesh_fingerprint:r646d_hash(res_tmd),floor_faces:scene.res.tmds.get(res_tmd).map(|t|super::floor_tiles::scan_faces(&t.tmd,&t.raw,&scene.res.vram)).unwrap_or_default(),
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
            alias_key:world.naming.assets.get(&(false,npc.key)).cloned(),kind:Kind::Npc(npc.record),mesh_fingerprint:None,floor_faces:Vec::new(),
        });
    }
    Ok((found,native_floors))
}

pub(super) fn catalog_one(index:&ProtIndex,name:&str)->Result<Vec<Asset>,String> {
    catalog_one_with_floors(index,name).map(|(assets,_)|assets)
}

// LCK_R646D_UNIQUE_WORLD_TERRAIN
// Deduplicates display rows only. Preserves complete donor catalog.
pub(super) fn r646d_unique_matches(
    assets:&[Asset],
    matched:&[bool],
    preferred:Option<usize>,
)->Vec<bool>{
    let mut visible=vec![false;assets.len()];

    // Category is part of the identity.
    let mut best=
        std::collections::HashMap::<(usize,u64),usize>::new();

    for (id,asset) in assets.iter().enumerate(){
        if !matched.get(id).copied().unwrap_or(false){
            continue;
        }

        if !matches!(
            asset.kind,Kind::Placement(_) | Kind::Terrain(_)
        ){
            // NPCs and reusable TMDs are not changed.
            visible[id]=true;
            continue;
        }

        let Some(signature)=asset.mesh_fingerprint else{
            // Never hide an asset whose appearance could not be read.
            visible[id]=true;
            continue;
        };

        let key=(asset.group(),signature);

        if let Some(previous)=best.get_mut(&key){
            if Some(id)==preferred ||
                (Some(*previous)!=preferred && id<*previous)
            {
                *previous=id;
            }
        }else{
            best.insert(key,id);
        }
    }

    for &id in best.values(){
        visible[id]=true;
    }

    visible
}

#[cfg(test)]
mod r646d_visual_tests {
    use super::*;

    fn item(kind:Kind,source:&str,hash:Option<u64>)->Asset{
        Asset{
            source:source.into(),
            source_label:source.into(),
            label:"Test asset".into(),
            key:format!("{source}:{kind:?}"),
            alias_key:None,
            kind,
            mesh_fingerprint:hash,
            floor_faces:Vec::new(),
        }
    }

    #[test]
    fn repeated_placements_across_story_variants_collapse(){
        let v=vec![
            item(Kind::Placement(0),"town01",Some(7)),
            item(Kind::Placement(1),"town01",Some(7)),
            item(Kind::Placement(2),"town0b",Some(7)),
        ];
        assert_eq!(
            r646d_unique_matches(&v,&[true,true,true],None),
            vec![true,false,false]
        );
        assert_eq!(v.len(),3);
    }

    #[test]
    fn objects_and_terrain_are_independent(){
        let v=vec![
            item(Kind::Placement(0),"town01",Some(7)),
            item(Kind::Terrain(0),"town01",Some(7)),
            item(Kind::Terrain(1),"town0b",Some(7)),
        ];
        assert_eq!(
            r646d_unique_matches(&v,&[true,true,true],None),
            vec![true,true,false]
        );
    }

    #[test]
    fn npc_mesh_and_unknown_visuals_are_preserved(){
        let v=vec![
            item(Kind::Npc(0),"a",Some(7)),
            item(Kind::Npc(1),"a",Some(7)),
            item(Kind::Mesh(0),"a",Some(7)),
            item(Kind::Mesh(1),"a",Some(7)),
            item(Kind::Placement(0),"a",None),
        ];
        assert_eq!(
            r646d_unique_matches(&v,&[true;5],None),
            vec![true;5]
        );
    }

    #[test]
    fn chosen_donor_and_source_search_are_preserved(){
        let v=vec![
            item(Kind::Placement(0),"a",Some(7)),
            item(Kind::Placement(0),"b",Some(7)),
        ];
        assert_eq!(
            r646d_unique_matches(&v,&[true,true],Some(1)),
            vec![false,true]
        );
        assert_eq!(
            r646d_unique_matches(&v,&[false,true],None),
            vec![false,true]
        );
    }

    #[test]
    fn changes_to_used_texture_pixels_change_identity(){
        let points=vec![
            [0.0,0.0,0.0],
            [1.0,0.0,0.0],
            [0.0,0.0,1.0]
        ];
        let model=crate::raster::Model{
            positions:points.clone(),
            rest:points,
            object_ids:vec![0;3],
            uvs:vec![[0,0],[1,0],[0,1]],
            cba_tsb:vec![[0,0x100];3],
            modulation:vec![[128;3];3],
            fill:vec![[0;3];3],
            textured:vec![1;3],
            indices:vec![0,1,2],
            center:[0.5,0.0,0.5],
            radius:1.0,
        };

        let mut a=legaia_tim::Vram::new();
        a.write_block(0,0,2,2,&[1,0,1,0,1,0,1,0]);
        let mut b=a.clone();
        b.write_block(0,0,1,1,&[2,0]);

        assert_ne!(
            r646d_visual_signature(&model,&a),
            r646d_visual_signature(&model,&b)
        );
    }
}
// LCK_R646A_UNIQUE_VISUAL_REPOSITORY
// This changes browser visibility only, never donor identity or export data.
fn r646_kind_rank(kind:Kind)->u8 {
    match kind {
        Kind::Placement(_)=>0,
        Kind::Terrain(_)=>1,
        Kind::Mesh(_)=>2,
        Kind::Npc(_)=>3,
    }
}

/// One representative per verified visual-v1 identity.
/// A source-specific jump or active selection can choose its own donor.
/// Models lacking a verified visual key remain independent.
pub(super) fn r646_unique_visual_matches(
    assets:&[Asset],
    matched:&[bool],
    preferred:Option<usize>,
)->Vec<bool>{
    let mut visible=vec![false;assets.len()];
    let mut best=std::collections::HashMap::<&str,usize>::new();

    for (id,asset) in assets.iter().enumerate() {
        if !matched.get(id).copied().unwrap_or(false) {
            continue;
        }

        // MAN actors have separate script/animation identities.
        if matches!(asset.kind,Kind::Npc(_)) {
            visible[id]=true;
            continue;
        }

        let Some(visual)=asset.alias_key.as_deref()
            .filter(|key|key.starts_with("visual-v1:"))
        else {
            visible[id]=true;
            continue;
        };

        if let Some(existing)=best.get_mut(visual) {
            let old=*existing;

            let new_rank=(
                Some(id)!=preferred,
                r646_kind_rank(asset.kind),
                id
            );

            let old_rank=(
                Some(old)!=preferred,
                r646_kind_rank(assets[old].kind),
                old
            );

            if new_rank<old_rank {
                *existing=id;
            }
        } else {
            best.insert(visual,id);
        }
    }

    for &id in best.values() {
        visible[id]=true;
    }

    visible
}
pub(super) fn scan_disc(index:Arc<ProtIndex>, names:Vec<String>, tx:Sender<Batch>) {
    for name in names {
        let (assets,floors,failed)=match catalog_one_with_floors(&index,&name) {
            Ok((assets,floors))=>(assets,floors,false),
            Err(_)=>(Vec::new(),Vec::new(),true),
        };
        if tx.send(Batch{assets,floors,failed}).is_err(){break;}
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
mod r646_unique_visual_tests {
    use super::*;

    fn sample(source:&str,kind:Kind,visual:Option<&str>)->Asset{
        Asset {
            source:source.into(),
            source_label:source.into(),
            label:"Test model".into(),
            key:format!("{source}:{kind:?}"),
            alias_key:visual.map(str::to_owned),
            kind,
            mesh_fingerprint:Some(123),
            floor_faces:Vec::new(),
        }
    }

    #[test]
    fn placement_terrain_and_mesh_share_one_visual_row(){
        let assets=vec![
            sample("town01",Kind::Mesh(0),Some("visual-v1:a:x")),
            sample("town0b",Kind::Placement(4),Some("visual-v1:a:x")),
            sample("town0c",Kind::Terrain(8),Some("visual-v1:a:x")),
            sample("town01",Kind::Mesh(1),Some("visual-v1:b:x")),
        ];

        let before=assets.len();
        let visible=r646_unique_visual_matches(
            &assets,&vec![true;before],None
        );

        assert_eq!(visible,vec![false,true,false,true]);
        assert_eq!(assets.len(),before);
    }

    #[test]
    fn identical_geometry_different_materials_stay_distinct(){
        let assets=vec![
            sample("a",Kind::Mesh(0),Some("visual-v1:geo:palette1")),
            sample("b",Kind::Mesh(0),Some("visual-v1:geo:palette2")),
        ];

        assert_eq!(
            r646_unique_visual_matches(&assets,&[true,true],None),
            vec![true,true]
        );
    }

    #[test]
    fn npc_and_unknown_visuals_are_not_discarded(){
        let assets=vec![
            sample("a",Kind::Npc(1),None),
            sample("a",Kind::Npc(2),None),
            sample("b",Kind::Mesh(1),None),
        ];

        assert_eq!(
            r646_unique_visual_matches(&assets,&[true,true,true],None),
            vec![true,true,true]
        );
    }

    #[test]
    fn source_search_and_focused_reference_remain_accessible(){
        let assets=vec![
            sample("town01",Kind::Placement(1),Some("visual-v1:same:m")),
            sample("town0b",Kind::Placement(9),Some("visual-v1:same:m")),
        ];

        assert_eq!(
            r646_unique_visual_matches(&assets,&[false,true],None),
            vec![false,true]
        );

        assert_eq!(
            r646_unique_visual_matches(&assets,&[true,true],Some(1)),
            vec![false,true]
        );
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn foreign_geometry_fingerprint_never_substitutes_a_local_source() {
        let mut asset=Asset { source:"donor".into(), source_label:"donor".into(),
            label:"object".into(), key:"cell:donor|placed:0".into(),
            alias_key:None,kind:Kind::Placement(0), mesh_fingerprint:fingerprint(&[1,2,3]),floor_faces:Vec::new() };
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
            key:"cell:town01|placed:4".into(),alias_key:None,kind:Kind::Placement(4),mesh_fingerprint:Some(5),floor_faces:Vec::new()};
        assert_eq!(a.kind_label(),"MAP Base Form");
        assert_ne!(a.source,"road01");
        assert_eq!(a.group(),0);
    }
}
