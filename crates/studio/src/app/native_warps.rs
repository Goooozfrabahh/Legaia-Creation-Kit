// LCK_R57_NATIVE_WARPS
// Native MAP kind-0 same-cell teleports only.
// Cross-cell MAN opcodes, scripts, story gates and arbitrary
// arrival facing are intentionally NOT claimed supported.

use anyhow::{Result,Context,ensure,bail};
use eframe::egui;
use legaia_engine_scene::scene::{Scene,ProtIndex};
use legaia_disc_patch::disc::DiscPatcher;
use serde::{Serialize,Deserialize};
use sha2::{Sha256,Digest};
use std::{path::Path,fs};

const REGION:usize=0x10000;
const BLOCK:usize=0x2000;
const GRID:usize=0x8000;
const DIM:usize=128;
const TELEPORT_BIT:u16=0x0200;

#[derive(Clone,Debug,PartialEq,Eq,Serialize,Deserialize)]
pub(super) struct Warp {
    pub source:[u8;2],
    pub arrival:[u8;2],
}

#[derive(Clone,Debug,PartialEq,Eq,Serialize,Deserialize)]
pub(super) struct SceneWarps {
    pub scene:String,
    pub original_hash:String,
    pub records:Vec<Warp>,
}

#[derive(Default)]
pub(super) struct Editor {
    pub open:bool,
    pub marker_overlay_visible:bool,
    pub marker_right_captured:bool,
    pub r621_context_pos:Option<egui::Pos2>,
    pub r621_cross_request:Option<[u8;2]>,
    pub r621_context_error:Option<String>,
    scene:String,
    original_hash:String,
    draft:Vec<Warp>,
    selected:usize,
    dragging:Option<(usize,bool)>,
    dirty:bool,
    error:Option<String>,
}

fn read_i16(data:&[u8],at:usize)->Result<i16>{
    let bytes=data.get(at..at+2)
        .context("MAP trigger header truncated")?;
    Ok(i16::from_le_bytes(bytes.try_into()?))
}

fn header(block:&[u8])->Result<(usize,usize)>{
    ensure!(block.len()>=BLOCK,"MAP trigger block is short");
    let off=read_i16(block,2)?;
    let count=read_i16(block,4)?;
    ensure!(off>=0 && count>=0,"Negative MAP warp offset/count");
    let (off,count)=(off as usize,count as usize);
    ensure!(off+count*4<=BLOCK,"MAP warp table exceeds block");
    ensure!(count==0 || off>=14,"Warp table overlaps trigger header");
    Ok((off,count))
}

fn parse(map:&[u8])->Result<Vec<Warp>>{
    let block=map.get(REGION..REGION+BLOCK)
        .context("Native MAP primary trigger region unavailable")?;
    let (off,count)=header(block)?;
    let mut out=Vec::with_capacity(count);
    for i in 0..count {
        let p=&block[off+i*4..off+i*4+4];
        out.push(Warp{
            source:[p[0],p[1]],
            arrival:[p[2],p[3]],
        });
    }
    Ok(out)
}

fn native_hash(map:&[u8])->Result<String>{
    let block=map.get(REGION..REGION+BLOCK)
        .context("MAP trigger block unavailable")?;
    let (off,count)=header(block)?;
    let mut hash=Sha256::new();
    hash.update(&block[2..6]);
    hash.update(&block[off..off+count*4]);
    Ok(format!("{:x}",hash.finalize()))
}

pub(super) fn source_map(
    index:&ProtIndex,
    name:&str
)->Result<Vec<u8>>{
    let scene=Scene::load(index,name)?;
    let entry=scene.field_map_index(index)
        .context("Scene has no native MAP")?;
    Ok(index.entry_bytes(entry)?.as_ref().clone())
}

// R6.0: unchanged saved warp snapshots are not native edits.
pub(super) fn has_effective_changes(
    index:&ProtIndex,
    edited:&SceneWarps
)->Result<bool>{
    let original=source_map(index,&edited.scene)?;

    ensure!(
        native_hash(&original)?==edited.original_hash,
        "Saved warp source differs from the opened retail disc"
    );

    Ok(parse(&original)?!=edited.records)
}
fn validate_records(records:&[Warp])->Result<()> {
    ensure!(records.len()<=i16::MAX as usize,
        "Native trigger count exceeds i16 capacity");

    let mut sources=std::collections::BTreeSet::new();
    for row in records {
        ensure!(
            row.source[0]<128 && row.source[1]<128,
            "A warp source must be inside the 128x128 MAP grid"
        );
        ensure!(sources.insert(row.source),
            "Two kind-0 warps cannot share one source tile");
    }
    Ok(())
}

