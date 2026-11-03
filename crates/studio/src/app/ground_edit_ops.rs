//! LCK Stage G2: non-destructive editor-visible native MAP ground-piece edits.
//! Neither native MAP sampling nor IMG export consumes this layer.
use std::collections::{BTreeMap,BTreeSet};
use serde::{Serialize,Deserialize};
use eframe::egui::{self,Ui};
use legaia_scene_formats::field_objects::WalkHeightfield;
use crate::raster::Model;

pub(super) const ORIGINAL_BASE:u32=0xE000_0000;
pub(super) const COPY_BASE:u32=0xF000_0000;
pub(super) type Cell=[u8;2];
#[derive(Clone,Copy,Debug,PartialEq,Serialize,Deserialize)]
pub(super) struct Transform { pub offset:[f32;3],pub turns:i32, #[serde(default="unit_scale")] pub scale:[f32;3] }
fn unit_scale()->[f32;3]{[1.;3]}
impl Default for Transform {fn default()->Self{Self{offset:[0.;3],turns:0,scale:unit_scale()}}}
#[derive(Clone,Debug,PartialEq,Serialize,Deserialize)]
pub(super) struct CopyPiece {pub uid:u64,pub source:Cell,pub transform:Transform}
#[derive(Clone,Debug,Default,PartialEq,Serialize,Deserialize)]
struct FileData {version:u8, originals:Vec<(Cell,Transform)>, copies:Vec<CopyPiece>, #[serde(default)] deleted:Vec<Cell>}
#[derive(Clone,Copy,Debug,PartialEq,Eq,PartialOrd,Ord)]
pub(super) enum Key {Native(Cell),Copy(u64)}
#[derive(Clone)]
struct Drag {pointer:egui::Pos2, bases:Vec<(Key,Transform)>}
// LCK_G4_DEFER_GROUND_SELECTION
#[derive(Clone,Copy)]
struct PendingPress {
    hit:Option<Key>,
    shift:bool,
    ctrl:bool,
    was_selected:bool,
}
#[derive(Clone,Default)]
pub(super) struct State {
    pub originals:BTreeMap<Cell,Transform>,
    pub deleted:BTreeSet<Cell>,
    pub copies:Vec<CopyPiece>,
    pub selected_copies:BTreeSet<u64>,
    pub active_copy:Option<u64>,
    pub scene:String,
    drag:Option<Drag>,
    pending_press:Option<PendingPress>,
    drag_moved:bool,
    next_uid:u64,
}
impl State {
    pub fn clear_for_scene(&mut self,name:&str) {
        if self.scene!=name { *self=Self::default();self.scene=name.into(); }
    }
    pub fn has_edits(&self)->bool {
        !self.originals.is_empty() || !self.copies.is_empty() || !self.deleted.is_empty()
    }
    pub fn can_export_deletions(&self)->bool {
        !self.deleted.is_empty() && self.originals.is_empty() && self.copies.is_empty()
    }
    pub fn transform(&self,key:Key)->Option<Transform> {
        match key {Key::Native(c)=>Some(*self.originals.get(&c).unwrap_or(&Transform::default())),
                   Key::Copy(uid)=>self.copies.iter().find(|p|p.uid==uid).map(|p|p.transform)}
    }
    pub fn set_transform(&mut self,key:Key,transform:Transform) {
        match key {Key::Native(c)=>{if transform==Transform::default(){self.originals.remove(&c);}else{self.originals.insert(c,transform);}},
                   Key::Copy(uid)=>{if let Some(p)=self.copies.iter_mut().find(|p|p.uid==uid){p.transform=transform;}}}
    }
    pub fn active(&self,original_active:Option<Cell>)->Option<Key> {
        self.active_copy.map(Key::Copy).or(original_active.map(Key::Native))
    }
    pub fn all_selected(&self,originals:&BTreeSet<Cell>)->Vec<Key> {
        originals.iter().copied().map(Key::Native).chain(self.selected_copies.iter().copied().map(Key::Copy)).collect()
    }
    pub fn clear_drag(&mut self){
        self.drag=None;
        self.pending_press=None;
        self.drag_moved=false;
    }
}

// W1: Compile only floor changes exactly representable in
// the existing 128-unit native MAP grid.

// LCK_W2A_RASTERIZED_FLOOR
// Compile scaled rectangles into independent native MAP cells.
// Native texture is repeated per cell, not stretched.
// Multi-level and foreign resources still need runtime extensions.
pub(super) fn native_128_plan(
    state:&State
)->Result<(BTreeSet<Cell>,Vec<(Cell,Cell)>),String>{

    fn cells(source:Cell,t:Transform)->Result<Vec<Cell>,String>{
        if t.offset[1].abs()>0.01
            || (t.scale[1]-1.).abs()>0.0001 {
            return Err(
                "Elevated/vertical-scale floors require extended runtime".into()
            );
        }

        if t.scale[0]<=0. || t.scale[2]<=0.
            || !t.scale.iter().all(|v|v.is_finite())
            || !t.offset.iter().all(|v|v.is_finite()) {
            return Err("Invalid native floor transform".into());
        }

        let odd=t.turns.rem_euclid(2)!=0;
        let sx=if odd{t.scale[2]}else{t.scale[0]};
        let sz=if odd{t.scale[0]}else{t.scale[2]};

        let cx=source[0] as f32*128.+64.+t.offset[0];
        let cz=source[1] as f32*128.+64.+t.offset[2];

        let lo=[
            (cx-64.*sx)/128.,
            (cz-64.*sz)/128.
        ];
        let hi=[
            (cx+64.*sx)/128.,
            (cz+64.*sz)/128.
        ];

        for v in lo.into_iter().chain(hi) {
            if (v-v.round()).abs()>0.0001 {
                return Err(
                    "Floor edges must align to 128-unit native grid".into()
                );
            }
        }

        let xa=lo[0].round() as i32;
        let xb=hi[0].round() as i32;
        let za=lo[1].round() as i32;
        let zb=hi[1].round() as i32;

        if xa<0 || za<0 || xb>127 || zb>127
            || xa>=xb || za>=zb {
            return Err(
                "Native floor rectangle outside MAP bounds".into()
            );
        }

        let count=((xb-xa)*(zb-za)) as usize;

        if count>256 {
            return Err("Per-piece limit: 256 native tiles".into());
        }

        let mut result=Vec::with_capacity(count);

        for z in za..zb {
            for x in xa..xb {
                result.push([x as u8,z as u8]);
            }
        }

        Ok(result)
    }

    let mut removals=state.deleted.clone();
    let mut stamps=Vec::new();
    let mut claimed=BTreeSet::new();

    for (&source,&transform) in &state.originals {
        if state.deleted.contains(&source) {
            return Err("Cannot transform a deleted floor".into());
        }

        let footprint=cells(source,transform)?;

        if !footprint.contains(&source) {
            removals.insert(source);
        }

        for dest in footprint {
            if !claimed.insert(dest) {
                return Err(
                    format!("Overlapping edited floors at {:?}",dest)
                );
            }

            if dest!=source {
                stamps.push((source,dest));
            }
        }
    }

    for copy in &state.copies {
        for dest in cells(copy.source,copy.transform)? {
            if !claimed.insert(dest) {
                return Err(
                    format!(
                        "Multiple floors at {:?} require layered runtime",
                        dest
                    )
                );
            }

            stamps.push((copy.source,dest));
        }
    }

    if removals.is_empty() && stamps.is_empty() {
        return Err(
            "No native footprint changes to export".into()
        );
    }

    Ok((removals,stamps))
}


// LCK_W2B_UNRESTRICTED
// Experimental native-grid approximation, not exact polygons.
fn w2b_cells(source:Cell,t:Transform,notes:&mut Vec<String>)->Vec<Cell>{
    let odd=t.turns.rem_euclid(2)!=0;
    let sx=if odd{t.scale[2]}else{t.scale[0]};
    let sz=if odd{t.scale[0]}else{t.scale[2]};

    if !sx.is_finite()||!sz.is_finite()||sx<=0.||sz<=0.||
        !t.offset.iter().all(|v|v.is_finite()) {
        notes.push(format!("SKIPPED invalid transform {:?}",source));
        return Vec::new();
    }

    if t.offset[1].abs()>0.01||(t.scale[1]-1.).abs()>0.0001 {
        notes.push(format!("APPROX {:?}: Y transform not encoded",source));
    }
    if t.turns.rem_euclid(4)!=0 {
        notes.push(format!("APPROX {:?}: texture rotation not encoded",source));
    }

    let cx=source[0] as f32*128.+64.+t.offset[0];
    let cz=source[1] as f32*128.+64.+t.offset[2];

    let lo=[(cx-64.*sx)/128.,(cz-64.*sz)/128.];
    let hi=[(cx+64.*sx)/128.,(cz+64.*sz)/128.];

    if lo.into_iter().chain(hi).any(|v|(v-v.round()).abs()>0.0001){
        notes.push(format!("APPROX {:?}: fractional edges rasterized",source));
    }

    let (xa,xb)=(lo[0].floor() as i32,hi[0].ceil() as i32);
    let (za,zb)=(lo[1].floor() as i32,hi[1].ceil() as i32);

    if xa<0||za<0||xb>127||zb>127 {
        notes.push(format!("CLIPPED {:?}: outside native grid",source));
    }

    let mut result=Vec::new();
    for z in za.max(0)..zb.min(127){
        for x in xa.max(0)..xb.min(127){
            result.push([x as u8,z as u8]);
        }
    }
    result
}

pub(super) fn w2b_plan(state:&State)
    ->Result<(BTreeSet<Cell>,Vec<(Cell,Cell)>,Vec<String>),String>{

    let mut removed=state.deleted.clone();
    let mut stamps=Vec::new();
    let mut notes=Vec::new();
    let mut owner=BTreeMap::new();

    for (&source,&transform) in &state.originals {
        let footprint=w2b_cells(source,transform,&mut notes);
        if footprint.is_empty(){
            notes.push(format!("SKIPPED original {:?}: outside grid",source));
            continue;
        }
        removed.insert(source);
        for target in footprint {
            if let Some(previous)=owner.insert(target,source){
                notes.push(format!("OVERLAP {:?}: {:?} replaced by {:?}",
                    target,previous,source));
            }
            stamps.push((source,target));
        }
    }

    for copy in &state.copies {
        let footprint=w2b_cells(copy.source,copy.transform,&mut notes);
        if footprint.is_empty(){
            notes.push(format!("SKIPPED copy {}: outside grid",copy.uid));
            continue;
        }
        for target in footprint {
            if let Some(previous)=owner.insert(target,copy.source){
                notes.push(format!("OVERLAP {:?}: {:?} replaced by copy {}",
                    target,previous,copy.uid));
            }
            stamps.push((copy.source,target));
        }
    }

    if removed.is_empty()&&stamps.is_empty(){
        return Err("No edited floor cells inside writable MAP".into());
    }
    Ok((removed,stamps,notes))
}

fn quad_for(hf:&WalkHeightfield,cell:Cell)->Option<usize>{
    hf.positions.chunks_exact(4).enumerate().find(|(_,p)|
        ((p[0][0]/128.).round() as i32)==cell[0] as i32 &&
        ((p[0][2]/128.).round() as i32)==cell[1] as i32
    ).map(|(i,_)|i)
}
fn is_valid(hf:&WalkHeightfield,c:Cell)->bool{quad_for(hf,c).is_some()}
fn transform_point(mut p:[f32;3],center:[f32;3],t:Transform)->[f32;3]{
    let x=(p[0]-center[0])*t.scale[0];let z=(p[2]-center[2])*t.scale[2];
    let (rx,rz)=match t.turns.rem_euclid(4){0=>(x,z),1=>(-z,x),2=>(-x,-z),_=>(z,-x)};
    p[0]=center[0]+rx+t.offset[0];
    p[1]=center[1]+(p[1]-center[1])*t.scale[1]+t.offset[1];
    p[2]=center[2]+rz+t.offset[2];p
}
fn center_quad(base:&Model,q:usize)->[f32;3]{
    let mut c=[0.;3];for i in 0..4 {for k in 0..3 {c[k]+=base.positions[q*4+i][k]/4.;}}c
}
/// Copies the render stream. Original PS1 MAP and AssembledScene remain untouched.
/// Scene draw order is original ground, native objects, then editor floor copies.
pub(super) fn compose(base:&Model,hf:&WalkHeightfield,state:&State,interactive:bool)->Model{
    let mut m=base.clone();
    let native_indices=legaia_engine_scene::field_ground::render_indices(hf);
    let count=hf.positions.len()/4;
    for q in 0..count {
        if (q+1)*4>base.positions.len(){break;}
        if interactive {for j in 0..4 {m.object_ids[q*4+j]=ORIGINAL_BASE+q as u32;}}
        let Some(p)=hf.positions.get(q*4) else{continue;};
        let cell=[(p[0]/128.).round() as u8,(p[2]/128.).round() as u8];
        if let Some(&transform)=state.originals.get(&cell){
            let center=center_quad(base,q);
            for j in 0..4 {m.positions[q*4+j]=transform_point(base.positions[q*4+j],center,transform);}
        }
    }
    if !state.deleted.is_empty() && m.indices.len() >= hf.indices.len() {
        let original_len=hf.indices.len();
        let mut retained=Vec::with_capacity(m.indices.len());
        for (q,triangles) in m.indices[..original_len].chunks_exact(6).enumerate() {
            let p=&hf.positions[q*4];
            let cell=[
                (p[0]/128.).round() as u8,
                (p[2]/128.).round() as u8
            ];
            if !state.deleted.contains(&cell) {
                retained.extend_from_slice(triangles);
            }
        }
        retained.extend_from_slice(&m.indices[original_len..]);
        m.indices=retained;
    }
    for (copy_idx,copy) in state.copies.iter().enumerate(){
        let Some(q)=quad_for(hf,copy.source) else{continue;};
        if (q+1)*4>base.positions.len(){continue;}
        let begin=m.positions.len() as u32;
        let center=center_quad(base,q);
        for j in 0..4 {
            let i=q*4+j;
            m.positions.push(transform_point(base.positions[i],center,copy.transform));
            m.rest.push(base.rest.get(i).copied().unwrap_or(base.positions[i]));
            m.object_ids.push(if interactive {COPY_BASE+copy_idx as u32}else{u32::MAX});
            m.uvs.push(base.uvs[i]);
            m.cba_tsb.push(base.cba_tsb[i]);
            m.modulation.push(base.modulation[i]);
            m.fill.push(base.fill[i]);
            m.textured.push(base.textured[i]);
        }
        for &idx in native_indices.get(q*6..q*6+6).unwrap_or(&[]){
            m.indices.push(begin+(idx as usize-q*4) as u32);
        }
    }
    // Preserve the native world camera scale when the user moves a tile.
    m
}
pub(super) fn from_pick(id:u32,hf:&WalkHeightfield,state:&State)->Option<Key>{
    if id>=COPY_BASE {
        return state.copies.get((id-COPY_BASE) as usize).map(|p|Key::Copy(p.uid));
    }
    if id>=ORIGINAL_BASE {
        let q=(id-ORIGINAL_BASE) as usize;
        let p=hf.positions.get(q*4)?;
        return Some(Key::Native([(p[0]/128.).round() as u8,(p[2]/128.).round() as u8]));
    }
    None
}
pub(super) fn highlighted_ids(state:&State,hf:&WalkHeightfield,originals:&BTreeSet<Cell>)->Vec<u32>{
    let mut out=Vec::new();
    for (q,p) in hf.positions.chunks_exact(4).enumerate(){
        let cell=[(p[0][0]/128.).round() as u8,(p[0][2]/128.).round() as u8];
        if originals.contains(&cell) && !state.deleted.contains(&cell){out.push(ORIGINAL_BASE+q as u32);}
    }
    for (i,p) in state.copies.iter().enumerate(){
        if state.selected_copies.contains(&p.uid){out.push(COPY_BASE+i as u32);}
    }
    out
}

// First press: reserve movement ONLY for previously selected tiles.
pub(super) fn press(
    state:&mut State,
    originals:&mut BTreeSet<Cell>,
    _active:&mut Option<Cell>,
    hit:Option<Key>,
    pos:egui::Pos2,
    shift:bool,
    ctrl:bool
){
    let was_selected=hit.is_some_and(|key|match key {
        Key::Native(c)=>originals.contains(&c),
        Key::Copy(uid)=>state.selected_copies.contains(&uid),
    });

    state.pending_press=Some(PendingPress{
        hit,shift,ctrl,was_selected
    });
    state.drag_moved=false;

    state.drag=if was_selected {
        Some(Drag{
            pointer:pos,
            bases:state.all_selected(originals)
                .into_iter()
                .filter_map(|key|
                    state.transform(key).map(|t|(key,t))
                ).collect(),
        })
    }else{
        None
    };
}

// Mouse release: select the original floor hit, not the floor
// currently underneath a camera that may have panned.
pub(super) fn release(
    state:&mut State,
    originals:&mut BTreeSet<Cell>,
    active:&mut Option<Cell>
)->bool{
    let Some(p)=state.pending_press.take() else {
        state.drag=None;
        return false;
    };

    state.drag=None;
    let moved=std::mem::replace(&mut state.drag_moved,false);

    if moved || (p.was_selected && !p.shift && !p.ctrl){
        return false;
    }

    if !p.shift && !p.ctrl {
        originals.clear();
        state.selected_copies.clear();
    }

    match p.hit {
        Some(Key::Native(c))=>{
            if p.ctrl && originals.contains(&c){
                originals.remove(&c);
            }else{
                originals.insert(c);
            }

            *active=originals.contains(&c)
                .then_some(c)
                .or_else(||originals.iter().next_back().copied());

            state.active_copy=None;
        },

        Some(Key::Copy(uid))=>{
            if p.ctrl && state.selected_copies.contains(&uid){
                state.selected_copies.remove(&uid);
            }else{
                state.selected_copies.insert(uid);
            }

            state.active_copy=state.selected_copies
                .contains(&uid)
                .then_some(uid)
                .or_else(||state.selected_copies.iter()
                    .next_back().copied());

            *active=None;
        },

        None=>{
            if !p.shift && !p.ctrl {
                originals.clear();
                *active=None;
                state.selected_copies.clear();
                state.active_copy=None;
            }
        }
    }
    true
}
pub(super) fn drag(state:&mut State,cam:crate::raster::Camera,
    model:&Model,rect:egui::Rect,at:egui::Pos2,ctrl:bool,mirror:bool)->bool{
    let Some(session)=state.drag.clone() else{return false;};
    let delta=at-session.pointer;
    if delta.length_sq()<=36.0{return false;}
    let fov=45f32.to_radians()*0.5;
    let dist=model.radius/fov.sin()*cam.zoom.max(0.0000001);
    let step=(2.0*dist*fov.tan()/rect.height().max(1.0)).max(0.35);
    let (sy,cy)=cam.yaw.sin_cos();let (sp,_cp)=cam.pitch.sin_cos();
    let x=if mirror {-delta.x}else{delta.x};
    let (shift_x,shift_y,shift_z)=if ctrl {(0.0,-delta.y*step,0.0)}
        else {((x*cy+delta.y*sy*sp)*step,0.0,(x*sy-delta.y*cy*sp)*step)};
    for (key,base) in session.bases {
        let mut updated=base;
        updated.offset[0]+=shift_x;updated.offset[1]+=shift_y;updated.offset[2]+=shift_z;
        state.set_transform(key,updated);
    }
    state.drag_moved=true;
    true
}
pub(super) fn rotate(state:&mut State,selected:&BTreeSet<Cell>,delta:i32)->bool{
    let keys=state.all_selected(selected);
    if keys.is_empty(){return false;}
    for key in keys {if let Some(mut t)=state.transform(key){t.turns=(t.turns+delta).rem_euclid(4);state.set_transform(key,t);}}
    true
}
pub(super) fn duplicate(state:&mut State,selected:&mut BTreeSet<Cell>,active:&mut Option<Cell>,hf:&WalkHeightfield)->usize{
    let keys=state.all_selected(selected);
    let mut added=Vec::new();
    for key in keys {
        if state.copies.len()+added.len()>=4096 {break;}
        let source=match key {Key::Native(c)=>c,Key::Copy(uid)=>match state.copies.iter().find(|p|p.uid==uid){Some(p)=>p.source,None=>continue}};
        if !is_valid(hf,source){continue;}
        let Some(mut transform)=state.transform(key) else{continue;};
        transform.offset[0]+=128.0;
        state.next_uid+=1;
        added.push(CopyPiece{uid:state.next_uid,source,transform});
    }
    let count=added.len();
    if count!=0 {
        selected.clear();*active=None;state.selected_copies.clear();
        for p in &added {state.selected_copies.insert(p.uid);state.active_copy=Some(p.uid);}
        state.copies.extend(added);
    }
    count
}
// LCK_G3_NATIVE_FLOOR_DELETE
pub(super) fn delete_selected(
    state:&mut State,
    originals:&mut BTreeSet<Cell>,
    active:&mut Option<Cell>,
)->usize {
    let count=originals.len()+state.selected_copies.len();

    for &cell in originals.iter() {
        state.originals.remove(&cell);
        state.deleted.insert(cell);
    }

    state.copies.retain(|copy|
        !state.selected_copies.contains(&copy.uid)
    );

    originals.clear();
    *active=None;
    state.selected_copies.clear();
    state.active_copy=None;
    state.clear_drag();
    count
}

pub(super) fn controls(ui:&mut Ui,state:&mut State,originals:&mut BTreeSet<Cell>,active:&mut Option<Cell>,hf:&WalkHeightfield)->bool{
    let mut changed=false;
    ui.horizontal_wrapped(|ui|{
        if ui.button("Delete floor (Del)").clicked(){
            changed|=delete_selected(state,originals,active)>0;
        }
        if ui.button("Duplicate floor (Ctrl+D)").clicked(){
            changed|=duplicate(state,originals,active,hf)>0;
        }
        if ui.button("Rotate -90 (Q)").clicked(){changed|=rotate(state,originals,-1);}
        if ui.button("Rotate +90 (E)").clicked(){changed|=rotate(state,originals,1);}
        ui.small(format!("{} copies",state.copies.len()));
    });
    if let Some(key)=state.active(*active){
        if let Some(mut t)=state.transform(key){
            ui.horizontal_wrapped(|ui|{
                ui.label("Offset X/Y/Z:");
                for axis in 0..3 {changed|=ui.add(egui::DragValue::new(&mut t.offset[axis]).speed(1.)).changed();}
                ui.label(format!("Yaw: {}°",t.turns.rem_euclid(4)*90));
                ui.label("Scale X/Y/Z:");
                for axis in 0..3 {changed|=ui.add(egui::DragValue::new(&mut t.scale[axis]).speed(0.01).range(0.05..=20.0)).changed();}
            });
            if changed {state.set_transform(key,t);}
        }
    }
    ui.small("Floor deletions can be exported as native MAP edits. Movement, rotation and duplication remain editor-only.");
    changed
}
pub(super) fn copies_hierarchy(ui:&mut Ui,state:&mut State,originals:&mut BTreeSet<Cell>,active:&mut Option<Cell>,filter:&str)->bool{
    let mut changed=false;
    egui::CollapsingHeader::new(format!("New Tiles ({})",state.copies.len()))
    .id_salt("lck_g2_floor_copies").default_open(true).show(ui,|ui|{
        for p in &state.copies {
            let label=format!("Copy #{} | source [{:03},{:03}] | yaw {}°",p.uid,p.source[0],p.source[1],p.transform.turns.rem_euclid(4)*90);
            if !filter.is_empty()&&!label.to_lowercase().contains(filter){continue;}
            let response=ui.selectable_label(state.selected_copies.contains(&p.uid),label);
            if response.clicked(){
                let mods=ui.input(|i|i.modifiers);
                let uid=p.uid;
                if !mods.shift&&!mods.ctrl {originals.clear();state.selected_copies.clear();}
                if mods.ctrl&&state.selected_copies.contains(&uid){state.selected_copies.remove(&uid);}else{state.selected_copies.insert(uid);}
                state.active_copy=state.selected_copies.contains(&uid).then_some(uid);
                *active=None;changed=true;
            }
        }
    });
    changed
}
pub(super) fn serialize(state:&State)->String {
    let data=FileData{version:1,originals:state.originals.iter().map(|(&c,&t)|(c,t)).collect(),copies:state.copies.clone(),deleted:state.deleted.iter().copied().collect()};
    format!("GROUND_EDIT\t{}\n",serde_json::to_string(&data).expect("Ground edit data is serializable"))
}
pub(super) fn strip(source:&str)->String {
    source.lines().filter(|line|!line.starts_with("GROUND_EDIT\t"))
        .map(|line|format!("{line}\n")).collect()
}
pub(super) fn parse(source:&str,hf:Option<&WalkHeightfield>,scene:&str)->Result<State,String>{
    let lines=source.lines().filter_map(|line|line.strip_prefix("GROUND_EDIT\t")).collect::<Vec<_>>();
    if lines.len()>1 {return Err("Duplicate GROUND_EDIT records".into());}
    let mut state=State::default();state.scene=scene.into();
    let Some(value)=lines.first() else{return Ok(state);};
    let data:FileData=serde_json::from_str(value).map_err(|e|format!("Invalid ground edit record: {e}"))?;
    if data.version!=1{return Err("Unsupported ground edit version".into());}
    let hf=hf.ok_or("Scene has ground edit records but no original MAP ground".to_string())?;
    let mut ids=BTreeSet::new();
    let mut originals=BTreeMap::new();
    for &(cell,t) in &data.originals {
        if !is_valid(hf,cell)||!valid_transform(t)||originals.insert(cell,t).is_some(){return Err("Invalid original ground edit".into());}
    }
    for p in &data.copies {
        if p.uid==0||!ids.insert(p.uid)||!is_valid(hf,p.source)||!valid_transform(p.transform){return Err("Invalid floor duplicate".into());}
    }
    if data.copies.len()>4096{return Err("Too many floor duplicates".into());}
    state.next_uid=data.copies.iter().map(|p|p.uid).max().unwrap_or(0);
    for cell in data.deleted {
        let identity=0x1_0000_0000u64
            + u64::from(cell[1])*128
            + u64::from(cell[0]);

        if !ids.insert(identity)
            || !is_valid(hf,cell)
            || originals.contains_key(&cell) {
            return Err("Invalid or duplicate deleted native tile".into());
        }
        state.deleted.insert(cell);
    }
    state.originals=originals;
    state.copies=data.copies;
    Ok(state)
}
fn valid_transform(t:Transform)->bool {
    t.offset.iter().all(|n|n.is_finite()&&n.abs()<=65536.) && t.scale.iter().all(|n|n.is_finite()&&(0.05..=20.).contains(n)) && (-10000..10000).contains(&t.turns)
}
#[cfg(test)]mod tests {
 use super::*;
 #[test]fn independent_copy_and_transform(){
    let mut state=State::default();
    let c=[4,7];state.set_transform(Key::Native(c),Transform{offset:[128.,0.,0.],turns:1,scale:unit_scale()});
    assert_eq!(state.transform(Key::Native(c)).unwrap().turns,1);
    let copy=CopyPiece{uid:1,source:c,transform:state.transform(Key::Native(c)).unwrap()};
    state.copies.push(copy);
    state.set_transform(Key::Copy(1),Transform{offset:[256.,0.,0.],turns:2,scale:unit_scale()});
    assert_eq!(state.transform(Key::Native(c)).unwrap().offset[0],128.);
    assert_eq!(state.transform(Key::Copy(1)).unwrap().turns,2);
 }
 #[test]fn persistence_roundtrip(){
    let mut hf=WalkHeightfield::default();
    hf.positions=vec![[0.,0.,0.],[128.,0.,0.],[0.,0.,128.],[128.,0.,128.]];
    let mut state=State::default();state.scene="town01".into();
    let mut selected=BTreeSet::new();selected.insert([0,0]);
    let mut active=Some([0,0]);
    assert_eq!(duplicate(&mut state,&mut selected,&mut active,&hf),1);
    let raw=format!("LCK_SCENE_V1\ttown01\n{}",serialize(&state));
    let loaded=parse(&raw,Some(&hf),"town01").unwrap();
    assert_eq!(loaded.copies,state.copies);
    assert_eq!(loaded.copies[0].transform.offset[0],128.0);
    assert!(!strip(&raw).contains("GROUND_EDIT"));
 }
}

