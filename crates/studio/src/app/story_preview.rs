//! R4.3A: editor-only native story-flag scene-entry preview.
use std::collections::{BTreeMap,BTreeSet};

// LCK_V207_NATIVE_MOTION
use legaia_engine_motion_vm::{
    ambient_motion::AmbientMotion,
    ambient_motion_ops::AmbientEffect,
};
use legaia_scene_formats::man_motion;
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
    tracks:Vec<PlaybackTrack>,
    animation_frame:usize,
    initial_motion_flags:BTreeSet<u16>,
    live_motion_flags:BTreeSet<u16>,
}

// LCK_V206_STORY_NAMES
//
// Flag IDs are not inherently descriptive. Decode native
// MAN event names and attach them as evidence to the
// actual SYSTEM flags referenced by those scripts.
//
// Names from retail records are NOT automatically treated
// as proven gameplay completion meanings.
//
// Rim Elm's previously established cumulative preview
// stages are retained as their original sequence.

fn rim_stages()->Vec<Stage>{
    vec![
        Stage::start(),
        Stage{
            label:"0x225 — Elder opening completed".into(),
            flags:vec![0x225]
        },
        Stage{
            label:"0x226 — Mei encounter completed".into(),
            flags:vec![0x225,0x226]
        },
        Stage{
            label:"0x227 — Vahn's house measuring completed".into(),
            flags:vec![0x225,0x226,0x227]
        },
        Stage{
            label:"0x22E — Hunters return / event aftermath".into(),
            flags:vec![0x225,0x226,0x227,0x22e]
        },
    ]
}

fn verified_flag_name(scene:&str,flag:u16)->Option<&'static str>{
    match (scene,flag) {
        ("town01"|"town0b"|"town0c",0x225)=>
            Some("Rim Elm — Elder opening completed"),
        ("town01"|"town0b"|"town0c",0x226)=>
            Some("Rim Elm — Mei encounter completed"),
        ("town01"|"town0b"|"town0c",0x227)=>
            Some("Rim Elm — Vahn house measuring completed"),
        ("town01"|"town0b"|"town0c",0x22e)=>
            Some("Rim Elm — Hunters return aftermath"),

        ("stone",0x1e8)=>
            Some("Gate of Shadows symbol puzzle solved"),

        ("tunnela",0x96..=0x9c)=>
            Some("Treasure chest opened (seven-chest set)"),

        ("rayman",0x1fd..=0x1ff)=>
            Some("Hari oracle consultation state"),

        ("geremi",0x1be)=>
            Some("Jeremi arrival event completed"),

        ("rikuroa",0x142)=>
            Some("Post-victory one-shot event"),

        ("doman",0x370)=>
            Some("Dr. Usha time-space briefing state"),

        _=>None,
    }
}

fn decode_native_name(raw:&[u8])->Option<String>{
    if raw.is_empty() || raw.len()>160 {
        return None;
    }

    let (decoded,_,invalid)=encoding_rs::SHIFT_JIS.decode(raw);

    if invalid {
        return None;
    }

    let clean=decoded.trim_matches('\0').trim();

    if clean.is_empty()
        || clean.chars().any(|c|c.is_control())
        || clean.chars().count()>80
    {
        return None;
    }

    Some(clean.to_owned())
}

fn native_record_name(
    parsed:&legaia_scene_formats::man_section::ManFile,
    man:&[u8],
    partition:usize,
    record:usize,
)->Option<String>{
    let raw=match partition {
        2=>legaia_engine_field::man_field_scripts::
            partition2_record_name(parsed,man,record)?,

        0=>{
            let relative=*parsed.partitions[0].get(record)?
                as usize;

            let start=parsed.data_region_offset
                .checked_add(relative)?;

            let count=usize::from(*man.get(start)?);

            if count==0 || count>80 {
                return None;
            }

            let begin=start.checked_add(1)?;
            let end=begin.checked_add(count.checked_mul(2)?)?;

            man.get(begin..end)?.to_vec()
        },

        _=>return None,
    };

    decode_native_name(&raw)
}

fn add_evidence(
    index:&mut BTreeMap<u16,BTreeSet<(u8,String)>>,
    flag:u16,
    priority:u8,
    description:String,
){
    index.entry(flag).or_default()
        .insert((priority,description));
}

fn best_description(
    scene:&str,
    flag:u16,
    evidence:&BTreeMap<u16,BTreeSet<(u8,String)>>,
)->String{
    if let Some(verified)=verified_flag_name(scene,flag) {
        return verified.to_owned();
    }

    evidence.get(&flag)
        .and_then(|sources|sources.iter().next())
        .map(|(_,label)|label.clone())
        .unwrap_or_else(||
            "System flag — meaning not yet verified".into()
        )
}

