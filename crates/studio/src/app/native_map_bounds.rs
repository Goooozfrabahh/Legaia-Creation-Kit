// LCK_R47_NATIVE_MAP_BOUNDS
// Read-only visualization of retail Legaia's 128 x 128 MAP.
// Does not modify MAP, MAN, collision, camera, or IMG output.

use eframe::egui::{self,Color32,Stroke};

use super::{
    ground_edit_ops::{self,Transform},
    viewport::Viewport,
    world_view::WorldModel,
};

use crate::raster::{Camera,Model};

const TILES:usize=128;
const TILE:f32=128.0;
const LIMIT:f32=16384.0;

#[derive(Clone,Copy)]
struct Marker {
    x:f32,
    z:f32,
    color:Color32,
    selected:bool,
}

struct Report {
    objects:usize,
    objects_outside:usize,
    edited_objects:usize,
    edited_objects_outside:usize,
    floors_inside:usize,
    floors_partial:usize,
    floors_outside:usize,
    markers:Vec<Marker>,
    edited_bounds:Option<[f32;4]>,
}

impl Default for Report {
    fn default()->Self {
        Self {
            objects:0,
            objects_outside:0,
            edited_objects:0,
            edited_objects_outside:0,
            floors_inside:0,
            floors_partial:0,
            floors_outside:0,
            markers:Vec::new(),
            edited_bounds:None,
        }
    }
}

fn inside(x:f32,z:f32)->bool {
    x.is_finite() && z.is_finite() &&
    (0.0..LIMIT).contains(&x) &&
    (0.0..LIMIT).contains(&z)
}

fn extend_bounds(report:&mut Report,x:f32,z:f32) {
    if !x.is_finite() || !z.is_finite() {return;}

    match &mut report.edited_bounds {
        Some(b)=>{
            b[0]=b[0].min(x);
            b[1]=b[1].max(x);
            b[2]=b[2].min(z);
            b[3]=b[3].max(z);
        }
        None=>{
            report.edited_bounds=Some([x,x,z,z]);
        }
    }
}

// Match ground_edit_ops::transform_point X/Z behavior.
fn floor_corners(source:[u8;2],t:Transform)->[[f32;2];4] {
    let cx=source[0] as f32*TILE+TILE*0.5;
    let cz=source[1] as f32*TILE+TILE*0.5;

    [(-64.0,-64.0),(64.0,-64.0),(64.0,64.0),(-64.0,64.0)]
        .map(|(dx,dz)|{
            let x=dx*t.scale[0];
            let z=dz*t.scale[2];

            let (rx,rz)=match t.turns.rem_euclid(4) {
                0=>(x,z),
                1=>(-z,x),
                2=>(-x,-z),
                _=>(z,-x),
            };

            [cx+rx+t.offset[0],cz+rz+t.offset[2]]
        })
}

fn add_floor(
    report:&mut Report,
    source:[u8;2],
    transform:Transform,
    selected:bool,
){
    let corners=floor_corners(source,transform);

    let valid=corners.iter().all(|p|
        p[0].is_finite() && p[1].is_finite()
    );

    if !valid {
        report.floors_outside+=1;
        return;
    }

    let min_x=corners.iter().map(|p|p[0])
        .fold(f32::INFINITY,f32::min);
    let max_x=corners.iter().map(|p|p[0])
        .fold(f32::NEG_INFINITY,f32::max);
    let min_z=corners.iter().map(|p|p[1])
        .fold(f32::INFINITY,f32::min);
    let max_z=corners.iter().map(|p|p[1])
        .fold(f32::NEG_INFINITY,f32::max);

    for p in corners {
        extend_bounds(report,p[0],p[1]);
    }

    let entirely_inside=
        min_x>=0. && max_x<=LIMIT &&
        min_z>=0. && max_z<=LIMIT;

    let intersects=
        max_x>0. && min_x<LIMIT &&
        max_z>0. && min_z<LIMIT;

    let color=if entirely_inside {
        report.floors_inside+=1;
        Color32::from_rgb(80,225,225)
    }else if intersects {
        report.floors_partial+=1;
        Color32::from_rgb(255,165,50)
    }else{
        report.floors_outside+=1;
        Color32::from_rgb(255,72,72)
    };

    let cx=(min_x+max_x)*0.5;
    let cz=(min_z+max_z)*0.5;

    report.markers.push(Marker{
        x:cx,z:cz,color,selected
    });
}

