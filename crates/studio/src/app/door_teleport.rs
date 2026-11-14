//! LCK R1/R2 native doors and teleports.
//! Native MAP kind-0/kind-1 records and MAN 0x3F destinations.
//! Labels and viewport markers are editor metadata, not native meshes.

use std::{
    collections::{BTreeMap,BTreeSet},
    fs,
    path::Path,
};
use anyhow::{Result,Context,ensure,bail};
use eframe::egui;
use serde::{Serialize,Deserialize};
use legaia_engine_scene::scene::{ProtIndex,Scene};
use legaia_scene_formats::{man_edit,man_section,scene_asset_table};
use legaia_disc_patch::{disc::DiscPatcher,compress_within};
use super::{collision_recipes,ps1_export,ps1_original_guard};
mod named_targets;

#[derive(Clone,Debug,PartialEq,Eq,Serialize,Deserialize)]
pub(super) struct Destination {
    pub name:String,
    pub index:i16,
    pub entry_x:u8,
    pub entry_z:u8,
    pub facing:u8,
}

#[derive(Clone,Debug,PartialEq,Eq,Serialize,Deserialize)]
pub(super) enum Kind {
    Local {
        half_x:u8,
        half_z:u8,
    },
    Script {
        slot:Option<u8>,
        op_pc:Option<usize>,
        destination:Option<Destination>,
    },
}

#[derive(Clone,Debug,PartialEq,Eq,Serialize,Deserialize)]
pub(super) enum Origin {
    Local {fallback:bool,row:usize},
    Walk {fallback:bool,row:usize},
    ScriptOnly {op_pc:usize},
    New,
}

#[derive(Clone,Debug,PartialEq,Eq,Serialize,Deserialize)]
pub(super) struct Door {
    pub uid:u64,
    pub label:String,
    pub origin:Origin,
    pub source:[u8;2],
    pub size:[u8;2],
    pub enabled:bool,
    pub kind:Kind,
}

impl Door {
    fn visible_name(&self)->String {
        if self.label.trim().is_empty() {
            match &self.kind {
                Kind::Local{..}=>format!("Same-scene warp #{}",self.uid),
                Kind::Script{op_pc:Some(pc),..}=>format!("Scripted doorway @MAN +0x{pc:X}"),
                Kind::Script{..}=>format!("MAN-bound trigger #{}",self.uid),
            }
        } else {
            self.label.clone()
        }
    }

    fn local_row(&self,x:u8,z:u8)->[u8;4] {
        match &self.kind {
            Kind::Local{half_x,half_z}=>[x,z,*half_x,*half_z],
            Kind::Script{slot:Some(slot),..}=>[x,z,*slot,1],
            _=>[x,z,0,0],
        }
    }

    fn geometry(&self)->Result<Vec<[u8;4]>> {
        if !self.enabled ||
            matches!(&self.origin,Origin::ScriptOnly{..}) {
            return Ok(Vec::new());
        }

        ensure!(
            self.size[0]>0 && self.size[1]>0,
            "Door #{} has zero trigger size",
            self.uid
        );

        ensure!(
            self.size[0] as usize*self.size[1] as usize<=512,
            "Door #{} exceeds 512 trigger tiles",
            self.uid
        );

        ensure!(
            (self.source[0] as u16+self.size[0] as u16)<=128 &&
            (self.source[1] as u16+self.size[1] as u16)<=128,
            "Door #{} exceeds MAP grid",
            self.uid
        );

        if matches!(&self.kind,Kind::Script{slot:None,..}) {
            bail!("Door #{} has no native MAN partition-2 slot",self.uid);
        }

        let mut rows=Vec::new();

        for z in self.source[1]..self.source[1]+self.size[1] {
            for x in self.source[0]..self.source[0]+self.size[0] {
                rows.push(self.local_row(x,z));
            }
        }

        Ok(rows)
    }
}

#[derive(Clone,Debug,Serialize,Deserialize)]
struct Saved {
    version:u32,
    scene:String,
    map_fingerprint:String,
    fallback_fingerprint:String,
    man_fingerprint:String,
    records:Vec<Door>,
    #[serde(default)]
    auxiliary:Vec<R605NativeTrigger>,
    next_uid:u64,
    // LCK_R630C_VERTICAL_MARKERS
    #[serde(default)]
    r630c_warp_y:BTreeMap<u64,f32>,
    #[serde(default)]
    r630c_aux_y:Vec<f32>,
}

// LCK TRIGGER INSPECTOR R3.3
// Native script evidence, never script execution.
//
// A MAP kind-1 trigger's MAN operand indexes the FLAT
// [P0, P1, P2] record table.
// It is not a partition-2-relative record number.

#[derive(Clone,Debug,Default)]
struct ScriptSummary {
    flat_index:u8,
    partition:usize,
    record_start:usize,
    script_start:usize,
    record_end:usize,
    native_name:Option<String>,
    raw_name_hex:String,
    header_conditions:String,
    flags:Vec<String>,
    operations:Vec<String>,
    truncated:bool,
}

fn lck_native_record_name(bytes:&[u8])->Option<String>{
    if bytes.is_empty() || bytes.len()>160 {
        return None;
    }

    // Retail record-name fields are 2-byte encoded.
    // Normalize ASCII pairs only when the pattern is uniform;
    // otherwise decode Shift-JIS.
    let bytes=if bytes.len().is_multiple_of(2) &&
        bytes.chunks_exact(2).all(|p|p[1]==0) {
        bytes.iter().step_by(2).copied().collect::<Vec<_>>()
    }else if bytes.len().is_multiple_of(2) &&
        bytes.chunks_exact(2).all(|p|p[0]==0) {
        bytes.iter().skip(1).step_by(2).copied().collect::<Vec<_>>()
    }else{
        bytes.to_vec()
    };

    let (text,_,bad)=encoding_rs::SHIFT_JIS.decode(&bytes);
    let clean=text.trim_matches('\0').trim();

    (!bad &&
        !clean.is_empty() &&
        clean.chars().count()<=80 &&
        clean.chars().all(|c|!c.is_control()))
        .then(||clean.to_owned())
}

fn lck_script_summaries(man:&[u8])->BTreeMap<u8,ScriptSummary>{
    use legaia_scene_formats::{man_section,field_disasm};
    use field_disasm::InsnInfo;

    let mut result=BTreeMap::new();

    let Ok(file)=man_section::parse(man) else{
        return result;
    };

    let mut stops=Vec::new();

    for part in &file.partitions {
        for &rel in part {
            stops.push(file.data_region_offset+rel as usize);
        }
    }

    for sec in &file.sections {
        stops.push(sec.offset);
    }

    stops.sort_unstable();
    stops.dedup();

    let mut flat=0usize;

    for (partition,part) in file.partitions.iter().enumerate() {
        for &rel in part {
            let record_index=flat;
            flat+=1;

            let Ok(flat_index)=u8::try_from(record_index) else{
                continue;
            };

            let start=file.data_region_offset+rel as usize;

            let end=stops.iter().copied()
                .find(|&p|p>start)
                .unwrap_or(man.len())
                .min(man.len());

            if start>=end {
                continue;
            }

            let Some(&name_length)=man.get(start) else{
                continue;
            };

            let n=name_length as usize;
            let mut raw_name_hex=String::new();
            let mut native_name=None;

            let mut header_conditions=
                String::from("No P2 condition-list header");

            // Partition 0: [N][N*2 name][attr]
            // Partition 1: [N][N*2 locals][4-byte placement]
            // Partition 2: name + three condition blocks,
            // followed by the native script bytecode.

            let mut pc=match partition {
                0|2=>{
                    if n>80 {
                        continue;
                    }

                    let name_end=start+1+n*2;

                    let Some(bytes)=man.get(start+1..name_end) else{
                        continue;
                    };

                    raw_name_hex=bytes.iter()
                        .map(|b|format!("{b:02X}"))
                        .collect::<Vec<_>>()
                        .join(" ");

                    native_name=lck_native_record_name(bytes);

                    if partition==0 {
                        name_end+1
                    }else{
                        let mut at=name_end;

                        let Some(&c0)=man.get(at) else{
                            continue;
                        };
                        at+=1+c0 as usize;

                        let Some(&c1)=man.get(at) else{
                            continue;
                        };
                        at+=1+c1 as usize*2;

                        let Some(&c2)=man.get(at) else{
                            continue;
                        };
                        at+=1+c2 as usize*2;

                        header_conditions=format!(
                            "Native P2 condition-list lengths: {c0}, {c1}, {c2}"
                        );

                        at
                    }
                },

                1=>start+1+n*2+4,

                _=>continue,
            };

            if pc>end {
                continue;
            }

            let mut summary=ScriptSummary {
                flat_index,
                partition,
                record_start:start,
                script_start:pc,
                record_end:end,
                native_name,
                raw_name_hex,
                header_conditions,
                ..Default::default()
            };

            let mut scanned=0usize;

            while pc<end && scanned<512 {
                let Ok(insn)=field_disasm::decode(man,pc) else{
                    summary.truncated=true;
                    break;
                };

                if insn.size==0 ||
                    pc.saturating_add(insn.size)>end {
                    summary.truncated=true;
                    break;
                }

                match &insn.info {
                    InsnInfo::SystemFlag{kind,idx,..}=>{
                        summary.flags.push(format!(
                            "System flag {kind:?} 0x{idx:04X}, MAN +0x{pc:X}"
                        ));
                    },

                    InsnInfo::GFlag{kind,bit}=>{
                        summary.flags.push(format!(
                            "Global flag {kind:?} bit {bit}, MAN +0x{pc:X}"
                        ));
                    },

                    InsnInfo::LFlag{kind,bit}=>{
                        summary.flags.push(format!(
                            "Local flag {kind:?} bit {bit}, MAN +0x{pc:X}"
                        ));
                    },

                    InsnInfo::CFlag{kind,bit}=>{
                        summary.flags.push(format!(
                            "Context flag {kind:?} bit {bit}, MAN +0x{pc:X}"
                        ));
                    },

                    _=>{},
                }

                if !matches!(
                    &insn.info,
                    InsnInfo::Nop | InsnInfo::TextSegment{..}
                ) && summary.operations.len()<100 {
                    summary.operations.push(
                        field_disasm::format_instruction(&insn,man)
                    );
                }

                pc+=insn.size;
                scanned+=1;
            }

            if pc<end {
                summary.truncated=true;
            }

            result.insert(flat_index,summary);
        }
    }

    result
}

#[cfg(test)]
mod lck_trigger_inspector_tests {
    use super::*;

    #[test]
    fn invalid_man_yields_no_invented_script_ownership() {
        assert!(lck_script_summaries(&[0,1,2,3]).is_empty());
    }

    #[test]
    fn names_are_editor_metadata_not_native_door_changes() {
        let x=Door {
            uid:12,
            label:"Test".into(),
            origin:Origin::Local{fallback:false,row:2},
            source:[3,4],
            size:[1,1],
            enabled:true,
            kind:Kind::Local{half_x:5,half_z:6}
        };

        let mut state=State::default();
        state.original=vec![x.clone()];
        state.records=vec![x];
        state.records[0].label="User name".into();

        assert!(!state.has_native_changes());
    }

    #[test]
    fn native_record_names_require_printable_decodable_bytes() {
        assert_eq!(
            lck_native_record_name(&[
                b'D',0,b'O',0,b'O',0,b'R',0
            ]),
            Some("DOOR".into())
        );

        assert_eq!(
            lck_native_record_name(&[0,0,0,0]),
            None
        );
    }
}
#[derive(Clone)]
struct R604MarkerDrag {
    uid:u64,
    start:egui::Pos2,
    anchor:[f32;3],
    source:[u8;2],
    changed:bool,
    vertical:bool,
    height_start:f32,
}

// LCK_R630C_VERTICAL_MARKERS
#[derive(Clone)]
struct R630CAuxDrag {
    index:usize,
    start:egui::Pos2,
    anchor:[f32;3],
    height_start:f32,
}
// LCK_V2084_STORY_TRIGGER_PREVIEW
//
// These fields affect the editor preview only.
// Neither project records nor IMG export are
// rewritten when a user changes story stages.

impl State {
    fn v2084_p2_allowed(
        gates:&(Vec<u16>,Vec<u16>),
        flags:&[u16]
    )->bool {
        let (blocked,required)=gates;

        !blocked.iter().any(|f|flags.contains(f))
            && required.iter().all(|f|flags.contains(f))
    }

    pub(super) fn v2084_set_story_flags(
        &mut self,
        flags:Option<&[u16]>
    ){
        if self.v2084_stage_flags.as_deref()==flags {
            return;
        }

        self.v2084_stage_flags=flags.map(|v|v.to_vec());
        self.v2084_paths.clear();

        let Some(flags)=flags else{return;};
        let Some(man)=self.v2084_man.as_deref()
        else{return;};

        let Ok(parsed)=
            legaia_scene_formats::man_section::parse(man)
        else{return;};

        for door in &self.records {
            let Kind::Script{slot:Some(slot),..}=&door.kind
            else{continue;};

            if self.v2084_paths.contains_key(slot){
                continue;
            }

            if let Some((outcome,_))=
                legaia_engine_field::man_field_scripts::
                    flat_record_path_walk(
                        &parsed,
                        man,
                        usize::from(*slot),
                        &|flag|flags.contains(&flag)
                    )
            {
                self.v2084_paths.insert(*slot,outcome);
            }
        }
    }

    fn v2084_story_door_visible(&self,d:&Door)->bool {
        let Some(flags)=self.v2084_stage_flags.as_deref()
        else{return true;};

        let Kind::Script{slot:Some(slot),..}=&d.kind
        else{return true;};

        if let Some(gates)=self.v2084_p2_gates.get(slot){
            if !Self::v2084_p2_allowed(gates,flags){
                return false;
            }
        }

        !matches!(
            self.v2084_paths.get(slot),
            Some(
                legaia_engine_field::man_field_scripts::
                    RecordPathEnd::Closed
            )
        )
    }

    fn v2084_stage_outcome(&self,d:&Door)->Option<String>{
        self.v2084_stage_flags.as_ref()?;

        let Kind::Script{slot:Some(slot),..}=&d.kind
        else{return None;};

        if !self.v2084_story_door_visible(d){
            return Some(
                "Story preview: this scripted transition is inactive in the selected flag state."
                    .into()
            );
        }

        use legaia_engine_field::man_field_scripts::RecordPathEnd;

        match self.v2084_paths.get(slot) {
            Some(RecordPathEnd::SceneChange(
                (index,name,x,z,facing)
            ))=>Some(format!(
                "Story preview target: {} | index {} | arrival ({},{}) | facing {}",
                name,index,x,z,facing
            )),

            Some(RecordPathEnd::Closed)=>Some(
                "Story preview: this path does not reach a scene transition."
                    .into()
            ),

            Some(RecordPathEnd::Undecided)=>Some(
                "Story preview: this script uses additional runtime conditions; its destination cannot be established from story flags alone."
                    .into()
            ),

            None=>Some(
                "Story preview: no verified transition path for this native record."
                    .into()
            ),
        }
    }
}

#[cfg(test)]
mod lck_v2084_trigger_tests {
    use super::*;

    #[test]
    fn p2_c1_blocks_if_any_flag_is_set(){
        let gates=(vec![0x225],vec![]);

        assert!(State::v2084_p2_allowed(&gates,&[]));

        assert!(!State::v2084_p2_allowed(
            &gates,&[0x225]
        ));
    }

    #[test]
    fn p2_c2_requires_every_flag(){
        let gates=(vec![],vec![0x226,0x227]);

        assert!(!State::v2084_p2_allowed(
            &gates,&[0x226]
        ));

        assert!(State::v2084_p2_allowed(
            &gates,&[0x226,0x227]
        ));
    }

    #[test]
    fn original_unfiltered_view_remains_available(){
        let state=State::default();

        let door=Door{
            uid:1,
            label:String::new(),
            origin:Origin::New,
            source:[10,10],
            size:[1,1],
            enabled:true,
            kind:Kind::Script{
                slot:Some(1),
                op_pc:None,
                destination:None,
            },
        };

        assert!(state.v2084_story_door_visible(&door));
    }
}
// LCK_R605_OTHER_TRIGGERS
// Read-only provenance for kind-1 gate-0 and kind-2 MAP records.
// These are not automatically doorway teleports.
#[derive(Clone,Debug,Hash,PartialEq,Eq,Serialize,Deserialize)]
struct R605NativeTrigger {
    kind:u8,
    gate:u8,
    tile:[u8;2],
    record:u8,
    extra:u8,
    fallback:bool,
    row:usize,
    // Descriptor, object-world X/Z, contact-world X/Z.
    objects:Vec<(u16,[i16;2],[i16;2])>,
}

fn lck_r605_kind2(block:&[u8])->Vec<[u8;4]> {
    if block.len()<14 {
        return Vec::new();
    }

    let offset=i16::from_le_bytes([block[10],block[11]]);
    let count=i16::from_le_bytes([block[12],block[13]]);

    if offset<14 || count<=0 {
        return Vec::new();
    }

    let start=offset as usize;

    let Some(end)=start.checked_add(count as usize*4)
    else {
        return Vec::new();
    };

    if end>block.len() {
        return Vec::new();
    }

    block[start..end]
        .chunks_exact(4)
        .map(|b|[b[0],b[1],b[2],b[3]])
        .collect()
}
#[derive(Clone,Default)]
pub(super) struct State {
    pub scene:String,
    inspector_scripts:BTreeMap<u8,ScriptSummary>,
    other_triggers:Vec<R605NativeTrigger>,
    original_auxiliary:Vec<R605NativeTrigger>,
    aux_properties:Option<usize>,
    trigger_mask:Option<u8>,
    marker_menu:Option<(u64,egui::Pos2)>,
    marker_properties:Option<u64>,
    r622_destinations:Option<Vec<R622Scene>>,
    r622_search:String,
    r639_named_catalog:named_targets::Catalog,
    r639_named_search:String,
    pub(super) r622_export_requested:bool,
    r604_marker_drag:Option<R604MarkerDrag>,
    r604_marker_focus:bool,
    r630c_aux_drag:Option<R630CAuxDrag>,
    r630c_warp_y:BTreeMap<u64,f32>,
    r630c_aux_y:Vec<f32>,
    pub marker_right_consumed:bool,
    records:Vec<Door>,
    original:Vec<Door>,
    map_fingerprint:String,
    fallback_fingerprint:String,
    man_fingerprint:String,
    native_p2_records:usize,
    next_uid:u64,
    selected:Option<u64>,
    pub show_markers:bool,
    v2084_man:Option<Vec<u8>>,
    v2084_stage_flags:Option<Vec<u16>>,
    v2084_p2_gates:BTreeMap<u8,(Vec<u16>,Vec<u16>)>,
    v2084_paths:BTreeMap<
        u8,
        legaia_engine_field::man_field_scripts::RecordPathEnd
    >,
    donor:String,
    donor_row:usize,
    message:String,
    undo:Vec<Vec<Door>>,
    redo:Vec<Vec<Door>>,
}

/// Original MAP ground quads store native PSX Y-down heights.
/// Renderer-space is Y-up; sample exactly as native NPC placement does.
// LCK_R622_EDITABLE_DESTINATIONS
// Destination selections are taken from retail MAN scene-change
// instructions. The scene name, index and arrival are kept together.

#[derive(Clone,Debug)]
struct R622Arrival {
    from_scene:String,
    index:i16,
    x:u8,
    z:u8,
    facing:u8,
}

#[derive(Clone,Debug)]
struct R622Scene {
    name:String,
    arrivals:Vec<R622Arrival>,
}

fn r622_build_catalog(index:&ProtIndex)->Vec<R622Scene> {
    let names=index.cdname_scene_names();

    let mut field_cells=BTreeSet::<String>::new();

    let mut inbound=
        BTreeMap::<String,Vec<R622Arrival>>::new();

    for source_name in &names {
        let Ok(scene)=Scene::load(index,source_name) else {
            continue;
        };

        if scene.field_map_index(index).is_some() {
            field_cells.insert(source_name.clone());
        }

        let Ok(Some(man))=scene.field_man_payload(index) else {
            continue;
        };

        for site in man_edit::scene_change_sites(&man) {
            if site.name.is_empty() || site.dir>7 {
                continue;
            }

            inbound.entry(site.name).or_default().push(
                R622Arrival{
                    from_scene:source_name.clone(),
                    index:site.index,
                    x:site.entry_x,
                    z:site.entry_z,
                    facing:site.dir,
                }
            );
        }
    }

    let mut output=Vec::new();

    for name in field_cells {
        let mut arrivals=inbound.remove(&name)
            .unwrap_or_default();

        arrivals.sort_by_key(|a|{
            (
                if a.from_scene.starts_with("map"){0}else{1},
                a.index,
                a.x,
                a.z,
                a.facing
            )
        });

        let mut seen=BTreeSet::new();

        arrivals.retain(|a|{
            seen.insert((
                a.index,a.x,a.z,a.facing
            ))
        });

        output.push(R622Scene{name,arrivals});
    }

    output
}

