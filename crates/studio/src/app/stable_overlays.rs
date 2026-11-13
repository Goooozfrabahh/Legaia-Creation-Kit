// LCK_V203_CONTINUOUS_OVERLAYS
//
// World-anchored grid and collision visualization.
//
// The 3D world position of every grid line and collision cell
// remains fixed. Only its screen projection changes with camera.
//
// Editor-only display. Does not modify collision, scene,
// project, MAP, native IMG, or selection records.

use std::collections::{BTreeMap,BTreeSet};

use eframe::egui::{
    self,Color32,Pos2,Rect,Stroke,Ui,Vec2
};

use crate::raster::{Camera,Model};

use super::{
    collision_wire::CollisionGrid,
    overlay_style,
    viewport::Viewport,
};

struct Projection {
    rect:Rect,
    w:f32,
    h:f32,
    f:f32,
    distance:f32,
    near:f32,
    center:[f32;3],
    offset:[f32;3],
    sy:f32,
    cy:f32,
    sp:f32,
    cp:f32,
    mirror:bool,
}

impl Projection {
    fn from_parts(
        rect:Rect,
        pixels:[usize;2],
        camera:Camera,
        center:[f32;3],
        radius:f32,
        mirror:bool,
    )->Option<Self>{
        if pixels[0]==0 || pixels[1]==0
            || rect.width()<=0.0 || rect.height()<=0.0
        {
            return None;
        }

        let half=22.5f32.to_radians();
        let distance=radius/half.sin()
            *camera.zoom.max(0.0000001);

        if !distance.is_finite() || distance<=0.0 {
            return None;
        }

        let (sy,cy)=camera.yaw.sin_cos();
        let (sp,cp)=camera.pitch.sin_cos();

        let w=pixels[0] as f32;
        let h=pixels[1] as f32;

        Some(Self{
            rect,w,h,
            f:(h*0.5)/half.tan(),
            distance,
            near:(distance*0.003).max(0.0001),
            center,
            offset:camera.target_offset,
            sy,cy,sp,cp,
            mirror,
        })
    }

    fn new(view:&Viewport,model:&Model)->Option<Self>{
        let (rect,pixels)=view.overlay_projection()?;

        Self::from_parts(
            rect,pixels,view.cam,
            model.center,model.radius,
            view.world_mirrored
        )
    }

    // Identical yaw/pitch and depth convention to the
    // original LCK CPU rasterizer.
    fn camera_point(&self,p:[f32;3])->[f32;3]{
        let x=p[0]-(self.center[0]+self.offset[0]);
        let y=p[1]-(self.center[1]+self.offset[1]);
        let z=p[2]-(self.center[2]+self.offset[2]);

        let x1=x*self.cy+z*self.sy;
        let z1=-x*self.sy+z*self.cy;

        [
            x1,
            y*self.cp-z1*self.sp,
            y*self.sp+z1*self.cp+self.distance
        ]
    }

    fn screen(&self,p:[f32;3])->Option<Pos2>{
        if p[2]<=self.near || !p[2].is_finite() {
            return None;
        }

        let mut x=self.w*0.5+self.f*p[0]/p[2];
        let y=self.h*0.5+self.f*p[1]/p[2];

        if self.mirror {
            x=self.w-1.0-x;
        }

        let result=Pos2::new(
            self.rect.left()+x*self.rect.width()/self.w,
            self.rect.top()+y*self.rect.height()/self.h,
        );

        (result.x.is_finite() && result.y.is_finite())
            .then_some(result)
    }

    fn point(&self,p:[f32;3])->Option<Pos2>{
        self.screen(self.camera_point(p))
    }

