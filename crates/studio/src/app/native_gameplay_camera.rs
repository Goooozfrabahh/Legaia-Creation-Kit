// LCK_R616D_NATIVE_GAMEPLAY_CAMERA
// Native MAN section-3 camera zones. Retail IMG is never modified.
use anyhow::{anyhow,Context,Result,ensure};
use eframe::egui;
use serde::{Deserialize,Serialize};
use std::{convert::TryInto,fs,path::Path};
use legaia_disc_patch::{disc::DiscPatcher,compress_within};
use legaia_engine_scene::scene::{ProtIndex,Scene};
use legaia_scene_formats::{man_section,scene_asset_table};
use legaia_engine_field::{
    camera_zone::{CameraZoneConfig,ComposeInputs,compose},
    field_regions::{RegionTable,refresh_region_attributes,zone_query},
};
use super::{collision_recipes,ps1_export,viewport::Viewport};

const STRIDE:usize=18;

#[derive(Clone,Debug,PartialEq,Eq,Serialize,Deserialize)]
pub(super) struct CameraRow {
    pub name:String,
    pub raw:[u8;STRIDE]
}

#[derive(Clone,Debug,PartialEq,Eq,Serialize,Deserialize)]
pub(super) struct SceneCameras {
    pub scene:String,
    pub fingerprint:String,
    pub original:Vec<[u8;STRIDE]>,
    pub rows:Vec<CameraRow>,
}

impl SceneCameras {
    pub(super) fn changed(&self)->bool {
        self.original.len()!=self.rows.len() ||
        self.original.iter().zip(&self.rows)
            .any(|(a,b)|*a!=b.raw)
    }
}

#[derive(Default)]
pub(super) struct Editor {
    pub open:bool,
    pub show_markers:bool,
    scene:String,
    source:String,
    fingerprint:String,
    original:Vec<[u8;STRIDE]>,
    rows:Vec<CameraRow>,
    selected:usize,
    sample:[u8;2],
    map:Vec<u8>,
    half_eye:bool,
    dirty:bool,
}

fn parse_rows(man:&[u8])->Result<Vec<[u8;STRIDE]>> {
    let file=man_section::parse(man)
        .map_err(|e|anyhow!("MAN parse: {e}"))?;

    let body=file.sections[3].body(man)
        .context("MAN camera section missing")?;

    let count=*body.first()
        .context("MAN camera count missing")? as usize;

    ensure!(
        1+count*STRIDE<=body.len(),
        "MAN camera record table truncated"
    );

    body[1..1+count*STRIDE]
        .chunks_exact(STRIDE)
        .map(|chunk|
            chunk.try_into().map_err(anyhow::Error::from)
        )
        .collect()
}

fn rewrite_man(man:&[u8],data:&SceneCameras)->Result<Vec<u8>> {
    let file=man_section::parse(man)
        .map_err(|e|anyhow!("MAN parse: {e}"))?;

    let sec=file.sections[3];
    let old=parse_rows(man)?;

    ensure!(
        old==data.original,
        "MAN camera records changed since editor loaded"
    );

    ensure!(
        data.rows.len()<=255,
        "Native MAN camera count must fit u8"
    );

    for row in &data.rows {
        if row.raw[0]==1 {
            ensure!(
                row.raw[1]<=row.raw[3] &&
                row.raw[2]<=row.raw[4] &&
                row.raw[3]<128 &&
                row.raw[4]<128,
                "Invalid bounded camera rectangle"
            );
        }
    }

    let intended=data.rows.iter()
        .map(|r|r.raw)
        .collect::<Vec<_>>();

    let mut out;

    if data.rows.len()==old.len() {
        out=man.to_vec();

        for (i,row) in data.rows.iter().enumerate() {
            let off=sec.body_offset()+1+i*STRIDE;
            out[off..off+STRIDE].copy_from_slice(&row.raw);
        }
    } else {
        ensure!(
            data.rows.len()>old.len(),
            "Deleting native camera records is unsupported"
        );

        let body=sec.body(man)
            .context("MAN section 3 unavailable")?;

        ensure!(
            body.len()==1+old.len()*STRIDE,
            "Camera section has trailing bytes; unsafe to relocate"
        );

        let len=1+data.rows.len()*STRIDE;

        ensure!(
            len<=0x00ff_ffff,
            "MAN section overflow"
        );

        out=Vec::with_capacity(man.len()+len-body.len());
        out.extend_from_slice(&man[..sec.offset]);

        let n=len as u32;

        out.extend_from_slice(&[
            n as u8,
            (n>>8) as u8,
            (n>>16) as u8
        ]);

        out.push(data.rows.len() as u8);

        for row in &data.rows {
            out.extend_from_slice(&row.raw);
        }

        out.extend_from_slice(&man[sec.end_offset()..]);
    }

    man_section::parse(&out)
        .map_err(|e|anyhow!("Rewritten MAN invalid: {e}"))?;

    ensure!(
        parse_rows(&out)?==intended,
        "Camera write/readback mismatch"
    );

    Ok(out)
}

