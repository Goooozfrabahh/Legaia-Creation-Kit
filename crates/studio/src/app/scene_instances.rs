//! Foundation 02: non-destructive editor-only scene instance overlay.
//! This is deliberately NOT a writer for PS1 placement tables or the game disc.
use super::world_view::WorldModel;
use std::collections::HashSet;

#[derive(Debug, Clone, PartialEq)]
pub(super) struct NewInstance {
    pub uid: u64,
    pub source: (bool, usize),
    pub offset: [f32; 3],
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct ForeignInstance {
    pub uid:u64,
    pub source:String,
    pub kind:String,
    pub record:usize,
    pub center:[f32;3],
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) enum NameTarget { Native(bool, usize), Created(u64) }

#[derive(Debug, Clone, Default, PartialEq)]
pub(super) struct SceneDocument {
    pub mesh_collision: Vec<super::collision_generation::MeshOnlyRecipe>,
    pub collision_recipes: Vec<super::collision_recipes::Assignment>,
    pub moves: Vec<((bool, usize), [f32; 3])>,
    pub created: Vec<NewInstance>,
    pub foreign: Vec<ForeignInstance>,
    pub names: Vec<(NameTarget, String)>,
    /// Editor-only rotation in radians; original native orientations stay unchanged.
    pub rotations: Vec<(NameTarget, [f32; 3])>,
    /// Per-axis scale; default omitted to preserve legacy .lckscene files.
    pub scales: Vec<(NameTarget, [f32; 3])>,
    /// Native ANM clip IDs (1-indexed) and 0-indexed frame; editor-only.
    pub animations: Vec<(NameTarget, u8, usize)>,
    /// Appearance override using another real TMD from the same scene pool.
    pub models: Vec<(NameTarget, u8)>,
    pub hidden: Vec<(bool, usize)>,
}

pub(super) fn valid_name(name: &str) -> Result<Option<String>, String> {
    let value = name.trim();
    if value.is_empty() { return Ok(None); }
    if value.chars().count() > 80 || value.len() > 256 ||
        value.chars().any(char::is_control) {
        return Err("Name must contain 1-80 printable characters (at most 256 UTF-8 bytes)".into());
    }
    Ok(Some(value.into()))
}

fn hex_name(name: &str) -> String {
    name.as_bytes().iter().map(|b| format!("{b:02x}")).collect()
}
fn decode_name(hex: &str) -> Result<String, String> {
    if hex.len() % 2 != 0 || hex.len() > 512 { return Err("Invalid encoded name length".into()); }
    let bytes = hex.as_bytes().chunks_exact(2).map(|c| {
        std::str::from_utf8(c).ok().and_then(|h| u8::from_str_radix(h,16).ok())
            .ok_or_else(|| "Invalid hexadecimal name".to_string())
    }).collect::<Result<Vec<u8>,_>>()?;
    let name = String::from_utf8(bytes).map_err(|_| "Name is not UTF-8".to_string())?;
    valid_name(&name)?.ok_or_else(|| "Cannot store an empty name".into())
}

fn native_key(kind: &str, index: &str, world: &WorldModel) -> Result<(bool,usize),String> {
    let terrain = match kind { "terrain"=>true, "placement"=>false,
        _=>return Err("Unknown native record type".into()) };
    let i = index.parse::<usize>().map_err(|_| "Invalid native index".to_string())?;
    if world.at(terrain,i).is_none_or(|d|d.editor_uid.is_some()) {
        return Err("Native object not found in this scene".into());
    }
    Ok((terrain,i))
}

fn kind(terrain: bool) -> &'static str { if terrain { "terrain" } else { "placement" } }

/// Version 1 remains backward compatible with old MOVE/INSTANCE overlays.
/// Names and hidden originals are editor-only metadata, never PS1 writes.
pub(super) fn serialize(scene_name: &str, world: &WorldModel) -> String {
    let mut out = format!("LCK_SCENE_V1\t{scene_name}\n");
    for d in &world.draws {
        if let Some(foreign)=d.editor_uid.and_then(|uid|world.foreign.iter().find(|f|f.uid==uid)) {
            let uid=foreign.uid;
            let center=[0,1,2].map(|axis|d.original_center[axis]+d.offset[axis]);
            out.push_str(&format!("FOREIGN\t{uid}\t{}\t{}\t{}\t{}\t{}\t{}\n",
                hex_name(&foreign.source),foreign.kind,foreign.record,center[0],center[1],center[2]));
            if let Some(name)=&d.display_name {
                out.push_str(&format!("INSTANCE_NAME\t{uid}\t{}\n",hex_name(name)));
            }
            if d.rotation != [0.0;3] {
                out.push_str(&format!("INSTANCE_ROT\t{uid}\t{}\t{}\t{}\n",d.rotation[0],d.rotation[1],d.rotation[2]));
            }
            if d.scale != [1.0;3] {
                out.push_str(&format!("INSTANCE_SCALE\t{uid}\t{}\t{}\t{}\n",d.scale[0],d.scale[1],d.scale[2]));
            }
        } else if let (Some(uid), Some((terrain, index))) = (d.editor_uid, d.source_ref) {
            out.push_str(&format!("INSTANCE\t{uid}\t{}\t{index}\t{}\t{}\t{}\n",
                kind(terrain), d.offset[0], d.offset[1], d.offset[2]));
            if let Some(name) = &d.display_name {
                out.push_str(&format!("INSTANCE_NAME\t{uid}\t{}\n", hex_name(name)));
            }
            if d.rotation != [0.0;3] {
                out.push_str(&format!("INSTANCE_ROT\t{uid}\t{}\t{}\t{}\n",
                    d.rotation[0],d.rotation[1],d.rotation[2]));
            }
            if d.scale != [1.0;3] {
                out.push_str(&format!("INSTANCE_SCALE\t{uid}\t{}\t{}\t{}\n",
                    d.scale[0],d.scale[1],d.scale[2]));
            }
            if let Some(npc)=world.npc((false,d.index)) {
                if npc.model_index!=npc.original_model_index {
                    out.push_str(&format!("INSTANCE_MODEL\t{uid}\t{}\n",npc.model_index));
                }
                out.push_str(&format!("INSTANCE_ANIM\t{uid}\t{}\t{}\n",npc.clip,npc.frame));
            }
        } else if d.source_ref.is_none() {
            if d.offset != [0.0; 3] {
                out.push_str(&format!("MOVE\t{}\t{}\t{}\t{}\t{}\n",
                    kind(d.terrain), d.index, d.offset[0], d.offset[1], d.offset[2]));
            }
            if let Some(name) = &d.display_name {
                out.push_str(&format!("NAME\t{}\t{}\t{}\n", kind(d.terrain),d.index,hex_name(name)));
            }
            if d.rotation != [0.0;3] {
                out.push_str(&format!("ROTATE\t{}\t{}\t{}\t{}\t{}\n",
                    kind(d.terrain),d.index,d.rotation[0],d.rotation[1],d.rotation[2]));
            }
            if d.scale != [1.0;3] {
                out.push_str(&format!("SCALE\t{}\t{}\t{}\t{}\t{}\n",
                    kind(d.terrain),d.index,d.scale[0],d.scale[1],d.scale[2]));
            }
            if let Some(npc)=world.npc((d.terrain,d.index)) {
                if npc.model_index!=npc.original_model_index {
                    out.push_str(&format!("NPC_MODEL\t{}\t{}\t{}\n",
                        kind(d.terrain),d.index,npc.model_index));
                }
                if npc.clip!=npc.anim_id || npc.frame!=0 {
                    out.push_str(&format!("ANIM\t{}\t{}\t{}\t{}\n",
                        kind(d.terrain),d.index,npc.clip,npc.frame));
                }
            }
            if d.hidden { out.push_str(&format!("HIDE\t{}\t{}\n", kind(d.terrain), d.index)); }
        }
    }
    for recipe in world.collision.mesh_recipes.values() {
        out.push_str("MESH_COLLISION\t");
        out.push_str(&serde_json::to_string(recipe).expect("Serializable mesh collision"));out.push('\n');
    }
    for assignment in &world.collision.recipes {
        // JSON is one line; evidence control characters are forbidden. Exact
        // source IDs are persisted, never current draw indices.
        out.push_str("COLLISION\t");
        out.push_str(&serde_json::to_string(assignment).expect("Serializable collision recipe"));
        out.push('\n');
    }
    out
}

pub(super) fn parse(text: &str, scene_name: &str, world: &WorldModel) -> Result<SceneDocument, String> {
    if text.len() > 2_000_000 { return Err("Project overlay exceeds safety limit".into()); }
    let mut lines = text.lines();
    if lines.next() != Some(format!("LCK_SCENE_V1\t{scene_name}").as_str()) {
        return Err("Project overlay belongs to another scene or is not LCK_SCENE_V1".into());
    }
    let mut doc = SceneDocument::default();
    let mut seen_moves = HashSet::new();
    let mut seen_ids = HashSet::new();
    let mut seen_names = HashSet::new();
    let mut seen_hidden = HashSet::new();
    let mut seen_rotations = HashSet::new();
    let mut seen_scales = HashSet::new();
    let mut seen_anims = HashSet::new();
    let mut seen_models = HashSet::new();
    let mut seen_collision=std::collections::BTreeSet::new();
    for (line_index, line) in lines.enumerate() {
        if line.trim().is_empty() { continue; }
        let items: Vec<_> = line.split('\t').collect();
        let at = line_index + 2;
        match items.first().copied() {
            Some("MESH_COLLISION")=>{
                if items.len()!=2 {return Err(format!("Incorrect MESH_COLLISION fields at line {at}"));}
                let recipe:super::collision_generation::MeshOnlyRecipe=serde_json::from_str(items[1]).map_err(|e|e.to_string())?;
                recipe.validate()?;
                if doc.mesh_collision.iter().any(|r|r.uid==recipe.uid) {return Err("Duplicate mesh collision owner".into());}
                doc.mesh_collision.push(recipe);
            }
            Some("COLLISION")=>{
                if items.len()!=2 {return Err(format!("Incorrect COLLISION fields at line {at}"));}
                let assignment:super::collision_recipes::Assignment=serde_json::from_str(items[1])
                    .map_err(|e|format!("Invalid COLLISION at line {at}: {e}"))?;
                assignment.recipe.validate()?;
                if !seen_collision.insert(assignment.owner.clone()) {return Err("Duplicate collision owner".into());}
                doc.collision_recipes.push(assignment);
            }
            Some("HIDE") => {
                if items.len()!=3 { return Err(format!("Incorrect HIDE fields at line {at}")); }
                let key = native_key(items[1],items[2],world)?;
                if !seen_hidden.insert(key) { return Err(format!("Duplicate HIDE at line {at}")); }
                doc.hidden.push(key);
            }
            Some("NAME") => {
                if items.len()!=4 { return Err(format!("Incorrect NAME fields at line {at}")); }
                let (terrain,index)=native_key(items[1],items[2],world)?;
                let target=NameTarget::Native(terrain,index);
                if !seen_names.insert(target.clone()) { return Err(format!("Duplicate NAME at line {at}")); }
                doc.names.push((target,decode_name(items[3])?));
            }
            Some("INSTANCE_NAME") => {
                if items.len()!=3 { return Err(format!("Incorrect INSTANCE_NAME fields at line {at}")); }
                let uid=items[1].parse::<u64>().map_err(|_|format!("Invalid UID at line {at}"))?;
                if uid==0 { return Err(format!("Invalid UID at line {at}")); }
                let target=NameTarget::Created(uid);
                if !seen_names.insert(target.clone()) { return Err(format!("Duplicate INSTANCE_NAME at line {at}")); }
                doc.names.push((target,decode_name(items[2])?));
            }
            Some("ROTATE") | Some("INSTANCE_ROT") => {
                let created=items[0]=="INSTANCE_ROT";
                if items.len() != (if created {5} else {6}) {
                    return Err(format!("Incorrect rotation fields at line {at}"));
                }
                let (target, start)=if created {
                    let uid=items[1].parse::<u64>().map_err(|_|format!("Bad rotation UID at line {at}"))?;
                    if uid==0 { return Err(format!("Invalid rotation UID at line {at}")); }
                    (NameTarget::Created(uid),2)
                } else {
                    let (terrain,index)=native_key(items[1],items[2],world)?;
                    (NameTarget::Native(terrain,index),3)
                };
                if !seen_rotations.insert(target.clone()) {
                    return Err(format!("Duplicate rotation at line {at}"));
                }
                let mut rotation=[0.0;3];
                for (axis,text) in items[start..].iter().enumerate() {
                    rotation[axis]=text.parse::<f32>().map_err(|_|format!("Invalid angle at line {at}"))?;
                }
                if !rotation.iter().all(|v| v.is_finite() && v.abs()<1000.0) {
                    return Err(format!("Rotation out of range at line {at}"));
                }
                doc.rotations.push((target,rotation));
            }
            Some("SCALE") | Some("INSTANCE_SCALE") => {
                let created = items[0] == "INSTANCE_SCALE";
                if items.len() != (if created {5} else {6}) {
                    return Err(format!("Incorrect scale fields at line {at}"));
                }
                let (target,start) = if created {
                    let uid=items[1].parse::<u64>().map_err(|_|format!("Invalid scale UID at line {at}"))?;
                    if uid == 0 { return Err(format!("Invalid scale UID at line {at}")); }
                    (NameTarget::Created(uid),2)
                } else {
                    let (terrain,index)=native_key(items[1],items[2],world)?;
                    (NameTarget::Native(terrain,index),3)
                };
                if !seen_scales.insert(target.clone()) {
                    return Err(format!("Duplicate scale at line {at}"));
                }
                let mut scale=[1.0;3];
                for (axis,text) in items[start..].iter().enumerate() {
                    scale[axis]=text.parse::<f32>().map_err(|_|format!("Invalid scale at line {at}"))?;
                }
                if !scale.iter().all(|v|v.is_finite() && (0.05..=20.0).contains(v)) {
                    return Err(format!("Scale out of range (0.05..20) at line {at}"));
                }
                doc.scales.push((target,scale));
            }
            Some("NPC_MODEL") | Some("INSTANCE_MODEL") => {
                let created=items[0]=="INSTANCE_MODEL";
                let expected=if created {3} else {4};
                if items.len()!=expected {return Err(format!("Incorrect model fields at line {at}"));}
                let (target,start)=if created {
                    let uid=items[1].parse::<u64>().map_err(|_|format!("Invalid model UID at line {at}"))?;
                    if uid==0 {return Err(format!("Invalid model UID at line {at}"));}
                    (NameTarget::Created(uid),2)
                } else {
                    let (terrain,index)=native_key(items[1],items[2],world)?;
                    (NameTarget::Native(terrain,index),3)
                };
                if !seen_models.insert(target.clone()) {return Err(format!("Duplicate model at line {at}"));}
                let index=items[start].parse::<u8>().map_err(|_|format!("Invalid model TMD index at line {at}"))?;
                doc.models.push((target,index));
            }
            Some("ANIM") | Some("INSTANCE_ANIM") => {
                let created=items[0]=="INSTANCE_ANIM";
                let expected=if created {4} else {5};
                if items.len()!=expected {return Err(format!("Incorrect animation fields at line {at}"));}
                let (target, start)=if created {
                    let uid=items[1].parse::<u64>().map_err(|_|format!("Invalid animation UID at line {at}"))?;
                    if uid==0 {return Err(format!("Invalid animation UID at line {at}"));}
                    (NameTarget::Created(uid),2)
                } else {
                    let (terrain,index)=native_key(items[1],items[2],world)?;
                    (NameTarget::Native(terrain,index),3)
                };
                if !seen_anims.insert(target.clone()) {return Err(format!("Duplicate animation at line {at}"));}
                let clip=items[start].parse::<u8>().map_err(|_|format!("Invalid clip at line {at}"))?;
                let frame=items[start+1].parse::<usize>().map_err(|_|format!("Invalid frame at line {at}"))?;
                doc.animations.push((target,clip,frame));
            }
            Some("FOREIGN") => {
                if items.len()!=8{return Err(format!("Incorrect FOREIGN fields at line {at}"));}
                let uid=items[1].parse::<u64>().map_err(|_|format!("Invalid foreign UID at line {at}"))?;
                if uid==0 || !seen_ids.insert(uid){return Err(format!("Duplicate or zero foreign UID at line {at}"));}
                let source=decode_name(items[2])?;
                let kind=items[3];
                if !matches!(kind,"placement"|"terrain"|"mesh") {
                    return Err(format!("Unsupported FOREIGN donor kind at line {at}"));
                }
                let record=items[4].parse::<usize>().map_err(|_|format!("Invalid donor record at line {at}"))?;
                let mut center=[0f32;3];
                for axis in 0..3 {
                    center[axis]=items[5+axis].parse::<f32>().map_err(|_|format!("Invalid foreign coordinate at line {at}"))?;
                }
                if center.iter().any(|v|!v.is_finite()||v.abs()>=1_000_000.0){return Err(format!("Foreign placement out of range at line {at}"));}
                doc.foreign.push(ForeignInstance{uid,source,kind:kind.into(),record,center});
            }
            Some("MOVE") | Some("INSTANCE") => {
                let created = items[0]=="INSTANCE";
                let expected = if created { 7 } else { 6 };
                if items.len()!=expected {
                    return Err(format!("Incorrect field count at line {at}"));
                }
                let (uid, pos) = if created {
                    (Some(items[1].parse::<u64>().map_err(|_| format!("Bad UID at line {at}"))?), 2)
                } else { (None,1) };
                let (terrain,index)=native_key(items[pos],items[pos+1],world)?;
                let mut offset=[0.0_f32;3];
                for (axis,raw) in items[pos+2..].iter().enumerate() {
                    offset[axis]=raw.parse::<f32>().map_err(|_|format!("Invalid coordinate at line {at}"))?;
                }
                if !offset.iter().all(|v|v.is_finite() && v.abs()<1_000_000.0) {
                    return Err(format!("Out-of-range coordinate at line {at}"));
                }
                if let Some(uid)=uid {
                    if uid==0 || !seen_ids.insert(uid) { return Err(format!("Repeated or zero UID at line {at}")); }
                    doc.created.push(NewInstance {uid, source:(terrain,index),offset});
                } else {
                    if !seen_moves.insert((terrain,index)) { return Err(format!("Duplicate MOVE at line {at}")); }
                    doc.moves.push(((terrain,index),offset));
                }
            }
            _ => return Err(format!("Unrecognized record at line {at}")),
        }
        if doc.moves.len()+doc.created.len()+doc.foreign.len()+doc.names.len()+doc.rotations.len()+doc.scales.len()+doc.animations.len()+doc.models.len()+doc.hidden.len()+doc.collision_recipes.len()+doc.mesh_collision.len()>10_000 {
            return Err("Project overlay has too many records".into());
        }
    }
    for recipe in &doc.mesh_collision {
        let foreign=doc.foreign.iter().find(|f|f.uid==recipe.uid).ok_or("Mesh collision references missing foreign owner")?;
        if (foreign.source.as_str(),foreign.kind.as_str(),foreign.record)!=(recipe.scene.as_str(),recipe.kind.as_str(),recipe.record) {
            return Err("Mesh collision donor identity mismatch".into());
        }
        if doc.collision_recipes.iter().any(|a|a.owner==super::collision_recipes::ObjectId::Created(recipe.uid)) {return Err("Conflicting collision recipe representations".into());}
    }
    for target in doc.names.iter().map(|(t,_)|t)
        .chain(doc.rotations.iter().map(|(t,_)|t))
        .chain(doc.scales.iter().map(|(t,_)|t))
        .chain(doc.animations.iter().map(|(t,_,_)|t))
        .chain(doc.models.iter().map(|(t,_)|t)) {
        if let NameTarget::Created(uid)=target {
            if !seen_ids.contains(uid) { return Err(format!("Metadata references missing editor instance #{uid}")); }
        }
    }
    for (target,model) in &doc.models {
        let source=match target {
            NameTarget::Native(terrain,index)=>(*terrain,*index),
            NameTarget::Created(uid)=>doc.created.iter().find(|e|e.uid==*uid)
                .map(|e|e.source).ok_or("Cannot attach MAN model overrides to a foreign editor mesh")?,
        };
        let npc=world.npc(source).ok_or("Appearance target is not an NPC")?;
        if npc.conditional || !npc.drawable || npc.model_pool.get(*model as usize)
            .and_then(|entry|entry.as_ref()).is_none() {
            return Err("NPC appearance override references unavailable TMD".into());
        }
    }
    for (target,clip,frame) in &doc.animations {
        let source=match target {
            NameTarget::Native(terrain,index)=>(*terrain,*index),
            NameTarget::Created(uid)=>doc.created.iter().find(|e|e.uid==*uid)
                .map(|e|e.source).ok_or("Animation references nonexistent copy")?,
        };
        let npc=world.npc(source).ok_or("Animation target is not a field actor")?;
        let actual_model=doc.models.iter().find(|(target_model,_)|target_model==target)
            .map(|(_,model)|*model).unwrap_or(npc.model_index);
        let compatible=if actual_model==npc.model_index {
            npc.frames_for(*clip)
        } else {
            let nobj=npc.model_pool.get(actual_model as usize)
                .and_then(|entry|entry.as_ref()).map(|item|item.1).unwrap_or(0);
            if *clip==0 && nobj==1 {Some(1)}
            else {npc.anm_bank.as_ref().and_then(|bank|
                bank.record((*clip as usize).saturating_sub(1)).ok()
                    .filter(|header|*clip!=0 && header.bone_count as u32==nobj)
                    .map(|header|header.frame_count as usize))}
        };
        if npc.conditional || !npc.drawable || compatible.is_none_or(|count|*frame>=count) {
            return Err("Invalid or incompatible NPC clip/frame in scene file".into());
        }
    }
    Ok(doc)
}

/// Legacy offset documents obey the same all-or-nothing rule as full overlays.
pub(super) fn apply_offsets(world:&mut WorldModel, edits:&[((bool,usize),[f32;3])]) -> Result<(),String> {
    let mut staged=world.clone();
    let keys=staged.draws.iter().filter(|d|d.offset!=[0.0;3])
        .map(|d|(d.terrain,d.index)).collect::<Vec<_>>();
    for key in keys {
        if !staged.set_offset(key.0,key.1,[0.0;3]) {return Err("Cannot reset layout position".into());}
    }
    for &(key,offset) in edits {
        if !staged.set_offset(key.0,key.1,offset) {return Err("Invalid layout target or position".into());}
    }
    *world=staged;
    Ok(())
}

pub(super) fn apply(doc: &SceneDocument, world: &mut WorldModel, native_placements: usize,
    index:&legaia_engine_scene::scene::ProtIndex) -> Result<(), String> {
    let mut staged=world.clone();
    apply_staged(doc,&mut staged,native_placements,index)?;
    *world=staged;
    Ok(())
}

fn apply_staged(doc:&SceneDocument, world:&mut WorldModel, native_placements:usize,
    index:&legaia_engine_scene::scene::ProtIndex) -> Result<(),String> {
    world.collision.restoring=true;
    for assignment in &doc.collision_recipes {
        assignment.recipe.validate()?;
        // Source equality is checked against each restored owner below, after
        // foreign donor provenance has been resolved in the staged document.
    }
    // Resolve and validate every donor BEFORE discarding the current overlay.
    // An absent source cell must not silently wipe the user's edited world.
    let donors=doc.foreign.iter().map(|entry|{
        let donor=legaia_engine_world_views::scene_staging::assemble_field_scene(index,&entry.source)
            .map_err(|error|format!("Could not restore foreign source {}: {error}",entry.source))?;
        super::world_view::foreign_original_geometry(&donor,&entry.kind,entry.record)?;
        Ok::<_,String>(donor)
    }).collect::<Result<Vec<_>,_>>()?;
    // Scene overlays replace, rather than merge, the previous preview edits.
    world.clear_created();
    world.collision.recipes.clear();
    // Restoring all hidden triangles must precede a TMD vertex-count swap;
    // hidden-triangle offsets otherwise refer to the previous index stream.
    for key in world.draws.iter().filter(|d|d.editor_uid.is_none())
        .map(|d|(d.terrain,d.index)).collect::<Vec<_>>() {
        if world.at(key.0,key.1).is_some_and(|d|d.hidden) && !world.set_hidden(key,false) {
            return Err("Could not restore hidden native triangles".into());
        }
    }
    // Reset the preview clips to original first frames, so loading or undoing
    // an older .lckscene deterministically restores the original poses.
    let original: Vec<_> = world.npcs.iter().filter(|n|!n.is_copy)
        .map(|n| ((false,n.key),n.anim_id)).collect();
    for (key,clip) in original {
        world.set_hidden(key,false);
        if let Some(npc)=world.npc(key) {
            let default_model=npc.original_model_index;
            if npc.model_index!=default_model {world.set_npc_model(key,default_model)?;}
        }
        if world.npc(key).is_some_and(|npc|npc.clip!=clip || npc.frame!=0) {
            world.set_npc_animation(key,clip,0)?;
        }
    }
    let keys: Vec<_> = world.draws.iter().map(|d|(d.terrain,d.index)).collect();
    for key in keys {
        world.set_hidden(key,false);
        world.set_display_name(key,None);
        // Only changed fields need resetting; missing/unrenderable original
        // catalog entries do not require fabricated geometry.
        let draw=world.at(key.0,key.1).ok_or("Original instance disappeared")?;
        let (scale,rotation,offset)=(draw.scale,draw.rotation,draw.offset);
        if scale!=[1.0;3] && !world.set_scale(key.0,key.1,[1.0;3]) {
            return Err("Could not reset native scale".into());
        }
        if rotation!=[0.0;3] && !world.set_rotation(key.0,key.1,[0.0;3]) {
            return Err("Could not reset native rotation".into());
        }
        if offset!=[0.0;3] && !world.set_offset(key.0,key.1,[0.0;3]) {
            return Err("Could not reset native position".into());
        }
    }
    for &(key,offset) in &doc.moves {
        if !world.set_offset(key.0,key.1,offset) { return Err("Could not apply a native move".into()); }
    }
    for entry in &doc.created {
        world.duplicate_native(entry.source,native_placements,entry.uid,entry.offset)?;
    }
    for (entry,donor) in doc.foreign.iter().zip(donors.iter()) {
        world.insert_foreign(donor,&entry.kind,entry.record,entry.uid,entry.center,index)?;
    }
    for (target,index) in &doc.models {
        let key=match target {
            NameTarget::Native(terrain,index)=>(*terrain,*index),
            NameTarget::Created(uid)=>world.draws.iter().find(|d|d.editor_uid==Some(*uid))
                .map(|d|(d.terrain,d.index)).ok_or("Missing copy for model")?,
        };
        world.set_npc_model(key,*index)?;
    }
    for (target,clip,frame) in &doc.animations {
        let key=match target {
            NameTarget::Native(terrain,index)=>(*terrain,*index),
            NameTarget::Created(uid)=>world.draws.iter().find(|d|d.editor_uid==Some(*uid))
                .map(|d|(d.terrain,d.index)).ok_or("Missing copy for animation")?,
        };
        world.set_npc_animation(key,*clip,*frame)?;
    }
    for (target,name) in &doc.names {
        let key=match target {
            NameTarget::Native(terrain,index)=>(*terrain,*index),
            NameTarget::Created(uid) => world.draws.iter().find(|d|d.editor_uid==Some(*uid))
                .map(|d|(d.terrain,d.index)).ok_or("Missing created instance for name")?,
        };
        if !world.set_display_name(key,Some(name.clone())) { return Err("Cannot set scene object name".into()); }
    }
    for (target, rotation) in &doc.rotations {
        let key=match target {
            NameTarget::Native(terrain,index)=>(*terrain,*index),
            NameTarget::Created(uid)=>world.draws.iter().find(|d| d.editor_uid==Some(*uid))
                .map(|d|(d.terrain,d.index)).ok_or("Missing created instance for rotation")?,
        };
        if !world.set_rotation(key.0,key.1,*rotation) {
            return Err("Could not apply scene-instance rotation".into());
        }
    }
    for (target, scale) in &doc.scales {
        let key=match target {
            NameTarget::Native(terrain,index)=>(*terrain,*index),
            NameTarget::Created(uid)=>world.draws.iter().find(|d|d.editor_uid==Some(*uid))
                .map(|d|(d.terrain,d.index)).ok_or("Missing created instance for scale")?,
        };
        if !world.set_scale(key.0,key.1,*scale) {
            return Err("Could not apply scene-instance scale".into());
        }
    }
    for &key in &doc.hidden {
        if !world.set_hidden(key,true) { return Err("Cannot hide native scene object".into()); }
    }
    let mut collision_owners=std::collections::BTreeSet::new();
    // Creation defaults apply to new editor actions, never overwrite the
    // exact recipe set (including deliberate absence) in a saved document.
    world.collision.recipes.clear();
    world.collision.mesh_recipes.clear();
    for recipe in &doc.mesh_collision {
        recipe.validate()?;
        let foreign=world.foreign.iter().find(|f|f.uid==recipe.uid).ok_or("Mesh collision owner missing after restore")?;
        if foreign.source!=recipe.scene || foreign.kind!=recipe.kind || foreign.record!=recipe.record ||
            super::collision_recipes::digest(&foreign.raw_tmd)!=recipe.tmd_sha256 {
            return Err("Mesh collision source TMD changed".into());
        }
        if world.collision.mesh_recipes.insert(recipe.uid,recipe.clone()).is_some() {return Err("Duplicate mesh collision owner".into());}
    }
    for assignment in &doc.collision_recipes {
        if !collision_owners.insert(assignment.owner.clone()) {return Err("Duplicate collision owner".into());}
        let key=world.draws.iter().find_map(|draw| {
            let key=(draw.terrain,draw.index);
            (world.collision.owner(world,key).as_ref()==Some(&assignment.owner)).then_some(key)
        }).ok_or("Collision recipe owner is missing")?;
        world.set_collision_recipe(key,assignment.recipe.clone())?;
    }
    world.collision.restoring=false;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::world_view::{DrawInfo, WorldModel};

    fn test_world() -> WorldModel {
        WorldModel {
            model: None, instances: 1, unavailable: 0, unique_models: 1,
            ground_triangles: 0, ground_vram_covered: 0, ground_textured: 0,
            npcs:Vec::new(),npc_issue:None,foreign:Vec::new(),collision:Default::default(),foreign_vram:std::collections::HashMap::new(),
            draws: vec![DrawInfo {
                id: 0, terrain: false, index: 0, center:[0.0;3], radius: 1.0,
                original_center:[0.0;3], offset:[0.0;3], rotation:[0.0;3],scale:[1.0;3], vertex_start:0,vertex_end:3,
                source_ref:None, editor_uid:None,
                display_name:None, hidden:false, hidden_triangles:Vec::new(),
            }],
        }
    }

    #[test]
    fn automatic_collision_is_independent_saved_and_not_regenerated_over_edits() {
        use super::super::collision_recipes::{self,ObjectId,Evidence};
        let mut world=test_world();
        let positions=vec![[256.0,0.0,256.0],[256.0,128.0,256.0],[256.0,0.0,384.0]];
        world.model=Some(crate::raster::Model{rest:positions.clone(),positions,indices:vec![0,1,2],
            object_ids:vec![0;3],uvs:vec![[0,0];3],cba_tsb:vec![[0,0];3],modulation:vec![[128;3];3],
            fill:vec![[0;3];3],textured:vec![0;3],center:[0.0;3],radius:512.0});
        let (map,assignment)=collision_recipes::fixture();
        world.collision.sources.insert((false,0),assignment.recipe.source);
        let first=world.duplicate_native((false,0),1,90,[64.0,0.0,0.0]).unwrap();
        let original_source=world.collision.sources[&(false,0)].object.clone();
        assert!(!world.collision.recipes.iter().any(|r|r.owner==ObjectId::Native(original_source.clone())));
        let automatic=world.collision.recipes[0].recipe.clone();
        assert!(matches!(automatic.evidence,Evidence::MeshDerived{..}));assert!(automatic.generated.is_some());
        assert_eq!(world.collision.generated_bases.len(),1);
        let second=world.duplicate_native((false,0),1,91,[128.0,0.0,0.0]).unwrap();
        assert_eq!(world.collision.generated_bases.len(),1);
        let mut edited=automatic.clone();edited.evidence=Evidence::UserAuthored{note:"Corrected doorway".into()};
        edited.nonblocking=vec![[4,6]];
        world.set_collision_recipe(first,edited.clone()).unwrap();
        world.ensure_automatic_collision(first).unwrap();
        world.inherit_collision_recipe(first,second).unwrap();
        assert_eq!(world.collision.recipes.iter().find(|a|a.owner==ObjectId::Created(91)).unwrap().recipe,edited);
        let index=legaia_engine_scene::scene::ProtIndex::from_bytes(legaia_disc_patch::disc::synth::synth_prot(&[]),None).unwrap();
        let saved=serialize("test",&world);let doc=parse(&saved,"test",&world).unwrap();
        world.delete_created(first).unwrap();
        apply(&doc,&mut world,1,&index).unwrap();assert_eq!(serialize("test",&world),saved);
        let mut bad=doc.clone();bad.collision_recipes[0].recipe.generated.as_mut().unwrap().mesh_sha256="0".repeat(64);
        assert!(apply(&bad,&mut world,1,&index).is_err());assert_eq!(serialize("test",&world),saved);
        // Remove all corrections, then validate generated native additions.
        for a in &mut world.collision.recipes {a.recipe.nonblocking.clear();}
        let output=world.compile_authored_collision("test",0,&map).unwrap();
        for i in 0x4000..0x8000 {assert_eq!(output[i]&map[i],map[i]);assert_eq!(output[i]&15,map[i]&15);}
        assert!(world.set_rotation(second.0,second.1,[0.0,0.4,0.0]));
        assert!(world.set_scale(second.0,second.1,[1.2,1.0,1.0]));
        assert!(world.compile_authored_collision("test",0,&map).is_ok());
    }
    #[test]
    fn oversized_collision_copy_is_rejected_without_losing_document() {
        use super::super::collision_recipes;
        let mut world=test_world();
        world.model=Some(crate::raster::Model {
            positions:vec![[1.0,2.0,3.0];3],rest:vec![[1.0,2.0,3.0];3],
            object_ids:vec![0;3],uvs:vec![[0,0];3],cba_tsb:vec![[0,0];3],
            modulation:vec![[128;3];3],fill:vec![[0;3];3],textured:vec![0;3],
            indices:vec![0,1,2],center:[0.0;3],radius:1.0,
        });
        let (_,mut assignment)=collision_recipes::fixture();
        world.collision.sources.insert((false,0),assignment.recipe.source.clone());
        assignment.recipe.walls=(0..256).flat_map(|x|(0..256).map(move|z|[x,z])).collect();
        world.set_collision_recipe((false,0),assignment.recipe).unwrap();
        let mut blocked=false;
        for uid in 1..10 {
            let before=serialize("test",&world);let vertices=world.model.as_ref().unwrap().positions.clone();
            if world.duplicate_native((false,0),1,uid,[0.0;3]).is_err() {
                assert_eq!(serialize("test",&world),before);
                assert_eq!(world.model.as_ref().unwrap().positions,vertices);
                blocked=true;break;
            }
        }
        assert!(blocked,"File size guard must reject before an unreadable overlay can be saved");
    }

    #[test]
    fn collision_recipes_roundtrip_copy_edit_delete_and_history_independently() {
        use super::super::collision_recipes::{self,ObjectId,Evidence};
        let mut world=test_world();
        world.model=Some(crate::raster::Model {
            positions:vec![[1.0,2.0,3.0];3],rest:vec![[1.0,2.0,3.0];3],
            object_ids:vec![0;3],uvs:vec![[0,0];3],cba_tsb:vec![[0,0];3],
            modulation:vec![[128;3];3],fill:vec![[0;3];3],textured:vec![0;3],
            indices:vec![0,1,2],center:[0.0;3],radius:1.0,
        });
        let (_,assignment)=collision_recipes::fixture();
        world.collision.sources.insert((false,0),assignment.recipe.source.clone());
        world.set_collision_recipe((false,0),assignment.recipe.clone()).unwrap();
        let index=legaia_engine_scene::scene::ProtIndex::from_bytes(
            legaia_disc_patch::disc::synth::synth_prot(&[]),None).unwrap();
        let original=serialize("test",&world);
        let copy=world.duplicate_native((false,0),1,9,[64.0,0.0,0.0]).unwrap();
        assert_eq!(world.collision.recipes.len(),2);
        let mut recipe=assignment.recipe.clone();recipe.walls=vec![[8,9]];
        world.set_collision_recipe(copy,recipe).unwrap();
        let copy2=world.duplicate_native((false,0),1,10,[128.0,0.0,0.0]).unwrap();
        world.inherit_collision_recipe(copy,copy2).unwrap();
        assert_eq!(world.collision.recipes.iter().find(|a|a.owner==ObjectId::Created(10)).unwrap().recipe.walls,vec![[8,9]]);
        world.remove_collision_recipe(copy2).unwrap();
        let copy3=world.duplicate_native((false,0),1,11,[128.0,0.0,0.0]).unwrap();
        world.inherit_collision_recipe(copy2,copy3).unwrap();
        assert!(!world.collision.recipes.iter().any(|a|a.owner==ObjectId::Created(11)));
        world.delete_created(copy3).unwrap();world.delete_created(copy2).unwrap();
        assert_eq!(world.collision.recipes.iter().find(|a|matches!(a.owner,ObjectId::Native(_))).unwrap().recipe,assignment.recipe);
        assert_eq!(world.collision.recipes.iter().find(|a|a.owner==ObjectId::Created(9)).unwrap().recipe.source,assignment.recipe.source);
        let history=super::super::editor_history_capture(&world,"test",Some(copy));
        let saved=serialize("test",&world);
        let parsed=parse(&saved,"test",&world).unwrap();
        assert!(matches!(parsed.collision_recipes[0].recipe.evidence,Evidence::UserAuthored{..}));
        apply(&parsed,&mut world,1,&index).unwrap();assert_eq!(serialize("test",&world),saved);
        world.delete_created(copy).unwrap();assert_eq!(world.collision.recipes.len(),1);
        let undo=parse(&history.overlay,"test",&world).unwrap();
        apply(&undo,&mut world,1,&index).unwrap();
        assert_eq!(serialize("test",&world),saved);
        let original_doc=parse(&original,"test",&world).unwrap();
        apply(&original_doc,&mut world,1,&index).unwrap();
        assert_eq!(serialize("test",&world),original);
        apply(&undo,&mut world,1,&index).unwrap();assert_eq!(serialize("test",&world),saved);

        let mut stale=undo.clone();stale.collision_recipes[0].recipe.source.map_sha256="0".repeat(64);
        assert!(apply(&stale,&mut world,1,&index).is_err());assert_eq!(serialize("test",&world),saved);
        let mut missing=undo.clone();missing.collision_recipes[1].owner=ObjectId::Created(999);
        assert!(apply(&missing,&mut world,1,&index).is_err());assert_eq!(serialize("test",&world),saved);
        let mut late=undo.clone();late.names.push((NameTarget::Created(999),"Invalid late name".into()));
        assert!(apply(&late,&mut world,1,&index).is_err());assert_eq!(serialize("test",&world),saved);
        assert!(parse(&format!("{saved}{}",saved.lines().find(|s|s.starts_with("COLLISION")).unwrap()),"test",&world).is_err());
        let (_,map_assignment)=collision_recipes::fixture();
        let mut authored_only=test_world();
        authored_only.collision.sources.insert((false,0),map_assignment.recipe.source.clone());
        authored_only.set_collision_recipe((false,0),map_assignment.recipe.clone()).unwrap();
        let (native,_)=collision_recipes::fixture();
        assert_ne!(authored_only.compile_authored_collision("test",0,&native).unwrap(),native);
        authored_only.draws[0].hidden=true;
        assert_eq!(authored_only.compile_authored_collision("test",0,&native).unwrap(),native);
        // Native observations remain distinct after persistence and never
        // become authored ownership merely by saving/loading.
        let mut observed=map_assignment.recipe;
        observed.evidence=Evidence::NativeObserved{evidence:"Verified descriptor; walls remain unknown".into()};
        authored_only.set_collision_recipe((false,0),observed).unwrap();
        let observations=parse(&serialize("test",&authored_only),"test",&authored_only).unwrap();
        assert!(matches!(observations.collision_recipes[0].recipe.evidence,Evidence::NativeObserved{..}));
    }

    #[test]
    fn native_recipe_identity_survives_adapter_index_change() {
        use super::super::collision_recipes;
        let mut world=test_world();let (_,assignment)=collision_recipes::fixture();
        world.collision.sources.insert((false,0),assignment.recipe.source.clone());
        world.set_collision_recipe((false,0),assignment.recipe).unwrap();
        let saved=serialize("test",&world);
        let source=world.collision.sources.remove(&(false,0)).unwrap();
        world.collision.sources.insert((false,7),source);world.draws[0].index=7;
        let parsed=parse(&saved,"test",&world).unwrap();
        let index=legaia_engine_scene::scene::ProtIndex::from_bytes(
            legaia_disc_patch::disc::synth::synth_prot(&[]),None).unwrap();
        apply(&parsed,&mut world,1,&index).unwrap();
        assert_eq!(serialize("test",&world),saved);
    }

    #[test]
    fn late_restore_failure_preserves_instances_donors_geometry_and_document() {
        let mut world=test_world();
        world.model=Some(crate::raster::Model {
            positions:vec![[1.0,2.0,3.0];3],rest:vec![[1.0,2.0,3.0];3],
            object_ids:vec![0;3],uvs:vec![[0,0];3],cba_tsb:vec![[0,0];3],
            modulation:vec![[128;3];3],fill:vec![[0;3];3],textured:vec![0;3],
            indices:vec![0,1,2],center:[0.0;3],radius:1.0,
        });
        world.set_offset(false,0,[20.0,0.0,0.0]);
        world.duplicate_native((false,0),1,9,[40.0,0.0,0.0]).unwrap();
        world.foreign.push(super::super::world_view::ForeignRef {
            uid:9,source:"donor".into(),kind:"mesh".into(),record:2,
            raw_tmd:vec![1,2,3],native_record:Some([7;32]),donor_wall_bits:vec![(2,3)],donor_center:[0.0;3],
        });
        let texture=std::sync::Arc::new(legaia_tim::Vram::new());
        world.foreign_vram.insert(1,texture.clone());
        let overlay=serialize("test",&world);
        let positions=world.model.as_ref().unwrap().positions.clone();
        let refs=world.foreign.clone();
        let index=legaia_engine_scene::scene::ProtIndex::from_bytes(
            legaia_disc_patch::disc::synth::synth_prot(&[]),None).unwrap();
        // Reset and a valid movement occur before an invalid name target fails.
        let doc=SceneDocument {moves:vec![((false,0),[100.0,0.0,0.0])],
            names:vec![(NameTarget::Created(999),"bad".into())],..Default::default()};
        assert!(apply(&doc,&mut world,1,&index).is_err());
        assert_eq!(serialize("test",&world),overlay);
        assert_eq!(world.instances,2);
        assert_eq!(world.foreign,refs);
        assert!(std::sync::Arc::ptr_eq(&world.foreign_vram[&1],&texture));
        assert_eq!(world.model.as_ref().unwrap().positions,positions);
        assert!(apply_offsets(&mut world,&[((false,0),[80.0,0.0,0.0]),((false,999),[0.0;3])]).is_err());
        assert_eq!(serialize("test",&world),overlay);
        assert_eq!(world.model.as_ref().unwrap().positions,positions);
        let valid=SceneDocument {moves:vec![((false,0),[100.0,0.0,0.0])],..Default::default()};
        apply(&valid,&mut world,1,&index).unwrap();
        assert_eq!(world.draws[0].offset,[100.0,0.0,0.0]);
        assert!(world.foreign.is_empty());
    }

    #[test]
    fn foreign_sources_have_unique_ids_and_roundtrip_without_native_aliasing() {
        let world=test_world();
        let txt="LCK_SCENE_V1\tcave01\nFOREIGN\t77\t62616c64656e\tmesh\t2\t25\t30\t45\nINSTANCE_ROT\t77\t0\t1\t0\n";
        let doc=parse(txt,"cave01",&world).unwrap();
        assert_eq!(doc.foreign.len(),1);
        assert_eq!(doc.foreign[0].source,"balden");
        assert_eq!(doc.foreign[0].center,[25.0,30.0,45.0]);
        assert_eq!(doc.rotations.len(),1);
        assert!(parse(&format!("{txt}FOREIGN\t77\t62616c64656e\tmesh\t1\t0\t0\t0\n"),"cave01",&world).is_err());
        assert!(parse("LCK_SCENE_V1\tcave01\nFOREIGN\t1\t62616c64656e\tactor\t0\t0\t0\t0\n","cave01",&world).is_err());
    }
    #[test]
    fn npc_clip_validation_rejects_malformed_and_non_npc_targets() {
        let world=test_world();
        assert!(parse("LCK_SCENE_V1\tcave01\nANIM\tplacement\t0\t2\t0\n",
            "cave01", &world).is_err());
        assert!(parse("LCK_SCENE_V1\tcave01\nINSTANCE_ANIM\t1\t3\t0\n",
            "cave01", &world).is_err());
        assert!(parse("LCK_SCENE_V1\tcave01\nANIM\tplacement\t0\tNaN\t0\n",
            "cave01", &world).is_err());
    }
    #[test]
    fn scale_syntax_is_backward_compatible_and_rejects_invalid_values() {
        let world=test_world();
        let older=parse("LCK_SCENE_V1\tcave01\nMOVE\tplacement\t0\t2\t3\t4\n", "cave01", &world).unwrap();
        assert!(older.scales.is_empty());
        let txt="LCK_SCENE_V1\tcave01\nINSTANCE\t8\tplacement\t0\t0\t0\t0\nSCALE\tplacement\t0\t2\t1.5\t0.5\nINSTANCE_SCALE\t8\t1\t2\t3\n";
        let parsed=parse(txt,"cave01",&world).unwrap();
        assert_eq!(parsed.scales.len(),2);
        assert_eq!(parsed.scales[0],(NameTarget::Native(false,0),[2.0,1.5,0.5]));
        assert_eq!(parsed.scales[1],(NameTarget::Created(8),[1.0,2.0,3.0]));
        assert!(parse("LCK_SCENE_V1\tcave01\nSCALE\tplacement\t0\tNaN\t1\t1\n", "cave01", &world).is_err());
        assert!(parse("LCK_SCENE_V1\tcave01\nSCALE\tplacement\t0\t0\t1\t1\n", "cave01", &world).is_err());
        assert!(parse("LCK_SCENE_V1\tcave01\nINSTANCE_SCALE\t44\t1\t1\t1\n", "cave01", &world).is_err());
        let mut edited=test_world(); edited.draws[0].scale=[1.25,2.0,0.75];
        let saved=serialize("cave01",&edited);
        assert!(saved.contains("SCALE\tplacement\t0\t1.25\t2\t0.75"));
        assert_eq!(parse(&saved,"cave01",&world).unwrap().scales[0].1,[1.25,2.0,0.75]);
    }
    #[test]
    fn rotation_overlay_parses_and_validates_old_and_new_syntax() {
        let world=test_world();
        let text="LCK_SCENE_V1\tcave01\nINSTANCE\t9\tplacement\t0\t3\t0\t0\nROTATE\tplacement\t0\t0.25\t0.75\t0\nINSTANCE_ROT\t9\t0\t1.5\t0\n";
        let doc=parse(text,"cave01",&world).unwrap();
        assert_eq!(doc.rotations.len(),2);
        assert_eq!(doc.rotations[0],(NameTarget::Native(false,0),[0.25,0.75,0.0]));
        assert_eq!(doc.rotations[1],(NameTarget::Created(9),[0.0,1.5,0.0]));
        let old=parse("LCK_SCENE_V1\tcave01\nMOVE\tplacement\t0\t3\t2\t1\n", "cave01",&world).unwrap();
        assert!(old.rotations.is_empty());
        assert!(parse("LCK_SCENE_V1\tcave01\nINSTANCE_ROT\t999\t0\t1\t0\n", "cave01",&world).is_err());
        assert!(parse("LCK_SCENE_V1\tcave01\nROTATE\tplacement\t0\tNaN\t0\t0\n", "cave01",&world).is_err());
    }
    #[test]
    fn rotation_serialization_roundtrips_without_changing_old_document_version() {
        let mut world=test_world();
        world.draws[0].rotation=[0.125,0.5,-0.25];
        let saved=serialize("cave01",&world);
        assert!(saved.starts_with("LCK_SCENE_V1\tcave01\n"));
        assert!(saved.contains("ROTATE\tplacement\t0\t0.125\t0.5\t-0.25"));
        let doc=parse(&saved,"cave01",&test_world()).unwrap();
        assert_eq!(doc.rotations,vec![(NameTarget::Native(false,0),[0.125,0.5,-0.25])]);
    }
    #[test]
    fn parse_and_serialize_editor_scene_records() {
        let world = test_world();
        let text = "LCK_SCENE_V1\tcave01\nMOVE\tplacement\t0\t2\t3\t4\nINSTANCE\t8\tplacement\t0\t100\t0\t-1\n";
        let doc = parse(text, "cave01", &world).unwrap();
        assert_eq!(doc.moves,vec![((false,0),[2.0,3.0,4.0])]);
        assert_eq!(doc.created,vec![NewInstance {uid:8, source:(false,0), offset:[100.0,0.0,-1.0]}]);
        assert!(parse(text,"town01",&world).is_err());
        let empty = serialize("cave01", &world);
        assert_eq!(empty,"LCK_SCENE_V1\tcave01\n");
    }

    #[test]
    fn rename_and_native_hide_roundtrip_in_overlay() {
        let mut world=test_world();
        world.set_display_name((false,0),Some("Conkram / gate 🏛".to_string()));
        // The serialized visibility marker does not require the game disc.
        world.draws[0].hidden=true;
        let txt=serialize("conkram",&world);
        assert!(txt.contains("NAME\tplacement\t0\t"));
        assert!(txt.contains("HIDE\tplacement\t0"));
        let doc=parse(&txt,"conkram",&test_world()).unwrap();
        assert_eq!(doc.names,vec![(NameTarget::Native(false,0),"Conkram / gate 🏛".to_string())]);
        assert_eq!(doc.hidden,vec![(false,0)]);
        let duplicate="LCK_SCENE_V1\tconkram\nINSTANCE\t9\tplacement\t0\t0\t0\t0\nINSTANCE_NAME\t9\t67617465\n";
        let named=parse(duplicate,"conkram",&test_world()).unwrap();
        assert_eq!(named.names,vec![(NameTarget::Created(9),"gate".into())]);
        assert!(parse("LCK_SCENE_V1\tconkram\nINSTANCE_NAME\t9\t67617465\n", "conkram",&test_world()).is_err());
        assert!(parse("LCK_SCENE_V1\tconkram\nNAME\tplacement\t0\tzz\n", "conkram",&test_world()).is_err());
        assert!(valid_name("\u{0001}bad").is_err());
    }

    #[test]
    fn invalid_duplicate_or_non_finite_coordinates_fail_without_edits() {
        let world = test_world();
        assert!(parse("LCK_SCENE_V1\tcave01\nINSTANCE\t1\tplacement\t0\t0\t0\t0\nINSTANCE\t1\tplacement\t0\t0\t0\t0\n","cave01",&world).is_err());
        assert!(parse("LCK_SCENE_V1\tcave01\nMOVE\tplacement\t0\tNaN\t0\t0\n","cave01",&world).is_err());
        assert!(parse("LCK_SCENE_V1\tcave01\nMOVE\tplacement\t1\t0\t0\t0\n","cave01",&world).is_err());
    }
}
