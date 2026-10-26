//! Automatic, instance-local collision proposals. Mesh geometry is authored
//! evidence, never ownership of retail walls or a verified walkable surface.
use super::{collision_recipes::{digest,Evidence,Recipe},world_view::WorldModel};
use serde::{Serialize,Deserialize};
use std::collections::BTreeSet;

#[derive(Clone,Debug,PartialEq,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct GeneratedCollision {
    pub version:u32,
    pub mesh_sha256:String,
    /// Exact rest-space faces from this draw, not its bounding box.
    pub triangles:Vec<[[f32;3];3]>,
    pub pivot:[f32;3],
    /// Heuristic review band above the mesh's lowest point, in PS1 units.
    /// This is not a recovered player height or native terrain sample.
    pub band_height:f32,
    pub min_face_area:f32,
    pub native_inspection:String,
}

/// Geometry-only donors have no verified MAP placement. Preserve their real
/// TMD provenance instead of fabricating a MAP identity or native collider.
#[derive(Clone,Debug,PartialEq,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct MeshOnlyRecipe {
    pub uid:u64,
    pub scene:String,
    pub kind:String,
    pub record:usize,
    pub tmd_sha256:String,
    pub generated:GeneratedCollision,
}
impl MeshOnlyRecipe {
    pub fn validate(&self)->Result<(),String> {
        if self.uid==0 || self.scene.is_empty() || self.scene.len()>128 ||
            !matches!(self.kind.as_str(),"mesh"|"placement"|"terrain") ||
            self.tmd_sha256.len()!=64 || !self.tmd_sha256.bytes().all(|b|b.is_ascii_hexdigit()) {
            return Err("Invalid mesh-only collision provenance".into());
        }
        self.generated.validate()
    }
}

impl GeneratedCollision {
    pub fn validate(&self)->Result<(),String> {
        if self.version!=1 || self.mesh_sha256.len()!=64 || !self.mesh_sha256.bytes().all(|b|b.is_ascii_hexdigit()) ||
            self.triangles.is_empty() || self.triangles.len()>16000 ||
            self.triangles.iter().flatten().flatten().chain(&self.pivot).any(|v|!v.is_finite()||v.abs()>=1_000_000.0) ||
            !self.band_height.is_finite() || !(1.0..=1024.0).contains(&self.band_height) ||
            !self.min_face_area.is_finite() || !(0.0..=4096.0).contains(&self.min_face_area) || self.native_inspection.len()>2048 {
            return Err("Invalid generated collision definition".into());
        }
        if digest(&serde_json::to_vec(&self.triangles).map_err(|e|e.to_string())?)!=self.mesh_sha256 {
            return Err("Generated collision mesh fingerprint changed".into());
        }
        Ok(())
    }

    pub fn transformed(&self,offset:[f32;3],rotation:[f32;3],scale:[f32;3])->Result<Vec<[[f32;3];3]>,String> {
        self.validate()?;
        if offset.iter().chain(&rotation).chain(&scale).any(|v|!v.is_finite()) ||
            scale.iter().any(|v|!(0.05..=20.0).contains(v)) {return Err("Invalid generated collision transform".into());}
        Ok(self.triangles.iter().map(|tri|tri.map(|p|super::world_view::editor_rotate_vertex(p,self.pivot,offset,rotation,scale))).collect())
    }

