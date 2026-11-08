
//! C1 experimental native MAP wall authoring. No ownership guesses.
use std::{collections::{BTreeMap,BTreeSet},fs,path::Path};
use anyhow::{Result,Context,ensure};
use eframe::egui::{self,Pos2,Rect,Ui,Color32,Stroke};
use legaia_disc_patch::disc::DiscPatcher;
use legaia_engine_scene::scene::{Scene,ProtIndex};
use crate::raster::Model;
use super::{collision_wire::{CollisionGrid,Projector,wall_offset},viewport::Viewport};
type Cell=(i32,i32);

/// An explicit operation result. Duplication never remaps its donor group.
#[derive(Clone,Debug)]
pub(super) enum EditEvent {
    Duplicated(Vec<(Cell,Cell)>),
    Moved(Vec<(Cell,Cell)>),
    Rotated(Vec<(Cell,Cell)>),
}

/// A 90-degree rotation about the selected boxes' shared bounding-box center.
/// Collision is a 64-unit grid. Where an exact quarter-turn would end
/// between cells, snap the entire result by the same half-cell offset.
pub(super) fn rotation_pairs(
    selected:&BTreeSet<Cell>, turns:i32
)->std::result::Result<Vec<(Cell,Cell)>,String>{
    if selected.is_empty(){return Err("Select collision boxes first".into());}
    let min_x=selected.iter().map(|c|c.0).min().unwrap();
    let max_x=selected.iter().map(|c|c.0).max().unwrap();
    let min_z=selected.iter().map(|c|c.1).min().unwrap();
    let max_z=selected.iter().map(|c|c.1).max().unwrap();
    let (cx,cz)=(min_x+max_x,min_z+max_z); // double-precision pivot in cell units
    let mut pairs=Vec::new();
    let mut destinations=BTreeSet::new();
    for &(x,z) in selected {
        let (dx,dz)=(2*x-cx,2*z-cz);
        let (nx,nz)=match turns.rem_euclid(4){
            0=>(dx,dz),
            1=>(-dz,dx),
            2=>(-dx,-dz),
            _=>(dz,-dx),
        };
        let (tx,tz)=(cx+nx,cz+nz);
        // The parity offset is constant across a rotation of one selection:
        // snapping these half-cell coordinates keeps relative box spacing.
        let target=(tx.div_euclid(2)+tx.rem_euclid(2),
                    tz.div_euclid(2)+tz.rem_euclid(2));
        if wall_offset(target.0,target.1).is_none() || !destinations.insert(target) {
            return Err("Rotated collision boxes overlap or leave the native grid".into());
        }
        pairs.push(((x,z),target));
    }
    Ok(pairs)
}

#[derive(Clone)]
struct Press {cell:Cell,start:Pos2,grab:[f32;3],selected_before:bool,group:BTreeSet<Cell>,baseline:BTreeMap<Cell,bool>,height:BTreeMap<Cell,i32>,changed:bool}
#[derive(Clone,Default)]
pub(super) struct State {
    scene:String,
    walls:BTreeMap<Cell,bool>,
    heights:BTreeMap<Cell,i32>,
    selected:BTreeSet<Cell>,
    press:Option<Press>,
    last_edit:Option<EditEvent>,
    pub pointer_on_wall:bool,
    add_x:i32,add_z:i32,
}
#[derive(serde::Serialize,serde::Deserialize)]
struct Document {version:u32,scene:String,walls:Vec<(i32,i32,bool)>,heights:Vec<(i32,i32,i32)>}
impl State {
    pub(super) fn native_edit_snapshot(
        &self
    )->(Vec<(i32,i32,bool)>,Vec<(i32,i32,i32)>) {
        let walls=self.walls.iter()
            .map(|(&(x,z),&occupied)|(x,z,occupied))
            .collect();
        let heights=self.heights.iter()
            .filter(|(_,offset)| **offset!=0)
            .map(|(&(x,z),&offset)|(x,z,offset))
            .collect();
        (walls,heights)
    }

