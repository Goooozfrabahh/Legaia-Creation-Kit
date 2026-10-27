//! Common native import inventory. Resource discovery/reuse is not permission
//! to export: the existing adapters must serialize and validate every request.
use std::{collections::{BTreeMap,BTreeSet},sync::Arc};
use anyhow::{Result,Context,ensure};
use legaia_engine_scene::{scene::{ProtIndex,Scene},scene_assembly::AssembledScene};
use super::{world_view::WorldModel,collision_recipes::digest};

#[derive(Clone,Copy,Debug,PartialEq,Eq,PartialOrd,Ord)]
pub(super) enum Category { MapPlacement, TerrainTile, RawMesh, Actor }
#[derive(Clone,Debug,PartialEq,Eq,PartialOrd,Ord)]
pub(super) enum InstanceId { Created(u64), OriginalDraw(bool,usize) }
#[derive(Clone,Copy,Debug,PartialEq,Eq,PartialOrd,Ord)]
pub(super) enum ResourceKind { Tmd, Tim, TextureContext }
#[derive(Clone,Debug,PartialEq,Eq,PartialOrd,Ord)]
pub(super) struct ResourceId { pub kind:ResourceKind, pub sha256:String }
#[derive(Clone,Debug,PartialEq,Eq,PartialOrd,Ord)]
pub(super) struct Origin {
    pub scene:String,
    // Resolved/decompressed source location, NOT an address safe to patch
    // directly in a packed or compressed PROT entry.
    pub entry:Option<u32>, pub offset:Option<usize>,
}
#[derive(Clone,Debug,PartialEq,Eq)]
pub(super) struct Resource {
    pub payload:Option<Arc<[u8]>>,
    pub origins:BTreeSet<Origin>,
    /// Discovered in destination assembly, not proof of runtime pointer-table
    /// registration or available memory. The native adapter must validate use.
    pub resident:bool,
}
#[derive(Clone,Copy,Debug,PartialEq,Eq,PartialOrd,Ord)]
pub(super) enum Requirement {
    Geometry, TexturePagesCluts, PlacementAllocation, SpawnWindow,
    WallCollision, WalkableFloors, ActorAnimation, ScriptBindingsTriggers,
    ScenePackingAndMemory, Transform,
}
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub(super) enum Adapter { NativeMap, ResidentStaticBridge, NativeMan, Unsupported }
#[derive(Clone,Debug,PartialEq,Eq)]
pub(super) struct Request {
    pub id:InstanceId, pub source:String, pub category:Category,
    pub source_record:usize, pub adapter:Adapter,
    pub resources:BTreeSet<ResourceId>,
    pub requirements:BTreeSet<Requirement>,
}
#[derive(Clone,Debug,Default,PartialEq,Eq)]
pub(super) struct NativeAssetPlan {
    pub requests:BTreeMap<InstanceId,Request>,
    pub resources:BTreeMap<ResourceId,Resource>,
}
impl NativeAssetPlan {
    fn register(&mut self,kind:ResourceKind,bytes:&[u8],origin:Origin,resident:bool)->Result<ResourceId> {
        ensure!(!bytes.is_empty(),"Empty native resource {:?} from {}",kind,origin.scene);
        let id=ResourceId{kind,sha256:digest(bytes)};
        if let Some(resource)=self.resources.get_mut(&id) {
            ensure!(resource.payload.as_deref()==Some(bytes),"Native resource fingerprint collision");
            resource.origins.insert(origin);resource.resident|=resident;
        } else {
            self.resources.insert(id.clone(),Resource{payload:Some(Arc::from(bytes)),origins:BTreeSet::from([origin]),resident});
        }
        Ok(id)
    }
    fn texture_context(&mut self,scene:&AssembledScene,resident:bool)->ResourceId {
        let id=ResourceId{kind:ResourceKind::TextureContext,sha256:super::naming::material_key(&scene.res.vram)};
        let resource=self.resources.entry(id.clone()).or_insert_with(||Resource{payload:None,origins:BTreeSet::new(),resident:false});
        resource.origins.insert(Origin{scene:scene.name.clone(),entry:None,offset:None});resource.resident|=resident;id
    }
    fn add_request(&mut self,request:Request)->Result<()> {
        ensure!(!matches!(request.id,InstanceId::Created(0)),"Zero native export instance UID");
        ensure!(!self.requests.contains_key(&request.id),"Duplicate native export instance identity {:?}",request.id);
        ensure!(request.resources.iter().all(|id|self.resources.contains_key(id)),"Unresolved resource graph edge");
        self.requests.insert(request.id.clone(),request);Ok(())
    }
    /// Inventory is deterministic and intentionally has no `can_export` flag.
    /// Resident TMD/TIM bytes do not prove physics, binding or loader closure.
    pub fn summary(&self)->String {
        use std::fmt::Write;
        let mut out=format!("Native asset dependency inventory: {} independent requests, {} immutable resources. OriginalDraw addresses are plan-local; created UIDs retain persistent editor identity.\n",self.requests.len(),self.resources.len());
        for request in self.requests.values() {
            let resident=request.resources.iter().filter(|id|self.resources[*id].resident).count();
            writeln!(out,"{:?}: {}/{} {:?}, route {:?}, {}/{} exact destination-discovered resources; required {:?}",request.id,request.source,request.source_record,request.category,request.adapter,resident,request.resources.len(),request.requirements).unwrap();
            if request.adapter==Adapter::Unsupported {out.push_str("  BLOCKED: no native category adapter; scene packing/behavior or engine support is required.\n");}
            if request.resources.iter().any(|id|!self.resources[id].resident) {
                out.push_str("  Nonresident dependencies: exact resource reuse unavailable. Static bridge may prove decoded equivalence; otherwise native packing/VRAM relocation is required. No transplant is implied.\n");
            }
        }
        out.push_str("Inventory is not dependency closure. Native adapters must validate allocation, physics, bindings, native tables, packing/memory limits and full readback; mesh proposals never prove wall ownership or walkability.");out
    }
    pub fn validate_routes(&self)->Result<()> {
        let blocked=self.requests.values().filter(|r|r.adapter==Adapter::Unsupported)
            .map(|r|format!("{:?} {:?} from {} #{}",r.id,r.category,r.source,r.source_record)).collect::<Vec<_>>();
        ensure!(blocked.is_empty(),"Unsupported native asset categories: {}. No requested instance was omitted",blocked.join("; "));Ok(())
    }
}

