//! Authentic field actor placement rendering for LCK.
//! All content is read from the selected disc: MAN partition-1 positions,
//! scene-local TMDs, ANM initial poses, and the assembled scene's VRAM.
//! No invented NPC geometry, assets, or script state is introduced.
use super::{bounds, DrawInfo, NPC_KEY_BASE, editor_rotate_vertex, WorldModel};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use crate::raster::Model;
use legaia_engine_scene::npc_catalog::{catalog_scene_npcs, scene_anm_bundle};
use legaia_engine_scene::scene::{ProtIndex, Scene};
use legaia_engine_scene::scene_assembly::AssembledScene;

/// A native MAN actor record, kept even when its display is conditionally
/// suppressed or its mesh cannot be decoded. Names are NOT inferred from dialog.
#[derive(Clone)]
pub(crate) struct NpcInfo {
    pub(crate) key: usize,
    pub(crate) record: usize,
    pub(crate) model_index: u8,
    pub(crate) original_model_index: u8,
    pub(crate) nobj: u32,
    /// Shared list of original models available in this cell for appearance override.
    pub(crate) model_pool: Arc<Vec<Option<(Arc<[u8]>,u32)>>> ,
    pub(crate) anim_id: u8,
    pub(crate) kind: &'static str,
    pub(crate) dialog: Option<String>,
    pub(crate) conditional: bool,
    pub(crate) posed: bool,
    pub(crate) drawable: bool,
    /// Metadata for a duplicated, independent editor instance.
    pub(crate) is_copy: bool,
    pub(crate) clip: u8,
    pub(crate) frame: usize,
    /// Scene-native clips whose bone counts match this character model.
    pub(crate) clips: Vec<(u8, usize)>,
    /// Original PS1 source content, shared by Arc without copying per frame.
    pub(crate) raw_tmd: Option<Arc<[u8]>>,
    pub(crate) anm_bank: Option<Arc<legaia_actor_formats::player_anm::PlayerAnmBundle>>,
    /// Shared decoded clips: playing at 15 fps does not re-decompress the ANM.
    animation_cache: Arc<Mutex<HashMap<u8,Arc<legaia_actor_formats::monster_archive::MonsterAnimation>>>>,
    /// Native PSX coordinates, with +Y down.
    pub(crate) psx_position: [f32; 3],
}

/// Interpolate the four native MAP corners. Keep this separate from the
/// scene loader so slope interpolation can be regression-tested without disc IO.
fn bilerp_floor(v: &[[f32;3]], x: f32, z: f32) -> f32 {
    let tx = ((x-v[0][0])/(v[1][0]-v[0][0]).max(1.0)).clamp(0.0,1.0);
    let tz = ((z-v[0][2])/(v[2][2]-v[0][2]).max(1.0)).clamp(0.0,1.0);
    v[0][1]*(1.0-tx)*(1.0-tz) + v[1][1]*tx*(1.0-tz)
        + v[2][1]*(1.0-tx)*tz + v[3][1]*tx*tz
}

/// Sample the original walk-floor quad at the NPC's spawn location. The MAP
/// heightfield emits four ordered corners for each 128-unit floor tile.
/// All data stays in original PSX Y-down coordinates until the render flip.
/// A scene lacking a walk-heightfield defaults to the zero floor, visibly
/// marked in the native actor's source metadata rather than a fabricated one.
fn floor_y(scene: &AssembledScene, x: f32, z: f32) -> f32 {
    let Some(hf) = scene.ground.as_ref() else { return 0.0; };
    for v in hf.positions.chunks_exact(4) {
        let x0 = v[0][0];
        let x1 = v[1][0];
        let z0 = v[0][2];
        let z1 = v[2][2];
        if x >= x0 && x <= x1 && z >= z0 && z <= z1 {
            return bilerp_floor(v,x,z);
        }
    }
    0.0
}