// LCK_R627_DESTINATION_BROWSER
// Preserve native scene IDs and destination indices.
// Grouping and filtering are editor-only presentation.
fn r627_destination_groups<'a>(
    catalog:&'a [R622Scene],
    query:&str
)->Vec<(String,Vec<&'a R622Scene>)> {
    let mut grouped=
        BTreeMap::<String,Vec<&R622Scene>>::new();

    for scene in catalog {
        let family=super::lck_r627_cell_family(
            &scene.name
        );

        grouped.entry(family)
            .or_default()
            .push(scene);
    }

    let term=query.trim().to_lowercase();

    let mut output=Vec::new();
    let mut standalone=Vec::new();

    for (family,scenes) in grouped {
        let family_match=
            family.to_lowercase().contains(&term);

        let family_size=scenes.len();

        let filtered=scenes.into_iter()
            .filter(|scene|{
                let native_name=crate::place_name(
                    &scene.name
                ).unwrap_or(&scene.name);

                family_match
                    || scene.name.to_lowercase().contains(&term)
                    || native_name.to_lowercase().contains(&term)
            })
            .collect::<Vec<_>>();

        if filtered.is_empty(){
            continue;
        }

        if family_size>1 {
            output.push((family,filtered));
        }else{
            standalone.extend(filtered);
        }
    }

    if !standalone.is_empty(){
        output.push((
            "Other / standalone cells".to_owned(),
            standalone
        ));
    }

    output
}
fn r622_select_arrival(
    destination:&mut Destination,
    scene:&R622Scene,
    position:usize
)->bool {
    let Some(arrival)=scene.arrivals.get(position) else {
        return false;
    };

    destination.name=scene.name.clone();
    destination.index=arrival.index;
    destination.entry_x=arrival.x;
    destination.entry_z=arrival.z;
    destination.facing=arrival.facing;

    true
}

#[cfg(test)]
mod lck_r622_destination_tests {
    use super::*;

    #[test]
    fn selecting_arrival_updates_entire_native_destination() {
        let scene=R622Scene{
            name:"map01".into(),
            arrivals:vec![R622Arrival{
                from_scene:"town01".into(),
                index:85,
                x:96,
                z:25,
                facing:4,
            }]
        };

        let mut destination=Destination{
            name:"town01".into(),
            index:0,
            entry_x:0,
            entry_z:0,
            facing:0,
        };

        assert!(r622_select_arrival(
            &mut destination,&scene,0
        ));

        assert_eq!(destination.name,"map01");
        assert_eq!(destination.index,85);
        assert_eq!(destination.entry_x,96);
        assert_eq!(destination.entry_z,25);
        assert_eq!(destination.facing,4);
    }

    #[test]
    fn unknown_arrival_does_not_change_destination() {
        let scene=R622Scene{
            name:"unknown".into(),
            arrivals:Vec::new(),
        };

        let mut destination=Destination{
            name:"map01".into(),
            index:85,
            entry_x:96,
            entry_z:25,
            facing:4,
        };

        assert!(!r622_select_arrival(
            &mut destination,&scene,0
        ));

        assert_eq!(destination.name,"map01");
        assert_eq!(destination.index,85);
    }
}
// LCK_R626_TRIGGER_IDENTITY
// Earlier export stages may relocate a native region table.
// That does not authorize editing or retargeting different
// MAP trigger rows.
//
// Kinds 0, 1 and 2 must retain their exact native row data.
// Kind 3 is the region table, which may legitimately move.
fn r626_verify_trigger_identity(
    retail_map:&[u8],
    staged_map:&[u8],
)->Result<()> {
    ensure!(
        retail_map.len()>=0x12000 &&
        staged_map.len()>=0x12000,
        "R626: MAP trigger validation requires full native MAP"
    );

    let retail=&retail_map[0x10000..0x12000];
    let staged=&staged_map[0x10000..0x12000];

    for kind in 0..3usize {
        ensure!(
            table(retail,kind)?==table(staged,kind)?,
            "R626: Native MAP kind-{kind} trigger rows changed in an earlier export stage. Refusing to retarget unrelated warp records"
        );
    }

    Ok(())
}
fn native_door_floor_y(
    ground:Option<&legaia_scene_formats::field_objects::WalkHeightfield>,
    x:f32,z:f32
)->f32{
    let Some(hf)=ground else {return 0.0;};

    for v in hf.positions.chunks_exact(4) {
        let x0=v[0][0];
        let x1=v[1][0];
        let z0=v[0][2];
        let z1=v[2][2];

        if x<x0 || x>x1 || z<z0 || z>z1 {
            continue;
        }

        let tx=((x-x0)/(x1-x0).max(1.0)).clamp(0.0,1.0);
        let tz=((z-z0)/(z1-z0).max(1.0)).clamp(0.0,1.0);

        let a=v[0][1]*(1.0-tx)*(1.0-tz);
        let b=v[1][1]*tx*(1.0-tz);
        let c=v[2][1]*(1.0-tx)*tz;
        let d=v[3][1]*tx*tz;

        return -(a+b+c+d);
    }

    0.0
}
impl State {
    pub fn load(
        index:&ProtIndex,
        scene:&str
    )->std::result::Result<Self,String> {
        let native=Scene::load(index,scene)
            .map_err(|e|e.to_string())?;

        let mut state=Self{
            scene:scene.into(),
            next_uid:1,
            show_markers:true,
            ..Default::default()
        };

        let Some(map_id)=native.field_map_index(index) else {
            return Ok(state);
        };

        let map=index.entry_bytes_extended(map_id)
            .map_err(|e|e.to_string())?;

        if map.len()<0x12000 {
            return Err("Native MAP shorter than 0x12000".into());
        }

        state.map_fingerprint=
            collision_recipes::digest(&map[0x10000..0x12000]);

        // Retail MAP is streamed as one contiguous 0x28-sector
        // load window. The +0x12000 fallback may span multiple
        // PROT entries, so the next entry's individual footprint
        // must never be used as the complete fallback.
        let lba=index.entry_start_lba_retail(map_id as u16)
            .ok_or("Native MAP disc LBA unavailable")?;

        let physical=index.prot_dat_raw_bytes(
            lba as u64*0x800,
            0x14000,
        ).map_err(|e|
            format!("Read contiguous native MAP window: {e}")
        )?;

        if physical.len()!=0x14000 ||
           physical.get(..0x12000)!=Some(map.as_slice()) {
            return Err(
                "Contiguous MAP window does not match the selected scene's original MAP"
                    .into()
            );
        }

        let fallback=physical[0x12000..0x14000].to_vec();

        state.fallback_fingerprint=
            collision_recipes::digest(&fallback);

        let man=native.field_man_payload(index)
            .map_err(|e|e.to_string())?;

        state.native_p2_records=man.as_ref()
            .and_then(|m|man_section::parse(m).ok())
            .map(|parsed|parsed.partitions[2].len())
            .unwrap_or(0);
        state.man_fingerprint=man.as_ref()
            .map(|x|collision_recipes::digest(x))
            .unwrap_or_default();
        if let Some(man)=man.as_ref(){
            state.inspector_scripts=lck_script_summaries(man);

            state.v2084_man=Some(man.to_vec());

            if let Ok(parsed)=legaia_scene_formats::man_section::parse(man){
                let p2base=parsed.partitions[0].len()
                    +parsed.partitions[1].len();

                for record in 0..parsed.partitions[2].len(){
                    let Ok(flat)=u8::try_from(p2base+record)
                    else{continue;};

                    if let Some(gates)=
                        legaia_engine_field::man_field_scripts::
                            partition2_record_gates(
                                &parsed,man,record
                            )
                    {
                        state.v2084_p2_gates.insert(flat,gates);
                    }
                }
            }
        }

        // Parse BOTH trigger banks from the exact physical load.
        // This also preserves fallback rows when its PROT entry is
        // shorter than the contiguous runtime trigger window.
        let local_a=
            legaia_engine_field::field_regions::
                parse_intra_scene_teleports(&map[0x10000..0x12000]);

        let local_b=
            legaia_engine_field::field_regions::
                parse_intra_scene_teleports(&fallback);

        let walk_a=
            legaia_engine_field::field_regions::
                parse_tile_triggers(&map[0x10000..0x12000]);

        let walk_b=
            legaia_engine_field::field_regions::
                parse_tile_triggers(&fallback);

        // Keep actual MAP objects separate from trigger lookup tiles.
        let native_objects=
            legaia_engine_field::field_regions::
                parse_map_objects(&map);

        let mut sites=
            BTreeMap::<u8,Vec<man_edit::SceneChangeSite>>::new();

        let mut used=BTreeSet::<usize>::new();

        if let Some(man)=man.as_ref() {
            let changes=man_edit::scene_change_sites(man);

            for (site,slot) in changes.iter().cloned()
                .zip(man_edit::site_p2_slots(man,&changes)) {

                if let Some(slot)=slot
                    .and_then(|s|u8::try_from(s).ok()) {
                    sites.entry(slot).or_default().push(site);
                }
            }
        }

        for (fallback,rows) in [
            (false,local_a),
            (true,local_b)
        ] {
            for (row,d) in rows.into_iter().enumerate() {
                let uid=state.allocate();

                state.records.push(Door{
                    uid,
                    label:String::new(),
                    origin:Origin::Local{fallback,row},
                    source:[d.tile_x,d.tile_z],
                    size:[1,1],
                    enabled:true,
                    kind:Kind::Local{
                        half_x:d.dest_x,
                        half_z:d.dest_z,
                    }
                });
            }
        }

        for (fallback,rows) in [
            (false,walk_a),
            (true,walk_b)
        ] {
            for (row,d) in rows.into_iter().enumerate() {
                if d.gate!=1 {
                    let objects=if d.gate==0 {
                        native_objects.iter()
                            .filter(|o|
                                o.key_tile==(d.tile_x,d.tile_z)
                            )
                            .map(|o|(
                                o.descriptor,
                                [o.world.0,o.world.1],
                                [o.contact.0,o.contact.1]
                            ))
                            .collect()
                    }else{
                        Vec::new()
                    };

                    state.other_triggers.push(R605NativeTrigger{
                        kind:1,
                        gate:d.gate,
                        tile:[d.tile_x,d.tile_z],
                        record:d.record,
                        extra:0,
                        fallback,
                        row,
                        objects,
                    });

                    continue;
                }

                let site=sites.get(&d.record)
                    .and_then(|found|
                        (found.len()==1).then_some(&found[0])
                    );

                if let Some(s)=site {
                    used.insert(s.op_pc);
                }

                let destination=site.map(|s|Destination{
                    name:s.name.clone(),
                    index:s.index,
                    entry_x:s.entry_x,
                    entry_z:s.entry_z,
                    facing:s.dir,
                });

                let uid=state.allocate();

                state.records.push(Door{
                    uid,
                    label:String::new(),
                    origin:Origin::Walk{fallback,row},
                    source:[d.tile_x,d.tile_z],
                    size:[1,1],
                    enabled:true,
                    kind:Kind::Script{
                        slot:Some(d.record),
                        op_pc:site.map(|s|s.op_pc),
                        destination,
                    },
                });
            }
        }

        // Native kind-2 records are retained as read-only diagnostics.
        // No serializer or behavioral meaning is invented.
        for (fallback,block) in [
            (false,&map[0x10000..0x12000]),
            (true,&fallback[..])
        ] {
            for (row,raw) in lck_r605_kind2(block)
                .into_iter().enumerate()
            {
                state.other_triggers.push(R605NativeTrigger{
                    kind:2,
                    gate:255,
                    tile:[raw[0],raw[1]],
                    record:raw[2],
                    extra:raw[3],
                    fallback,
                    row,
                    objects:Vec::new(),
                });
            }
        }

        if let Some(man)=man.as_ref() {
            for s in man_edit::scene_change_sites(man) {
                if used.contains(&s.op_pc) {
                    continue;
                }

                let uid=state.allocate();

                state.records.push(Door{
                    uid,
                    label:String::new(),
                    origin:Origin::ScriptOnly{op_pc:s.op_pc},
                    source:[0,0],
                    size:[1,1],
                    enabled:true,
                    kind:Kind::Script{
                        slot:None,
                        op_pc:Some(s.op_pc),
                        destination:Some(Destination{
                            name:s.name,
                            index:s.index,
                            entry_x:s.entry_x,
                            entry_z:s.entry_z,
                            facing:s.dir,
                        }),
                    },
                });
            }
        }

        state.original=state.records.clone();
        state.original_auxiliary=state.other_triggers.clone();
        Ok(state)
    }

    fn allocate(&mut self)->u64 {
        let n=self.next_uid;
        self.next_uid+=1;
        n
    }

    pub fn embedded_record(&self,scene:&str)->String {
        if self.scene!=scene {
            return String::new();
        }

        // R640_CANONICAL_TRIGGER_HEIGHTS
        // Missing display heights mean zero, exactly as restore() interprets them.
        // Preserve explicit entries, including invalid overlong arrays so validation
        // can reject those rather than silently truncating user data.
        let mut auxiliary_heights=self.r630c_aux_y.clone();
        if auxiliary_heights.len()<self.other_triggers.len(){
            auxiliary_heights.resize(self.other_triggers.len(),0.0);
        }
        let saved=Saved{
            version:1,
            scene:scene.into(),
            map_fingerprint:self.map_fingerprint.clone(),
            fallback_fingerprint:self.fallback_fingerprint.clone(),
            man_fingerprint:self.man_fingerprint.clone(),
            records:self.records.clone(),
            auxiliary:self.other_triggers.clone(),
            next_uid:self.next_uid,
            r630c_warp_y:self.r630c_warp_y.clone(),
            r630c_aux_y:auxiliary_heights,
        };

        format!(
            "DOOR_EDIT\t{}\n",
            serde_json::to_string(&saved)
                .expect("Door snapshot serializable")
        )
    }

    pub fn restore(
        &mut self,
        text:&str
    )->std::result::Result<(),String> {
        let records=text.lines()
            .filter_map(|line|line.strip_prefix("DOOR_EDIT\t"))
            .collect::<Vec<_>>();

        if records.len()>1 {
            return Err("Multiple native door overlays".into());
        }

        let Some(json)=records.first() else {
            return Ok(());
        };

        let saved:Saved=serde_json::from_str(json)
            .map_err(|e|format!("Door record JSON: {e}"))?;

        if saved.version!=1 ||
           saved.scene!=self.scene ||
           saved.map_fingerprint!=self.map_fingerprint ||
           saved.fallback_fingerprint!=self.fallback_fingerprint ||
           saved.man_fingerprint!=self.man_fingerprint {
            return Err(
                "Door overlay is for a different MAP/MAN revision; native records preserved"
                    .into()
            );
        }

        let old=self.original.iter()
            .filter(|d|!matches!(&d.origin,Origin::New))
            .map(|d|(d.uid,d.origin.clone()))
            .collect::<Vec<_>>();

        for (uid,origin) in old {
            let Some(saved_door)=saved.records.iter()
                .find(|d|d.uid==uid) else {
                return Err(format!(
                    "Door overlay missing native UID {uid}"
                ));
            };

            if saved_door.origin!=origin {
                return Err(format!(
                    "Door overlay changed native identity {uid}"
                ));
            }
        }

        let mut uids=BTreeSet::new();

        for d in &saved.records {
            if !uids.insert(d.uid) {
                return Err("Repeated door UID".into());
            }

            if d.label.chars().count()>128 ||
               d.label.contains('\r') ||
               d.label.contains('\n') {
                return Err("Invalid door label".into());
            }

            if !matches!(&d.origin,Origin::ScriptOnly{..}) {
                d.geometry().map_err(|e|e.to_string())?;
            }
        }

        if saved.next_uid<=*uids.iter().max().unwrap_or(&0) {
            return Err("Door UID allocation collision".into());
        }

        if !saved.auxiliary.is_empty(){
            if saved.auxiliary.len()!=self.other_triggers.len(){
                return Err(
                    "Saved auxiliary count differs from native MAP".into()
                );
            }

            for (a,b) in saved.auxiliary.iter()
                .zip(self.other_triggers.iter())
            {
                if a.kind!=b.kind
                    || a.gate!=b.gate
                    || a.row!=b.row
                    || a.fallback!=b.fallback
                    || a.objects!=b.objects
                    || a.tile[0]>=128
                    || a.tile[1]>=128
                {
                    return Err(
                        "Saved auxiliary record changed native identity or coordinates".into()
                    );
                }
            }

            self.other_triggers=saved.auxiliary;
        }

        // LCK_R630C_VERTICAL_MARKERS
        // Height is editor metadata; native MAP rows have X/Z only.
        if saved.r630c_warp_y.iter().any(|(&uid,&height)|
            !saved.records.iter().any(|d|d.uid==uid)
                || !height.is_finite()
                || height.abs()>8192.0
        ){
            return Err("Saved warp marker height is invalid".into());
        }

        if !saved.r630c_aux_y.is_empty()
            && saved.r630c_aux_y.len()!=self.other_triggers.len()
        {
            return Err(
                "Saved auxiliary marker height count differs from native trigger inventory".into()
            );
        }

        if saved.r630c_aux_y.iter().any(|height|
            !height.is_finite() || height.abs()>8192.0
        ){
            return Err("Saved auxiliary trigger height is invalid".into());
        }

        self.r630c_warp_y=saved.r630c_warp_y;
        self.r630c_aux_y=saved.r630c_aux_y;
        self.r630c_aux_y.resize(
            self.other_triggers.len(),0.0
        );
        self.records=saved.records;
        self.next_uid=saved.next_uid;
        self.renumber_independent_cross();

        Ok(())
    }

    fn remember(&mut self) {
        self.undo.push(self.records.clone());

        if self.undo.len()>100 {
            self.undo.remove(0);
        }

        self.redo.clear();
    }

    fn undo(&mut self) {
        if let Some(prev)=self.undo.pop() {
            self.redo.push(
                std::mem::replace(&mut self.records,prev)
            );
            self.renumber_independent_cross();
        }
    }

    fn redo(&mut self) {
        if let Some(next)=self.redo.pop() {
            self.undo.push(
                std::mem::replace(&mut self.records,next)
            );
            self.renumber_independent_cross();
        }
    }

    fn duplicate(&mut self) {
        let Some(d)=self.records.iter()
            .find(|r|Some(r.uid)==self.selected)
            .cloned() else {
            return;
        };

        if matches!(&d.origin,Origin::ScriptOnly{..}) {
            self.message=
                "Script-only opcode cannot be duplicated as a trigger; choose a bound walk-door"
                .into();
            return;
        }

        self.remember();

        let mut copy=d;
        copy.uid=self.allocate();
        copy.origin=Origin::New;

        copy.label=format!("{} copy",copy.visible_name());
        copy.source[0]=copy.source[0]
            .saturating_add(1)
            .min(127);

        if copy.source[0] as u16+copy.size[0] as u16>128 {
            copy.source[0]=0;
        }

        let independent=matches!(&copy.kind,Kind::Script{op_pc:None,..});
        self.selected=Some(copy.uid);
        self.records.push(copy);
        if independent {self.renumber_independent_cross();}
        self.message=if independent {
            "Duplicated independent MAP trigger with its OWN new MAN P2 script."
                .into()
        }else{
            "Duplicated MAP trigger; existing MAN script is SHARED."
                .into()
        };
    }

    // LCK_R620_INDEPENDENT_CROSS_CELL
    fn renumber_independent_cross(&mut self) {
        let mut next=self.native_p2_records;
        for d in &mut self.records {
            if matches!(&d.origin,Origin::New) &&
                matches!(&d.kind,Kind::Script{op_pc:None,..}) {
                if let Kind::Script{slot,..}=&mut d.kind {
                    *slot=u8::try_from(next).ok();
                }
                next+=1;
            }
        }
    }