    /// Persist the authored collision layer alongside the scene/project overlay.
    /// Group membership itself is stored in EDITOR_GROUP records.
    pub fn embedded_record(&self,scene:&str)->String{
        let data=Document{version:1,scene:scene.to_owned(),walls:self.walls.iter().map(|(&(x,z),&v)|(x,z,v)).collect(),heights:self.heights.iter().map(|(&(x,z),&y)|(x,z,y)).collect()};
        format!("COLLISION_EDIT\t{}\n",serde_json::to_string(&data).expect("Collision state serializable"))
    }
    pub fn from_embedded_record(text:&str,scene:&str)->std::result::Result<Self,String>{
        let lines=text.lines().filter_map(|line|line.strip_prefix("COLLISION_EDIT\t")).collect::<Vec<_>>();
        if lines.len()>1{return Err("Duplicate collision edit overlays".into());}
        let mut state=State::default();state.scene=scene.into();
        let Some(json)=lines.first() else{return Ok(state);};
        let data:Document=serde_json::from_str(json).map_err(|e|format!("Invalid collision edit record: {e}"))?;
        if data.version!=1||data.scene!=scene{return Err("Collision edit record belongs to another scene".into());}
        for (x,z,v) in data.walls {
            if wall_offset(x,z).is_none()||state.walls.insert((x,z),v).is_some(){return Err("Invalid or duplicate collision coordinate".into());}
        }
        for (x,z,y) in data.heights {
            if wall_offset(x,z).is_none()||state.heights.insert((x,z),y).is_some(){return Err("Invalid or duplicate collision height".into());}
        }
        Ok(state)
    }
    pub fn has_edits(&self)->bool { !self.walls.is_empty() || self.heights.values().any(|&y|y!=0) }
    pub fn has_selection(&self)->bool { !self.selected.is_empty() }
    pub fn selected_cells(&self)->BTreeSet<(i32,i32)>{self.selected.clone()}
    pub fn include_selected(&mut self, cells:&BTreeSet<(i32,i32)>){self.selected.extend(cells.iter().copied());}
    pub fn replace_selected(&mut self,cells:BTreeSet<(i32,i32)>){self.selected=cells;}
    pub(super) fn take_edit(&mut self)->Option<EditEvent>{self.last_edit.take()}

    /// Rebuild a rotation from the mouse-down snapshot, so no drifting or
    /// accumulation occurs during a held RMB gesture.
    pub(super) fn rotate_from_snapshot(
        &mut self,grid:&CollisionGrid,snapshot:&State,
        selected:&BTreeSet<Cell>,turns:i32
    )->std::result::Result<Vec<(Cell,Cell)>,String>{
        let pairs=rotation_pairs(selected,turns)?;
        let old=selected.clone();
        for &(from,to) in &pairs {
            if !snapshot.occupied(grid,from) {
                return Err("A selected collision box is missing".into());
            }
            if !old.contains(&to) && snapshot.occupied(grid,to) {
                return Err("Collision rotation would overwrite an unrelated box".into());
            }
        }
        let mut next=snapshot.clone();
        let heights=pairs.iter().map(|(from,_)|
            snapshot.heights.get(from).copied()
        ).collect::<Vec<_>>();
        for &(from,_) in &pairs {
            next.set(grid,from,false);
            next.heights.remove(&from);
        }
        next.selected.clear();
        for ((_,to),height) in pairs.iter().zip(heights) {
            next.set(grid,*to,true);
            if let Some(value)=height {next.heights.insert(*to,value);}
            next.selected.insert(*to);
        }
        next.last_edit=None;
        *self=next;
        Ok(pairs)
    }
    /// Reflect grid occupancy as one transaction. A wall cell is its own mirror
    /// when reflected individually; only an assembly mirror changes coordinates.
    pub fn mirror_cells(&mut self,grid:&CollisionGrid,pairs:&[((i32,i32),(i32,i32))])->std::result::Result<(),String>{
        use std::collections::BTreeSet;
        let old=pairs.iter().map(|p|p.0).collect::<BTreeSet<_>>();
        let new=pairs.iter().map(|p|p.1).collect::<BTreeSet<_>>();
        if new.len()!=pairs.len()||new.iter().any(|&(x,z)|wall_offset(x,z).is_none()){
            return Err("Mirrored collision box would overlap or leave native wall grid".into());
        }
        let mut heights=Vec::new();
        for &(from,_) in pairs {
            if !self.occupied(grid,from){return Err("Selected collision box no longer exists".into());}
            heights.push(self.heights.get(&from).copied());
        }
        for &cell in &old {self.set(grid,cell,false);self.heights.remove(&cell);}
        for (&(_,to),height) in pairs.iter().zip(heights){
            self.set(grid,to,true);
            if let Some(y)=height {self.heights.insert(to,y);}
        }
        self.selected=new;
        Ok(())
    }