// LCK_V2084_SCENE_LOCAL_FLAGS
//
// Each CDNAME scene has its own MAN. Never assign the
// complete Rim Elm progression to every scene variant.
//
// P2 header gates are native event conditions.
// Clean SystemFlag TEST instructions also contribute
// scene-local evidence. Unverified disassembly and
// developer flag-setting menus are excluded.

fn v2084_stages_from_evidence(
    scene:&str,
    evidence:&std::collections::BTreeMap<
        u16,std::collections::BTreeSet<&'static str>
    >
)->Vec<Stage>{
    let mut stages=vec![Stage::start()];
    let mut included=std::collections::BTreeSet::<u16>::new();

    // Only the original town01 receives this verified
    // Rim Elm chronological sequence. Each beat still
    // requires an actual reference in town01's MAN.
    if scene=="town01" {
        let mut cumulative=Vec::<u16>::new();

        for beat in rim_stages().into_iter().skip(1){
            let Some(flag)=beat.flags.last().copied()
            else{continue;};

            if !evidence.contains_key(&flag){
                continue;
            }

            for flag in beat.flags {
                if evidence.contains_key(&flag)
                    && !cumulative.contains(&flag)
                {
                    cumulative.push(flag);
                }
            }

            included.insert(flag);

            stages.push(Stage{
                label:format!(
                    "{} [verified in town01 MAN]",
                    beat.label
                ),
                flags:cumulative.clone(),
            });
        }
    }

    // Independent variant-local condition preview.
    // Numeric flag order does NOT imply game chronology.
    for (&flag,sources) in evidence {
        if included.contains(&flag){
            continue;
        }

        let provenance=sources.iter()
            .copied()
            .collect::<Vec<_>>()
            .join(", ");

        stages.push(Stage{
            label:format!(
                "0x{flag:03X} - {provenance} [{scene}; order unverified]"
            ),
            flags:vec![flag],
        });
    }

    stages
}

pub(super) fn stages_for(scene:&str,index:&ProtIndex)->Vec<Stage>{
    use legaia_engine_field::man_field_scripts::{
        partition2_record_gates,
        walk_partition_gflag_sites,
        FlagBank,
    };
    use legaia_scene_formats::field_disasm::FlagKind;

    let empty=||vec![Stage::start()];

    let Ok(native)=Scene::load(index,scene)
    else{return empty();};

    let Ok(Some(man))=native.field_man_payload(index)
    else{return empty();};

    let Ok(parsed)=legaia_scene_formats::man_section::parse(&man)
    else{return empty();};

    let mut evidence=
        std::collections::BTreeMap::<
            u16,std::collections::BTreeSet<&'static str>
        >::new();

    for record in 0..parsed.partitions[2].len(){
        if let Some((c1,c2))=
            partition2_record_gates(&parsed,&man,record)
        {
            for flag in c1.into_iter().chain(c2){
                evidence.entry(flag).or_default()
                    .insert("P2 event header");
            }
        }
    }

    for (partition,description) in [
        (0usize,"P0 object script"),
        (1usize,"P1 actor script"),
        (2usize,"P2 event script"),
    ]{
        for site in walk_partition_gflag_sites(
            &parsed,&man,partition
        ){
            if site.bank!=FlagBank::System
                || site.kind!=FlagKind::Test
                || !site.clean
                || site.text_alias
                || site.debug_menu
            {
                continue;
            }

            evidence.entry(site.flag).or_default()
                .insert(description);
        }
    }

    v2084_stages_from_evidence(scene,&evidence)
}

#[cfg(test)]
mod lck_v2084_variant_tests {
    use super::*;

    #[test]
    fn variant_does_not_inherit_other_rim_stages(){
        let mut evidence=std::collections::BTreeMap::new();
        evidence.insert(
            0x227,
            std::collections::BTreeSet::from([
                "P2 event header"
            ])
        );

        let stages=v2084_stages_from_evidence(
            "town0b",&evidence
        );

        assert_eq!(stages.len(),2);
        assert_eq!(stages[1].flags,vec![0x227]);
    }

    #[test]
    fn town01_beats_require_native_evidence(){
        let mut evidence=std::collections::BTreeMap::new();

        for flag in [0x225,0x226]{
            evidence.insert(
                flag,
                std::collections::BTreeSet::from([
                    "P2 event header"
                ])
            );
        }

        let stages=v2084_stages_from_evidence(
            "town01",&evidence
        );

        assert_eq!(stages.len(),3);
        assert_eq!(stages[1].flags,vec![0x225]);
        assert_eq!(
            stages[2].flags,
            vec![0x225,0x226]
        );
    }

