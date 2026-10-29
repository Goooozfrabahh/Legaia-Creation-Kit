//! Temporary permissive export attempts. Policy checks are warnings; real
//! encoder failures remain explicit and never prevent other attempts/output.
use super::*;
use std::{cell::{Cell,RefCell},collections::HashMap,io::Write};
thread_local! {
    static ACTIVE:Cell<bool>=const{Cell::new(false)};
    static WARNINGS:RefCell<Vec<String>>=const{RefCell::new(Vec::new())};
}
pub(in crate::app) fn active()->bool {ACTIVE.with(Cell::get)}
pub(in crate::app) fn warn_check(message:String)->bool {
    if active(){note(format!("CHECK BYPASSED: {message}"));true}else{false}
}
fn note(message:String){WARNINGS.with(|w|w.borrow_mut().push(message));}
pub(in crate::app) fn diagnostic(message:String){note(message);}

pub(in crate::app) fn failure_log(output:&Path,manifest:&str,error:&str)->Result<PathBuf>{
    let parent=output.parent().context("No output parent")?;let stamp=std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_nanos();
    let dir=parent.join(format!("{}_failed_export_{stamp}",output.file_stem().and_then(|s|s.to_str()).unwrap_or("IMG")));std::fs::create_dir(&dir)?;
    let path=dir.join("debug.txt");std::fs::write(&path,format!("OUTPUT FAILED: {}\n{error}\nFull requested manifest:\n{manifest}\nNo successful output claim. Existing file, if any, is not this completed export.\n",output.display()))?;Ok(path)
}
struct Scope(bool);
impl Drop for Scope {fn drop(&mut self){ACTIVE.with(|v|v.set(self.0));}}
fn scope()->Scope{Scope(ACTIVE.with(|v|v.replace(true)))}
#[cfg(test)]
pub(in crate::app) fn with_test_scope<T>(f:impl FnOnce()->T)->T{let _scope=scope();f()}
fn attempt<T>(label:&str,log:&mut Vec<String>,f:impl FnOnce()->Result<T>)->Option<T> {
    log.push(format!("ATTEMPT {label}"));
    let result=std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
    WARNINGS.with(|warnings|{for warning in warnings.borrow_mut().drain(..){log.push(format!("WARNING {label}: {warning}"));}});
    match result {
        Ok(Ok(value))=>{log.push(format!("ENCODED/RESOLVED {label}"));Some(value)},
        Ok(Err(error))=>{log.push(format!("FAILED {label}: {error:#}"));None},
        Err(error)=>{let msg=error.downcast_ref::<String>().map(String::as_str).or_else(||error.downcast_ref::<&str>().copied()).unwrap_or("native encoder panic");log.push(format!("FAILED {label}: native writer could not encode/access requested data: {msg}"));None}
    }
}
fn integer(value:f32)->Result<i32>{
    anyhow::ensure!(value.is_finite() && value.abs()<i32::MAX as f32,"Non-finite/out-of-range native coordinate");
    let encoded=value.round() as i32;
    note(format!("NATIVE PRECISION coordinate: requested {value}; encoded {encoded}; error {} PS1 units",encoded as f32-value));Ok(encoded)
}
fn relative_native_translation(requested:[f32;3],donor:[f32;3],encoded:[i32;3])->Result<[f32;3]>{
    let mut result=[0.;3];for a in 0..3 {result[a]=integer(requested[a]-donor[a])?.checked_sub(encoded[a]).context("Native translation difference overflow")? as f32;}Ok(result)
}
fn grid(cell:(u8,u8))->usize {GRID_OFF+(cell.1 as usize*128+cell.0 as usize)*2}
fn id_at(map:&[u8],cell:(u8,u8))->Result<usize>{
    anyhow::ensure!(cell.0<128&&cell.1<128,"Native reference outside 128x128 MAP");let at=grid(cell);
    Ok((u16::from_le_bytes(map.get(at..at+2).context("Native grid truncated")?.try_into()?)&511) as usize)
}
fn record(map:&[u8],id:usize)->Result<[u8;32]>{Ok(map.get(id*32..id*32+32).context("Native descriptor truncated")?.try_into()?)}
fn detach(map:&mut [u8],cell:(u8,u8))->Result<usize>{
    let id=id_at(map,cell)?;
    let refs=map[GRID_OFF..0x10000].chunks_exact(2).filter(|w|u16::from_le_bytes([w[0],w[1]])&511==id as u16).count();
    if refs<=1{return Ok(id);}
    let target=blank_object_record(map)?;let raw=record(map,id)?;map[target*32..target*32+32].copy_from_slice(&raw);
    let at=grid(cell);let word=u16::from_le_bytes(map[at..at+2].try_into()?);map[at..at+2].copy_from_slice(&((word&!511)|target as u16).to_le_bytes());
    note(format!("Shared descriptor {id} detached to {target}; unrelated references retained"));Ok(target)
}
fn source(scene:&AssembledScene,key:(bool,usize))->Result<(u8,u8)>{
    Ok(if key.0 {scene.terrain.get(key.1)}else{scene.placements.get(key.1)}.context("No native MAP source for editor target")?.cell)
}
fn translate_record(map:&mut [u8],cell:(u8,u8),delta:[f32;3])->Result<usize>{
    let id=detach(map,cell)?;let mut raw=record(map,id)?;let old=ObjectRecord::parse(&raw,0).context("Invalid MAP record")?;
    let [dx,dy,dz]=[integer(delta[0])?,integer(delta[1])?,integer(delta[2])?];
    for (offset,value) in [(0,old.x_off as i32+dx),(2,old.y_off as i32-dy),(4,old.z_off as i32-dz)] {
        raw[offset..offset+2].copy_from_slice(&i16::try_from(value).context("Requested placement exceeds native signed 16-bit record encoding")?.to_le_bytes());
    }
    if old.is_placed(){let moved=ObjectRecord::parse(&raw,0).unwrap();if let Err(e)=adjust_collider(&moved,&old,&mut raw){note(format!("Record-local collider correction not encoded: {e:#}"));}}
    map[id*32..id*32+32].copy_from_slice(&raw);
    note(format!("Native MAP record {id}: offsets {:?} -> {:?}; no anchor/cull/overlap/binding/ownership preflight. External walls, floors and scripts are not inferred",[old.x_off,old.y_off,old.z_off],[ObjectRecord::parse(&raw,0).unwrap().x_off,ObjectRecord::parse(&raw,0).unwrap().y_off,ObjectRecord::parse(&raw,0).unwrap().z_off]));Ok(id)
}
/// Allocate a native descriptor/reference anywhere representable. Occupied
/// world-space positions and distant anchors do not block this allocator.
pub(in crate::app) fn copy_record(map:&mut [u8],original:&[u8],source_cell:(u8,u8),delta:[f32;3],tiers:&[Option<i32>;16],source_tiers:Option<&[Option<i32>;16]>)->Result<(usize,(u8,u8))>{
    let old_id=id_at(original,source_cell)?;let raw=record(original,old_id)?;let old=ObjectRecord::parse(&raw,0).unwrap();
    let id=blank_object_record(map)?;let dx=integer(delta[0])?;let dy=integer(delta[1])?;let dz=integer(delta[2])?;
    let wx=field_objects::world_x(source_cell.0,old.x_off)+dx;let wz=field_objects::world_z(source_cell.1,old.z_off)+dz;
    let mut best=None;
    for row in 0..128 {for col in 0..128 {
        let cell=(col as u8,row as u8);let at=grid(cell);let word=u16::from_le_bytes(map[at..at+2].try_into()?);
        if word&511!=0 {continue;} // one reference per native grid word is a format capacity, not spatial overlap
        let xo=wx-(col*128+64);let zo=row*128+64-wz;
        if i16::try_from(xo).is_err()||i16::try_from(zo).is_err(){continue;}
        let score=(xo as i64).pow(2)+(zo as i64).pow(2);
        if best.as_ref().is_none_or(|&(s,_,_,_)|score<s){best=Some((score,cell,xo,zo));}
    }}
    let (_,cell,x,z)=best.context("Native MAP reference table has no additional encodable slot; no dynamic-spawn writer is implemented (no unrelated object overwritten)")?;
    let src_tier=(original[WALL_OFF+source_cell.1 as usize*128+source_cell.0 as usize]&15) as usize;
    let dst_tier=(map[WALL_OFF+cell.1 as usize*128+cell.0 as usize]&15) as usize;
    let correction=match(source_tiers.unwrap_or(tiers)[src_tier],tiers[dst_tier]){(Some(a),Some(b))=>a-b,_=>{note("Floor anchor compensation unavailable; raw Y-offset written and dependency is unverified".into());0}};
    let mut next=raw;for (at,value) in [(0,x),(2,old.y_off as i32-dy+correction),(4,z)]{next[at..at+2].copy_from_slice(&i16::try_from(value).context("Copy offset exceeds native i16 representation")?.to_le_bytes());}
    if old.is_placed(){let moved=ObjectRecord::parse(&next,0).unwrap();if let Err(e)=adjust_collider(&moved,&old,&mut next){note(format!("Copied collider correction failed: {e:#}"));}}
    map[id*32..id*32+32].copy_from_slice(&next);
    let at=grid(cell);let oldword=u16::from_le_bytes(map[at..at+2].try_into()?);
    map[at..at+2].copy_from_slice(&((oldword&!511)|id as u16|if old.is_placed(){0}else{field_objects::CELL_VISIBLE}).to_le_bytes());
    note(format!("Independent descriptor {id}, reference {cell:?}, native offsets ({x},{z}); occupied visual locations allowed; spawn-window/physics/runtime behavior unverified"));Ok((id,cell))
}
pub(in crate::app) fn yaw(map:&mut [u8],cell:(u8,u8),rotation:[f32;3],center:[f32;3])->Result<()> {
    anyhow::ensure!(rotation.iter().all(|v|v.is_finite()),"Non-finite rotation");
    let id=detach(map,cell)?;let mut raw=record(map,id)?;let old=ObjectRecord::parse(&raw,0).unwrap();
    anyhow::ensure!(rotation[0].abs()<0.00001&&rotation[2].abs()<0.00001&&old.rot_x&4095==0&&old.rot_z&4095==0,"Available native transform writer cannot compose pitch/roll/tilted matrices; writer missing, not a claimed PS1 engine limit");
    let angle=rotation[1] as f64/std::f64::consts::TAU*4096.;
    note(format!("NATIVE PRECISION yaw: requested {} radians ({angle} ticks); encoded {} ticks ({} radians)",rotation[1],angle.round(),angle.round()*std::f64::consts::TAU/4096.));
    let pos=[field_objects::world_x(cell.0,old.x_off) as f32,0.,field_objects::world_z(cell.1,old.z_off) as f32];
    let transformed=world_view::editor_rotate_vertex(pos,[center[0],0.,center[2]],[0.;3],rotation,[1.;3]);
    let x=i16::try_from(integer(transformed[0])?-(cell.0 as i32*128+64))?;let z=i16::try_from(cell.1 as i32*128+64-integer(transformed[2])?)?;
    raw[..2].copy_from_slice(&x.to_le_bytes());raw[4..6].copy_from_slice(&z.to_le_bytes());
    let value=(old.rot_y&0xf000)|((old.rot_y as i32+angle.round() as i32).rem_euclid(4096) as u16);raw[10..12].copy_from_slice(&value.to_le_bytes());
    map[id*32..id*32+32].copy_from_slice(&raw);note("Native yaw and pivot compensation written; arbitrary-angle actor-box/wall transforms are not serialized by this writer".into());Ok(())
}
pub(in crate::app) fn prepare(source_path:&Path,scene:&AssembledScene,world:&WorldModel,index:&ProtIndex)->Result<NativeExportPlan>{
    let _scope=scope();WARNINGS.with(|w|w.borrow_mut().clear());
    let source_path=source_path.canonicalize()?;let image=std::fs::read(&source_path)?;
    let overlay=scene_instances::serialize(&scene.name,world);
    let mut log=vec!["UNRESTRICTED IMG EXPORT — EXPERIMENTAL ATTEMPT, NOT A COMPLETE/VERIFIED EXPORT".into(),format!("Source: {}\nScene: {}\nFull requested manifest:\n{overlay}",source_path.display(),scene.name),"WARNING All compatibility/ownership/binding/collision/overlap/spawn-window preflight is disabled. Encoder failures are retained per operation. Original scripts, collision and resources may become inconsistent; no gameplay verification.".into()];
    let mut disc=DiscPatcher::open(image)?;
    let mut imports=ImportCoverage{requested:world.foreign.len(),written:0};
    let mut object_verification=Vec::new();
    let doc=attempt("parse exact editor manifest",&mut log,||scene_instances::parse(&overlay,&scene.name,world).map_err(anyhow::Error::msg));
    if let Some(doc)=doc {
        let native=attempt("resolve selected native cell",&mut log,||Scene::load(index,&scene.name));
        if let Some(native)=native {
            if let Some(entry)=native.field_map_index(index).map(|i|i as usize) {
                compile(&mut disc,entry,scene,world,index,&doc,&mut log,&mut imports,&mut object_verification);
            }else{log.push("FAILED No native MAP writer for selected cell; all requested edits remain unwritten".into());}
        }
    }
    let _=attempt("structural native IMG reparse (not gameplay)",&mut log,||DiscPatcher::open(disc.image().to_vec()));
    let failed=log.iter().filter(|l|l.starts_with("FAILED ")).cloned().collect::<Vec<_>>();
    let written=log.iter().filter(|l|l.starts_with("WRITTEN ")).cloned().collect::<Vec<_>>();
    log.push(format!("IMPORT COVERAGE: {} requested; {} native objects written; {} FAILED IMPORTS. {}",imports.requested,imports.written,imports.failed(),imports.failure_message().unwrap_or_default()));
    log.push(format!("FINAL STATUS: {}. {} successful writer attempts; {} failures. Read all warnings. No claim of complete native behavior or gameplay compatibility.",if failed.is_empty(){"ATTEMPT WRITTEN / DEPENDENCIES UNVERIFIED"}else{"PARTIAL / REQUESTED CHANGES NOT FULLY WRITTEN"},written.len(),failed.len()));
    Ok(NativeExportPlan{dependencies:Default::default(),source:source_path,image:disc.into_image(),patched:written,skipped:failed,continuation_receipt:None,unrestricted_log:Some(log.join("\n")),imports:Some(imports),object_verification})
}