    pub fn clear_for_scene(&mut self,scene:&str){ if self.scene!=scene {*self=Self{scene:scene.into(),..Self::default()};} }
    fn occupied(&self,grid:&CollisionGrid,c:Cell)->bool {self.walls.get(&c).copied().unwrap_or_else(||grid.native_wall(c.0,c.1))}
    fn set(&mut self,grid:&CollisionGrid,c:Cell,v:bool){if grid.native_wall(c.0,c.1)==v{self.walls.remove(&c);}else{self.walls.insert(c,v);}}
    fn pick(&self,ui:&Ui,rect:Rect,view:&Viewport,model:&Model,grid:&CollisionGrid,pos:Pos2)->Option<Cell>{
        let proj=Projector::new(ui,rect,view,model);
        let mut candidate=None;
        let mut best=22.0_f32.powi(2);
        for z in 0..256_i32 {for x in 0..256_i32 {
            let c=(x,z);
            if !self.occupied(grid,c){continue;}
            let wx=x as f32*64.0+32.0;let wz=(z-2) as f32*64.0+32.0;
            let y=grid.ground_y(wx,wz)+54.0+*self.heights.get(&c).unwrap_or(&0) as f32;
            if let Some(p)=proj.project([wx,y,wz]){
                let d=(p-pos).length_sq();if d<best{best=d;candidate=Some(c);}
            }
        }} candidate
    }
    pub(super) fn context_hit(
        &self,ui:&Ui,rect:Rect,view:&Viewport,
        model:&Model,grid:&CollisionGrid,pos:Pos2
    )->Option<Cell>{
        self.pick(ui,rect,view,model,grid,pos)
    }

    pub(super) fn context_select(&mut self,cell:Cell){
        // Opening a popup on one member must never collapse a selected group.
        if !self.selected.contains(&cell) {
            self.selected.clear();
            self.selected.insert(cell);
        }
    }

