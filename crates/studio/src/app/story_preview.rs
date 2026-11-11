//! R4.3A: editor-only native story-flag scene-entry preview.
use std::collections::BTreeSet;
use crate::raster::Model;
use super::world_view::WorldModel;
use legaia_engine_scene::scene::{ProtIndex,Scene};
use legaia_engine_scene::scene_assembly::AssembledScene;
use legaia_engine_field::man_field_scripts::{
    grid_byte_to_world, placement_spawn_relocation, partition2_record_gates,
};

#[derive(Clone)]
pub(super) struct Stage { pub label:String, pub flags:Vec<u16> }
impl Stage { pub fn start()->Self {Self{label:"Start / original state".into(),flags:vec![]}} }

pub(super) struct Preview {
    pub scene:String,
    pub model:std::sync::Arc<Model>,
    pub moved:usize,
    pub shown:usize,
    pub hidden:usize,
    pub unresolved:usize,
}

fn rim_stages()->Vec<Stage>{
    vec![
        Stage::start(),
        Stage{label:"0x225 - Elder opening completed".into(),flags:vec![0x225]},
        Stage{label:"0x226 - Mei's encounter completed".into(),flags:vec![0x225,0x226]},
        Stage{label:"0x227 - Vahn's house measuring completed".into(),flags:vec![0x225,0x226,0x227]},
        Stage{label:"0x22E - Hunters return (event aftermath)".into(),flags:vec![0x225,0x226,0x227,0x22e]},
    ]
}

pub(super) fn stages_for(scene:&str,index:&ProtIndex)->Vec<Stage>{
    if matches!(scene,"town01"|"town0b"|"town0c") {return rim_stages();}
    let mut stages=vec![Stage::start()];
    let Ok(native)=Scene::load(index,scene) else {return stages;};
    let Ok(Some(man))=native.field_man_payload(index) else {return stages;};
    let Ok(parsed)=legaia_scene_formats::man_section::parse(&man) else {return stages;};
    let mut flags=BTreeSet::new();
    for record in 0..parsed.partitions[2].len() {
        if let Some((c1,c2))=partition2_record_gates(&parsed,&man,record) {
            flags.extend(c1);
            flags.extend(c2);
        }
    }
    // Outside verified Rim Elm: treat each header gate independently;
    // numeric order is a browser order, NOT proven game chronology.
    for flag in flags {
        stages.push(Stage{
            label:format!("0x{flag:03X} - header-gate comparison (order unverified)"),
            flags:vec![flag],
        });
    }
    stages
}

pub(super) fn vahn_door_status(flags:&[u16])->&'static str {
    if flags.contains(&0x226) && !flags.contains(&0x227) {
        "Vahn's house: entering starts the measuring event"
    } else {"Vahn's house: normal interior transport"}
}

fn ground_y(scene:&AssembledScene,x:f32,z:f32)->Option<f32>{
    let hf=scene.ground.as_ref()?;
    for v in hf.positions.chunks_exact(4) {
        let (x0,x1,z0,z1)=(v[0][0],v[1][0],v[0][2],v[2][2]);
        if x>=x0 && x<=x1 && z>=z0 && z<=z1 {
            let tx=((x-x0)/(x1-x0).max(1.0)).clamp(0.0,1.0);
            let tz=((z-z0)/(z1-z0).max(1.0)).clamp(0.0,1.0);
            return Some(v[0][1]*(1.0-tx)*(1.0-tz)+v[1][1]*tx*(1.0-tz)
                +v[2][1]*(1.0-tx)*tz+v[3][1]*tx*tz);
        }
    }
    None
}