fn compile(disc:&mut DiscPatcher,entry:usize,scene:&AssembledScene,world:&WorldModel,index:&ProtIndex,doc:&scene_instances::SceneDocument,log:&mut Vec<String>,coverage:&mut ImportCoverage,reports:&mut Vec<serde_json::Value>){
    // Import each requested instance independently using the existing native
    // resource registration writer. No collision/runtime acceptance gate.
    let mut imported_targets=HashMap::new();
    let mut resource_session=None;
    let mut intended_models=HashMap::<u64,String>::new();
    // Identical donor instances share the registered native resource closure,
    // while each receives its own descriptor and MAP reference. Cache only the
    // exact donor identity, never a nearby or merely similar object.
    let mut prototypes:HashMap<(String,String,usize),((u8,u8),[i32;3])>=HashMap::new();
    let tiers=Scene::load(index,&scene.name).ok().and_then(|n|n.field_floor_height_lut(index).ok().flatten()).map(|h|h.map(|v|Some(-(v as i32)))).unwrap_or([None;16]);
    for foreign in &doc.foreign {
        let label=format!("IMPORT UID {} from {} {} #{}",foreign.uid,foreign.source,foreign.kind,foreign.record);
        let identity=(foreign.source.clone(),foreign.kind.clone(),foreign.record);
        if let Some(&(cell,encoded_translation))=prototypes.get(&identity) {
            if let Some((candidate,descriptor,allocated))=attempt(&label,log,||{
                let original=disc.read_entry_footprint(entry)?;let mut map=original.clone();
                let donor=world.foreign.iter().find(|f|f.uid==foreign.uid).context("Foreign source provenance absent")?;
                let delta=relative_native_translation(foreign.center,donor.donor_center,encoded_translation)?;
                let (descriptor,allocated)=copy_record(&mut map,&original,cell,delta,&tiers,None)?;
                let mut candidate=DiscPatcher::open(disc.image().to_vec())?;candidate.patch_prot_entry(entry,0,&map)?;
                Ok((candidate,descriptor,allocated))
            }){
                *disc=candidate;imported_targets.insert(scene_instances::NameTarget::Created(foreign.uid),allocated);
                coverage.written+=1;
                if let Ok(hash)=independent_hash(disc,entry,allocated){intended_models.insert(foreign.uid,hash);}
                log.push(format!("WRITTEN {label}: independent MAP descriptor {descriptor} at {:#x}, reference {allocated:?}; REUSED exact donor TMD/TIM resource closure from reference {cell:?}; no additional texture uploads or bundle growth",descriptor*32));
            }
            continue;
        }
        if let Some(candidate)=attempt(&label,log,||{
            anyhow::ensure!(matches!(foreign.kind.as_str(),"placement"|"terrain"),"Existing native import writer has no {} descriptor/actor adapter; no different source object substituted",foreign.kind);
            let mut isolated=world.clone();
            isolated.draws.retain(|d|d.editor_uid.is_none()||d.editor_uid==Some(foreign.uid));
            for d in &mut isolated.draws{d.hidden=false;d.offset=[0.;3];d.rotation=[0.;3];d.scale=[1.;3];}
            if let Some(d)=isolated.draws.iter_mut().find(|d|d.editor_uid==Some(foreign.uid)){d.original_center=foreign.center;d.center=foreign.center;}
            isolated.foreign.retain(|f|f.uid==foreign.uid);isolated.collision.recipes.clear();isolated.collision.mesh_recipes.clear();
            NativeExportPlan::compile_saved_imports_session(DiscPatcher::open(disc.image().to_vec())?,scene,&isolated,index,
                super::super::native_asset_plan::scene_registration::Limits{max_models:u16::MAX as usize-field_objects::FIELD_ACTOR_PACK_BIAS as usize,max_decoded:usize::MAX,max_encoded:usize::MAX},&mut resource_session)
        }){for &(uid,descriptor,cell,selector) in &candidate.placements{imported_targets.insert(scene_instances::NameTarget::Created(uid),cell);
                coverage.written+=1;
                if let Ok(hash)=independent_hash(&candidate.disc,entry,cell){intended_models.insert(uid,hash);}
                if let Some(donor)=world.foreign.iter().find(|f|f.uid==uid){prototypes.insert(identity.clone(),(cell,[0,1,2].map(|a|(foreign.center[a]-donor.donor_center[a]).round() as i32)));}
                log.push(format!("WRITTEN {label}: MAP PROT {entry}, descriptor {descriptor} at {:#x}, grid {cell:?} at {:#x}, TMD selector {selector}",descriptor*32,grid(cell)));}
            for idx in [entry+3,entry+4] {
                if let (Ok(before),Ok(after))=(disc.read_entry_footprint(idx),candidate.disc.read_entry_footprint(idx)) {
                    if before!=after {log.push(format!("WRITTEN {label}: RESOURCE PROT {idx}, {} -> {} bytes, SHA256 {}",before.len(),after.len(),super::super::collision_recipes::digest(&after)));}
                }
            }
            *disc=candidate.disc;log.push(format!("WRITTEN {label}: resource registration and native reference committed to IMG buffer"));}
    }
    let Some(original)=attempt("read native MAP",log,||disc.read_entry_footprint(entry)) else{return};let mut map=original.clone();
    let mut targets:HashMap<scene_instances::NameTarget,(u8,u8)>=imported_targets.clone();
    let tiers=Scene::load(index,&scene.name).ok().and_then(|n|n.field_floor_height_lut(index).ok().flatten()).map(|h|h.map(|v|Some(-(v as i32)))).unwrap_or([None;16]);
    for created in &doc.created {
        if let Some(npc)=world.npc(created.source){npc_attempt(disc,entry,npc,Some(created.uid),created.offset,log);continue;}
        let label=format!("DUPLICATE UID {} source {:?}",created.uid,created.source);
        if let Some((_,cell))=attempt(&label,log,||copy_record(&mut map,&original,source(scene,created.source)?,created.offset,&tiers,None)) {targets.insert(scene_instances::NameTarget::Created(created.uid),cell);}
    }
    for &(key,delta) in &doc.moves {
        if doc.hidden.contains(&key){log.push(format!("SUPERSEDED MOVE {key:?}: final editor state deletes this object"));continue;}
        if let Some(npc)=world.npc(key){npc_attempt(disc,entry,npc,None,delta,log);continue;}
        let label=format!("MOVE {key:?} delta {delta:?}");let before=map.clone();
        if attempt(&label,log,||translate_record(&mut map,source(scene,key)?,delta)).is_none(){map=before;}
        log.push(format!("WARNING {label}: independent MAP walls, floor geometry and MAN/script dependencies are not automatically relocated; unresolved relationships do not block output"));
    }
    for (target,rotation) in &doc.rotations {
        let key=match target {scene_instances::NameTarget::Native(t,i)=>Some((*t,*i)),_=>None};
        if key.is_some_and(|k|doc.hidden.contains(&k)){log.push(format!("SUPERSEDED ROTATE {target:?}: deleted"));continue;}
        let label=format!("ROTATE {target:?} {rotation:?}");let before=map.clone();
        if attempt(&label,log,||{
            let cell=match key{Some(k)=>source(scene,k)?,None=>*targets.get(target).context("Created/imported instance has no successfully allocated MAP reference")?};
            let draw=match target{scene_instances::NameTarget::Native(t,i)=>world.at(*t,*i),scene_instances::NameTarget::Created(uid)=>world.draws.iter().find(|d|d.editor_uid==Some(*uid))}.context("Missing exact rotation pivot")?;
            yaw(&mut map,cell,*rotation,draw.center)
        }).is_none(){map=before;}
    }
    for (target,scale) in &doc.scales {log.push(format!("ATTEMPT SCALE {target:?} {scale:?}"));log.push(format!("FAILED SCALE {target:?}: available MAP descriptor has no scale field; no native TMD vertex/normal transform registration writer is implemented for this edit. No replacement edit was made"));}
    for &key in &doc.hidden {
        let label=format!("DELETE {key:?}");
        attempt(&label,log,||{anyhow::ensure!(world.npc(key).is_none(),"No native MAN actor/script deletion serializer implemented");let cell=source(scene,key)?;let at=grid(cell);let word=u16::from_le_bytes(map[at..at+2].try_into()?);map[at..at+2].copy_from_slice(&(word&!511).to_le_bytes());Ok(())});
        log.push(format!("WARNING {label}: geometry/spawn reference removed; standalone walls/floors/scripts were not deleted by an inferred ownership rule"));
    }
    let mut appearance=HashSet::new();
    for (target,clip,frame) in &doc.animations {
        appearance.insert(target.clone());
        if *frame!=0 {log.push(format!("FAILED ANIMATION FRAME {target:?} clip {clip} frame {frame}: native MAN writer supports starting animation ID, but has no preview-frame spawn field"));}
    }
    for (target,_) in &doc.models {appearance.insert(target.clone());}
    for target in appearance {
        let key=match target {scene_instances::NameTarget::Native(t,i)=>Some((t,i)),scene_instances::NameTarget::Created(uid)=>doc.created.iter().find(|c|c.uid==uid).map(|c|c.source)};
        if let Some((key,npc))=key.and_then(|k|world.npc(k).map(|n|(k,n))) {
            // A move already wrote the current model/clip together with position.
            if matches!(target,scene_instances::NameTarget::Native(..)) && !doc.moves.iter().any(|(k,_)|*k==key) && !doc.hidden.contains(&key) {
                npc_attempt(disc,entry,npc,None,[0.;3],log);
            }
        }else{log.push(format!("FAILED APPEARANCE {target:?}: no native MAN appearance adapter for this target; requested selectors remain unwritten"));}
    }
    for assignment in &doc.collision_recipes {
        let label=format!("COLLISION {:?}",assignment.owner);
        let before=map.clone();
        if attempt(&label,log,||{
            let draw=world.draws.iter().find(|d|world.collision.owner(world,(d.terrain,d.index)).as_ref()==Some(&assignment.owner)).context("Collision owner not present in editor document")?;
            if draw.hidden {return Ok(());}
            // Use the existing geometry rasterizer; ignore provenance/ownership
            // permission gates. Explicit open-space edits can clear native walls.
            let recipe=&assignment.recipe;
            // FOREIGN persistence stores an absolute center. Its saved mesh
            // recipe keeps the original rest-space pivot, so derive translation
            // from those two actual coordinates (also equals offset before save).
            let mut walls=if let Some(g)=&recipe.generated {g.wall_cells([0,1,2].map(|a|draw.center[a]-g.pivot[a]),draw.rotation,draw.scale).map_err(anyhow::Error::msg)?}else{Default::default()};
            let (dx,dz)=if recipe.walls.is_empty()&&recipe.nonblocking.is_empty(){(0,0)}else{
                let dx=integer(draw.offset[0])?;let dz=integer(draw.offset[2])?;
                anyhow::ensure!(dx%64==0&&dz%64==0,"Manual collision cells require exact 64-unit grid translation; writer cannot encode fractional wall cells");(dx,dz)};
            for &[x,z] in &recipe.walls {walls.insert([x+dx/64,z+dz/64]);}
            for (cells,blocked) in [(&recipe.nonblocking,false),(&walls.iter().copied().collect::<Vec<_>>(),true)] {
                for &[x,z] in cells {let (x,z)=if blocked {(x,z)}else{(x+dx/64,z+dz/64)};
                    anyhow::ensure!((0..256).contains(&x)&&(0..256).contains(&z),"Collision cell outside native 256x256 bit grid: ({x},{z})");
                    let at=0x4000+(z as usize/2)*128+x as usize/2;let mask=1<< (4+(z as usize%2)*2+x as usize%2);
                    if blocked {map[at]|=mask;}else{map[at]&=!mask;}
                }
            }
            if recipe.generated.is_none()&&(draw.rotation!=[0.;3]||draw.scale!=[1.;3]||draw.offset[1]!=0.) {log::warn!("Manual wall cell writer encodes X/Z cells only");bail!("Manual wall cell writer has no rotation/scale/vertical representation; no approximate collider substituted");}
            Ok(())
        }).is_none(){map=before;}
    }
    for recipe in &doc.mesh_collision {log.push(format!("FAILED MESH COLLISION: available writer cannot encode mesh-only collision request {recipe:?}"));}
    if map!=original {
        for id in 0..512 {let at=id*32;if map[at..at+32]!=original[at..at+32]{
            log.push(format!("ENCODED MAP PROT {entry} descriptor {id} at {at:#x}: before {:02x?}; after {:02x?}",&original[at..at+32],&map[at..at+32]));
        }}
        if attempt("publish MAP fields to native IMG sectors",log,||disc.patch_prot_entry(entry,0,&map)).is_some(){
            log.push("WRITTEN MAP: all successful MAP operation attempts above committed together to IMG buffer; failed operations were rolled back individually".into());
        }else{log.push("FAILED MAP COMMIT: successful MAP encodings above did not reach IMG; imports/MAN writes are independent".into());}
    }
    for foreign in &doc.foreign{
        let result=(||->Result<serde_json::Value>{
            let cell=*imported_targets.get(&scene_instances::NameTarget::Created(foreign.uid)).context("No committed native MAP reference")?;
            let map=disc.read_entry_footprint(entry)?;let id=id_at(&map,cell)?;let r=ObjectRecord::parse(&map,id).context("Native descriptor missing")?;
            let loaded_model=independent_model(disc,entry,cell)?;
            let actual=loaded_model.sha256;let expected=intended_models.get(&foreign.uid).context("No independent identity witness recorded at commit")?;
            anyhow::ensure!(&actual==expected,"Cumulative registration identity changed after later imports/transforms");
            let anchor=[cell.0 as i32+r.col_delta as i32,cell.1 as i32+r.row_delta as i32];
            let anchor_word=if (0..128).contains(&anchor[0])&&(0..128).contains(&anchor[1]){Some(u16::from_le_bytes(map[grid((anchor[0]as u8,anchor[1]as u8))..][..2].try_into()?))}else{None};
            let donor=legaia_engine_world_views::scene_staging::assemble_field_scene(index,&foreign.source).map_err(anyhow::Error::msg)?;
            let original=if foreign.kind=="terrain"{donor.terrain.get(foreign.record)}else{donor.placements.get(foreign.record)}.context("Donor draw missing")?;
            let donor_native=Scene::load(index,&foreign.source)?;
            let binding=donor_native.field_object_binds(index)?.and_then(|b|b.get(&original.anchor).copied());
            let binding_evidence=if let Some(binding)=binding{
                let man=donor_native.field_man_payload(index)?.context("Donor bound object has no MAN")?;
                let inventory=super::super::native_script_dependencies::SceneInventory::read(&man).map_err(anyhow::Error::msg)?;
                let record=inventory.records.iter().find(|r|r.flat==binding.record as usize).context("Donor binding record missing")?;
                serde_json::json!({"record":binding.record,"animation":binding.anim_id,"man_sha256":inventory.man_sha256,
                    "record_sha256":super::super::collision_recipes::digest(&man[record.start..record.start+record.len]),"record_range":[record.start,record.len],"pc0":record.pc0,
                    "record_bytes":&man[record.start..record.start+record.len],"decoded_instructions":record.scan.instructions,"incoming_extended_context_sites":inventory.incoming(record.flat),
                    "scan_error":record.scan.error,"transplanted":false,"control_flow_closure_verified":false})
            }else{serde_json::Value::Null};
            let mut gaps=vec!["native rendering, heap availability and subsequent runtime writes not observed".to_owned(),"collision encoding does not verify walkability or interaction".into()];
            if let Some(binding)=binding{gaps.push(format!("MISSING: donor MAN object context {} / ANM clip {} was not relocated; static geometry encoding does not preserve donor behavior",binding.record,binding.anim_id));}
            if anchor_word.is_none(){gaps.push("Native footprint anchor is outside MAP".into());}
            if anchor_word.is_some_and(|w|w&field_objects::CELL_BIND_OWNED!=0){gaps.push("Native window sweep skips a bind-owned anchor; matching init-sweep MAN binding must be verified".into());}
            let raw=world.foreign.iter().find(|f|f.uid==foreign.uid).context("Donor provenance missing")?;
            let failures=log.iter().filter(|l|l.starts_with("FAILED ")&&(l.contains(&format!("Created({})",foreign.uid))||l.contains(&format!("UID {} ",foreign.uid)))).cloned().collect::<Vec<_>>();
            Ok(serde_json::json!({"uid":foreign.uid,"source_scene":foreign.source,"category":foreign.kind,"source_catalog_ordinal":foreign.record,
                "source_tmd_sha256":super::super::collision_recipes::digest(&raw.raw_tmd),"encoded_successfully":failures.is_empty(),"native_import_reference_written":true,"operation_failures":failures,"img_write_readback_verified":false,
                "requested_editor_center":foreign.center,
                "donor_binding_evidence":binding_evidence,
                "native_loader_identity":{"status":"independent_static_contract_matched_cumulatively","runtime_verified":false,"relocated_tmd_sha256":actual,"relocated_tmd_bytes":loaded_model.bytes.len(),"scene_selector":r.pack_index_field,"global_slot":r.pack_index_field as usize+5,"pointer_table_address":format!("{:#x}",0x8007c018+(r.pack_index_field as u32+5)*4)},
                "native_reference":{"map_entry":entry,"descriptor":id,"cell":cell,"anchor":anchor,"anchor_word":anchor_word,"rotation_ticks":[r.rot_x,r.rot_y,r.rot_z],"offsets":[r.x_off,r.y_off,r.z_off],"cull_radius":original.cull_radius},
                "render_visibility_verified":false,"collision_interaction_verified":false,"missing_or_unverified_dependencies":gaps}))
        })();
        let value=match result{Ok(value)=>value,Err(e)=>{log.push(format!("FAILED FINAL IDENTITY UID {}: {e:#}",foreign.uid));serde_json::json!({"uid":foreign.uid,"source_scene":foreign.source,"category":foreign.kind,"encoded_successfully":false,"img_write_readback_verified":false,"native_loader_identity":{"status":"failed_or_unavailable","runtime_verified":false},"render_visibility_verified":false,"collision_interaction_verified":false,"missing_or_unverified_dependencies":[format!("{e:#}")]})}};
        log.push(format!("PER-OBJECT VERIFICATION: {value}"));reports.push(value);
    }
    // Attempt the existing data-only native floor writer for a terrain source
    // with a real keyed primary row. This is explicit key association, not a
    // claim of mesh ownership. No experimental engine patch is installed here.
    let mut floor_requests=Vec::new();
    for &(key,delta) in &doc.moves {if key.0&&!doc.hidden.contains(&key){floor_requests.push((key,delta,false));}}
    for created in &doc.created {if created.source.0 {floor_requests.push((created.source,created.offset,true));}}
    for (key,delta,duplicate) in floor_requests {
        let label=format!("FLOOR keyed to terrain {key:?}, delta {delta:?}, duplicate {duplicate}");
        let result=attempt(&label,log,||{
            let cell=source(scene,key)?;
            let loaded=disc.read_disc_sectors(disc.entry_disc_lba(entry).context("No native MAP LBA")?,0x28)?;
            let rows=legaia_engine_field::field_elevation::parse_elevation_overrides(&loaded[0x10000..0x12000]);
            let selected=rows.iter().find(|r|(r.tile_x,r.tile_z)==cell);
            let Some(row)=selected else{return Ok(None)};
            note("A native primary elevation key matches this terrain cell; ownership is unverified. Attempting existing data-only writer; no engine code changes".into());
            let expected=super::super::native_elevation_codec::serialize(row)?;
            let mut edit=super::super::native_floor_patch::Edit::translated(expected,[integer(delta[0])?,integer(delta[2])?],duplicate)?;
            edit.height_delta=-integer(delta[1])?;
            let native=Scene::load(index,&scene.name)?;
            let lut=native.field_floor_height_lut(index)?.context("No native floor ladder")?.map(|y|-(y as i32));
            let plan=super::super::native_floor_patch::compile(&loaded,&lut,&[edit])?;
            let output=plan.apply(&loaded)?;
            let mut candidate=DiscPatcher::open(disc.image().to_vec())?;
            candidate.patch_prot_entry(entry,0,&output[..0x12000])?;Ok(Some(candidate))
        });
        match result {Some(Some(candidate))=>{*disc=candidate;log.push(format!("WRITTEN {label}: native floor row and grid flag committed"));},Some(None)=>log.push(format!("WARNING {label}: no primary keyed floor row. Shared tier/bilinear floor data remains unchanged; no ownership-based replacement inferred")),None=>{}}
    }
}
fn independent_hash(disc:&DiscPatcher,entry:usize,cell:(u8,u8))->Result<String>{
    Ok(independent_model(disc,entry,cell)?.sha256)
}
fn independent_model(disc:&DiscPatcher,entry:usize,cell:(u8,u8))->Result<super::super::native_loader_trace::Model>{
    let map=disc.read_entry_footprint(entry)?;let r=ObjectRecord::parse(&map,id_at(&map,cell)?).context("Native MAP record missing")?;
    let bundle=disc.read_entry_footprint(entry+3)?;let(base,_)=super::super::native_asset_plan::scene_registration::native_descriptor_table(&bundle)?;
    let pac=disc.read_entry_footprint(entry+4)?;let trace=super::super::native_loader_trace::trace(&bundle,base,Some(&pac))?;
    Ok(trace.models.get(r.pack_index_field as usize).context("Native loader contract did not register selected TMD")?.clone())
}
fn npc_attempt(disc:&mut DiscPatcher,entry:usize,npc:&world_view::NpcInfo,uid:Option<u64>,delta:[f32;3],log:&mut Vec<String>){
    let label=format!("MAN actor {} copy {uid:?} delta {delta:?}",npc.record);
    if delta[1].abs()>EPS {log.push(format!("FAILED {label} Y component: existing MAN spawn writer has no elevation field; Y request {0} is not serialized",delta[1]));}
    if let Some(plan)=attempt(&label,log,||{
        let edit=NpcEdit{uid,record:npc.record,source_position:(npc.psx_position[0].round() as i32,npc.psx_position[2].round() as i32),original_model:npc.original_model_index,original_anim:npc.anim_id,model:npc.model_index,anim:npc.clip,delta,kind:npc.kind};
        let plan=compile_plan(PathBuf::new(),DiscPatcher::open(disc.image().to_vec())?,disc.image().len(),entry,&ExportEdits{npcs:vec![edit],..Default::default()})?;
        DiscPatcher::open(plan.image)
    }){*disc=plan;log.push(format!("WRITTEN {label}: native MAN changes committed to IMG buffer"));}
}