fn route(category:Category,foreign:bool)->Adapter {
    match (category,foreign) {
        (Category::MapPlacement,false)=>Adapter::NativeMap,
        (Category::MapPlacement,true)=>Adapter::ResidentStaticBridge,
        (Category::Actor,false)=>Adapter::NativeMan,
        _=>Adapter::Unsupported,
    }
}
fn requirements(category:Category)->BTreeSet<Requirement> {
    use Requirement::*;
    let mut all=BTreeSet::from([Geometry,TexturePagesCluts,PlacementAllocation,SpawnWindow,WallCollision,WalkableFloors,ScriptBindingsTriggers,ScenePackingAndMemory,Transform]);
    // MAP placements can be bound/posed objects. Their structural category
    // cannot prove animation/state independence without native binding closure.
    if matches!(category,Category::Actor|Category::MapPlacement) {all.insert(ActorAnimation);}
    all
}
fn category(kind:&str)->Result<Category> {
    match kind {"placement"=>Ok(Category::MapPlacement),"terrain"=>Ok(Category::TerrainTile),"mesh"=>Ok(Category::RawMesh),"npc"=>Ok(Category::Actor),_=>anyhow::bail!("Unknown native repository category {kind}")}
}

fn register_scene_textures(plan:&mut NativeAssetPlan,scene:&AssembledScene,index:&ProtIndex,resident:bool)->Result<BTreeSet<ResourceId>> {
    let mut resources=BTreeSet::from([plan.texture_context(scene,resident)]);
    let native=Scene::load(index,&scene.name)?;
    for tim in legaia_engine_scene::scene_bundle::scene_tim_layout(&native)? {
        let entry=native.entries.iter().find(|e|e.idx==tim.entry_idx).context("TIM source entry absent")?;
        let end=tim.offset.checked_add(tim.byte_len).context("TIM source range overflow")?;
        let raw=entry.bytes.get(tim.offset..end).context("TIM source range truncated")?;
        resources.insert(plan.register(ResourceKind::Tim,raw,Origin{scene:scene.name.clone(),entry:Some(tim.entry_idx),offset:Some(tim.offset)},resident)?);
    }
    Ok(resources)
}