// Kind-1 is a true activation box. Kind-0 is an anchor point.
// For mask camera records (2..31), the box is a visual window,
// not necessarily the actual activation zone.
fn camera_box(raw:&[u8;STRIDE])->Option<[u8;4]> {
    match raw[0] {
        0=>Some([
            raw[1],raw[2],
            raw[1],raw[2]
        ]),
        1=>Some([
            raw[1],raw[2],
            raw[3],raw[4]
        ]),
        2..=31 if raw[1]!=0=>Some([
            raw[3],raw[4],
            raw[1],raw[2]
        ]),
        _=>None,
    }
}

fn starter(tile:[u8;2])->CameraRow {
    let mut raw=[0u8;STRIDE];

    raw[0]=1;
    raw[1]=tile[0].saturating_sub(2);
    raw[2]=tile[1].saturating_sub(2);
    raw[3]=tile[0].saturating_add(2).min(127);
    raw[4]=tile[1].saturating_add(2).min(127);
    raw[5]=0x10;

    raw[12..14].copy_from_slice(&0x01b8i16.to_le_bytes());
    raw[14..16].copy_from_slice(&0x4000i16.to_le_bytes());
    raw[16..18].copy_from_slice(&0x0300i16.to_le_bytes());

    CameraRow {
        name:"New Interior Camera".into(),
        raw
    }
}

fn from_disc(
    source:&Path,
    index:&ProtIndex,
    scene:&str
)->Result<(String,Vec<[u8;STRIDE]>,Vec<u8>,bool)> {
    let selected=Scene::load(index,scene)?;

    let map_id=selected.field_map_index(index)
        .context("Scene has no native MAP")? as usize;

    let disc=DiscPatcher::open(fs::read(source)?)?;
    let map=disc.read_entry_footprint(map_id)?;

    ensure!(
        map.len()>=0x12000,
        "Native MAP is truncated"
    );

    let (_,_,_,_,man)=ps1_export::find_native_man(
        &disc,map_id
    )?;

    let parsed=man_section::parse(&man)
        .map_err(|e|anyhow!("MAN parse: {e}"))?;

    Ok((
        collision_recipes::digest(&man),
        parse_rows(&man)?,
        map,
        parsed.header.low_flag
    ))
}

// LCK_R617E_CAMERA_UX

fn r617_camera_type(kind:u8)->&'static str {
    match kind {
        0 => "Region anchor",
        1 => "Rectangular zone",
        2..=31 => "MAP region / mask",
        _ => "Unknown native camera type"
    }
}

fn r617_mode_name(mode:u8)->&'static str {
    match mode {
        1 => "Follow Vahn",
        2 => "Follow Vahn (alternate)",
        3 => "Look at fixed position",
        4 => "Aim from region center",
        5 => "Fixed shot",
        6 => "Keep previous camera",
        _ => "Unidentified camera mode"
    }
}

fn r617_mode_help(mode:u8)->&'static str {
    match mode {
        1|2 => "The viewpoint follows Vahn. These two modes use different retail formulas.",
        3 => "Looks toward a fixed anchor. Some native angle fields become coordinates.",
        4 => "The game calculates orientation using the active MAP region.",
        5 => "Fixed view: native bytes 6-8 are position parameters, not ordinary easing.",
        6 => "Keep the camera already active. No new target pose is composed.",
        _ => "This mode has not been identified. Preserve its raw bytes unless tested."
    }
}

fn r617_deg(value:i16)->f64 {
    f64::from(value)*360.0/4096.0
}

fn r617_native(degrees:f64)->i16 {
    (degrees*4096.0/360.0).round()
        .clamp(f64::from(i16::MIN), f64::from(i16::MAX)) as i16
}

