//! Read-only game-wide native evidence. No export permission is granted here.
//! Native addresses, decoded-container addresses and content identities are
//! separate; none is a transient viewport index or an inferred wall owner.
use std::collections::{BTreeMap,BTreeSet};
use anyhow::{Context,Result,ensure};
use serde::{Serialize,Deserialize};
use serde_json::{Value,json};
use legaia_engine_scene::{scene::{Scene,ProtIndex},model_bank::SceneModelBank};
use legaia_scene_formats::field_objects::{self,ObjectRecord};
use super::{collision_recipes::digest,native_script_dependencies::SceneInventory,
    ps1_export::native_dependencies::NativeBindings};

#[derive(Clone,Debug,Serialize,Deserialize,PartialEq,Eq,PartialOrd,Ord)]
pub(super) struct Address {
    /// `map:<entry>:<sha>`, `decoded-man:<sha>`, etc. Never an IMG offset.
    pub container:String,
    pub offset:usize,
    pub length:usize,
}
#[derive(Clone,Copy,Debug,Serialize,Deserialize,PartialEq,Eq)]
pub(super) enum Relationship {
    /// Sole consumer in the explicitly stated reference domain, NOT full
    /// runtime ownership, script closure or export authorization.
    ExclusiveReference,SharedReference,References,Affects,Unrelated,Unresolved,
}
#[derive(Clone,Debug,Serialize,Deserialize)]
pub(super) struct Dependency {
    pub target:String,
    pub relationship:Relationship,
    pub evidence:Vec<Address>,
    pub explanation:String,
}
#[derive(Clone,Debug,Serialize,Deserialize)]
pub(super) struct Record {
    pub kind:String,
    pub address:Option<Address>,
    pub fields:Value,
    pub dependencies:Vec<Dependency>,
}
#[derive(Default,Debug,Serialize,Deserialize)]
pub(super) struct CellInventory {
    pub schema:u32,
    pub scene:String,
    pub entry_range:[u32;2],
    pub records:BTreeMap<String,Record>,
    pub gaps:Vec<String>,
    /// Distinct variants are not merged by a name prefix or geometry hash.
    pub export_authorized:bool,
}
impl CellInventory {
    fn insert(&mut self,id:String,kind:&str,address:Option<Address>,fields:Value)->Result<()> {
        ensure!(!self.records.contains_key(&id),"Duplicate native identity {id}");
        self.records.insert(id,Record{kind:kind.into(),address,fields,dependencies:vec![]});Ok(())
    }
    fn edge(&mut self,from:&str,to:&str,relationship:Relationship,evidence:Vec<Address>,explanation:&str)->Result<()> {
        ensure!(self.records.contains_key(to),"Unresolved graph endpoint {to}");
        self.records.get_mut(from).context("Missing graph source")?.dependencies.push(Dependency{
            target:to.into(),relationship,evidence,explanation:explanation.into()});Ok(())
    }
    /// Bounded graph traversal includes incoming effects, so deleting a target
    /// does not overlook scripts which reference it. Unknowns remain explicit.
    pub fn closure(&self,selected:&BTreeSet<String>)->Result<BTreeSet<String>> {
        ensure!(selected.iter().all(|id|self.records.contains_key(id)),"Unknown selected native identity");
        let mut found=selected.clone();
        loop {
            let mut next=found.clone();
            for (id,record) in &self.records {
                for edge in &record.dependencies {
                    if edge.relationship==Relationship::Unrelated {continue;}
                    if found.contains(id) {next.insert(edge.target.clone());}
                    if found.contains(&edge.target) && edge.relationship==Relationship::Affects {next.insert(id.clone());}
                }
            }
            if next==found {return Ok(found);}found=next;
        }
    }
    pub fn validate(&self)->Result<()> {
        ensure!(!self.export_authorized,"Inventory cannot authorize export");
        for record in self.records.values() {
            for edge in &record.dependencies {
                ensure!(self.records.contains_key(&edge.target),"Dangling native graph reference");
                ensure!(!edge.explanation.is_empty(),"Missing relationship explanation");
                ensure!(matches!(edge.relationship,Relationship::Unresolved) || !edge.evidence.is_empty(),"Relationship lacks native evidence");
            }
        }Ok(())
    }
}
fn addr(container:&str,offset:usize,length:usize)->Address {Address{container:container.into(),offset,length}}