    /// Conservative triangle/grid intersection after clipping to a review
    /// band. No hull, flood fill or AABB fill closes openings between faces.
    pub fn wall_cells(&self,offset:[f32;3],rotation:[f32;3],scale:[f32;3])->Result<BTreeSet<[i32;2]>,String> {
        let triangles=self.transformed(offset,rotation,scale)?;
        let bottom=triangles.iter().flatten().map(|p|p[1]).fold(f32::INFINITY,f32::min);
        let top=bottom+self.band_height*scale[1];
        let mut cells=BTreeSet::new();let mut work=0usize;
        for tri in triangles {
            let Some((area,vertical))=face_measure(tri) else {continue;};
            if area<self.min_face_area || !vertical {continue;}
            let clipped=clip_height(&clip_height(&tri,bottom,true),top,false);
            if clipped.len()<3 {continue;}
            let poly:Vec<_>=clipped.iter().map(|p|[p[0],p[2]]).collect();
            let minx=poly.iter().map(|p|p[0]).fold(f32::INFINITY,f32::min);
            let maxx=poly.iter().map(|p|p[0]).fold(f32::NEG_INFINITY,f32::max);
            let minz=poly.iter().map(|p|p[1]).fold(f32::INFINITY,f32::min);
            let maxz=poly.iter().map(|p|p[1]).fold(f32::NEG_INFINITY,f32::max);
            let x0=(minx/64.0).floor() as i32;let x1=(maxx/64.0).floor() as i32;
            let z0=(minz/64.0).floor() as i32+2;let z1=(maxz/64.0).floor() as i32+2;
            if x0<0 || z0<0 || x1>255 || z1>255 {return Err("Generated collision crosses native MAP boundaries".into());}
            work+=(x1-x0+1) as usize*(z1-z0+1) as usize;
            if work>2_000_000 {return Err("Generated collision exceeds triangle/grid intersection budget".into());}
            for z in z0..=z1 {for x in x0..=x1 {
                let min=[x as f32*64.0,(z-2) as f32*64.0];let max=[min[0]+64.0,min[1]+64.0];
                if polygon_hits_cell(&poly,min,max) {cells.insert([x,z]);}
            }}
        }
        Ok(cells)
    }
    pub fn surface_counts(&self)->(usize,usize,usize) {
        let mut floor=0;let mut wall=0;let mut decorative=0;
        for tri in &self.triangles {match face_measure(*tri) {
            Some((area,_)) if area<self.min_face_area=>decorative+=1,
            Some((_,true))=>wall+=1,Some((_,false))=>floor+=1,None=>decorative+=1,
        }}
        (floor,wall,decorative)
    }

    /// Properties wireframe uses the same saved faces, classification and
    /// review band as generation. Floor candidates stay visibly distinct.
    pub fn preview_faces(&self,offset:[f32;3],rotation:[f32;3],scale:[f32;3])->Result<Vec<([[f32;3];3],bool)>,String> {
        let triangles=self.transformed(offset,rotation,scale)?;
        let bottom=triangles.iter().flatten().map(|p|p[1]).fold(f32::INFINITY,f32::min);
        let top=bottom+self.band_height*scale[1];let mut faces=Vec::new();
        for tri in triangles {
            let Some((area,steep))=face_measure(tri) else {continue;};
            if area<self.min_face_area {continue;}
            let polygon=if steep {clip_height(&clip_height(&tri,bottom,true),top,false)}else{tri.to_vec()};
            for i in 1..polygon.len().saturating_sub(1) {faces.push(([polygon[0],polygon[i],polygon[i+1]],steep));}
        }
        Ok(faces)
    }
}

fn face_measure(tri:[[f32;3];3])->Option<(f32,bool)> {
    let u=[0,1,2].map(|k|tri[1][k]-tri[0][k]);let v=[0,1,2].map(|k|tri[2][k]-tri[0][k]);
    let n=[u[1]*v[2]-u[2]*v[1],u[2]*v[0]-u[0]*v[2],u[0]*v[1]-u[1]*v[0]];
    let length=n.iter().map(|v|v*v).sum::<f32>().sqrt();
    if !length.is_finite() || length<0.0001 {None} else {Some((length*0.5,n[1].abs()/length<0.70))}
}
fn clip_height(input:&[[f32;3]],height:f32,above:bool)->Vec<[f32;3]> {
    let mut out=Vec::new();if input.is_empty(){return out;}
    for i in 0..input.len() {
        let a=input[i];let b=input[(i+1)%input.len()];
        let inside=|p:[f32;3]|if above {p[1]>=height}else{p[1]<=height};
        let ia=inside(a);let ib=inside(b);
        if ia {out.push(a);}
        if ia!=ib {let t=(height-a[1])/(b[1]-a[1]);out.push([0,1,2].map(|k|a[k]+t*(b[k]-a[k])));}
    }
    out
}
fn polygon_hits_cell(poly:&[[f32;2]],min:[f32;2],max:[f32;2])->bool {
    // Convex polygon clipping also handles the line projection of a vertical
    // face. A surviving segment means actual geometry meets this cell.
    let mut points=poly.to_vec();
    for (axis,bound,greater) in [(0,min[0],true),(0,max[0],false),(1,min[1],true),(1,max[1],false)] {
        if points.is_empty(){return false;}
        let mut next=Vec::new();
        for i in 0..points.len() {
            let a=points[i];let b=points[(i+1)%points.len()];
            let inside=|p:[f32;2]|if greater {p[axis]>=bound}else{p[axis]<=bound};
            let ia=inside(a);let ib=inside(b);
            if ia {next.push(a);}
            if ia!=ib {let t=(bound-a[axis])/(b[axis]-a[axis]);next.push([a[0]+t*(b[0]-a[0]),a[1]+t*(b[1]-a[1])]);}
        }
        points=next;
    }
    !points.is_empty()
}