    #[test]
    fn empty_scene_has_only_original_view(){
        let evidence=std::collections::BTreeMap::new();

        assert_eq!(
            v2084_stages_from_evidence(
                "unknown",&evidence
            ).len(),
            1
        );
    }
}
pub(super) fn vahn_door_status(flags:&[u16])->&'static str {
    if flags.contains(&0x226) && !flags.contains(&0x227) {
        "Vahn's house: entering starts the measuring event"
    } else {"Vahn's house: normal interior transport"}
}

// LCK_V206_STORY_ANIMATION
//
// Only already bound native scene ANM clips are sampled.
// Does not simulate field-script movement, dialogues,
// effects, camera commands or complete cutscene timing.

// V2.0.7: scene-native NPC movement.
//
// Every channel carries one actual MAN tail-section-1
// script with its original story-flag variant selectors.
//
// Teleport coordinates are not repurposed as walkpaths.
// No invented route interpolation is used.

#[derive(Clone)]
struct MotionChannel {
    vm:AmbientMotion,
    initial:AmbientMotion,
    variants:Vec<(u16,Vec<u8>)>,
    active:Option<usize>,
}

fn v207_has_walk(code:&[u8])->bool {
    let mut pc=0usize;

    while pc<code.len() {
        let op=code[pc];

        let Some(width)=man_motion::op_width(op)
        else {
            return false;
        };

        if pc+width>code.len() {
            return false;
        }

        if matches!(op,0x03|0x06|0x18|0x19|0x20) {
            return true;
        }

        pc+=width;
    }

    false
}

fn v207_select_variant(
    variants:&[(u16,Vec<u8>)],
    flags:&BTreeSet<u16>,
)->Option<usize> {
    variants.iter().position(|(selector,_)|
        *selector!=0xFFFF
            && flags.contains(&(selector&0x0FFF))
    ).or_else(||
        variants.iter().position(|(selector,_)|
            *selector==0xFFFF
        )
    )
}

impl MotionChannel {
    fn new(
        actor:&super::world_view::NpcInfo,
        target:[f32;3],
        variants:Vec<(u16,Vec<u8>)>,
    )->Option<Self> {
        if variants.is_empty()
            || !target[0].is_finite()
            || !target[2].is_finite()
            || target[0]<i16::MIN as f32
            || target[0]>i16::MAX as f32
            || target[2]<i16::MIN as f32
            || target[2]>i16::MAX as f32
        {
            return None;
        }

        let x=target[0].round() as i16;
        let z=target[2].round() as i16;

        if x>=16320 && z>=16320 {
            return None;
        }

        let mut vm=AmbientMotion::new(
            actor.record as u32,
            0
        ).with_position(x,z);

        // Stable actor-specific RNG initialization.
        vm.rng=0x9E37_79B9u32
            .wrapping_mul(
                (actor.record as u32).wrapping_add(1)
            )
            .wrapping_add(0x1234_5678);

        let initial=vm.clone();

        Some(Self {
            vm,
            initial,
            variants,
            active:None,
        })
    }

    fn reset(&mut self) {
        self.vm=self.initial.clone();
        self.active=None;
    }

    fn tick(&mut self,flags:&mut BTreeSet<u16>) {
        let Some(chosen)=v207_select_variant(
            &self.variants,
            flags
        )else {
            return;
        };

        if self.active!=Some(chosen) {
            self.active=Some(chosen);
            self.vm.pc=0;
            self.vm.cursor=0;
        }

        let code=&self.variants[chosen].1;

        // Reuse the existing retail-derived motion VM.
        let _=self.vm.tick(code,1);

        // Any flag writes are confined to this
        // preview's temporary runtime state.
        for effect in self.vm.effects.drain(..) {
            match effect {
                AmbientEffect::SystemFlagSet(flag)=>{
                    flags.insert(flag);
                }
                AmbientEffect::SystemFlagClear(flag)=>{
                    flags.remove(&flag);
                }
                _=>{}
            }
        }
    }
}