/// A storage relation only. Disjoint native records can still be connected by
/// scripts, selectors or triggers; this never proves behavioral independence.
pub(super) fn storage_relation(a:&Address,b:&Address)->Relationship {
    if a.container!=b.container {return Relationship::Unresolved;}
    let (Some(aend),Some(bend))=(a.offset.checked_add(a.length),b.offset.checked_add(b.length)) else {return Relationship::Unresolved;};
    if aend<=b.offset || bend<=a.offset {Relationship::Unrelated}
    else {Relationship::SharedReference}
}

#[derive(Clone,Copy,Debug,Serialize,Deserialize,PartialEq,Eq)]
pub(super) enum Operation {Move,Rotate,Scale,Duplicate,Delete,CrossCellImport}
#[derive(Debug,Serialize,Deserialize)]
pub(super) struct EditAssessment {
    pub operation:Operation,
    pub selected:BTreeSet<String>,
    pub dependency_closure:BTreeSet<String>,
    pub shared_references:BTreeSet<String>,
    pub unresolved_relationships:BTreeSet<String>,
    pub required_capabilities:BTreeSet<String>,
    pub export_authorized:bool,
}
impl CellInventory {
    /// Adapter work is selected by record type and dependency edges, never
    /// scene names or known object numbers. This report is not an executable
    /// export plan and cannot bypass the established native writers.
    pub fn assess(&self,selected:&BTreeSet<String>,operation:Operation)->Result<EditAssessment> {
        ensure!(!selected.is_empty(),"No native objects requested");
        let closure=self.closure(selected)?;
        let mut result=EditAssessment{operation,selected:selected.clone(),dependency_closure:closure.clone(),shared_references:BTreeSet::new(),unresolved_relationships:BTreeSet::new(),required_capabilities:BTreeSet::new(),export_authorized:false};
        for id in &closure {
            let record=&self.records[id];
            for edge in &record.dependencies {
                if edge.relationship==Relationship::SharedReference {result.shared_references.insert(edge.target.clone());}
                if edge.relationship==Relationship::Unresolved {result.unresolved_relationships.insert(format!("{id} -> {}: {}",edge.target,edge.explanation));}
            }
            match record.kind.as_str() {
                "WallBitfield"=>{result.required_capabilities.insert("proven wall contributions and overlap reconstruction".into());},
                "ElevationOverride"=>{result.required_capabilities.insert("sub-cell elevation ownership, relocation and representability".into());},
                "ManRecord"|"ManActor"=>{result.required_capabilities.insert("MAN control-flow, animation and incoming context closure".into());},
                "RegionTrigger"|"TileTrigger"=>{result.required_capabilities.insert("trigger behavior and coordinate relocation".into());},
                "UnresolvedModel"=>{result.unresolved_relationships.insert(format!("{id}: native model bytes unavailable"));},
                _=>{},
            }
        }
        let operation_capabilities:&[&str]=match operation {
            Operation::Move=>&["native position and physical dependency serialization"],
            Operation::Rotate=>&["native rotation, collider and surface rotation"],
            Operation::Scale=>&["independent native geometry, animation and physical scaling"],
            Operation::Duplicate=>&["independent native reference allocation and behavior identity"],
            Operation::Delete=>&["reference removal and retained shared contribution reconstruction"],
            Operation::CrossCellImport=>&["donor-independent native model/texture/animation registration, VRAM and memory budgets"],
        };
        result.required_capabilities.extend(operation_capabilities.iter().map(|s|s.to_string()));Ok(result)
    }
}