fn r617_word(
    ui:&mut egui::Ui,
    raw:&mut [u8;STRIDE],
    at:usize,
    label:&str,
    help:&str,
    angles:bool
)->bool {
    let old=i16::from_le_bytes([raw[at],raw[at+1]]);

    if angles {
        let mut angle=r617_deg(old);

        let changed=ui.horizontal(|ui|{
            ui.label(label).on_hover_text(help);

            ui.add(
                egui::DragValue::new(&mut angle)
                    .speed(0.5)
                    .suffix(" degrees")
            ).changed()
        }).inner;

        if changed {
            raw[at..at+2].copy_from_slice(
                &r617_native(angle).to_le_bytes()
            );
        }

        changed
    }else {
        let mut value=old;

        let changed=ui.horizontal(|ui|{
            ui.label(label).on_hover_text(help);

            ui.add(
                egui::DragValue::new(&mut value).speed(4.0)
            ).changed()
        }).inner;

        if changed {
            raw[at..at+2].copy_from_slice(
                &value.to_le_bytes()
            );
        }

        changed
    }
}

pub(super) fn window(
    ctx:&egui::Context,
    editor:&mut Editor,
    source:&Path,
    index:&ProtIndex,
    scene:&str,
    saved:Option<&SceneCameras>,
    project_open:bool
)->Result<Option<SceneCameras>> {
    if !editor.open {
        return Ok(None);
    }

    let src=source.display().to_string();

    if editor.scene!=scene || editor.source!=src {
        let (fingerprint,original,map,half)=
            from_disc(source,index,scene)?;

        editor.scene=scene.to_owned();
        editor.source=src;
        editor.fingerprint=fingerprint;

        editor.original=original.clone();

        editor.rows=original.into_iter()
            .enumerate()
            .map(|(i,raw)|CameraRow {
                name:format!("Native Camera {i}"),
                raw
            })
            .collect();

        if let Some(saved)=saved {
            ensure!(
                saved.fingerprint==editor.fingerprint &&
                saved.original==editor.original &&
                saved.scene==scene,
                "Saved camera project does not match the original scene MAN"
            );

            editor.rows=saved.rows.clone();
        }

        editor.map=map;
        editor.half_eye=half;
        editor.selected=0;
        editor.sample=[35,30];
        editor.dirty=false;
    }

    let mut opened=editor.open;
    let mut close=false;
    let mut result=None;

    egui::Window::new("Gameplay Cameras - Properties & Settings")
        .id(egui::Id::new("lck_r616d_cameras"))
        .open(&mut opened)
        .default_size(egui::vec2(810.0,610.0))
        .min_width(700.0)
        .resizable(true)
        .show(ctx,|ui|{
            ui.heading("Gameplay Cameras");

            ui.small(format!(
                "Cell: {}    |    {} camera records",
                editor.scene,
                editor.rows.len()
            ));

            ui.small(
                "These are in-game cameras from MAN data. They are separate from the free-look editor camera."
            );

            ui.separator();

            ui.horizontal(|ui|{
                ui.vertical(|ui|{
                    ui.set_min_width(220.0);
                    ui.set_max_width(240.0);

                    ui.strong("Cameras in this cell");
                    ui.small("Select a camera to edit it.");

                    egui::ScrollArea::vertical()
                        .id_salt("r617e_list")
                        .max_height(290.0)
                        .show(ui,|ui|{
                            for (i,row) in editor.rows.iter().enumerate() {
                                let label=if row.name.trim().is_empty(){
                                    format!("Camera {i}")
                                }else {
                                    row.name.clone()
                                };

                                let picked=ui.selectable_label(
                                    editor.selected==i,
                                    format!("#{i}  {label}")
                                );

                                if picked.clicked(){
                                    editor.selected=i;
                                }

                                picked.on_hover_text(format!(
                                    "{} / {}",
                                    r617_camera_type(row.raw[0]),
                                    r617_mode_name(row.raw[5]>>4)
                                ));
                            }
                        });

                    ui.separator();

                    if ui.add_enabled(
                        editor.rows.len()<255,
                        egui::Button::new("+ New interior camera")
                    )
                    .on_hover_text(
                        "Creates a native rectangular camera zone around the preview position."
                    )
                    .clicked(){
                        editor.rows.push(starter(editor.sample));
                        editor.selected=editor.rows.len()-1;
                        editor.dirty=true;
                    }

                    if ui.add_enabled(
                        editor.rows.len()<255 && !editor.rows.is_empty(),
                        egui::Button::new("Duplicate selected")
                    )
                    .on_hover_text(
                        "Creates a separate copy of this camera record."
                    )
                    .clicked(){
                        if let Some(mut copy)=
                            editor.rows.get(editor.selected).cloned()
                        {
                            copy.name.push_str(" Copy");
                            editor.rows.push(copy);
                            editor.selected=editor.rows.len()-1;
                            editor.dirty=true;
                        }
                    }

                    ui.horizontal(|ui|{
                        if ui.add_enabled(
                            editor.selected>0,
                            egui::Button::new("Move up")
                        ).clicked(){
                            let i=editor.selected;
                            editor.rows.swap(i,i-1);
                            editor.selected-=1;
                            editor.dirty=true;
                        }

                        if ui.add_enabled(
                            editor.selected+1<editor.rows.len(),
                            egui::Button::new("Move down")
                        ).clicked(){
                            let i=editor.selected;
                            editor.rows.swap(i,i+1);
                            editor.selected+=1;
                            editor.dirty=true;
                        }
                    });

                    ui.small("Priority: first matching zone wins.");
                    ui.separator();

                    ui.checkbox(
                        &mut editor.show_markers,
                        "Show indicators in Render Window"
                    )
                    .on_hover_text(
                        "Editor-only visual overlays: does not change PS1 camera records."
                    );

                    ui.small(
                        "Also available from View > Gameplay Cameras."
                    );
                });

                ui.separator();

                ui.vertical(|ui|{
                    ui.set_min_width(430.0);

                    egui::ScrollArea::vertical()
                        .id_salt("r617e_details")
                        .max_height(460.0)
                        .show(ui,|ui|{
                            let sample=editor.sample;

                            if let Some(row)=
                                editor.rows.get_mut(editor.selected)
                            {
                                ui.strong("Camera Info");

                                ui.horizontal(|ui|{
                                    ui.label("Name")
                                        .on_hover_text(
                                            "Only used in your LCK project, not in the original PS1 code."
                                        );

                                    editor.dirty |=ui.text_edit_singleline(
                                        &mut row.name
                                    ).changed();
                                });

                                ui.label(format!(
                                    "Type: {}",
                                    r617_camera_type(row.raw[0])
                                ))
                                .on_hover_text(
                                    "Native camera-zone type from MAN section 3."
                                );

                                ui.label(format!(
                                    "Priority: {}",
                                    editor.selected+1
                                ));

                                ui.separator();

                                ui.strong("Activation Area");

                                match row.raw[0] {
                                    1=>{
                                        ui.small(
                                            "The area where a camera-zone query can select this camera. Values are native MAP tile coordinates."
                                        );

                                        for (at,label) in [
                                            (1usize,"Minimum X"),
                                            (3usize,"Maximum X"),
                                            (2usize,"Minimum Z"),
                                            (4usize,"Maximum Z")
                                        ] {
                                            ui.horizontal(|ui|{
                                                ui.label(label);

                                                editor.dirty |=ui.add(
                                                    egui::DragValue::new(
                                                        &mut row.raw[at]
                                                    ).range(0..=127)
                                                ).changed();
                                            });
                                        }

                                        if ui.button(
                                            "Center area on preview position"
                                        ).clicked(){
                                            row.raw[1]=sample[0].saturating_sub(2);
                                            row.raw[2]=sample[1].saturating_sub(2);

                                            row.raw[3]=sample[0]
                                                .saturating_add(2).min(127);

                                            row.raw[4]=sample[1]
                                                .saturating_add(2).min(127);

                                            editor.dirty=true;
                                        }

                                        if row.raw[1]>row.raw[3] ||
                                            row.raw[2]>row.raw[4]
                                        {
                                            ui.colored_label(
                                                egui::Color32::YELLOW,
                                                "The area is reversed. Correct its bounds before exporting."
                                            );
                                        }
                                    },

                                    0=>{
                                        ui.small(
                                            "This camera checks a MAP region containing an anchor tile; it isn't a standalone rectangular trigger."
                                        );

                                        for (at,label) in [
                                            (1usize,"Anchor X"),
                                            (2usize,"Anchor Z")
                                        ] {
                                            ui.horizontal(|ui|{
                                                ui.label(label);

                                                editor.dirty |=ui.add(
                                                    egui::DragValue::new(
                                                        &mut row.raw[at]
                                                    ).range(0..=127)
                                                ).changed();
                                            });
                                        }
                                    },

                                    _=>{
                                        ui.small(
                                            "This camera uses MAP region masks. Its true activation area is not necessarily a rectangle."
                                        );

                                        ui.small(
                                            "See Advanced Native Fields for its original bytes."
                                        );
                                    }
                                }

                                ui.separator();

                                ui.strong("Camera Behavior");

                                let mut mode=row.raw[5]>>4;

                                egui::ComboBox::from_id_salt(
                                    "r617e_mode"
                                )
                                .selected_text(r617_mode_name(mode))
                                .show_ui(ui,|ui|{
                                    for option in 1u8..=6 {
                                        ui.selectable_value(
                                            &mut mode,
                                            option,
                                            r617_mode_name(option)
                                        );
                                    }
                                });

                                if mode!=(row.raw[5]>>4){
                                    row.raw[5]=(mode<<4)|(row.raw[5]&15);
                                    editor.dirty=true;
                                }

                                ui.small(r617_mode_help(mode));

                                if matches!(mode,1|2|4){
                                    editor.dirty |=r617_word(
                                        ui,
                                        &mut row.raw,
                                        10,
                                        "Yaw (horizontal)",
                                        "Camera rotation in degrees. The game can modify yaw based on the region.",
                                        true
                                    );

                                    editor.dirty |=r617_word(
                                        ui,
                                        &mut row.raw,
                                        12,
                                        "Pitch (tilt)",
                                        "Camera tilt in degrees; converted back to native PS1 angle units.",
                                        true
                                    );

                                }else if matches!(mode,3|5) {
                                    ui.small(
                                        "This mode uses native anchor coordinates in some angle fields. Edit them in Advanced."
                                    );
                                }

                                if mode!=6 {
                                    editor.dirty |=r617_word(
                                        ui,
                                        &mut row.raw,
                                        14,
                                        "Distance / depth",
                                        "Native camera depth parameter; not automatically a world-unit distance.",
                                        false
                                    );

                                    editor.dirty |=r617_word(
                                        ui,
                                        &mut row.raw,
                                        16,
                                        "Perspective (H)",
                                        "GTE projection scale, not the camera's physical height.",
                                        false
                                    );
                                }

                                ui.separator();

                                ui.collapsing(
                                    "Advanced Native Fields",
                                    |ui|{
                                        ui.small(
                                            "Raw MAN camera settings. Only change unknown fields if you can verify them in DuckStation."
                                        );

                                        ui.monospace(format!(
                                            "Record kind: {}   Mode byte: 0x{:02X}",
                                            row.raw[0],
                                            row.raw[5]
                                        ));

                                        for (at,label) in [
                                            (6usize,"Native byte 6"),
                                            (7usize,"Native byte 7"),
                                            (8usize,"Native byte 8"),
                                            (9usize,"Native byte 9 / easing")
                                        ] {
                                            ui.horizontal(|ui|{
                                                ui.label(label);

                                                editor.dirty |=ui.add(
                                                    egui::DragValue::new(
                                                        &mut row.raw[at]
                                                    )
                                                ).changed();
                                            });
                                        }

                                        if row.raw[0]>=2 {
                                            for at in 1usize..=4 {
                                                ui.horizontal(|ui|{
                                                    ui.label(format!(
                                                        "Region byte {at}"
                                                    ));

                                                    editor.dirty |=ui.add(
                                                        egui::DragValue::new(
                                                            &mut row.raw[at]
                                                        )
                                                    ).changed();
                                                });
                                            }
                                        }

                                        let mut low=row.raw[5]&0x0f;

                                        ui.horizontal(|ui|{
                                            ui.label("Mode low flags");

                                            if ui.add(
                                                egui::DragValue::new(&mut low)
                                                    .range(0..=15)
                                            ).changed(){
                                                row.raw[5]=
                                                    (row.raw[5]&0xf0)|low;

                                                editor.dirty=true;
                                            }
                                        });

                                        for (at,label) in [
                                            (10usize,"Raw word 10-11"),
                                            (12usize,"Raw word 12-13"),
                                            (14usize,"Raw depth 14-15"),
                                            (16usize,"Raw projection 16-17")
                                        ] {
                                            editor.dirty |=r617_word(
                                                ui,
                                                &mut row.raw,
                                                at,
                                                label,
                                                "Signed native 16-bit field; meaning depends on camera mode.",
                                                false
                                            );
                                        }

                                        ui.small(
                                            "Complete original-format record (hex):"
                                        );

                                        ui.monospace(
                                            row.raw.iter()
                                                .map(|x|format!("{x:02X}"))
                                                .collect::<Vec<_>>()
                                                .join(" ")
                                        );
                                    }
                                );

                            }else {
                                ui.label(
                                    "Select a camera in the list to edit its properties."
                                );
                            }

                            ui.separator();

                            ui.strong(
                                "Preview / Camera Simulation"
                            );

                            ui.small(
                                "Move sample Vahn to see which camera would match when the game's camera query runs."
                            );

                            ui.horizontal(|ui|{
                                ui.label("Vahn tile X");

                                ui.add(
                                    egui::DragValue::new(
                                        &mut editor.sample[0]
                                    ).range(0..=127)
                                );

                                ui.label("Z");

                                ui.add(
                                    egui::DragValue::new(
                                        &mut editor.sample[1]
                                    ).range(0..=127)
                                );
                            });

                            if let Some(row)=
                                editor.rows.get(editor.selected)
                            {
                                if let Some([x0,z0,x1,z1])=
                                    camera_box(&row.raw)
                                {
                                    if ui.button(
                                        "Preview at selected camera"
                                    ).clicked(){
                                        editor.sample=[
                                            ((u16::from(x0)+
                                              u16::from(x1))/2) as u8,

                                            ((u16::from(z0)+
                                              u16::from(z1))/2) as u8
                                        ];
                                    }
                                }
                            }

                            if editor.map.len()>=0x12000 &&
                                editor.rows.len()<=255
                            {
                                let region=RegionTable::parse(
                                    &editor.map[0x10000..]
                                );

                                let (x,z)=(
                                    editor.sample[0] as i32,
                                    editor.sample[1] as i32
                                );

                                let (_,attrs)=
                                    refresh_region_attributes(
                                        region.as_ref(),
                                        x,z,false
                                    );

                                let mut table=
                                    vec![editor.rows.len() as u8];

                                for row in &editor.rows {
                                    table.extend_from_slice(&row.raw);
                                }

                                if let Some(hit)=zone_query(
                                    &table,
                                    region.as_ref(),
                                    &attrs,
                                    x,z
                                ) {
                                    if let Some(record)=hit.record {
                                        if let Ok(raw)=
                                            <[u8;STRIDE]>::try_from(record)
                                        {
                                            let active=editor.rows
                                                .iter()
                                                .position(|r|r.raw==raw);

                                            let mut cfg=
                                                CameraZoneConfig::default();

                                            cfg.load_record(&raw);

                                            let computed=compose(
                                                &cfg,
                                                &ComposeInputs{
                                                    player:[
                                                        x*128+64,
                                                        0,
                                                        z*128+64
                                                    ],

                                                    floor_y:0,
                                                    attr_box:attrs.box_bytes,
                                                    live_pitch:0x1b8,
                                                    live_yaw:0,
                                                    half_eye_y:editor.half_eye
                                                }
                                            );

                                            ui.label(format!(
                                                "Matching camera: {}",
                                                active.map(|i|format!(
                                                    "#{i} {}",
                                                    editor.rows[i].name
                                                ))
                                                .unwrap_or_else(
                                                    ||"Unknown".into()
                                                )
                                            ));

                                            ui.small(format!(
                                                "Yaw {:.1} degrees | Pitch {:.1} degrees",
                                                r617_deg(
                                                    computed.target.yaw
                                                ),
                                                r617_deg(
                                                    computed.target.pitch
                                                )
                                            ));

                                            ui.small(format!(
                                                "Eye {:?} | Perspective H {}",
                                                computed.target.eye,
                                                computed.target.h
                                            ));

                                            if active!=Some(editor.selected) {
                                                ui.colored_label(
                                                    egui::Color32::YELLOW,
                                                    "Selected camera is not first matching at this position."
                                                );
                                            }
                                        }
                                    }else {
                                        ui.small(
                                            "No camera matched the preview position."
                                        );
                                    }
                                }
                            }

                            ui.small(
                                "Math preview only: actual ground elevation, smoothing and script activation aren't simulated."
                            );
                        });
                });
            });

            ui.separator();

            ui.horizontal(|ui|{
                if !project_open {
                    ui.colored_label(
                        egui::Color32::YELLOW,
                        "Open an LCK project to save camera edits."
                    );
                }

                if ui.add_enabled(
                    project_open && editor.rows.len()<=255,
                    egui::Button::new("Apply to LCK Project")
                )
                .on_hover_text(
                    "Stores changes in the project. Then use File > Save Project and export a new IMG."
                )
                .clicked(){
                    result=Some(SceneCameras{
                        scene:editor.scene.clone(),
                        fingerprint:editor.fingerprint.clone(),
                        original:editor.original.clone(),
                        rows:editor.rows.clone()
                    });

                    editor.dirty=false;
                }

                if editor.dirty {
                    ui.small("Unapplied edits");
                }

                if ui.button("Close").clicked(){
                    close=true;
                }
            });

            ui.small(
                "Editor-only names; native camera bytes export through R616D. New camera activation may need a script."
            );
        });

    editor.open=opened && !close;
    Ok(result)
}

