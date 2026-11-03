
//! C1 experimental native MAP wall authoring. No ownership guesses.
use std::{collections::{BTreeMap,BTreeSet},fs,path::Path};
use anyhow::{Result,Context,ensure};
use eframe::egui::{self,Pos2,Rect,Ui,Color32,Stroke};
use legaia_disc_patch::disc::DiscPatcher;
use legaia_engine_scene::scene::{Scene,ProtIndex};
use crate::raster::Model;
use super::{collision_wire::{CollisionGrid,Projector,wall_offset},viewport::Viewport};
type Cell=(i32,i32);
#[derive(Clone)]
struct Press {cell:Cell,start:Pos2,selected_before:bool,group:BTreeSet<Cell>,baseline:BTreeMap<Cell,bool>,height:BTreeMap<Cell,i32>,changed:bool}
#[derive(Clone,Default)]
pub(super) struct State {
    scene:String,
    walls:BTreeMap<Cell,bool>,
    heights:BTreeMap<Cell,i32>,
    selected:BTreeSet<Cell>,
    press:Option<Press>,
    pub pointer_on_wall:bool,
    add_x:i32,add_z:i32,
}
#[derive(serde::Serialize,serde::Deserialize)]
struct Document {version:u32,scene:String,walls:Vec<(i32,i32,bool)>,heights:Vec<(i32,i32,i32)>}
impl State {
    pub fn has_edits(&self)->bool { !self.walls.is_empty() || self.heights.values().any(|&y|y!=0) }
    pub fn has_selection(&self)->bool { !self.selected.is_empty() }
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
                    self.press=Some(Press{cell:hit,start:pos,selected_before,group:self.selected.clone(),baseline:self.walls.clone(),height:self.heights.clone(),changed:false});
                }
            }
        }
        self.press.as_ref().is_some_and(|p|p.selected_before)
    }
    fn shifted(&mut self,grid:&CollisionGrid,group:&BTreeSet<Cell>,dx:i32,dz:i32){
        let original:Vec<Cell>=group.iter().copied().collect();
        if original.iter().any(|c|wall_offset(c.0+dx,c.1+dz).is_none()){return;}
        for &c in &original {self.set(grid,c,false);}
        self.selected.clear();
        for c in original {
            let next=(c.0+dx,c.1+dz);
            self.set(grid,next,true);
            if let Some(height)=self.heights.remove(&c){self.heights.insert(next,height);}
            self.selected.insert(next);
        }
    }
    pub fn finish(&mut self,ui:&Ui,rect:Rect,view:&Viewport,model:&Model,grid:&CollisionGrid){
        let (down,released,point,ctrl,shift)=ui.input(|i|(i.pointer.primary_down(),i.pointer.primary_released(),i.pointer.interact_pos(),i.modifiers.ctrl,i.modifiers.shift));
        if let (Some(press),Some(pos))=(self.press.clone(),point){
            if down && press.selected_before && (pos-press.start).length_sq()>36.0 {
                if ctrl {
                    let delta=((press.start.y-pos.y)/4.0).round() as i32*16;
                    self.heights=press.height.clone();
                    for &c in &press.group {self.heights.insert(c,press.height.get(&c).copied().unwrap_or(0)+delta);}
                }else{
                    let delta=super::editor_drag_offset([0.;3],pos-press.start,view.cam,model.radius,rect.height(),view.world_mirrored,false);
                    let dx=(delta[0]/64.).round() as i32;
                    let dz=(delta[2]/64.).round() as i32;
                    self.walls=press.baseline.clone();self.heights=press.height.clone();
                    self.shifted(grid,&press.group,dx,dz);
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
                let copy=self.selected.clone();self.selected.clear();
                for c in copy {let next=(c.0+1,c.1);if wall_offset(next.0,next.1).is_some(){self.set(grid,next,true);self.selected.insert(next);}}
            }
            for (key,sign) in [(egui::Key::Q,-1),(egui::Key::E,1)]{
                if ui.input_mut(|i|i.consume_key(egui::Modifiers::NONE,key)) {self.rotate(grid,sign);}
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
    fn rotate(&mut self,grid:&CollisionGrid,sign:i32){
        let group=self.selected.clone();if group.is_empty(){return;}
        let cx=group.iter().map(|c|c.0).sum::<i32>() as f32/group.len() as f32;
        let cz=group.iter().map(|c|c.1).sum::<i32>() as f32/group.len() as f32;
        let transformed=group.iter().map(|&c|{let x=c.0 as f32-cx;let z=c.1 as f32-cz;if sign>0{((cx-z).round() as i32,(cz+x).round() as i32)}else{((cx+z).round() as i32,(cz-x).round() as i32)}}).collect::<Vec<_>>();
        if transformed.iter().any(|c|wall_offset(c.0,c.1).is_none()){return;}
        let elevations=group.iter().map(|&c|self.heights.get(&c).copied().unwrap_or(0)).collect::<Vec<_>>();
        for &c in &group{self.set(grid,c,false);self.heights.remove(&c);}
        self.selected.clear();
        for (target,height) in transformed.into_iter().zip(elevations){self.set(grid,target,true);if height!=0{self.heights.insert(target,height);}self.selected.insert(target);}
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
        if self.heights.values().any(|&h|h!=0){ui.colored_label(Color32::YELLOW,"Y shifts are saved as editor metadata; retail MAP wall occupancy has no per-box height. IMG exports X/Z wall edits only.");}
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
        ensure!(self.scene==name,"Collision layer/scene mismatch");
        ensure!(target.exists(),"Exported IMG not found");
        ensure!(!self.walls.is_empty(),"No X/Z collision changes to overlay");

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

        fs::write(&tmp,&bytes)?;
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
        ensure!(self.scene==name,"Collision layer/scene mismatch");
        ensure!(!self.walls.is_empty(),"No X/Z collision changes to export");
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
        fs::write(&tmp,&bytes)?;
        let verified=(||->Result<()> {let disc=DiscPatcher::open(fs::read(&tmp)?)?;ensure!(disc.read_entry_footprint(entry)?==after,"Published collision IMG differs");Ok(())})();
        if let Err(e)=verified{let _=fs::remove_file(&tmp);return Err(e);}
        if let Err(e)=fs::rename(&tmp,output){let _=fs::remove_file(&tmp);return Err(e.into());}
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