/// Every grid reference is counted, including invisible, ground-only and
/// descriptor-zero cells. Sharing never depends on which objects render.
fn map_records(out:&mut CellInventory,entry:u32,map:&[u8],load:&[u8])->Result<()> {
    ensure!(map.len()==0x12000 && load.len()==0x14000 && load[..0x12000]==*map,"Unverified native MAP load window");
    let container=format!("map:{entry}:{}",digest(map));
    out.insert("map".into(),"Map",Some(addr(&container,0,map.len())),json!({"entry":entry,"sha256":digest(map),"load_sha256":digest(load)}))?;
    out.insert("walls".into(),"WallBitfield",Some(addr(&container,0x4000,0x4000)),json!({"bits":map[0x4000..0x8000].iter().map(|b|(b&0xf0).count_ones()).sum::<u32>(),"ownership":"not encoded by object ID","physical_z_bias":-128}))?;
    out.insert("floors".into(),"FloorTierGrid",Some(addr(&container,0x4000,0x4000)),json!({"shared_wall_bytes":true,"mask":15,"lut":"separate MAN dependency"}))?;
    let mut refs=BTreeMap::<usize,Vec<[u8;2]>>::new();
    for (i,bytes) in map[0x8000..0x10000].chunks_exact(2).enumerate() {
        let word=u16::from_le_bytes([bytes[0],bytes[1]]);
        refs.entry((word&511) as usize).or_default().push([(i%128) as u8,(i/128) as u8]);
    }
    let binds=NativeBindings::parse(&load[0x10000..0x12000],&load[0x12000..])?;
    let load_container=format!("load:{entry}:{}",digest(load));
    for id in 0..512 {
        let rec=ObjectRecord::parse(map,id).context("Truncated descriptor")?;
        let cells=refs.get(&id).cloned().unwrap_or_default();
        let did=format!("descriptor:{id}");
        out.insert(did,"MapDescriptor",Some(addr(&container,id*32,32)),json!({
            "references":cells,"reference_count":cells.len(),"raw":&map[id*32..id*32+32],
            "placed":rec.is_placed(),"selector":rec.pack_index_field,"rotation":[rec.rot_x,rec.rot_y,rec.rot_z],
            "offset":[rec.x_off,rec.y_off,rec.z_off],"terrain_texture":[rec.terrain_tile as u16,rec.terrain_tpage,rec.terrain_clut]}))?;
    }
    // Retain all trigger rows, including shadowed duplicates and region rows.
    for (block,start) in [("primary",0x10000usize),("fallback",0x12000)] {
        let bytes=&load[start..start+0x2000];
        for kind in 0..4 {
            let offset=i16::from_le_bytes(bytes[kind*4+2..kind*4+4].try_into()?);
            let count=i16::from_le_bytes(bytes[kind*4+4..kind*4+6].try_into()?);
            if count<=0 {continue;}
            let stride=if kind==3 {8}else{4};
            if offset<18 || offset as usize+count as usize*stride>bytes.len() {
                out.gaps.push(format!("{block} kind-{kind}: invalid trigger row range"));continue;
            }
            for (i,row) in bytes[offset as usize..offset as usize+count as usize*stride].chunks_exact(stride).enumerate() {
                out.insert(format!("trigger:{block}:{kind}:{i}"),if kind==2 {"ElevationOverride"}else if kind==3 {"RegionTrigger"}else {"TileTrigger"},
                    Some(addr(&load_container,start+offset as usize+i*stride,stride)),json!({"kind":kind,"raw":row,"cell":[row[0],row[1]],"block":block}))?;
            }
        }
    }
    let trigger_ids=out.records.iter().filter(|(_,r)|matches!(r.kind.as_str(),"ElevationOverride"|"TileTrigger")).map(|(id,r)|(id.clone(),r.fields.clone(),r.address.clone().unwrap())).collect::<Vec<_>>();
    for (&id,cells) in &refs {
        if id==0 {continue;}
        let rec=ObjectRecord::parse(map,id).unwrap();
        for &[col,row] in cells {
            let go=0x8000+(row as usize*128+col as usize)*2;
            let word=u16::from_le_bytes(map[go..go+2].try_into()?);
            let placed=rec.is_placed();
            if !placed && word&field_objects::CELL_VISIBLE==0 {continue;}
            let oid=format!("object:{entry}:{col}:{row}");let did=format!("descriptor:{id}");
            let anchor=[col as i32+rec.col_delta as i32,row as i32+rec.row_delta as i32];
            let binding=if (0..128).contains(&anchor[0])&&(0..128).contains(&anchor[1]) {binds.at((anchor[0] as u8,anchor[1] as u8))}else{None};
            out.insert(oid.clone(),if placed {"MapPlacement"}else{"TerrainDecoration"},Some(addr(&container,go,2)),json!({"cell":[col,row],"descriptor":id,"grid_word":word,"anchor":anchor,
                "world_xz":[field_objects::world_x(col,rec.x_off),field_objects::world_z(row,rec.z_off)],"selector":rec.pack_index_field,
                "native_bind":binding.map(|b|b.record),
                "binding_evidence":binding.map(|b|addr(&load_container,if b.block=="MAP" {0x10000+b.offset}else{0x12000+b.offset},4))}))?;
            out.edge(&oid,&did,if cells.len()==1 {Relationship::ExclusiveReference}else{Relationship::SharedReference},vec![addr(&container,go,2)],"Complete MAP grid reference count only; ground fields, scripts and runtime consumers require separate closure")?;
            out.edge(&oid,"walls",Relationship::Unresolved,vec![],"MAP wall bits have no native object-owner IDs; proximity cannot establish ownership")?;
            let floor=0x4000+row as usize*128+col as usize;
            let samples=if placed {vec![addr(&container,floor,1)]}else{[0,1,128,129].into_iter().map(|d|addr(&container,floor+d,1)).collect()};
            out.edge(&oid,"floors",Relationship::References,samples,"Placed objects read their tier; decorations read four raw flat-offset corner bytes, including native boundary behavior. Sampling is not ownership")?;
            for (tid,fields,source) in &trigger_ids {
                let tcell=fields["cell"].as_array().context("Invalid trigger cell")?;
                let t=[tcell[0].as_i64().unwrap() as i32,tcell[1].as_i64().unwrap() as i32];
                if t==[col as i32,row as i32] || (placed && t==anchor) {
                    out.edge(&oid,tid,Relationship::References,vec![source.clone()],"Exact native trigger coordinate match; does not prove exclusive ownership or relocation closure")?;
                }
            }
        }
    }
    Ok(())
}