fn collect(
    world:&WorldModel,
    ground:&ground_edit_ops::State,
    selected:Option<(bool,usize)>,
)->Report {
    let mut report=Report::default();

    for draw in &world.draws {
        if draw.hidden {continue;}

        let x=draw.center[0];
        let z=draw.center[2];

        let edited=draw.editor_uid.is_some()
            || draw.offset.iter().any(|v|v.abs()>0.01)
            || draw.rotation.iter().any(|v|v.abs()>0.0001)
            || draw.scale.iter().any(|v|(v-1.0).abs()>0.0001);

        report.objects+=1;
        if !inside(x,z) {
            report.objects_outside+=1;
        }

        if edited {
            report.edited_objects+=1;

            if !inside(x,z) {
                report.edited_objects_outside+=1;
            }

            // Use actual modified mesh vertices, not just the
            // center of a large structure.
            if let Some(model)=world.model.as_ref() {
                if let Some(points)=model.positions.get(
                    draw.vertex_start..draw.vertex_end
                ) {
                    for p in points {
                        extend_bounds(&mut report,p[0],p[2]);
                    }
                }
            }
        }

        report.markers.push(Marker{
            x,z,
            color:if !inside(x,z) {
                Color32::from_rgb(255,72,72)
            }else if edited {
                Color32::from_rgb(255,210,80)
            }else{
                Color32::from_gray(150)
            },
            selected:selected==Some((draw.terrain,draw.index)),
        });
    }

    for (&source,&transform) in &ground.originals {
        if !ground.deleted.contains(&source) {
            add_floor(&mut report,source,transform,false);
        }
    }

    for copy in &ground.copies {
        add_floor(
            &mut report,
            copy.source,
            copy.transform,
            ground.active_copy==Some(copy.uid)
        );
    }

    report
}

// Exact CPU-rasterizer projection, adapted for egui points.
// This follows the editor's display-only mirror setting.
fn project(
    rect:egui::Rect,
    cam:Camera,
    model:&Model,
    mirror:bool,
    ppp:f32,
    p:[f32;3],
)->Option<egui::Pos2>{
    let w=((rect.width()*ppp) as usize).clamp(16,900) as f32;
    let h=((rect.height()*ppp) as usize).clamp(16,680) as f32;

    let (sy,cy)=cam.yaw.sin_cos();
    let (sp,cp)=cam.pitch.sin_cos();

    let half_fov=45f32.to_radians()*0.5;
    let f=(h*0.5)/half_fov.tan();

    let dist=model.radius/half_fov.sin()
        *cam.zoom.max(0.0000001);

    let x=p[0]-(model.center[0]+cam.target_offset[0]);
    let y=p[1]-(model.center[1]+cam.target_offset[1]);
    let z=p[2]-(model.center[2]+cam.target_offset[2]);

    let x1=x*cy+z*sy;
    let z1=-x*sy+z*cy;
    let y2=y*cp-z1*sp;
    let depth=y*sp+z1*cp+dist;

    let near=(dist*0.003).max(0.0001);

    if depth<=near || !depth.is_finite() {return None;}

    let sx=w*0.5+f*x1/depth;
    let sy=h*0.5+f*y2/depth;

    let sx=if mirror {w-1.0-sx}else{sx};

    let pos=egui::pos2(
        rect.left()+sx*rect.width()/w,
        rect.top()+sy*rect.height()/h
    );

    (pos.x.is_finite() && pos.y.is_finite())
        .then_some(pos)
}

fn paint_line(
    painter:&egui::Painter,
    rect:egui::Rect,
    cam:Camera,
    model:&Model,
    mirror:bool,
    ppp:f32,
    a:[f32;3],
    b:[f32;3],
    stroke:Stroke,
){
    if let (Some(a),Some(b))=(
        project(rect,cam,model,mirror,ppp,a),
        project(rect,cam,model,mirror,ppp,b)
    ){
        painter.line_segment([a,b],stroke);
    }
}

fn paint_3d(
    ui:&egui::Ui,
    rect:egui::Rect,
    view:&Viewport,
    model:&Model,
    grid_y:f32,
    stride:usize,
){
    let painter=ui.painter().with_clip_rect(rect);
    let ppp=ui.ctx().pixels_per_point();
    let cam=view.cam;
    let mirror=view.world_mirrored;

    let minor=Stroke::new(
        0.8,
        Color32::from_rgba_unmultiplied(95,200,240,65)
    );

    let major=Stroke::new(
        1.4,
        Color32::from_rgba_unmultiplied(95,220,255,140)
    );

    let edge=Stroke::new(
        3.0,
        Color32::from_rgb(255,95,85)
    );

    let stride=stride.clamp(1,32);

    for i in (0..=TILES).step_by(stride) {
        let coord=i as f32*TILE;
        let stroke=if i%32==0 {major}else{minor};

        paint_line(
            &painter,rect,cam,model,mirror,ppp,
            [coord,grid_y,0.],
            [coord,grid_y,LIMIT],
            stroke
        );

        paint_line(
            &painter,rect,cam,model,mirror,ppp,
            [0.,grid_y,coord],
            [LIMIT,grid_y,coord],
            stroke
        );
    }

    for (a,b) in [
        ([0.,grid_y,0.],[LIMIT,grid_y,0.]),
        ([LIMIT,grid_y,0.],[LIMIT,grid_y,LIMIT]),
        ([LIMIT,grid_y,LIMIT],[0.,grid_y,LIMIT]),
        ([0.,grid_y,LIMIT],[0.,grid_y,0.])
    ] {
        paint_line(
            &painter,rect,cam,model,mirror,ppp,
            a,b,edge
        );
    }
}