    // Liang-Barsky line clipping to the actual displayed
    // raster rectangle. No endpoint has to be inside.
    fn clip_screen(
        &self,
        a:Pos2,
        b:Pos2
    )->Option<[Pos2;2]>{
        let dx=b.x-a.x;
        let dy=b.y-a.y;

        let p=[-dx,dx,-dy,dy];

        let q=[
            a.x-self.rect.left(),
            self.rect.right()-a.x,
            a.y-self.rect.top(),
            self.rect.bottom()-a.y,
        ];

        let mut t0=0.0f32;
        let mut t1=1.0f32;

        for i in 0..4 {
            if p[i].abs()<0.000001 {
                if q[i]<0.0 {
                    return None;
                }

                continue;
            }

            let t=q[i]/p[i];

            if p[i]<0.0 {
                t0=t0.max(t);
            } else {
                t1=t1.min(t);
            }

            if t0>t1 {
                return None;
            }
        }

        Some([
            Pos2::new(a.x+dx*t0,a.y+dy*t0),
            Pos2::new(a.x+dx*t1,a.y+dy*t1)
        ])
    }

    // Clip the line against the near plane in 3D first.
    // Only then project and clip its screen coordinates.
    // This prevents long grid lines from disappearing when
    // a single original endpoint moves behind the camera.
    fn segment(
        &self,
        a:[f32;3],
        b:[f32;3],
    )->Option<[Pos2;2]>{
        let mut a=self.camera_point(a);
        let mut b=self.camera_point(b);

        let near=self.near*1.001+0.0001;

        if a[2]<near && b[2]<near {
            return None;
        }

        if a[2]<near {
            let t=(near-a[2])/(b[2]-a[2]);

            for k in 0..3 {
                a[k]+=t*(b[k]-a[k]);
            }
        }

        if b[2]<near {
            let t=(near-b[2])/(a[2]-b[2]);

            for k in 0..3 {
                b[k]+=t*(a[k]-b[k]);
            }
        }

        let a=self.screen(a)?;
        let b=self.screen(b)?;

        self.clip_screen(a,b)
    }

    fn line(
        &self,
        painter:&egui::Painter,
        a:[f32;3],
        b:[f32;3],
        stroke:Stroke,
    ){
        if let Some(points)=self.segment(a,b) {
            painter.line_segment(points,stroke);
        }
    }

    fn cell_width(&self,depth:f32)->f32{
        if depth<=self.near {
            return f32::INFINITY;
        }

        self.f*64.0/depth
            *self.rect.width()/self.w
    }
}

fn color_with_alpha(rgb:[u8;3],alpha:u8)->Color32 {
    Color32::from_rgba_unmultiplied(
        rgb[0],rgb[1],rgb[2],alpha
    )
}

fn brighten(rgb:[u8;3],factor:f32)->[u8;3]{
    rgb.map(|channel|
        ((channel as f32)*factor)
            .clamp(0.0,255.0) as u8
    )
}