/// Append static first-frame NPC meshes into the same raster stream as world
/// geometry, preserving one object ID per native MAN actor for click-select.
/// Conditional spawns are indexed but NOT shown at their off-map sentinel;
/// special party/savepoint pool models and currently unposable multi-object
/// actors are explicitly counted by the catalog, not faked with placeholders.
pub(super) fn append_scene_npcs(
    assembled: &AssembledScene,
    index: &ProtIndex,
    combined: &mut Model,
    draws: &mut Vec<DrawInfo>,
) -> (Vec<NpcInfo>, Option<String>) {
    let catalog = match catalog_scene_npcs(index, &assembled.name, &assembled.res, None) {
        Ok(c) => c,
        Err(e) => return (Vec::new(), Some(e)),
    };
    let issue = if catalog.special_count != 0 || catalog.unposable_count != 0 {
        Some(format!("{} special global-pool actors; {} unposable multi-object actors omitted",
            catalog.special_count,catalog.unposable_count))
    } else {None};
    let anim_bank = Scene::load(index, &assembled.name).ok()
        .and_then(|s|scene_anm_bundle(&s)).map(Arc::new);
    let model_pool = Arc::new(assembled.res.tmds.iter().take(240).map(|t| {
        Some((Arc::<[u8]>::from(t.raw.as_slice()),t.tmd.objects.len() as u32))
    }).collect::<Vec<_>>());
    let mut clips_cache: HashMap<u32, Vec<(u8, usize)>> = HashMap::new();
    let animation_cache=Arc::new(Mutex::new(HashMap::new()));
    let mut npcs=Vec::with_capacity(catalog.entries.len());
    for entry in catalog.entries {
        let p = &entry.placement;
        let key = NPC_KEY_BASE + p.index;
        let psx_x = f32::from(p.world_x);
        let psx_z = f32::from(p.world_z);
        let psx_y = if entry.conditional {0.0} else {floor_y(assembled,psx_x,psx_z)};
        let psx_position = [psx_x, psx_y, psx_z];
        let raw_tmd = model_pool.get(p.model_index as usize)
            .and_then(|item|item.as_ref().map(|(raw,_)|raw.clone()));
        let clips = clips_cache.entry(entry.nobj)
            .or_insert_with(||compatible_clips(anim_bank.as_deref(),entry.nobj)).clone();
        let mut posed = false;
        let model = if entry.conditional {None} else {
            assembled.res.tmds.get(p.model_index as usize)
                .and_then(|rtmd|Model::from_tmd(&rtmd.raw))
                .map(|mut mesh| {
                    if let Some(anm) = anim_bank.as_ref()
                        && let Some(record) = (p.anim_id as usize).checked_sub(1)
                        && let Ok(header) = anm.record(record)
                        && header.bone_count as u32 == entry.nobj
                        && let Some(clip) = anm.record_to_monster_animation(record)
                        && let Some(frame) = clip.frames.first() {
                            mesh.pose(frame);
                            posed = true;
                    }
                    mesh
                })
        };
        let renderable = model.as_ref().is_some_and(|m| {
            !m.indices.is_empty() && !m.positions.is_empty()
        });
        let mut vertex_start=0;
        let mut vertex_end=0;
        let mut center=[psx_x,-psx_y,psx_z];
        let mut radius=0.0;
        let id = draws.len() as u32;
        if let Some(mesh) = model.filter(|_|renderable) {
            if let (Ok(base),Ok(count)) = (u32::try_from(combined.positions.len()),
                                            u32::try_from(mesh.positions.len())) {
                if base.checked_add(count).is_some() &&
                   mesh.indices.iter().all(|&i|i < count) {
                    vertex_start=base as usize;
                    vertex_end=vertex_start+mesh.positions.len();
                    let positions: Vec<_>=mesh.positions.iter().map(|v| [
                        v[0]+psx_x, -v[1]-psx_y, v[2]+psx_z
                    ]).collect();
                    (center,radius)=bounds(&positions);
                    combined.positions.extend_from_slice(&positions);
                    combined.indices.extend(mesh.indices.iter().map(|&i|base+i));
                    combined.object_ids.extend(std::iter::repeat_n(id,positions.len()));
                    combined.uvs.extend_from_slice(&mesh.uvs);
                    combined.cba_tsb.extend_from_slice(&mesh.cba_tsb);
                    combined.modulation.extend_from_slice(&mesh.modulation);
                    combined.fill.extend_from_slice(&mesh.fill);
                    combined.textured.extend_from_slice(&mesh.textured);
                }
            }
        }
        let drawable = vertex_end > vertex_start;
        // Even an unrendered record has a stable hierarchy key; its position
        // remains inspectable and no fake triangles appear on screen.
        draws.push(DrawInfo {
            id, terrain:false, index:key, center, original_center:center,
            radius, offset:[0.0;3], rotation:[0.0;3], scale:[1.0;3],
            vertex_start, vertex_end, source_ref:None, editor_uid:None,
            display_name:None, hidden:false, hidden_triangles:Vec::new(),
        });
        npcs.push(NpcInfo {
            key,record:p.index,model_index:p.model_index,original_model_index:p.model_index,
            model_pool:model_pool.clone(),nobj:entry.nobj,anim_id:p.anim_id,
            kind:entry.kind, dialog:entry.dialog,conditional:entry.conditional,
            posed, drawable,psx_position, is_copy:false, clip:p.anim_id, frame:0,
            clips, raw_tmd, anm_bank:anim_bank.clone(),
            animation_cache:animation_cache.clone(),
        });
    }
    (npcs,issue)
}