// Repack primary kind-0 table without moving kind-1/2 records.
// Enlargement uses a verified zero-filled free area.
// Other MAP bytes and other trigger tables remain unchanged.

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
fn compile_map(map:&[u8],edited:&SceneWarps)->Result<Vec<u8>>{
    ensure!(map.len()>=REGION+BLOCK,"Native MAP too small");
    ensure!(
        native_hash(map)?==edited.original_hash,
        "MAP kind-0 source differs from project. Reload original IMG before exporting"
    );
    validate_records(&edited.records)?;

    let original=parse(map)?;
    let mut out=map.to_vec();

    let b=&map[REGION..REGION+BLOCK];
    let (old_off,old_count)=header(b)?;

    let bytes_len=edited.records.len()*4;

    let destination=if bytes_len<=old_count*4
        && old_off>=14
    {
        old_off
    }else{
        // R633: allocate past the protected region-table area.
        r633_free_trigger_slot(b,bytes_len)?
    };

    // Remove the old kind-0 payload, not the adjacent kind-1 data.
    if old_count>0 {
        out[REGION+old_off..REGION+old_off+old_count*4].fill(0);
    }

    for (i,row) in edited.records.iter().enumerate(){
        let at=REGION+destination+i*4;
        out[at..at+4].copy_from_slice(&[
            row.source[0],
            row.source[1],
            row.arrival[0],
            row.arrival[1]
        ]);
    }

    out[REGION+2..REGION+4]
        .copy_from_slice(&(destination as i16).to_le_bytes());
    out[REGION+4..REGION+6]
        .copy_from_slice(&(edited.records.len() as i16).to_le_bytes());

    // Derived fast-gate bits: the player dispatch requires 0x200
    // at every kind-0 source tile. Preserve descriptor and other
    // trigger bits. Never overwrite the existing 0x400 kind-1 bit.
    let old_positions=original.iter()
        .map(|r|r.source)
        .collect::<std::collections::BTreeSet<_>>();

    let new_positions=edited.records.iter()
        .map(|r|r.source)
        .collect::<std::collections::BTreeSet<_>>();

    for source in old_positions.difference(&new_positions) {
        let at=GRID+(
            source[1] as usize*DIM+source[0] as usize
        )*2;
        let value=u16::from_le_bytes(out[at..at+2].try_into()?);
        out[at..at+2]
            .copy_from_slice(&(value&!TELEPORT_BIT).to_le_bytes());
    }

    for source in &new_positions {
        let at=GRID+(
            source[1] as usize*DIM+source[0] as usize
        )*2;
        let value=u16::from_le_bytes(out[at..at+2].try_into()?);
        out[at..at+2]
            .copy_from_slice(&(value|TELEPORT_BIT).to_le_bytes());
    }

    ensure!(
        parse(&out)?==edited.records,
        "Native MAP warp serialization did not round-trip"
    );

    for i in 0..out.len() {
        if i<GRID || (i>=REGION+BLOCK) {
            ensure!(map[i]==out[i],
                "Unrelated native MAP byte changed at {i:#x}");
        }
    }

    Ok(out)
}

// Only patch an IMG that the ordinary exporter already created.
// Write a staged image, independently verify it, then replace the
// newly exported image. The original retail source is untouched.