impl WorldModel {
    pub fn ensure_foreign_mesh_collision(&mut self,key:(bool,usize))->Result<(),String> {
        if self.collision.restoring {return Ok(());}
        let draw=self.at(key.0,key.1).ok_or("Foreign collision target absent")?;
        let uid=draw.editor_uid.ok_or("Foreign collision UID absent")?;
        if self.collision.recipes.iter().any(|a|a.owner==super::collision_recipes::ObjectId::Created(uid)) ||
            self.collision.mesh_recipes.contains_key(&uid) {return Ok(());}
        let foreign=self.foreign.iter().find(|f|f.uid==uid).ok_or("Foreign resource absent")?;
        let generated=self.mesh_collision_geometry(key,"No independently verified native MAP collider for this donor; exact original TMD retained")?;
        let recipe=MeshOnlyRecipe{uid,scene:foreign.source.clone(),kind:foreign.kind.clone(),record:foreign.record,
            tmd_sha256:digest(&foreign.raw_tmd),generated};
        recipe.validate()?;
        self.collision.mesh_recipes.insert(uid,recipe);
        if super::scene_instances::serialize(&foreign.source,self).len()>2_000_000 {
            self.collision.mesh_recipes.remove(&uid);return Err("Automatic collision exceeds scene file safety limit".into());
        }
        Ok(())
    }
    /// Default only for new native World Objects with verifiable provenance.
    /// Existing edited recipes and deliberately empty copied overrides win.
    pub fn ensure_automatic_collision(&mut self,key:(bool,usize))->Result<(),String> {
        if self.collision.restoring {return Ok(());}
        let Some(owner)=self.collision.owner(self,key) else {return Ok(());};
        if self.collision.recipes.iter().any(|a|a.owner==owner) {return Ok(());}
        let Some(source)=self.collision.source(self,key).cloned() else {return Ok(());};
        let recipe=self.automatic_collision_recipe(key)?;
        let generated=recipe.generated.as_ref().ok_or("Missing generated geometry")?;
        let cache_key=format!("{}:{}:{}",source.map_sha256,source.object.descriptor_id,generated.mesh_sha256);
        let mut recipe=recipe;
        if let Some(base)=self.collision.generated_bases.get(&cache_key) {
            // Full equality includes pivot and inspection: equal meshes in a
            // different pose/source never borrow incompatible coordinates.
            if Some(base)==recipe.generated.as_ref() {recipe.generated=Some(base.clone());}
        }
        self.set_collision_recipe(key,recipe.clone())?;
        self.collision.generated_bases.insert(cache_key,recipe.generated.unwrap());
        Ok(())
    }

    pub fn automatic_collision_recipe(&self,key:(bool,usize))->Result<Recipe,String> {
        let source=self.collision.source(self,key).cloned().ok_or("Native provenance unavailable for automatic collision")?;
        let rec=legaia_scene_formats::field_objects::ObjectRecord::parse(&source.descriptor,0).ok_or("Native collider descriptor unavailable")?;
        let offset=legaia_scene_formats::field_objects::collision_footprint_offset(&rec);
        // Descriptor offsets identify a native collider center, not its live
        // class, extents, enabled state or walls. Do not invent a native shape.
        let generated=self.mesh_collision_geometry(key,&format!("Descriptor flags {:#x}, verified collider offset {offset:?}; independent complete native shape/runtime class unavailable. Mesh-derived review proposal; not retail wall ownership or verified walkability",rec.flags))?;
        Ok(Recipe{version:1,source,evidence:Evidence::MeshDerived{
            note:"Automatically generated from this object's mesh; requires review, native ownership unresolved".into()},
            walls:vec![],nonblocking:vec![],generated:Some(generated)})
    }