pub(super) fn frame_active(view:&mut super::viewport::Viewport,model:&Model,hf:&WalkHeightfield,
    state:&State,active_native:Option<Cell>)->bool{
    let key=state.active(active_native);
    let Some(key)=key else{return false;};
    let source=match key {Key::Native(c)=>c,Key::Copy(uid)=>match state.copies.iter().find(|p|p.uid==uid){Some(p)=>p.source,None=>return false}};
    let Some(quad)=quad_for(hf,source) else{return false;};
    let Some(t)=state.transform(key) else{return false;};
    let center=center_quad(model,quad);
    let center=[center[0]+t.offset[0],center[1]+t.offset[1],center[2]+t.offset[2]];
    let radius=96.0;
    view.frame_instance(model,center,radius,[0.,0.,0.,1.],[0.;3]);
    view.version=view.version.wrapping_add(1);
    true
}

#[cfg(test)]
mod g3_native_delete_tests {
    use super::*;

    #[test]
    fn deleted_ground_roundtrip() {
     let mut hf=WalkHeightfield::default();
     hf.positions=vec![
         [0.,0.,0.],[128.,0.,0.],
         [0.,0.,128.],[128.,0.,128.]
     ];

     let mut state=State::default();
     state.scene="town01".into();

     let mut selected=BTreeSet::from([[0,0]]);
     let mut active=Some([0,0]);

     assert_eq!(
         delete_selected(&mut state,&mut selected,&mut active),1
     );

     assert!(state.deleted.contains(&[0,0]));
     assert!(state.can_export_deletions());

     let saved=format!(
         "LCK_SCENE_V1\ttown01\n{}",serialize(&state)
     );

     let loaded=parse(&saved,Some(&hf),"town01").unwrap();

     assert!(loaded.deleted.contains(&[0,0]));
 }

}