fn map_point(rect:egui::Rect,x:f32,z:f32)->egui::Pos2 {
    egui::pos2(
        rect.left()+x.clamp(0.,LIMIT)/LIMIT*rect.width(),
        rect.top()+z.clamp(0.,LIMIT)/LIMIT*rect.height()
    )
}

fn paint_overview(
    ui:&mut egui::Ui,
    report:&Report,
    stride:usize
){
    let size=ui.available_width().clamp(250.,420.);
    let (rect,_)=ui.allocate_exact_size(
        egui::vec2(size,size),
        egui::Sense::hover()
    );

    let painter=ui.painter().with_clip_rect(rect);

    painter.rect_filled(
        rect,0.0,Color32::from_rgb(22,27,33)
    );

    let stride=stride.clamp(1,32);

    for i in (0..=TILES).step_by(stride) {
        let v=i as f32*TILE;

        let stroke=Stroke::new(
            if i%32==0 {1.1}else{0.5},
            if i%32==0 {
                Color32::from_gray(95)
            }else{
                Color32::from_gray(50)
            }
        );

        painter.line_segment([
            map_point(rect,v,0.),
            map_point(rect,v,LIMIT)
        ],stroke);

        painter.line_segment([
            map_point(rect,0.,v),
            map_point(rect,LIMIT,v)
        ],stroke);
    }

    let edge=Stroke::new(
        2.5,Color32::from_rgb(255,95,85)
    );

    for (a,b) in [
        ((0.,0.),(LIMIT,0.)),
        ((LIMIT,0.),(LIMIT,LIMIT)),
        ((LIMIT,LIMIT),(0.,LIMIT)),
        ((0.,LIMIT),(0.,0.))
    ] {
        painter.line_segment([
            map_point(rect,a.0,a.1),
            map_point(rect,b.0,b.1)
        ],edge);
    }

    // Draw original objects first, edited ones after them.
    for marker in &report.markers {
        if !marker.x.is_finite() || !marker.z.is_finite(){
            continue;
        }

        let position=map_point(
            rect,marker.x,marker.z
        );

        let radius=if marker.selected {4.5}else{2.0};

        painter.circle_filled(
            position,radius,marker.color
        );

        if marker.selected {
            painter.circle_stroke(
                position,7.0,
                Stroke::new(1.5,Color32::WHITE)
            );
        }
    }
}

pub(super) fn show(
    ui:&mut egui::Ui,
    rect:egui::Rect,
    view:&Viewport,
    model:&Model,
    world:&WorldModel,
    ground:&ground_edit_ops::State,
    selected:Option<(bool,usize)>,
    grid_y:f32,
    stride:usize,
){
    let report=collect(world,ground,selected);

    paint_3d(ui,rect,view,model,grid_y,stride);

    egui::Window::new("Native MAP Grid Inspector")
        .id(egui::Id::new("lck_r47_native_map_grid"))
        .default_width(425.0)
        .resizable(true)
        .show(ui.ctx(),|ui|{
            ui.strong("Original PS1 MAP: 128 x 128 tiles");
            ui.label(
                "One tile = 128 world units. X/Z range: 0 to 16,384."
            );
            ui.small(
                "Top-down projection: +X right, +Z down. Red perimeter is the retail MAP limit."
            );

            paint_overview(ui,&report,stride);

            ui.separator();

            ui.colored_label(
                Color32::from_gray(190),
                format!(
                    "Native and created objects: {} ({} centers outside)",
                    report.objects,report.objects_outside
                )
            );

            ui.colored_label(
                Color32::from_rgb(255,210,80),
                format!(
                    "Edited/created objects: {} ({} centers outside)",
                    report.edited_objects,
                    report.edited_objects_outside
                )
            );

            ui.colored_label(
                Color32::from_rgb(80,225,225),
                format!(
                    "Ground edits fully inside: {}",
                    report.floors_inside
                )
            );

            ui.colored_label(
                Color32::from_rgb(255,165,50),
                format!(
                    "Ground edits crossing edge: {}",
                    report.floors_partial
                )
            );

            ui.colored_label(
                Color32::from_rgb(255,72,72),
                format!(
                    "Ground edits entirely outside: {}",
                    report.floors_outside
                )
            );

            if let Some(b)=report.edited_bounds {
                ui.separator();
                ui.strong("Edited building footprint");
                ui.monospace(format!(
                    "X: {:.1} .. {:.1}",
                    b[0],b[1]
                ));
                ui.monospace(format!(
                    "Z: {:.1} .. {:.1}",
                    b[2],b[3]
                ));

                let overhang=[
                    (-b[0]).max(0.),
                    (b[1]-LIMIT).max(0.),
                    (-b[2]).max(0.),
                    (b[3]-LIMIT).max(0.)
                ];

                ui.monospace(format!(
                    "Overflow -X {:.1}, +X {:.1}, -Z {:.1}, +Z {:.1}",
                    overhang[0],overhang[1],
                    overhang[2],overhang[3]
                ));
            }

            ui.separator();
            ui.small(
                "Gray: original object. Yellow: edited/created object. Cyan: floor. Orange: floor crossing the boundary. Red: outside. White ring: selected object."
            );
            ui.small(
                "Object outside counts use centers; edited object bounds use actual mesh vertices. Ground limits use transformed tile corners."
            );
            ui.small(
                "Read-only diagnosis. Native export, walkability, visibility, and PS1 memory capacity are not changed."
            );
        });
}

