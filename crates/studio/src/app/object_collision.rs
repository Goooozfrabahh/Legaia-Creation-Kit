//! LCK R1: transform-attached geometric collision DRAFT for the selected draw.
//!
//! Every face comes from that object's own current rendered vertices, not from
//! screen-visible or nearby MAP walls. Original objects, native duplicates and
//! donor imports use the same method. WorldModel keeps immutable rest vertices;
//! its move/rotate/scale functions update `positions`, so this outline tracks
//! those transformations without maintaining a second drifting mesh.
//!
//! Important: a rendered triangle is NOT proof of PS1 collision or walkability.
//! This is an editable-data foundation for later collision authoring and native
//! MAP/heightfield encoding. It never writes an IMG or silently marks a floor
//! as traversable. The real MAP overlay remains completely separate.

use eframe::egui::{Align2, Color32, FontId, Pos2, Rect, Stroke, Ui, Vec2};
use crate::raster::Model;
use super::{collision_wire::Projector, viewport::Viewport, world_view::DrawInfo};

const MAX_DRAWN_FACES: usize = 1200;

/// Saved generated definition, independent of the original MAP wall overlay
/// and of any later selection change while Properties is pinned to a UID.
pub(super) fn paint_generated(ui:&Ui,rect:Rect,view:&Viewport,model:&Model,draw:&DrawInfo,
    generated:&super::collision_generation::GeneratedCollision) {
    if draw.hidden {return;}
    type Faces=std::sync::Arc<Vec<([[f32;3];3],bool)>>;
    let cache=eframe::egui::Id::new(("generated_collision_wireframe",&generated.mesh_sha256,generated.pivot.map(f32::to_bits),
        generated.band_height.to_bits(),generated.min_face_area.to_bits(),draw.offset.map(f32::to_bits),
        draw.rotation.map(f32::to_bits),draw.scale.map(f32::to_bits)));
    let faces=ui.ctx().data_mut(|d|d.get_temp::<Result<Faces,String>>(cache)).unwrap_or_else(||{
        let result=generated.preview_faces(draw.offset,draw.rotation,draw.scale).map(std::sync::Arc::new);
        ui.ctx().data_mut(|d|d.insert_temp(cache,result.clone()));result
    });
    let painter=ui.painter_at(rect);
    let faces=match faces {Ok(faces)=>faces,Err(error)=>{
        painter.text(rect.min+Vec2::splat(12.0),Align2::LEFT_TOP,format!("Generated collision preview unavailable: {error}"),FontId::monospace(11.0),Color32::LIGHT_RED);return;
    }};
    let projector=Projector::new(ui,rect,view,model);let mut drawn=0;
    for (tri,steep) in faces.iter() {
        if drawn>=MAX_DRAWN_FACES {break;}
        let [Some(a),Some(b),Some(c)]=tri.map(|p|projector.project(p)) else {continue;};
        let bounds=Rect::from_min_max(Pos2::new(a.x.min(b.x).min(c.x),a.y.min(b.y).min(c.y)),
            Pos2::new(a.x.max(b.x).max(c.x),a.y.max(b.y).max(c.y)));
        if !bounds.intersects(rect) {continue;}
        let color=if *steep {Color32::from_rgb(228,96,255)}else{Color32::from_rgb(52,255,152)};
        let stroke=Stroke::new(1.0_f32,color);
        for line in [[a,b],[b,c],[c,a]] {painter.line_segment(line,stroke);}
        drawn+=1;
    }
    let label=format!("Saved generated collision DRAFT • {} faces shown / {} • purple walls, green floor candidates\nGeometry review only; manual grid corrections are in Properties. Native export remains guarded.",drawn,faces.len());
    let label_rect=Rect::from_min_size(rect.min+Vec2::splat(8.0),Vec2::new(rect.width().min(720.0)-16.0,44.0));
    painter.rect_filled(label_rect,3.0,Color32::from_black_alpha(205));
    painter.text(label_rect.min+Vec2::splat(5.0),Align2::LEFT_TOP,label,FontId::monospace(10.0),Color32::WHITE);
}

#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub(super) enum Surface { Horizontal, Steep }

/// Classification uses the transformed normal, so tilting a floor onto its
/// side correctly makes it a steep candidate rather than retaining floor type.
fn surface(tri: [[f32;3];3]) -> Option<Surface> {
    if tri.iter().flatten().any(|v|!v.is_finite()) { return None; }
    let u=[0,1,2].map(|k|tri[1][k]-tri[0][k]);
    let v=[0,1,2].map(|k|tri[2][k]-tri[0][k]);
    let n=[u[1]*v[2]-u[2]*v[1],u[2]*v[0]-u[0]*v[2],u[0]*v[1]-u[1]*v[0]];
    let size=(n[0]*n[0]+n[1]*n[1]+n[2]*n[2]).sqrt();
    if size < 0.0001 || !size.is_finite() { return None; }
    if n[1].abs()/size >= 0.70 { Some(Surface::Horizontal) }
    else { Some(Surface::Steep) }
}

/// Visit only faces whose THREE vertices belong to this *specific* instance.
/// Do not infer object ownership from nearby native MAP wall samples. A single
/// selected object is processed lazily: no world-sized collider clones are
/// allocated on each frame, and no other object is ever modified.
fn each_face(model:&Model,draw:&DrawInfo,mut accept:impl FnMut([[f32;3];3],Surface)) -> (usize,usize) {
    if draw.hidden || draw.vertex_start>=draw.vertex_end ||
        draw.vertex_end>model.positions.len() {return (0,0);}
    let mut horizontal=0;let mut steep=0;
    for ids in model.indices.chunks_exact(3) {
        if !ids.iter().all(|&i| {
            let i=i as usize;
            i>=draw.vertex_start && i<draw.vertex_end
        }) {continue;}
        let face=[model.positions[ids[0] as usize],model.positions[ids[1] as usize],
            model.positions[ids[2] as usize]];
        let Some(kind)=surface(face) else {continue;};
        match kind {Surface::Horizontal=>horizontal+=1,Surface::Steep=>steep+=1}
        accept(face,kind);
    }
    (horizontal,steep)
}

