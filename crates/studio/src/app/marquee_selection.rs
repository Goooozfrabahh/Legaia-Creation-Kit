//! C3 - Active-layer marquee selection.
//! Editor-only screen-space selection. Never modifies PS1 data.

use std::collections::BTreeSet;
use eframe::egui::{self,Pos2,Rect,Ui,Color32,Stroke};
use crate::raster::Model;

#[derive(Default)]
pub(super) struct State{
    start:Option<Pos2>,
    last:Option<Pos2>,
    mods:egui::Modifiers,
}


// Dual-button marquee input, kept separate from the
// existing checkbox-driven primary-button input.
fn dual_mouse_edges(
    primary_down:bool,
    secondary_down:bool,
    primary_pressed:bool,
    secondary_pressed:bool,
    has_started:bool,
)->(bool,bool,bool){
    let both=primary_down && secondary_down;
    let pressed=both && (primary_pressed || secondary_pressed);
    let released=has_started && !both;
    (pressed,both,released)
}

impl State{
    pub fn cancel(&mut self){
        self.start=None;
        self.last=None;
    }

    pub fn update(
        &mut self,ui:&Ui,viewport:Rect,
        enabled:bool,blocked:bool,dual_mouse:bool)->Option<(Rect,egui::Modifiers)>{
        if !enabled || blocked {
            self.cancel();
            return None;
        }

        let (pressed,down,released,pos,mods,alt)=ui.input(|i|(
            i.pointer.primary_pressed(),
            i.pointer.primary_down(),
            i.pointer.primary_released(),
            i.pointer.interact_pos(),
            i.modifiers,
            i.modifiers.alt
        ));
        let (pressed,down,released)=if dual_mouse{
            let (lp,rp,lstart,rstart)=ui.input(|i|(
                i.pointer.primary_down(),
                i.pointer.secondary_down(),
                i.pointer.primary_pressed(),
                i.pointer.secondary_pressed()
            ));
            dual_mouse_edges(
                lp,rp,lstart,rstart,self.start.is_some()
            )
        }else{
            (pressed,down,released)
        };


        if pressed && !alt {
            if let Some(point)=pos.filter(|p|viewport.contains(*p)){
                self.start=Some(point);
                self.last=Some(point);
                self.mods=mods;
            }
        }

        let Some(start)=self.start else{return None;};

        let clamp=|p:Pos2|{
            egui::pos2(
                p.x.clamp(viewport.left(),viewport.right()),
                p.y.clamp(viewport.top(),viewport.bottom())
            )
        };

        if down {
            if let Some(point)=pos{
                self.last=Some(clamp(point));
            }
        }

        let end=self.last.unwrap_or(start);

        if down && (end-start).length_sq()>36.0 {
            let area=Rect::from_two_pos(start,end);
            let painter=ui.painter().with_clip_rect(viewport);

            painter.rect_filled(
                area,0.0,
                Color32::from_rgba_unmultiplied(55,140,255,32)
            );

            let corners=[
                area.left_top(),area.right_top(),
                area.right_bottom(),area.left_bottom()
            ];

            for i in 0..4{
                painter.line_segment(
                    [corners[i],corners[(i+1)%4]],
                    Stroke::new(1.5,Color32::from_rgb(80,165,255))
                );
            }
        }

        if released {
            let small=(end-start).length_sq()<=36.0;
            let area=if small {
                Rect::from_center_size(start,egui::vec2(5.0,5.0))
            }else{
                Rect::from_two_pos(start,end)
            }.intersect(viewport);

            let modifiers=self.mods;
            self.cancel();

            return Some((area,modifiers));
        }

        if !down {
            self.cancel();
        }

        None
    }
}

fn orient(a:Pos2,b:Pos2,c:Pos2)->f32{
    (b.x-a.x)*(c.y-a.y)-(b.y-a.y)*(c.x-a.x)
}

fn inside_triangle(p:Pos2,t:[Pos2;3])->bool{
    let a=orient(t[0],t[1],p);
    let b=orient(t[1],t[2],p);
    let c=orient(t[2],t[0],p);

    let negative=a<0.0||b<0.0||c<0.0;
    let positive=a>0.0||b>0.0||c>0.0;

    !(negative&&positive)
}

fn segments_cross(a:Pos2,b:Pos2,c:Pos2,d:Pos2)->bool{
    let p=orient(a,b,c);
    let q=orient(a,b,d);
    let r=orient(c,d,a);
    let s=orient(c,d,b);

    (p<=0.0&&q>=0.0||p>=0.0&&q<=0.0)
        &&(r<=0.0&&s>=0.0||r>=0.0&&s<=0.0)
}