    fn new_cross_scene(&mut self)->Result<()> {
        ensure!(!self.man_fingerprint.is_empty(),
            "This cell has no usable native MAN record table");
        let prior=self.records.iter().filter(|d|
            matches!(&d.origin,Origin::New) &&
            matches!(&d.kind,Kind::Script{op_pc:None,..})
        ).count();
        ensure!(self.native_p2_records+prior<256,
            "No free u8 partition-2 MAN script ID for another cross-cell warp");
        let destination=self.records.iter()
            .filter(|d|Some(d.uid)==self.selected)
            .chain(self.records.iter())
            .find_map(|d|match &d.kind {
                Kind::Script{destination:Some(dest),..}=>Some(dest.clone()),
                _=>None,
            })
            .context("No decoded native 0x3F destination to seed index/CDNAME; select an existing cross-scene door first")?;
        let source=(0..128u8).flat_map(|z|(0..128u8).map(move |x|[x,z]))
            .find(|p|!self.records.iter().any(|d|
                d.enabled && d.source==*p))
            .context("No free MAP trigger tile")?;
        self.remember();
        let uid=self.allocate();
        self.records.push(Door {
            uid,
            label:format!("Independent Cross-Cell Warp {uid}"),
            origin:Origin::New,
            source,
            size:[1,1],
            enabled:true,
            kind:Kind::Script{
                slot:None,
                op_pc:None,
                destination:Some(destination),
            },
        });
        self.renumber_independent_cross();
        self.selected=Some(uid);
        self.message="Created independent MAP trigger and reserved a new MAN script. Edit CDNAME, native index, entry X/Z and facing before export.".into();
        Ok(())
    }
    // LCK_R621_WARP_CONTEXT_MENU
    // Create independent scripted warp beside the clicked marker.
    pub(super) fn r621_create_from_marker(
        &mut self,
        template_uid:Option<u64>,
        source_hint:Option<[u8;2]>
    )->Result<()> {
        if let Some(uid)=template_uid {
            ensure!(
                self.records.iter().any(|d|d.uid==uid),
                "Selected trigger no longer exists"
            );
            self.selected=Some(uid);
        }

        let origin=source_hint.or_else(||{
            template_uid.and_then(|uid|
                self.records.iter()
                    .find(|d|d.uid==uid)
                    .map(|d|d.source)
            )
        });

        // The R620 operation allocates an independent MAN P2 script.
        self.new_cross_scene()?;

        let created=self.selected
            .context("New cross-cell warp was not selected")?;

        if let Some(source)=origin {
            let occupied=self.records.iter()
                .filter(|d|d.uid!=created && d.enabled)
                .flat_map(|d|{
                    let mut tiles=Vec::new();

                    for dz in 0..d.size[1] {
                        for dx in 0..d.size[0] {
                            tiles.push([
                                d.source[0].saturating_add(dx),
                                d.source[1].saturating_add(dz),
                            ]);
                        }
                    }

                    tiles
                })
                .collect::<std::collections::BTreeSet<_>>();

            let mut nearest=None;

            for distance in 1..=8i16 {
                if nearest.is_some(){break;}

                for dz in -distance..=distance {
                    if nearest.is_some(){break;}

                    for dx in -distance..=distance {
                        if dx.abs().max(dz.abs())!=distance {
                            continue;
                        }

                        let x=source[0] as i16+dx;
                        let z=source[1] as i16+dz;

                        if !(0..128).contains(&x) ||
                           !(0..128).contains(&z) {
                            continue;
                        }

                        let candidate=[x as u8,z as u8];

                        if !occupied.contains(&candidate) {
                            nearest=Some(candidate);
                            break;
                        }
                    }
                }
            }

            if let Some(position)=nearest {
                if let Some(d)=self.records.iter_mut()
                    .find(|d|d.uid==created) {
                    d.source=position;
                }
            }
        }

        self.marker_properties=Some(created);

        self.message=format!(
            "Independent cross-cell warp UID {created} created. Set CDNAME, scene index, arrival and facing."
        );

        Ok(())
    }
    fn new_local(&mut self) {
        self.remember();

        let uid=self.allocate();

        self.records.push(Door{
            uid,
            label:"New same-scene doorway".into(),
            origin:Origin::New,
            source:[0,0],
            size:[1,1],
            enabled:true,
            kind:Kind::Local{
                half_x:0,
                half_z:0,
            },
        });

        self.selected=Some(uid);
    }

    fn import(&mut self,index:&ProtIndex)->Result<()> {
        ensure!(
            !self.donor.trim().is_empty(),
            "Enter donor scene CDNAME"
        );

        let donor=Self::load(index,self.donor.trim())
            .map_err(anyhow::Error::msg)?;

        let candidate=donor.records.get(self.donor_row)
            .context("Donor row outside available native door list")?
            .clone();

        ensure!(
            !matches!(&candidate.origin,Origin::ScriptOnly{..}),
            "Script-only donor has no walk-on trigger to import"
        );

        let mut new=candidate.clone();

        if let Kind::Script{
            slot,
            op_pc,
            destination
        }=&mut new.kind {
            let Some(target_dest)=destination else {
                bail!(
                    "Donor gate has no uniquely decoded 0x3F destination"
                );
            };

            let matching=self.records.iter().find_map(|r|
                match &r.kind {
                    Kind::Script{
                        slot:Some(s),
                        op_pc:Some(p),
                        destination:Some(d)
                    } if d==&*target_dest=>Some((*s,*p)),
                    _=>None,
                }
            );

            let Some((native_slot,native_pc))=matching else {
                bail!(
                    "No matching native destination/record in this scene. Cross-cell MAN bytecode transplantation is not yet proven; import rejected."
                );
            };

            *slot=Some(native_slot);
            *op_pc=Some(native_pc);
        }

        self.remember();

        new.uid=self.allocate();
        new.origin=Origin::New;
        new.label=format!(
            "Imported {} / {}",
            self.donor,
            candidate.visible_name()
        );

        self.selected=Some(new.uid);
        self.records.push(new);

        Ok(())
    }

    fn delete_selected(&mut self) {
        let Some(i)=self.records.iter()
            .position(|d|Some(d.uid)==self.selected) else {
            return;
        };

        self.remember();

        if matches!(&self.records[i].origin,Origin::New) {
            self.records.remove(i);
            self.selected=None;
            self.renumber_independent_cross();
        }
        else if !matches!(
            &self.records[i].origin,
            Origin::ScriptOnly{..}
        ) {
            self.records[i].enabled=false;
        }
        else {
            self.message=
                "Script-only operations cannot be deleted without script control-flow authoring"
                    .into();
        }
    }

    fn align_shared(&mut self) {
        let Some(row)=self.records.iter()
            .find(|r|Some(r.uid)==self.selected) else {
            return;
        };

        let Kind::Script{
            op_pc:Some(pc),
            destination:Some(dest),
            ..
        }=&row.kind else {
            return;
        };

        let (pc,dest)=(*pc,dest.clone());
        self.remember();

        for r in &mut self.records {
            if let Kind::Script{
                op_pc:Some(other),
                destination,
                ..
            }=&mut r.kind {
                if *other==pc {
                    *destination=Some(dest.clone());
                }
            }
        }

        self.message=format!(
            "Set every link to shared MAN 0x{pc:X} to the same destination"
        );
    }

    pub(super) fn has_native_changes(&self)->bool {
        let mut live=self.records.clone();
        let mut old=self.original.clone();

        for r in live.iter_mut().chain(old.iter_mut()) {
            r.label.clear();
        }

        live!=old || self.other_triggers!=self.original_auxiliary
    }

    // RMB over a marker opens this menu, not an object rotation.
    fn native_marker_context_ui(
        &mut self,
        ctx:&egui::Context,
        index:Option<&ProtIndex>,
        ground:Option<&legaia_scene_formats::field_objects::WalkHeightfield>
    ) {
        if let Some((uid,pos))=self.marker_menu {
            let mut choose_properties=false;
            let mut create_cross_cell=false;
            let mut close_menu=false;

            let response=egui::Window::new("Trigger marker menu")
                .id(egui::Id::new("lck_native_trigger_context_r3"))
                .title_bar(false)
                .resizable(false)
                .fixed_pos(pos)
                .show(ctx,|ui|{
                    if ui.button("Properties...").clicked(){
                        choose_properties=true;
                    }

                    ui.separator();

                    // LCK_R621_WARP_CONTEXT_MENU
                    if ui.button("New Cross-Cell Warp").clicked(){
                        create_cross_cell=true;
                    }

                    if ui.button("Cancel").clicked(){
                        close_menu=true;
                    }
                });

            if choose_properties {
                self.selected=Some(uid);
                self.marker_properties=Some(uid);
                close_menu=true;
            }

            if create_cross_cell {
                if let Err(error)=self.r621_create_from_marker(
                    Some(uid),None
                ){
                    self.message=format!(
                        "Cross-cell warp creation failed: {error:#}"
                    );
                    self.selected=Some(uid);
                    self.marker_properties=Some(uid);
                }
                close_menu=true;
                ctx.request_repaint();
            }

            if ctx.input(|i|i.key_pressed(egui::Key::Escape)){
                close_menu=true;
            }

            if let Some(response)=response {
                if ctx.input(|i|i.pointer.primary_pressed()) &&
                    ctx.input(|i|i.pointer.interact_pos())
                        .is_some_and(|p|!response.response.rect.contains(p)) {
                    close_menu=true;
                }
            }

            if close_menu {
                self.marker_menu=None;
            }
        }

        // LCK_R622_EDITABLE_DESTINATIONS
        if let Some(uid)=self.marker_properties {
            if self.r622_destinations.is_none() {
                if let Some(index)=index {
                    self.r622_destinations=
                        Some(r622_build_catalog(index));
                }
            }

            let Some(i)=self.records.iter()
                .position(|d|d.uid==uid) else {
                self.marker_properties=None;
                return;
            };

            let scene_name=self.scene.clone();

            let evidence=match &self.records[i].kind {
                Kind::Script{op_pc:Some(pc),..}=>{
                    self.inspector_scripts.values()
                        .find(|s|
                            s.partition==2 &&
                            s.script_start<=*pc &&
                            *pc<s.record_end
                        )
                        .cloned()
                },
                _=>None,
            };

            let before=self.records.clone();
            let named_choices=self.r639_named_catalog.choices(&self.scene,&self.records,uid);
            let named_errors=self.r639_named_catalog.errors.clone();
            let named_search=&mut self.r639_named_search;
            let catalog=self.r622_destinations.as_ref();
            // R627: filter after the search field is edited.

            let search=&mut self.r622_search;
            let marker_height=self.r630c_warp_y
                .entry(uid).or_insert(0.0);
            let door=&mut self.records[i];

            let mut open=true;
            let mut request_export=false;

            egui::Window::new("Warp Properties")
                .id(egui::Id::new("lck_r622_warp_properties"))
                .resizable(true)
                .default_width(455.0)
                .open(&mut open)
                .show(ctx,|ui|{
                    ui.strong(format!(
                        "{} / Warp {}",
                        scene_name,uid
                    ));

                    ui.separator();

                    ui.label("Warp name");
                    ui.text_edit_singleline(&mut door.label);

                    let x=(door.source[0] as f32
                        +door.size[0] as f32*0.5)*128.0;
                    let z=(door.source[1] as f32
                        +door.size[1] as f32*0.5)*128.0;

                    let base=native_door_floor_y(
                        ground,x,z
                    )+LCK_R58_GIZMO_LIFT;

                    let mut world_y=base+*marker_height;

                    ui.horizontal(|ui|{
                        ui.label("Marker Y (world)");
                        if ui.add(
                            egui::DragValue::new(&mut world_y)
                                .speed(8.0)
                                .range(-16384.0..=16384.0)
                        ).changed(){
                            *marker_height=(world_y-base)
                                .clamp(-8192.0,8192.0);
                        }
                    });

                    ui.horizontal(|ui|{
                        ui.label("Y offset from ground marker");
                        ui.add(
                            egui::DragValue::new(marker_height)
                                .speed(8.0)
                                .range(-8192.0..=8192.0)
                        );
                    });

                    ui.small(
                        "Ctrl + left-drag moves this marker vertically. Native activation still uses MAP X/Z."
                    );

                    if door.label.chars().count()>128 {
                        door.label=door.label.chars()
                            .take(128).collect();
                    }

                    ui.separator();
                    ui.strong("Trigger Position");

                    if matches!(
                        &door.origin,
                        Origin::ScriptOnly{..}
                    ) {
                        ui.small(
                            "This is an unbound script rather than a walk-on tile."
                        );
                    }else{
                        ui.horizontal(|ui|{
                            ui.label("Tile X");
                            ui.add(
                                egui::DragValue::new(
                                    &mut door.source[0]
                                ).range(0..=127)
                            );

                            ui.label("Tile Z");
                            ui.add(
                                egui::DragValue::new(
                                    &mut door.source[1]
                                ).range(0..=127)
                            );
                        });

                        ui.horizontal(|ui|{
                            ui.label("Width");
                            ui.add(
                                egui::DragValue::new(
                                    &mut door.size[0]
                                ).range(1..=32)
                            );

                            ui.label("Depth");
                            ui.add(
                                egui::DragValue::new(
                                    &mut door.size[1]
                                ).range(1..=32)
                            );
                        });

                        ui.checkbox(
                            &mut door.enabled,
                            "Enabled"
                        );

                        ui.small(
                            "Position this trigger on your colored floor tile. The floor color is visual; entering the trigger tile activates the warp."
                        );
                    }

                    ui.separator();

                    match &mut door.kind {
                        Kind::Local{half_x,half_z}=>{
                            ui.strong("Same-Cell Destination");

                            ui.small(
                                "This original MAP kind-0 warp stays within the current cell."
                            );

                            ui.horizontal(|ui|{
                                ui.label("Arrival X");
                                ui.add(
                                    egui::DragValue::new(half_x)
                                );

                                ui.label("Arrival Z");
                                ui.add(
                                    egui::DragValue::new(half_z)
                                );
                            });

                            ui.small(
                                "Use right-click > New Cross-Cell Warp to create an independent scene-changing trigger."
                            );
                        },

                        Kind::Script{
                            slot,
                            op_pc,
                            destination,
                        }=>{
                            if let Some(dest)=destination.as_mut(){
                                named_targets::picker(ui,uid,dest,
                                    catalog.map(|rows|rows.as_slice()),
                                    &named_choices,named_search,&named_errors);
                                ui.strong("Destination Cell");
                                if let Some(catalog)=catalog {
                                    ui.horizontal(|ui|{
                                        ui.label("Search");
                                        ui.text_edit_singleline(search);
                                    });

                                    // LCK_R627_DESTINATION_BROWSER
                                    // LCK_R627A_KEEP_DROPDOWN_OPEN
egui::ComboBox::from_id_salt(
                                        ("r622_cell",uid)
                                    )
                                        .width(395.0)
                                        .selected_text(format!(
                                            "{} ({})",
                                            super::lck_r627_cell_family(
                                                &dest.name
                                            ),
                                            dest.name
                                        ))
                                        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                                        .show_ui(ui,|ui|{
                                            // Filter uses the text as edited
                                            // in this UI frame.
                                            let groups=
                                                r627_destination_groups(
                                                    catalog,search
                                                );

                                            if groups.is_empty(){
                                                ui.label(
                                                    "No matching locations or scene IDs."
                                                );
                                            }

                                            for (family,scenes) in groups {
                                                egui::CollapsingHeader::new(
                                                    format!(
                                                        "{} ({})",
                                                        family,
                                                        scenes.len()
                                                    )
                                                )
                                                    .id_salt((
                                                        "r627_warp_location",
                                                        uid,
                                                        family.as_str()
                                                    ))
                                                    .default_open(false)
                                                    .open(
                                                        if search.trim().is_empty(){
                                                            None
                                                        }else{
                                                            Some(true)
                                                        }
                                                    )
                                                    .show(ui,|ui|{
                                                        for cell in scenes {
                                                            let readable=
                                                                crate::place_name(
                                                                    &cell.name
                                                                ).unwrap_or(
                                                                    &cell.name
                                                                );

                                                            let available=
                                                                !cell.arrivals.is_empty();

                                                            let label=if available {
                                                                format!(
                                                                    "{} ({}) - {} arrival(s)",
                                                                    readable,
                                                                    cell.name,
                                                                    cell.arrivals.len()
                                                                )
                                                            }else{
                                                                format!(
                                                                    "{} ({}) - no known arrival",
                                                                    readable,
                                                                    cell.name
                                                                )
                                                            };

                                                            if ui.add_enabled(
                                                                available,
                                                                egui::Button::new(label)
                                                            ).clicked(){
                                                                r622_select_arrival(
                                                                    dest,cell,0
                                                                );
                                                                egui::Popup::close_all(ui.ctx());
                                                            }
                                                        }
                                                    });
                                            }
                                        });
                                    ui.separator();
                                    ui.strong("Destination Position");

                                    if let Some(cell)=catalog.iter()
                                        .find(|c|c.name==dest.name)
                                    {
                                        egui::ComboBox::from_id_salt(
                                            ("r622_position",uid)
                                        )
                                            .width(395.0)
                                            .selected_text(format!(
                                                "Tile X {} / Z {} / facing {}",
                                                dest.entry_x&127,
                                                dest.entry_z&127,
                                                dest.facing
                                            ))
                                            .show_ui(ui,|ui|{
                                                for (position,arrival) in
                                                    cell.arrivals.iter()
                                                        .enumerate()
                                                {
                                                    let label=format!(
                                                        "X {} Z {} / face {} / from {}",
                                                        arrival.x&127,
                                                        arrival.z&127,
                                                        arrival.facing,
                                                        arrival.from_scene
                                                    );

                                                    let selected=
                                                        dest.index==arrival.index &&
                                                        dest.entry_x==arrival.x &&
                                                        dest.entry_z==arrival.z &&
                                                        dest.facing==arrival.facing;

                                                    if ui.selectable_label(
                                                        selected,label
                                                    ).clicked(){
                                                        r622_select_arrival(
                                                            dest,
                                                            cell,
                                                            position
                                                        );
                                                        ui.close();
                                                    }
                                                }
                                            });
                                    }
                                }else{
                                    ui.colored_label(
                                        egui::Color32::YELLOW,
                                        "Destination catalog unavailable."
                                    );
                                }

                                ui.separator();
                                ui.strong("Fine-Tune Arrival Position");

                                let mut x=dest.entry_x&127;
                                let mut z=dest.entry_z&127;
                                let mut x_far=(dest.entry_x&128)!=0;
                                let mut z_far=(dest.entry_z&128)!=0;

                                ui.horizontal(|ui|{
                                    ui.label("X");
                                    if ui.add(
                                        egui::DragValue::new(&mut x)
                                            .range(0..=127)
                                    ).changed(){
                                        dest.entry_x=
                                            (dest.entry_x&128)|x;
                                    }

                                    ui.label("Z");
                                    if ui.add(
                                        egui::DragValue::new(&mut z)
                                            .range(0..=127)
                                    ).changed(){
                                        dest.entry_z=
                                            (dest.entry_z&128)|z;
                                    }
                                });

                                ui.horizontal(|ui|{
                                    if ui.checkbox(
                                        &mut x_far,
                                        "X far half"
                                    ).changed(){
                                        dest.entry_x=
                                            (dest.entry_x&127) |
                                            if x_far {128}else{0};
                                    }

                                    if ui.checkbox(
                                        &mut z_far,
                                        "Z far half"
                                    ).changed(){
                                        dest.entry_z=
                                            (dest.entry_z&127) |
                                            if z_far {128}else{0};
                                    }
                                });

                                ui.horizontal(|ui|{
                                    ui.label("Facing");
                                    ui.add(
                                        egui::DragValue::new(
                                            &mut dest.facing
                                        ).range(0..=7)
                                    );
                                });

                                ui.small(
                                    "The native destination index is assigned from the selected retail arrival. Changing coordinates keeps that index."
                                );
                            }else{
                                ui.colored_label(
                                    egui::Color32::YELLOW,
                                    "This script has no decoded scene-change destination."
                                );
                            }

                            if op_pc.is_some() {
                                ui.small(
                                    "Existing native warp. Its MAN script may be shared with other triggers."
                                );
                            }else if slot.is_some(){
                                ui.small(
                                    "Independent editor-created cross-cell warp."
                                );
                            }
                        },
                    }

                    ui.separator();

                    if ui.button(
                        "Export IMG for DuckStation..."
                    ).clicked(){
                        request_export=true;
                    }

                    ui.small(
                        "Uses the unified exporter, including pending floor and warp edits. Save Project (Ctrl+S) to preserve your work."
                    );

                    ui.collapsing(
                        "Advanced / Native Diagnostics",
                        |ui|{
                            ui.label(format!(
                                "UID: {}",
                                door.uid
                            ));

                            ui.label(format!(
                                "Source scene: {}",
                                scene_name
                            ));

                            ui.label(format!(
                                "Origin: {:?}",
                                door.origin
                            ));

                            if let Kind::Script{
                                slot,op_pc,destination
                            }=&door.kind {
                                ui.label(format!(
                                    "MAN P2 slot: {:?}",
                                    slot
                                ));

                                ui.label(format!(
                                    "0x3F opcode: {:?}",
                                    op_pc
                                ));

                                if let Some(dest)=destination {
                                    ui.label(format!(
                                        "Native destination index: {}",
                                        dest.index
                                    ));

                                    ui.label(format!(
                                        "Encoded arrival: X {}, Z {}, facing {}",
                                        dest.entry_x,
                                        dest.entry_z,
                                        dest.facing
                                    ));
                                }
                            }

                            if let Some(summary)=&evidence {
                                ui.separator();

                                ui.label(format!(
                                    "P2 record: +0x{:X}..+0x{:X}",
                                    summary.record_start,
                                    summary.record_end
                                ));

                                for flag in &summary.flags {
                                    ui.monospace(flag);
                                }

                                ui.collapsing(
                                    "Recognized instructions",
                                    |ui|{
                                        for operation in
                                            &summary.operations
                                        {
                                            ui.monospace(operation);
                                        }
                                    }
                                );
                            }else{
                                ui.small(
                                    "No independently verified original P2 record summary."
                                );
                            }
                        }
                    );
                });

            if before!=self.records {
                self.undo.push(before);

                if self.undo.len()>100 {
                    self.undo.remove(0);
                }

                self.redo.clear();
            }

            if request_export {
                self.r622_export_requested=true;
                ctx.request_repaint();
            }

            if !open {
                self.marker_properties=None;
            }
        }
    }
    // LCK_R4_STORY_FLAG_TOOLBAR: read-only, per-scene decoded references.
    pub(super) fn story_flag_references(&self)->Vec<String> {
        let mut names=std::collections::BTreeSet::<String>::new();
        for script in self.inspector_scripts.values() {
            for reference in &script.flags {
                let name=reference.split(", MAN +").next()
                    .unwrap_or(reference.as_str()).trim();
                if !name.is_empty() { names.insert(name.to_owned()); }
            }
        }
        names.into_iter().collect()
    }
    // LCK_R4_2_CELL_FLAGS
    // Restrict toolbar flags to script records referenced by
    // native triggers/transitions belonging to the active cell.
    //
    // Scene identity is checked explicitly. Never reuse flags
    // from a previously loaded cell.
    //
    // This is a read-only reference browser, not an interpreter
    // of gameplay story state.
    pub(super) fn story_flag_references_for_cell(
        &self,
        cell:&str
    )->Vec<String>{
        if self.scene!=cell {
            return Vec::new();
        }

        let mut linked=std::collections::BTreeSet::<u8>::new();

        for record in &self.records {
            if let Kind::Script{slot,op_pc,..}=&record.kind {
                if let Some(index)=slot {
                    if self.inspector_scripts.contains_key(index) {
                        linked.insert(*index);
                        continue;
                    }
                }

                // A standalone scene-change marker may identify
                // its parent script by the native instruction PC.
                if let Some(pc)=op_pc {
                    for (&index,summary) in &self.inspector_scripts {
                        if summary.script_start<=*pc &&
                            *pc<summary.record_end {
                            linked.insert(index);
                        }
                    }
                }
            }
        }

        let mut names=std::collections::BTreeSet::<String>::new();

        for index in linked {
            if let Some(script)=self.inspector_scripts.get(&index) {
                for reference in &script.flags {
                    let name=reference.split(", MAN +")
                        .next()
                        .unwrap_or(reference.as_str())
                        .trim();

                    if !name.is_empty() {
                        names.insert(name.to_owned());
                    }
                }
            }
        }

        names.into_iter().collect()
    }
    // LCK_R606_NAMED_EVENT_HIERARCHY
    // Native names are evidence, not proof that the record represents
    // a physical door mesh. Never replace an original identifier.
    fn r606_man_name(record:&ScriptSummary)->String {
        match record.native_name.as_deref() {
            Some(raw) if !raw.trim().is_empty()=>{
                match super::descriptive_names::description(raw) {
                    Some(noun)=>format!("{noun} — {raw}"),
                    None=>raw.to_owned(),
                }
            },
            _=>format!(
                "Unlabeled MAN record #{} (P{})",
                record.flat_index,record.partition
            ),
        }
    }