/// Inspect source ANM headers first; only mesh-compatible clips are selectable.
fn compatible_clips(bank: Option<&legaia_actor_formats::player_anm::PlayerAnmBundle>,
                    nobj:u32)->Vec<(u8,usize)> {
    let mut result=Vec::new();
    if let Some(bank)=bank {
        for id in 1..=bank.record_count.min(255) {
            let slot=(id-1) as usize;
            if let Ok(header)=bank.record(slot)
                && header.bone_count as u32 == nobj
                && header.frame_count > 0 {
                result.push((id as u8,header.frame_count as usize));
            }
        }
    }
    result
}

impl NpcInfo {
    fn action(&self, clip: u8) -> Result<Arc<legaia_actor_formats::monster_archive::MonsterAnimation>,String> {
        if clip==0 {return Err("Zero is rest, not an ANM clip".into());}
        let mut cache=self.animation_cache.lock().map_err(|_|"Animation cache lock was poisoned")?;
        if let Some(action)=cache.get(&clip) {return Ok(action.clone());}
        let bank=self.anm_bank.as_ref().ok_or("No scene animation bundle")?;
        let action=Arc::new(bank.record_to_monster_animation(clip as usize-1)
            .ok_or("Native animation clip could not be decoded")?);
        cache.insert(clip,action.clone());
        Ok(action)
    }
    pub(crate) fn frames_for(&self, clip: u8) -> Option<usize> {
        if clip == 0 && self.nobj == 1 { return Some(1); }
        self.clips.iter().find(|&&(id,_)|id==clip).map(|&(_,frames)|frames)
    }
    /// Dialog identifies a conversation, not the speaker. Never pretend a
    /// text line identifies an NPC; use evidence-only labels until named.
    pub(crate) fn evidence_label(&self) -> String {
        match self.dialog.as_ref().map(|s|s.trim()).filter(|s|!s.is_empty()) {
            Some(line) => format!("Field {} #{} — dialog: {}", self.kind,self.record,
                line.chars().take(42).collect::<String>()),
            None => format!("Field {} #{} (TMD {})",self.kind,self.record,self.model_index),
        }
    }
}

/// Replace one actor's indexed triangle block, rebasing every later vertex.
/// Pure function so we can regression-test vertex-count changes without disc IO.
fn rebase_actor_indices(indices:&[u32],start:usize,end:usize,
                        new_len:usize,replacement:&[u32]) -> Result<Vec<u32>,String> {
    if end<start || start>u32::MAX as usize || end>u32::MAX as usize ||
        new_len>u32::MAX as usize || start.checked_add(new_len).is_none_or(|v|v>u32::MAX as usize) ||
        indices.len()%3!=0 || replacement.len()%3!=0 {
        return Err("Invalid raster mesh span".into());
    }
    if replacement.iter().any(|&index|index as usize>=new_len) {
        return Err("Replacement triangle indexes are out of bounds".into());
    }
    let shift=new_len as i64-(end-start) as i64;
    let mut result=Vec::with_capacity(indices.len()+replacement.len());
    for tri in indices.chunks_exact(3) {
        let inside=tri.iter().filter(|&&index|(index as usize)>=start && (index as usize)<end).count();
        if inside==3 {continue;}
        if inside!=0 {return Err("Cross-object triangle references an edited actor".into());}
        for &index in tri {
            let adjusted=if index as usize>=end {index as i64+shift} else {index as i64};
            if adjusted<0 || adjusted>u32::MAX as i64 {
                return Err("Shifted index is out of range".into());
            }
            result.push(adjusted as u32);
        }
    }
    result.extend(replacement.iter().map(|&index|index+start as u32));
    Ok(result)
}