#[cfg(test)]
mod r617e_ui_tests {
    use super::*;

    #[test]
    fn mode_display(){
        assert_eq!(r617_mode_name(1),"Follow Vahn");
    }

    #[test]
    fn kind_display(){
        assert_eq!(r617_camera_type(1),"Rectangular zone");
    }

    #[test]
    fn angles_roundtrip(){
        assert_eq!(r617_native(r617_deg(450)),450);
    }

    #[test]
    fn anchor_mode_help(){
        assert!(r617_mode_help(5).contains("position"));
    }
}
pub(super) fn paint(
    ui:&egui::Ui,
    rect:egui::Rect,
    model:&crate::raster::Model,
    view:&Viewport,
    ed:&Editor,
    scene:&str
) {
    if !ed.show_markers || ed.scene!=scene {
        return;
    }

    let painter=ui.painter().with_clip_rect(rect);

    for (i,row) in ed.rows.iter().enumerate(){
        let Some([x0,z0,x1,z1])=camera_box(&row.raw)
        else{
            continue;
        };

        let y=28.0;

        let corners=[
            [x0 as f32*128.0,y,z0 as f32*128.0],
            [x1 as f32*128.0,y,z0 as f32*128.0],
            [x1 as f32*128.0,y,z1 as f32*128.0],
            [x0 as f32*128.0,y,z1 as f32*128.0]
        ];

        let color=if i==ed.selected {
            egui::Color32::YELLOW
        }else{
            egui::Color32::LIGHT_BLUE
        };

        for n in 0..4 {
            if let (Some(a),Some(b))=(
                view.project_world_marker(model,corners[n]),
                view.project_world_marker(model,corners[(n+1)%4])
            ){
                painter.line_segment(
                    [a,b],
                    egui::Stroke::new(
                        if i==ed.selected {2.5}else{1.1},
                        color
                    )
                );
            }
        }

        let center=[
            (x0 as f32+x1 as f32)*64.0,
            72.0,
            (z0 as f32+z1 as f32)*64.0
        ];

        if let Some(p)=view.project_world_marker(
            model,center
        ){
            painter.circle_filled(
                p,
                if i==ed.selected {9.0}else{6.0},
                color
            );

            painter.text(
                p+egui::vec2(11.0,-10.0),
                egui::Align2::LEFT_CENTER,
                format!("Camera #{i}"),
                egui::FontId::proportional(13.0),
                color
            );
        }
    }
}