    fn r606_script_for(&self,door:&Door)->Option<&ScriptSummary> {
        let Kind::Script{slot,op_pc,..}=&door.kind else {
            return None
        };

        if let Some(pc)=op_pc {
            return self.inspector_scripts.values().find(|s|
                s.script_start<=*pc && *pc<s.record_end
            );
        }

        slot.and_then(|id|self.inspector_scripts.get(&id))
            .filter(|s|s.partition==2)
    }

    fn r606_door_label(&self,door:&Door)->String {
        if !door.label.trim().is_empty(){
            return format!("{} [editor name]",door.label.trim());
        }

        match &door.kind {
            Kind::Local{half_x,half_z}=>format!(
                "Unnamed interior warp → landing ({half_x},{half_z}) [half-tiles]"
            ),

            Kind::Script{slot,op_pc,destination}=>{
                let native=self.r606_script_for(door)
                    .map(Self::r606_man_name)
                    .unwrap_or_else(||format!(
                        "Unnamed MAN trigger (operand {:?}, opcode {:?})",
                        slot,op_pc
                    ));

                let verified=op_pc.and_then(|pc|
                    self.inspector_scripts.values().find(|s|
                        s.script_start<=pc && pc<s.record_end
                    )
                ).is_some();

                if verified {
                    if let Some(dest)=destination {
                        let friendly=crate::place_name(&dest.name)
                            .unwrap_or(dest.name.as_str());

                        return format!(
                            "{native} → {friendly} [{}]",
                            dest.name
                        );
                    }
                }

                native
            }
        }
    }

    fn r606_door_group(door:&Door)->usize {
        if !door.enabled {return 5;}

        if matches!(&door.origin,Origin::New) {
            return 4;
        }

        if matches!(&door.origin,Origin::ScriptOnly{..}) {
            return 3;
        }

        match &door.kind {
            Kind::Local{..}=>0,
            Kind::Script{slot:Some(_),..}=>1,
            Kind::Script{..}=>2,
        }
    }

    pub(super) fn named_hierarchy(
        &mut self,
        ui:&mut egui::Ui,
        filter:&str
    ){
        ui.small(
            "Native MAN names first; verified descriptive translations appear alongside Japanese labels. MAP-only records may have no native name."
        );

        let mut select=None;

        let groups=[
            "Local / Interior Warps",
            "Cross-Cell Warps",
            "Other Script-bound Triggers",
            "Standalone MAN Scene Changes",
            "New Editor Warps",
            "Disabled / Removed Triggers",
        ];

        for (category,title) in groups.iter().enumerate() {
            let count=self.records.iter()
                .filter(|d|Self::r606_door_group(d)==category)
                .count();

            egui::CollapsingHeader::new(format!("{title} ({count})"))
                .default_open(false)
                .show(ui,|ui|{
                    for door in self.records.iter()
                        .filter(|d|Self::r606_door_group(d)==category)
                    {
                        let name=self.r606_door_label(door);

                        let label=format!(
                            "{name} | UID {} | MAP tile ({},{}) | {:?}",
                            door.uid,
                            door.source[0],
                            door.source[1],
                            door.origin
                        );

                        if !filter.is_empty()
                            && !label.to_lowercase().contains(filter)
                        {
                            continue;
                        }

                        let response=ui.selectable_label(
                            self.selected==Some(door.uid),&label
                        );
                        if response.clicked(){
                            select=Some(door.uid);
                        }
                        response.context_menu(|ui|{
                            if ui.button("Show in World Repository").clicked(){
                                super::repo_jump::request(
                                    ui.ctx(),
                                    super::repo_jump::Target::Record{
                                        kind:"Door / warp / trigger".into(),
                                        title:label.clone(),
                                        details:format!(
                                            "Native scene {} | UID {} | source MAP cell ({},{}) | origin {:?}",
                                            self.scene,door.uid,
                                            door.source[0],door.source[1],
                                            door.origin
                                        )
                                    }
                                );
                                ui.close();
                            }
                        });
                    }
                });
        }

        if let Some(uid)=select {
            self.selected=Some(uid);
            self.marker_properties=Some(uid);
        }

        let extra_groups=[
            "Object-bound Script Contacts (MAP gate 0)",
            "MAP Kind-2 Records (unverified function)",
            "Other MAP Trigger Gates",
        ];

        for (category,title) in extra_groups.iter().enumerate(){
            let matches_kind=|t:&R605NativeTrigger|match category {
                0=>t.kind==1 && t.gate==0,
                1=>t.kind==2,
                _=>t.kind==1 && t.gate!=0 && t.gate!=1,
            };

            let count=self.other_triggers.iter()
                .filter(|t|matches_kind(t))
                .count();

            egui::CollapsingHeader::new(format!("{title} ({count})"))
                .default_open(false)
                .show(ui,|ui|{
                    for t in self.other_triggers.iter()
                        .filter(|t|matches_kind(t))
                    {
                        // MAP gate-0's operand identifies a P0 object
                        // binding. Do not use it as a P2 script name.
                        let native=if t.kind==1 && t.gate==0 {
                            self.inspector_scripts.get(&t.record)
                                .filter(|s|s.partition==0)
                                .map(Self::r606_man_name)
                        }else{
                            None
                        };

                        let name=native.unwrap_or_else(||match category {
                            0=>"Unnamed object-binding trigger".into(),
                            1=>"Unnamed MAP kind-2 record".into(),
                            _=>"Unknown MAP gate record".into(),
                        });

                        let bank=if t.fallback {
                            "fallback"
                        }else{
                            "primary"
                        };

                        let label=format!(
                            "{name} | {bank} row {} | tile ({},{}) | native operand {} | gate {} | raw {}",
                            t.row,
                            t.tile[0],
                            t.tile[1],
                            t.record,
                            t.gate,
                            t.extra
                        );

                        if !filter.is_empty()
                            && !label.to_lowercase().contains(filter)
                        {
                            continue;
                        }

                        ui.label(&label).context_menu(|ui|{
    if ui.button("Show in World Repository").clicked(){
        super::repo_jump::request(
            ui.ctx(),
            super::repo_jump::Target::Record{
                kind:"MAN / MAP / story source record".into(),
                title:label.clone(),
                details:label.clone()
            }
        );
        ui.close();
    }
});

                        for (descriptor,world,contact) in &t.objects {
                            ui.small(format!(
                                "MAP descriptor {descriptor} | object X/Z ({},{}) | contact X/Z ({},{})",
                                world[0],world[1],
                                contact[0],contact[1]
                            ));
                        }
                    }
                });
        }

        let partitions=[
            "MAN P0 — Named Objects / Bindings",
            "MAN P1 — Field Actors",
            "MAN P2 — Script / Event Records",
        ];

        for (partition,title) in partitions.iter().enumerate(){
            let count=self.inspector_scripts.values()
                .filter(|s|s.partition==partition)
                .count();

            egui::CollapsingHeader::new(format!("{title} ({count})"))
                .default_open(false)
                .show(ui,|ui|{
                    for script in self.inspector_scripts.values()
                        .filter(|s|s.partition==partition)
                    {
                        let name=Self::r606_man_name(script);

                        let label=format!(
                            "{name} | flat MAN #{} | +0x{:X}..+0x{:X}",
                            script.flat_index,
                            script.script_start,
                            script.record_end
                        );

                        if !filter.is_empty()
                            && !label.to_lowercase().contains(filter)
                        {
                            continue;
                        }

                        ui.label(&label).context_menu(|ui|{
    if ui.button("Show in World Repository").clicked(){
        super::repo_jump::request(
            ui.ctx(),
            super::repo_jump::Target::Record{
                kind:"MAN / MAP / story source record".into(),
                title:label.clone(),
                details:label.clone()
            }
        );
        ui.close();
    }
});
                    }
                });
        }

        let refs=self.inspector_scripts.values()
            .map(|s|s.flags.len())
            .sum::<usize>();

        egui::CollapsingHeader::new(
            format!("Story / Condition Flag References ({refs})")
        )
        .default_open(false)
        .show(ui,|ui|{
            ui.small(
                "Read-only references, not evaluated runtime story conditions."
            );

            for script in self.inspector_scripts.values(){
                let name=Self::r606_man_name(script);

                for reference in &script.flags {
                    let label=format!(
                        "{name} | MAN #{} | {reference}",
                        script.flat_index
                    );

                    if filter.is_empty()
                        || label.to_lowercase().contains(filter)
                    {
                        ui.label(&label).context_menu(|ui|{
    if ui.button("Show in World Repository").clicked(){
        super::repo_jump::request(
            ui.ctx(),
            super::repo_jump::Target::Record{
                kind:"MAN / MAP / story source record".into(),
                title:label.clone(),
                details:label.clone()
            }
        );
        ui.close();
    }
});
                    }
                }
            }
        });
    }
    pub fn ui(
        &mut self,
        ui:&mut egui::Ui,
        index:&ProtIndex,
        source:&Path
    )->Option<[f32;3]> {
        let mut frame=None;

        egui::CollapsingHeader::new(
            format!("Doors & Transitions ({})",self.records.len())
        )
        .default_open(false)
        .show(ui,|ui|{
            ui.checkbox(
                &mut self.show_markers,
                "Show Warps"
            );

            ui.small(
                "MAP kind-0 = interior teleport; MAP kind-1 gate-1 = MAN script; 0x3F = scene change. No invented door meshes."
            );

            ui.small(
                "Overlapping trigger tiles preserve retail first-match order; later rows may be shadowed."
            );

            ui.horizontal(|ui|{
                if ui.button("New Local Warp").clicked() {
                    self.new_local();
                }


                if ui.button("Duplicate").clicked() {
                    self.duplicate();
                }

                if ui.button("Delete").clicked() {
                    self.delete_selected();
                }
            });

            ui.horizontal(|ui|{
                if ui.button("Undo door edit").clicked() {
                    self.undo();
                }

                if ui.button("Redo door edit").clicked() {
                    self.redo();
                }
            });

            ui.separator();

            egui::CollapsingHeader::new(format!(
                "Additional MAP Triggers ({})",
                self.other_triggers.len()
            ))
            .default_open(false)
            .show(ui,|ui|{
                ui.small(
                    "Read-only native records. Object binds and kind-2 records are not automatically player teleports."
                );

                for trigger in &self.other_triggers {
                    let bank=if trigger.fallback {
                        "fallback"
                    }else{
                        "primary"
                    };

                    let category=match (trigger.kind,trigger.gate) {
                        (1,0)=>"Object script binding",
                        (2,_)=>"MAP kind-2 record",
                        _=>"Other gate value",
                    };

                    ui.label(format!(
                        "{category} | {bank} row {} | tile ({},{}) | MAN/record {} | gate {} | raw extra {}",
                        trigger.row,
                        trigger.tile[0],
                        trigger.tile[1],
                        trigger.record,
                        trigger.gate,
                        trigger.extra
                    ));

                    for (descriptor,world,contact) in &trigger.objects {
                        ui.small(format!(
                            "    MAP descriptor {descriptor} | object ({},{}) | contact ({},{})",
                            world[0],world[1],
                            contact[0],contact[1]
                        ));
                    }
                }
            });

            ui.separator();

            for d in &self.records {
                let kind=match &d.kind {
                    Kind::Local{..}=>"local",
                    Kind::Script{slot:Some(_),..}=>"cross-scene",
                    Kind::Script{..}=>"script-only",
                };

                let suffix=if d.enabled {
                    ""
                }else{
                    " [disabled]"
                };

                if ui.selectable_label(
                    self.selected==Some(d.uid),
                    format!(
                        "{} [{kind}]{}",
                        d.visible_name(),
                        suffix
                    )
                ).clicked(){
                    self.selected=Some(d.uid);
                }
            }

            if let Some(id)=self.selected {
                if let Some(i)=self.records.iter()
                    .position(|d|d.uid==id) {

                    ui.separator();
                    ui.strong("Selected door / teleport");

                    let before=self.records.clone();

                    {
                        let d=&mut self.records[i];

                        ui.text_edit_singleline(&mut d.label);
                        ui.small(format!(
                            "UID {} / native {:?}",
                            d.uid,d.origin
                        ));

                        let scripted_only=matches!(
                            &d.origin,
                            Origin::ScriptOnly{..}
                        );

                        if !scripted_only {
                            ui.horizontal(|ui|{
                                ui.label("Trigger tile X/Z");

                                ui.add(
                                    egui::DragValue::new(
                                        &mut d.source[0]
                                    ).range(0..=127)
                                );

                                ui.add(
                                    egui::DragValue::new(
                                        &mut d.source[1]
                                    ).range(0..=127)
                                );
                            });

                            ui.horizontal(|ui|{
                                ui.label("Area W/H in tiles");

                                ui.add(
                                    egui::DragValue::new(
                                        &mut d.size[0]
                                    ).range(1..=32)
                                );

                                ui.add(
                                    egui::DragValue::new(
                                        &mut d.size[1]
                                    ).range(1..=32)
                                );
                            });

                            ui.checkbox(
                                &mut d.enabled,
                                "Enabled in native MAP"
                            );

                            if ui.button("Frame trigger").clicked() {
                                frame=Some([
                                    d.source[0] as f32*128.0+64.0,
                                    0.0,
                                    d.source[1] as f32*128.0+64.0
                                ]);
                            }
                        }

                        match &mut d.kind {
                            Kind::Local{half_x,half_z}=>{
                                ui.strong(
                                    "Same-scene landing (64-unit steps)"
                                );

                                ui.horizontal(|ui|{
                                    ui.label(
                                        "Half-tile destination X/Z"
                                    );
                                    ui.add(
                                        egui::DragValue::new(half_x)
                                    );
                                    ui.add(
                                        egui::DragValue::new(half_z)
                                    );
                                });

                                if ui.button("Frame destination").clicked() {
                                    frame=Some([
                                        *half_x as f32*64.0+64.0,
                                        0.0,
                                        (*half_z as f32+1.0)*64.0,
                                    ]);
                                }
                            },

                            Kind::Script{
                                slot,
                                op_pc,
                                destination
                            }=>{
                                ui.small(format!(
                                    "MAN partition-2 slot {:?}; 0x3F opcode {:?}. Script is SHARED with other triggers using this slot.",
                                    slot,op_pc
                                ));

                                if let Some(dest)=destination {
                                    ui.strong(
                                        "Cross-scene arrival operands"
                                    );

                                    ui.horizontal(|ui|{
                                        ui.label("CDNAME");
                                        ui.text_edit_singleline(
                                            &mut dest.name
                                        );
                                    });

                                    ui.horizontal(|ui|{
                                        ui.label(
                                            "Native destination index"
                                        );

                                        ui.add(
                                            egui::DragValue::new(
                                                &mut dest.index
                                            )
                                        );
                                    });

                                    ui.horizontal(|ui|{
                                        ui.label("Entry X/Z");

                                        ui.add(
                                            egui::DragValue::new(
                                                &mut dest.entry_x
                                            )
                                        );

                                        ui.add(
                                            egui::DragValue::new(
                                                &mut dest.entry_z
                                            )
                                        );
                                    });

                                    ui.horizontal(|ui|{
                                        ui.label("Facing 0..7");

                                        ui.add(
                                            egui::DragValue::new(
                                                &mut dest.facing
                                            ).range(0..=7)
                                        );
                                    });

                                    ui.small(
                                        "Changing the scene name also requires the matching native destination index. Unsupported combinations fail native preflight."
                                    );
                                }
                                else {
                                    ui.colored_label(
                                        egui::Color32::YELLOW,
                                        "No unique native 0x3F site; valid trigger binding can be duplicated, but destination cannot be edited here."
                                    );
                                }
                            },
                        }
                    }

                    if before!=self.records {
                        self.undo.push(before);

                        if self.undo.len()>100 {
                            self.undo.remove(0);
                        }

                        self.redo.clear();
                    }

                    if ui.button(
                        "Apply destination to all shared-script doors"
                    ).clicked(){
                        self.align_shared();
                    }
                }
            }

            ui.separator();
            ui.strong(
                "Import eligible door from another original scene"
            );

            ui.horizontal(|ui|{
                ui.label("CDNAME");
                ui.text_edit_singleline(&mut self.donor);
            });

            ui.horizontal(|ui|{
                ui.label("Door row (0-based)");

                ui.add(
                    egui::DragValue::new(
                        &mut self.donor_row
                    )
                );

                if ui.button("Import").clicked() {
                    match self.import(index) {
                        Ok(())=>{
                            self.message=
                                "Imported a native compatible trigger into this scene"
                                    .into();
                        },
                        Err(e)=>{
                            self.message=format!(
                                "Import blocked: {e:#}"
                            );
                        },
                    }
                }
            });

            ui.small(
                "Local teleports are portable as records. Cross-scene imports need an existing matching MAN destination in the target; arbitrary donor scripts are not silently transplanted."
            );

            if self.has_native_changes() {
                if ui.button(
                    "Export DOORS to a NEW IMG..."
                ).clicked() {
                    let name=format!(
                        "LCK_{}_DOORS_EXPERIMENTAL.img",
                        self.scene
                    );

                    if let Some(dest)=rfd::FileDialog::new()
                        .set_file_name(name)
                        .add_filter(
                            "Raw PS1 IMG",
                            &["img"]
                        )
                        .save_file() {

                        match self.export_new(
                            source,
                            &dest,
                            index
                        ){
                            Ok(report)=>{
                                self.message=report;
                            },
                            Err(e)=>{
                                self.message=format!(
                                    "Door IMG export FAILED: {e:#}"
                                );
                            },
                        }
                    }
                }
            }

            if !self.message.is_empty() {
                ui.separator();
                ui.label(&self.message);
            }
        });

        frame
    }