#[cfg(test)]
mod g4_pan_tests {
    use super::*;

    #[test]
    fn first_press_pans_second_press_moves(){
        let mut state=State::default();
        let mut selected=BTreeSet::new();
        let mut active=None;
        let key=Key::Native([4,7]);

        press(&mut state,&mut selected,&mut active,
            Some(key),egui::pos2(0.,0.),false,false);

        assert!(selected.is_empty());
        assert!(state.drag.is_none());

        assert!(release(&mut state,&mut selected,&mut active));
        assert!(selected.contains(&[4,7]));

        press(&mut state,&mut selected,&mut active,
            Some(key),egui::pos2(0.,0.),false,false);

        assert!(state.drag.is_some());
        assert!(!release(&mut state,&mut selected,&mut active));
        assert_eq!(active,Some([4,7]));
    }
}

#[cfg(test)] mod w1_ground_plan_tests {
    use super::*;

    #[test]
    fn legacy_ground_transform_defaults_to_unit_scale(){
        let old=r#"{"offset":[128.0,0.0,0.0],"turns":0}"#;
        let loaded:Transform=serde_json::from_str(old).unwrap();
        assert_eq!(loaded.scale,[1.,1.,1.]);
    }

    #[test]
    fn aligned_ground_copy_plans_native_destination(){
        let mut state=State::default();
        let mut transform=Transform::default();
        transform.offset[0]=128.;

        state.copies.push(CopyPiece{
            uid:1,source:[10,11],transform
        });

        let (removed,stamps)=native_128_plan(&state).unwrap();

        assert!(removed.is_empty());
        assert_eq!(stamps,vec![([10,11],[11,11])]);

        state.copies[0].transform.scale[0]=2.;
        assert!(native_128_plan(&state).is_err());
    }
}