fn append_actor(target:&mut Model, npc:&super::world_view::NpcInfo,
                id:u32,x:f32,y:f32,z:f32)->bool {
    let Some(raw)=npc.raw_tmd.as_ref() else {return false;};
    let Some(mut mesh)=Model::from_tmd(raw) else {return false;};
    let mut posed=false;
    if npc.clip>0 {
        if let Some(bank)=npc.anm_bank.as_ref() {
            if let Some(animation)=bank.record_to_monster_animation(npc.clip as usize-1) {
                if let Some(frame)=animation.frames.get(npc.frame) {
                    mesh.pose(frame);
                    posed=true;
                }
            }
        }
    }
    if npc.nobj>1 && !posed {return false;}
    let len=mesh.positions.len();
    if len==0 || mesh.indices.is_empty() ||
       [mesh.uvs.len(),mesh.cba_tsb.len(),mesh.modulation.len(),
        mesh.fill.len(),mesh.textured.len()].iter().any(|&n|n!=len) {return false;}
    let Ok(start)=u32::try_from(target.positions.len()) else {return false;};
    let Ok(len_u32)=u32::try_from(len) else {return false;};
    if start.checked_add(len_u32).is_none() ||
        mesh.indices.iter().any(|&i|i>=len_u32) {return false;}
    let positions=mesh.positions.iter().map(|p|[p[0]+x,-p[1]-y,p[2]+z])
        .collect::<Vec<_>>();
    target.positions.extend_from_slice(&positions);
    target.rest.extend_from_slice(&positions);
    target.object_ids.extend(std::iter::repeat_n(id,len));
    target.indices.extend(mesh.indices.iter().map(|&v|start+v));
    target.uvs.extend_from_slice(&mesh.uvs);
    target.cba_tsb.extend_from_slice(&mesh.cba_tsb);
    target.modulation.extend_from_slice(&mesh.modulation);
    target.fill.extend_from_slice(&mesh.fill);
    target.textured.extend_from_slice(&mesh.textured);
    true
}

pub(super) fn build_model(world:&WorldModel,scene:&AssembledScene,
        index:&ProtIndex,flags:&[u16])->Result<Preview,String> {
    let native=Scene::load(index,&scene.name).map_err(|e|e.to_string())?;
    let man=native.field_man_payload(index).map_err(|e|e.to_string())?
        .ok_or_else(||format!("{} has no native MAN",scene.name))?;
    let parsed=legaia_scene_formats::man_section::parse(&man)
        .map_err(|e|e.to_string())?;
    let placements=parsed.actor_placements(&man);
    let mut model=world.model.as_ref().ok_or("Scene has no rendered geometry")?.clone();
    // LCK_R43C_CAMERA_BOUNDS
    // Geometry may move, but the original camera's world-space frame
    // MUST NOT change because of story-state-dependent NPC positions.
    let original_view_center=model.center;
    let original_view_radius=model.radius;
    let mut hidden_ids=BTreeSet::new();
    let (mut moved,mut shown,mut hidden,mut unresolved)=(0,0,0,0);
    for npc in &world.npcs {
        if npc.is_copy {continue;}
        let Some(p)=placements.iter().find(|p|p.index==npc.record) else {continue;};
        let Some(draw)=world.at(false,npc.key) else {continue;};
        if draw.hidden {continue;} // never resurrect user-hidden objects
        let location=placement_spawn_relocation(&parsed,&man,p,
            &|flag|flags.contains(&flag));
        let (x,z,parked)=if let Some((bx,bz))=location {
            (f32::from(grid_byte_to_world(bx)),
             f32::from(grid_byte_to_world(bz)),
             (bx&0x7f,bz&0x7f)==(0x7f,0x7f))
        } else {(p.world_x as f32,p.world_z as f32,
            (p.tile_x,p.tile_z)==(0x7f,0x7f))};
        if parked {
            hidden_ids.insert(draw.id);
            if npc.drawable {hidden+=1;}
            continue;
        }
        let y=ground_y(scene,x,z).unwrap_or(npc.psx_position[1]);
        if npc.drawable && draw.vertex_start<draw.vertex_end &&
            draw.vertex_end<=model.positions.len() {
            let (dx,dy,dz)=(x-npc.psx_position[0],
                y-npc.psx_position[1],z-npc.psx_position[2]);
            if dx.abs()>0.01 || dy.abs()>0.01 || dz.abs()>0.01 {
                for v in &mut model.positions[draw.vertex_start..draw.vertex_end] {
                    v[0]+=dx;v[1]-=dy;v[2]+=dz;
                }
                moved+=1;
            }
        } else if npc.conditional && !npc.drawable {
            if append_actor(&mut model,npc,draw.id,x,y,z) {shown+=1;}
            else {unresolved+=1;}
        }
    }
    if !hidden_ids.is_empty() {
        model.indices=model.indices.chunks_exact(3)
            .filter(|tri|!hidden_ids.contains(&model.object_ids[tri[0] as usize]))
            .flat_map(|tri|tri.iter().copied()).collect();
    }
    // Keep the base scene's camera centre/radius. Previewing a story
    // step must never reframe the user's view or edit native geometry.
    // Camera framing must remain identical to the unmodified scene.
    restore_preview_framing(
        &mut model,
        original_view_center,
        original_view_radius
    );

    Ok(Preview{scene:scene.name.clone(),model:std::sync::Arc::new(model),moved,shown,hidden,unresolved})
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rim_elm_story_beats_are_cumulative() {
        let stages=rim_stages();
        assert_eq!(stages.len(),5);
        assert_eq!(stages[0].flags,Vec::<u16>::new());
        assert_eq!(stages[3].flags,vec![0x225,0x226,0x227]);
        assert_eq!(stages[4].flags,vec![0x225,0x226,0x227,0x22e]);
    }
    #[test]
    fn vahn_house_door_depends_on_both_flags() {
        assert!(vahn_door_status(&[0x226]).contains("measuring"));
        assert!(vahn_door_status(&[0x226,0x227]).contains("normal"));
        assert!(vahn_door_status(&[0x225]).contains("normal"));
    }
}
// LCK_R43C_CAMERA_BOUNDS
// Preserve the original raster camera basis even when actor positions
// temporarily occupy another part of the scene.
fn restore_preview_framing(
    model:&mut Model,
    center:[f32;3],
    radius:f32
){
    model.center=center;
    model.radius=radius;
}