    pub fn begin(&mut self,ui:&Ui,rect:Rect,view:&Viewport,model:&Model,grid:Option<&CollisionGrid>,scene:&str)->bool{
        self.clear_for_scene(scene);
        let (pressed,down,point)=ui.input(|i|(i.pointer.primary_pressed(),i.pointer.primary_down(),i.pointer.interact_pos()));
        if !down && !pressed {self.pointer_on_wall=false;return false;}
        if pressed {
            self.press=None;
            self.pointer_on_wall=false;
            if let (Some(grid),Some(pos))=(grid,point.filter(|p|rect.contains(*p))){
                if let Some(hit)=self.pick(ui,rect,view,model,grid,pos){
                    self.pointer_on_wall=true;
                    let selected_before=self.selected.contains(&hit);
                    let wx=hit.0 as f32*64.0+32.0;
                    let wz=(hit.1-2) as f32*64.0+32.0;
                    let y=grid.ground_y(wx,wz)+54.0
                        +*self.heights.get(&hit).unwrap_or(&0) as f32;
                    self.press=Some(Press{
                        cell:hit,start:pos,grab:[wx,y,wz],
                        selected_before,group:self.selected.clone(),
                        baseline:self.walls.clone(),
                        height:self.heights.clone(),changed:false
                    });
                }
            }
        }
        self.press.as_ref().is_some_and(|p|p.selected_before)
    }
    fn shifted(&mut self,grid:&CollisionGrid,group:&BTreeSet<Cell>,dx:i32,dz:i32)->bool{
        let original:Vec<Cell>=group.iter().copied().collect();
        if original.iter().any(|c|wall_offset(c.0+dx,c.1+dz).is_none()){return false;}
        for &c in &original {self.set(grid,c,false);}
        self.selected.clear();
        for c in original {
            let next=(c.0+dx,c.1+dz);
            self.set(grid,next,true);
            if let Some(height)=self.heights.remove(&c){self.heights.insert(next,height);}
            self.selected.insert(next);
        }
        true
    }
    pub fn finish(&mut self,ui:&Ui,rect:Rect,view:&Viewport,model:&Model,grid:&CollisionGrid,snap:Option<f32>){
        self.last_edit=None;
        let (down,released,point,ctrl,shift)=ui.input(|i|(i.pointer.primary_down(),i.pointer.primary_released(),i.pointer.interact_pos(),i.modifiers.ctrl,i.modifiers.shift));
        if let (Some(press),Some(pos))=(self.press.clone(),point){
            if down && press.selected_before && (pos-press.start).length_sq()>36.0 {
                if ctrl {
                    let raw=super::drag_anchor::vertical_delta(
                        ui,rect,model,view.cam,
                        press.start,pos,press.grab,view.world_mirrored
                    );
                    let delta=super::drag_anchor::snapped(raw,snap)
                        .round() as i32;
                    self.heights=press.height.clone();
                    for &c in &press.group {self.heights.insert(c,press.height.get(&c).copied().unwrap_or(0)+delta);}
                }else{
                    let delta=super::drag_anchor::horizontal_delta(
                        ui,rect,model,view.cam,
                        press.start,pos,press.grab,view.world_mirrored
                    );
                    let dx=super::drag_anchor::collision_cells(delta[0],snap);
                    let dz=super::drag_anchor::collision_cells(delta[2],snap);
                    let before_selection=self.selected.clone();
                    self.walls=press.baseline.clone();self.heights=press.height.clone();
                    if self.shifted(grid,&press.group,dx,dz)
                        && before_selection!=self.selected {
                        // Emit even when dx/dz return to zero, so the
                        // original group coordinates are restored on reversal.
                        self.last_edit=Some(EditEvent::Moved(press.group.iter().map(|&(x,z)|
                            ((x,z),(x+dx,z+dz))
                        ).collect()));
                    }
                }
                if let Some(p)=self.press.as_mut(){p.changed=true;}
            }
        }
        if released {
            if let Some(p)=self.press.take(){
                if !p.changed && !view.world_pan_dragged && point.is_some_and(|x|(x-p.start).length_sq()<=36.) {
                    if !ctrl && !shift {self.selected.clear();}
                    if ctrl && self.selected.contains(&p.cell){self.selected.remove(&p.cell);} else {self.selected.insert(p.cell);}
                }
            }
            self.pointer_on_wall=false;
        }
        if !ui.ctx().wants_keyboard_input() && self.has_selection(){
            if ui.input_mut(|i|i.consume_key(egui::Modifiers::NONE,egui::Key::Delete)){
                for c in self.selected.clone(){self.set(grid,c,false);}self.selected.clear();
            }
            if ui.input_mut(|i|i.consume_shortcut(&egui::KeyboardShortcut::new(egui::Modifiers::COMMAND,egui::Key::D))){
                let donors=self.selected.clone();
                // An occupied cell cannot hold two separately addressable boxes.
                // Find a vacant translation for the *whole* selection instead of
                // overwriting an original or constructing a partial group.
                let pairs=(1..256_i32)
                    .flat_map(|n|[(n,0),(-n,0),(0,n),(0,-n)])
                    .find_map(|(dx,dz)|{
                    let pairs=donors.iter().copied()
                        .map(|c|(c,(c.0+dx,c.1+dz))).collect::<Vec<_>>();
                    if pairs.iter().all(|&(_,to)|wall_offset(to.0,to.1).is_some()
                        && !self.occupied(grid,to)) {Some(pairs)}else{None}
                });
                if let Some(pairs)=pairs {
                    let heights=pairs.iter().map(|(from,_)|
                        self.heights.get(from).copied()
                    ).collect::<Vec<_>>();
                    self.selected.clear();
                    for ((_,to),height) in pairs.iter().zip(heights) {
                        self.set(grid,*to,true);
                        if let Some(y)=height {self.heights.insert(*to,y);}
                        self.selected.insert(*to);
                    }
                    self.last_edit=Some(EditEvent::Duplicated(pairs));
                }
            }
            for (key,sign) in [(egui::Key::Q,-1),(egui::Key::E,1)]{
                if ui.input_mut(|i|i.consume_key(egui::Modifiers::NONE,key)) {
                    let snapshot=self.clone();
                    let selection=self.selected.clone();
                    if let Ok(pairs)=self.rotate_from_snapshot(grid,&snapshot,&selection,sign) {
                        if pairs.iter().any(|(from,to)|from!=to) {
                            self.last_edit=Some(EditEvent::Rotated(pairs));
                        }
                    }
                }
            }
        }
        let painter=ui.painter().with_clip_rect(rect);
        let proj=Projector::new(ui,rect,view,model);
        for &c in &self.selected{
            if !self.occupied(grid,c){continue;}
            let x0=c.0 as f32*64.;
            let z0=(c.1-2) as f32*64.;
            let y0=grid.ground_y(x0+32.,z0+32.)+self.heights.get(&c).copied().unwrap_or(0) as f32;
            let y1=y0+108.0;

            let corners=[
                [x0,y0,z0],
                [x0+64.0,y0,z0],
                [x0+64.0,y0,z0+64.0],
                [x0,y0,z0+64.0],
                [x0,y1,z0],
                [x0+64.0,y1,z0],
                [x0+64.0,y1,z0+64.0],
                [x0,y1,z0+64.0],
            ];
            let pts=corners.map(|point|proj.project(point));
            if pts.iter().all(|p| p.is_some()){
                let p=pts.map(|q| q.unwrap());
                for (a,b) in [
                    (0usize,1usize),(1,2),(2,3),(3,0),
                    (4,5),(5,6),(6,7),(7,4),
                    (0,4),(1,5),(2,6),(3,7),
                ]{
                    painter.line_segment([p[a],p[b]],Stroke::new(3.0,Color32::YELLOW));
                }
            }
        }
        // Cyan outline for user-added cells; deletions are omitted from the composite overlay.
        for (&c,&v) in &self.walls{
            if !v || grid.native_wall(c.0,c.1){continue;}
            let x=c.0 as f32*64.+32.;let z=(c.1-2) as f32*64.+32.;let y=grid.ground_y(x,z)+54.;
            if let Some(p)=proj.project([x,y,z]){painter.circle_stroke(p,7.,Stroke::new(2.,Color32::from_rgb(55,225,240)));}
        }
    }
    pub fn composite(&self,grid:&CollisionGrid)->CollisionGrid {grid.with_wall_overrides(&self.walls)}
    pub fn controls(&mut self,ui:&mut Ui,grid:Option<&CollisionGrid>){
        ui.horizontal_wrapped(|ui|{
            ui.label(format!("Collision edit | {} selected | {} wall edits",self.selected.len(),self.walls.len()));
            ui.small("Click select | drag selected move | Ctrl+drag Y (preview only) | Ctrl+D copy | Del erase | Q/E rotate | selected cells now highlight as full boxes");
            if let Some(g)=grid {
                if ui.button("Add box at X/Z").clicked(){let c=(self.add_x,self.add_z);if wall_offset(c.0,c.1).is_some(){self.set(g,c,true);self.selected.clear();self.selected.insert(c);}}
                ui.add(egui::DragValue::new(&mut self.add_x).range(0..=255).prefix("X "));
                ui.add(egui::DragValue::new(&mut self.add_z).range(0..=255).prefix("Z "));
                if ui.button("Clear selected collision").clicked(){for c in self.selected.clone(){self.set(g,c,false);}self.selected.clear();}
                if ui.button("Restore all wall edits").clicked(){self.walls.clear();self.heights.clear();self.selected.clear();}
                if !self.selected.is_empty(){
                    let current=self.selected.iter().next().and_then(|c|self.heights.get(c)).copied().unwrap_or(0);
                    let mut next=current;
                    if ui.add(egui::DragValue::new(&mut next).speed(16).prefix("Y preview ")).changed(){for &c in &self.selected{self.heights.insert(c,next);}}
                }
            }
            if ui.button("Save collision edits...").clicked(){if let Some(path)=rfd::FileDialog::new().add_filter("LCK collision", &["lckcollision"]).save_file(){if let Err(e)=self.save(&path){ui.ctx().debug_painter().text(Pos2::new(10.,10.),egui::Align2::LEFT_TOP,format!("Collision save: {e}"),egui::FontId::default(),Color32::RED);}}}
            if ui.button("Load collision edits...").clicked(){if let Some(path)=rfd::FileDialog::new().add_filter("LCK collision", &["lckcollision"]).pick_file(){let _=self.load(&path);}}
        });
        if self.heights.values().any(|&h|h!=0){ui.label("Y shifts are saved as editor metadata; retail MAP wall occupancy has no per-box height. IMG exports X/Z wall edits only.");}
    }

