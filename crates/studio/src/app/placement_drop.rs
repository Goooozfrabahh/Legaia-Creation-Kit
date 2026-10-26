//! Source-to-viewport placement for existing scene-native objects.
//!
//! Catalog source selection is independent of the displayed viewport.
//! Default placement uses a camera-facing plane exactly at the selected
//! source mesh's F/Frame Selected viewing distance, rather than requiring
//! a raycast hit on native walk-ground. Holding Shift opts into original
//! walk-ground snapping where available. No PS1 image is modified.

use eframe::egui;
use crate::raster::{Camera, Model};
use legaia_engine_scene::scene_assembly::AssembledScene;
use super::{EditorCatalogItem, world_view};

type DropRequest = ((bool, usize), [f32; 3], [f32; 3], Option<(u8, u8, usize)>);

pub(super) fn native_source_key(scene: &AssembledScene, world: &world_view::WorldModel,
                                asset: EditorCatalogItem) -> Option<(bool,usize)> {
    let valid = |key: (bool,usize)| -> bool {
        world.at(key.0,key.1).is_some_and(|d| {
            !d.hidden && d.editor_uid.is_none() && d.vertex_start < d.vertex_end &&
            world.npc(key).is_none_or(|npc| !npc.conditional && npc.drawable && !npc.is_copy)
        })
    };
    let key = match asset {
        EditorCatalogItem::Draw(terrain,index) => (terrain,index),
        EditorCatalogItem::Mesh(slot) => {
            // One mesh may be placed many times. The first available original
            // instance gives it a known PS1 orientation and complete UVs.
            if slot >= scene.env_tmds.len() {return None;}
            scene.placements.iter().enumerate()
                .find(|(idx,d)|d.env_slot==slot && valid((false,*idx)))
                .map(|(idx,_)|(false,idx))
                .or_else(||scene.terrain.iter().enumerate()
                    .find(|(idx,d)|d.env_slot==slot && valid((true,*idx)))
                    .map(|(idx,_)|(true,idx)))?
        },
        _ => return None,
    };
    valid(key).then_some(key)
}

pub(super) fn viewport_pixels(rect: egui::Rect, ppp: f32, max: [usize;2]) -> [usize;2] {
    [((rect.width()*ppp) as usize).clamp(16,max[0].max(16)),
     ((rect.height()*ppp) as usize).clamp(16,max[1].max(16))]
}

pub(super) fn viewport_pixel(rect:egui::Rect,pointer:egui::Pos2,px:[usize;2],mirror:bool)->[f32;2]{
    let x=(pointer.x-rect.left())*px[0] as f32/rect.width().max(1.0);
    let y=(pointer.y-rect.top())*px[1] as f32/rect.height().max(1.0);
    [if mirror {(px[0] as f32-1.0-x).max(0.0)} else {x}, y]
}

fn sub(a:[f32;3],b:[f32;3])->[f32;3]{[a[0]-b[0],a[1]-b[1],a[2]-b[2]]}
fn dot(a:[f32;3],b:[f32;3])->f32{a[0]*b[0]+a[1]*b[1]+a[2]*b[2]}
fn cross(a:[f32;3],b:[f32;3])->[f32;3]{
    [a[1]*b[2]-a[2]*b[1],a[2]*b[0]-a[0]*b[2],a[0]*b[1]-a[1]*b[0]]
}

/// Double-sided Moller-Trumbore (walk-ground can have either winding).
fn triangle_hit(origin:[f32;3],dir:[f32;3],a:[f32;3],b:[f32;3],c:[f32;3])->Option<f32>{
    let ab=sub(b,a);let ac=sub(c,a);
    let p=cross(dir,ac);let determinant=dot(ab,p);
    if determinant.abs()<1e-6 {return None;}
    let inv=1.0/determinant;let tvec=sub(origin,a);
    let u=dot(tvec,p)*inv;
    if !(-0.00001..=1.00001).contains(&u){return None;}
    let q=cross(tvec,ab);let v=dot(dir,q)*inv;
    if v< -0.00001 || u+v>1.00001{return None;}
    let t=dot(ac,q)*inv;
    (t>0.0001 && t.is_finite()).then_some(t)
}