#[cfg(test)]
mod native_map_bounds_tests {
    use super::*;

    #[test]
    fn native_grid_exact_dimensions(){
        assert_eq!(TILES*TILES,16384);
        assert_eq!(TILE*TILES as f32,LIMIT);
        assert!(inside(0.,0.));
        assert!(inside(16383.,16383.));
        assert!(!inside(16384.,100.));
        assert!(!inside(-1.,100.));
    }

    #[test]
    fn moved_floor_reports_real_world_corners(){
        let t=Transform{
            offset:[128.,0.,256.],
            ..Transform::default()
        };

        let corners=floor_corners([18,19],t);

        assert_eq!(corners[0],[2432.,2688.]);
        assert_eq!(corners[2],[2560.,2816.]);
    }

    #[test]
    fn partial_and_fully_outside_are_distinct(){
        let mut report=Report::default();

        add_floor(
            &mut report,[0,20],
            Transform{
                offset:[-64.,0.,0.],
                ..Transform::default()
            },false
        );

        add_floor(
            &mut report,[0,20],
            Transform{
                offset:[-20000.,0.,0.],
                ..Transform::default()
            },false
        );

        assert_eq!(report.floors_partial,1);
        assert_eq!(report.floors_outside,1);
    }

    #[test]
    fn quarter_turn_swaps_x_and_z(){
        let corners=floor_corners(
            [20,20],
            Transform{
                turns:1,
                scale:[2.,1.,1.],
                ..Transform::default()
            }
        );

        let width=corners.iter().map(|p|p[0])
            .fold(f32::NEG_INFINITY,f32::max)
            -corners.iter().map(|p|p[0])
            .fold(f32::INFINITY,f32::min);

        let depth=corners.iter().map(|p|p[1])
            .fold(f32::NEG_INFINITY,f32::max)
            -corners.iter().map(|p|p[1])
            .fold(f32::INFINITY,f32::min);

        assert!((width-128.).abs()<0.001);
        assert!((depth-256.).abs()<0.001);
    }
}

// LCK_R48_EXTENDED_GRID
// Extended authoring grid. This is new LCK-owned storage,
// not a rewritten retail MAP or an implemented PS1 runtime hook.
//
// Original 128x128 remains fixed at tile coordinates 0..127.
// Larger grids grow equally in all four directions.

#[derive(Clone,serde::Serialize,serde::Deserialize)]
pub(super) struct ExtendedFloorCell {
    pub source:[u8;2],
    pub uid:Option<u64>,
    pub requested_y:f32,
}

#[derive(Clone,serde::Serialize,serde::Deserialize)]
pub(super) struct ExtendedGridAuthoring {
    pub version:u32,
    pub scene:String,
    pub side:usize,
    pub cells:Vec<Option<ExtendedFloorCell>>,
    pub staged_tiles:usize,
    pub outside_original:usize,
    pub skipped_pieces:usize,
    pub clipped_pieces:usize,
    #[serde(skip)]
    pub status:String,
}

impl Default for ExtendedGridAuthoring {
    fn default()->Self {
        Self::new("",128)
    }
}

impl ExtendedGridAuthoring {
    pub(super) fn new(scene:&str,side:usize)->Self {
        let side=side.clamp(128,256);

        Self {
            version:1,
            scene:scene.to_owned(),
            side,
            cells:vec![None;side*side],
            staged_tiles:0,
            outside_original:0,
            skipped_pieces:0,
            clipped_pieces:0,
            status:String::new(),
        }
    }

    pub(super) fn origin_tile(&self)->i32 {
        -((self.side as i32-128)/2)
    }

    pub(super) fn index(&self,x:i32,z:i32)->Option<usize> {
        let origin=self.origin_tile();
        let col=x-origin;
        let row=z-origin;

        if col<0 || row<0 ||
           col>=self.side as i32 ||
           row>=self.side as i32 {
            return None;
        }

        Some(row as usize*self.side+col as usize)
    }