pub(super) fn publish(plan:NativeExportPlan,output:&Path)->Result<ExportResult>{
    // Filesystem preservation stays mandatory even when native acceptance is disabled.
    let output=if output.is_absolute(){output.to_path_buf()}else{std::env::current_dir()?.join(output)};
    anyhow::ensure!(!output.exists()&&output!=plan.source,"Choose a new output file; source and existing artifacts are never overwritten");
    let parent=output.parent().context("Output folder missing")?;
    let stamp=std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_nanos();
    let logs=parent.join(format!("{}_export_{stamp}",output.file_stem().and_then(|s|s.to_str()).unwrap_or("IMG")));
    std::fs::create_dir(&logs)?;let debug=logs.join("debug.txt");
    let mut report=plan.unrestricted_log.clone().unwrap_or_default();report.push_str(&format!("\nOutput requested: {}\n",output.display()));
    std::fs::write(&debug,&report)?;
    let verification=logs.join("object-verification.json");
    std::fs::write(&verification,serde_json::to_vec_pretty(&serde_json::json!({"schema":1,"output":output,"objects":plan.object_verification,"native_runtime_verified":false,"render_verified":false,"interaction_verified":false}))?)?;
    let expected_hash=super::super::collision_recipes::digest(&plan.image);
    let result=(||->Result<()>{
        let staging=logs.join("output.lckbuilding");let mut file=std::fs::OpenOptions::new().write(true).create_new(true).open(&staging)?;
        file.write_all(&plan.image)?;file.sync_all()?;drop(file);
        std::fs::hard_link(&staging,&output).context("Create experimental IMG without replacement")?;std::fs::remove_file(staging)?;
        verify_output_file(&output,Some(plan.image.len() as u64))?;
        use sha2::Digest;use std::io::Read;
        let mut input=std::fs::File::open(&output)?;let mut hash=sha2::Sha256::new();let mut buffer=[0u8;65536];
        loop {let n=input.read(&mut buffer)?;if n==0{break;}hash.update(&buffer[..n]);}
        anyhow::ensure!(format!("{:x}",hash.finalize())==expected_hash,"Output IMG readback hash mismatch: {}",output.display());Ok(())})();
    match result {Ok(())=>report.push_str("OUTPUT WRITTEN. Experimental native writes only; partial/dependency failures above remain unresolved.\n"),Err(e)=>{report.push_str(&format!("OUTPUT FAILED: {e:#}\n"));std::fs::write(&debug,report)?;return Err(e);}}
    report.push_str(&format!("OUTPUT FILE VERIFIED: {} ({} bytes; disk readback matches intended IMG)\nOutput SHA256: {expected_hash}\n",output.display(),plan.image.len()));
    let mut objects=plan.object_verification;
    for object in &mut objects{object["img_write_readback_verified"]=serde_json::json!(object["native_import_reference_written"].as_bool()==Some(true));report.push_str(&format!("FINAL PER-OBJECT VERIFICATION: {object}\n"));}
    std::fs::write(&debug,&report)?;
    std::fs::write(&verification,serde_json::to_vec_pretty(&serde_json::json!({"schema":1,"output":output,"img_sha256":expected_hash,"objects":objects,"native_runtime_verified":false,"render_verified":false,"interaction_verified":false}))?)?;
    let mut skipped=plan.skipped;
    for object in &objects{if let Some(gaps)=object["missing_or_unverified_dependencies"].as_array(){for gap in gaps{if gap.as_str().is_some_and(|g|g.starts_with("MISSING:")){skipped.push(format!("MISSING NATIVE DEPENDENCY UID {}: {}",object["uid"],gap.as_str().unwrap()));}}}}
    skipped.push(format!("UNRESTRICTED EXPERIMENT — NOT PROVEN COMPLETE. Full debug.txt: {}; per-object report: {}",debug.display(),verification.display()));
    Ok(ExportResult{path:output,patched:plan.patched,skipped,imports:plan.imports})
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn written_import_with_missing_behavior_is_prominently_incomplete(){
        let result=ExportResult{path:"partial.img".into(),patched:vec![],skipped:vec!["MISSING NATIVE DEPENDENCY UID 2: MAN object context and animation".into()],imports:Some(ImportCoverage{requested:3,written:3})};
        assert!(result.experimental_summary().contains("INCOMPLETE NATIVE BEHAVIOR"));
        assert!(result.verification_warning().unwrap().contains("NOT verified working objects"));
    }
    #[test]
    fn zero_successful_imports_are_not_confused_with_successful_collision_writes(){
        let result=ExportResult{path:"partial.img".into(),patched:vec!["WRITTEN collision".into()],skipped:vec!["FAILED IMPORT UID 1".into()],imports:Some(ImportCoverage{requested:3,written:0})};
        assert!(result.experimental_summary().contains("NO REQUESTED OBJECTS IMPORTED"));assert!(result.experimental_summary().contains("3 FAILED IMPORTS"));
        assert!(result.imports.unwrap().failure_message().unwrap().contains("scene export is incomplete"));
    }
    #[test]
    fn shared_resources_round_each_requested_placement_from_its_original_donor()->Result<()>{
        for (first,next) in [(0.4,0.8),(0.6,0.2),(-0.4,-0.8),(-0.6,-0.2)] {
            let encoded=integer(first)?;let delta=relative_native_translation([next,0.,0.],[0.;3],[encoded,0,0])?;
            assert_eq!(encoded+delta[0] as i32,integer(next)?,"resource reuse must not accumulate relative rounding");
        }Ok(())
    }
    #[test]
    fn output_verification_rejects_missing_empty_and_truncated_files()->Result<()>{
        let dir=tempfile::tempdir()?;let path=dir.path().join("output.img");
        assert!(verify_output_file(&path,None).is_err());std::fs::write(&path,[])?;assert!(verify_output_file(&path,None).is_err());
        std::fs::write(&path,vec![0;2352])?;assert!(verify_output_file(&path,Some(4704)).is_err());assert_eq!(verify_output_file(&path,Some(2352))?,2352);Ok(())
    }
    #[test]
    #[ignore="replay a user-authored foreign export log; never generate an arranged scene"]
    fn replay_user_foreign_export()->Result<()>{
        let source=PathBuf::from(std::env::var_os("LCK_FOREIGN_SOURCE").context("Source required")?);
        let text=std::fs::read_to_string(std::env::var_os("LCK_FOREIGN_LOG").context("User export log required")?)?;
        let start=text.find("LCK_SCENE_V1\t").context("Manifest missing")?;
        let manifest=text[start..].lines().take_while(|l|!l.is_empty()).collect::<Vec<_>>().join("\n");
        let library=crate::library::Library::open(&source)?;
        let scene=legaia_engine_world_views::scene_staging::assemble_field_scene(&library.index,"town01").map_err(anyhow::Error::msg)?;
        let mut world=world_view::build_world(&scene,&library.index);
        let doc=scene_instances::parse(&manifest,"town01",&world).map_err(anyhow::Error::msg)?;
        scene_instances::apply(&doc,&mut world,scene.placements.len(),&library.index).map_err(anyhow::Error::msg)?;
        let editor_before=scene_instances::serialize("town01",&world);
        let plan=prepare(&source,&scene,&world,&library.index)?;
        anyhow::ensure!(scene_instances::serialize("town01",&world)==editor_before,"Export modified editor transforms");
        println!("{}",plan.unrestricted_log.as_deref().unwrap_or("NO LOG"));
        anyhow::ensure!(plan.skipped.is_empty(),"Native writer failures remain: {:?}",plan.skipped);
        let before_map=Scene::load(&library.index,"town01")?.field_map_index(&library.index).context("No MAP")? as usize;
        let planned=DiscPatcher::open(plan.image.clone())?;
        let intended_map=planned.read_entry_footprint(before_map)?;
        if let Some(baseline)=std::env::var_os("LCK_COLLISION_BASELINE"){
            let previous=DiscPatcher::open(std::fs::read(baseline)?)?;let map=previous.read_entry_footprint(before_map)?;
            anyhow::ensure!(map[0x4000..0x8000]==intended_map[0x4000..0x8000]&&map[0x10000..]==intended_map[0x10000..],"User-observed collision/floors/bindings changed");
            for idx in 0..previous.entry_count(){if ![before_map,before_map+3,before_map+4].contains(&idx){anyhow::ensure!(previous.read_entry_footprint(idx)?==planned.read_entry_footprint(idx)?,"Unrelated native entry changed: {idx}");}}
            for idx in [before_map+3,before_map+4]{anyhow::ensure!(planned.read_entry_footprint(idx)?.len()<=0x62c00,"Committed native carrier {idx} exceeds actual arena");}
            println!("MIXED NATIVE AUDIT: observed collision/floors/bindings and unrelated native entries preserved; all committed carriers fit the field arena");
        }
        if let Some(baseline)=std::env::var_os("LCK_VRAM_BASELINE"){
            use crate::app::native_asset_plan::{scene_registration,texture_relocation};
            use legaia_scene_formats::scene_asset_table;
            let old=DiscPatcher::open(std::fs::read(baseline)?)?;
            let verified_map=old.read_entry_footprint(before_map)?;
            if verified_map!=intended_map {for (at,(a,b)) in verified_map.iter().zip(&intended_map).enumerate().filter(|(_, (a,b))|a!=b).take(100){println!("MAP DIFFERENCE {at:#x}: verified {a:#04x}, candidate {b:#04x}");}}
            anyhow::ensure!(verified_map==intended_map,"VRAM-only fix changed user-verified MAP placements/collision/floors/bindings");
            for idx in 0..old.entry_count(){if idx!=before_map+3&&idx!=before_map+4{anyhow::ensure!(old.read_entry_footprint(idx)?==planned.read_entry_footprint(idx)?,"Unrelated PROT entry {idx} changed");}}
            let limits=scene_registration::Limits{max_models:1024,max_decoded:16*1024*1024,max_encoded:16*1024*1024};
            let bundle=planned.read_entry_footprint(before_map+3)?;let pac=planned.read_entry_footprint(before_map+4)?;
            anyhow::ensure!(pac.len()<=0x62c00,"Sector-rounded PAC still exceeds native field arena");
            let (_,vram)=scene_registration::native_texture_sequence(&bundle,Some(&pac),limits)?;
            let ob=old.read_entry_footprint(before_map+3)?;let op=old.read_entry_footprint(before_map+4)?;
            let (_,ov)=scene_registration::native_texture_sequence(&ob,Some(&op),limits)?;
            let sb=planned.read_entry_footprint(before_map+3)?;
            let mut clobbered=vram.clone();let mut old_clobbered=ov.clone();
            for r in texture_relocation::field_runtime_reservations(){let bytes=vec![0xa5;r.w as usize*r.h as usize*2];clobbered.write_block(r.x,r.y,r.w,r.h,&bytes);old_clobbered.write_block(r.x,r.y,r.w,r.h,&bytes);}
            let source_disc=DiscPatcher::open(std::fs::read(&source)?)?;
            let source_bundle=source_disc.read_entry_footprint(before_map+3)?;let source_pac=source_disc.read_entry_footprint(before_map+4)?;
            let (_,source_vram)=scene_registration::native_texture_sequence(&source_bundle,Some(&source_pac),limits)?;
            anyhow::ensure!((0..1024*512).all(|i|!source_vram.is_written(i%1024,i/1024)||source_vram.as_u16()[i]==vram.as_u16()[i]),"Original native texture/palette upload words changed");
            let base_models=scene_asset_table::mesh_pool(&source_bundle).len();
            let old_models=scene_asset_table::mesh_pool(&ob);let new_models=scene_asset_table::mesh_pool(&sb);
            anyhow::ensure!(old_models.len()==new_models.len(),"Duplicate resources changed registration count");
            let(mut checked,mut old_corrupted)=(0,0);
            for i in base_models..new_models.len(){
                let old_raw=&old_models[i].bytes;let raw=&new_models[i].bytes;
                let (n,changed)=texture_relocation::texture_sample_changes(old_raw,&ov,raw,&vram)?;checked+=n;anyhow::ensure!(changed==0,"Relocated native TMD texture/palette samples differ");
                anyhow::ensure!(texture_relocation::texture_sample_changes(raw,&vram,raw,&clobbered)?.1==0,"Framebuffer writes corrupt new allocation");
                old_corrupted+=texture_relocation::texture_sample_changes(old_raw,&ov,old_raw,&old_clobbered)?.1;
            }
            anyhow::ensure!(checked>0&&old_corrupted>0,"Framebuffer corruption regression did not exercise imported materials");
            println!("VRAM REGRESSION: {checked} imported native TMD UV samples match; {old_corrupted} old samples corrupt under simulated framebuffer writes, zero fixed samples corrupt. MAP/collision/floors/bindings and unrelated entries byte-identical. PAC {} <= {} bytes. Automated simulation, NOT gameplay.",pac.len(),0x62c00);
        }
        let original_map=DiscPatcher::open(std::fs::read(&source)?)?.read_entry_footprint(before_map)?;
        let new_cells=(0..128).flat_map(|z|(0..128).map(move |x|(x as u8,z as u8))).filter(|&cell|id_at(&original_map,cell).ok()!=id_at(&intended_map,cell).ok()).collect::<Vec<_>>();
        anyhow::ensure!(new_cells.len()==doc.foreign.len(),"Foreign requests not all independently allocated");
        if let Some(output)=std::env::var_os("LCK_FOREIGN_OUTPUT"){
            let done=super::super::export(plan,Path::new(&output))?;
            let read=DiscPatcher::open(std::fs::read(&done.path)?)?;anyhow::ensure!(read.read_entry_footprint(before_map)?==intended_map,"On-disk MAP differs");
            let reopened=crate::library::Library::open(&done.path)?;
            let assembled=legaia_engine_world_views::scene_staging::assemble_field_scene(&reopened.index,"town01").map_err(anyhow::Error::msg)?;
            for cell in new_cells {let draw=assembled.placements.iter().chain(&assembled.terrain).find(|p|p.cell==cell).context("New native instance missing after reopening IMG")?;anyhow::ensure!(assembled.res.tmds.get(draw.res_tmd).is_some(),"Imported TMD missing after reopening");}
            println!("Verified output {}; native scene reparsed and all foreign instances/resources resolved. Gameplay NOT VERIFIED",done.path.display());
        }
        Ok(())
    }
    fn map()->Vec<u8>{
        let mut m=vec![0;0x12000];
        m[32..34].copy_from_slice(&10i16.to_le_bytes());
        m[32+6]=255;m[32+7]=255;
        m[32+0x12..32+0x14].copy_from_slice(&0x181eu16.to_le_bytes());
        let at=grid((34,34));m[at..at+2].copy_from_slice(&0x401u16.to_le_bytes());
        m[0x4000..0x8000].fill(0xf0);m
    }
    #[test]
    fn unrestricted_check_scope_does_not_disable_normal_validation(){
        fn gated()->Result<()>{export_check!(false,"test compatibility warning");Ok(())}
        assert!(gated().is_err());
        {let _scope=scope();assert!(gated().is_ok());assert!(WARNINGS.with(|w|w.borrow().iter().any(|s|s.contains("test compatibility warning"))));}
        assert!(gated().is_err());
    }
    #[test]
    fn occupied_wall_bound_record_and_distant_copy_are_attempted_exactly()->Result<()>{
        let original=map();let mut moved=original.clone();
        translate_record(&mut moved,(34,34),[8192.,0.,-64.])?;
        assert_eq!(i16::from_le_bytes(moved[32..34].try_into()?),8202);
        assert_eq!(&moved[0x4000..0x8000],&original[0x4000..0x8000]);
        let (id,cell)=copy_record(&mut moved,&original,(34,34),[12000.,0.,0.],&[Some(0);16],None)?;
        assert_ne!(id,1);assert_ne!(cell,(34,34));
        let copied=ObjectRecord::parse(&moved,id).unwrap();
        assert_eq!(field_objects::world_x(cell.0,copied.x_off),field_objects::world_x(34,10)+12000);
        assert_eq!(id_at(&moved,(34,34))?,1);Ok(())
    }
    #[test]
    fn native_precision_is_logged_and_encoding_failures_never_report_written(){
        let mut log=Vec::new();let mut m=map();
        assert!(attempt("unrepresentable coordinate",&mut log,||translate_record(&mut m,(34,34),[1e9,0.,0.])).is_none());
        assert!(attempt("fractional coordinate",&mut log,||translate_record(&mut m,(34,34),[0.5,0.,0.])).is_some());
        assert!(attempt("exact coordinate",&mut log,||translate_record(&mut m,(34,34),[64.,0.,0.])).is_some());
        assert!(log.iter().any(|s|s.starts_with("FAILED unrepresentable coordinate")));
        assert!(log.iter().any(|s|s.contains("NATIVE PRECISION coordinate: requested 0.5; encoded 1")));
        assert!(log.iter().any(|s|s.starts_with("ENCODED/RESOLVED exact coordinate")));
        assert!(!log.iter().any(|s|s.starts_with("WRITTEN")));
    }
    #[test]
    #[ignore="explicit unedited pipeline check only; requires source and new output, creates no edited test scene"]
    fn unedited_retail_pipeline_output_and_debug()->Result<()>{
        let path=PathBuf::from(std::env::var_os("LCK_UNRESTRICTED_SOURCE").context("Source required")?);
        let output=PathBuf::from(std::env::var_os("LCK_UNRESTRICTED_OUTPUT").context("New output required")?);
        let library=crate::library::Library::open(&path)?;
        let scene=legaia_engine_world_views::scene_staging::assemble_field_scene(&library.index,"town01").map_err(anyhow::Error::msg)?;
        let world=world_view::build_world(&scene,&library.index);
        let plan=prepare(&path,&scene,&world,&library.index)?;
        anyhow::ensure!(plan.skipped.is_empty(),"Unedited pipeline unexpectedly failed: {:?}",plan.skipped);
        let original_hash=super::super::super::collision_recipes::digest(&plan.image);
        let done=super::super::export(plan,&output)?;
        let read=std::fs::read(&done.path)?;DiscPatcher::open(read.clone())?;
        anyhow::ensure!(super::super::super::collision_recipes::digest(&read)==original_hash,"Output changed during publication");
        anyhow::ensure!(super::super::super::collision_recipes::digest(&std::fs::read(path)?)==original_hash,"Unedited source changed");
        let marker=done.skipped.last().context("Missing debug location")?;
        let debug=Path::new(marker.split("Full debug.txt: ").nth(1).context("Missing debug path")?);
        anyhow::ensure!(std::fs::read_to_string(debug)?.contains("OUTPUT WRITTEN"),"No completed diagnostic log");
        println!("UNEDITED PIPELINE ONLY — no edited scene/GUI/gameplay verified. Output {} SHA256 {original_hash}\n{}",done.path.display(),debug.display());Ok(())
    }
}