fn triangle_hits(t:[Pos2;3],area:Rect)->bool{
    let min=egui::pos2(
        t.iter().map(|p|p.x).fold(f32::INFINITY,f32::min),
        t.iter().map(|p|p.y).fold(f32::INFINITY,f32::min)
    );

    let max=egui::pos2(
        t.iter().map(|p|p.x).fold(f32::NEG_INFINITY,f32::max),
        t.iter().map(|p|p.y).fold(f32::NEG_INFINITY,f32::max)
    );

    if !Rect::from_min_max(min,max).intersects(area){
        return false;
    }

    if t.iter().any(|p|area.contains(*p)){
        return true;
    }

    let corners=[
        area.left_top(),area.right_top(),
        area.right_bottom(),area.left_bottom()
    ];

    if corners.iter().any(|p|inside_triangle(*p,t)){
        return true;
    }

    for i in 0..3{
        for j in 0..4{
            if segments_cross(
                t[i],t[(i+1)%3],
                corners[j],corners[(j+1)%4]
            ){
                return true;
            }
        }
    }

    false
}

/// Return all rendered mesh identities touched by the rectangle.
/// Native objects and floor pieces share this exact renderer.
// LCK_C3_PROJECTOR_REPAIR
//
// The collision marquee already uses the exact viewport
// projector. Reuse it for objects and floor tiles.
//
// Instead of independently calculating perspective, collect
// the projected bounds of rendered triangles by object ID.
// This supports native objects, NPCs, original floor tiles,
// and duplicated textured floor pieces.
//
// Deleted geometry is excluded because it has no triangles
// in Model.indices.

fn ids_from_projected_triangles(
    indices:&[u32],
    owners:&[u32],
    screen:&[Option<Pos2>],
    selection:Rect,
)->BTreeSet<u32>{
    let mut bounds=std::collections::BTreeMap::<u32,Rect>::new();

    for triangle in indices.chunks_exact(3){
        let [a,b,c]=[
            triangle[0] as usize,
            triangle[1] as usize,
            triangle[2] as usize
        ];

        let Some(&id)=owners.get(a) else{continue;};

        if id==u32::MAX {
            continue;
        }

        let (
            Some(&Some(p0)),
            Some(&Some(p1)),
            Some(&Some(p2))
        )=(
            screen.get(a),
            screen.get(b),
            screen.get(c)
        )else{continue;};

        // Ignore triangles that have no visible area.
        let min=egui::pos2(
            p0.x.min(p1.x).min(p2.x),
            p0.y.min(p1.y).min(p2.y)
        );

        let max=egui::pos2(
            p0.x.max(p1.x).max(p2.x),
            p0.y.max(p1.y).max(p2.y)
        );

        let rect=Rect::from_min_max(min,max);

        bounds.entry(id)
            .and_modify(|existing|{
                *existing=existing.union(rect);
            })
            .or_insert(rect);
    }

    bounds.into_iter()
        .filter_map(|(id,bounds)|
            bounds.intersects(selection).then_some(id)
        )
        .collect()
}

// LCK_C3_RASTER_PICK_FALLBACK
// Use BOTH projected geometry and the ordinary raster
// click picker. This operates on the active composite Model,
// including floor copies and edited native geometry.
pub(super) fn intersecting_ids(
    ui:&Ui,
    model:&Model,
    view:&super::viewport::Viewport,
    viewport:Rect,
    selection:Rect,
)->BTreeSet<u32>{
    let selection=selection.intersect(viewport);

    if selection.width()<=0.0 || selection.height()<=0.0{
        return BTreeSet::new();
    }

    // Pass one: projected triangle bounds. This selects
    // meshes partly inside the rectangle, including thin
    // objects that might fall between sample points.
    let projector=super::collision_wire::Projector::new(
        ui,viewport,view,model
    );

    let projected=model.positions.iter()
        .map(|&p|projector.project(p))
        .collect::<Vec<_>>();

    let mut found=ids_from_projected_triangles(
        &model.indices,
        &model.object_ids,
        &projected,
        selection
    );

    if !found.is_empty() {return found;}

    // Pass two: sample LCK's exact ordinary-click picker.
    // This uses the same perspective, model triangles and
    // mirror conversion as the working single-click tool.
    let scale=ui.ctx().pixels_per_point();

    let width=((viewport.width()*scale) as usize)
        .clamp(16,900);

    let height=((viewport.height()*scale) as usize)
        .clamp(16,680);

    let columns=((selection.width()/20.0).ceil() as usize)
        .clamp(2,4);

    let rows=((selection.height()/20.0).ceil() as usize)
        .clamp(2,4);

    for row in 0..rows{
        for col in 0..columns{
            let x=selection.left()
                +selection.width()
                    *((col as f32+0.5)/columns as f32);

            let y=selection.top()
                +selection.height()
                    *((row as f32+0.5)/rows as f32);

            let mut px=(x-viewport.left())
                *width as f32/viewport.width().max(1.0);

            if view.world_mirrored{
                px=(width as f32-1.0-px).max(0.0);
            }

            let py=(y-viewport.top())
                *height as f32/viewport.height().max(1.0);

            if let Some(id)=crate::raster::pick_instance(
                model,view.cam,width,height,[px,py]
            ){
                if id!=u32::MAX{
                    found.insert(id);
                }
            }
        }
    }

    found
}