    pub(super) fn cancel_pointer_gesture(&mut self){
        self.press=None;
        self.pointer_on_wall=false;
    }

    pub(super) fn apply_marquee(
        &mut self,ui:&Ui,viewport:Rect,view:&Viewport,
        model:&Model,grid:&CollisionGrid,
        selection:Rect,mods:egui::Modifiers
    ){
        let projector=Projector::new(ui,viewport,view,model);
        let mut hits=BTreeSet::new();

        for z in 0..256_i32{
            for x in 0..256_i32{
                let cell=(x,z);

                if !self.occupied(grid,cell){
                    continue;
                }

                let x0=x as f32*64.0;
                let z0=(z-2) as f32*64.0;

                let y0=grid.ground_y(
                    x0+32.0,z0+32.0
                )+self.heights.get(&cell)
                    .copied().unwrap_or(0) as f32;

                let corners=[
                    [x0,y0,z0],
                    [x0+64.0,y0,z0],
                    [x0+64.0,y0,z0+64.0],
                    [x0,y0,z0+64.0],
                    [x0,y0+108.0,z0],
                    [x0+64.0,y0+108.0,z0],
                    [x0+64.0,y0+108.0,z0+64.0],
                    [x0,y0+108.0,z0+64.0],
                ];

                let mut bounds:Option<Rect>=None;

                for point in corners{
                    if let Some(p)=projector.project(point){
                        bounds=Some(match bounds{
                            None=>Rect::from_min_max(p,p),
                            Some(old)=>old.union(
                                Rect::from_min_max(p,p)
                            ),
                        });
                    }
                }

                if bounds.is_some_and(|r|r.intersects(selection)){
                    hits.insert(cell);
                }
            }
        }

        if !mods.shift && !mods.ctrl{
            self.selected.clear();
        }

        for cell in hits{
            if mods.ctrl{
                if !self.selected.insert(cell){
                    self.selected.remove(&cell);
                }
            }else{
                self.selected.insert(cell);
            }
        }
    }