pub(super) fn overlay_existing(
    target:&Path,
    index:&ProtIndex,
    edited:&SceneWarps
)->Result<String>{
    super::ps1_original_guard::ensure_existing_overlay(target)?;

    let source_scene=Scene::load(index,&edited.scene)?;
    let entry=source_scene.field_map_index(index)
        .context("Edited warp cell has no native MAP")? as usize;

    let source=fs::read(target)?;
    let length=source.len();
    let mut disc=DiscPatcher::open(source)?;

    let before=disc.read_entry_footprint(entry)?;
    let after=compile_map(&before,edited)?;

    ensure!(after!=before,
        "All authored warp values match the original; no IMG warp changes required");

    disc.patch_prot_entry(
        entry,GRID as u64,&after[GRID..REGION]
    )?;
    disc.patch_prot_entry(
        entry,REGION as u64,&after[REGION..REGION+BLOCK]
    )?;

    ensure!(
        disc.read_entry_footprint(entry)?==after,
        "Native MAP physical-sector readback mismatch"
    );

    let published=disc.into_image();
    ensure!(published.len()==length,
        "Native IMG size unexpectedly changed");

    let staged=target.with_extension("r57_warp_staging");
    let preserved=target.with_extension("r57_before_warps");

    ensure!(!staged.exists() && !preserved.exists(),
        "Previous temporary warp export files exist");

    {
        use std::io::Write;
        let mut f=fs::OpenOptions::new()
            .write(true).create_new(true).open(&staged)?;
        f.write_all(&published)?;
        f.sync_all()?;
    }

    let verified=(||->Result<()>{
        let test=DiscPatcher::open(fs::read(&staged)?)?;
        ensure!(test.read_entry_footprint(entry)?==after,
            "Staged IMG warp readback failed");
        Ok(())
    })();

    if let Err(error)=verified {
        let _=fs::remove_file(&staged);
        return Err(error);
    }

    fs::rename(target,&preserved)?;

    if let Err(error)=fs::rename(&staged,target) {
        let _=fs::rename(&preserved,target);
        let _=fs::remove_file(&staged);
        return Err(error.into());
    }

    let verified=(||->Result<()>{
        let image=fs::read(target)?;
        ensure!(image.len()==length,"Published IMG changed length");
        let test=DiscPatcher::open(image)?;
        ensure!(test.read_entry_footprint(entry)?==after,
            "Published IMG warp readback failed");
        Ok(())
    })();

    if let Err(error)=verified {
        let _=fs::remove_file(target);
        fs::rename(&preserved,target)
            .context("Restore previous exported IMG after failure")?;
        return Err(error);
    }

    fs::remove_file(&preserved)?;

    Ok(format!(
        "NATIVE MAP KIND-0 WARPS: {} records written to scene {} / PROT {}; source positions and destinations verified by IMG readback; gameplay testing required",
        edited.records.len(),edited.scene,entry
    ))
}

// A warp-only project may have no object or collision edits.
// This path creates a new output from the source before applying
// the native warp overlay. Existing files are never overwritten.

pub(super) fn publish_warp_only(
    source:&Path,
    destination:&Path,
    index:&ProtIndex,
    edited:&SceneWarps
)->Result<super::ps1_export::ExportResult>{
    super::ps1_original_guard::ensure_new_output(
        source,destination
    )?;

    ensure!(!destination.exists(),
        "Warp output already exists");

    fs::copy(source,destination)
        .context("Create warp-only IMG from retail source")?;

    let report=overlay_existing(destination,index,edited);

    match report {
        Ok(message)=>Ok(super::ps1_export::ExportResult{
            path:destination.to_path_buf(),
            patched:vec![message],
            skipped:vec![],
            imports:None,
        }),
        Err(error)=>{
            let _=fs::remove_file(destination);
            Err(error)
        }
    }
}

// Source records are loaded from the same indexed MAP that the
// regular editor already uses. Draft edits are not committed
// until the user clicks Apply to Project.

// LCK_V2082F_AUTOMATIC_WARP_COMMIT
//
// The warp dialog owns a working draft.
// Explicit Save Project or Export IMG validates and
// stages this draft without an extra Apply click.

pub(super) fn pending_snapshot(
    editor:&Editor,
    index:&ProtIndex,
    scene:&str
)->Result<Option<SceneWarps>> {
    if !editor.dirty || editor.scene!=scene {
        return Ok(None);
    }

    let source=source_map(index,scene)?;

    ensure!(
        native_hash(&source)?==editor.original_hash,
        "Pending warp draft belongs to a different native MAP revision"
    );

    validate_records(&editor.draft)?;

    Ok(Some(SceneWarps {
        scene:scene.to_owned(),
        original_hash:editor.original_hash.clone(),
        records:editor.draft.clone(),
    }))
}