/// Load donor scenes at most once per plan, retain original resource bytes and
/// provenance, and share only byte-identical immutable payloads. No VRAM,
/// native record, scene resource table or script is written here.
pub(super) fn collect(scene:&AssembledScene,world:&WorldModel,index:&ProtIndex)->Result<NativeAssetPlan> {
    let mut plan=NativeAssetPlan::default();
    let mut scene_resources=BTreeMap::<String,BTreeSet<ResourceId>>::new();
    let mut donors=BTreeMap::new();
    let mut foreign_ids=BTreeSet::new();
    for foreign in &world.foreign {ensure!(foreign.uid!=0 && foreign_ids.insert(foreign.uid),"Duplicate or zero foreign UID {}",foreign.uid);}
    let mut destination_tmds=Vec::new();
    for tmd in &scene.res.tmds {
        destination_tmds.push(plan.register(ResourceKind::Tmd,&tmd.raw,Origin{scene:scene.name.clone(),entry:Some(tmd.entry_idx),offset:Some(tmd.offset)},true)?);
    }
    scene_resources.insert(scene.name.clone(),register_scene_textures(&mut plan,scene,index,true)?);
    // Inventory only requested instances, not every unchanged native object.
    for draw in &world.draws {
        let npc=world.npc((draw.terrain,draw.index));
        let changed=draw.editor_uid.is_some() || draw.hidden || draw.offset!=[0.0;3] || draw.rotation!=[0.0;3] || draw.scale!=[1.0;3]
            || npc.is_some_and(|n|n.model_index!=n.original_model_index || n.clip!=n.anim_id || n.frame!=0)
            || world.collision.owner(world,(draw.terrain,draw.index)).is_some_and(|id|world.collision.recipes.iter().any(|a|a.owner==id));
        if !changed {continue;}
        let id=draw.editor_uid.map(InstanceId::Created).unwrap_or(InstanceId::OriginalDraw(draw.terrain,draw.index));
        let foreign=draw.editor_uid.and_then(|uid|world.foreign.iter().find(|f|f.uid==uid));
        let source_key=draw.source_ref.unwrap_or((draw.terrain,draw.index));
        let (source,kind,record)=if let Some(f)=foreign {(f.source.as_str(),category(&f.kind)?,f.record)}
            else if let Some(n)=npc {(scene.name.as_str(),Category::Actor,n.record)}
            else {(scene.name.as_str(),if source_key.0 {Category::TerrainTile}else{Category::MapPlacement},source_key.1)};
        if source!=scene.name && !donors.contains_key(source) {
            donors.insert(source.to_owned(),legaia_engine_world_views::scene_staging::assemble_field_scene(index,source).map_err(anyhow::Error::msg).with_context(||format!("Resolve donor {source}"))?);
        }
        let original=if source==scene.name {scene}else{&donors[source]};
        let mut resources=BTreeSet::new();
        if kind==Category::Actor {
            // NPC model selectors belong to their own model pool; never treat
            // them as environment/resource-TMD indices.
            if let Some(n)=npc {
                let raw=n.raw_tmd.as_ref().context("Actor native geometry unavailable")?;
                resources.insert(plan.register(ResourceKind::Tmd,raw,Origin{scene:source.into(),entry:None,offset:None},true)?);
            } else {anyhow::bail!("Foreign actor geometry provenance unsupported");}
        } else {
            let resource=match kind {
                Category::MapPlacement=>original.placements.get(record).map(|p|p.res_tmd),
                Category::TerrainTile=>original.terrain.get(record).map(|p|p.res_tmd),
                Category::RawMesh=>original.env_tmds.get(record).copied(),_=>None,
            }.context("Native source resource selector missing")?;
            let tmd=original.res.tmds.get(resource).context("Native source TMD missing")?;
            if let Some(f)=foreign {ensure!(f.raw_tmd==tmd.raw,"Donor native TMD changed for UID {}",f.uid);}
            resources.insert(if source==scene.name {destination_tmds[resource].clone()}else{
                plan.register(ResourceKind::Tmd,&tmd.raw,Origin{scene:source.into(),entry:Some(tmd.entry_idx),offset:Some(tmd.offset)},false)?
            });
        }
        // Retain the scene-local TIM payload inventory as dependencies rather
        // than guessing which partial upload/CLUT is sufficient for this mesh.
        if !scene_resources.contains_key(source) {
            scene_resources.insert(source.into(),register_scene_textures(&mut plan,original,index,false)?);
        }
        resources.extend(scene_resources[source].iter().cloned());
        plan.add_request(Request{id,source:source.into(),category:kind,source_record:record,adapter:route(kind,foreign.is_some()),resources,requirements:requirements(kind)})?;
    }
    // Foreign metadata may not vanish merely because its renderer draw did.
    for foreign in &world.foreign {ensure!(plan.requests.contains_key(&InstanceId::Created(foreign.uid)),"Foreign UID {} has no independent export draw",foreign.uid);}
    Ok(plan)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn origin(scene:&str)->Origin {Origin{scene:scene.into(),entry:Some(7),offset:Some(16)}}
    #[test]
    fn exact_resources_share_but_instances_and_material_contexts_do_not_merge() {
        let mut plan=NativeAssetPlan::default();
        let a=plan.register(ResourceKind::Tmd,&[1,2,3],origin("town-a"),true).unwrap();
        let b=plan.register(ResourceKind::Tmd,&[1,2,3],origin("town-b"),false).unwrap();assert_eq!(a,b);
        let texture=plan.register(ResourceKind::Tim,&[1,2,3],origin("town-b"),false).unwrap();assert_ne!(a,texture);
        let palette=plan.register(ResourceKind::Tim,&[1,2,4],origin("town-b"),false).unwrap();assert_ne!(texture,palette);
        for uid in [17,18] {plan.add_request(Request{id:InstanceId::Created(uid),source:"town-b".into(),category:Category::MapPlacement,source_record:2,
            adapter:Adapter::ResidentStaticBridge,resources:BTreeSet::from([a.clone(),texture.clone()]),requirements:requirements(Category::MapPlacement)}).unwrap();}
        assert_eq!(plan.requests.len(),2);assert_eq!(plan.resources[&a].origins.len(),2);assert!(plan.resources[&a].resident);
        assert!(!plan.resources[&texture].resident);assert!(plan.summary().contains("VRAM relocation is required"));
        let duplicate=plan.requests[&InstanceId::Created(17)].clone();assert!(plan.add_request(duplicate).is_err());
    }
    #[test]
    fn category_adapters_fail_closed_without_a_native_writer() {
        for (category,local,foreign) in [(Category::MapPlacement,Adapter::NativeMap,Adapter::ResidentStaticBridge),
            (Category::TerrainTile,Adapter::Unsupported,Adapter::Unsupported),(Category::RawMesh,Adapter::Unsupported,Adapter::Unsupported),
            (Category::Actor,Adapter::NativeMan,Adapter::Unsupported)] {
            assert_eq!(route(category,false),local);assert_eq!(route(category,true),foreign);
            let request=Request{id:InstanceId::Created(9),source:"donor".into(),category,source_record:0,adapter:foreign,resources:BTreeSet::new(),requirements:requirements(category)};
            let mut plan=NativeAssetPlan::default();plan.add_request(request).unwrap();assert_eq!(plan.validate_routes().is_ok(),foreign!=Adapter::Unsupported);
            assert!(plan.requests[&InstanceId::Created(9)].requirements.contains(&Requirement::WalkableFloors));
            if matches!(category,Category::MapPlacement|Category::Actor) {assert!(plan.requests[&InstanceId::Created(9)].requirements.contains(&Requirement::ActorAnimation));}
        }
        assert!(category("invented").is_err());
    }
    #[test]
    fn texture_reuse_requires_upload_coverage_and_pixel_identity() {
        let (mut scene,_)=super::super::ps1_export::tests::actor_fixture();
        scene.res.vram=legaia_tim::Vram::new();
        let mut plan=NativeAssetPlan::default();
        let absent=plan.texture_context(&scene,true);
        scene.name="foreign-town".into();scene.res.vram.write_block(768,400,1,1,&[0,0]);
        let transparent=plan.texture_context(&scene,false);assert_ne!(absent,transparent);
        scene.res.vram.write_block(768,400,1,1,&[31,0]);
        let coloured=plan.texture_context(&scene,false);assert_ne!(transparent,coloured);
        assert!(!plan.resources[&coloured].resident);
        scene.name="another-town".into();let exact=plan.texture_context(&scene,false);assert_eq!(exact,coloured);
        assert_eq!(plan.resources[&coloured].origins.len(),2);
        // No fabricated VRAM allocation or TMD CBA/TSB rewrite is produced by
        // discovering a different context. Such relocation still needs a writer.
        assert!(plan.resources[&coloured].payload.is_none());
    }
}