pub(super) fn inspect(index:&ProtIndex,name:&str)->Result<CellInventory> {
    let scene=Scene::load(index,name)?;
    let mut out=CellInventory{schema:1,scene:name.into(),entry_range:[scene.start,scene.end],..Default::default()};
    for entry in &scene.entries {
        out.insert(format!("entry:{}",entry.idx),"ProtEntry",Some(addr(&format!("prot:{}:{}",entry.idx,digest(&entry.bytes)),0,entry.bytes.len())),
            json!({"sha256":digest(&entry.bytes),"classification":format!("{:?}",entry.class),"length":entry.bytes.len()}))?;
    }
    if let Some(entry)=scene.field_map_index(index) {
        let lba=index.entry_start_lba_retail(entry.try_into()?).context("MAP physical address unavailable")?;
        let load=index.prot_dat_raw_bytes(lba as u64*2048,0x14000)?;
        // Use the physical streaming window, not a guessed following entry.
        if let Err(error)=map_records(&mut out,entry,&load[..0x12000],&load) {out.gaps.push(format!("MAP inventory incomplete: {error:#}"));}
    } else {out.gaps.push("No field-MAP layout established for this CDNAME block; retained all entry records".into());}
    let bank=SceneModelBank::build(&scene);
    let models=bank.materialise(&scene);
    for (selector,bytes) in models.iter().enumerate() {
        let id=format!("model:{selector}");
        let source=bank.scene_source(selector as u16);
        if let Some(bytes)=bytes {
            let parsed=legaia_tmd::parse(bytes);
            out.insert(id,"NativeModel",Some(addr(&format!("decoded-tmd:{}",digest(bytes)),0,bytes.len())),json!({"sha256":digest(bytes),"source":format!("{source:?}"),"selector":selector,"tmd_valid":parsed.is_ok()}))?;
            if let Err(e)=parsed {out.gaps.push(format!("Model {selector} TMD validation: {e}"));}
        } else {
            out.insert(id,"UnresolvedModel",None,json!({"selector":selector,"source":format!("{source:?}")}))?;
            out.gaps.push(format!("Native model {selector} failed materialisation"));
        }
    }
    let objects=out.records.iter().filter(|(_,r)|matches!(r.kind.as_str(),"MapPlacement"|"TerrainDecoration")).map(|(id,r)|(id.clone(),r.fields["selector"].as_u64().unwrap(),r.address.clone().unwrap())).collect::<Vec<_>>();
    for (id,selector,source) in objects {
        let target=format!("model:{selector}");
        if out.records.contains_key(&target) {out.edge(&id,&target,Relationship::References,vec![source],"Native descriptor +0x10 indexes the scene model-registration bank; immutable content equality is not instance identity")?;}
        else {
            out.insert(target.clone(),"UnresolvedModel",None,json!({"selector":selector,"reason":"absent from established scene bank"}))?;
            out.edge(&id,&target,Relationship::Unresolved,vec![source],"Descriptor selector could not be resolved in the native bank")?;
            out.gaps.push(format!("{id}: model selector {selector} absent from resolved native bank"));
        }
    }
    match scene.field_man_payload(index) {
        Ok(Some(man))=>{if let Err(e)=man_records(&mut out,&man) {out.gaps.push(format!("MAN inventory incomplete: {e:#}"));}},
        Ok(None)=>out.gaps.push("No MAN payload established; script dependencies unresolved".into()),
        Err(e)=>out.gaps.push(format!("MAN load: {e:#}")),
    }
    if !out.records.contains_key("floor-lut") {out.gaps.push("Floor LUT unresolved without a valid MAN header".into());}
    if let Some(bundle)=legaia_engine_scene::scene_bundle::find_bundle(&scene) {
        bundle_records(&mut out,&bundle)?;
    }
    let animations=out.records.iter().filter(|(_,r)|r.kind=="AnimationSelector").map(|(id,r)|(id.clone(),r.fields.clone(),r.address.clone().unwrap())).collect::<Vec<_>>();
    let anm_assets=out.records.iter().filter(|(_,r)|r.kind=="DecodedAsset"&&r.fields["type_byte"]==6).map(|(id,_)|id.clone()).collect::<Vec<_>>();
    out.insert("animation-closure".into(),"UnresolvedDependency",None,json!({"reason":"clip decoding, shared banks, resource lifetime and runtime script clip changes require closure"}))?;
    for (id,fields,source) in animations {
        if fields["special"]==false && fields["anim_id"].as_u64()!=Some(0) {
            for asset in &anm_assets {out.edge(&id,asset,Relationship::References,vec![source.clone()],"Native scene animation selector depends on the scene ANM asset; clip validity still unresolved")?;}
        }
        out.edge(&id,"animation-closure",Relationship::Unresolved,vec![],"Native animation header is inventoried; compatible clip/bone/lifetime closure has not been established")?;
    }
    match legaia_engine_scene::scene_bundle::scene_tim_layout(&scene) {
        Ok(tims)=>for (i,tim) in tims.iter().enumerate() {
            let bytes=scene.entries.iter().find(|e|e.idx==tim.entry_idx).context("TIM entry absent")?;
            let raw=bytes.bytes.get(tim.offset..tim.offset+tim.byte_len).context("TIM payload range invalid")?;
            out.insert(format!("tim:{i}"),"TimUpload",Some(addr(&format!("prot:{}:{}",tim.entry_idx,digest(&bytes.bytes)),tim.offset,tim.byte_len)),json!({"sha256":digest(raw),"kind":format!("{:?}",tim.kind),"width":tim.width,"height":tim.height,"clut":tim.has_clut}))?;
        },Err(e)=>out.gaps.push(format!("TIM inventory: {e:#}")),
    }
    out.insert("texture-context".into(),"TextureContext",None,json!({"scope":"scene upload context","primitive_closure_verified":false}))?;
    let tims=out.records.iter().filter(|(_,r)|r.kind=="TimUpload").map(|(id,r)|(id.clone(),r.address.clone().unwrap())).collect::<Vec<_>>();
    for (id,source) in tims {out.edge("texture-context",&id,Relationship::References,vec![source],"Observed scene-local TIM upload; upload ordering and per-primitive image/CLUT closure are separate")?;}
    let models=out.records.iter().filter(|(_,r)|r.kind=="NativeModel").map(|(id,_)|id.clone()).collect::<Vec<_>>();
    for id in models {out.edge(&id,"texture-context",Relationship::Unresolved,vec![],"TMD content is resolved but full per-primitive page/CLUT and compressed/shared upload closure is not yet proven")?;}
    let effects=out.records.iter().filter(|(_,r)|r.kind=="ManRecord").map(|(id,r)|(id.clone(),r.address.clone().unwrap(),r.fields["effects"].clone())).collect::<Vec<_>>();
    for (id,source,effects) in effects {
        if let Some(effects)=effects.as_array() {for effect in effects {
            let target=match effect["kind"].as_str() {Some("FloorLadderOperation")=>"floor-lut",Some("ClutEffect")=>"texture-context",_=>continue};
            if out.records.contains_key(target) {
                let pc=effect["pc"].as_u64().unwrap() as usize;let size=effect["size"].as_u64().unwrap() as usize;
                out.edge(&id,target,Relationship::Affects,vec![addr(&source.container,source.offset+pc,size)],"Decoded native scene-global floor or CLUT operation; conditions and execution order are unresolved")?;
            }
        }}
    }
    out.gaps.push("TIM upload inventory is scene-context evidence; per-primitive image/CLUT closure, ANM/shared banks, packed storage and memory budgets require further resolution".into());
    out.gaps.push("Linear MAN scans do not prove control flow, helper effects, dynamic targets or runtime wall/floor ownership".into());
    out.validate()?;Ok(out)
}