pub(super) fn interact(
    ui:&egui::Ui,
    rect:egui::Rect,
    model:&crate::raster::Model,
    view:&Viewport,
    ed:&mut Editor,
    scene:&str
)->bool {
    if !ed.show_markers ||
        ed.scene!=scene ||
        !ui.input(|i|i.pointer.primary_pressed())
    {
        return false;
    }

    let Some(pointer)=ui.input(|i|i.pointer.interact_pos())
    else{
        return false;
    };

    if !rect.contains(pointer) ||
        ui.ctx().layer_id_at(pointer)
            .is_some_and(|layer|layer!=ui.layer_id())
    {
        return false;
    }

    let mut best=None;
    let mut distance=14.0f32*14.0;

    for (i,row) in ed.rows.iter().enumerate(){
        let Some([x0,z0,x1,z1])=camera_box(&row.raw)
        else{
            continue;
        };

        let p=[
            (x0 as f32+x1 as f32)*64.0,
            72.0,
            (z0 as f32+z1 as f32)*64.0
        ];

        if let Some(projected)=view.project_world_marker(
            model,p
        ){
            let d=(projected-pointer).length_sq();

            if d<distance {
                best=Some(i);
                distance=d;
            }
        }
    }

    if let Some(i)=best {
        ed.selected=i;
        true
    }else{
        false
    }
}