/// Exactly invert the CPU rasterizer's scene-camera yaw/pitch projection.
/// `pixel` is already horizontally unmirrored. The eye origin is the
/// scene-centered virtual camera, not the near plane or the PSX origin.
fn camera_ray(model:&Model,cam:Camera,pixel:[f32;2],px:[usize;2])
    ->Option<([f32;3],[f32;3])>{
    if px[0]==0 || px[1]==0{return None;}
    let fov=45f32.to_radians();
    let f=(px[1] as f32*0.5)/(fov*0.5).tan();
    let distance=model.radius/(fov*0.5).sin()*cam.zoom.max(0.0000001);
    if !f.is_finite() || !distance.is_finite(){return None;}
    let (sy,cy)=cam.yaw.sin_cos();let (sp,cp)=cam.pitch.sin_cos();
    let to_world=|q:[f32;3]|{
        let y=q[1]*cp+q[2]*sp;
        let z1= -q[1]*sp+q[2]*cp;
        [q[0]*cy-z1*sy,y,q[0]*sy+z1*cy]
    };
    let backwards=to_world([0.0,0.0,distance]);
    let target=[0,1,2].map(|i|model.center[i]+cam.target_offset[i]);
    let origin=[0,1,2].map(|i|target[i]-backwards[i]);
    let ray=to_world([(pixel[0]-px[0] as f32*0.5)/f,
                      (pixel[1]-px[1] as f32*0.5)/f,1.0]);
    let mag=dot(ray,ray).sqrt();
    if mag<=1e-9{return None;}
    Some((origin,ray.map(|v|v/mag)))
}

/// Cast against ONLY original collision/walk-ground polygons. The original
/// walk-ground already has u32::MAX object IDs in LCK's model and is not
/// selectable; this avoids snapping onto NPC heads or roof decorations.
fn walk_hit(model:&Model,cam:Camera,pixel:[f32;2],px:[usize;2])->Option<[f32;3]>{
    let (eye,dir)=camera_ray(model,cam,pixel,px)?;
    let mut nearest=f32::INFINITY;
    for tri in model.indices.chunks_exact(3){
        let [a,b,c]=[tri[0] as usize,tri[1] as usize,tri[2] as usize];
        if a>=model.positions.len() || b>=model.positions.len() || c>=model.positions.len(){continue;}
        if model.object_ids.get(a)!=Some(&u32::MAX) ||
            model.object_ids.get(b)!=Some(&u32::MAX) ||
            model.object_ids.get(c)!=Some(&u32::MAX){continue;}
        if let Some(t)=triangle_hit(eye,dir,model.positions[a],model.positions[b],model.positions[c]){
            nearest=nearest.min(t);
        }
    }
    nearest.is_finite().then(||[0,1,2].map(|i|eye[i]+dir[i]*nearest))
}

/// Center the asset on a camera-facing plane at the same distance that
/// Viewport::frame_instance (the F key) would put its camera eye from this
/// mesh's center. This is NOT a fabricated collision plane or floor height:
/// it is an editor-only staging location that tracks the pointer exactly.
fn frame_distance_drop(model:&Model,cam:Camera,pixel:[f32;2],px:[usize;2],
                       radius:f32)->Option<[f32;3]>{
    let (eye,ray)=camera_ray(model,cam,pixel,px)?;
    if !radius.is_finite() || radius<=0.0 {return None;}
    let fov_half=45f32.to_radians()*0.5;
    // Must mirror Viewport::frame_instance's (radius / world radius * 1.8)
    // zoom clamp exactly to preserve its established visual scale.
    let zoom=(radius.max(0.01)/model.radius.max(0.01)*1.8).clamp(0.00015,50.0);
    let distance=model.radius/fov_half.sin()*zoom;
    let (_,forward)=camera_ray(model,cam,
        [px[0] as f32*0.5,px[1] as f32*0.5],px)?;
    let denom=dot(ray,forward);
    if !distance.is_finite() || denom<=0.05 {return None;}
    let t=distance/denom;
    let center=[0,1,2].map(|i|eye[i]+ray[i]*t);
    center.iter().all(|v|v.is_finite()).then_some(center)
}