pub(super) fn native_resized_sources(
    state:&State
)->BTreeSet<Cell>{
    let mut result=BTreeSet::new();

    for (&source,t) in &state.originals {
        if (t.scale[0]-1.).abs()>0.0001
            || (t.scale[2]-1.).abs()>0.0001 {
            result.insert(source);
        }
    }

    for p in &state.copies {
        if (p.transform.scale[0]-1.).abs()>0.0001
            || (p.transform.scale[2]-1.).abs()>0.0001 {
            result.insert(p.source);
        }
    }

    result
}

#[cfg(test)]
mod w2a_native_footprint_tests {
    use super::*;

    #[test]
    fn doubled_copy_yields_four_native_cells(){
        let mut state=State::default();
        let mut t=Transform::default();

        t.scale=[2.,1.,2.];
        t.offset=[576.,0.,64.];

        state.copies.push(CopyPiece{
            uid:1,source:[10,11],transform:t
        });

        let (removed,stamps)=native_128_plan(&state).unwrap();

        assert!(removed.is_empty());
        assert_eq!(stamps,vec![
            ([10,11],[14,11]),
            ([10,11],[15,11]),
            ([10,11],[14,12]),
            ([10,11],[15,12])
        ]);
    }

    #[test]
    fn quarter_rotation_swaps_rectangular_footprint(){
        let mut state=State::default();
        let mut t=Transform::default();

        t.turns=1;
        t.scale=[2.,1.,1.];
        t.offset=[512.,0.,320.];

        state.copies.push(CopyPiece{
            uid:1,source:[10,11],transform:t
        });

        let (_,stamps)=native_128_plan(&state).unwrap();

        assert_eq!(
            stamps,
            vec![
                ([10,11],[14,13]),
                ([10,11],[14,14])
            ]
        );
    }

    #[test]
    fn rejects_unaligned_and_overlapping_floors(){
        let mut state=State::default();
        let mut t=Transform::default();

        t.scale[0]=1.25;

        state.copies.push(CopyPiece{
            uid:1,source:[10,11],transform:t
        });

        assert!(native_128_plan(&state).is_err());

        state.copies[0].transform=Transform::default();
        state.copies.push(CopyPiece{
            uid:2,
            source:[10,11],
            transform:Transform::default()
        });

        assert!(native_128_plan(&state).is_err());
    }
}

#[cfg(test)]
mod w2b_raster_tests {
    use super::*;

    #[test]
    fn fractional_floor_position_is_not_blocked(){
        let mut state=State::default();
        let mut t=Transform::default();
        t.offset=[27.25,0.,-25.58];

        state.copies.push(CopyPiece{
            uid:1,source:[19,26],transform:t
        });

        let (deleted,writes,notes)=w2b_plan(&state).unwrap();

        assert!(deleted.is_empty());
        assert_eq!(writes.len(),4);
        assert!(notes.iter().any(|n|n.contains("fractional")));
    }
}