    fn add_piece(
        &mut self,
        source:[u8;2],
        transform:Transform,
        uid:Option<u64>,
    ){
        let odd=transform.turns.rem_euclid(2)!=0;

        let sx=if odd {
            transform.scale[2].abs()
        }else{
            transform.scale[0].abs()
        };

        let sz=if odd {
            transform.scale[0].abs()
        }else{
            transform.scale[2].abs()
        };

        if !sx.is_finite() || !sz.is_finite() ||
           sx<=0. || sz<=0. ||
           !transform.offset.iter().all(|v|v.is_finite()) {
            self.skipped_pieces+=1;
            return;
        }

        let cx=source[0] as f32*128.+64.+transform.offset[0];
        let cz=source[1] as f32*128.+64.+transform.offset[2];

        let x0=((cx-64.*sx)/128.).floor() as i32;
        let x1=((cx+64.*sx)/128.).ceil() as i32;
        let z0=((cz-64.*sz)/128.).floor() as i32;
        let z1=((cz+64.*sz)/128.).ceil() as i32;

        let origin=self.origin_tile();
        let last=origin+self.side as i32;

        if x0<origin || z0<origin || x1>last || z1>last {
            self.clipped_pieces+=1;
        }

        let mut written=0usize;

        for z in z0.max(origin)..z1.min(last) {
            for x in x0.max(origin)..x1.min(last) {
                if let Some(index)=self.index(x,z) {
                    self.cells[index]=Some(ExtendedFloorCell{
                        source,
                        uid,
                        requested_y:transform.offset[1],
                    });

                    written+=1;
                }
            }
        }

        if written==0 {
            self.skipped_pieces+=1;
        }
    }

    // Rebuild editor-owned extended cell storage from the actual
    // floor edits. Native out-of-bounds copies are retained here,
    // even though the retail serializer cannot consume them yet.
    pub(super) fn stage(
        &mut self,
        scene:&str,
        ground:&ground_edit_ops::State,
        side:usize,
    ){
        let side=side.clamp(128,256);

        if self.scene!=scene || self.side!=side {
            *self=Self::new(scene,side);
        }

        self.cells.fill(None);
        self.staged_tiles=0;
        self.outside_original=0;
        self.skipped_pieces=0;
        self.clipped_pieces=0;

        for (&source,&transform) in &ground.originals {
            if !ground.deleted.contains(&source) {
                self.add_piece(source,transform,None);
            }
        }

        for copy in &ground.copies {
            self.add_piece(
                copy.source,
                copy.transform,
                Some(copy.uid)
            );
        }

        let origin=self.origin_tile();

        for (i,cell) in self.cells.iter().enumerate() {
            if cell.is_none(){continue;}

            self.staged_tiles+=1;

            let x=origin+(i%self.side) as i32;
            let z=origin+(i/self.side) as i32;

            if !(0..128).contains(&x) ||
               !(0..128).contains(&z) {
                self.outside_original+=1;
            }
        }
    }

    // A separate LCK authoring sidecar. This deliberately does NOT
    // patch any raw IMG or claim native runtime compatibility.
    pub(super) fn save(&self)->Result<std::path::PathBuf,String> {
        let exe=std::env::current_exe()
            .map_err(|e|e.to_string())?;

        let parent=exe.parent()
            .ok_or("Cannot locate LCK directory")?;

        let dir=parent.join("grid_extensions");

        std::fs::create_dir_all(&dir)
            .map_err(|e|e.to_string())?;

        let scene=self.scene.chars()
            .map(|c|{
                if c.is_ascii_alphanumeric() || c=='_' {
                    c
                }else{
                    '_'
                }
            })
            .collect::<String>();

        let name=format!(
            "{}_{}x{}_authoring.json",
            if scene.is_empty() {"scene"}else{&scene},
            self.side,self.side
        );

        let path=dir.join(name);

        // Never silently replace a previous authoring snapshot.
        let mut candidate=path.clone();

        if candidate.exists() {
            for number in 1..10000 {
                let attempt=dir.join(format!(
                    "{}_{}x{}_authoring_{}.json",
                    scene,self.side,self.side,number
                ));

                if !attempt.exists() {
                    candidate=attempt;
                    break;
                }
            }
        }

        let json=serde_json::to_vec_pretty(self)
            .map_err(|e|e.to_string())?;

        std::fs::write(&candidate,json)
            .map_err(|e|e.to_string())?;

        Ok(candidate)
    }
}