pub(super) fn mark_committed(editor:&mut Editor){
    editor.dirty=false;
    editor.error=None;
}
pub(super) fn window(
    ctx:&egui::Context,
    editor:&mut Editor,
    index:&ProtIndex,
    scene:&str,
    saved:Option<&SceneWarps>
)->Option<SceneWarps>{
    if editor.scene!=scene {
        editor.scene=scene.to_owned();
        editor.selected=0;
        editor.dirty=false;
        editor.error=None;
        editor.dragging=None;

        match source_map(index,scene)
            .and_then(|map|Ok((native_hash(&map)?,parse(&map)?)))
        {
            Ok((hash,rows))=>{
                editor.original_hash=hash.clone();
                editor.draft=saved
                    .filter(|s|s.original_hash==hash)
                    .map(|s|s.records.clone())
                    .unwrap_or(rows);
            }
            Err(error)=>{
                editor.error=Some(error.to_string());
                editor.draft.clear();
            }
        }
    }

    if !editor.open {return None;}

    let mut output=None;
    let mut open=editor.open;

    egui::Window::new("Native MAP Warps - Same Cell")
        .open(&mut open)
        .resizable(true)
        .default_width(420.0)
        .show(ctx,|ui|{
            ui.strong(format!("Cell: {scene}"));
            ui.small(
                "MAP kind-0 teleports only. These can move the player within one cell."
            );
            ui.small(
                "Cross-cell doors and MAN scripted transitions are separate native features."
            );

            if let Some(error)=&editor.error {
                ui.colored_label(egui::Color32::RED,error);
                return;
            }

            ui.separator();

            egui::ScrollArea::vertical()
                .max_height(170.0)
                .show(ui,|ui|{
                    for (i,row) in editor.draft.iter().enumerate(){
                        if ui.selectable_label(
                            i==editor.selected,
                            format!(
                                "#{i}  ({},{}) -> ({},{})",
                                row.source[0],row.source[1],
                                row.arrival[0],row.arrival[1]
                            )
                        ).clicked(){
                            editor.selected=i;
                        }
                    }
                });

            ui.separator();

            if let Some(row)=editor.draft.get_mut(editor.selected){
                ui.strong("Selected Warp");
                ui.label("Source tile - where the player activates it");
                ui.horizontal(|ui|{
                    ui.label("X");
                    editor.dirty |= ui.add(
                        egui::DragValue::new(&mut row.source[0])
                            .range(0..=127)
                    ).changed();
                    ui.label("Z");
                    editor.dirty |= ui.add(
                        egui::DragValue::new(&mut row.source[1])
                            .range(0..=127)
                    ).changed();
                });

                ui.label("Arrival - native half-tile coordinates");
                ui.horizontal(|ui|{
                    ui.label("X");
                    editor.dirty |= ui.add(
                        egui::DragValue::new(&mut row.arrival[0])
                    ).changed();
                    ui.label("Z");
                    editor.dirty |= ui.add(
                        egui::DragValue::new(&mut row.arrival[1])
                    ).changed();
                });

                let wx=row.arrival[0] as i32*64+64;
                let wz=(row.arrival[1] as i32+1)*64;
                ui.small(format!(
                    "Native arrival world coordinates: X={wx}, Z={wz}"
                ));
            }

            ui.separator();

            ui.horizontal_wrapped(|ui|{
                if ui.button("New Warp").clicked(){
                    let occupied=editor.draft.iter()
                        .map(|r|r.source)
                        .collect::<std::collections::BTreeSet<_>>();

                    if let Some(source)=(0..128u8)
                        .flat_map(|z|(0..128u8).map(move |x|[x,z]))
                        .find(|p|!occupied.contains(p))
                    {
                        editor.draft.push(Warp{
                            source,
                            arrival:[source[0]*2,source[1]*2]
                        });
                        editor.selected=editor.draft.len()-1;
                        editor.dirty=true;
                    }
                }

                if ui.button("Duplicate").clicked(){
                    if let Some(mut row)=editor.draft
                        .get(editor.selected).cloned()
                    {
                        let occupied=editor.draft.iter()
                            .map(|r|r.source)
                            .collect::<std::collections::BTreeSet<_>>();

                        if let Some(source)=(0..128u8)
                            .flat_map(|z|(0..128u8).map(move |x|[x,z]))
                            .find(|p|!occupied.contains(p))
                        {
                            row.source=source;
                            editor.draft.push(row);
                            editor.selected=editor.draft.len()-1;
                            editor.dirty=true;
                        }
                    }
                }

                if ui.button("Delete Warp").clicked()
                    && !editor.draft.is_empty()
                {
                    editor.draft.remove(editor.selected);
                    editor.selected=editor.selected
                        .min(editor.draft.len().saturating_sub(1));
                    editor.dirty=true;
                }
            });

            ui.separator();

            if ui.button("Apply to Project").clicked(){
                match validate_records(&editor.draft){
                    Ok(())=>{
                        output=Some(SceneWarps{
                            scene:scene.to_owned(),
                            original_hash:editor.original_hash.clone(),
                            records:editor.draft.clone(),
                        });
                        editor.dirty=false;
                        editor.error=None;
                    }
                    Err(error)=>editor.error=Some(error.to_string()),
                }
            }

            if editor.dirty {
                ui.weak(
                    "Pending warp edits will be included automatically when you Save Project or Export IMG."
                );
            }

            ui.small(
                "Save Project (Ctrl+S) stores the edits. Export Native PS1 IMG compiles verified MAP changes."
            );
        });

    editor.open=open;
    output
}