impl WorldModel {
    /// Swap the render appearance without editing the PS1 MAN record.
    /// This can change vertex count: rebuild aligned raster attributes and
    /// rebase every later vertex index, preserving unrelated cells and draws.
    pub(crate) fn set_npc_model(&mut self,key:(bool,usize),new_index:u8)
        -> Result<(),String> {
        let npc_pos=self.npcs.iter().position(|n|(false,n.key)==key)
            .ok_or("Not a native field actor")?;
        let actor=&self.npcs[npc_pos];
        if actor.conditional || !actor.drawable {return Err("Unavailable NPC has no drawable model".into());}
        if actor.model_index==new_index {return Ok(());}
        if self.draws.iter().any(|d|d.hidden && !d.hidden_triangles.is_empty()) {
            return Err("Restore hidden native objects before changing an NPC model (protects triangle restore indices)".into());
        }
        let (raw,nobj)=actor.model_pool.get(new_index as usize)
            .and_then(|item|item.as_ref()).ok_or("TMD slot not found in this cell")?.clone();
        let clips=compatible_clips(actor.anm_bank.as_deref(),nobj);
        if nobj>1 && clips.is_empty() {
            return Err("Multipart model has no compatible PS1 animation; replacement refused".into());
        }
        let clip=if clips.iter().any(|(id,_)|*id==actor.clip) {actor.clip}
            else if nobj==1 {0} else {clips[0].0};
        let frame=if clip==actor.clip && clips.iter().any(|(id,frames)|*id==clip && actor.frame<*frames)
            {actor.frame} else {0};
        let mut mesh=Model::from_tmd(&raw).ok_or("Replacement TMD does not contain renderable geometry")?;
        if clip!=0 {
            let action=actor.action(clip)?;
            mesh.pose(action.frames.get(frame).ok_or("Model animation frame missing")?);
        }
        let native=actor.psx_position;
        let d=self.at(key.0,key.1).ok_or("Missing draw")?;
        if d.hidden {return Err("Restore hidden actor before changing its appearance".into());}
        let (start,end,id,pivot,offset,rotation,scale)=(d.vertex_start,d.vertex_end,d.id,d.original_center,
            d.offset,d.rotation,d.scale);
        let old_len=end-start;
        let new_len=mesh.positions.len();
        let model=self.model.as_mut().ok_or("No scene raster")?;
        if start>end || end>model.positions.len() || end>model.rest.len() || new_len==0 ||
            mesh.indices.iter().any(|&i|i as usize>=new_len) ||
            model.positions.len()-old_len+new_len>u32::MAX as usize {
            return Err("Unsafe replacement mesh span".into());
        }
        let shift=new_len as i64-old_len as i64;
        let indices=rebase_actor_indices(&model.indices,start,end,new_len,&mesh.indices)?;
        let local_rest:Vec<[f32;3]>=mesh.positions.iter().map(|v|[
            v[0]+native[0],-v[1]-native[1],v[2]+native[2]]).collect();
        let positions:Vec<[f32;3]>=local_rest.iter().map(|&v|
            editor_rotate_vertex(v,pivot,offset,rotation,scale)).collect();
        let (_,new_radius)=bounds(&positions);
        // Validate all arrays before changing a single one.
        let old_total=model.positions.len();
        if [model.rest.len(),model.object_ids.len(),model.uvs.len(),model.cba_tsb.len(),
            model.modulation.len(),model.fill.len(),model.textured.len()]
            .iter().any(|&len|len!=old_total) {return Err("Renderer attributes are misaligned".into());}
        model.positions.splice(start..end,positions);
        model.rest.splice(start..end,local_rest);
        model.object_ids.splice(start..end,std::iter::repeat_n(id,new_len));
        model.uvs.splice(start..end,mesh.uvs);
        model.cba_tsb.splice(start..end,mesh.cba_tsb);
        model.modulation.splice(start..end,mesh.modulation);
        model.fill.splice(start..end,mesh.fill);
        model.textured.splice(start..end,mesh.textured);
        model.indices=indices;
        for draw in &mut self.draws {
            if (draw.terrain,draw.index)==key {
                draw.vertex_end=start+new_len;
                draw.radius=new_radius;
            } else if draw.vertex_start>=end {
                draw.vertex_start=(draw.vertex_start as i64+shift) as usize;
                draw.vertex_end=(draw.vertex_end as i64+shift) as usize;
            }
        }
        let actor=&mut self.npcs[npc_pos];
        actor.model_index=new_index;
        actor.nobj=nobj;
        actor.raw_tmd=Some(raw);
        actor.clips=clips;
        actor.clip=clip;
        actor.frame=frame;
        actor.posed=clip!=0;
        Ok(())
    }