pub(super) fn prepare_drop(scene:&AssembledScene,world:&world_view::WorldModel,
    asset:EditorCatalogItem,cam:Camera,pixel:[f32;2],px:[usize;2],snap_ground:bool)
    ->Result<DropRequest,String>{
    let key=native_source_key(scene,world,asset).ok_or_else(||
        "This resource has no placeable original mesh in this cell. Cross-cell model/VRAM importing is not yet supported".to_owned())?;
    let model=world.model.as_ref().ok_or("No assembled world model")?;
    let draw=world.at(key.0,key.1).ok_or("Native source not found")?;
    let ground=if snap_ground { walk_hit(model,cam,pixel,px) } else { None };
    let offset=if let Some(hit)=ground {
        // When the artist deliberately asks for snapping, align the mesh's
        // lowest point to original walk-ground (same as Stage I).
        let bottom=model.rest.get(draw.vertex_start..draw.vertex_end)
            .ok_or("Source vertices out of bounds")?
            .iter().map(|p|p[1]).reduce(f32::min).ok_or("Source mesh has no vertices")?;
        [hit[0]-draw.original_center[0],hit[1]-bottom,hit[2]-draw.original_center[2]]
    } else {
        // Works on sky, behind props, over water, and in cells without
        // collision data. The native PS1 orientation is preserved.
        let anchor=frame_distance_drop(model,cam,pixel,px,draw.radius)
            .ok_or("Cannot calculate the F-distance camera placement point")?;
        [0,1,2].map(|i|anchor[i]-draw.original_center[i])
    };
    if offset.iter().any(|v|!v.is_finite() || v.abs()>=1_000_000.0){
        return Err("Drop point exceeds editable scene bounds".into());
    }
    let actor=world.npc(key).map(|n|(n.model_index,n.clip,n.frame));
    Ok((key,offset,[0.0;3],actor))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn walk_hit_is_double_sided_and_ignores_other_surfaces(){
        let a=[-5.0,0.0,-5.0];let b=[5.0,0.0,-5.0];let c=[0.0,0.0,5.0];
        let origin=[0.0,10.0,0.0];let dir=[0.0,-1.0,0.0];
        assert_eq!(triangle_hit(origin,dir,a,b,c),Some(10.0));
        assert_eq!(triangle_hit(origin,dir,c,b,a),Some(10.0));
        assert_eq!(triangle_hit(origin,[0.0,1.0,0.0],a,b,c),None);
    }
    #[test]
    fn frame_distance_uses_same_geometry_as_f_key_and_tracks_cursor(){
        // Scene-centered camera, a F-framed 3-unit asset, 45-degree FOV.
        let mut model=Model {
            positions:vec![],rest:vec![],object_ids:vec![],
            uvs:vec![],cba_tsb:vec![],modulation:vec![],fill:vec![],
            textured:vec![],indices:vec![],center:[0.0;3],radius:120.0,
        };
        let cam=Camera::default();
        let center=frame_distance_drop(&model,cam,[400.0,300.0],[800,600],3.0).unwrap();
        let (eye,forward)=camera_ray(&model,cam,[400.0,300.0],[800,600]).unwrap();
        let depth=dot(sub(center,eye),forward);
        let expected=120.0/(45f32.to_radians()*0.5).sin()*(3.0/120.0*1.8);
        assert!((depth-expected).abs()<0.0001);
        let right=frame_distance_drop(&model,cam,[500.0,300.0],[800,600],3.0).unwrap();
        assert!((dot(sub(right,eye),forward)-expected).abs()<0.0001);
        assert!((right[0]-center[0]).abs()+(right[2]-center[2]).abs()>0.1);
        model.radius=100.0;
        assert!(frame_distance_drop(&model,cam,[400.0,300.0],[800,600],0.0).is_none());
    }
    #[test]
    fn mirrored_pixels_inverse_match_viewport(){
        let rect=egui::Rect::from_min_size(egui::pos2(100.0,40.0),egui::vec2(200.0,100.0));
        assert_eq!(viewport_pixel(rect,egui::pos2(150.0,65.0),[400,200],false),[100.0,50.0]);
        assert_eq!(viewport_pixel(rect,egui::pos2(150.0,65.0),[400,200],true),[299.0,50.0]);
    }
}


/// Foreign geometry placement uses the same camera ray and F-distance plane
/// as native drops; foreign asset geometry is centered at the returned point.
pub(super) fn foreign_drop_center(world:&world_view::WorldModel,cam:Camera,
    pixel:[f32;2],px:[usize;2],radius:f32,bottom:f32,snap:bool)->Result<[f32;3],String>{
    let model=world.model.as_ref().ok_or("No destination world model")?;
    let center=if snap {walk_hit(model,cam,pixel,px)
        .map(|hit|[hit[0],hit[1]-bottom,hit[2]])}else{None}
        .or_else(||frame_distance_drop(model,cam,pixel,px,radius))
        .ok_or("Cannot position foreign original mesh at pointer")?;
    if center.iter().any(|v|!v.is_finite()||v.abs()>=1_000_000.0){
        return Err("Cross-cell drop point is outside editor bounds".into());
    }
    Ok(center)
}