pub(super) fn show_extension(
    ui:&mut egui::Ui,
    viewport_rect:egui::Rect,
    view:&Viewport,
    model:&Model,
    grid:&mut ExtendedGridAuthoring,
    display_y:f32,
    stride:usize,
){
    if grid.side<=128 {return;}

    let pad=((grid.side-128)/2) as f32*128.;
    let min=-pad;
    let max=LIMIT+pad;

    let painter=ui.painter().with_clip_rect(viewport_rect);
    let ppp=ui.ctx().pixels_per_point();

    let border=Stroke::new(
        3.,
        Color32::from_rgb(55,225,255)
    );

    let line=Stroke::new(
        0.8,
        Color32::from_rgba_unmultiplied(55,225,255,45)
    );

    let step=stride.clamp(1,32);

    for i in (0..=grid.side).step_by(step) {
        let c=min+i as f32*128.;

        paint_line(
            &painter,viewport_rect,view.cam,model,
            view.world_mirrored,ppp,
            [c,display_y,min],
            [c,display_y,max],
            line
        );

        paint_line(
            &painter,viewport_rect,view.cam,model,
            view.world_mirrored,ppp,
            [min,display_y,c],
            [max,display_y,c],
            line
        );
    }

    for (a,b) in [
        ([min,display_y,min],[max,display_y,min]),
        ([max,display_y,min],[max,display_y,max]),
        ([max,display_y,max],[min,display_y,max]),
        ([min,display_y,max],[min,display_y,min])
    ] {
        paint_line(
            &painter,viewport_rect,view.cam,model,
            view.world_mirrored,ppp,a,b,border
        );
    }

    egui::Window::new("Extended MAP Authoring Capacity")
        .id(egui::Id::new("lck_r48_extended_grid"))
        .default_width(345.)
        .resizable(true)
        .show(ui.ctx(),|ui|{
            ui.strong(format!(
                "{} x {} authoring grid",
                grid.side,grid.side
            ));

            ui.label(format!(
                "Total tile positions: {}",
                grid.side*grid.side
            ));

            ui.label(format!(
                "New positions beyond retail: {}",
                grid.side*grid.side-128*128
            ));

            ui.label(format!(
                "Grid tile coordinates: {} through {}",
                grid.origin_tile(),
                grid.origin_tile()+grid.side as i32-1
            ));

            ui.separator();

            ui.label("Top-down extended authoring grid");

            let (map_rect,_)=ui.allocate_exact_size(
                egui::vec2(270.,270.),
                egui::Sense::hover()
            );

            let map_painter=ui.painter()
                .with_clip_rect(map_rect);

            map_painter.rect_filled(
                map_rect,0.,
                Color32::from_rgb(23,29,34)
            );

            let to_screen=|x:f32,z:f32| {
                egui::pos2(
                    map_rect.left()
                        +(x-min)/(max-min)*map_rect.width(),
                    map_rect.top()
                        +(z-min)/(max-min)*map_rect.height()
                )
            };

            for (a,b) in [
                ((min,min),(max,min)),
                ((max,min),(max,max)),
                ((max,max),(min,max)),
                ((min,max),(min,min)),
            ] {
                map_painter.line_segment([
                    to_screen(a.0,a.1),
                    to_screen(b.0,b.1)
                ],Stroke::new(
                    2.5,
                    Color32::from_rgb(55,225,255)
                ));
            }

            // The original retail boundary never moves.
            for (a,b) in [
                ((0.,0.),(LIMIT,0.)),
                ((LIMIT,0.),(LIMIT,LIMIT)),
                ((LIMIT,LIMIT),(0.,LIMIT)),
                ((0.,LIMIT),(0.,0.))
            ] {
                map_painter.line_segment([
                    to_screen(a.0,a.1),
                    to_screen(b.0,b.1)
                ],Stroke::new(
                    2.5,
                    Color32::from_rgb(255,95,85)
                ));
            }

            let origin=grid.origin_tile();

            for (i,cell) in grid.cells.iter().enumerate() {
                if cell.is_none() {continue;}

                let x=origin+(i%grid.side) as i32;
                let z=origin+(i/grid.side) as i32;

                let outside=!(0..128).contains(&x)
                    || !(0..128).contains(&z);

                let pos=to_screen(
                    (x as f32+0.5)*128.,
                    (z as f32+0.5)*128.
                );

                map_painter.circle_filled(
                    pos,1.7,
                    if outside {
                        Color32::from_rgb(255,210,70)
                    }else{
                        Color32::from_rgb(90,225,200)
                    }
                );
            }

            ui.separator();

            ui.label(format!(
                "Staged edited floor cells: {}",
                grid.staged_tiles
            ));

            ui.colored_label(
                Color32::from_rgb(255,210,70),
                format!(
                    "Staged outside original MAP: {}",
                    grid.outside_original
                )
            );

            ui.label(format!(
                "Partially clipped pieces: {}",
                grid.clipped_pieces
            ));

            ui.label(format!(
                "Unstaged/outside extended area: {}",
                grid.skipped_pieces
            ));

            ui.small(
                "Red = original retail MAP. Cyan = selected extended capacity. Yellow = floor data outside retail bounds."
            );

            ui.separator();

            if ui.button("Save extended authoring data").clicked() {
                grid.status=match grid.save(){
                    Ok(path)=>format!(
                        "Saved: {}",path.display()
                    ),
                    Err(error)=>format!(
                        "Save failed: {error}"
                    ),
                };
            }

            if !grid.status.is_empty() {
                ui.small(&grid.status);
            }

            ui.separator();

            ui.colored_label(
                Color32::from_rgb(255,185,85),
                "PS1 RUNTIME EXTENSION NOT YET INSTALLED"
            );

            ui.small(
                "The extra cells exist in LCK authoring storage and can be saved to a separate JSON file. Native IMG exports still use the original 128x128 grids until the PS1 loader, renderer, floor sampler, and collision routines are extended."
            );
        });
}