pub(super) fn paint_grid(
    ui:&Ui,
    view:&Viewport,
    model:&Model,
    height:f32,
    stride:usize,
    extended_side:usize,
){
    let Some(projection)=Projection::new(view,model)
    else {
        return;
    };

    let style=overlay_style::snapshot();
    let opacity=style.grid_opacity();

    if opacity<=0.00001 {
        return;
    }

    let painter=ui.painter()
        .with_clip_rect(projection.rect);

    let side=match extended_side {
        160|192|224|256=>extended_side,
        _=>128,
    };

    let stride=stride.clamp(1,32);
    let half=((side-128)/2) as f32*128.0;

    let min=-half;
    let max=16384.0+half;

    let y=height+3.0;

    // Screen-pixel widths do not shrink into dashes
    // at a distance. Grid coordinates never move.
    let line_alpha=(
        78.0+177.0*opacity
    ).clamp(0.0,255.0) as u8;

    let minor=Stroke::new(
        1.9,
        color_with_alpha(
            style.grid_color,
            line_alpha
        )
    );

    let major=Stroke::new(
        2.8,
        color_with_alpha(
            style.grid_major_color(),
            line_alpha
        )
    );

    for i in (0..=side).step_by(stride) {
        let c=min+i as f32*128.0;

        let stroke=if i%32==0 {
            major
        }else{
            minor
        };

        projection.line(
            &painter,
            [c,y,min],
            [c,y,max],
            stroke
        );

        projection.line(
            &painter,
            [min,y,c],
            [max,y,c],
            stroke
        );
    }

    // Preserve the real retail boundary even when
    // the authoring grid is increased beyond 128.
    let retail=[
        [0.0,y,0.0],
        [16384.0,y,0.0],
        [16384.0,y,16384.0],
        [0.0,y,16384.0]
    ];

    let red=Stroke::new(
        2.8,
        Color32::from_rgba_unmultiplied(
            255,91,86,line_alpha
        )
    );

    for i in 0..4 {
        projection.line(
            &painter,
            retail[i],
            retail[(i+1)%4],
            red
        );
    }

    if side>128 {
        let outer=[
            [min,y,min],
            [max,y,min],
            [max,y,max],
            [min,y,max],
        ];

        let cyan=Stroke::new(
            2.8,
            Color32::from_rgba_unmultiplied(
                50,225,245,line_alpha
            )
        );

        for i in 0..4 {
            projection.line(
                &painter,
                outer[i],
                outer[(i+1)%4],
                cyan
            );
        }
    }
}

pub(super) fn paint_collision(
    ui:&Ui,
    view:&Viewport,
    model:&Model,
    grid:&CollisionGrid,
    walls:&BTreeMap<(i32,i32),bool>,
    heights:&BTreeMap<(i32,i32),i32>,
    selected:&BTreeSet<(i32,i32)>,
){
    let Some(projection)=Projection::new(view,model)
    else {
        return;
    };

    let style=overlay_style::snapshot();
    let opacity=style.collision_opacity();

    if opacity<=0.00001 && selected.is_empty() {
        return;
    }

    let painter=ui.painter()
        .with_clip_rect(projection.rect);

    // No global 1800-box cap. Every native occupied
    // wall cell participates in visibility assessment.
    for z in 0..256i32 {
        for x in 0..256i32 {
            let cell=(x,z);

            let native=grid.native_wall(x,z);
            let occupied=walls.get(&cell)
                .copied()
                .unwrap_or(native);

            if !occupied {
                continue;
            }

            let selected_box=selected.contains(&cell);

            if opacity<=0.00001 && !selected_box {
                continue;
            }

            let base_color=if selected_box {
                [255,220,75]
            }else if native {
                style.collision_color
            }else{
                style.added_collision_color()
            };

            let x0=x as f32*64.0;
            let z0=(z-2) as f32*64.0;

            let y0=grid.ground_y(
                x0+32.0,z0+32.0
            ) +heights.get(&cell)
                .copied()
                .unwrap_or(0) as f32;

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

            let center=[
                x0+32.0,
                y0+54.0,
                z0+32.0
            ];

            let camera_center=
                projection.camera_point(center);

            let centre_pos=
                projection.screen(camera_center);

            let projected_width=
                projection.cell_width(camera_center[2]);

            // Screen-space culling only. No world-distance
            // cutoff or fixed number-of-boxes budget.
            if let Some(pos)=centre_pos {
                if !projection.rect
                    .expand(110.0)
                    .contains(pos)
                    && projected_width<110.0
                {
                    continue;
                }
            }else if camera_center[2]
                <projection.near-150.0
            {
                continue;
            }

            let edge_alpha=if selected_box {
                255
            }else{
                (92.0+163.0*opacity)
                    .clamp(0.0,255.0) as u8
            };

            let fill_alpha=if selected_box {
                66
            }else{
                (opacity*205.0)
                    .clamp(0.0,255.0) as u8
            };

            let edge=Stroke::new(
                if selected_box {2.8}else{1.7},
                color_with_alpha(
                    base_color,edge_alpha
                )
            );

            // Below the minimum projected size, separate
            // 3D edges are physically subpixel. Keep an
            // occupancy indicator rather than hiding it.
            if projected_width<3.0 {
                if let Some(pos)=centre_pos {
                    if projection.rect.contains(pos) {
                        let size=if selected_box {
                            4.0
                        }else{
                            2.6
                        };

                        painter.rect_filled(
                            Rect::from_center_size(
                                pos,
                                Vec2::splat(size)
                            ),
                            0.0,
                            color_with_alpha(
                                base_color,
                                edge_alpha
                            )
                        );
                    }
                }

                continue;
            }

            // Modest distance: show a visible small box.
            // Close distance: full twelve-edge 3D box.
            //
            // Faces use fixed directional shading, not
            // camera-dependent selection or world edits.
            for (indices,shade) in [
                ([4usize,5,6,7],1.12f32),
                ([0usize,1,5,4],0.78),
                ([1usize,2,6,5],0.94),
                ([2usize,3,7,6],0.69),
                ([3usize,0,4,7],0.85),
            ] {
                let mut points=[Pos2::ZERO;4];
                let mut valid=true;

                for i in 0..4 {
                    if let Some(pt)=projection.point(
                        corners[indices[i]]
                    ){
                        points[i]=pt;
                    }else{
                        valid=false;
                        break;
                    }
                }

                if !valid {
                    continue;
                }

                // Avoid giant fill polygons at a
                // near-plane intersection. The edges
                // below remain correctly clipped.
                if points.iter().any(|pt|
                    pt.x.abs()>100000.0
                    || pt.y.abs()>100000.0
                ){
                    continue;
                }

                let tinted=brighten(base_color,shade);

                painter.add(
                    egui::Shape::convex_polygon(
                        points.to_vec(),
                        color_with_alpha(
                            tinted,fill_alpha
                        ),
                        Stroke::NONE
                    )
                );
            }

            // Always draw these from the same fixed
            // native 64x64 coordinates, including the
            // segments crossing the near plane.
            for (a,b) in [
                (0usize,1usize),(1,2),(2,3),(3,0),
                (4,5),(5,6),(6,7),(7,4),
                (0,4),(1,5),(2,6),(3,7)
            ] {
                projection.line(
                    &painter,
                    corners[a],
                    corners[b],
                    edge
                );
            }
        }
    }
}