fn warp_camera_ray(
    model:&crate::raster::Model,
    cam:crate::raster::Camera,
    pixel:[f32;2],
    px:[usize;2],
)->Option<([f32;3],[f32;3])>{
    if px[0]==0 || px[1]==0 {
        return None;
    }

    let fov=45f32.to_radians();

    let f=(px[1] as f32*0.5)
        /(fov*0.5).tan();

    let distance=model.radius
        /(fov*0.5).sin()
        *cam.zoom.max(0.0000001);

    if !f.is_finite() || !distance.is_finite() {
        return None;
    }

    let (sy,cy)=cam.yaw.sin_cos();
    let (sp,cp)=cam.pitch.sin_cos();

    let to_world=|q:[f32;3]|{
        let y=q[1]*cp+q[2]*sp;
        let z1=-q[1]*sp+q[2]*cp;

        [
            q[0]*cy-z1*sy,
            y,
            q[0]*sy+z1*cy
        ]
    };

    let backwards=to_world([0.0,0.0,distance]);

    let target=[0,1,2].map(|i|
        model.center[i]+cam.target_offset[i]
    );

    let origin=[0,1,2].map(|i|
        target[i]-backwards[i]
    );

    let ray=to_world([
        (pixel[0]-px[0] as f32*0.5)/f,
        (pixel[1]-px[1] as f32*0.5)/f,
        1.0
    ]);

    let magnitude=(
        ray[0]*ray[0]+
        ray[1]*ray[1]+
        ray[2]*ray[2]
    ).sqrt();

    if !magnitude.is_finite() || magnitude<=1e-9 {
        return None;
    }

    Some((
        origin,
        ray.map(|value|value/magnitude)
    ))
}
fn projected(
    p:[f32;3],
    rect:egui::Rect,
    model:&crate::raster::Model,
    cam:crate::raster::Camera,
    mirror:bool
)->Option<egui::Pos2>{
    let (sy,cy)=cam.yaw.sin_cos();
    let (sp,cp)=cam.pitch.sin_cos();
    let fov=45.0f32.to_radians();
    let f=(rect.height()*0.5)/(fov*0.5).tan();
    let dist=model.radius/(fov*0.5).sin()*cam.zoom.max(1e-7);
    let v=[0,1,2].map(|i|
        p[i]-model.center[i]-cam.target_offset[i]
    );
    let x1=v[0]*cy+v[2]*sy;
    let z1=-v[0]*sy+v[2]*cy;
    let y2=v[1]*cp-z1*sp;
    let z2=v[1]*sp+z1*cp+dist;
    if z2<=0.001 || !z2.is_finite(){return None;}
    let mut x=rect.width()*0.5+f*x1/z2;
    let y=rect.height()*0.5+f*y2/z2;
    if mirror {x=rect.width()-x;}
    Some(egui::pos2(rect.left()+x,rect.top()+y))
}

fn floor_y(
    hf:Option<&legaia_scene_formats::field_objects::WalkHeightfield>,
    tile:[u8;2]
)->f32{
    if let Some(hf)=hf {
        for p in hf.positions.chunks_exact(4){
            let x=(p[0][0]/128.0).round() as i32;
            let z=(p[0][2]/128.0).round() as i32;
            if [x,z]==[tile[0] as i32,tile[1] as i32] {
                return p.iter().map(|q|q[1]).sum::<f32>()/4.0;
            }
        }
    }
    0.0
}

fn marker_world(
    row:&Warp,
    arrival:bool,
    hf:Option<&legaia_scene_formats::field_objects::WalkHeightfield>
)->[f32;3]{
    let (x,z,tile)=if arrival{
        (
            row.arrival[0] as f32*64.0+64.0,
            (row.arrival[1] as f32+1.0)*64.0,
            [row.arrival[0]>>1,row.arrival[1]>>1]
        )
    }else{
        (
            row.source[0] as f32*128.0+64.0,
            row.source[1] as f32*128.0+64.0,
            row.source
        )
    };
    [x,floor_y(hf,tile),z]
}

