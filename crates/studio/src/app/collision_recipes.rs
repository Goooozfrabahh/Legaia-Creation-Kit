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
    /// Automatically proposed geometry; never verified native collision.
    MeshDerived { note: String },
    /// A native observation is retained for provenance, but does not prove
    /// exclusive wall ownership and cannot authorize wall compilation.
    NativeObserved { evidence: String },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Recipe {
    pub version: u32,
    pub source: SourceProvenance,
    pub evidence: Evidence,
    /// Absolute source-grid sub-cells at the original object transform.
    /// No geometry-derived candidates are accepted implicitly.
    pub walls: Vec<[i32; 2]>,
    /// Explicit authored open-space requirements, never permission to clear
    /// native walls or another instance's contribution. Older V1 files omit it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub nonblocking: Vec<[i32; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generated: Option<super::collision_generation::GeneratedCollision>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
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
    /// Reuse exact immutable geometry definitions, never native wall ownership.
    pub generated_bases: BTreeMap<String,super::collision_generation::GeneratedCollision>,
    pub foreign_sources: BTreeMap<u64,SourceProvenance>,
    pub mesh_recipes: BTreeMap<u64,super::collision_generation::MeshOnlyRecipe>,
    /// Only set on the transactional restoration clone. Saved definitions
    /// replace creation defaults; missing recipes retain deliberate absence.
    pub restoring: bool,
}