    fn save(&self,path:&Path)->Result<()>{
        let data=Document{version:1,scene:self.scene.clone(),walls:self.walls.iter().map(|(&(x,z),&v)|(x,z,v)).collect(),heights:self.heights.iter().map(|(&(x,z),&y)|(x,z,y)).collect()};
        fs::write(path,serde_json::to_vec_pretty(&data)?)?;Ok(())
    }
    fn load(&mut self,path:&Path)->Result<()>{
        let data:Document=serde_json::from_slice(&fs::read(path)?)?;
        ensure!(data.version==1&&data.scene==self.scene,"Collision file belongs to another scene or schema");
        let walls=data.walls.into_iter().map(|(x,z,b)|((x,z),b)).collect::<BTreeMap<_,_>>();
        ensure!(walls.keys().all(|&(x,z)|wall_offset(x,z).is_some()),"Invalid MAP subcell");
        self.walls=walls;self.heights=data.heights.into_iter().map(|(x,z,y)|((x,z),y)).collect();self.selected.clear();Ok(())
    }

    pub fn overlay_existing(&self,target:&Path,name:&str,index:&ProtIndex)->Result<String>{
        super::ps1_original_guard::ensure_existing_overlay(target)?;
        ensure!(self.scene==name,"Collision layer/scene mismatch");
        ensure!(target.exists(),"Exported IMG not found");
        ensure!(!self.walls.is_empty(),"No X/Z collision changes to overlay");
        ensure!(self.heights.values().all(|&y|y==0),
            "Native IMG cannot encode per-box collision Y offsets. Reset Y to zero or use an explicit engine extension; no partial export written");

        let scene=Scene::load(index,name)?;
        let entry=scene.field_map_index(index).context("Native MAP not found")? as usize;

        let original=fs::read(target)?;
        let len=original.len();
        let mut disc=DiscPatcher::open(original)?;
        let before=disc.read_entry_footprint(entry)?;
        ensure!(before.len()>=0x8000,"Native MAP too short");

        let mut after=before.clone();
        for (&(x,z),&occupied) in &self.walls {
            let (off,bit)=wall_offset(x,z).context("Wall outside MAP grid")?;
            let at=0x4000+off;
            if occupied { after[at] |= bit; } else { after[at] &= !bit; }
        }

        ensure!(
            before[0x4000..0x8000].iter().zip(&after[0x4000..0x8000]).any(|(a,b)|a!=b),
            "No effective collision overlay changes"
        );

        for (i,(&a,&b)) in before.iter().zip(after.iter()).enumerate(){
            if i>=0x4000 && i<0x8000 {
                ensure!(a&15==b&15,"Floor height nibble changed");
            } else {
                ensure!(a==b,"Non-wall MAP data changed");
            }
        }

        disc.patch_prot_entry(entry,0x4000,&after[0x4000..0x8000])?;
        ensure!(disc.read_entry_footprint(entry)?==after,"MAP readback mismatch");

        let bytes=disc.into_image();
        ensure!(bytes.len()==len,"IMG length changed");

        let tmp=target.with_extension("u1a_collision_overlaying");
        ensure!(!tmp.exists(),"Temporary overlay output already exists");

        {
            use std::io::Write as _;
            let mut file=fs::OpenOptions::new()
                .write(true).create_new(true).open(&tmp)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
        }
        let verified=(||->Result<()>{
            let disc=DiscPatcher::open(fs::read(&tmp)?)?;
            ensure!(disc.read_entry_footprint(entry)?==after,"Published collision overlay IMG differs");
            Ok(())
        })();

        if let Err(e)=verified{
            let _=fs::remove_file(&tmp);
            return Err(e);
        }

        // Preserve the base native export until the merged
        // collision image has been published and verified.
        let preserved=target.with_extension("u1a_pre_collision_img");
        ensure!(
            !preserved.exists(),
            "U1A backup output already exists"
        );

        fs::rename(target,&preserved)?;

        if let Err(error)=fs::rename(&tmp,target){
            let restoration=fs::rename(&preserved,target);
            let _=fs::remove_file(&tmp);

            if let Err(restore_error)=restoration {
                anyhow::bail!(
                    "Collision publish failed: {error}. Base IMG remains at {}. Restoration failed: {restore_error}",
                    preserved.display()
                );
            }

            return Err(error.into());
        }

        let verified=(||->Result<()>{
            let published=fs::read(target)?;
            ensure!(
                published.len()==len,
                "Published merged IMG length changed"
            );

            let disc=DiscPatcher::open(published)?;
            ensure!(
                disc.read_entry_footprint(entry)?==after,
                "Merged collision MAP readback differs"
            );

            Ok(())
        })();

        if let Err(error)=verified {
            let _=fs::remove_file(target);

            if let Err(restore_error)=fs::rename(&preserved,target){
                anyhow::bail!(
                    "Merged verification failed: {error}. Original export remains at {}. Restore failed: {restore_error}",
                    preserved.display()
                );
            }

            return Err(error);
        }

        fs::remove_file(&preserved)?;

        let unsupported=self.heights.values().filter(|&&y|y!=0).count();
        Ok(format!(
            "Collision overlay verified on existing IMG: {} | {} coordinates | {} vertical offsets NOT exported.",
            target.display(),self.walls.len(),unsupported
        ))
    }