#[cfg(test)]
mod lck_r48_extended_grid_tests {
    use super::*;

    #[test]
    fn enlarged_grid_has_real_storage_and_centered_origin(){
        let grid=ExtendedGridAuthoring::new("town01",160);

        assert_eq!(grid.cells.len(),25600);
        assert_eq!(grid.origin_tile(),-16);
        assert_eq!(grid.index(-16,-16),Some(0));
        assert_eq!(grid.index(143,143),Some(25599));
        assert_eq!(grid.index(144,0),None);
        assert_eq!(grid.index(-17,0),None);
    }

    #[test]
    fn new_cells_accept_floor_copies_outside_retail(){
        let mut ground=ground_edit_ops::State::default();

        ground.copies.push(ground_edit_ops::CopyPiece{
            uid:17,
            source:[127,10],
            transform:Transform{
                offset:[256.,0.,0.],
                ..Transform::default()
            },
        });

        let mut grid=ExtendedGridAuthoring::default();
        grid.stage("town01",&ground,160);

        assert!(grid.outside_original>0);

        let index=grid.index(129,10).unwrap();

        assert_eq!(
            grid.cells[index].as_ref().unwrap().uid,
            Some(17)
        );
    }

    #[test]
    fn scene_change_resets_extended_authoring_storage(){
        let mut grid=ExtendedGridAuthoring::new("town01",160);

        grid.cells[0]=Some(ExtendedFloorCell{
            source:[10,10],
            uid:Some(1),
            requested_y:0.,
        });

        grid.stage(
            "town02",
            &ground_edit_ops::State::default(),
            192
        );

        assert_eq!(grid.scene,"town02");
        assert_eq!(grid.side,192);
        assert_eq!(grid.cells.len(),36864);
        assert!(grid.cells.iter().all(Option::is_none));
    }
}

// LCK_R49_WORLD_FIXED_GRID
// The grid is real editor-only geometry in the SAME depth-tested
// CPU raster pass as the world. Do not refit the model: that would
// change camera center/radius and make the scene appear to move.
//
// Reserved ID MAX-1 is never pickable and is not a native MAP ID.

const R49_GRID_ID:u32=u32::MAX-1;

fn r49_ribbon(
    model:&mut Model,
    a:[f32;3],
    b:[f32;3],
    width:f32,
    rgb:[u8;3],
){
    let dx=b[0]-a[0];
    let dz=b[2]-a[2];
    let len=(dx*dx+dz*dz).sqrt();

    if !len.is_finite() || len<0.001 {
        return;
    }

    let nx=-dz/len*width*0.5;
    let nz=dx/len*width*0.5;

    let verts=[
        [a[0]+nx,a[1],a[2]+nz],
        [a[0]-nx,a[1],a[2]-nz],
        [b[0]-nx,b[1],b[2]-nz],
        [b[0]+nx,b[1],b[2]+nz],
    ];

    let Ok(base)=u32::try_from(model.positions.len()) else {
        return;
    };

    if base.checked_add(3).is_none() {
        return;
    }

    model.positions.extend(verts);
    model.rest.extend(verts);
    model.object_ids.extend([R49_GRID_ID;4]);
    model.uvs.extend([[0,0];4]);
    model.cba_tsb.extend([[0,0];4]);
    model.modulation.extend([[128,128,128];4]);
    model.fill.extend([rgb;4]);
    model.textured.extend([0u8;4]);

    model.indices.extend([
        base,base+1,base+2,
        base,base+2,base+3,
    ]);
}