    pub fn viewport(
        &mut self,
        ui:&mut egui::Ui,
        view:&super::viewport::Viewport,
        model:&crate::raster::Model,
        ground:Option<&legaia_scene_formats::field_objects::WalkHeightfield>,
        index:Option<&ProtIndex>,
    )->bool{
        if !self.show_markers {
            return false;
        }

        let Some((rect,_))=view.world_image else {
            return false;
        };

        // Share the exact image rect, pixel cap, camera and vertical
        // mirror with the actual scene renderer.
        let project=|p:[f32;3]|->Option<egui::Pos2>{
            view.project_world_marker(model,p)
        };


        let pointer=ui.input(|i|
            i.pointer.interact_pos()
        );

        let down=ui.input(|i|
            i.pointer.primary_pressed()
        );

        // R630C: movement axis selected when mouse is pressed.
        let ctrl=ui.input(|i|i.modifiers.ctrl);
        self.marker_right_consumed=false;
        let right=ui.input(|i|i.pointer.secondary_pressed());
        let mut hit=false;
        let mut click=None;

        for d in &self.records {
            if !self.v2082_door_visible(d){
                continue;
            }

            let x=(
                d.source[0] as f32+
                d.size[0] as f32*0.5
            )*128.0;

            let z=(
                d.source[1] as f32+
                d.size[1] as f32*0.5
            )*128.0;

            let Some(pos)=project([x,native_door_floor_y(ground,x,z)+LCK_R58_GIZMO_LIFT+self.r630c_warp_y.get(&d.uid).copied().unwrap_or(0.0),z]) else {
                continue;
            };

            let radius=18.0;

            let hover=pointer.is_some_and(|p|
                p.distance(pos)<radius
            );

            if down && hover {
                hit=true;
                click=Some(d.uid);
            }
            let marker_uncovered=pointer.and_then(|p|ui.ctx().layer_id_at(p))
                .is_none_or(|layer|layer==ui.layer_id());

            if right && hover && marker_uncovered {
                if let Some(p)=pointer {
                    self.selected=Some(d.uid);
                    self.marker_menu=Some((d.uid,p));
                    self.marker_right_consumed=true;
                }
            }

            // LCK_R58_WORLD_MARKERS: visual geometry is rendered in 3D above.
        }

        // V2.0.8.2F: auxiliary records have independent
        // right-click Properties selection.
        if (right || (down && ctrl && click.is_none()))
            && !self.marker_right_consumed {
            let uncovered=pointer
                .and_then(|p|ui.ctx().layer_id_at(p))
                .is_none_or(|layer|layer==ui.layer_id());

            let mut chosen=None;

            if uncovered {
                'aux: for (index,t) in
                    self.other_triggers.iter().enumerate()
                {
                    if !self.v2082_aux_visible(t){
                        continue;
                    }

                    for (x,z) in self.v2082_aux_points(index,t){
                        let y=native_door_floor_y(
                            ground,x,z
                        )+LCK_R58_GIZMO_LIFT
                            +self.r630c_aux_y.get(index)
                                .copied().unwrap_or(0.0);

                        let Some(pos)=project([x,y,z])
                        else{continue;};

                        if pointer.is_some_and(|p|
                            p.distance(pos)<16.0
                        ){
                            chosen=Some((index,x,z));
                            break 'aux;
                        }
                    }
                }
            }

            if let Some((index,x,z))=chosen {
                if right {
                    self.aux_properties=Some(index);
                    self.marker_right_consumed=true;
                    self.marker_menu=None;
                }else if let Some(start)=pointer {
                    let height=self.r630c_aux_y.get(index)
                        .copied().unwrap_or(0.0);

                    self.r630c_aux_drag=Some(R630CAuxDrag{
                        index,
                        start,
                        anchor:[
                            x,
                            native_door_floor_y(ground,x,z)
                                +LCK_R58_GIZMO_LIFT+height,
                            z
                        ],
                        height_start:height,
                    });
                    hit=true;
                }
            }
        }
        if ui.input(|i|i.pointer.primary_pressed()) && click.is_none(){
            self.r604_marker_focus=false;
            self.r604_marker_drag=None;
        }

        if let Some(uid)=click {
            self.selected=Some(uid);
            self.r604_marker_focus=true;

            if let (Some(start),Some(d))=(
                pointer,
                self.records.iter().find(|d|d.uid==uid)
            ){
                let x=(d.source[0] as f32+d.size[0] as f32*0.5)*128.0;
                let z=(d.source[1] as f32+d.size[1] as f32*0.5)*128.0;

                self.r604_marker_drag=Some(R604MarkerDrag{
                    uid,
                    start,
                    anchor:[
                        x,native_door_floor_y(ground,x,z)
                            +LCK_R58_GIZMO_LIFT
                            +self.r630c_warp_y.get(&uid)
                                .copied().unwrap_or(0.0),z
                    ],
                    source:d.source,
                    changed:false,
                    vertical:ctrl,
                    height_start:self.r630c_warp_y.get(&uid)
                        .copied().unwrap_or(0.0)
                });
            }
        }

        // LCK_R630C_VERTICAL_MARKERS
        // Normal LMB = X/Z. Ctrl + LMB = Y only.
        if let Some(drag)=self.r604_marker_drag.clone(){
            if !ui.input(|i|i.pointer.primary_down()){
                self.r604_marker_drag=None;
            }else if let Some(pos)=pointer {
                if (pos-drag.start).length_sq()>36.0 {
                    if drag.vertical {
                        let delta=super::drag_anchor::vertical_delta(
                            ui,rect,model,view.cam,
                            drag.start,pos,drag.anchor,
                            view.world_mirrored
                        );

                        let next=(drag.height_start+delta)
                            .clamp(-8192.0,8192.0);

                        let previous=self.r630c_warp_y
                            .get(&drag.uid)
                            .copied().unwrap_or(0.0);

                        if (next-previous).abs()>0.01 {
                            self.r630c_warp_y.insert(drag.uid,next);
                            hit=true;
                        }
                    }else{
                        let delta=super::drag_anchor::horizontal_delta(
                            ui,rect,model,view.cam,
                            drag.start,pos,drag.anchor,
                            view.world_mirrored
                        );

                        if let Some(current)=self.records.iter()
                            .find(|d|d.uid==drag.uid)
                        {
                            let max_x=(128-current.size[0] as i32).max(0);
                            let max_z=(128-current.size[1] as i32).max(0);

                            let next=[
                                (drag.source[0] as i32
                                    +(delta[0]/128.0).round() as i32)
                                    .clamp(0,max_x) as u8,
                                (drag.source[1] as i32
                                    +(delta[2]/128.0).round() as i32)
                                    .clamp(0,max_z) as u8
                            ];

                            if next!=current.source {
                                if !drag.changed {
                                    self.remember();
                                }

                                if let Some(d)=self.records.iter_mut()
                                    .find(|d|d.uid==drag.uid)
                                {
                                    d.source=next;
                                }

                                if let Some(active)=self.r604_marker_drag.as_mut(){
                                    active.changed=true;
                                }

                                hit=true;
                            }
                        }
                    }
                }
            }
        }

        // Auxiliary MAP trigger Ctrl + LMB movement.
        if let Some(drag)=self.r630c_aux_drag.clone(){
            if !ui.input(|i|i.pointer.primary_down()){
                self.r630c_aux_drag=None;
            }else if let Some(pos)=pointer {
                if (pos-drag.start).length_sq()>36.0 {
                    let delta=super::drag_anchor::vertical_delta(
                        ui,rect,model,view.cam,
                        drag.start,pos,drag.anchor,
                        view.world_mirrored
                    );

                    let next=(drag.height_start+delta)
                        .clamp(-8192.0,8192.0);

                    if self.r630c_aux_y.len()<self.other_triggers.len(){
                        self.r630c_aux_y.resize(
                            self.other_triggers.len(),0.0
                        );
                    }

                    if let Some(value)=self.r630c_aux_y.get_mut(drag.index){
                        if (next-*value).abs()>0.01 {
                            *value=next;
                            hit=true;
                        }
                    }
                }
            }
        }
        if self.r604_marker_focus
            && !ui.ctx().wants_keyboard_input()
            && ui.input_mut(|i|i.consume_key(
                egui::Modifiers::NONE,egui::Key::Delete
            ))
        {
            self.delete_selected();
            self.r604_marker_drag=None;
            self.r604_marker_focus=false;
            hit=true;
        }

        self.native_marker_context_ui(ui.ctx(),index,ground);
        self.v2082_aux_properties_ui(ui.ctx(),ground);
        hit || self.r604_marker_drag.is_some() || self.r630c_aux_drag.is_some()
    }

    fn collect_man_edits(
        &self
    )->Result<Vec<man_edit::DestEdit>> {
        let mut intended=
            BTreeMap::<usize,Destination>::new();

        for d in &self.records {
            if let Kind::Script{
                op_pc:Some(pc),
                destination:Some(dest),
                ..
            }=&d.kind {
                if let Some(existing)=intended.insert(
                    *pc,
                    dest.clone()
                ){
                    ensure!(
                        existing==*dest,
                        "MAN +0x{pc:X} is SHARED by doors with conflicting destination edits; use Apply destination to all shared-script doors"
                    );
                }
            }
        }

        let originals=self.original.iter()
            .filter_map(|d|
                match &d.kind {
                    Kind::Script{
                        op_pc:Some(pc),
                        destination:Some(dest),
                        ..
                    }=>Some((*pc,dest)),
                    _=>None,
                }
            )
            .collect::<BTreeMap<_,_>>();

        let mut edits=Vec::new();

        for (pc,dest) in intended {
            if originals.get(&pc)
                .is_some_and(|d|**d==dest) {
                continue;
            }

            ensure!(
                originals.contains_key(&pc),
                "New MAN source opcode +0x{pc:X} has no original verification"
            );

            ensure!(
                dest.name.bytes().all(|b|
                    b.is_ascii_lowercase() ||
                    b.is_ascii_digit() ||
                    b==b'_'
                ) &&
                dest.name.len()<=255 &&
                !dest.name.is_empty(),
                "Destination '{}' is not a valid CDNAME field-scene label",
                dest.name
            );

            edits.push(man_edit::DestEdit{
                op_pc:pc,
                index:dest.index,
                name:dest.name.into_bytes(),
                entry_x:dest.entry_x,
                entry_z:dest.entry_z,
                dir:dest.facing,
            });
        }

        Ok(edits)
    }

    fn build_kind(
        &self,
        block:bool,
        kind:u8,
        old:&[[u8;4]]
    )->Result<Vec<[u8;4]>> {
        let mut by_row=
            BTreeMap::<usize,&Door>::new();

        for d in &self.records {
            let indexed=match &d.origin {
                Origin::Local{fallback,row}
                    if kind==0 && *fallback==block=>
                        Some(*row),

                Origin::Walk{fallback,row}
                    if kind==1 && *fallback==block=>
                        Some(*row),

                _=>None
            };

            if let Some(i)=indexed {
                ensure!(
                    by_row.insert(i,d).is_none(),
                    "Duplicate native trigger identity"
                );
            }
        }

        let mut out=Vec::new();

        for (i,&row) in old.iter().enumerate() {
            if let Some(d)=by_row.get(&i) {
                out.extend(d.geometry()?);
            }
            else {
                out.push(row);
            }
        }

        for d in &self.records {
            if !matches!(&d.origin,Origin::New) {
                continue;
            }

            if (kind==0 &&
                matches!(&d.kind,Kind::Local{..})) ||
               (kind==1 &&
                matches!(&d.kind,Kind::Script{..})) {

                if !block {
                    out.extend(d.geometry()?);
                }
            }
        }

        ensure!(
            out.len()<i16::MAX as usize,
            "Native trigger count overflow"
        );

        Ok(out)
    }

    fn compile(
        &self,
        image:Vec<u8>,
        index:&ProtIndex
    )->Result<(Vec<u8>,String)> {
        ensure!(
            !self.scene.is_empty(),
            "No active scene"
        );

        ensure!(
            self.has_native_changes(),
            "No native door/teleport edits to serialize; names are editor metadata only"
        );

        let native=Scene::load(index,&self.scene)?;

        let map_entry=native.field_map_index(index)
            .context("Active scene has no native MAP")?
            as usize;

        let mut disc=DiscPatcher::open(image)?;
        let mut map=disc.read_entry_footprint(map_entry)?;

        ensure!(
            map.len()>=0x12000,
            "Native MAP is truncated"
        );

        // LCK_R626_TRIGGER_IDENTITY
        // Check the retail baseline fingerprint, then compare
        // the real trigger rows against the staged MAP. Region
        // table relocation alone must not block warp editing.
        let retail_map=index.entry_bytes_extended(
            map_entry as u32
        )?;

        ensure!(
            retail_map.len()>=0x12000,
            "R626: Original native MAP unavailable"
        );

        ensure!(
            collision_recipes::digest(
                &retail_map[0x10000..0x12000]
            )==self.map_fingerprint,
            "R626: Editor warp records belong to a different retail MAP revision"
        );

        r626_verify_trigger_identity(
            &retail_map,
            &map
        )?;

        let lba=disc.entry_disc_lba(map_entry)
            .context("Missing MAP LBA")?;

        let loaded=disc.read_disc_sectors(lba,0x28)?;

        ensure!(
            loaded.len()==0x14000,
            "MAP 0x28-sector runtime window incomplete"
        );

        let fallback=loaded[0x12000..0x14000].to_vec();

        ensure!(
            collision_recipes::digest(&fallback)==
                self.fallback_fingerprint,
            "Native fallback MAP trigger window changed since door editing"
        );

        let sibling=disc.read_entry_footprint(map_entry+1)?;

        // Primary-only edits must not require the fallback to fit
        // entirely in one adjacent PROT entry. Fallback writes
        // remain gated on a provably complete native footprint.
        let fallback_single_owner=
            sibling.len()>=0x2000 &&
            sibling[..0x2000]==fallback;

        let original_fallback=fallback.clone();

        let mut blocks=[
            map[0x10000..0x12000].to_vec(),
            fallback
        ];

        let mut expected=[
            [
                Vec::<[u8;4]>::new(),
                Vec::<[u8;4]>::new()
            ],
            [
                Vec::<[u8;4]>::new(),
                Vec::<[u8;4]>::new()
            ],
        ];

        let mut changed=0usize;

        for block in 0..2 {
            for kind in 0..2usize {
                let original_rows=table(
                    &blocks[block],
                    kind
                )?
                .into_iter()
                .map(|v|
                    <[u8;4]>::try_from(v.as_slice()).unwrap()
                )
                .collect::<Vec<_>>();

                let mut edited_rows=original_rows.clone();

                if kind==1 {
                    self.v2082_apply_aux_rows(
                        &mut edited_rows,
                        block==1,
                        1
                    )?;
                }

                let next=self.build_kind(
                    block==1,
                    kind as u8,
                    &edited_rows
                )?;

                let old=original_rows;

                expected[block][kind]=next.clone();

                if next!=old {
                    rewrite_table(
                        &mut blocks[block],
                        kind,
                        &next
                    )?;
                    changed+=1;
                }
            }
        }

        // V2.0.8.2F: serialize existing native kind-2
        // rows without changing their identities,
        // counts or table addresses.
        for bank in 0..2 {
            let old=lck_r605_kind2(&blocks[bank]);
            let mut next=old.clone();

            self.v2082_apply_aux_rows(
                &mut next,
                bank==1,
                2
            )?;

            if next!=old {
                let begin=i16::from_le_bytes([
                    blocks[bank][10],
                    blocks[bank][11]
                ]);

                ensure!(
                    begin>=14,
                    "Invalid kind-2 MAP table offset"
                );

                for (row,data) in next.iter().enumerate(){
                    let at=begin as usize+row*4;

                    ensure!(
                        at+4<=blocks[bank].len(),
                        "Native kind-2 MAP row out of bounds"
                    );

                    blocks[bank][at..at+4]
                        .copy_from_slice(data);
                }

                ensure!(
                    lck_r605_kind2(&blocks[bank])==next,
                    "Native kind-2 MAP readback differs"
                );

                changed+=1;
            }
        }
        let old_primary_0=
            table(&map[0x10000..0x12000],0)?;
        let old_primary_1=
            table(&map[0x10000..0x12000],1)?;

        let old_fallback_0=
            table(&original_fallback,0)?;
        let old_fallback_1=
            table(&original_fallback,1)?;

        let mut affected=BTreeSet::new();

        for rows in [
            old_primary_0,
            old_primary_1,
            old_fallback_0,
            old_fallback_1
        ] {
            for r in rows {
                affected.insert((r[0],r[1]));
            }
        }

        for kind in 0..2usize {
            for block in 0..2 {
                for row in &expected[block][kind] {
                    affected.insert((row[0],row[1]));
                }
            }
        }

        let mut stamps=
            BTreeMap::<(u8,u8),u16>::new();

        for kind in 0..2usize {
            for block in 0..2 {
                for row in &expected[block][kind] {
                    *stamps.entry((row[0],row[1]))
                        .or_default() |=0x200u16<<kind;
                }
            }
        }

        for (x,z) in affected {
            ensure!(
                x<128 && z<128,
                "Native trigger source outside 128x128 MAP"
            );

            let at=0x8000+
                (z as usize*128+x as usize)*2;

            let word=u16::from_le_bytes([
                map[at],
                map[at+1]
            ]);

            let bits=*stamps.get(&(x,z))
                .unwrap_or(&0);

            map[at..at+2].copy_from_slice(
                &((word & !0x0600)|bits).to_le_bytes()
            );
        }

        map[0x10000..0x12000]
            .copy_from_slice(&blocks[0]);

        disc.patch_prot_entry(
            map_entry,
            0,
            &map
        )?;

        if blocks[1]!=original_fallback {
            ensure!(
                fallback_single_owner,
                "Fallback door edit spans multiple PROT entries; physical fallback writer is not yet implemented. Original IMG remains untouched."
            );

            disc.patch_prot_entry(
                map_entry+1,
                0,
                &blocks[1]
            )?;
        }

        ensure!(
            disc.read_entry_footprint(map_entry)?==map,
            "Native MAP door readback failed"
        );

        if blocks[1]!=original_fallback {
            ensure!(
                disc.read_entry_footprint(map_entry+1)?
                    [..0x2000]==blocks[1],
                "Fallback MAP door readback failed"
            );
        }

        let edits=self.collect_man_edits()?;
        let new_cross=self.records.iter().filter_map(|door|{
            if !matches!(&door.origin,Origin::New) {return None;}
            match &door.kind {
                Kind::Script{slot:Some(slot),op_pc:None,destination:Some(dest)}
                    =>Some(man_edit::IndependentWarp{
                        slot:*slot,name:dest.name.clone(),index:dest.index,
                        entry_x:dest.entry_x,entry_z:dest.entry_z,dir:dest.facing,
                    }),
                _=>None,
            }
        }).collect::<Vec<_>>();
        ensure!(self.native_p2_records+new_cross.len()<=256,
            "Independent warp count exceeds native P2 u8 script IDs");
        for (i,warp) in new_cross.iter().enumerate() {
            ensure!(warp.slot as usize==self.native_p2_records+i,
                "New cross-cell P2 slots are inconsistent");
            ensure!(index.cdname_scene_names().iter().any(|s|s==&warp.name),
                "Unknown cross-cell destination CDNAME '{}'",warp.name);
            ensure!(warp.dir<=7,"Cross-cell arrival facing must be 0..7");
        }

        for edit in &edits {
            let name=std::str::from_utf8(
                &edit.name
            ).context(
                "Destination CDNAME is not UTF-8"
            )?;

            ensure!(
                index.cdname_scene_names()
                    .iter().any(|existing|existing==name),
                "Door points to unknown scene '{name}'; choose a real CDNAME destination"
            );

            ensure!(
                edit.dir<=7,
                "Arrival facing must be an 8-sector native direction"
            );
        }

        if !edits.is_empty() || !new_cross.is_empty() {
            let (
                idx,
                offset,
                budget,
                slot,
                decoded
            )=ps1_export::find_native_man(
                &disc,
                map_entry
            )?;

            ensure!(
                collision_recipes::digest(&decoded)==
                    self.man_fingerprint,
                "Existing MAN changed since the editor loaded; cannot safely reuse original op offsets"
            );

            let amended=if edits.is_empty(){decoded.clone()}else{
                man_edit::apply_dest_edits(&decoded,&edits)
                    .map_err(|e|anyhow::anyhow!("MAN door rewrite: {e}"))?
            };
            let rewritten=man_edit::append_independent_warps(
                &amended,&new_cross
            ).map_err(|e|anyhow::anyhow!("MAN new cross-cell warp: {e}"))?;

            let compressed=compress_within(
                &rewritten,
                budget
            ).context(
                "Door MAN no longer fits native scene bundle compressed footprint"
            )?;

            disc.patch_prot_entry(
                idx,
                offset as u64,
                &compressed
            )?;

            if rewritten.len()!=decoded.len() {
                let size=
                    scene_asset_table::encode_size_word(
                        3,
                        rewritten.len() as u32
                    );

                disc.patch_prot_entry(
                    idx,
                    scene_asset_table::
                        SceneAssetTable::size_word_offset(slot)
                        as u64,
                    &size.to_le_bytes()
                )?;

                let entry=disc.read_entry_footprint(idx)?;

                let total=u32::from_le_bytes(
                    entry[4..8].try_into()?
                );

                let updated=total
                    .checked_sub(decoded.len() as u32)
                    .and_then(|v|
                        v.checked_add(rewritten.len() as u32)
                    )
                    .context(
                        "MAN bundle decoded-size overflow"
                    )?;

                disc.patch_prot_entry(
                    idx,
                    4,
                    &updated.to_le_bytes()
                )?;
            }

            let check=disc.read_entry_footprint(idx)?;

            let (verify,_)=legaia_lzs::decompress_tracked(
                &check[offset..],
                rewritten.len()
            )?;

            ensure!(
                verify==rewritten,
                "MAN compressed native sector readback mismatch"
            );
        }

        let out=disc.into_image();

        let _=DiscPatcher::open(
            out.clone()
        ).context(
            "Native IMG structural reparse"
        )?;

        Ok((
            out,
            format!(
                "Native doors: {} MAP trigger tables updated, {} MAN scene-change edits; IMG structure verified; gameplay test required",
                changed,
                edits.len()+new_cross.len()
            )
        ))
    }