    pub fn export(&self,source:&Path,output:&Path,name:&str,index:&ProtIndex)->Result<String>{
        super::ps1_original_guard::ensure_new_output(source,output)?;
        ensure!(self.scene==name,"Collision layer/scene mismatch");
        ensure!(!self.walls.is_empty(),"No X/Z collision changes to export");
        ensure!(self.heights.values().all(|&y|y==0),
            "Native IMG cannot encode per-box collision Y offsets. Reset Y to zero or use an explicit engine extension; no partial export written");
        ensure!(source!=output,"Cannot overwrite input IMG");
        ensure!(!output.exists(),"Choose a NEW IMG filename");
        let scene=Scene::load(index,name)?;
        let entry=scene.field_map_index(index).context("Native MAP not found")? as usize;
        let original=fs::read(source)?;let len=original.len();
        let mut disc=DiscPatcher::open(original)?;
        let before=disc.read_entry_footprint(entry)?;
        ensure!(before.len()>=0x8000,"Native MAP too short");
        let mut after=before.clone();
        for (&(x,z),&occupied) in &self.walls {
            let (off,bit)=wall_offset(x,z).context("Wall outside MAP grid")?;
            let at=0x4000+off;
            if occupied {after[at]|=bit;}else{after[at]&=!bit;}
        }
        ensure!(before[0x4000..0x8000].iter().zip(&after[0x4000..0x8000]).any(|(a,b)|a!=b),"No effective collision changes");
        for (i,(&a,&b)) in before.iter().zip(after.iter()).enumerate(){if i>=0x4000&&i<0x8000{ensure!(a&15==b&15,"Floor height nibble changed");}else{ensure!(a==b,"Non-wall MAP data changed");}}
        disc.patch_prot_entry(entry,0x4000,&after[0x4000..0x8000])?;
        ensure!(disc.read_entry_footprint(entry)?==after,"MAP readback mismatch");
        let bytes=disc.into_image();ensure!(bytes.len()==len,"IMG length changed");
        let tmp=output.with_extension("lckcollision_building");ensure!(!tmp.exists(),"Temporary output already exists");
        {
            use std::io::Write as _;
            let mut file=fs::OpenOptions::new()
                .write(true).create_new(true).open(&tmp)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
        }
        let verified=(||->Result<()> {let disc=DiscPatcher::open(fs::read(&tmp)?)?;ensure!(disc.read_entry_footprint(entry)?==after,"Published collision IMG differs");Ok(())})();
        if let Err(e)=verified{let _=fs::remove_file(&tmp);return Err(e);}
        if let Err(e)=fs::hard_link(&tmp,output){
            let _=fs::remove_file(&tmp);
            return Err(e.into());
        }
        fs::remove_file(&tmp)?;
        let unsupported=self.heights.values().filter(|&&y|y!=0).count();
        let note=format!("C1 native wall edits: {} coordinates; {} vertical offsets NOT exported (retail MAP does not encode wall height). No collision ownership inferred. Gameplay unverified.",self.walls.len(),unsupported);
        let _=fs::write(output.with_extension("collision_debug.txt"),&note);
        Ok(format!("Collision IMG verified: {}. {}",output.display(),note))
    }
}