    /// Re-pose one PS1 field actor only. All geometry and editor transforms
    /// remain in a separate overlay; source disc data is never modified.
    pub(crate) fn set_npc_animation(&mut self,key:(bool,usize),clip:u8,frame:usize)
        -> Result<(),String> {
        let npc_pos=self.npcs.iter().position(|n| (false,n.key)==key)
            .ok_or("Selection is not a field actor")?;
        let npc=&self.npcs[npc_pos];
        if npc.conditional || !npc.drawable { return Err("NPC has no initially visible model".into()); }
        let frames=npc.frames_for(clip).ok_or("Animation not compatible with this model")?;
        if frame>=frames { return Err("Animation frame out of range".into()); }
        let bytes=npc.raw_tmd.as_deref().ok_or("NPC source TMD unavailable")?;
        let mut mesh=Model::from_tmd(bytes).ok_or("NPC mesh could not be decoded")?;
        if clip!=0 {
            let action=npc.action(clip)?;
            mesh.pose(action.frames.get(frame).ok_or("Native frame unavailable")?);
        }
        let native=npc.psx_position;
        let d=self.at(key.0,key.1).ok_or("NPC draw is missing")?;
        let (start,end,pivot,offset,rotation,scale)=(d.vertex_start,d.vertex_end,d.original_center,
            d.offset,d.rotation,d.scale);
        let model=self.model.as_mut().ok_or("Scene raster unavailable")?;
        if end<=start || mesh.positions.len()!=end-start || end>model.positions.len() || end>model.rest.len() {
            return Err("NPC clip changes vertex layout; preview safely refused".into());
        }
        for (idx,local) in mesh.positions.iter().enumerate() {
            let original=[local[0]+native[0], -local[1]-native[1],local[2]+native[2]];
            model.rest[start+idx]=original;
            model.positions[start+idx]=editor_rotate_vertex(original,pivot,offset,rotation,scale);
        }
        self.npcs[npc_pos].clip=clip;
        self.npcs[npc_pos].frame=frame;
        self.npcs[npc_pos].posed=clip!=0;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn npc_key_namespace_is_separate_from_environment() {
        assert!(NPC_KEY_BASE > 1_000_000);
        assert_ne!(NPC_KEY_BASE+3,3);
    }
    #[test]
    fn replacement_indices_rebase_both_larger_and_smaller_models() {
        let indices=[0,1,2, 3,4,5, 6,7,8];
        // Replace actor [3,6) with an actor of 6 vertices (growth +3).
        assert_eq!(rebase_actor_indices(&indices,3,6,6,&[0,1,2,3,4,5]).unwrap(),
            vec![0,1,2, 9,10,11, 3,4,5,6,7,8]);
        // Replace actor [3,6) with a single triangle of 3 vertices.
        assert_eq!(rebase_actor_indices(&indices,3,6,3,&[0,1,2]).unwrap(),
            vec![0,1,2, 6,7,8, 3,4,5]);
        assert!(rebase_actor_indices(&[0,3,4],3,6,3,&[0,1,2]).is_err());
        assert!(rebase_actor_indices(&indices,3,6,2,&[0,1,2]).is_err());
    }
    #[test]
    fn native_floor_corner_and_slope_interpolation() {
        let quad = [[0.0,0.0,0.0],[128.0,128.0,0.0],
                    [0.0,64.0,128.0],[128.0,192.0,128.0]];
        assert_eq!(bilerp_floor(&quad,0.0,0.0),0.0);
        assert_eq!(bilerp_floor(&quad,128.0,128.0),192.0);
        assert_eq!(bilerp_floor(&quad,64.0,64.0),96.0);
    }
}