pub(super) fn interact(
    ui:&egui::Ui,
    rect:egui::Rect,
    model:&crate::raster::Model,
    cam:crate::raster::Camera,
    mirror:bool,
    hf:Option<&legaia_scene_formats::field_objects::WalkHeightfield>,
    editor:&mut Editor,
)->bool{
    if !editor.marker_overlay_visible || editor.error.is_some(){
        editor.dragging=None;
        editor.marker_right_captured=false;
        return false;
    }

    let pointer=ui.input(|i|i.pointer.interact_pos());
    let pressed=ui.input(|i|i.pointer.primary_pressed());
    let down=ui.input(|i|i.pointer.primary_down());
    let released=ui.input(|i|i.pointer.primary_released());
    // V2.0.8.3: original source and arrival circles.
    // Right-click selects the corresponding native warp,
    // opening the existing Native MAP Warp Properties.
    editor.marker_right_captured=false;

    if ui.input(|i|i.pointer.secondary_pressed()) {
        let topmost=pointer
            .and_then(|p|ui.ctx().layer_id_at(p))
            .is_none_or(|layer|layer==ui.layer_id());

        if topmost {
            if let Some(pos)=pointer.filter(|p|
                rect.contains(*p)
            ){
                let mut best_distance=400.0f32;
                let mut selected=None;

                for (i,row) in editor.draft.iter().enumerate(){
                    for arrival in [false,true] {
                        let world=marker_world(row,arrival,hf);

                        if let Some(marker)=projected(
                            world,rect,model,cam,mirror
                        ){
                            let distance=(pos-marker).length_sq();

                            if distance<best_distance {
                                best_distance=distance;
                                selected=Some(i);
                            }
                        }
                    }
                }

                if let Some(index)=selected {
                    editor.selected=index;
                    // LCK_R621_WARP_CONTEXT_MENU
                    editor.r621_context_pos=Some(pos);
                    editor.r621_context_error=None;
                    editor.dragging=None;
                    editor.marker_right_captured=true;

                    ui.ctx().request_repaint();
                }
            }
        }
    }


    if pressed {
        editor.dragging=None;
        if let Some(pointer)=pointer.filter(|p|rect.contains(*p)){
            let mut nearest=None;
            let mut distance=576.0f32;

            for (i,row) in editor.draft.iter().enumerate(){
                for arrival in [false,true]{
                    let world=marker_world(row,arrival,hf);
                    if let Some(point)=projected(
                        world,rect,model,cam,mirror
                    ){
                        let d=(pointer-point).length_sq();
                        if d<distance {
                            nearest=Some((i,arrival));
                            distance=d;
                        }
                    }
                }
            }

            if let Some(hit)=nearest{
                editor.selected=hit.0;
                editor.dragging=Some(hit);
            }
        }
    }

    if let (Some((i,arrival)),Some(pointer))=(editor.dragging,pointer){
        if down && rect.contains(pointer) {
            let dimensions=super::placement_drop::viewport_pixels(
                rect,ui.ctx().pixels_per_point(),[900,680]
            );
            let pixel=super::placement_drop::viewport_pixel(
                rect,pointer,dimensions,mirror
            );

            if let Some((eye,ray))=
                warp_camera_ray(
                    model,cam,pixel,dimensions
                )
            {
                if let Some(row)=editor.draft.get_mut(i){
                    let y=marker_world(row,arrival,hf)[1];
                    if ray[1].abs()>1e-6 {
                        let t=(y-eye[1])/ray[1];
                        if t>0.0 && t.is_finite(){
                            let x=eye[0]+ray[0]*t;
                            let z=eye[2]+ray[2]*t;

                            if arrival{
                                row.arrival=[
                                    ((x-64.0)/64.0).round()
                                        .clamp(0.0,255.0) as u8,
                                    (z/64.0-1.0).round()
                                        .clamp(0.0,255.0) as u8
                                ];
                            }else{
                                row.source=[
                                    ((x-64.0)/128.0).round()
                                        .clamp(0.0,127.0) as u8,
                                    ((z-64.0)/128.0).round()
                                        .clamp(0.0,127.0) as u8
                                ];
                            }
                            editor.dirty=true;
                        }
                    }
                }
            }
        }
    }

    if released {editor.dragging=None;}
    editor.dragging.is_some() || editor.marker_right_captured
}