fn man_records(out:&mut CellInventory,man:&[u8])->Result<()> {
    let inventory=SceneInventory::read(man).map_err(anyhow::Error::msg)?;
    let container=format!("decoded-man:{}",inventory.man_sha256);
    // FUN_8003C83C has exactly these two native special cases. These are
    // shared runtime channels, not missing flat MAN records 248 and 251.
    out.insert("context:player".into(),"SharedRuntimeContext",None,json!({"selector":248,"resolver":"FUN_8003C83C","identity":"player"}))?;
    out.insert("context:system".into(),"SharedRuntimeContext",None,json!({"selector":251,"resolver":"FUN_8003C83C","identity":"scene system channel"}))?;
    let lut=man[2..0x22].chunks_exact(2).map(|b|i16::from_le_bytes([b[0],b[1]])).collect::<Vec<_>>();
    out.insert("floor-lut".into(),"FloorHeightLut",Some(addr(&container,2,32)),json!({"values":lut,"runtime_mutable":true}))?;
    if out.records.contains_key("floors") {out.edge("floors","floor-lut",Relationship::References,vec![addr(&container,2,32)],"Native MAN header supplies all 16 signed floor heights; scripts may change the live ladder")?;}
    for record in &inventory.records {
        let id=format!("man:{}",record.flat);
        out.insert(id,"ManRecord",Some(addr(&container,record.start,record.len)),json!({"partition":record.partition,"index":record.record,"flat":record.flat,"pc0":record.pc0,"scan_error":record.scan.error,"effects":record.scan.effects,
            "wall_writes":record.scan.wall_writes.iter().map(|w|json!({"pc":w.pc,"sub":w.sub,"rectangle":[w.x0,w.z0,w.x1,w.z1],"mask":w.mask})).collect::<Vec<_>>()}))?;
    }
    let system_script=format!("man:{}",inventory.partition_counts[0].max(0));
    if let Some(source)=out.records.get(&system_script).and_then(|r|r.address.clone()) {
        out.edge("context:system",&system_script,Relationship::References,vec![source],"Native scene-entry system script is partition 1 record 0; shared runtime channel is not an independently owned object script")?;
    }
    for record in &inventory.records {
        let id=format!("man:{}",record.flat);
        for &(pc,target) in &record.scan.targets {
            let special=match target {0xf8=>Some("context:player"),0xfb=>Some("context:system"),_=>None};
            if let Some(target)=special {
                out.edge(&id,target,Relationship::Affects,vec![addr(&container,record.start+pc,2)],"Native context resolver special channel; this relationship is shared runtime behavior, not object ownership")?;continue;
            }
            let target=format!("man:{target}");
            if out.records.contains_key(&target) {out.edge(&id,&target,Relationship::Affects,vec![addr(&container,record.start+pc,2)],"Decoded extended context instruction and selector; not proof of execution or exclusive script ownership")?;}
            else {
                let unknown=format!("unresolved-context:{target}");
                if !out.records.contains_key(&unknown) {out.insert(unknown.clone(),"UnresolvedDependency",None,json!({"target":target,"reason":"absent MAN record, special system context or dynamic context requires resolution"}))?;}
                out.edge(&id,&unknown,Relationship::Unresolved,vec![addr(&container,record.start+pc,2)],"Decoded context selector does not resolve to an inventoried MAN record")?;
                out.gaps.push(format!("{id}: context target {target} not resolved"));
            }
        }
        if out.records.contains_key("walls") && !record.scan.wall_writes.is_empty() {
            out.edge(&id,"walls",Relationship::Affects,record.scan.wall_writes.iter().map(|w|addr(&container,record.start+w.pc,if w.sub<2 {6}else{7})).collect(),"Decoded native wall-grid writers; conditions and exclusive contributions unresolved")?;
        }
    }
    let file=legaia_scene_formats::man_section::parse(man).map_err(anyhow::Error::msg)?;
    let actors=file.actor_placements(man);
    for actor in actors {
        let id=format!("actor:{}",actor.index);let source=addr(&container,actor.record_offset,actor.script_pc0);
        out.insert(id.clone(),"ManActor",Some(source.clone()),serde_json::to_value(&actor)?)?;
        let animation=format!("animation:{}",actor.index);
        let animation_source=addr(&container,actor.record_offset+actor.script_pc0-3,1);
        out.insert(animation.clone(),"AnimationSelector",Some(animation_source.clone()),json!({"anim_id":actor.anim_id,"special":actor.special_model}))?;
        out.edge(&id,&animation,Relationship::References,vec![animation_source],"Native actor header animation operand; bank depends on special-model selection")?;
        if out.records.contains_key("floors") {out.edge(&id,"floors",Relationship::References,vec![source.clone()],"Native spawn coordinates sample scene floor state; actor scripts can subsequently reposition it")?;}
        let base=inventory.partition_counts[0].max(0) as usize;
        let record=format!("man:{}",base+actor.index);
        if out.records.contains_key(&record) {out.edge(&id,&record,Relationship::References,vec![source.clone()],"Native partition-1 actor header belongs to this MAN record; runtime scripts may change actor state")?;}
        let model=format!("model:{}",actor.model_index);
        if !actor.special_model && out.records.contains_key(&model) {out.edge(&id,&model,Relationship::References,vec![source],"Native actor model operand indexes scene bank")?;}
        else {
            let unknown=format!("unresolved-actor-model:{}",actor.model_index);
            if !out.records.contains_key(&unknown) {out.insert(unknown.clone(),"UnresolvedDependency",None,json!({"model":actor.model_index,"special":actor.special_model}))?;}
            out.edge(&id,&unknown,Relationship::Unresolved,vec![source],"Actor model requires a shared/special bank or an unresolved scene-bank entry")?;
            out.gaps.push(format!("{id}: shared/special or unresolved model {} requires separate bank closure",actor.model_index));
        }
    }
    let binds=out.records.iter().filter(|(_,r)|r.kind=="MapPlacement").filter_map(|(id,r)|r.fields["native_bind"].as_u64().map(|flat|(id.clone(),flat,r.fields["binding_evidence"].clone()))).collect::<Vec<_>>();
    for (id,flat,source) in binds {
        let target=format!("man:{flat}");
        if out.records.contains_key(&target) {out.edge(&id,&target,Relationship::References,vec![serde_json::from_value(source)?],"First-match kind-1 native binding resolves this flat MAN context; incoming dependencies retained separately")?;}
        else {
            let unknown=format!("unresolved-man:{flat}");
            if !out.records.contains_key(&unknown) {out.insert(unknown.clone(),"UnresolvedDependency",None,json!({"flat":flat,"reason":"bound context not decoded"}))?;}
            out.edge(&id,&unknown,Relationship::Unresolved,vec![],"Native bind points to an absent or undecodable MAN context")?;
            out.gaps.push(format!("{id}: bound MAN flat record {flat} is unresolved"));
        }
    }
    out.gaps.extend(inventory.gaps);Ok(())
}