#[cfg(test)]
mod tests{
    use super::*;
    #[test]
    fn dual_mouse_press_order_and_release(){
        assert_eq!(
            dual_mouse_edges(true,false,true,false,false),
            (false,false,false)
        );
        assert_eq!(
            dual_mouse_edges(true,true,false,true,false),
            (true,true,false)
        );
        assert_eq!(
            dual_mouse_edges(true,true,true,false,false),
            (true,true,false)
        );
        assert_eq!(
            dual_mouse_edges(true,true,false,false,true),
            (false,true,false)
        );
        assert_eq!(
            dual_mouse_edges(true,false,false,false,true),
            (false,false,true)
        );
        assert_eq!(
            dual_mouse_edges(false,true,false,false,true),
            (false,false,true)
        );
    }



    #[test]
    fn projector_bounds_select_objects_and_floor_tiles(){
        let indices=[0,1,2,3,4,5,6,7,8];

        let objects=[
            12,12,12,
            0xE000_0010,0xE000_0010,0xE000_0010,
            u32::MAX,u32::MAX,u32::MAX
        ];

        let screen=[
            Some(egui::pos2(10.0,10.0)),
            Some(egui::pos2(30.0,10.0)),
            Some(egui::pos2(10.0,30.0)),

            Some(egui::pos2(50.0,50.0)),
            Some(egui::pos2(70.0,50.0)),
            Some(egui::pos2(50.0,70.0)),

            Some(egui::pos2(50.0,50.0)),
            Some(egui::pos2(70.0,50.0)),
            Some(egui::pos2(50.0,70.0)),
        ];

        let selection=Rect::from_min_max(
            egui::pos2(5.0,5.0),
            egui::pos2(80.0,80.0)
        );

        let selected=ids_from_projected_triangles(
            &indices,&objects,&screen,selection
        );

        assert!(selected.contains(&12));
        assert!(selected.contains(&0xE000_0010));
        assert!(!selected.contains(&u32::MAX));
        assert_eq!(selected.len(),2);
    }

    #[test]
    fn deleted_floor_without_triangles_cannot_be_selected(){
        let indices=[0,1,2];

        let owners=[
            0xE000_0001,0xE000_0001,0xE000_0001,
            0xE000_0002,0xE000_0002,0xE000_0002
        ];

        let screen=[
            Some(egui::pos2(10.0,10.0)),
            Some(egui::pos2(30.0,10.0)),
            Some(egui::pos2(10.0,30.0)),

            Some(egui::pos2(10.0,10.0)),
            Some(egui::pos2(30.0,10.0)),
            Some(egui::pos2(10.0,30.0)),
        ];

        let selected=ids_from_projected_triangles(
            &indices,&owners,&screen,
            Rect::from_min_max(
                egui::pos2(5.0,5.0),
                egui::pos2(35.0,35.0)
            )
        );

        assert!(selected.contains(&0xE000_0001));
        assert!(!selected.contains(&0xE000_0002));
    }



    #[test]
    fn triangle_inside_selection(){
        let tri=[
            egui::pos2(10.0,10.0),
            egui::pos2(70.0,10.0),
            egui::pos2(10.0,70.0)
        ];

        assert!(triangle_hits(
            tri,
            Rect::from_min_max(
                egui::pos2(20.0,20.0),
                egui::pos2(30.0,30.0)
            )
        ));
    }

    #[test]
    fn rectangle_outside_triangle(){
        let tri=[
            egui::pos2(10.0,10.0),
            egui::pos2(20.0,10.0),
            egui::pos2(10.0,20.0)
        ];

        assert!(!triangle_hits(
            tri,
            Rect::from_min_max(
                egui::pos2(60.0,60.0),
                egui::pos2(80.0,80.0)
            )
        ));
    }

    #[test]
    fn rectangle_crosses_triangle_edge(){
        let tri=[
            egui::pos2(0.0,0.0),
            egui::pos2(100.0,0.0),
            egui::pos2(50.0,100.0)
        ];

        assert!(triangle_hits(
            tri,
            Rect::from_min_max(
                egui::pos2(40.0,-5.0),
                egui::pos2(60.0,5.0)
            )
        ));
    }
}
