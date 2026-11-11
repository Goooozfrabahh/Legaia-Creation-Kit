//! Screen-ray dragging for LCK's native world viewport.
//! Matches the retail-scene rasterizer's perspective and mirror.
use eframe::egui::{self,Pos2,Rect,Ui};
use crate::raster::{Camera,Model};

struct Ray {
    eye:[f32;3],
    dir:[f32;3],
}

// LCK_R51_ALIGNMENT
fn ray(
    ui:&Ui,
    rect:Rect,
    model:&Model,
    cam:Camera,
    pos:Pos2,
    mirror:bool
)->Option<Ray>{
    let px=super::placement_drop::viewport_pixels(
        rect,
        ui.ctx().pixels_per_point(),
        [900,680]
    );

    let point=super::placement_drop::viewport_pixel(
        rect,pos,px,mirror
    );

    let (eye,dir)=super::placement_drop::camera_ray(
        model,cam,point,px
    )?;

    Some(Ray{eye,dir})
}

fn on_plane(ray:&Ray,y:f32)->Option<[f32;3]>{
    if ray.dir[1].abs()<0.04{return None;}
    let t=(y-ray.eye[1])/ray.dir[1];
    if !t.is_finite() || t<=0.0 || t>100000000.0 {
        return None;
    }
    Some([0,1,2].map(|k|ray.eye[k]+ray.dir[k]*t))
}

/// Plane movement keeps the *picked point*, not the bounding
/// sphere centre, at the cursor's projected world position.
pub(super) fn horizontal_delta(
    ui:&Ui,rect:Rect,model:&Model,cam:Camera,
    start:Pos2,end:Pos2,pivot:[f32;3],mirror:bool
)->[f32;3]{
    let Some(a)=ray(ui,rect,model,cam,start,mirror)
        else{return [0.;3];};
    let Some(b)=ray(ui,rect,model,cam,end,mirror)
        else{return [0.;3];};

    if let (Some(p),Some(q))=(
        on_plane(&a,pivot[1]),
        on_plane(&b,pivot[1])
    ){
        let dx=q[0]-p[0]; let dz=q[2]-p[2];
        if dx.is_finite() && dz.is_finite() && dx.abs()<=4096.0 && dz.abs()<=4096.0 {
            return [dx,0.0,dz];
        }
    }

    // Near-horizontal view: use a camera-facing plane at
    // the selected piece's actual perspective depth.
    let (sy,cy)=cam.yaw.sin_cos();
    let (sp,cp)=cam.pitch.sin_cos();
    let fwd=[-sy*cp,sp,cy*cp];

    let depth=[0,1,2].iter()
        .map(|&k|(pivot[k]-a.eye[k])*fwd[k])
        .sum::<f32>().max(1.0);

    let point=|r:&Ray|{
        let denom=[0,1,2].iter()
            .map(|&k|r.dir[k]*fwd[k]).sum::<f32>();
        let t=depth/denom.max(0.001);
        [0,1,2].map(|k|r.eye[k]+r.dir[k]*t)
    };

    let p=point(&a);
    let q=point(&b);

    [(q[0]-p[0]).clamp(-4096.0,4096.0),0.0,
     (q[2]-p[2]).clamp(-4096.0,4096.0)]
}

fn vertical_hit(ray:&Ray,pivot:[f32;3])->Option<f32>{
    let dx=pivot[0]-ray.eye[0];
    let dz=pivot[2]-ray.eye[2];
    let denominator=ray.dir[0]*ray.dir[0]
        +ray.dir[2]*ray.dir[2];

    if denominator<0.000001{return None;}

    let t=(dx*ray.dir[0]+dz*ray.dir[2])
        /denominator;

    if !t.is_finite() || t<=0.0{return None;}

    Some(ray.eye[1]+t*ray.dir[1])
}

pub(super) fn vertical_delta(
    ui:&Ui,rect:Rect,model:&Model,cam:Camera,
    start:Pos2,end:Pos2,pivot:[f32;3],mirror:bool
)->f32{
    let (Some(a),Some(b))=(
        ray(ui,rect,model,cam,start,mirror),
        ray(ui,rect,model,cam,end,mirror)
    )else{return 0.0;};

    if let (Some(y0),Some(y1))=(
        vertical_hit(&a,pivot),
        vertical_hit(&b,pivot)
    ){
        let result=y1-y0;
        if result.is_finite() && result.abs()<=4096.0{return result;}
    }

    let units=model.radius/(45f32.to_radians()*0.5).sin()
        *cam.zoom.max(0.0000001)
        *2.0*(45f32.to_radians()*0.5).tan()
        /rect.height().max(1.0);

    (-(end.y-start.y)*units).clamp(-4096.0,4096.0)
}

pub(super) fn snapped(value:f32,step:Option<f32>)->f32{
    if let Some(step)=step.filter(|v|v.is_finite() && *v>=1.0){
        (value/step).round()*step
    }else{
        value
    }
}

pub(super) fn collision_cells(delta:f32,step:Option<f32>)->i32{
    let cells=step.map(|v|
        ((v.max(64.0)/64.0).ceil() as i32).max(1)
    ).unwrap_or(1);
    ((delta/(64.0*cells as f32)).round() as i32)*cells
}

#[cfg(test)]
mod tests{
    use super::*;

    #[test]
    fn snap_modes(){
        assert_eq!(snapped(15.5,None),15.5);
        assert_eq!(snapped(15.5,Some(16.0)),16.0);
        assert_eq!(snapped(-123.0,Some(128.0)),-128.0);
    }

    #[test]
    fn native_collision_grid_steps(){
        assert_eq!(collision_cells(70.0,None),1);
        assert_eq!(collision_cells(70.0,Some(64.0)),1);
        assert_eq!(collision_cells(150.0,Some(128.0)),2);
        assert_eq!(collision_cells(97.0,Some(256.0)),0);
        assert_eq!(collision_cells(150.0,Some(256.0)),4);
    }

    #[test]
    fn flat_plane_intersection(){
        let r=Ray{
            eye:[0.0,100.0,0.0],
            dir:[1.0,-1.0,1.0],
        };
        assert_eq!(on_plane(&r,0.0),
            Some([100.0,0.0,100.0]));
    }
}