pub(super) fn overlay_existing(
    target:&Path,
    retail:&Path,
    index:&ProtIndex,
    scene:&str,
    camera:&SceneCameras
)->Result<String> {
    ensure!(
        camera.scene==scene,
        "Camera snapshot is for a different cell"
    );

    ensure!(
        camera.changed(),
        "No native camera changes requested"
    );

    let selected=Scene::load(index,scene)?;

    let map=selected.field_map_index(index)
        .context("Scene MAP absent")? as usize;

    let original=DiscPatcher::open(fs::read(retail)?)?;

    let (_,_,_,_,original_man)=
        ps1_export::find_native_man(&original,map)?;

    ensure!(
        collision_recipes::digest(&original_man)==camera.fingerprint &&
        parse_rows(&original_man)?==camera.original,
        "Source MAN fingerprint or camera records changed"
    );

    let mut disc=DiscPatcher::open(fs::read(target)?)?;

    let (entry,offset,budget,slot,decoded)=
        ps1_export::find_native_man(&disc,map)?;

    ensure!(
        parse_rows(&decoded)?==camera.original,
        "Another exporter stage changed the camera table"
    );

    let rewritten=rewrite_man(&decoded,camera)?;

    let packed=compress_within(
        &rewritten,budget
    ).context(
        "Camera MAN exceeds the native compressed allocation"
    )?;

    disc.patch_prot_entry(
        entry,
        offset as u64,
        &packed
    )?;

    if rewritten.len()!=decoded.len(){
        let new_len=u32::try_from(rewritten.len())?;

        let size=scene_asset_table::encode_size_word(
            3,new_len
        );

        disc.patch_prot_entry(
            entry,
            scene_asset_table::SceneAssetTable::
                size_word_offset(slot) as u64,
            &size.to_le_bytes()
        )?;

        let entry_bytes=disc.read_entry_footprint(entry)?;

        let total=u32::from_le_bytes(
            entry_bytes[4..8].try_into()?
        );

        let updated=total
            .checked_sub(decoded.len() as u32)
            .and_then(|n|n.checked_add(new_len))
            .context(
                "Scene asset bundle decoded-size overflow"
            )?;

        disc.patch_prot_entry(
            entry,
            4,
            &updated.to_le_bytes()
        )?;
    }

    let footprint=disc.read_entry_footprint(entry)?;

    let (read,_)=legaia_lzs::decompress_tracked(
        &footprint[offset..],
        rewritten.len()
    )?;

    ensure!(
        read==rewritten,
        "Camera staged MAN readback mismatch"
    );

        // LCK_R618B_CAMERA_WRITE
    fs::write(target,disc.into_image()).with_context(||format!(
        "CAMERA MAN WRITE FAILED: staged={} readonly={}",
        target.display(),
        fs::metadata(target)
            .map(|m|m.permissions().readonly())
            .unwrap_or(false)
    ))?;

    let verified=DiscPatcher::open(fs::read(target)?)?;
    let physical=verified.read_entry_footprint(entry)?;

    let (read,_)=legaia_lzs::decompress_tracked(
        &physical[offset..],
        rewritten.len()
    )?;

    ensure!(
        read==rewritten,
        "Camera physical IMG readback mismatch"
    );

    Ok(format!(
        "R616D NATIVE CAMERAS: {} original, {} exported; MAN physically verified; camera-script activation not yet proven",
        camera.original.len(),
        camera.rows.len()
    ))
}