fn bundle_records(out:&mut CellInventory,bundle:&legaia_engine_scene::scene_bundle::BundleSource<'_>)->Result<()> {
    let bytes=bundle.bytes();let table=bundle.table_offset();
    let count=u32::from_le_bytes(bytes[table..table+4].try_into()?) as usize;
    let descriptors=bundle.descriptors();ensure!(count<=descriptors.len(),"Bundle descriptor count exceeds resolved table");
    let container=format!("prot:{}:{}",bundle.entry_idx(),digest(bytes));
    for (i,d) in descriptors.iter().take(count).enumerate() {
        let id=format!("asset-descriptor:{i}");let start=table+d.data_offset as usize;
        let end=descriptors.iter().take(count).map(|other|table+other.data_offset as usize).filter(|&at|at>start).min().unwrap_or(bytes.len());
        out.insert(id.clone(),"BundleDescriptor",Some(addr(&container,table+8+i*8,8)),json!({"type_byte":d.type_byte,"decoded_size":d.size,"payload_offset":start,"storage_end":end,"memory_budget_verified":false}))?;
        if d.size==0 || d.data_offset==0 {continue;}
        let payload=bytes.get(start..end);
        let decoded=payload.context("Compressed descriptor outside native entry").and_then(|p|legaia_lzs::decompress_tracked(p,d.size as usize).map_err(Into::into));
        match decoded {
            Ok((data,consumed)) if data.len()==d.size as usize=>{
                let target=format!("decoded-asset:{i}");
                out.insert(target.clone(),"DecodedAsset",Some(addr(&format!("decoded-asset:{}",digest(&data)),0,data.len())),json!({"type_byte":d.type_byte,"sha256":digest(&data),"decoded_size":data.len(),"compressed_bytes":consumed}))?;
                out.edge(&id,&target,Relationship::References,vec![addr(&container,start,consumed)],"Tracked LZS payload decoded to the descriptor's exact size; storage footprint is distinct from runtime memory budget")?;
            },
            other=>out.gaps.push(format!("Asset descriptor {i}: storage/decode path unresolved ({})",match other {Ok((data,_))=>format!("decoded {} of {} bytes",data.len(),d.size),Err(e)=>format!("{e:#}")})),
        }
    }Ok(())
}

#[cfg(test)]
#[path="native_cell_inventory_tests.rs"]
mod tests;