#[cfg(test)]
mod v203_overlay_tests {
    use super::*;

    fn projection()->Projection {
        let mut camera=Camera::default();
        camera.yaw=0.0;
        camera.pitch=0.0;
        camera.zoom=1.0;

        Projection::from_parts(
            Rect::from_min_size(
                Pos2::ZERO,
                Vec2::new(900.0,500.0)
            ),
            [900,500],
            camera,
            [0.0;3],
            1000.0,
            false,
        ).unwrap()
    }

    #[test]
    fn v203_long_line_clips_as_one_continuous_segment() {
        let p=projection();

        let points=p.segment(
            [-10000.0,0.0,0.0],
            [10000.0,0.0,0.0],
        ).unwrap();

        assert!(
            (points[1].x-points[0].x).abs()
                >p.rect.width()*0.95
        );

        assert!(points.iter().all(|pt|
            p.rect.contains(*pt)
                || (pt.x-p.rect.right()).abs()<0.01
        ));
    }

    #[test]
    fn v203_near_plane_intersection_keeps_visible_line() {
        let p=projection();

        let a=[-100.0,0.0,-p.distance-100.0];
        let b=[100.0,0.0,0.0];

        let result=p.segment(a,b);

        assert!(result.is_some());
    }

    #[test]
    fn v203_distant_collision_has_finite_marker_size() {
        let p=projection();

        let width=p.cell_width(p.distance*1000.0);

        assert!(width.is_finite());
        assert!(width<3.0);

        // The render path assigns a 2.6-pixel marker
        // instead of removing these occupied cells.
        let marker=2.6f32;

        assert!(marker>width);
    }
}