    pub fn export_new(
        &self,
        source:&Path,
        output:&Path,
        index:&ProtIndex
    )->Result<String> {
        ps1_original_guard::ensure_new_output(
            source,
            output
        )?;

        ensure!(
            !output.exists(),
            "Output IMG already exists; select a new filename"
        );

        let original=fs::read(source)
            .context("Read native source IMG")?;

        let (bytes,msg)=self.compile(
            original.clone(),
            index
        )?;

        ensure!(
            bytes.len()==original.len(),
            "Native door export changed raw disc byte count"
        );

        let temp=output.with_extension(
            "lckdoorbuilding"
        );

        ensure!(
            !temp.exists(),
            "Door export staging path already exists"
        );

        let result=(||->Result<()> {
            use std::io::Write;

            let mut file=fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&temp)?;

            file.write_all(&bytes)?;
            file.sync_all()?;
            drop(file);

            let verify=fs::read(&temp)?;

            ensure!(
                verify==bytes,
                "Door staging IMG failed complete byte-for-byte readback"
            );

            fs::rename(&temp,output)?;
            Ok(())
        })();

        if result.is_err() {
            let _=fs::remove_file(&temp);
        }

        result?;

        Ok(format!(
            "{} -> {}",
            msg,
            output.display()
        ))
    }

    pub fn overlay_existing(
        &self,
        target:&Path,
        index:&ProtIndex
    )->Result<String> {
        if !self.has_native_changes() {
            return Ok("No native door changes".into());
        }

        let original=fs::read(target)
            .context(
                "Read generated IMG for door overlay"
            )?;

        let (bytes,msg)=self.compile(
            original.clone(),
            index
        )?;

        ensure!(
            bytes.len()==original.len(),
            "Door overlay altered image size"
        );

        let backup=target.with_extension(
            "lckdoorbackup"
        );

        ensure!(
            !backup.exists(),
            "Door IMG recovery backup already exists"
        );

        fs::write(&backup,&original)?;

        let result=(||->Result<()> {
            use std::io::Write;

            let mut file=fs::OpenOptions::new()
                .write(true)
                .truncate(true)
                .open(target)?;

            file.write_all(&bytes)?;
            file.sync_all()?;
            drop(file);

            ensure!(
                fs::read(target)?==bytes,
                "Door overlay IMG readback mismatch"
            );

            Ok(())
        })();

        match result {
            Ok(())=>{
                let _=fs::remove_file(backup);
                Ok(msg)
            },
            Err(error)=>{
                let _=fs::copy(&backup,target);
                Err(error.context(format!(
                    "Original generated IMG preserved at {}",
                    backup.display()
                )))
            }
        }
    }
}

pub(super) fn strip(text:&str)->String {
    text.lines()
        .filter(|line|
            !line.starts_with("DOOR_EDIT\t")
        )
        .map(|line|format!("{line}\n"))
        .collect()
}

fn table(
    block:&[u8],
    kind:usize
)->Result<Vec<Vec<u8>>> {
    ensure!(
        kind<=3 && block.len()>=0x2000,
        "Native trigger block invalid"
    );

    let off=i16::from_le_bytes([
        block[kind*4+2],
        block[kind*4+3]
    ]);

    let count=i16::from_le_bytes([
        block[kind*4+4],
        block[kind*4+5]
    ]);

    if count<=0 {
        return Ok(Vec::new());
    }

    let len=if kind==3 {8}else{4};

    ensure!(
        off>=18,
        "Invalid kind-{kind} trigger offset {off}"
    );

    let start=off as usize;
    let end=start+count as usize*len;

    ensure!(
        end<=block.len(),
        "Kind-{kind} trigger table outside MAP window"
    );

    Ok(block[start..end]
        .chunks_exact(len)
        .map(|v|v.to_vec())
        .collect())
}