pub(super) fn paint(
    ui:&egui::Ui,
    rect:egui::Rect,
    model:&crate::raster::Model,
    cam:crate::raster::Camera,
    mirror:bool,
    hf:Option<&legaia_scene_formats::field_objects::WalkHeightfield>,
    editor:&Editor
){
    if !editor.marker_overlay_visible || editor.error.is_some(){
        return;
    }

    let painter=ui.painter();
    for (i,row) in editor.draft.iter().enumerate(){
        let source=projected(
            marker_world(row,false,hf),
            rect,model,cam,mirror
        );
        let destination=projected(
            marker_world(row,true,hf),
            rect,model,cam,mirror
        );

        if let (Some(a),Some(b))=(source,destination){
            painter.line_segment(
                [a,b],
                egui::Stroke::new(
                    1.0,egui::Color32::LIGHT_BLUE
                )
            );
        }

        for (arrival,p,color) in [
            (false,source,egui::Color32::YELLOW),
            (true,destination,egui::Color32::LIGHT_GREEN)
        ]{
            if let Some(pos)=p.filter(|p|rect.contains(*p)){
                painter.circle_filled(
                    pos,if i==editor.selected {14.0}else{10.0},color
                );

                if i==editor.selected {
                    painter.text(
                        pos+egui::vec2(9.0,-9.0),
                        egui::Align2::LEFT_CENTER,
                        if arrival{
                            format!("Warp {i} arrival")
                        }else{
                            format!("Warp {i} source")
                        },
                        egui::FontId::proportional(12.0),
                        ui.visuals().text_color()
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod lck_v2083_warp_view_tests {
    use super::*;

    #[test]
    fn marker_overlay_does_not_require_window(){
        let editor=Editor{
            open:false,
            marker_overlay_visible:true,
            ..Default::default()
        };

        assert!(!editor.open);
        assert!(editor.marker_overlay_visible);
    }

    #[test]
    fn original_source_and_arrival_coordinates(){
        let row=Warp{
            source:[10,20],
            arrival:[33,41],
        };

        let source=marker_world(&row,false,None);
        let arrival=marker_world(&row,true,None);

        assert_eq!(source[0],1344.0);
        assert_eq!(source[2],2624.0);

        assert_eq!(arrival[0],2176.0);
        assert_eq!(arrival[2],2688.0);

        assert_ne!(source,arrival);
    }
}

// LCK_R621_WARP_CONTEXT_MENU
pub(super) fn r621_context_menu(
    ui:&egui::Ui,
    editor:&mut Editor
){
    let Some(position)=editor.r621_context_pos else {
        return;
    };

    let mut properties=false;
    let mut create=false;
    let mut close=false;

    let popup=egui::Window::new("Warp marker actions")
        .id(egui::Id::new("lck_r621_local_warp_context"))
        .title_bar(false)
        .resizable(false)
        .fixed_pos(position)
        .show(ui.ctx(),|menu|{
            menu.strong(format!(
                "Local Warp #{}",editor.selected
            ));

            if menu.button("Properties...").clicked(){
                properties=true;
            }

            menu.separator();

            if menu.button("New Cross-Cell Warp").clicked(){
                create=true;
            }

            if let Some(error)=&editor.r621_context_error {
                menu.separator();
                menu.colored_label(
                    egui::Color32::RED,error
                );
            }

            if menu.button("Cancel").clicked(){
                close=true;
            }
        });

    if properties {
        editor.open=true;
        editor.r621_context_error=None;
        close=true;
    }

    if create {
        if let Some(row)=editor.draft.get(editor.selected) {
            editor.r621_cross_request=Some(row.source);
            editor.r621_context_error=None;
            close=true;
        }else{
            editor.r621_context_error=Some(
                "Selected warp record is unavailable".into()
            );
        }
    }

    if ui.input(|i|i.key_pressed(egui::Key::Escape)){
        close=true;
    }

    if let Some(response)=popup {
        if ui.input(|i|i.pointer.primary_pressed()) &&
            ui.input(|i|i.pointer.interact_pos())
                .is_some_and(|p|
                    !response.response.rect.contains(p)
                ){
            close=true;
        }
    }

    if close {
        editor.r621_context_pos=None;
        editor.r621_context_error=None;
    }
}#[cfg(test)]
mod lck_r57_native_warp_tests {
    use super::*;

    fn fixture()->Vec<u8>{
        let mut map=vec![0u8;REGION+BLOCK];
        map[REGION+2..REGION+4]
            .copy_from_slice(&32i16.to_le_bytes());
        map[REGION+4..REGION+6]
            .copy_from_slice(&1i16.to_le_bytes());
        map[REGION+32..REGION+36]
            .copy_from_slice(&[10,20,40,60]);
        map
    }

    #[test]
    fn relocating_native_warp_updates_source_and_destination(){
        let map=fixture();
        let edit=SceneWarps{
            scene:"town01".into(),
            original_hash:native_hash(&map).unwrap(),
            records:vec![Warp{
                source:[12,24],arrival:[44,66]
            }]
        };
        let after=compile_map(&map,&edit).unwrap();
        assert_eq!(parse(&after).unwrap(),edit.records);
        let source=GRID+(24*DIM+12)*2;
        assert_ne!(
            u16::from_le_bytes(after[source..source+2].try_into().unwrap())
                &TELEPORT_BIT,0
        );
        assert_eq!(&map[..GRID],&after[..GRID]);
    }

    #[test]
    fn deletion_removes_native_trigger_record(){
        let map=fixture();
        let edit=SceneWarps{
            scene:"town01".into(),
            original_hash:native_hash(&map).unwrap(),
            records:vec![]
        };
        let after=compile_map(&map,&edit).unwrap();
        assert!(parse(&after).unwrap().is_empty());
    }

    #[test]
    fn creation_allocates_native_table_and_preserves_other_bytes(){
        let map=fixture();
        let edit=SceneWarps{
            scene:"town01".into(),
            original_hash:native_hash(&map).unwrap(),
            records:vec![
                Warp{source:[10,20],arrival:[40,60]},
                Warp{source:[11,20],arrival:[42,62]}
            ]
        };
        let after=compile_map(&map,&edit).unwrap();
        assert_eq!(parse(&after).unwrap(),edit.records);
        assert_eq!(&map[..GRID],&after[..GRID]);
    }

    #[test]
    fn duplicate_trigger_and_stale_source_are_rejected(){
        let map=fixture();
        let mut edit=SceneWarps{
            scene:"town01".into(),
            original_hash:native_hash(&map).unwrap(),
            records:vec![
                Warp{source:[10,20],arrival:[40,60]},
                Warp{source:[10,20],arrival:[42,62]}
            ]
        };
        assert!(compile_map(&map,&edit).is_err());
        edit.records.pop();
        edit.original_hash="stale".into();
        assert!(compile_map(&map,&edit).is_err());
    }
}

// LCK_R633_TRIGGER_REGION_ISOLATION
#[cfg(test)]
mod lck_r633_warp_storage_tests {
    use super::*;

    #[test]
    fn new_warp_table_does_not_occupy_region_padding()
        ->Result<()>{
        let mut map=vec![0u8;REGION+BLOCK];

        let off=REGION;

        map[off+2..off+4]
            .copy_from_slice(&18i16.to_le_bytes());

        map[off+4..off+6]
            .copy_from_slice(&1i16.to_le_bytes());

        map[off+18..off+22]
            .copy_from_slice(&[2,3,10,11]);

        map[off+14..off+16]
            .copy_from_slice(&0x484i16.to_le_bytes());

        map[off+16..off+18]
            .copy_from_slice(&14i16.to_le_bytes());

        let region_row=off+0x484+5*8;

        map[region_row..region_row+8]
            .copy_from_slice(&[9,60,45,0,0,0,0,0]);

        let before=map[off+0x484..off+0x800]
            .to_vec();

        let edited=SceneWarps{
            scene:"town01".into(),
            original_hash:native_hash(&map)?,
            records:vec![
                Warp{
                    source:[2,3],
                    arrival:[10,11]
                },
                Warp{
                    source:[97,52],
                    arrival:[35,0]
                }
            ]
        };

        let output=compile_map(&map,&edited)?;

        let new_off=read_i16(
            &output[REGION..REGION+BLOCK],2
        )? as usize;

        assert!(new_off>=0x800);

        assert_eq!(
            &output[off+0x484..off+0x800],
            &before
        );

        assert_eq!(parse(&output)?,edited.records);

        Ok(())
    }
}
