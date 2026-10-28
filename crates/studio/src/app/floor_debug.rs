//! Native-data comparison viewer, independent of geometry bounds and editor
//! transforms. It is a sampled visualization, never evidence of game execution.
use anyhow::{Result,Context,ensure};
use eframe::egui::{self,Color32,Stroke,Pos2,Vec2,Rect,Align2,FontId};
use legaia_code_hooks::extended_floor::{self,Row};
use legaia_engine_scene::scene::{Scene,ProtIndex};
use legaia_engine_field::field_elevation::{self,ElevationOverride};
use super::{collision_wire::Projector,viewport::Viewport};
use crate::raster::Model;

pub(super) struct DebugFloor {map:Vec<u8>,lut:[i32;16],elevations:Vec<ElevationOverride>,pub rows:Vec<Row>}
impl DebugFloor {
    pub fn from_data(map:Vec<u8>,lut:[i32;16],elevations:Vec<ElevationOverride>,rows:Vec<Row>)->Result<Self> {
        ensure!(map.len()==0x12000 && !rows.is_empty(),"Invalid native comparison data");
        Ok(Self{map,lut,elevations,rows})
    }
    pub fn load(index:&ProtIndex,name:&str,_key:String)->Result<Self> {
        let scene=Scene::load(index,name)?;let id=scene.field_map_index(index).context("Cell has no native MAP")?;
        let map=index.entry_bytes(id)?.to_vec();let at=extended_floor::table_offset(&map)?;
        let rows=extended_floor::decode(&map[at..]).context("This cell has no extended floor table; open the Stage E or observable test IMG")?;
        let mut elevations=field_elevation::parse_elevation_overrides(&map[0x10000..]);
        let extended=index.entry_bytes_extended(id)?;
        if let Some(fallback)=extended.get(0x12000..) {elevations.extend(field_elevation::parse_elevation_overrides(fallback));}
        else if let Ok(next)=index.entry_bytes_extended(id+1) {elevations.extend(field_elevation::parse_elevation_overrides(&next));}
        let lut=scene.field_floor_height_lut(index)?.context("No native floor ladder")?.map(|v|-(v as i32));
        Self::from_data(map,lut,elevations,rows)
    }
    pub fn retail(&self,x:i32,z:i32,bilinear:bool)->i32 {
        let tx=x>>7;let tz=z>>7;
        if !(0..128).contains(&tx)||!(0..128).contains(&tz){return 0;}
        let at=0x4000+(tz*128+tx) as usize;
        let heights=[0,1,128,129].map(|d|self.lut[(self.map[at+d]&15) as usize]);
        let go=0x8000+(tz*128+tx) as usize*2;
        let word=u16::from_le_bytes(self.map[go..go+2].try_into().unwrap());
        if !bilinear && word&0x800!=0 {
            return (heights.iter().sum::<i32>()>>2)+field_elevation::lookup_elevation_override(&self.elevations,tx as u8,tz as u8).map_or(0,|r|r.delta_at(x,z));
        }
        let wx=x&127;let wz=z&127;
        (heights[0]*(128-wx)*(128-wz)+heights[1]*wx*(128-wz)+heights[2]*(128-wx)*wz+heights[3]*wx*wz)/16384
    }
    pub fn stage(&self,x:i32,z:i32)->i32 {
        self.rows.iter().find(|r|r.destination==[(x>>6) as u8,(z>>6) as u8]).map_or_else(||self.retail(x,z,false),|r|
            self.retail(x+r.delta[0] as i32,z+r.delta[1] as i32,r.mode==2)+r.height_delta as i32)
    }
    /// Native static wall query including its X edge rule and +2 Z-row bias.
    pub fn wall(&self,x:i32,z:i32)->bool {
        if x<0||z<0{return true;}
        let xc=((x+63)>>6)-1;let zc=(z>>6)+2;
        let at=0x4000+((((zc/2)&127)*128+(xc/2&127)) as usize);
        self.map[at]&(1<<(4+(xc&1)+2*(zc&1)))!=0
    }
    pub fn window(&self,ctx:&egui::Context) {
        egui::Window::new("Native floor sampler comparison").default_width(640.).show(ctx,|ui|{
            ui.label("Orange: original/source • cyan: Stage E/destination • red: stored native wall");
            ui.label("Native integer heights (PSX Y-down). Preview uses stored ladder; live game values come from the read-only telemetry monitor.");
            let min=[self.rows.iter().map(|r|r.destination[0] as i32*64).min().unwrap()-64,self.rows.iter().map(|r|r.destination[1] as i32*64).min().unwrap()-64];
            let max=[self.rows.iter().map(|r|r.destination[0] as i32*64+64).max().unwrap()+64,self.rows.iter().map(|r|r.destination[1] as i32*64+64).max().unwrap()+64];
            let (rect,response)=ui.allocate_exact_size(Vec2::new(600.,300.),egui::Sense::hover());
            let painter=ui.painter_at(rect);let project=|x:f32,z:f32|Pos2::new(rect.left()+(x-min[0] as f32)/(max[0]-min[0]) as f32*rect.width(),rect.top()+(z-min[1] as f32)/(max[1]-min[1]) as f32*rect.height());
            for z in (min[1]..max[1]).step_by(64){for x in (min[0]..max[0]).step_by(64){
                let cell=Rect::from_min_max(project(x as f32,z as f32),project((x+64) as f32,(z+64) as f32));
                if let Some(row)=self.rows.iter().find(|r|r.destination==[(x/64) as u8,(z/64) as u8]) {
                    let color=if row.mode==2 {Color32::from_rgb(245,145,50)}else{Color32::from_rgb(40,225,240)};
                    painter.rect_filled(cell.shrink(1.),0.,color.gamma_multiply(0.3));
                    for [a,b] in [[cell.left_top(),cell.right_top()],[cell.right_top(),cell.right_bottom()],[cell.right_bottom(),cell.left_bottom()],[cell.left_bottom(),cell.left_top()]]{painter.line_segment([a,b],Stroke::new(2.,color));}
                    painter.text(cell.center(),Align2::CENTER_CENTER,format!("{}→{}",self.retail(x+32,z+32,false),self.stage(x+32,z+32)),FontId::monospace(11.),color);
                }
                if self.wall(x+32,z+32){painter.line_segment([cell.left_top(),cell.right_bottom()],Stroke::new(2.,Color32::RED));}
            }}
            if let Some(p)=response.hover_pos(){let x=min[0]+((p.x-rect.left())/rect.width()*(max[0]-min[0]) as f32) as i32;let z=min[1]+((p.y-rect.top())/rect.height()*(max[1]-min[1]) as f32) as i32;
                response.on_hover_text(format!("X={x}, Z={z}\nRetail ground={}\nStage E ground={}\nStored wall={}",self.retail(x,z,false),self.stage(x,z),self.wall(x,z)));}
            ui.label(format!("Exact half-open bounds shown in native X/Z; window X {}..{}, Z {}..{}",min[0],max[0],min[1],max[1]));
            ui.colored_label(Color32::YELLOW,"Static walls alone cannot prove reachability: actor colliders, scripts, start position and ledge rules also matter. Controlled proxy probes need no access.");
            egui::Grid::new("floor-record-values").striped(true).show(ui,|ui|{
                for title in ["Native region [X0,X1) × [Z0,Z1)","Mode","Retail center","Stage E center","Static wall"]{ui.label(title);}ui.end_row();
                for r in &self.rows {let x=r.destination[0] as i32*64;let z=r.destination[1] as i32*64;
                    ui.monospace(format!("[{x},{}) × [{z},{})",x+64,z+64));ui.label(if r.mode==2 {"vacated source"}else{"relocated floor"});ui.monospace(self.retail(x+32,z+32,false).to_string());ui.monospace(self.stage(x+32,z+32).to_string());ui.label(self.wall(x+32,z+32).to_string());ui.end_row();}
            });
        });
    }
    pub fn paint(&self,ui:&egui::Ui,rect:Rect,view:&Viewport,model:&Model) {
        let p=Projector::new(ui,rect,view,model);let painter=ui.painter_at(rect);
        // Eight-unit native sample lattice; never interpolate across a 64-unit
        // discontinuity. Hover in the separate plan for exact integer queries.
        for row in &self.rows {let x=row.destination[0] as i32*64;let z=row.destination[1] as i32*64;
            for original in [true,false] {let color=if original {Color32::from_rgb(245,145,50)}else{Color32::from_rgb(40,225,240)};
                let sample=|dx:i32,dz:i32|{let wx=x+dx.min(63);let wz=z+dz.min(63);let y=if original {self.retail(wx,wz,false)}else{self.stage(wx,wz)};p.project([(x+dx) as f32,-y as f32,(z+dz) as f32])};
                for dz in (0..64).step_by(8){for dx in (0..64).step_by(8){let v=[sample(dx,dz),sample(dx+8,dz),sample(dx+8,dz+8),sample(dx,dz+8)];
                    for (a,b) in [(0,1),(1,2),(2,3),(3,0)] {if let (Some(a),Some(b))=(v[a],v[b]){painter.line_segment([a,b],Stroke::new(1.,color));}}
                }}
            }
        }
    }
}