    fn mesh_collision_geometry(&self,key:(bool,usize),inspection:&str)->Result<GeneratedCollision,String> {
        let draw=self.at(key.0,key.1).ok_or("Collision target absent")?;
        if draw.terrain || self.npc(key).is_some() {return Err("Automatic World Object collision does not author terrain or NPC behavior".into());}
        let model=self.model.as_ref().ok_or("Collision mesh unavailable")?;
        let mut triangles=Vec::new();
        for ids in model.indices.chunks_exact(3) {
            if ids.iter().all(|&i|(i as usize)>=draw.vertex_start && (i as usize)<draw.vertex_end) {
                let tri=[model.rest[ids[0] as usize],model.rest[ids[1] as usize],model.rest[ids[2] as usize]];
                if face_measure(tri).is_some() {triangles.push(tri);}
            }
        }
        if triangles.is_empty() {return Err("Cannot generate collision: no nondegenerate instance triangles".into());}
        let generated=GeneratedCollision{version:1,mesh_sha256:digest(&serde_json::to_vec(&triangles).map_err(|e|e.to_string())?),
            triangles,pivot:draw.original_center,band_height:128.0,min_face_area:16.0,native_inspection:inspection.into()};
        generated.validate()?;Ok(generated)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn definition(triangles:Vec<[[f32;3];3]>)->GeneratedCollision {
        GeneratedCollision{version:1,mesh_sha256:digest(&serde_json::to_vec(&triangles).unwrap()),triangles,pivot:[0.0;3],
            band_height:128.0,min_face_area:16.0,native_inspection:"Synthetic geometry, no native ownership".into()}
    }
    #[test]
    fn geometry_keeps_openings_and_filters_floor_and_overhead_faces() {
        let wall=|x:f32|[[x,0.0,192.0],[x,120.0,192.0],[x,0.0,320.0]];
        let floor=[[0.0,0.0,0.0],[512.0,0.0,0.0],[0.0,0.0,512.0]];
        let arch=[[128.0,180.0,192.0],[128.0,240.0,192.0],[384.0,180.0,192.0]];
        let g=definition(vec![wall(128.0),wall(384.0),floor,arch]);
        let cells=g.wall_cells([0.0;3],[0.0;3],[1.0;3]).unwrap();
        assert!(cells.contains(&[2,5]) && cells.contains(&[6,5]));
        assert!(!cells.contains(&[4,5]),"door opening must not become an AABB wall");
        assert!(!cells.contains(&[0,2]),"floor must not become a wall");
        assert_eq!(g.surface_counts(),(1,3,0));
        let preview=g.preview_faces([0.0;3],[0.0;3],[1.0;3]).unwrap();
        assert!(preview.iter().any(|(_,steep)|!*steep));
        for (triangle,steep) in preview {if steep {assert!(triangle.iter().all(|p|p[1]<=128.0));}}
    }
    #[test]
    fn transformed_faces_use_the_same_renderer_transform_without_drift() {
        let g=definition(vec![[[256.0,0.0,256.0],[256.0,128.0,256.0],[256.0,0.0,384.0]]]);
        let rotation=[0.0,std::f32::consts::FRAC_PI_2,0.0];let scale=[2.0,1.5,0.5];let offset=[512.0,10.0,1024.0];
        let transformed=g.transformed(offset,rotation,scale).unwrap();
        for i in 0..3 {assert_eq!(transformed[0][i],super::super::world_view::editor_rotate_vertex(g.triangles[0][i],g.pivot,offset,rotation,scale));}
        let original=g.wall_cells([0.0;3],[0.0;3],[1.0;3]).unwrap();
        let moved=g.wall_cells([64.0,0.0,128.0],[0.0;3],[1.0;3]).unwrap();
        assert_eq!(moved,original.iter().map(|c|[c[0]+1,c[1]+2]).collect());
    }
    #[test]
    fn malformed_geometry_and_budget_are_rejected() {
        let mut g=definition(vec![[[0.0,0.0,0.0],[0.0,64.0,0.0],[0.0,0.0,64.0]]]);
        g.triangles[0][0][0]=f32::NAN;assert!(g.validate().is_err());
        let g=definition(vec![[[0.0,0.0,0.0],[0.0,64.0,0.0],[0.0,0.0,64.0]]]);
        assert!(g.wall_cells([-128.0,0.0,0.0],[0.0;3],[1.0;3]).is_err());
    }
}