fn v207_attach_motion(
    tracks:&mut [PlaybackTrack],
    parsed:&legaia_scene_formats::man_section::ManFile,
    man:&[u8],
) {
    let count0=parsed.partitions[0].len();
    let records=man_motion::motion_records(man,parsed);

    for track in tracks {
        let Some(actor_id)=count0
            .checked_add(track.actor.record)
            .and_then(|id|u8::try_from(id).ok())
        else {
            continue;
        };

        if actor_id>=man_motion::ACTOR_PLAYER {
            continue;
        }

        let Some(record)=records.iter().find(|record|
            record.bindings.iter().any(|binding|
                binding.actor_id==actor_id
            )
        )else {
            continue;
        };

        let variants=man_motion::stream_variants(
            man,
            record
        )
        .into_iter()
        .filter_map(|variant| {
            let code=man.get(
                variant.code_offset..variant.code_end
            )?;

            if code.is_empty() {
                return None;
            }

            Some((
                variant.selector,
                code.to_vec()
            ))
        })
        .collect::<Vec<_>>();

        if !variants.iter().any(|(_,code)|
            v207_has_walk(code)
        ) {
            continue;
        }

        track.motion=MotionChannel::new(
            &track.actor,
            track.target,
            variants
        );
    }
}
#[derive(Clone)]
struct PlaybackTrack {
    actor:super::world_view::NpcInfo,
    draw:Option<super::world_view::DrawInfo>,
    start:usize,
    end:usize,
    target:[f32;3],
    base_target:[f32;3],
    motion:Option<MotionChannel>,
}

impl PlaybackTrack {
    fn for_actor(
        actor:&super::world_view::NpcInfo,
        draw:Option<super::world_view::DrawInfo>,
        start:usize,
        end:usize,
        target:[f32;3],
    )->Option<Self>{
        if start>=end
            || actor.raw_tmd.is_none()
            || (actor.nobj>1 && actor.anm_bank.is_none())
        {
            return None;
        }

        Some(Self{
            actor:actor.clone(),
            draw,
            start,
            end,
            target,
            base_target:target,
            motion:None,
        })
    }

    fn sample(&self,model:&mut Model,frame:usize)->bool{
        if self.end>model.positions.len() {
            return false;
        }

        let Some(source)=self.actor.raw_tmd.as_deref()
        else {
            return false;
        };

        let Some(mut mesh)=Model::from_tmd(source)
        else {
            return false;
        };

        // Prefer the movement VM's requested clip
        // only when compatible with this model.
        let clip=self.motion.as_ref()
            .and_then(|motion|motion.vm.requested_move)
            .filter(|clip|
                *clip>0
                    && self.actor.frames_for(*clip).is_some()
            )
            .unwrap_or(self.actor.clip);

        let mut posed=false;

        if clip>0 {
            if let Ok(action)=self.actor.action(clip) {
                if !action.frames.is_empty() {
                    let sampled=&action.frames[
                        frame%action.frames.len()
                    ];

                    mesh.pose(sampled);
                    posed=true;
                }
            }
        }

        // Single-object actors may translate using
        // their rest mesh even without an ANM.
        // Multi-object skeletal actors still require
        // compatible native pose data.
        if !posed && self.actor.nobj>1 {
            return false;
        }

        if mesh.positions.len()!=self.end-self.start {
            return false;
        }

        let native=self.actor.psx_position;

        let displacement=[
            self.target[0]-native[0],
            self.target[1]-native[1],
            self.target[2]-native[2],
        ];

        for (i,p) in mesh.positions.iter().enumerate(){
            let result=if let Some(draw)=&self.draw {
                let original=[
                    p[0]+native[0],
                    -p[1]-native[1],
                    p[2]+native[2],
                ];

                let transformed=
                    super::world_view::editor_rotate_vertex(
                        original,
                        draw.original_center,
                        draw.offset,
                        draw.rotation,
                        draw.scale,
                    );

                [
                    transformed[0]+displacement[0],
                    transformed[1]-displacement[1],
                    transformed[2]+displacement[2],
                ]
            }else{
                [
                    p[0]+self.target[0],
                    -p[1]-self.target[1],
                    p[2]+self.target[2],
                ]
            };

            model.positions[self.start+i]=result;
        }

        true
    }
}

impl Preview {
    pub(super) fn animation_actor_count(&self)->usize{
        self.tracks.len()
    }

    pub(super) fn animation_frame(&self)->usize{
        self.animation_frame
    }
}