pub(super) fn render_fixed_grid(
    original:&Model,
    height:f32,
    stride:usize,
    extended_side:usize,
)->Model {
    let mut model=original.clone();

    let side=match extended_side {
        160|192|224|256=>extended_side,
        _=>128,
    };

    let stride=stride.clamp(1,32);
    let half=((side-128)/2) as f32*128.0;

    let outer_min=-half;
    let outer_max=LIMIT+half;

    // Lift the lines slightly above the requested plane.
    // This reduces coplanar Z fighting without changing walkability.
    let y=height+3.0; // LCK_R612B_VISUAL_STYLE

    let minor=[89,166,196];
    let major=[117,208,231];
    let cyan=[50,225,245];
    let red=[255,88,78];

    for i in (0..=side).step_by(stride) {
        let coordinate=outer_min+i as f32*128.;

        let strong=i%32==0;
        let rgb=if strong {major}else{minor};
        let width=if strong {6.5}else{3.6};

        r49_ribbon(
            &mut model,
            [coordinate,y,outer_min],
            [coordinate,y,outer_max],
            width,rgb
        );

        r49_ribbon(
            &mut model,
            [outer_min,y,coordinate],
            [outer_max,y,coordinate],
            width,rgb
        );
    }

    // Original MAP boundary ALWAYS stays in its original location.
    let retail=[
        [0.,y,0.],
        [LIMIT,y,0.],
        [LIMIT,y,LIMIT],
        [0.,y,LIMIT],
    ];

    for i in 0..4 {
        r49_ribbon(
            &mut model,
            retail[i],
            retail[(i+1)%4],
            15.0,red
        );
    }

    if side>128 {
        let outer=[
            [outer_min,y,outer_min],
            [outer_max,y,outer_min],
            [outer_max,y,outer_max],
            [outer_min,y,outer_max],
        ];

        for i in 0..4 {
            r49_ribbon(
                &mut model,
                outer[i],
                outer[(i+1)%4],
                14.0,cyan
            );
        }
    }

    // Intentionally do NOT call model.refit().
    model.center=original.center;
    model.radius=original.radius;

    model
}

pub(super) fn inline_grid_report(
    ui:&mut egui::Ui,
    world:&WorldModel,
    ground:&ground_edit_ops::State,
    selected:Option<(bool,usize)>,
    stride:usize,
){
    let report=collect(world,ground,selected);

    ui.small(format!(
        "Scene objects: {} ({} centers beyond retail bounds)",
        report.objects,report.objects_outside
    ));

    ui.small(format!(
        "Edited objects: {} ({} centers beyond retail bounds)",
        report.edited_objects,report.edited_objects_outside
    ));

    ui.small(format!(
        "Ground: {} inside, {} crossing, {} outside",
        report.floors_inside,
        report.floors_partial,
        report.floors_outside
    ));

    if let Some(b)=report.edited_bounds {
        ui.monospace(format!(
            "Building X {:.0}..{:.0} / Z {:.0}..{:.0}",
            b[0],b[1],b[2],b[3]
        ));

        ui.monospace(format!(
            "Overhang: -X {:.0} +X {:.0} -Z {:.0} +Z {:.0}",
            (-b[0]).max(0.),
            (b[1]-LIMIT).max(0.),
            (-b[2]).max(0.),
            (b[3]-LIMIT).max(0.)
        ));
    }

    ui.collapsing("Top-down original MAP",|ui|{
        paint_overview(ui,&report,stride);
    });
}

#[cfg(test)]
mod lck_r49_world_fixed_grid_tests {
    use super::*;

    fn empty()->Model {
        Model {
            positions:Vec::new(),
            rest:Vec::new(),
            object_ids:Vec::new(),
            uvs:Vec::new(),
            cba_tsb:Vec::new(),
            modulation:Vec::new(),
            fill:Vec::new(),
            textured:Vec::new(),
            indices:Vec::new(),
            center:[12.,34.,56.],
            radius:345.0,
        }
    }

    #[test]
    fn ribbons_are_real_nonselectable_meshes(){
        let mut model=empty();

        r49_ribbon(
            &mut model,
            [0.,0.,0.],
            [128.,0.,0.],
            5.0,[255,0,0]
        );

        assert_eq!(model.positions.len(),4);
        assert_eq!(model.indices.len(),6);
        assert!(model.object_ids.iter().all(
            |&id|id==R49_GRID_ID
        ));
    }

    #[test]
    fn grid_does_not_move_camera_center_or_radius(){
        let original=empty();

        for size in [128,160,192,224,256] {
            let model=render_fixed_grid(
                &original,64.0,16,size
            );

            assert_eq!(model.center,original.center);
            assert_eq!(model.radius,original.radius);
            assert!(!model.indices.is_empty());
            assert_eq!(
                original.positions.len(),0
            );
        }
    }

    #[test]
    fn changing_height_moves_real_vertices(){
        let base=empty();

        let low=render_fixed_grid(&base,0.,32,160);
        let high=render_fixed_grid(&base,256.,32,160);

        assert_eq!(
            low.positions.len(),
            high.positions.len()
        );

        assert!(
            low.positions.iter().zip(&high.positions)
                .all(|(a,b)|(b[1]-a[1]-256.).abs()<0.001)
        );
    }
}