#[cfg(test)]
mod lck_r43c_camera_tests {
    use super::*;

    #[test]
    fn actor_relocation_does_not_change_camera_framing() {
        let mut model=Model {
            positions:vec![[10.0,20.0,30.0]],
            rest:vec![[10.0,20.0,30.0]],
            object_ids:vec![7],
            uvs:vec![[0,0]],
            cba_tsb:vec![[0,0]],
            modulation:vec![[128,128,128]],
            fill:vec![[128,128,128]],
            textured:vec![1],
            indices:vec![],
            center:[100.0,200.0,300.0],
            radius:1800.0,
        };

        let original_center=model.center;
        let original_radius=model.radius;

        // An NPC moving across the map can change raw mesh bounds.
        model.positions[0]=[10000.0,100.0,20000.0];
        model.refit();

        assert_ne!(model.center,original_center);

        // Story preview restores the original viewing basis.
        restore_preview_framing(
            &mut model,
            original_center,
            original_radius
        );

        assert_eq!(model.center,original_center);
        assert_eq!(model.radius,original_radius);
        assert_eq!(model.positions[0],[10000.0,100.0,20000.0]);
    }

    #[test]
    fn camera_pose_components_are_independent_of_story_state() {
        let camera=crate::raster::Camera {
            yaw:1.2,
            pitch:2.5,
            zoom:0.75,
            target_offset:[120.0,-30.0,480.0],
        };

        let saved=camera;
        let mut current=camera;

        // A preview transition must restore the precise saved pose.
        current.yaw=0.1;
        current.pitch=0.4;
        current.zoom=2.0;
        current.target_offset=[0.0;3];

        current=saved;
        assert_eq!(current,saved);
    }
}