// Repose only actors, not the entire town mesh.
// The original scene and PS1 records are never changed.
pub(super) fn pose_frame(
    preview:&mut Preview,
    frame:usize,
)->usize {
    // Reset preview-local motion to the selected
    // story stage when the timeline returns to zero.
    if frame<preview.animation_frame {
        preview.live_motion_flags=
            preview.initial_motion_flags.clone();

        for track in &mut preview.tracks {
            track.target=track.base_target;

            if let Some(motion)=track.motion.as_mut() {
                motion.reset();
            }
        }
    }

    let advance=frame
        .saturating_sub(preview.animation_frame)
        .min(128);

    // Preview UI is 15 FPS. Native motion runs
    // at two 30 Hz steps per displayed frame.
    for _ in 0..advance {
        for _ in 0..2 {
            for track in &mut preview.tracks {
                let Some(motion)=track.motion.as_mut()
                else {
                    continue;
                };

                motion.tick(
                    &mut preview.live_motion_flags
                );

                // Use native world X/Z.
                // Preserve the stage's current Y until
                // moving-heightfield sampling is added.
                track.target[0]=motion.vm.x as f32;
                track.target[2]=motion.vm.z as f32;
            }
        }
    }

    let model=std::sync::Arc::make_mut(
        &mut preview.model
    );

    let updated=preview.tracks.iter()
        .filter(|track|
            track.sample(model,frame)
        )
        .count();

    preview.animation_frame=frame;
    updated
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
    let mut tracks=Vec::<PlaybackTrack>::new();
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

            if let Some(track)=PlaybackTrack::for_actor(
                npc,
                Some(draw.clone()),
                draw.vertex_start,
                draw.vertex_end,
                [x,y,z]
            ){
                tracks.push(track);
            }

        } else if npc.conditional && !npc.drawable {
            let start=model.positions.len();

            if append_actor(&mut model,npc,draw.id,x,y,z) {
                shown+=1;

                if let Some(track)=PlaybackTrack::for_actor(
                    npc,
                    None,
                    start,
                    model.positions.len(),
                    [x,y,z]
                ){
                    tracks.push(track);
                }
            }else{
                unresolved+=1;
            }
        }
    }
    // Bind each visible actor to its native
    // MAN motion stream when one exists.
    v207_attach_motion(
        &mut tracks,
        &parsed,
        &man
    );

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

    let mut preview=Preview{
        scene:scene.name.clone(),
        model:std::sync::Arc::new(model),
        moved,shown,hidden,unresolved,
        tracks,
        animation_frame:0,
        initial_motion_flags:
            flags.iter().copied().collect(),
        live_motion_flags:
            flags.iter().copied().collect(),
    };

    // Keep every actor's original clip at frame zero
    // when a new story stage is selected.
    if !preview.tracks.is_empty(){
        let _=pose_frame(&mut preview,0);
    }

    Ok(preview)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn v207_selects_story_flag_variant() {
        let variants=vec![
            (0x225,vec![0x05,1]),
            (0xFFFF,vec![0x05,2]),
        ];

        let active=BTreeSet::from([0x225]);

        assert_eq!(
            v207_select_variant(&variants,&active),
            Some(0)
        );

        assert_eq!(
            v207_select_variant(
                &variants,
                &BTreeSet::new()
            ),
            Some(1)
        );
    }

    #[test]
    fn v207_detects_native_walking_ops() {
        assert!(v207_has_walk(
            &[0x03,0x00,0x01]
        ));

        assert!(v207_has_walk(
            &[0x05,0x08,0x18,0,0,0,0]
        ));

        assert!(!v207_has_walk(
            &[0x05,0x08,0x01]
        ));
    }

    #[test]
    fn v207_native_vm_changes_world_position() {
        let mut vm=AmbientMotion::new(
            1,0
        ).with_position(320,640);

        let original=(vm.x,vm.z);

        // Direction 0 walks along negative Z.
        // One native 128-unit leg.
        for _ in 0..4 {
            let _=vm.tick(
                &[0x03,0x00,0x01,0x01],
                1
            );
        }

        assert_ne!(
            (vm.x,vm.z),
            original
        );

        assert_eq!(vm.x,320);
        assert!(vm.z<640);
    }
    #[test]
    fn v206_verified_flags_have_descriptions(){
        assert!(
            verified_flag_name("stone",0x1e8)
                .unwrap()
                .contains("Gate of Shadows")
        );

        assert!(
            verified_flag_name("geremi",0x1be)
                .unwrap()
                .contains("Jeremi")
        );

        assert!(
            verified_flag_name("town01",0x227)
                .unwrap()
                .contains("Vahn")
        );
    }

    #[test]
    fn v206_unknown_flags_are_not_misnamed(){
        let evidence=BTreeMap::new();

        let text=best_description(
            "unknown_scene",
            0x777,
            &evidence
        );

        assert!(text.contains("not yet verified"));
        assert_eq!(
            verified_flag_name("unknown_scene",0x777),
            None
        );
    }

    #[test]
    fn v206_native_record_names_are_decoded(){
        let name=decode_native_name(
            b"Opening"
        ).unwrap();

        assert_eq!(name,"Opening");

        assert!(decode_native_name(&[]).is_none());
    }

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