#[cfg(test)]
mod r616d_camera_tests {
    use super::*;

    #[test]
    fn bounded_zone(){
        let c=starter([60,40]);
        assert_eq!(
            camera_box(&c.raw),
            Some([58,38,62,42])
        );
    }

    #[test]
    fn grid_edge(){
        let c=starter([127,127]);
        assert_eq!(
            camera_box(&c.raw),
            Some([125,125,127,127])
        );
    }

    #[test]
    fn native_angle_default(){
        let c=starter([25,20]);
        assert_eq!(
            i16::from_le_bytes([c.raw[12],c.raw[13]]),
            0x01b8
        );
    }

    #[test]
    fn detects_changed_bytes(){
        let c=starter([40,40]);
        let mut p=SceneCameras{
            scene:"town01".into(),
            fingerprint:"test".into(),
            original:vec![c.raw],
            rows:vec![c]
        };

        assert!(!p.changed());

        p.rows[0].raw[10]=4;
        assert!(p.changed());
    }

    #[test]
    fn name_is_not_a_native_edit(){
        let c=starter([40,40]);
        let mut p=SceneCameras{
            scene:"town01".into(),
            fingerprint:"test".into(),
            original:vec![c.raw],
            rows:vec![c]
        };

        p.rows[0].name="My Room".into();
        assert!(!p.changed());
    }

    #[test]
    fn project_json_roundtrip()->Result<()> {
        let p=SceneCameras{
            scene:"town01".into(),
            fingerprint:"test".into(),
            original:vec![],
            rows:vec![starter([60,40])]
        };

        let data=serde_json::to_vec(&p)?;

        assert_eq!(
            serde_json::from_slice::<SceneCameras>(&data)?,
            p
        );

        Ok(())
    }
}