pub fn extract_first_img_path(text:&str)->Option<std::path::PathBuf>{
    let bytes=text.as_bytes();
    for i in 0..bytes.len().saturating_sub(2){
        let b=bytes[i];
        if (b as char).is_ascii_alphabetic() && bytes.get(i+1)==Some(&b':') &&
           matches!(bytes.get(i+2),Some(b'\\')|Some(b'/')){
            let tail=&text[i..];
            let lower=tail.to_ascii_lowercase();
            if let Some(end)=lower.find(".img"){
                return Some(std::path::PathBuf::from(&tail[..end+4]));
            }
        }
    }
    None
}

pub fn scene_has_instance_edits(scene_name:&str, world:&super::world_view::WorldModel)->bool{
    super::scene_instances::serialize(scene_name,world).lines().any(|line|{
        matches!(line.split('\t').next().unwrap_or(""),
            "MOVE"|"INSTANCE"|"FOREIGN"|"ROTATE"|"SCALE"|
            "HIDE"|"NPC_MODEL"|"INSTANCE_ROT"|"INSTANCE_SCALE"|
            "INSTANCE_ANIM"|"ANIM"|"INSTANCE_MODEL")
    })
}


#[cfg(test)]mod tests {use super::*;
 #[test]fn exact_wall_bits_preserve_floor_nibbles(){let mut b=[0x07u8];let bit=1u8<<5;b[0]|=bit;assert_eq!(b[0]&0x0f,7);b[0]&=!bit;assert_eq!(b[0],7);}
 #[test]fn subcell_bounds(){assert!(wall_offset(255,255).is_some());assert!(wall_offset(256,0).is_none());}
}

#[cfg(test)]
mod lck_collision_group_tests {
    use super::*;
    #[test]
    fn four_boxes_rotate_about_selection_center() {
        let selected=BTreeSet::from([(10,10),(11,10),(10,11),(11,11)]);
        let pairs=rotation_pairs(&selected,1).unwrap();
        let targets=pairs.iter().map(|(_,to)|*to).collect::<BTreeSet<_>>();
        assert_eq!(targets,selected);
        assert!(pairs.contains(&((10,10),(11,10))));
    }
    #[test]
    fn asymmetrical_group_rotates_around_bounds_center() {
        let selected=BTreeSet::from([(10,10),(12,10),(12,12)]);
        let pairs=rotation_pairs(&selected,1).unwrap();
        assert!(pairs.contains(&((10,10),(12,10))));
        assert!(pairs.contains(&((12,10),(12,12))));
        assert!(pairs.contains(&((12,12),(10,12))));
    }
    #[test]
    fn half_cell_pivot_snaps_but_preserves_spacing() {
        let selected=BTreeSet::from([(10,10),(11,10)]);
        let pairs=rotation_pairs(&selected,1).unwrap();
        let targets=pairs.into_iter().map(|(_,to)|to).collect::<BTreeSet<_>>();
        assert_eq!(targets,BTreeSet::from([(11,10),(11,11)]));
        assert_eq!(rotation_pairs(&selected,0).unwrap().iter()
            .map(|(_,to)|*to).collect::<BTreeSet<_>>(),selected);
    }
}