/// Draw independent geometry-derived candidate faces over the raster image.
/// Cyan/orange MAP walls are from the actual game and are not repurposed here.
/// Purple: steep faces. Green: near-horizontal faces (NOT verified walkable).
/// Both are an editor visualization only, with native export still blocked.
pub(super) fn paint(ui:&Ui,rect:Rect,view:&Viewport,model:&Model,draw:&DrawInfo) {
    let projector=Projector::new(ui,rect,view,model);
    let painter=ui.painter().with_clip_rect(rect);
    let mut drawn=0usize;
    let (horizontal,steep)=each_face(model,draw,|tri,kind|{
        if drawn>=MAX_DRAWN_FACES {return;}
        let projected=tri.map(|v|projector.project(v));
        let [Some(a),Some(b),Some(c)]=projected else {return;};
        let min=Pos2::new(a.x.min(b.x).min(c.x),a.y.min(b.y).min(c.y));
        let max=Pos2::new(a.x.max(b.x).max(c.x),a.y.max(b.y).max(c.y));
        if !Rect::from_min_max(min,max).intersects(rect) {return;}
        let color=match kind {
            Surface::Horizontal=>Color32::from_rgb(52,255,152),
            Surface::Steep=>Color32::from_rgb(228,96,255),
        };
        let line=Stroke::new(1.0,color);
        painter.line_segment([a,b],line);
        painter.line_segment([b,c],line);
        painter.line_segment([c,a],line);
        drawn+=1;
    });
    let total=horizontal+steep;
    let label=if draw.editor_uid.is_none() {"original cell object"}
        else if draw.source_ref==Some((false,usize::MAX)) {"imported Game-wide asset"}
        else {"duplicated cell object"};
    let text_rect=Rect::from_min_size(Pos2::new(rect.left()+7.0,rect.bottom()-68.0),
        Vec2::new(rect.width().min(560.0)-7.0,62.0)).intersect(rect);
    painter.rect_filled(text_rect,3.0,Color32::from_black_alpha(205));
    let label1=format!("Selected mesh collision DRAFT | {label}");
    let label2=format!("{horizontal} horizontal / {steep} steep faces; {drawn} of {total} drawn");
    painter.text(text_rect.min+Vec2::new(7.0,5.0),Align2::LEFT_TOP,
        label1,FontId::monospace(11.0),Color32::WHITE);
    painter.text(text_rect.min+Vec2::new(7.0,23.0),Align2::LEFT_TOP,
        label2,FontId::monospace(10.0),Color32::LIGHT_GREEN);
    painter.text(text_rect.min+Vec2::new(7.0,40.0),Align2::LEFT_TOP,
        "NOT native MAP collision or IMG export; geometry-following candidate only",
        FontId::monospace(10.0),Color32::YELLOW);
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::world_view::DrawInfo;

    fn draw(id:u32,start:usize,end:usize)->DrawInfo {
        DrawInfo {id,terrain:false,index:id as usize,center:[0.0;3],radius:1.0,
            original_center:[0.0;3],offset:[0.0;3],rotation:[0.0;3],scale:[1.0;3],
            vertex_start:start,vertex_end:end,source_ref:None,editor_uid:None,
            display_name:None,hidden:false,hidden_triangles:Vec::new()}
    }
    fn model()->Model {
        let positions=vec![[0.0,0.0,0.0],[1.0,0.0,0.0],[0.0,0.0,1.0],
            [5.0,0.0,0.0],[5.0,2.0,0.0],[5.0,0.0,2.0]];
        Model { rest:positions.clone(),positions,indices:vec![0,1,2,3,4,5],
            object_ids:vec![0,0,0,1,1,1],uvs:Vec::new(),cba_tsb:Vec::new(),
            modulation:Vec::new(),fill:Vec::new(),textured:Vec::new(),
            center:[0.0;3],radius:10.0 }
    }
    #[test]
    fn near_horizontal_and_steep_triangles_are_distinct(){
        assert_eq!(surface([[0.0,0.0,0.0],[1.0,0.0,0.0],[0.0,0.0,1.0]]),Some(Surface::Horizontal));
        assert_eq!(surface([[0.0,0.0,0.0],[0.0,1.0,0.0],[0.0,0.0,1.0]]),Some(Surface::Steep));
        assert_eq!(surface([[0.0;3];3]),None);
    }
    #[test]
    fn only_selected_instance_faces_are_used_even_when_meshes_overlap(){
        let mesh=model();
        let mut coords=Vec::new();
        assert_eq!(each_face(&mesh,&draw(0,0,3),|tri,_|coords.push(tri)),(1,0));
        assert_eq!(coords.len(),1);
        assert_eq!(coords[0][0],[0.0,0.0,0.0]);
        assert_eq!(each_face(&mesh,&draw(1,3,6),|_,_|{}),(0,1));
    }
    #[test]
    fn modified_instance_vertices_change_candidate_geometry_not_its_neighbor(){
        let mut mesh=model();
        let other=mesh.positions[3..6].to_vec();
        for p in &mut mesh.positions[0..3] {p[0]+=128.0;p[1]*=2.0;}
        let mut faces=Vec::new();
        each_face(&mesh,&draw(0,0,3),|tri,_|faces.push(tri));
        assert_eq!(faces[0][0],[128.0,0.0,0.0]);
        assert_eq!(&mesh.positions[3..6],other.as_slice());
    }
}
