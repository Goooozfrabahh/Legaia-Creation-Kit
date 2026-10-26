//! Explicit authored collision. This compiler is deliberately disconnected
//! from PS1 export. Native walls are an immutable baseline, never inferred
//! instance ownership. Coordinates are native 64-unit wall sub-cells.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashMap};

pub(super) fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct NativeObjectId {
    pub scene: String,
    pub map_entry: u32,
    pub cell: [u8; 2],
    pub descriptor_id: u16,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SourceProvenance {
    pub object: NativeObjectId,
    pub map_sha256: String,
    pub man_sha256: Option<String>,
    pub descriptor: [u8; 32],
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub(super) enum ObjectId {
    Native(NativeObjectId),
    Created(u64),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) enum Evidence {
    /// An explicit user decision, not recovered retail ownership.
    UserAuthored { note: String },
    /// A native observation is retained for provenance, but does not prove
    /// exclusive wall ownership and cannot authorize wall compilation.
    NativeObserved { evidence: String },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Recipe {
    pub version: u32,
    pub source: SourceProvenance,
    pub evidence: Evidence,
    /// Absolute source-grid sub-cells at the original object transform.
    /// No geometry-derived candidates are accepted implicitly.
    pub walls: Vec<[i32; 2]>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Assignment {
    pub owner: ObjectId,
    pub recipe: Recipe,
}

#[derive(Clone, Default)]
pub(super) struct CollisionState {
    /// Native source IDs keyed only by the current rendering adapter. Recipes
    /// never persist these transient indices.
    pub sources: HashMap<(bool, usize), SourceProvenance>,
    pub recipes: Vec<Assignment>,
}

impl Recipe {
    pub fn validate(&self) -> Result<(), String> {
        if self.version != 1 { return Err("Unsupported collision recipe version".into()); }
        let source=&self.source;
        let id=&source.object;
        let hash_ok=|s:&str|s.len()==64 && s.bytes().all(|b|b.is_ascii_hexdigit());
        if id.scene.is_empty() || id.scene.len()>128 || id.scene.chars().any(char::is_control) ||
            id.cell.iter().any(|&v|v>=128) || id.descriptor_id>=512 ||
            !hash_ok(&source.map_sha256) || source.man_sha256.as_deref().is_some_and(|s|!hash_ok(s)) {
            return Err("Invalid collision source provenance".into());
        }
        let note=match &self.evidence {Evidence::UserAuthored{note}=>note, Evidence::NativeObserved{evidence}=>evidence};
        if note.trim().is_empty() || note.len()>1024 || note.chars().any(char::is_control) {
            return Err("Collision evidence must contain 1–1024 printable bytes".into());
        }
        if self.walls.len()>65536 {return Err("Collision recipe exceeds native grid capacity".into());}
        let mut seen=BTreeSet::new();
        for &cell in &self.walls {
            if cell.iter().any(|v|!(0..256).contains(v)) || !seen.insert(cell) {
                return Err("Collision sub-cells must be unique and inside 0..255".into());
            }
        }
        Ok(())
    }
}

impl CollisionState {
    pub fn source<'a>(&'a self, world:&super::world_view::WorldModel, key:(bool,usize)) -> Option<&'a SourceProvenance> {
        let draw=world.at(key.0,key.1)?;
        self.sources.get(&draw.source_ref.unwrap_or(key))
    }
    pub fn owner(&self, world:&super::world_view::WorldModel,key:(bool,usize))->Option<ObjectId> {
        let draw=world.at(key.0,key.1)?;
        if let Some(uid)=draw.editor_uid {Some(ObjectId::Created(uid))}
        else {Some(ObjectId::Native(self.sources.get(&key)?.object.clone()))}
    }
    pub fn set(&mut self, assignment:Assignment)->Result<(),String> {
        assignment.recipe.validate()?;
        if matches!(assignment.owner,ObjectId::Created(0)) {return Err("Zero collision owner UID".into());}
        if !self.sources.values().any(|s|s==&assignment.recipe.source) {
            return Err("Collision recipe source is stale or unavailable".into());
        }
        if let ObjectId::Native(id)=&assignment.owner {
            if id!=&assignment.recipe.source.object {return Err("Native recipe owner/source mismatch".into());}
        }
        let mut assignment=assignment;
        assignment.recipe.walls.sort();
        self.recipes.retain(|a|a.owner!=assignment.owner);
        self.recipes.push(assignment);
        self.recipes.sort_by(|a,b|a.owner.cmp(&b.owner));
        Ok(())
    }
    pub fn duplicate(&mut self, source:&ObjectId,uid:u64) {
        if let Some(original)=self.recipes.iter().find(|a|&a.owner==source).cloned() {
            let mut copy=original;copy.owner=ObjectId::Created(uid);
            self.recipes.push(copy);self.recipes.sort_by(|a,b|a.owner.cmp(&b.owner));
        }
    }
}

/// Bind source identities once from the exact loaded native resources. A
/// missing/undecodable source remains unavailable for collision authoring.
pub(super) fn source_inventory(scene:&legaia_engine_scene::scene_assembly::AssembledScene,
    index:&legaia_engine_scene::scene::ProtIndex)->Result<CollisionState,String> {
    use legaia_engine_scene::scene::Scene;
    let native=Scene::load(index,&scene.name).map_err(|e|e.to_string())?;
    let map_entry=native.field_map_index(index).ok_or("Scene has no native MAP")?;
    let map=index.entry_bytes(map_entry).map_err(|e|e.to_string())?;
    if map.len()!=0x12000 {return Err("Invalid native MAP for collision provenance".into());}
    let map_sha256=digest(&map);
    let man_sha256=native.field_man_payload(index).map_err(|e|e.to_string())?.map(|m|digest(&m));
    let mut state=CollisionState::default();
    for (i,draw) in scene.placements.iter().enumerate() {
        let (x,z)=draw.cell;
        if x>=128 || z>=128 {continue;}
        let grid=0x8000+(z as usize*128+x as usize)*2;
        let id=u16::from_le_bytes(map[grid..grid+2].try_into().unwrap())&511;
        let rec=legaia_scene_formats::field_objects::ObjectRecord::parse(&map,id as usize)
            .ok_or("Source descriptor unavailable")?;
        if !rec.is_placed() || legaia_scene_formats::field_objects::pack_mesh_index(id,&rec)!=Some(draw.env_slot as u16) ||
            legaia_scene_formats::field_objects::world_x(x,rec.x_off)!=draw.world_x ||
            legaia_scene_formats::field_objects::world_z(z,rec.z_off)!=draw.world_z ||
            [rec.rot_x,rec.rot_y,rec.rot_z]!=[draw.rot_x,draw.rot_y,draw.rot_z] {continue;}
        let mut descriptor=[0;32];descriptor.copy_from_slice(&map[id as usize*32..id as usize*32+32]);
        state.sources.insert((false,i),SourceProvenance{
            object:NativeObjectId{scene:scene.name.clone(),map_entry,cell:[x,z],descriptor_id:id},
            map_sha256:map_sha256.clone(),man_sha256:man_sha256.clone(),descriptor,
        });
    }
    Ok(state)
}

pub(super) struct Contribution<'a> {
    pub assignment: &'a Assignment,
    pub offset: [f32;3],
    pub rotation: [f32;3],
    pub scale: [f32;3],
}

impl super::world_view::WorldModel {
    /// Ctrl+D copies the selected instance's current recipe, including an
    /// intentional absence, rather than resurrecting its base's recipe.
    pub fn inherit_collision_recipe(&mut self,from:(bool,usize),to:(bool,usize))->Result<(),String> {
        let owner=self.collision.owner(self,from).ok_or("Collision source owner unavailable")?;
        let recipe=self.collision.recipes.iter().find(|a|a.owner==owner).map(|a|a.recipe.clone());
        if let Some(recipe)=recipe {self.set_collision_recipe(to,recipe)} else {self.remove_collision_recipe(to)}
    }
    pub fn set_collision_recipe(&mut self,key:(bool,usize),recipe:Recipe)->Result<(),String> {
        let source=self.collision.source(self,key).ok_or("Native object source unavailable for collision recipe")?;
        if source!=&recipe.source {return Err("Collision recipe target/source mismatch".into());}
        let owner=self.collision.owner(self,key).ok_or("Collision owner unavailable")?;
        let scene=recipe.source.object.scene.clone();
        let mut proposed=self.collision.clone();
        proposed.set(Assignment{owner,recipe})?;
        let previous=std::mem::replace(&mut self.collision,proposed);
        if super::scene_instances::serialize(&scene,self).len()>2_000_000 {
            self.collision=previous;return Err("Collision recipe exceeds scene file safety limit".into());
        }
        Ok(())
    }
    pub fn remove_collision_recipe(&mut self,key:(bool,usize))->Result<(),String> {
        let owner=self.collision.owner(self,key).ok_or("Collision owner unavailable")?;
        self.collision.recipes.retain(|a|a.owner!=owner);Ok(())
    }
    pub fn compile_authored_collision(&self,scene:&str,map_entry:u32,map:&[u8])->Result<Vec<u8>,String> {
        let mut contributions=Vec::new();
        for assignment in &self.collision.recipes {
            let draw=self.draws.iter().find(|d|self.collision.owner(self,(d.terrain,d.index)).as_ref()==Some(&assignment.owner))
                .ok_or("Collision owner missing from document")?;
            if draw.hidden {continue;}
            contributions.push(Contribution{assignment,offset:draw.offset,rotation:draw.rotation,scale:draw.scale});
        }
        compile(scene,map_entry,map,&contributions)
    }
}

/// Pure, additive, in-memory compilation. Recompute from the immutable native
/// baseline on every revision: moving/removing an authored contribution cannot
/// erase native bits or leave a previous authored footprint behind.
pub(super) fn compile(scene:&str,map_entry:u32,native:&[u8],contributions:&[Contribution<'_>])
    ->Result<Vec<u8>,String> {
    if native.len()!=0x12000 {return Err("Expected complete native MAP".into());}
    let fingerprint=digest(native);
    let mut owners=BTreeSet::new();
    let mut union=BTreeMap::<[i32;2],BTreeSet<ObjectId>>::new();
    for c in contributions {
        let assignment=c.assignment;let recipe=&assignment.recipe;
        recipe.validate()?;
        if !owners.insert(assignment.owner.clone()) {return Err("Duplicate collision owner".into());}
        if matches!(assignment.owner,ObjectId::Created(0)) {return Err("Zero collision owner UID".into());}
        if let ObjectId::Native(id)=&assignment.owner {
            if id!=&recipe.source.object {return Err("Collision owner/source mismatch".into());}
        }
        let source=&recipe.source;let id=&source.object;
        if id.scene!=scene || id.map_entry!=map_entry || source.map_sha256!=fingerprint {
            return Err("Collision recipe source MAP is stale or belongs to another scene".into());
        }
        let record=id.descriptor_id as usize*32;
        let grid=0x8000+(id.cell[1] as usize*128+id.cell[0] as usize)*2;
        if native[record..record+32]!=source.descriptor ||
            u16::from_le_bytes(native[grid..grid+2].try_into().unwrap())&511!=id.descriptor_id {
            return Err("Collision source descriptor/reference changed".into());
        }
        if !matches!(recipe.evidence,Evidence::UserAuthored{..}) {
            return Err("Native wall ownership is unresolved; an observation cannot authorize compilation".into());
        }
        if c.rotation!=[0.0;3] || c.scale!=[1.0;3] || c.offset[1]!=0.0 {
            return Err("Authored wall preview supports X/Z translation only".into());
        }
        let step=|v:f32|->Result<i32,String>{
            if !v.is_finite() || v.abs()>32768.0 || v%64.0!=0.0 {
                return Err("Wall translation must be an exact multiple of 64 native units".into());
            } Ok((v/64.0) as i32)
        };
        let dx=step(c.offset[0])?;let dz=step(c.offset[2])?;
        for &[x,z] in &recipe.walls {
            let moved=[x+dx,z+dz];
            if moved.iter().any(|v|!(0..256).contains(v)) {return Err("Authored wall moved outside native grid".into());}
            union.entry(moved).or_default().insert(assignment.owner.clone());
        }
    }
    let mut output=native.to_vec();
    for ([x,z],_) in union {
        let offset=0x4000+(z as usize/2)*128+x as usize/2;
        let bit=4+(z as usize%2)*2+x as usize%2;
        output[offset]|=1u8<<bit;
    }
    Ok(output)
}

#[derive(Clone,Default)]
struct RecipeDraft {walls:String,note:String,error:Option<String>}

fn parse_cells(text:&str)->Result<Vec<[i32;2]>,String> {
    if text.len()>1_000_000 {return Err("Wall input exceeds limit".into());}
    text.lines().filter(|s|!s.trim().is_empty()).map(|line| {
        let values=line.split(',').map(str::trim).map(|n|n.parse::<i32>().map_err(|_|"Enter one X,Z integer pair per line".to_string())).collect::<Result<Vec<_>,_>>()?;
        if values.len()!=2 {return Err("Enter one X,Z pair per line".into());}
        Ok([values[0],values[1]])
    }).collect()
}

/// Explicit authoring UI. Returns intent for the existing scene-history path;
/// it never mutates the live document while panels hold shared world borrows.
pub(super) fn show_editor(ui:&mut eframe::egui::Ui,world:&super::world_view::WorldModel,
    key:(bool,usize),index:&legaia_engine_scene::scene::ProtIndex)->Option<Option<Recipe>> {
    use eframe::egui;
    let Some(source)=world.collision.source(world,key) else {
        ui.small("Collision recipe authoring requires a verified native World Object source.");return None;
    };
    let Some(owner)=world.collision.owner(world,key) else {return None;};
    let saved=world.collision.recipes.iter().find(|a|a.owner==owner);
    let id=egui::Id::new(("lck_collision_recipe",format!("{owner:?}"),&source.map_sha256));
    let from_saved=||RecipeDraft{
        walls:saved.map(|a|a.recipe.walls.iter().map(|c|format!("{},{}",c[0],c[1])).collect::<Vec<_>>().join("\n")).unwrap_or_default(),
        note:saved.map(|a|match &a.recipe.evidence {Evidence::UserAuthored{note}=>note.clone(),Evidence::NativeObserved{evidence}=>evidence.clone()})
            .unwrap_or_else(||"Authored in LCK; no native ownership claimed".into()),error:None,
    };
    let mut draft=ui.ctx().data_mut(|data|data.get_temp::<RecipeDraft>(id)).unwrap_or_else(from_saved);
    ui.label(format!("Stable owner: {owner:?}"));
    ui.small("Explicit authored wall additions • in-memory only. Native walls remain unchanged. No walkable-surface or actor-collider authoring.");
    ui.label("Native wall sub-cells: one X,Z pair per line, each 0–255 (64 PS1 units). Coordinates refer to the source object's original pose.");
    ui.add(egui::TextEdit::multiline(&mut draft.walls).desired_rows(4).desired_width(f32::INFINITY));
    ui.label("Authorship / evidence note");ui.text_edit_singleline(&mut draft.note);
    let mut action=None;
    ui.horizontal_wrapped(|ui| {
        if ui.button("Save authored recipe").clicked() {
            let recipe=parse_cells(&draft.walls).and_then(|walls| {
                let recipe=Recipe{version:1,source:source.clone(),evidence:Evidence::UserAuthored{note:draft.note.clone()},walls};
                recipe.validate()?;Ok(recipe)
            });
            match recipe {Ok(recipe)=>{action=Some(Some(recipe));draft.error=None;},Err(error)=>draft.error=Some(error)}
        }
        if ui.add_enabled(saved.is_some(),egui::Button::new("Remove recipe")).clicked() {action=Some(None);draft=RecipeDraft::default();}
        if ui.button("Reload saved recipe").clicked() {draft=from_saved();}
        if ui.button("Validate saved contributions in memory").clicked() {
            draft.error=Some(match index.entry_bytes(source.object.map_entry).map_err(|e|e.to_string())
                .and_then(|map|world.compile_authored_collision(&source.object.scene,source.object.map_entry,&map)
                    .map(|compiled|map.iter().zip(&compiled).filter(|(a,b)|a!=b).count())) {
                    Ok(count)=>format!("In-memory validation passed: {count} changed MAP wall bytes. No IMG written; native export remains blocked for recipes."),
                    Err(error)=>format!("In-memory validation failed: {error}"),
                });
        }
    });
    if let Some(error)=&draft.error {ui.label(error);}
    ui.ctx().data_mut(|data|data.insert_temp(id,draft));
    action
}

#[cfg(test)]
pub(super) fn fixture()->(Vec<u8>,Assignment) {
    let mut map=vec![0;0x12000];map[0x4000]=0x15;
    let source=SourceProvenance{object:NativeObjectId{scene:"test".into(),map_entry:0,cell:[0,0],descriptor_id:0},
        map_sha256:digest(&map),man_sha256:None,descriptor:[0;32]};
    (map,Assignment{owner:ObjectId::Native(source.object.clone()),recipe:Recipe{
        version:1,source,evidence:Evidence::UserAuthored{note:"Explicit test authoring".into()},walls:vec![[2,2]],
    }})
}

#[cfg(test)]
mod tests {
    use super::*;
    fn contribution(a:&Assignment,offset:[f32;3])->Contribution<'_>{Contribution{assignment:a,offset,rotation:[0.0;3],scale:[1.0;3]}}
    #[test]
    fn exact_subcell_bits_and_grid_edges_preserve_native_floor() {
        let (map,mut a)=fixture();
        a.recipe.walls=vec![[0,0],[1,0],[0,1],[1,1],[255,255]];
        let result=compile("test",0,&map,&[contribution(&a,[0.0;3])]).unwrap();
        assert_eq!(result[0x4000],0xf5);assert_eq!(result[0x7fff],0x80);
        // An authored contribution sharing a native bit cannot claim it. A
        // subsequent translation leaves that original native bit untouched.
        a.recipe.walls=vec![[0,0]];
        let moved=compile("test",0,&map,&[contribution(&a,[128.0,0.0,0.0])]).unwrap();
        assert_eq!(moved[0x4000],0x15);assert_eq!(moved[0x4001],0x10);
        assert!(compile("test",0,&map,&[contribution(&a,[-64.0,0.0,0.0])]).is_err());
        a.recipe.walls=vec![[0,0],[0,0]];
        assert!(a.recipe.validate().is_err());
    }
    #[test]
    fn duplicates_overlap_and_move_without_erasing_native_or_other_owners() {
        let (map,a)=fixture();let mut b=a.clone();b.owner=ObjectId::Created(9);
        let first=compile("test",0,&map,&[contribution(&a,[0.0;3]),contribution(&b,[0.0;3])]).unwrap();
        let reversed=compile("test",0,&map,&[contribution(&b,[0.0;3]),contribution(&a,[0.0;3])]).unwrap();
        assert_eq!(first,reversed);assert_eq!(first[0x4081],0x10);
        let moved=compile("test",0,&map,&[contribution(&a,[0.0;3]),contribution(&b,[64.0,0.0,0.0])]).unwrap();
        assert_eq!(moved[0x4081],0x30);
        let deleted=compile("test",0,&map,&[contribution(&b,[128.0,0.0,0.0])]).unwrap();
        assert_eq!(deleted[0x4081],0);assert_eq!(deleted[0x4082],0x10);
        assert_eq!(deleted[0x4000],0x15);
        assert_eq!(&deleted[..0x4000],&map[..0x4000]);assert_eq!(&deleted[0x8000..],&map[0x8000..]);
        for i in 0x4000..0x8000 {assert_eq!(deleted[i]&15,map[i]&15);assert_eq!(deleted[i]&map[i],map[i]);}
    }
    #[test]
    fn stale_ambiguous_or_unrepresentable_contributions_fail() {
        let (map,a)=fixture();
        for offset in [[1.0,0.0,0.0],[f32::NAN,0.0,0.0],[-192.0,0.0,0.0],[16384.0,0.0,0.0],[0.0,1.0,0.0]] {
            assert!(compile("test",0,&map,&[contribution(&a,offset)]).is_err());
        }
        let mut b=a.clone();b.recipe.evidence=Evidence::NativeObserved{evidence:"Nearby native bit".into()};
        assert!(compile("test",0,&map,&[contribution(&b,[0.0;3])]).unwrap_err().contains("ownership"));
        let mut stale=map.clone();stale[0x4000]^=0x20;
        assert!(compile("test",0,&stale,&[contribution(&a,[0.0;3])]).is_err());
        assert!(compile("other",0,&map,&[contribution(&a,[0.0;3])]).is_err());
        assert!(compile("test",0,&map,&[contribution(&a,[0.0;3]),contribution(&a,[0.0;3])]).is_err());
        let mut c=contribution(&a,[0.0;3]);c.rotation=[0.0,1.0,0.0];
        assert!(compile("test",0,&map,&[c]).is_err());
    }
}