impl Recipe {
    pub fn validate(&self) -> Result<(), String> {
        if self.version != 1 { return Err("Unsupported collision recipe version".into()); }
        if let Some(generated)=&self.generated {generated.validate()?;}
        let source=&self.source;
        let id=&source.object;
        let hash_ok=|s:&str|s.len()==64 && s.bytes().all(|b|b.is_ascii_hexdigit());
        if id.scene.is_empty() || id.scene.len()>128 || id.scene.chars().any(char::is_control) ||
            id.cell.iter().any(|&v|v>=128) || id.descriptor_id>=512 ||
            !hash_ok(&source.map_sha256) || source.man_sha256.as_deref().is_some_and(|s|!hash_ok(s)) {
            return Err("Invalid collision source provenance".into());
        }
        let note=match &self.evidence {Evidence::UserAuthored{note}|Evidence::MeshDerived{note}=>note, Evidence::NativeObserved{evidence}=>evidence};
        if note.trim().is_empty() || note.len()>1024 || note.chars().any(char::is_control) {
            return Err("Collision evidence must contain 1–1024 printable bytes".into());
        }
        if self.walls.len()+self.nonblocking.len()>65536 {return Err("Collision recipe exceeds native grid capacity".into());}
        let mut seen=BTreeSet::new();
        for &cell in self.walls.iter().chain(&self.nonblocking) {
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
        if let Some(uid)=draw.editor_uid {if let Some(source)=self.foreign_sources.get(&uid) {return Some(source);}}
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
        if !self.sources.values().chain(self.foreign_sources.values()).any(|s|s==&assignment.recipe.source) {
            return Err("Collision recipe source is stale or unavailable".into());
        }
        if let ObjectId::Native(id)=&assignment.owner {
            if id!=&assignment.recipe.source.object {return Err("Native recipe owner/source mismatch".into());}
        }
        let mut assignment=assignment;
        assignment.recipe.walls.sort();
        assignment.recipe.nonblocking.sort();
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

/// Byte-exact additive MAP plan. This is a serialization primitive, NOT an
/// export permission or a claim that a recipe captures an object's collider.
/// Private fields prevent callers from supplying arbitrary wall/floor writes.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct WallAdditionPlan {
    source_sha256: String,
    output_sha256: String,
    writes: Vec<(usize, u8, u8)>,
}

impl WallAdditionPlan {
    pub fn compile(scene:&str, map_entry:u32, native:&[u8], contributions:&[Contribution<'_>])->Result<Self,String> {
        let output=compile(scene,map_entry,native,contributions)?;
        let mut writes=Vec::new();
        for (offset,(&before,&after)) in native.iter().zip(&output).enumerate() {
            if before==after {continue;}
            if !(0x4000..0x8000).contains(&offset) || before&15!=after&15 || before&after!=before {
                return Err("Collision compiler attempted a non-additive native write".into());
            }
            writes.push((offset,before,after));
        }
        let plan=Self{source_sha256:digest(native),output_sha256:digest(&output),writes};
        if plan.apply(native)?!=output {return Err("Collision patch readback differs from compilation".into());}
        Ok(plan)
    }

    /// Transactional application to the exact original MAP, never to a prior
    /// revision's output. No partial mutation escapes a stale-source failure.
    pub fn apply(&self,native:&[u8])->Result<Vec<u8>,String> {
        if native.len()!=0x12000 || digest(native)!=self.source_sha256 {
            return Err("Collision patch source MAP changed".into());
        }
        let mut output=native.to_vec();
        for &(offset,before,after) in &self.writes {
            if output[offset]!=before {return Err("Collision patch precondition failed".into());}
            output[offset]=after;
        }
        if digest(&output)!=self.output_sha256 {return Err("Collision patch readback hash mismatch".into());}
        Ok(output)
    }
}

impl super::world_view::WorldModel {
    /// Ctrl+D copies the selected instance's current recipe, including an
    /// intentional absence, rather than resurrecting its base's recipe.
    pub fn inherit_collision_recipe(&mut self,from:(bool,usize),to:(bool,usize))->Result<(),String> {
        let owner=self.collision.owner(self,from).ok_or("Collision source owner unavailable")?;
        let recipe=self.collision.recipes.iter().find(|a|a.owner==owner).map(|a|a.recipe.clone());
        if let Some(recipe)=recipe {self.set_collision_recipe(to,recipe)}
        else if matches!(owner,ObjectId::Native(_)) {self.ensure_automatic_collision(to)}
        else {self.remove_collision_recipe(to)}
    }
    pub fn set_collision_recipe(&mut self,key:(bool,usize),recipe:Recipe)->Result<(),String> {
        let source=self.collision.source(self,key).ok_or("Native object source unavailable for collision recipe")?;
        if source!=&recipe.source {return Err("Collision recipe target/source mismatch".into());}
        let owner=self.collision.owner(self,key).ok_or("Collision owner unavailable")?;
        if let ObjectId::Created(uid)=owner {if self.collision.mesh_recipes.contains_key(&uid) {
            return Err("Mesh-only and native collision recipes cannot share one owner".into());
        }}
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
        if !self.collision.mesh_recipes.is_empty() {return Err("Mesh-only collision requires verified destination MAP/collider dependencies before native compilation".into());}
        let mut contributions=Vec::new();
        for assignment in &self.collision.recipes {
            let draw=self.draws.iter().find(|d|self.collision.owner(self,(d.terrain,d.index)).as_ref()==Some(&assignment.owner))
                .ok_or("Collision owner missing from document")?;
            if draw.editor_uid.is_some_and(|uid|self.foreign.iter().any(|f|f.uid==uid)) {
                return Err("Foreign collision requires verified destination MAP and resource dependency resolution; donor provenance alone is insufficient".into());
            }
            if draw.hidden {continue;}
            contributions.push(Contribution{assignment,offset:draw.offset,rotation:draw.rotation,scale:draw.scale});
        }
        WallAdditionPlan::compile(scene,map_entry,map,&contributions)?.apply(map)
    }
}

/// Read-only authoring/preflight diagnostics. A successful additive compile
/// does not authorize object behavior, native collider or floor relocation.
pub(super) fn export_diagnostics(scene:&legaia_engine_scene::scene_assembly::AssembledScene,
    world:&super::world_view::WorldModel,index:&legaia_engine_scene::scene::ProtIndex)->String {
    use std::fmt::Write;
    let mut text=String::new();let mut sources=BTreeSet::new();
    writeln!(text,"Saved collision recipes: {}. Native export remains blocked until collision, ground and script dependency acceptance is established.",world.collision.recipes.len()).unwrap();
    for recipe in world.collision.mesh_recipes.values() {
        writeln!(text,"Owner UID {}: automatic mesh-only collision from {} / {} #{} / TMD {}. Destination MAP, native collider and script resource provenance unresolved.",recipe.uid,recipe.scene,recipe.kind,recipe.record,recipe.tmd_sha256).unwrap();
    }
    for assignment in &world.collision.recipes {
        let id=&assignment.recipe.source.object;
        writeln!(text,"Owner {:?}: {} / MAP {} / descriptor {} cell {:?}; {} explicit blocks, {} open requirements, generated mesh {}",
            assignment.owner,id.scene,id.map_entry,id.descriptor_id,id.cell,assignment.recipe.walls.len(),assignment.recipe.nonblocking.len(),assignment.recipe.generated.is_some()).unwrap();
        if let Some(draw)=world.draws.iter().find(|d|world.collision.owner(world,(d.terrain,d.index)).as_ref()==Some(&assignment.owner)) {
            if draw.editor_uid.is_some() {
                writeln!(text,"Copy collision intent: additive contribution only; original native walls remain protected. Unknown original wall ownership does not authorize copying those bits. Runtime actor collider, walkable surfaces and script-writer coverage still require independent acceptance.").unwrap();
            } else if draw.offset!=[0.0;3] || draw.rotation!=[0.0;3] || draw.scale!=[1.0;3] || draw.hidden {
                writeln!(text,"Original collision removal UNRESOLVED: an additive recipe cannot remove the retail footprint when an original moves, changes transform or is hidden. Exclusive/shared native wall ownership is required; leaving ghost collision is not preservation.").unwrap();
            }
        }
        if let Some(generated)=&assignment.recipe.generated {
            let (floors,steep,filtered)=generated.surface_counts();
            writeln!(text,"Mesh review candidates: {floors} floor, {steep} steep, {filtered} filtered faces; vertical review band {} PS1 units is heuristic, not verified actor height. Floor candidates are not compiled as walkable surfaces.",generated.band_height).unwrap();
        }
        if sources.insert(id.clone()) {
            let compiled=index.entry_bytes(id.map_entry).map_err(|e|e.to_string()).and_then(|map|world.compile_authored_collision(&id.scene,id.map_entry,&map)
                .map(|out|(map.iter().zip(&out).filter(|(a,b)|a!=b).count(),map.iter().zip(&out).map(|(&a,&b)|(b&!a).count_ones()).sum::<u32>())));
            match compiled {Ok((bytes,bits))=>{writeln!(text,"Additive wall planner/readback passed: {bytes} changed bytes, {bits} added wall bits; baseline walls and floor data retained. This is byte serialization evidence, not collision/surface acceptance.").unwrap();},
                Err(error)=>{writeln!(text,"Collision compilation BLOCKED: {error}").unwrap();}}
            if let Some(draw)=world.draws.iter().find(|d|world.collision.owner(world,(d.terrain,d.index)).as_ref()==Some(&assignment.owner)) {
                match super::native_dependency_report::report(scene,world,(draw.terrain,draw.index),index) {
                    Ok(report)=>text.push_str(&report),Err(error)=>{writeln!(text,"Native dependencies UNRESOLVED: {error}").unwrap();}
                }
            } else {writeln!(text,"Collision owner missing from document").unwrap();}
        }
    }
    text.push_str("Required acceptance: reviewed native collision/actor class; floor corners/LUT/kind-2 compatibility; all script writers/consumers and story variants; destination resource and native allocation limits. Mesh-derived floors are candidates only. No required behavior is omitted to permit export.");
    text
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
    let mut open=BTreeMap::<[i32;2],BTreeSet<ObjectId>>::new();
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
        if !matches!(recipe.evidence,Evidence::UserAuthored{..}|Evidence::MeshDerived{..}) {
            return Err("Native wall ownership is unresolved; an observation cannot authorize compilation".into());
        }
        if recipe.generated.is_none() && (c.rotation!=[0.0;3] || c.scale!=[1.0;3] || c.offset[1]!=0.0) {
            return Err("Authored wall preview supports X/Z translation only".into());
        }
        let step=|v:f32|->Result<i32,String>{
            if !v.is_finite() || v.abs()>32768.0 || v%64.0!=0.0 {
                return Err("Wall translation must be an exact multiple of 64 native units".into());
            } Ok((v/64.0) as i32)
        };
        let (dx,dz)=if recipe.walls.is_empty() && recipe.nonblocking.is_empty() && recipe.generated.is_some() {(0,0)}
            else {
                if c.rotation!=[0.0;3] || c.scale!=[1.0;3] || c.offset[1]!=0.0 {return Err("Manual grid corrections require unrotated, unscaled X/Z translation".into());}
                (step(c.offset[0])?,step(c.offset[2])?)
            };
        if let Some(generated)=&recipe.generated {
            let mut bits=generated.wall_cells(c.offset,c.rotation,c.scale)?;
            for &[x,z] in &recipe.nonblocking {bits.remove(&[x+dx,z+dz]);}
            for cell in bits {union.entry(cell).or_default().insert(assignment.owner.clone());}
        }
        for &[x,z] in &recipe.walls {
            let moved=[x+dx,z+dz];
            if moved.iter().any(|v|!(0..256).contains(v)) {return Err("Authored wall moved outside native grid".into());}
            union.entry(moved).or_default().insert(assignment.owner.clone());
        }
        for &[x,z] in &recipe.nonblocking {
            let moved=[x+dx,z+dz];
            if moved.iter().any(|v|!(0..256).contains(v)) {return Err("Nonblocking region moved outside native grid".into());}
            open.entry(moved).or_default().insert(assignment.owner.clone());
        }
    }
    for ([x,z],owners) in open {
        let offset=0x4000+(z as usize/2)*128+x as usize/2;
        let mask=1u8<<(4+(z as usize%2)*2+x as usize%2);
        if native[offset]&mask!=0 {
            return Err(format!("Nonblocking cell ({x},{z}) conflicts with protected native wall at MAP {offset:#x}, bit {mask:#04x}; requested by {owners:?}. Native ownership is unresolved"));
        }
        if let Some(blockers)=union.get(&[x,z]) {
            return Err(format!("Nonblocking cell ({x},{z}) conflicts with authored blocking contributors {blockers:?}; requested by {owners:?}"));
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
struct RecipeDraft {walls:String,nonblocking:String,note:String,error:Option<String>,replace_corrections:bool}

pub(super) fn generated_for(world:&super::world_view::WorldModel,key:(bool,usize))
    ->Option<&super::collision_generation::GeneratedCollision> {
    let owner=world.collision.owner(world,key);
    world.collision.recipes.iter().find(|a|Some(&a.owner)==owner.as_ref()).and_then(|a|a.recipe.generated.as_ref())
        .or_else(||world.at(key.0,key.1).and_then(|d|d.editor_uid).and_then(|uid|world.collision.mesh_recipes.get(&uid)).map(|r|&r.generated))
}

pub(super) fn generation_status(world:&super::world_view::WorldModel,key:(bool,usize))->String {
    if let Some(g)=generated_for(world,key) {
        let (floors,walls,filtered)=g.surface_counts();
        format!("Saved generated collision: {walls} steep faces, {floors} floor candidates, {filtered} filtered faces")
    } else if world.collision.owner(world,key).is_some_and(|owner|world.collision.recipes.iter().any(|a|a.owner==owner)) {
        "Saved explicit collision recipe; no generated geometry".into()
    } else {"No saved collision recipe (original object, intentional removal, or unavailable provenance)".into()}
}

/// Explicit regeneration action. Ordinary rendering, opening Properties and
/// duplication never regenerate over existing authored data.
pub(super) fn regeneration(world:&super::world_view::WorldModel,key:(bool,usize),replace_corrections:bool)->Result<Recipe,String> {
    let generated=world.automatic_collision_recipe(key)?;
    if !replace_corrections {
        if let Some(owner)=world.collision.owner(world,key) {
            if let Some(saved)=world.collision.recipes.iter().find(|a|a.owner==owner) {
                let mut next=saved.recipe.clone();next.generated=generated.generated;next.validate()?;return Ok(next);
            }
        }
    }
    Ok(generated)
}

/// Always-visible status and preview control in the right-click Properties
/// window. Resource availability affects editing, never panel visibility.
pub(super) fn show_properties(ui:&mut eframe::egui::Ui,world:&super::world_view::WorldModel,key:(bool,usize),
    index:Option<&legaia_engine_scene::scene::ProtIndex>,wireframe:&mut bool)->Option<Option<Recipe>> {
    ui.strong("Collision Properties (P1)");
    ui.label(generation_status(world,key));
    let generated=generated_for(world,key);
    ui.add_enabled(generated.is_some(),eframe::egui::Checkbox::new(wireframe,"Show this object's generated collision wireframe"))
        .on_hover_text("Saved generated geometry only; separate from the original MAP wall overlay. Visibility does not edit collision or move the camera.");
    if let Some(source)=world.collision.source(world,key) {
        ui.label(format!("Source: {} / MAP {} / descriptor {} / cell {:?}",source.object.scene,source.object.map_entry,source.object.descriptor_id,source.object.cell));
        ui.small(format!("MAP SHA-256 {}",source.map_sha256));
        let warning=world.collision.owner(world,key).and_then(|owner|{
            let draft_id=eframe::egui::Id::new(("lck_collision_recipe",format!("{owner:?}"),&source.map_sha256));
            ui.ctx().data_mut(|data|data.get_temp::<RecipeDraft>(draft_id)).and_then(|d|d.error)
        });
        if let Some(warning)=warning {
            ui.colored_label(eframe::egui::Color32::YELLOW,warning);
        }
    } else if let Some(recipe)=world.at(key.0,key.1).and_then(|d|d.editor_uid).and_then(|uid|world.collision.mesh_recipes.get(&uid)) {
        ui.label(format!("Source: {} / {} #{} / TMD {}",recipe.scene,recipe.kind,recipe.record,recipe.tmd_sha256));
    }
    if let Some(g)=generated {ui.small(&g.native_inspection);}
    ui.small("Generated collision is a review proposal. Native wall ownership, actor behavior and walkability remain unresolved; native export safeguards still apply.");
    let mut action=None;
    if let Some(index)=index {
        eframe::egui::CollapsingHeader::new("Collision authoring, regeneration and validation")
            .id_salt(("properties_collision_controls",format!("{:?}",world.collision.owner(world,key))))
            .show(ui,|ui|{action=show_editor(ui,world,key,index);});
    } else {ui.colored_label(eframe::egui::Color32::YELLOW,"Open the source library to enable manual collision editing and validation. Saved collision status is shown above.");}
    action
}

pub(super) fn parse_cells(text:&str)->Result<Vec<[i32;2]>,String> {
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
        if let Some(recipe)=world.at(key.0,key.1).and_then(|d|d.editor_uid).and_then(|uid|world.collision.mesh_recipes.get(&uid)) {
            let (floors,walls,decorative)=recipe.generated.surface_counts();
            ui.label(format!("Automatic donor mesh collision: {walls} steep, {floors} floor candidates, {decorative} filtered faces"));
            ui.small(format!("Source {} / {} #{}; TMD SHA-256 {}",recipe.scene,recipe.kind,recipe.record,recipe.tmd_sha256));
            ui.small("Transforms follow this instance. Native MAP/collider dependencies are unresolved; native export remains blocked.");
        }
        ui.small("Collision recipe authoring requires a verified native World Object source.");return None;
    };
    let Some(owner)=world.collision.owner(world,key) else {return None;};
    let saved=world.collision.recipes.iter().find(|a|a.owner==owner);
    let id=egui::Id::new(("lck_collision_recipe",format!("{owner:?}"),&source.map_sha256));
    let from_saved=||RecipeDraft{
        walls:saved.map(|a|a.recipe.walls.iter().map(|c|format!("{},{}",c[0],c[1])).collect::<Vec<_>>().join("\n")).unwrap_or_default(),
        nonblocking:saved.map(|a|a.recipe.nonblocking.iter().map(|c|format!("{},{}",c[0],c[1])).collect::<Vec<_>>().join("\n")).unwrap_or_default(),
        note:saved.map(|a|match &a.recipe.evidence {Evidence::UserAuthored{note}|Evidence::MeshDerived{note}=>note.clone(),Evidence::NativeObserved{evidence}=>evidence.clone()})
            .unwrap_or_else(||"Authored in LCK; no native ownership claimed".into()),error:None,replace_corrections:false,
    };
    let mut draft=ui.ctx().data_mut(|data|data.get_temp::<RecipeDraft>(id)).unwrap_or_else(from_saved);
    ui.label(format!("Stable owner: {owner:?}"));
    if let Some(generated)=saved.and_then(|a|a.recipe.generated.as_ref()) {
        let (floors,walls,decorative)=generated.surface_counts();
        ui.label(format!("Automatic mesh collision: {walls} steep faces, {floors} floor candidates, {decorative} tiny/degenerate faces filtered"));
        ui.small(&generated.native_inspection);
        ui.small(format!("Review band {} units; minimum face area {}. Floor candidates are not verified walkable. Mesh SHA-256 {}",generated.band_height,generated.min_face_area,generated.mesh_sha256));
    }
    ui.small("Explicit authored wall additions • in-memory only. Native walls remain unchanged. No walkable-surface or actor-collider authoring.");
    super::collision_authoring::show(ui,world,key,source,index,&mut draft.walls,&mut draft.nonblocking);
    ui.label("Native wall sub-cells: one X,Z pair per line, each 0–255 (64 PS1 units). Coordinates refer to the source object's original pose.");
    ui.add(egui::TextEdit::multiline(&mut draft.walls).desired_rows(4).desired_width(f32::INFINITY));
    ui.label("Nonblocking requirements: X,Z pairs (cannot clear original or other-instance walls)");
    ui.add(egui::TextEdit::multiline(&mut draft.nonblocking).desired_rows(2).desired_width(f32::INFINITY));
    ui.label("Authorship / evidence note");ui.text_edit_singleline(&mut draft.note);
    let mut action=None;
    if saved.is_none() && ui.button("Generate default collision from selected object").clicked() {
        match world.automatic_collision_recipe(key) {
            Ok(recipe)=>{action=Some(Some(recipe));draft.error=None;},
            Err(error)=>draft.error=Some(error),
        }
    }
    if saved.is_some() {
        ui.checkbox(&mut draft.replace_corrections,"Replace saved manual corrections when regenerating");
        let label=if draft.replace_corrections {"Regenerate geometry and replace corrections"}else{"Regenerate geometry (keep saved corrections)"};
        if ui.button(label).clicked() {
            match regeneration(world,key,draft.replace_corrections) {
                Ok(recipe)=>{
                    if draft.replace_corrections {draft.walls.clear();draft.nonblocking.clear();draft.note=match &recipe.evidence {
                        Evidence::UserAuthored{note}|Evidence::MeshDerived{note}=>note.clone(),Evidence::NativeObserved{evidence}=>evidence.clone()};}
                    action=Some(Some(recipe));draft.error=None;
                },Err(error)=>draft.error=Some(error),
            }
        }
        ui.small("Regeneration changes saved geometry only. Unsaved grid/text drafts are applied separately with Save authored recipe.");
    }
    ui.horizontal_wrapped(|ui| {
        if ui.button("Save authored recipe").clicked() {
            let recipe=parse_cells(&draft.walls).and_then(|walls| {
                let nonblocking=parse_cells(&draft.nonblocking)?;
                let recipe=Recipe{version:1,source:source.clone(),evidence:Evidence::UserAuthored{note:draft.note.clone()},walls,nonblocking,
                    generated:saved.and_then(|a|a.recipe.generated.clone())};
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
pub(super) fn properties_fixture()->super::world_view::WorldModel {
    let (_,mut world)=super::ps1_export::tests::actor_fixture();world.npcs.clear();
    world.draws[0].index=0;world.draws[0].offset=[0.0;3];world.draws[0].vertex_end=3;
    let positions=vec![[256.0,0.0,256.0],[256.0,128.0,256.0],[256.0,0.0,384.0]];
    world.model=Some(crate::raster::Model{rest:positions.clone(),positions,indices:vec![0,1,2],
        object_ids:vec![0;3],uvs:vec![[0,0];3],cba_tsb:vec![[0,0];3],modulation:vec![[128;3];3],
        fill:vec![[0;3];3],textured:vec![0;3],center:[256.0,64.0,320.0],radius:512.0});
    let (_,assignment)=fixture();world.collision.sources.insert((false,0),assignment.recipe.source);
    world.duplicate_native((false,0),1,90,[64.0,0.0,0.0]).unwrap();world
}

#[cfg(test)]
pub(super) fn fixture()->(Vec<u8>,Assignment) {
    let mut map=vec![0;0x12000];map[0x4000]=0x15;
    let source=SourceProvenance{object:NativeObjectId{scene:"test".into(),map_entry:0,cell:[0,0],descriptor_id:0},
        map_sha256:digest(&map),man_sha256:None,descriptor:[0;32]};
    (map,Assignment{owner:ObjectId::Native(source.object.clone()),recipe:Recipe{
        version:1,source,evidence:Evidence::UserAuthored{note:"Explicit test authoring".into()},walls:vec![[2,2]],nonblocking:vec![],generated:None,
    }})
}

#[cfg(test)]
mod tests {
    use super::*;
    fn contribution(a:&Assignment,offset:[f32;3])->Contribution<'_>{Contribution{assignment:a,offset,rotation:[0.0;3],scale:[1.0;3]}}
    #[test]
    fn preflight_distinguishes_copy_additions_from_original_removal() {
        let mut world=properties_fixture();
        let (scene,_)=super::super::ps1_export::tests::actor_fixture();
        let index=legaia_engine_scene::scene::ProtIndex::from_bytes(legaia_disc_patch::disc::synth::synth_prot(&[]),None).unwrap();
        let copied=export_diagnostics(&scene,&world,&index);
        assert!(copied.contains("Copy collision intent: additive"));
        assert!(copied.contains("Floor candidates are not compiled as walkable surfaces"));
        assert!(!copied.contains("Original collision removal UNRESOLVED"));
        let recipe=world.collision.recipes[0].recipe.clone();
        world.set_collision_recipe((false,0),recipe).unwrap();
        assert!(world.set_offset(false,0,[64.0,0.0,0.0]));
        let before=super::super::scene_instances::serialize("test",&world);
        let moved=export_diagnostics(&scene,&world,&index);
        assert!(moved.contains("Original collision removal UNRESOLVED"));
        assert!(moved.contains("Copy collision intent: additive"));
        assert_eq!(super::super::scene_instances::serialize("test",&world),before);
    }
    #[test]
    fn regeneration_preserves_manual_data_unless_replacement_is_explicit() {
        let mut world=properties_fixture();let key=(false,1);
        let mut recipe=world.collision.recipes[0].recipe.clone();
        recipe.walls=vec![[12,12]];recipe.nonblocking=vec![[13,12]];
        recipe.evidence=Evidence::UserAuthored{note:"Reviewed doorway".into()};
        world.set_collision_recipe(key,recipe.clone()).unwrap();
        let before=super::super::scene_instances::serialize("test",&world);
        let regenerated=regeneration(&world,key,false).unwrap();
        assert_eq!(regenerated.walls,recipe.walls);assert_eq!(regenerated.nonblocking,recipe.nonblocking);
        assert_eq!(regenerated.evidence,recipe.evidence);
        let replaced=regeneration(&world,key,true).unwrap();
        assert!(replaced.walls.is_empty()&&replaced.nonblocking.is_empty());
        assert_eq!(super::super::scene_instances::serialize("test",&world),before,"Rendering/proposing regeneration must not mutate the document");
    }
    #[test]
    fn nonblocking_requirements_never_clear_native_or_another_owner() {
        let (map,mut a)=fixture();a.recipe.walls.clear();a.recipe.nonblocking=vec![[0,0]];
        assert!(compile("test",0,&map,&[contribution(&a,[0.0;3])]).unwrap_err().contains("protected native wall"));
        a.recipe.nonblocking=vec![[2,2]];
        let (_,mut b)=fixture();b.owner=ObjectId::Created(7);
        assert!(compile("test",0,&map,&[contribution(&a,[0.0;3]),contribution(&b,[0.0;3])]).unwrap_err().contains("authored blocking contributors"));
        assert_eq!(compile("test",0,&map,&[contribution(&a,[0.0;3])]).unwrap(),map);
        a.recipe.walls=vec![[2,2]];assert!(a.recipe.validate().is_err());
    }
    #[test]
    fn legacy_recipes_default_to_no_generated_or_nonblocking_intent() {
        let (_,a)=fixture();let json=serde_json::to_string(&a).unwrap();
        assert!(!json.contains("nonblocking")&&!json.contains("generated"));
        let parsed:Assignment=serde_json::from_str(&json).unwrap();assert_eq!(parsed,a);
    }
    #[test]
    fn sealed_wall_plan_roundtrips_and_rejects_stale_or_accumulated_maps() {
        let (map,a)=fixture();let mut b=a.clone();b.owner=ObjectId::Created(42);
        let plan=WallAdditionPlan::compile("test",0,&map,&[contribution(&a,[0.0;3]),contribution(&b,[64.0,0.0,0.0])]).unwrap();
        let reverse=WallAdditionPlan::compile("test",0,&map,&[contribution(&b,[64.0,0.0,0.0]),contribution(&a,[0.0;3])]).unwrap();
        assert_eq!(plan,reverse);
        assert_eq!(plan.writes,vec![(0x4081,0,0x30)]);
        let output=plan.apply(&map).unwrap();
        // Independent native-format reader, rather than inspecting only
        // the compiler's byte-offset arithmetic.
        let reader=legaia_engine_field::minigame_floor::FloorGrid::new(&output);
        let original=legaia_engine_field::minigame_floor::FloorGrid::new(&map);
        assert_eq!(reader.wall_nibble(1,1),3);
        for z in 0..128 {for x in 0..128 {
            assert_eq!(reader.height_index(x,z),original.height_index(x,z));
            assert_eq!(reader.cell(x,z),original.cell(x,z));
            assert_eq!(reader.wall_nibble(x,z)&original.wall_nibble(x,z),original.wall_nibble(x,z));
        }}
        assert!(plan.apply(&output).unwrap_err().contains("source MAP changed"));
        for offset in [0,0x4000,0x8000,0x10000] {
            let mut stale=map.clone();stale[offset]^=1;
            let before=stale.clone();assert!(plan.apply(&stale).is_err());assert_eq!(stale,before);
        }
        assert_eq!(WallAdditionPlan::compile("test",0,&map,&[]).unwrap().apply(&map).unwrap(),map);
    }
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