// LCK_R633_TRIGGER_REGION_ISOLATION
//
// Offset is relative to the native 0x2000-byte MAP trigger block.
// Reserve 0x000..0x7FF for existing tables and region expansion.
// New trigger arrays use checked space at 0x800 or above.
//
// Do not select a span occupied by an active table even when
// its current bytes happen to be zero.
//
// Never overwrite nonzero bytes and never exceed the block.
fn r633_free_trigger_slot(
    block:&[u8],
    size:usize
)->Result<usize>{
    const RESERVED:usize=0x800;

    ensure!(
        block.len()==0x2000,
        "R633: expected exact native MAP trigger block"
    );

    ensure!(
        size>0 && size%4==0
            && size<=block.len()-RESERVED,
        "R633: requested trigger table does not fit available native MAP space"
    );

    let mut occupied=Vec::<(usize,usize)>::new();

    for kind in 0..4usize {
        let at=kind*4+2;

        let offset=i16::from_le_bytes([
            block[at],block[at+1]
        ]);

        let count=i16::from_le_bytes([
            block[at+2],block[at+3]
        ]);

        ensure!(
            count>=0,
            "R633: native trigger count is negative"
        );

        if count==0 {
            continue;
        }

        ensure!(
            offset>=18,
            "R633: native trigger table has invalid offset"
        );

        let stride=if kind==3 {8usize}else{4usize};

        let length=(count as usize)
            .checked_mul(stride)
            .context("R633: trigger table length overflow")?;

        let end=(offset as usize)
            .checked_add(length)
            .context("R633: trigger table boundary overflow")?;

        ensure!(
            end<=block.len(),
            "R633: referenced trigger table exceeds MAP block"
        );

        occupied.push((offset as usize,end));
    }

    let limit=block.len()-size;

    for at in (RESERVED..=limit).step_by(4) {
        let end=at+size;

        if occupied.iter().any(|&(first,last)|
            at<last && end>first
        ){
            continue;
        }

        if block[at..end].iter().all(|&byte|byte==0) {
            return Ok(at);
        }
    }

    anyhow::bail!(
        "R633: no free nonoverlapping native trigger storage above 0x800; preserved region and trigger records"
    )
}
fn rewrite_table(
    block:&mut [u8],
    kind:usize,
    records:&[[u8;4]]
)->Result<()> {
    ensure!(
        kind<2 && block.len()==0x2000,
        "Unsupported native trigger family"
    );

    let at=kind*4+2;

    let old_off=i16::from_le_bytes([
        block[at],
        block[at+1]
    ]);

    let old_count=i16::from_le_bytes([
        block[at+2],
        block[at+3]
    ]).max(0) as usize;

    let size=records.len()*4;

    ensure!(
        records.len()<i16::MAX as usize,
        "Native trigger record limit exceeded"
    );

    if records.is_empty() {
        block[at+2..at+4]
            .copy_from_slice(&0i16.to_le_bytes());

        return Ok(());
    }

    let begin=if old_off>=18 &&
        old_count>=records.len() {
        old_off as usize
    }
    else {
        // R633: new trigger rows may not consume future region slots.
        r633_free_trigger_slot(block,size)?
    };

    ensure!(
        begin+size<=block.len(),
        "Trigger table write overflows native MAP window"
    );

    for (i,row) in records.iter().enumerate() {
        block[begin+i*4..begin+i*4+4]
            .copy_from_slice(row);
    }

    block[at..at+2]
        .copy_from_slice(&(begin as i16).to_le_bytes());

    block[at+2..at+4]
        .copy_from_slice(
            &(records.len() as i16).to_le_bytes()
        );

    ensure!(
        table(block,kind)?
            .iter()
            .zip(records)
            .all(|(a,b)|
                a.as_slice()==b.as_slice()
            ),
        "Native MAP trigger reparse disagreed"
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appends_new_trigger_without_mutating_region_rows()
        ->Result<()> {
        let mut block=vec![0u8;0x2000];

        block[2..4]
            .copy_from_slice(&18i16.to_le_bytes());
        block[4..6]
            .copy_from_slice(&1i16.to_le_bytes());
        block[18..22]
            .copy_from_slice(&[2,3,10,11]);
        block[14..16]
            .copy_from_slice(&26i16.to_le_bytes());
        block[16..18]
            .copy_from_slice(&1i16.to_le_bytes());
        block[26..34]
            .copy_from_slice(&[3,3,4,4,1,0,0,0]);

        let original_region=block[26..34].to_vec();

        rewrite_table(
            &mut block,
            0,
            &[
                [2,3,10,11],
                [8,9,20,22]
            ]
        )?;

        assert_eq!(
            table(&block,0)?,
            vec![
                vec![2,3,10,11],
                vec![8,9,20,22]
            ]
        );

        assert_eq!(
            block[26..34],
            original_region
        );

        Ok(())
    }

    #[test]
    fn standalone_script_not_encoded_as_fake_trigger() {
        let d=Door{
            uid:1,
            label:String::new(),
            origin:Origin::ScriptOnly{op_pc:16},
            source:[0,0],
            size:[1,1],
            enabled:true,
            kind:Kind::Script{
                slot:None,
                op_pc:Some(16),
                destination:None
            }
        };

        assert!(d.geometry().unwrap().is_empty());
    }

    #[test]
    fn multi_tile_local_is_an_exact_grid_of_triggers()
        ->Result<()> {
        let d=Door{
            uid:5,
            label:"door".into(),
            origin:Origin::New,
            source:[4,3],
            size:[2,2],
            enabled:true,
            kind:Kind::Local{
                half_x:8,
                half_z:9
            }
        };

        assert_eq!(
            d.geometry()?,
            vec![
                [4,3,8,9],
                [5,3,8,9],
                [4,4,8,9],
                [5,4,8,9]
            ]
        );

        Ok(())
    }
}

#[cfg(test)]
mod lck_r4_flag_tests {
    use super::*;
    #[test]
    fn flag_references_are_deduplicated_without_opcodes() {
        let mut state=State::default();
        let mut a=ScriptSummary::default();
        a.flags.push("Global flag Read bit 3, MAN +0x20".into());
        a.flags.push("Global flag Read bit 3, MAN +0x44".into());
        a.flags.push("Local flag Set bit 7, MAN +0x51".into());
        state.inspector_scripts.insert(4,a);
        let got=state.story_flag_references();
        assert_eq!(got.len(),2);
        assert!(got.contains(&"Global flag Read bit 3".to_string()));
        assert!(got.contains(&"Local flag Set bit 7".to_string()));
    }
}

#[cfg(test)]
mod lck_r4_2_cell_flag_tests {
    use super::*;

    #[test]
    fn only_active_cell_trigger_scripts_supply_toolbar_flags() {
        let mut state=State::default();
        state.scene="town01".into();

        let mut linked=ScriptSummary::default();
        linked.flags.push(
            "Global flag Read bit 3, MAN +0x20".into()
        );
        linked.flags.push(
            "Global flag Read bit 3, MAN +0x44".into()
        );

        let mut unrelated=ScriptSummary::default();
        unrelated.flags.push(
            "Global flag Read bit 15, MAN +0x99".into()
        );

        state.inspector_scripts.insert(4,linked);
        state.inspector_scripts.insert(5,unrelated);

        state.records.push(Door {
            uid:12,
            label:"Native test trigger".into(),
            origin:Origin::Walk {
                fallback:false,
                row:2
            },
            source:[3,4],
            size:[1,1],
            enabled:true,
            kind:Kind::Script {
                slot:Some(4),
                op_pc:None,
                destination:None
            }
        });

        let flags=state.story_flag_references_for_cell("town01");

        assert_eq!(
            flags,
            vec!["Global flag Read bit 3".to_string()]
        );

        assert!(
            state.story_flag_references_for_cell("town02")
                .is_empty()
        );

        // The comprehensive read-only MAN source remains
        // available independently of the toolbar filter.
        assert_eq!(state.story_flag_references().len(),2);
    }
}

// ============================================================
// LCK_R58_WORLD_MARKERS
// Editor-only native door/warp display geometry.
// Same world-coordinate raster pipeline as the R4.9 fixed grid.
// Does not alter MAP, MAN, player position or exported IMG.
// ============================================================

// LCK_V2081_SCREEN_MARKERS
//
// Render native MAP/MAN markers above the scene image.
// Original marker coordinates and source records remain
// unchanged. No gameplay or IMG bytes are edited.

fn lck_v2081_diamond(
    center:egui::Pos2,
    radius:f32,
)->[egui::Pos2;4] {
    [
        egui::pos2(center.x-radius,center.y),
        egui::pos2(center.x,center.y-radius),
        egui::pos2(center.x+radius,center.y),
        egui::pos2(center.x,center.y+radius),
    ]
}

fn lck_v2081_draw(
    painter:&egui::Painter,
    center:egui::Pos2,
    radius:f32,
    color:egui::Color32,
) {
    let points=lck_v2081_diamond(center,radius);
    let stroke=egui::Stroke::new(2.0,color);

    for i in 0..4 {
        painter.line_segment(
            [points[i],points[(i+1)%4]],
            stroke
        );
    }

    painter.line_segment(
        [
            egui::pos2(center.x-radius*0.6,center.y),
            egui::pos2(center.x+radius*0.6,center.y)
        ],
        stroke
    );

    painter.line_segment(
        [
            egui::pos2(center.x,center.y-radius*0.6),
            egui::pos2(center.x,center.y+radius*0.6)
        ],
        stroke
    );

    painter.circle_filled(center,2.5,color);
}

// LCK_R623E_ROUND_WARPS
// Round marker matching the original local-warp appearance.
// Native records still provide the actual source coordinates.
fn lck_r623e_draw_round(
    painter:&egui::Painter,
    center:egui::Pos2,
    radius:f32,
    color:egui::Color32
){
    painter.circle_filled(
        center,
        radius,
        color
    );

    painter.circle_stroke(
        center,
        radius,
        egui::Stroke::new(
            1.5,
            egui::Color32::from_rgb(25,30,35)
        )
    );

    painter.circle_filled(
        center,
        2.0,
        egui::Color32::WHITE
    );
}
impl State {
    pub(super) fn lck_v2081_counts(&self)->(usize,usize){
        let doors=self.records.iter()
            .filter(|d|
                d.enabled &&
                !matches!(&d.origin,Origin::ScriptOnly{..})
            )
            .count();

        (doors,self.other_triggers.len())
    }

    pub(super) fn lck_v2081_paint(
        &self,
        ui:&egui::Ui,
        view:&super::viewport::Viewport,
        model:&crate::raster::Model,
        ground:Option<
            &legaia_scene_formats::field_objects::WalkHeightfield
        >,
    ){
        if !self.show_markers {
            return;
        }

        let Some((rect,_))=view.world_image else {
            return;
        };

        let painter=ui.painter_at(rect);

        let marker=|
            x:f32,
            z:f32,
            radius:f32,
            color:egui::Color32,
            round:bool,
            height:f32
        |{
            let y=native_door_floor_y(ground,x,z)
                +LCK_R58_GIZMO_LIFT+height;

            if let Some(pos)=view.project_world_marker(
                model,[x,y,z]
            ){
                if rect.contains(pos){
                    if round {
                        lck_r623e_draw_round(
                            &painter,pos,radius,color
                        );
                    }else{
                        lck_v2081_draw(
                            &painter,pos,radius,color
                        );
                    }
                }
            }
        };

        // Native local doors and scene transitions.
        for d in &self.records {
            if !self.v2082_door_visible(d){
                continue;
            }

            let x=(
                d.source[0] as f32+
                d.size[0] as f32*0.5
            )*128.0;

            let z=(
                d.source[1] as f32+
                d.size[1] as f32*0.5
            )*128.0;

            let color=if self.selected==Some(d.uid){
                egui::Color32::from_rgb(255,220,75)
            }else if matches!(&d.kind,Kind::Local{..}){
                egui::Color32::from_rgb(95,238,135)
            }else{
                egui::Color32::from_rgb(110,205,255)
            };

            marker(x,z,10.0,color,true,self.r630c_warp_y.get(&d.uid).copied().unwrap_or(0.0));

            if let Kind::Local{half_x,half_z}=&d.kind{
                let dx=*half_x as f32*64.0+64.0;
                let dz=(*half_z as f32+1.0)*64.0;

                marker(
                    dx,dz,8.0,
                    if self.selected==Some(d.uid){
                        color
                    }else{
                        egui::Color32::from_rgb(65,235,125)
                    },
                    true
                ,0.0);
            }
        }

        // MAP auxiliary triggers.
        // Gate-0 object bindings use actual contacts.
        for (index,trigger) in self.other_triggers.iter().enumerate(){
            if !self.v2082_aux_visible(trigger){
                continue;
            }
            let color=if trigger.kind==2 {
                egui::Color32::from_rgb(255,168,65)
            }else{
                egui::Color32::from_rgb(211,143,255)
            };

            for (x,z) in self.v2082_aux_points(index,trigger){
                marker(x,z,7.0,color,false,self.r630c_aux_y.get(index).copied().unwrap_or(0.0));
            }
        }
    }
}

#[cfg(test)]
mod lck_v2081_marker_tests {
    use super::*;

    #[test]
    fn marker_geometry_is_centered(){
        let p=lck_v2081_diamond(
            egui::pos2(100.0,80.0),10.0
        );

        assert_eq!(p[0],egui::pos2(90.0,80.0));
        assert_eq!(p[1],egui::pos2(100.0,70.0));
        assert_eq!(p[2],egui::pos2(110.0,80.0));
        assert_eq!(p[3],egui::pos2(100.0,90.0));
    }

    #[test]
    fn empty_marker_inventory_is_safe(){
        let state=State::default();
        assert_eq!(state.lck_v2081_counts(),(0,0));
    }
}
// LCK_V2082F_TRIGGER_FILTERS

const V2082_LOCAL:u8=1;
const V2082_SCRIPT:u8=2;
const V2082_BIND:u8=4;
const V2082_KIND2:u8=8;
const V2082_OTHER:u8=16;
const V2082_ALL:u8=31;

// LCK_R623D_UNIFIED_WARPS
const V2082_WARPS:u8=V2082_LOCAL|V2082_SCRIPT;

// LCK_R623E_ROUND_WARPS
// When no marker categories are displayed, a new selection
// starts from zero rather than the default ALL mask.
fn lck_r623e_toggle_mask(
    existing:u8,
    visible:bool,
    bits:u8,
    enabled:bool
)->u8 {
    let active=if visible {existing}else{0};

    if enabled {
        active|bits
    }else{
        active&!bits
    }
}


#[cfg(test)]
mod r643_local_warp_hotkey_tests {
    use super::*;

    #[test]
    fn toggling_local_warps_preserves_other_categories() {
        let mut state=State::default();
        state.show_markers=true;
        state.trigger_mask=Some(V2082_ALL);

        assert!(state.r643_local_category_enabled());

        assert!(!state.r643_toggle_local_warps());
        assert_eq!(
            state.trigger_mask,
            Some(V2082_ALL & !V2082_LOCAL)
        );
        assert!(state.show_markers);

        assert!(state.r643_toggle_local_warps());
        assert_eq!(state.trigger_mask,Some(V2082_ALL));
    }

    #[test]
    fn hidden_categories_can_enable_local_warps() {
        let mut state=State::default();
        state.show_markers=false;
        state.trigger_mask=Some(0);

        assert!(!state.r643_local_category_enabled());

        assert!(state.r643_toggle_local_warps());
        assert_eq!(state.trigger_mask,Some(V2082_LOCAL));
        assert!(state.show_markers);

        assert!(!state.r643_toggle_local_warps());
        assert_eq!(state.trigger_mask,Some(0));
        assert!(!state.show_markers);
    }
}
// LCK_V2083_ORIGINAL_WARP_CATEGORY
impl State {
    // R643B: toggle only the Local Warps category setting.
    // Runtime marker visibility can have additional conditions.
    fn r643_local_category_enabled(&self)->bool {
        self.show_markers
            && (self.v2082_mask() & V2082_LOCAL)!=0
    }

    pub(super) fn r643_toggle_local_warps(&mut self)->bool {
        let previously_enabled=self.r643_local_category_enabled();

        let mut mask=if self.show_markers {
            self.v2082_mask()
        }else{
            0
        };

        if previously_enabled {
            mask &= !V2082_LOCAL;
        }else{
            mask |= V2082_LOCAL;
        }

        self.trigger_mask=Some(mask);
        self.show_markers=mask!=0;

        self.r643_local_category_enabled()
    }
    pub(super) fn v2083_local_warps_visible(&self)->bool {
        // LCK_R623D_UNIFIED_WARPS
        // The legacy local-warp overlay would double-render markers
        // and intercept clicks intended for unified Warp Properties.
        false
    }

    fn v2082_mask(&self)->u8 {
        self.trigger_mask.unwrap_or(V2082_ALL)
    }

    fn v2082_door_visible(&self,d:&Door)->bool {
        if !self.v2084_story_door_visible(d){return false;}

        if !self.show_markers ||
            !d.enabled ||
            matches!(&d.origin,Origin::ScriptOnly{..})
        {
            return false;
        }

        // LCK_R623D_UNIFIED_WARPS
        // Local MAP kind-0 and scripted MAP kind-1 gate-1
        // warps share the same marker and interaction layer.
        match &d.kind {
            Kind::Local{..}=>{
                self.v2082_mask()&V2082_LOCAL!=0
            },
            Kind::Script{..}=>{
                self.v2082_mask()&V2082_SCRIPT!=0
            },
        }
    }

    fn v2082_aux_bit(t:&R605NativeTrigger)->u8 {
        match (t.kind,t.gate){
            (1,0)=>V2082_BIND,
            (2,_)=>V2082_KIND2,
            _=>V2082_OTHER,
        }
    }

    fn v2082_aux_visible(&self,t:&R605NativeTrigger)->bool {
        self.show_markers &&
            self.v2082_mask()&Self::v2082_aux_bit(t)!=0
    }

    fn v2082_aux_points(
        &self,
        index:usize,
        t:&R605NativeTrigger
    )->Vec<(f32,f32)>{
        let original=self.original_auxiliary
            .get(index);

        if t.kind==1
            && t.gate==0
            && !t.objects.is_empty()
            && original.is_some_and(|old|old.tile==t.tile)
        {
            return t.objects.iter().map(
                |(_,_,contact)|(
                    contact[0] as f32,
                    contact[1] as f32
                )
            ).collect();
        }

        vec![(
            (t.tile[0] as f32+0.5)*128.0,
            (t.tile[1] as f32+0.5)*128.0
        )]
    }

    pub(super) fn v2082_menu(
        &mut self,
        ui:&mut egui::Ui
    )->(bool,bool){
        ui.set_min_width(310.0);

        let previous=(
            self.show_markers,
            self.v2082_mask()
        );

        let mut open_warps=false;
        let mut all=self.show_markers
            && self.v2082_mask()==V2082_ALL;

        if ui.checkbox(&mut all,"All").changed(){
            self.show_markers=all;
            self.trigger_mask=Some(
                if all {V2082_ALL}else{0}
            );
        }

        ui.separator();

                // LCK_R623D_UNIFIED_WARPS
        // One checkbox controls both native warp families.
        let mut show_warps=self.show_markers
            && self.v2082_mask()&V2082_WARPS!=0;

        if ui.checkbox(&mut show_warps,"Warps").changed(){
            let mask=lck_r623e_toggle_mask(
                self.v2082_mask(),
                self.show_markers,
                V2082_WARPS,
                show_warps
            );

            self.trigger_mask=Some(mask);
            self.show_markers=mask!=0;
        }

        ui.small(
            "Local + cross-cell. Drag markers to move; right-click for Properties."
        );

        ui.separator();

        for (bit,name) in [
            (V2082_BIND,"Object Bindings"),
            (V2082_KIND2,"Kind-2 Records"),
            (V2082_OTHER,"Other Trigger Gates"),
        ]{
            let mut checked=self.show_markers
                && self.v2082_mask()&bit!=0;

            ui.horizontal(|ui|{
                if ui.checkbox(&mut checked,name).changed(){
                    let mask=lck_r623e_toggle_mask(
                        self.v2082_mask(),
                        self.show_markers,
                        bit,
                        checked
                    );

                    self.trigger_mask=Some(mask);
                    self.show_markers=mask!=0;
                }

                if ui.small_button("Properties").clicked(){
                    if bit==V2082_LOCAL {
                        open_warps=true;
                    }else if bit==V2082_SCRIPT {
                        let uid=self.records.iter()
                            .find(|d|
                                d.enabled &&
                                matches!(&d.kind,Kind::Script{..})
                            )
                            .map(|d|d.uid);

                        if let Some(uid)=uid {
                            self.selected=Some(uid);
                            self.marker_properties=Some(uid);
                        }
                    }else{
                        self.aux_properties=
                            self.other_triggers.iter()
                            .position(|t|
                                Self::v2082_aux_bit(t)==bit
                            );
                    }
                }
            });
        }

        ui.separator();

        ui.small(
            "Enabled categories determine both marker visibility and right-click selection."
        );

        (
            previous!=(self.show_markers,self.v2082_mask()),
            open_warps
        )
    }

    fn v2082_aux_properties_ui(
        &mut self,
        ctx:&egui::Context,
        ground:Option<&legaia_scene_formats::field_objects::WalkHeightfield>
    ){
        let Some(index)=self.aux_properties
        else{return;};

        let Some(record)=self.other_triggers.get(index).cloned()
        else{
            self.aux_properties=None;
            return;
        };

        let category=Self::v2082_aux_bit(&record);
        let mut open=true;
        let mut selected=index;

        if self.r630c_aux_y.len()<self.other_triggers.len(){
            self.r630c_aux_y.resize(
                self.other_triggers.len(),0.0
            );
        }

        egui::Window::new("Native Trigger Properties")
            .id(egui::Id::new("lck_v2082_aux_properties"))
            .open(&mut open)
            .resizable(true)
            .default_width(440.0)
            .show(ctx,|ui|{
                ui.strong(match category {
                    V2082_BIND=>"Object Script Bindings",
                    V2082_KIND2=>"MAP Kind-2 Records",
                    _=>"Other MAP Trigger Gates",
                });

                ui.small(format!(
                    "Cell: {} | Existing native records",
                    self.scene
                ));

                ui.separator();

                egui::ScrollArea::vertical()
                    .max_height(155.0)
                    .show(ui,|ui|{
                        for (j,t) in self.other_triggers.iter()
                            .enumerate()
                            .filter(|(_,t)|
                                Self::v2082_aux_bit(t)==category
                            )
                        {
                            if ui.selectable_label(
                                selected==j,
                                format!(
                                    "{} row {} | tile ({},{}) | operand {}",
                                    if t.fallback {"Fallback"}else{"Primary"},
                                    t.row,
                                    t.tile[0],
                                    t.tile[1],
                                    t.record
                                )
                            ).clicked(){
                                selected=j;
                            }
                        }
                    });

                ui.separator();

                let marker_position=self.other_triggers
                    .get(selected)
                    .and_then(|trigger|
                        self.v2082_aux_points(
                            selected,trigger
                        ).into_iter().next()
                    );

                let base=marker_position
                    .map(|(x,z)|
                        native_door_floor_y(ground,x,z)
                            +LCK_R58_GIZMO_LIFT
                    )
                    .unwrap_or(LCK_R58_GIZMO_LIFT);

                if let Some(t)=self.other_triggers.get_mut(selected){
                    let height=&mut self.r630c_aux_y[selected];
                    let mut world_y=base+*height;

                    ui.horizontal(|ui|{
                        ui.label("Marker Y (world)");
                        if ui.add(
                            egui::DragValue::new(&mut world_y)
                                .speed(8.0)
                                .range(-16384.0..=16384.0)
                        ).changed(){
                            *height=(world_y-base)
                                .clamp(-8192.0,8192.0);
                        }
                    });

                    ui.horizontal(|ui|{
                        ui.label("Y offset from ground marker");
                        ui.add(
                            egui::DragValue::new(height)
                                .speed(8.0)
                                .range(-8192.0..=8192.0)
                        );
                    });

                    ui.small(
                        "Ctrl + left-drag adjusts display height. Native trigger activation remains X/Z based."
                    );
                    ui.label(format!(
                        "Native kind {} | gate {} | {} bank | row {}",
                        t.kind,t.gate,
                        if t.fallback {"fallback"}else{"primary"},
                        t.row
                    ));

                    ui.horizontal(|ui|{
                        ui.label("Tile X");
                        ui.add(
                            egui::DragValue::new(&mut t.tile[0])
                                .range(0..=127)
                        );
                        ui.label("Tile Z");
                        ui.add(
                            egui::DragValue::new(&mut t.tile[1])
                                .range(0..=127)
                        );
                    });

                    ui.horizontal(|ui|{
                        ui.label("Record operand");
                        ui.add(
                            egui::DragValue::new(&mut t.record)
                                .range(0..=255)
                        );
                    });

                    if t.kind==2 {
                        ui.horizontal(|ui|{
                            ui.label("Raw extra operand");
                            ui.add(
                                egui::DragValue::new(&mut t.extra)
                                    .range(0..=255)
                            );
                        });
                    }

                    ui.separator();

                    ui.small(
                        "Native kind, bank, row and gate are fixed. Existing MAP coordinate and operand bytes are editable."
                    );

                    if t.kind==2 {
                        ui.weak(
                            "Kind-2 gameplay behavior is not established. Byte-level editing does not imply a verified gameplay effect."
                        );
                    }else{
                        ui.weak(
                            "Changing an operand may change its MAN binding. Verify the destination and test gameplay."
                        );
                    }

                    if !t.objects.is_empty(){
                        ui.collapsing(
                            "Object contact evidence",
                            |ui|{
                                for (descriptor,world,contact) in &t.objects {
                                    ui.label(format!(
                                        "Descriptor {descriptor} | object ({},{}) | contact ({},{})",
                                        world[0],world[1],
                                        contact[0],contact[1]
                                    ));
                                }
                            }
                        );
                    }
                }
            });

        self.aux_properties=if open {
            Some(selected)
        }else{
            None
        };
    }

    fn v2082_apply_aux_rows(
        &self,
        rows:&mut [[u8;4]],
        fallback:bool,
        kind:u8
    )->Result<()> {
        for t in self.other_triggers.iter()
            .filter(|t|t.fallback==fallback && t.kind==kind)
        {
            let old=self.original_auxiliary.iter()
                .find(|old|
                    old.kind==t.kind
                    && old.fallback==t.fallback
                    && old.row==t.row
                )
                .context("Original native trigger record missing")?;

            ensure!(
                t.gate==old.gate
                    && t.kind==old.kind
                    && t.tile[0]<128
                    && t.tile[1]<128,
                "Native trigger identity or coordinates invalid"
            );

            let row=rows.get_mut(t.row)
                .context("Native MAP trigger row missing")?;

            let expected=[
                old.tile[0],
                old.tile[1],
                old.record,
                if kind==2 {old.extra}else{old.gate}
            ];

            ensure!(
                *row==expected,
                "Native MAP trigger row differs from the loaded source"
            );

            *row=[
                t.tile[0],
                t.tile[1],
                t.record,
                if kind==2 {t.extra}else{t.gate}
            ];
        }

        Ok(())
    }
}

#[cfg(test)]
mod lck_v2082f_trigger_tests {
    use super::*;

    #[test]
    fn category_filters_are_independent(){
        let mut state=State::default();
        state.show_markers=true;
        state.trigger_mask=Some(V2082_KIND2);

        let t=R605NativeTrigger {
            kind:2,
            gate:255,
            tile:[10,20],
            record:1,
            extra:2,
            fallback:false,
            row:0,
            objects:vec![],
        };

        assert!(state.v2082_aux_visible(&t));

        state.trigger_mask=Some(V2082_BIND);

        assert!(!state.v2082_aux_visible(&t));
    }

    #[test]
    fn native_kind2_row_roundtrips()->Result<()>{
        let before=R605NativeTrigger {
            kind:2,
            gate:255,
            tile:[10,20],
            record:5,
            extra:7,
            fallback:false,
            row:0,
            objects:vec![],
        };

        let mut after=before.clone();
        after.tile=[12,21];
        after.record=9;
        after.extra=11;

        let state=State {
            original_auxiliary:vec![before],
            other_triggers:vec![after],
            ..Default::default()
        };

        let mut rows=[[10,20,5,7]];

        state.v2082_apply_aux_rows(
            &mut rows,false,2
        )?;

        assert_eq!(rows,[[12,21,9,11]]);

        Ok(())
    }
}
const LCK_R58_GIZMO_ID:u32=u32::MAX-1;
const LCK_R58_GIZMO_LIFT:f32=26.0;

// Append untextured world geometry with a non-pickable ID.
fn lck_r58_face(
    model:&mut crate::raster::Model,
    vertices:[[f32;3];3],
    color:[u8;3]
){
    if !vertices.iter().flatten().all(|v|v.is_finite()){
        return;
    }

    let Ok(base)=u32::try_from(model.positions.len()) else {
        return;
    };

    if base.checked_add(2).is_none(){
        return;
    }

    model.positions.extend(vertices);
    model.rest.extend(vertices);
    model.object_ids.extend([LCK_R58_GIZMO_ID;3]);
    model.uvs.extend([[0,0];3]);
    model.cba_tsb.extend([[0,0];3]);
    model.modulation.extend([[128,128,128];3]);
    model.fill.extend([color;3]);
    model.textured.extend([0u8;3]);
    model.indices.extend([base,base+1,base+2]);
}

// 3D ribbon equivalent of the R4.9 fixed-grid ribbon.
// Coordinates are world X/Y/Z, not screen pixels.
fn lck_r58_ribbon(
    model:&mut crate::raster::Model,
    a:[f32;3],
    b:[f32;3],
    width:f32,
    color:[u8;3]
){
    let dx=b[0]-a[0];
    let dz=b[2]-a[2];
    let len=(dx*dx+dz*dz).sqrt();

    if !len.is_finite() || len<0.001 {
        return;
    }

    let nx=-dz/len*width*0.5;
    let nz=dx/len*width*0.5;

    let v0=[a[0]+nx,a[1],a[2]+nz];
    let v1=[a[0]-nx,a[1],a[2]-nz];
    let v2=[b[0]-nx,b[1],b[2]-nz];
    let v3=[b[0]+nx,b[1],b[2]+nz];

    lck_r58_face(model,[v0,v1,v2],color);
    lck_r58_face(model,[v0,v2,v3],color);
}

// A real 3D marker: diamond ring, cross and raised apex.
// It has no independent 2D painter position.
fn lck_r58_glyph(
    model:&mut crate::raster::Model,
    center:[f32;3],
    radius:f32,
    color:[u8;3]
){
    if !center.iter().all(|v|v.is_finite()){
        return;
    }

    let [x,y,z]=center;

    let corners=[
        [x-radius,y,z],
        [x,y,z-radius],
        [x+radius,y,z],
        [x,y,z+radius]
    ];

    for i in 0..4 {
        lck_r58_ribbon(
            model,corners[i],corners[(i+1)%4],
            4.0,color
        );
    }

    lck_r58_ribbon(
        model,
        [x-radius*0.55,y,z],
        [x+radius*0.55,y,z],
        3.0,color
    );

    lck_r58_ribbon(
        model,
        [x,y,z-radius*0.55],
        [x,y,z+radius*0.55],
        3.0,color
    );

    // Raised 3D geometry remains visible from tilted cameras.
    let apex=[x,y+radius*1.2,z];

    for i in 0..4 {
        lck_r58_face(
            model,
            [corners[i],corners[(i+1)%4],apex],
            color
        );
    }
}

// LCK_R623E_ROUND_WARPS
// Native world-space circular marker with the same center
// used for mouse selection and 2D overlay rendering.
fn lck_r623e_round_glyph(
    model:&mut crate::raster::Model,
    center:[f32;3],
    radius:f32,
    color:[u8;3]
){
    if !center.iter().all(|v|v.is_finite()) {
        return;
    }

    let [x,y,z]=center;
    let top=[x,y+radius*0.28,z];

    const SEGMENTS:usize=16;

    for i in 0..SEGMENTS {
        let t0=i as f32
            *std::f32::consts::TAU
            /SEGMENTS as f32;

        let t1=(i+1) as f32
            *std::f32::consts::TAU
            /SEGMENTS as f32;

        let p0=[
            x+radius*t0.cos(),
            y,
            z+radius*t0.sin()
        ];

        let p1=[
            x+radius*t1.cos(),
            y,
            z+radius*t1.sin()
        ];

        // Both windings keep the round marker visible
        // when the camera rotates around a cell.
        lck_r58_face(model,[top,p0,p1],color);
        lck_r58_face(model,[top,p1,p0],color);

        lck_r58_ribbon(
            model,p0,p1,3.0,color
        );
    }
}
impl State {
    // A stable revision lets LCK reuse its renderer cache until
    // markers, selection or authored coordinates actually change.
    pub(super) fn world_marker_revision(&self)->u64 {
        use std::hash::{Hash,Hasher};

        let mut hash=
            std::collections::hash_map::DefaultHasher::new();

        self.scene.hash(&mut hash);
        self.show_markers.hash(&mut hash);
        self.v2082_mask().hash(&mut hash);
        self.v2084_stage_flags.hash(&mut hash);
        self.selected.hash(&mut hash);
        self.other_triggers.hash(&mut hash);

        // LCK_R630C_VERTICAL_MARKERS
        for (&uid,&height) in &self.r630c_warp_y {
            uid.hash(&mut hash);
            height.to_bits().hash(&mut hash);
        }

        for &height in &self.r630c_aux_y {
            height.to_bits().hash(&mut hash);
        }

        for d in &self.records {
            d.uid.hash(&mut hash);
            d.source.hash(&mut hash);
            d.size.hash(&mut hash);
            d.enabled.hash(&mut hash);
            format!("{:?}",d.kind).hash(&mut hash);
            format!("{:?}",d.origin).hash(&mut hash);
        }

        hash.finish()
    }

    // Add markers before the SAME world raster render.
    // Preserve the scene center and radius exactly.
    pub(super) fn world_marker_model(
        &self,
        original:&crate::raster::Model,
        ground:Option<
            &legaia_scene_formats::field_objects::WalkHeightfield
        >
    )->crate::raster::Model {
        let mut model=original.clone();

        if !self.show_markers {
            return model;
        }

        for d in &self.records {
            // LCK_R623D_UNIFIED_WARPS
            if !self.v2082_door_visible(d){
                continue;
            }

            let x=(
                d.source[0] as f32+
                d.size[0] as f32*0.5
            )*128.0;

            let z=(
                d.source[1] as f32+
                d.size[1] as f32*0.5
            )*128.0;

            let y=native_door_floor_y(ground,x,z)
                +LCK_R58_GIZMO_LIFT
                +self.r630c_warp_y.get(&d.uid)
                    .copied().unwrap_or(0.0);

            let color=if self.selected==Some(d.uid) {
                [255,220,75]
            }else if matches!(&d.kind,Kind::Local{..}){
                [95,238,135]
            }else{
                [110,205,255]
            };

            let center=[x,y,z];

            lck_r623e_round_glyph(
                &mut model,
                center,
                48.0,
                color
            );

            // Same-cell arrival is independent of the source.
            // Display its actual world location using the same
            // Y convention and raster as every other marker.
            // Same-cell arrival is visible even without selecting
            // the corresponding activation trigger.
            if let Kind::Local{half_x,half_z}=&d.kind {
                let dx=*half_x as f32*64.0+64.0;
                let dz=(*half_z as f32+1.0)*64.0;

                let dy=native_door_floor_y(
                    ground,dx,dz
                )+LCK_R58_GIZMO_LIFT;

                let arrival=[dx,dy,dz];
                let selected=self.selected==Some(d.uid);

                if selected {
                    lck_r58_ribbon(
                        &mut model,center,arrival,4.0,color
                    );
                }

                lck_r623e_round_glyph(
                    &mut model,
                    arrival,
                    32.0,
                    if selected {color}else{[65,235,125]}
                );
            }
        }

        // R6.0.5: place auxiliary markers in the original world raster.
        // Gate-0 markers use verified native object contacts when known.
        for (index,trigger) in self.other_triggers.iter().enumerate() {
            if !self.v2082_aux_visible(trigger){
                continue;
            }

            let positions:Vec<(f32,f32)>=
                if trigger.kind==1
                    && trigger.gate==0
                    && !trigger.objects.is_empty()
                {
                    trigger.objects.iter().map(
                        |(_,_,contact)|(
                            contact[0] as f32,
                            contact[1] as f32
                        )
                    ).collect()
                }else{
                    vec![(
                        (trigger.tile[0] as f32+0.5)*128.0,
                        (trigger.tile[1] as f32+0.5)*128.0
                    )]
                };

            let color=if trigger.kind==2 {
                [255,168,65]
            }else{
                [211,143,255]
            };

            for (x,z) in positions {
                let y=native_door_floor_y(ground,x,z)
                    +LCK_R58_GIZMO_LIFT
                    +self.r630c_aux_y.get(index)
                        .copied().unwrap_or(0.0);

                lck_r58_glyph(
                    &mut model,[x,y,z],30.0,color
                );
            }
        }
        // CRITICAL: do not call refit().
        // Gizmos must not move the scene's camera center/radius.
        model.center=original.center;
        model.radius=original.radius;

        model
    }
}

#[cfg(test)]
mod lck_r58_world_marker_tests {
    use super::*;

    fn base()->crate::raster::Model {
        crate::raster::Model {
            positions:vec![],
            rest:vec![],
            object_ids:vec![],
            uvs:vec![],
            cba_tsb:vec![],
            modulation:vec![],
            fill:vec![],
            textured:vec![],
            indices:vec![],
            center:[5000.0,0.0,5000.0],
            radius:8000.0,
        }
    }

    fn sample()->State {
        let mut state=State::default();

        state.scene="town01".into();
        state.show_markers=true;

        state.records.push(Door{
            uid:7,
            label:"Test doorway".into(),
            origin:Origin::New,
            source:[10,20],
            size:[1,1],
            enabled:true,
            kind:Kind::Local{
                half_x:32,
                half_z:42,
            },
        });

        state
    }

    #[test]
    fn world_geometry_preserves_camera_basis(){
        let state=sample();
        let original=base();

        let result=state.world_marker_model(
            &original,None
        );

        assert!(!result.indices.is_empty());
        assert!(result.positions.len()>original.positions.len());

        assert_eq!(result.center,original.center);
        assert_eq!(result.radius,original.radius);

        assert_eq!(
            result.positions.len(),
            result.object_ids.len()
        );

        assert!(result.object_ids.iter().all(
            |&id|id==LCK_R58_GIZMO_ID
        ));

        // Native tile (10,20) should produce a world-space
        // marker centered at (1344,2624), independent of camera.
        assert!(result.positions.iter().any(|p|
            (p[0]-1344.0).abs()<26.0
                && (p[2]-2624.0).abs()<26.0
        ));
    }

    #[test]
    fn selecting_local_warp_adds_world_arrival_link(){
        let mut state=sample();
        let base=base();

        let unselected=state.world_marker_model(
            &base,None
        );

        state.selected=Some(7);

        let selected=state.world_marker_model(
            &base,None
        );

        assert!(
            selected.indices.len()>unselected.indices.len()
        );

        assert!(selected.positions.iter().any(|p|
            (p[0]-2112.0).abs()<18.0
                && (p[2]-2752.0).abs()<18.0
        ));

        assert_eq!(selected.center,base.center);
        assert_eq!(selected.radius,base.radius);
    }

    #[test]
    fn edits_change_revision_without_camera_mutation(){
        let mut state=sample();
        let original=base();

        let a=state.world_marker_revision();

        state.records[0].source=[30,40];

        assert_ne!(
            a,
            state.world_marker_revision()
        );

        let result=state.world_marker_model(
            &original,None
        );

        assert_eq!(original.center,result.center);
        assert_eq!(original.radius,result.radius);

        state.show_markers=false;

        let hidden=state.world_marker_model(
            &original,None
        );

        assert_eq!(
            hidden.indices.len(),
            original.indices.len()
        );
    }
}
#[cfg(test)]
mod lck_r605_visibility_tests {
    use super::*;

    #[test]
    fn kind2_records_are_not_silently_dropped() {
        let mut block=vec![0u8;0x2000];

        block[10..12].copy_from_slice(&14i16.to_le_bytes());
        block[12..14].copy_from_slice(&2i16.to_le_bytes());

        block[14..22].copy_from_slice(&[
            10,20,30,40,
            11,21,31,41
        ]);

        assert_eq!(
            lck_r605_kind2(&block),
            vec![
                [10,20,30,40],
                [11,21,31,41]
            ]
        );

        assert!(lck_r605_kind2(&[]).is_empty());
    }
}
#[cfg(test)]
mod lck_r606_hierarchy_tests {
    use super::*;

    fn sample()->(State,Door) {
        let mut state=State::default();

        state.inspector_scripts.insert(
            3,
            ScriptSummary{
                flat_index:3,
                partition:2,
                native_name:Some("女王ドア".into()),
                script_start:0x100,
                record_end:0x200,
                ..Default::default()
            }
        );

        let door=Door{
            uid:42,
            label:String::new(),
            origin:Origin::Walk{
                fallback:false,row:2
            },
            source:[10,20],
            size:[1,1],
            enabled:true,
            kind:Kind::Script{
                slot:Some(3),
                op_pc:Some(0x105),
                destination:Some(Destination{
                    name:"town02".into(),
                    index:0,
                    entry_x:0,
                    entry_z:0,
                    facing:0,
                })
            }
        };

        (state,door)
    }

    #[test]
    fn verified_native_name_precedes_technical_fallback(){
        let (state,door)=sample();

        let label=state.r606_door_label(&door);

        assert!(label.contains("Doorway"));
        assert!(label.contains("女王ドア"));
        assert!(label.contains("town02"));
    }

    #[test]
    fn editor_alias_precedes_native_name(){
        let (state,mut door)=sample();

        door.label="Custom named entrance".into();

        assert_eq!(
            state.r606_door_label(&door),
            "Custom named entrance [editor name]"
        );
    }

    #[test]
    fn unnamed_map_warp_is_not_falsely_named(){
        let (state,mut door)=sample();

        door.kind=Kind::Local{
            half_x:31,
            half_z:42
        };

        assert!(
            state.r606_door_label(&door)
                .contains("Unnamed interior warp")
        );
    }
}
#[cfg(test)]
mod lck_r620_cross_cell_editor_tests {
    use super::*;
    fn fresh(uid:u64)->Door {
        Door {
            uid,label:format!("Warp {uid}"),origin:Origin::New,
            source:[uid as u8,2],size:[1,1],enabled:true,
            kind:Kind::Script {slot:None,op_pc:None,
                destination:Some(Destination {
                    name:"map01".into(),index:2,
                    entry_x:20,entry_z:21,facing:0,
                })
            }
        }
    }
    #[test]
    fn independent_slots_reassign_after_delete_or_undo() {
        let mut state=State::default();
        state.native_p2_records=12;
        state.records=vec![fresh(1),fresh(2),fresh(3)];
        state.renumber_independent_cross();
        let ids=|s:&State|s.records.iter().map(|d|match &d.kind {
            Kind::Script{slot,..}=>*slot,
            _=>None,
        }).collect::<Vec<_>>();
        assert_eq!(ids(&state),vec![Some(12),Some(13),Some(14)]);
        state.records.remove(1);
        state.renumber_independent_cross();
        assert_eq!(ids(&state),vec![Some(12),Some(13)]);
    }
}

#[cfg(test)]
mod lck_r623d_unified_warps_tests {
    use super::*;

    fn record(uid:u64,kind:Kind)->Door {
        Door{
            uid,
            label:format!("Warp {uid}"),
            origin:Origin::New,
            source:[4,5],
            size:[1,1],
            enabled:true,
            kind,
        }
    }

    #[test]
    fn local_and_cross_cell_share_one_visible_warp_category(){
        let mut state=State::default();
        state.show_markers=true;
        state.trigger_mask=Some(V2082_WARPS);

        let local=record(
            1,
            Kind::Local{
                half_x:7,
                half_z:8,
            }
        );

        let cross=record(
            2,
            Kind::Script{
                slot:Some(3),
                op_pc:None,
                destination:Some(Destination{
                    name:"map01".into(),
                    index:85,
                    entry_x:96,
                    entry_z:25,
                    facing:4,
                }),
            }
        );

        assert!(state.v2082_door_visible(&local));
        assert!(state.v2082_door_visible(&cross));

        state.trigger_mask=Some(V2082_SCRIPT);

        assert!(!state.v2082_door_visible(&local));
        assert!(state.v2082_door_visible(&cross));

        state.trigger_mask=Some(0);

        assert!(!state.v2082_door_visible(&local));
        assert!(!state.v2082_door_visible(&cross));
    }

    #[test]
    fn legacy_warp_overlay_stays_disabled(){
        let mut state=State::default();
        state.show_markers=true;
        state.trigger_mask=Some(V2082_WARPS);

        assert!(!state.v2083_local_warps_visible());
    }

    #[test]
    fn changing_warp_filter_invalidates_marker_cache(){
        let mut state=State::default();
        state.show_markers=true;
        state.trigger_mask=Some(V2082_WARPS);

        let before=state.world_marker_revision();

        state.trigger_mask=Some(V2082_SCRIPT);

        assert_ne!(before,state.world_marker_revision());
    }
}


#[cfg(test)]
mod lck_r623e_warp_visibility_tests {
    use super::*;

    #[test]
    fn first_individual_selection_does_not_enable_all(){
        assert_eq!(
            lck_r623e_toggle_mask(
                V2082_ALL,
                false,
                V2082_BIND,
                true
            ),
            V2082_BIND
        );

        assert_eq!(
            lck_r623e_toggle_mask(
                V2082_ALL,
                false,
                V2082_WARPS,
                true
            ),
            V2082_WARPS
        );
    }

    #[test]
    fn individual_categories_can_be_combined_and_removed(){
        let first=lck_r623e_toggle_mask(
            0,false,V2082_BIND,true
        );

        let second=lck_r623e_toggle_mask(
            first,true,V2082_KIND2,true
        );

        assert_eq!(
            second,
            V2082_BIND|V2082_KIND2
        );

        let third=lck_r623e_toggle_mask(
            second,true,V2082_BIND,false
        );

        assert_eq!(third,V2082_KIND2);
    }

    #[test]
    fn disabling_last_category_clears_selection(){
        assert_eq!(
            lck_r623e_toggle_mask(
                V2082_WARPS,
                true,
                V2082_WARPS,
                false
            ),
            0
        );
    }
}


#[cfg(test)]
mod lck_r626_trigger_identity_tests {
    use super::*;

    fn original()->Vec<u8> {
        let mut map=vec![0u8;0x12000];
        let o=0x10000;

        map[o+2..o+4]
            .copy_from_slice(&0x20i16.to_le_bytes());

        map[o+4..o+6]
            .copy_from_slice(&1i16.to_le_bytes());

        map[o+0x20..o+0x24]
            .copy_from_slice(&[10,20,30,40]);

        map
    }

    #[test]
    fn region_header_relocation_does_not_change_warp_identity(){
        let old=original();
        let mut staged=old.clone();

        staged[0x10000+0x0e..0x10000+0x10]
            .copy_from_slice(&0x488i16.to_le_bytes());

        staged[0x10000+0x10..0x10000+0x12]
            .copy_from_slice(&15i16.to_le_bytes());

        assert!(
            r626_verify_trigger_identity(
                &old,&staged
            ).is_ok()
        );
    }

    #[test]
    fn changed_existing_warp_record_is_rejected(){
        let old=original();
        let mut staged=old.clone();

        staged[0x10000+0x20]^=1;

        assert!(
            r626_verify_trigger_identity(
                &old,&staged
            ).is_err()
        );
    }

    #[test]
    fn missing_native_map_is_rejected(){
        assert!(
            r626_verify_trigger_identity(
                &[0;128],&[0;128]
            ).is_err()
        );
    }
}


// LCK_R627_DESTINATION_BROWSER
#[cfg(test)]
mod lck_r627_destination_group_tests {
    use super::*;

    fn catalog()->Vec<R622Scene> {
        [
            "conc","conc2","conc3","concnow","concend",
            "town01","town0b"
        ].iter()
            .map(|scene|R622Scene {
                name:(*scene).to_owned(),
                arrivals:Vec::new(),
            })
            .collect()
    }

    #[test]
    fn conkram_query_returns_all_five_variants() {
        let data=catalog();
        let groups=r627_destination_groups(
            &data,"Conkram"
        );

        assert_eq!(groups.len(),1);
        assert_eq!(groups[0].0,"Conkram");
        assert_eq!(groups[0].1.len(),5);
    }

    #[test]
    fn native_id_query_selects_one_variant() {
        let data=catalog();
        let groups=r627_destination_groups(
            &data,"conc3"
        );

        assert_eq!(groups.len(),1);
        assert_eq!(groups[0].0,"Conkram");
        assert_eq!(groups[0].1.len(),1);
        assert_eq!(groups[0].1[0].name,"conc3");
    }

    #[test]
    fn search_is_case_insensitive() {
        let data=catalog();
        let groups=r627_destination_groups(
            &data,"RIM ELM"
        );

        assert_eq!(groups.len(),1);
        assert_eq!(groups[0].0,"Rim Elm");
        assert_eq!(groups[0].1.len(),2);
    }
}

// LCK_R630C_VERTICAL_MARKERS
#[cfg(test)]
mod lck_r630c_height_tests {
    use super::*;

    #[test]
    fn warp_height_roundtrips_through_project_snapshot(){
        let mut state=State::default();
        state.scene="town01".into();
        state.next_uid=2;

        state.records.push(Door{
            uid:1,
            label:"Test warp".into(),
            origin:Origin::New,
            source:[10,10],
            size:[1,1],
            enabled:true,
            kind:Kind::Local{
                half_x:20,
                half_z:20,
            },
        });

        state.r630c_warp_y.insert(1,512.0);

        let saved=state.embedded_record("town01");

        let mut restored=State::default();
        restored.scene="town01".into();
        restored.restore(&saved).unwrap();

        assert_eq!(
            restored.r630c_warp_y.get(&1),
            Some(&512.0)
        );
        assert_eq!(restored.records[0].source,[10,10]);
    }

    #[test]
    fn height_only_does_not_request_native_export(){
        let mut state=State::default();
        state.scene="town01".into();

        state.r630c_warp_y.insert(1,256.0);

        assert!(!state.has_native_changes());
    }

    #[test]
    fn old_saved_project_defaults_to_no_height_offset(){
        let mut state=State::default();
        state.scene="town01".into();
        state.next_uid=1;

        let saved=state.embedded_record("town01");

        let mut json:serde_json::Value=
            serde_json::from_str(
                saved.trim_end()
                    .strip_prefix("DOOR_EDIT\t").unwrap()
            ).unwrap();

        json.as_object_mut().unwrap()
            .remove("r630c_warp_y");

        json.as_object_mut().unwrap()
            .remove("r630c_aux_y");

        let legacy=format!(
            "DOOR_EDIT\t{}\n",
            serde_json::to_string(&json).unwrap()
        );

        let mut restored=State::default();
        restored.scene="town01".into();
        restored.restore(&legacy).unwrap();

        assert!(restored.r630c_warp_y.is_empty());
        assert!(restored.r630c_aux_y.is_empty());
    }
}

// LCK_R633_TRIGGER_REGION_ISOLATION
#[cfg(test)]
mod lck_r633_door_storage_tests {
    use super::*;

    fn fixture()->Vec<u8>{
        let mut block=vec![0u8;0x2000];

        block[2..4]
            .copy_from_slice(&18i16.to_le_bytes());

        block[4..6]
            .copy_from_slice(&1i16.to_le_bytes());

        block[18..22]
            .copy_from_slice(&[2,3,10,11]);

        block[14..16]
            .copy_from_slice(&0x484i16.to_le_bytes());

        block[16..18]
            .copy_from_slice(&14i16.to_le_bytes());

        let original=0x484+5*8;

        block[original..original+8]
            .copy_from_slice(&[9,60,45,0,0,0,0,0]);

        block
    }

    #[test]
    fn enlarged_door_table_preserves_all_region_space()
        ->Result<()>{
        let mut block=fixture();
        let region_before=block[0x484..0x800].to_vec();

        let records=[
            [2,3,10,11],
            [97,52,35,0],
            [17,29,34,0],
        ];

        rewrite_table(&mut block,0,&records)?;

        let at=i16::from_le_bytes([
            block[2],block[3]
        ]) as usize;

        assert!(at>=0x800);

        assert_eq!(
            &block[0x484..0x800],
            &region_before
        );

        let decoded=table(&block,0)?;

        assert_eq!(
            decoded,
            records.iter()
                .map(|row|row.to_vec())
                .collect::<Vec<_>>()
        );

        Ok(())
    }

    #[test]
    fn nonzero_upper_tail_is_never_overwritten()
        ->Result<()>{
        let mut block=fixture();

        block[0x800]=0x77;

        let at=r633_free_trigger_slot(&block,12)?;

        assert_eq!(at,0x804);
        assert_eq!(block[0x800],0x77);

        Ok(())
    }

    #[test]
    fn referenced_zero_filled_table_is_protected()
        ->Result<()>{
        let mut block=fixture();

        // Kind 1 owns these bytes even though they are zero.
        block[6..8]
            .copy_from_slice(&0x800i16.to_le_bytes());

        block[8..10]
            .copy_from_slice(&1i16.to_le_bytes());

        let at=r633_free_trigger_slot(&block,8)?;

        assert_eq!(at,0x804);

        Ok(())
    }
}

#[cfg(test)]
mod r640_snapshot_tests {
    use super::*;
    fn state(heights:Vec<f32>)->State {
        let mut state=State{scene:"map03".into(),next_uid:1,
            other_triggers:vec![R605NativeTrigger{
                kind:2,gate:0,tile:[4,5],record:0,extra:0,
                fallback:false,row:0,objects:vec![],
            }],r630c_aux_y:heights,..Default::default()};
        state.original_auxiliary=state.other_triggers.clone();
        state
    }
    #[test]
    fn unopened_trigger_properties_roundtrip_without_losing_zero_defaults() {
        let live=state(vec![]);
        let saved=live.embedded_record("map03");
        let mut restored=state(vec![]);
        restored.restore(&saved).unwrap();
        assert_eq!(restored.embedded_record("map03"),saved);
        assert!(!live.has_native_changes());
    }
    #[test]
    fn explicit_trigger_heights_are_preserved_and_invalid_arrays_still_fail() {
        let live=state(vec![123.5]);
        let saved=live.embedded_record("map03");
        let mut restored=state(vec![]);
        restored.restore(&saved).unwrap();
        assert_eq!(restored.r630c_aux_y,vec![123.5]);
        assert_eq!(restored.embedded_record("map03"),saved);
        let invalid=state(vec![1.,2.]).embedded_record("map03");
        assert!(state(vec![]).restore(&invalid).is_err());
    }
}
