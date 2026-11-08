//! Persistent LCK editor groups and X/Z reflections.
//! No group membership is written to the retail MAP/MAN stream.
use std::collections::{BTreeSet,HashSet};
use serde::{Serialize,Deserialize};
use super::{world_view::WorldModel,ground_edit_ops,collision_edit,collision_wire::CollisionGrid};

#[derive(Clone,Debug,PartialEq,Eq,PartialOrd,Ord,Hash,Serialize,Deserialize)]
pub(super) enum Member {
    NativeObject(bool,usize),
    CreatedObject(u64),
    NativeFloor([u8;2]),
    CopiedFloor(u64),
    Wall([i32;2]),
}
#[derive(Clone,Debug,PartialEq,Eq,Serialize,Deserialize)]
pub(super) struct Group {pub id:u64,pub members:Vec<Member>}

pub(super) fn draw_member(world:&WorldModel,key:(bool,usize))->Option<Member>{
    let d=world.at(key.0,key.1)?;
    Some(match d.editor_uid{Some(uid)=>Member::CreatedObject(uid),None=>Member::NativeObject(key.0,key.1)})
}
pub(super) fn member_draw(world:&WorldModel,m:&Member)->Option<(bool,usize)>{
    match m {
        Member::NativeObject(t,i)=>world.at(*t,*i).filter(|d|d.editor_uid.is_none()).map(|_|(*t,*i)),
        Member::CreatedObject(uid)=>world.draws.iter().find(|d|d.editor_uid==Some(*uid)).map(|d|(d.terrain,d.index)),
        _=>None,
    }
}
pub(super) fn selected(world:&WorldModel,primary:Option<(bool,usize)>,others:&BTreeSet<(bool,usize)>,
    floors:&BTreeSet<[u8;2]>,floor_copies:&BTreeSet<u64>,walls:&BTreeSet<(i32,i32)>)->BTreeSet<Member>{
    let mut out=super::multi_selection::keys(primary,others).into_iter().filter_map(|k|draw_member(world,k)).collect::<BTreeSet<_>>();
    out.extend(floors.iter().copied().map(Member::NativeFloor));
    out.extend(floor_copies.iter().copied().map(Member::CopiedFloor));
    out.extend(walls.iter().map(|&(x,z)|Member::Wall([x,z])));
    out
}
pub(super) fn group_for<'a>(groups:&'a[Group],member:&Member)->Option<&'a Group>{
    groups.iter().find(|g|g.members.contains(member))
}
pub(super) fn create(groups:&mut Vec<Group>,requested:BTreeSet<Member>)->Result<u64,String>{
    let mut all=requested;
    // Grouping an existing group merges its entire membership transitively.
    loop {
        let before=all.len();
        for g in groups.iter(){if g.members.iter().any(|m|all.contains(m)){all.extend(g.members.iter().cloned());}}
        if before==all.len(){break;}
    }
    if all.len()<2{return Err("Select at least two pieces to create a group".into());}
    if all.len()>4096{return Err("Group exceeds 4096 components".into());}
    let next=groups.iter().map(|g|g.id).max().unwrap_or(0).checked_add(1).ok_or("Group ID exhausted")?;
    groups.retain(|g|!g.members.iter().any(|m|all.contains(m)));
    groups.push(Group{id:next,members:all.into_iter().collect()});
    Ok(next)
}
pub(super) fn ungroup(groups:&mut Vec<Group>,member:&Member)->bool{
    let n=groups.len();groups.retain(|g|!g.members.contains(member));n!=groups.len()
}
pub(super) fn validate(groups:&[Group])->Result<(),String>{
    let mut ids=HashSet::new();let mut seen=BTreeSet::new();
    for g in groups {
        if g.id==0||!ids.insert(g.id)||g.members.len()<2||g.members.len()>4096{return Err("Invalid group record".into());}
        for m in &g.members{if !seen.insert(m.clone()){return Err("Object belongs to multiple groups".into());}}
    }
    Ok(())
}
pub(super) fn mirror_axis(axis:usize)->Result<usize,String>{
    match axis{0=>Ok(0),2=>Ok(2),_=>Err("Only X or Z mirroring is supported".into())}
}
fn member_center(world:&WorldModel,ground:&ground_edit_ops::State,m:&Member)->Option<[f32;3]>{
    if let Some(k)=member_draw(world,m){return world.at(k.0,k.1).map(|d|d.center);}
    match m {
        Member::NativeFloor(cell)=>{
            let t=ground.transform(ground_edit_ops::Key::Native(*cell))?;
            Some([cell[0] as f32*128.+64.+t.offset[0],t.offset[1],cell[1] as f32*128.+64.+t.offset[2]])
        },
        Member::CopiedFloor(uid)=>{
            let p=ground.copies.iter().find(|p|p.uid==*uid)?;
            Some([p.source[0] as f32*128.+64.+p.transform.offset[0],p.transform.offset[1],p.source[1] as f32*128.+64.+p.transform.offset[2]])
        },
        Member::Wall([x,z])=>Some([*x as f32*64.+32.,0.,(*z as f32-2.)*64.+32.]),
        _=>None,
    }
}
/// One transaction: authoring geometry and collision are cloned before any edits.
/// A failed participant never leaves partially reflected geometry.
pub(super) fn mirror(
    world:&mut WorldModel,ground:&mut ground_edit_ops::State,collision:&mut collision_edit::State,
    grid:Option<&CollisionGrid>,members:&BTreeSet<Member>,axis:usize,together:bool,
)->Result<(),String>{
    let axis=mirror_axis(axis)?;
    if members.is_empty(){return Err("Nothing selected to mirror".into());}
    let centers=members.iter().map(|m|member_center(world,ground,m).ok_or_else(||format!("Missing group member: {m:?}"))).collect::<Result<Vec<_>,_>>()?;
    let middle=if together {
        let min=centers.iter().map(|p|p[axis]).fold(f32::INFINITY,f32::min);
        let max=centers.iter().map(|p|p[axis]).fold(f32::NEG_INFINITY,f32::max);
        (min+max)*0.5
    } else {0.};
    let mut w=world.clone();let mut f=ground.clone();let mut c=collision.clone();
    let mut wall_pairs=Vec::new();
    for (m,center) in members.iter().zip(&centers) {
        let displacement=if together {2.*(middle-center[axis])}else{0.};
        if let Some((terrain,index))=member_draw(&w,m) {
            let d=w.at(terrain,index).ok_or("Object unavailable")?;
            let mut offset=d.offset;let mut scale=d.scale;let mut rotation=d.rotation;
            offset[axis]+=displacement;scale[axis]=-scale[axis];
            // World-space reflection: F * R = (F * R * F) * F.
            // For the editor's Rz * Ry * Rx Euler order, conjugation
            // changes these angular signs, preserving true X/Z mirroring
            // even when the object was already rotated.
            if axis==0 {rotation[1]=-rotation[1];rotation[2]=-rotation[2];}
            else {rotation[0]=-rotation[0];rotation[1]=-rotation[1];}
            if !w.set_rotation(terrain,index,rotation)
                || !w.set_scale(terrain,index,scale)
                || !w.set_offset(terrain,index,offset){
                return Err(format!("Object mirror cannot be represented: {m:?}"));
            }
        }else if let Member::NativeFloor(cell)=m {
            let key=ground_edit_ops::Key::Native(*cell);let mut t=f.transform(key).ok_or("Floor unavailable")?;
            // Turns rotate the local axes; choose the local axis that maps to world X/Z.
            let local=if t.turns.rem_euclid(2)!=0 {2-axis}else{axis};
            t.scale[local]=-t.scale[local];t.offset[axis]+=displacement;f.set_transform(key,t);
        }else if let Member::CopiedFloor(uid)=m {
            let key=ground_edit_ops::Key::Copy(*uid);let mut t=f.transform(key).ok_or("Floor copy unavailable")?;
            let local=if t.turns.rem_euclid(2)!=0 {2-axis}else{axis};
            t.scale[local]=-t.scale[local];t.offset[axis]+=displacement;f.set_transform(key,t);
        }else if let Member::Wall([x,z])=m {
            let mut cell=(*x,*z);
            let adjustment = if axis==2 {128.0} else {0.0};
            let target = (center[axis]+displacement-32.0+adjustment).div_euclid(64.0).round() as i32;
            if axis==0 {
                cell.0 = target;
            } else {
                cell.1 = target;
            }
            wall_pairs.push(((*x,*z),cell));
        }
    }
    if !wall_pairs.is_empty() {
        let grid=grid.ok_or("Native collision grid unavailable")?;
        c.mirror_cells(grid,&wall_pairs)?;
        for g in &mut w.groups {for m in &mut g.members {if let Member::Wall(cell)=m {if let Some((_,to))=wall_pairs.iter().find(|(from,_)|*from==(cell[0],cell[1])){*cell=[to.0,to.1];}}}}
    }
    *world=w;*ground=f;*collision=c;
    Ok(())
}

#[cfg(test)]mod tests {
 use super::*;
 #[test]fn group_merges_and_ungroups(){
  let a=Member::NativeObject(false,1);let b=Member::NativeFloor([2,3]);let c=Member::Wall([5,6]);
  let mut groups=Vec::new();let id=create(&mut groups,BTreeSet::from([a.clone(),b.clone()])).unwrap();
  assert_eq!(id,1);create(&mut groups,BTreeSet::from([a.clone(),c.clone()])).unwrap();
  assert_eq!(groups.len(),1);assert_eq!(groups[0].members.len(),3);
  assert!(ungroup(&mut groups,&b));assert!(groups.is_empty());
 }
 #[test]fn invalid_duplicate_membership_rejected(){
  let m=Member::NativeObject(false,1);
  let groups=vec![Group{id:1,members:vec![m.clone(),Member::NativeObject(false,2)]},Group{id:2,members:vec![m,Member::NativeFloor([1,2])]}];
  assert!(validate(&groups).is_err());
 }
}

#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub(super) enum Mover {Object,Floor,Wall}

/// Advance the other categories of a bound assembly by the source gesture's
/// incremental world delta. The category being directly dragged has already
/// been moved by its existing controller; do not apply its displacement twice.
pub(super) fn translate_companions(
    world:&mut WorldModel,ground:&mut ground_edit_ops::State,collision:&mut collision_edit::State,
    grid:Option<&CollisionGrid>,group_id:u64,mover:Mover,delta:[f32;3],
    wall_remainder:&mut [f32;2]
)->Result<(),String>{
    if !delta.iter().all(|n|n.is_finite()){return Err("Invalid group displacement".into());}
    if delta.iter().all(|n|n.abs()<0.0001){return Ok(());}
    let members=world.groups.iter().find(|g|g.id==group_id).ok_or("Group no longer exists")?.members.clone();
    let mut staged_world=world.clone();let mut staged_ground=ground.clone();let mut staged_collision=collision.clone();
    let mut next_remainder=*wall_remainder;
    let mut wall_step=[0_i32;2];
    if mover!=Mover::Wall{
        for (j,axis) in [0usize,2].iter().enumerate(){
            next_remainder[j]+=delta[*axis];
            wall_step[j]=(next_remainder[j]/64.).round() as i32;
            next_remainder[j]-=wall_step[j] as f32*64.;
        }
    }
    let mut wall_pairs=Vec::new();
    for m in &members{
        if let Some(k)=member_draw(&staged_world,m){
            if mover!=Mover::Object{
                let d=staged_world.at(k.0,k.1).ok_or("Group object unavailable")?;
                let next=[0,1,2].map(|a|d.offset[a]+delta[a]);
                if !staged_world.set_offset(k.0,k.1,next){return Err("Group object movement failed".into());}
            }
        }else if let Member::NativeFloor(cell)=m{
            if mover!=Mover::Floor{
                let key=ground_edit_ops::Key::Native(*cell);
                let mut t=staged_ground.transform(key).ok_or("Group floor unavailable")?;
                for a in 0..3 {t.offset[a]+=delta[a];}
                staged_ground.set_transform(key,t);
            }
        }else if let Member::CopiedFloor(uid)=m{
            if mover!=Mover::Floor{
                let key=ground_edit_ops::Key::Copy(*uid);
                let mut t=staged_ground.transform(key).ok_or("Group floor copy unavailable")?;
                for a in 0..3 {t.offset[a]+=delta[a];}
                staged_ground.set_transform(key,t);
            }
        }else if let Member::Wall([x,z])=m{
            if mover!=Mover::Wall && (wall_step[0]!=0||wall_step[1]!=0){
                wall_pairs.push(((*x,*z),(*x+wall_step[0],*z+wall_step[1])));
            }
        }
    }
    if !wall_pairs.is_empty(){
        let grid=grid.ok_or("Native collision grid is not loaded for the group")?;
        staged_collision.mirror_cells(grid,&wall_pairs)?;
        remap_walls(&mut staged_world.groups,&wall_pairs);
    }
    *world=staged_world;*ground=staged_ground;*collision=staged_collision;*wall_remainder=next_remainder;
    Ok(())
}
pub(super) fn remap_walls(groups:&mut[Group],pairs:&[((i32,i32),(i32,i32))]){
    for g in groups {for m in &mut g.members {if let Member::Wall(cell)=m {
        if let Some((_,target))=pairs.iter().find(|(original,_)|*original==(cell[0],cell[1])){
            *cell=[target.0,target.1];
        }
    }}}
}

/// A stable, immutable snapshot of the selected object transforms at RMB-down.
/// The pivot is the midpoint of the selected objects' world-space AABB.
#[derive(Clone,Default)]
pub(super) struct ObjectOrbit {
    pieces: Vec<OrbitPiece>,
    pivot: [f32;3],
}
#[derive(Clone)]
struct OrbitPiece {
    key: (bool,usize),
    center: [f32;3],
    offset: [f32;3],
    rotation: [f32;3],
}
impl ObjectOrbit {
    pub(super) fn capture(world:&WorldModel,keys:&BTreeSet<(bool,usize)>)->Self {
        let pieces:Vec<_>=keys.iter().filter_map(|&(terrain,index)|{
            let draw=world.at(terrain,index)?;
            if draw.hidden||draw.vertex_start>=draw.vertex_end{return None;}
            Some(OrbitPiece{key:(terrain,index),center:draw.center,
                offset:draw.offset,rotation:draw.rotation})
        }).collect();
        let mut pivot=[0.0;3];
        if !pieces.is_empty(){
            for axis in 0..3 {
                let lo=pieces.iter().map(|p|p.center[axis]).fold(f32::INFINITY,f32::min);
                let hi=pieces.iter().map(|p|p.center[axis]).fold(f32::NEG_INFINITY,f32::max);
                pivot[axis]=lo+(hi-lo)*0.5;
            }
        }
        Self{pieces,pivot}
    }
    pub(super) fn apply(&self,world:&mut WorldModel,axis:usize,delta:f32)->Result<(),String>{
        if axis>1||!delta.is_finite(){return Err("Invalid object orbit".into());}
        if self.pieces.is_empty(){return Err("No objects selected".into());}
        let model=world.model.as_ref().ok_or("No world geometry")?;
        let (s,c)=delta.sin_cos();
        let mut edits=Vec::with_capacity(self.pieces.len());
        for piece in &self.pieces {
            let draw=world.at(piece.key.0,piece.key.1).ok_or("Object removed during rotation")?;
            if draw.hidden || draw.vertex_start>=draw.vertex_end ||
                draw.vertex_end>model.positions.len() || draw.vertex_end>model.rest.len(){
                return Err("Group mesh changed during rotation".into());
            }
            let p=[0,1,2].map(|a|piece.center[a]-self.pivot[a]);
            // Matches world_view::editor_rotate_vertex: X then Y then Z.
            let q=if axis==1{[p[0]*c+p[2]*s,p[1],-p[0]*s+p[2]*c]}
                  else{[p[0],p[1]*c-p[2]*s,p[1]*s+p[2]*c]};
            let offset=[0,1,2].map(|a|piece.offset[a]+q[a]-p[a]);
            let mut rotation=piece.rotation;
            rotation[axis]=(rotation[axis]+delta+std::f32::consts::PI)
                .rem_euclid(std::f32::consts::TAU)-std::f32::consts::PI;
            if !offset.iter().all(|v|v.is_finite() && v.abs()<1_000_000.0)
                || !rotation.iter().all(|v|v.is_finite() && v.abs()<1000.0){
                return Err("Group rotation would exceed editor limits".into());
            }
            edits.push((piece.key,offset,rotation));
        }
        // Every participant has been validated. No frame-to-frame drift:
        // transforms are reconstructed from the RMB-down snapshot.
        for (key,offset,rotation) in edits {
            assert!(world.set_offset(key.0,key.1,offset));
            assert!(world.set_rotation(key.0,key.1,rotation));
        }
        Ok(())
    }
}

/// Copies of group members inherit a NEW group identity when at least two
/// original members were actually copied. Missing floor/wall resources are not
/// invented; copied subsets become a valid independent group of their own.
pub(super) fn duplicate_complete_groups(
    groups:&mut Vec<Group>,links:&[(Member,Member)]
)->Result<usize,String>{
    let mapping=links.iter().cloned().collect::<std::collections::BTreeMap<_,_>>();
    let original=groups.clone();
    let mut next=original.iter().map(|g|g.id).max().unwrap_or(0);
    let mut candidates=Vec::new();
    for group in &original {
        let members=group.members.iter().filter_map(|member|
            mapping.get(member).cloned()
        ).collect::<Vec<_>>();
        if members.len()<2 {continue;}
        next=next.checked_add(1).ok_or("Group IDs exhausted")?;
        candidates.push(Group{id:next,members});
    }
    let count=candidates.len();
    let mut checked=original;
    checked.extend(candidates);
    validate(&checked)?;
    *groups=checked;
    Ok(count)
}

#[cfg(test)]
mod lck_group_orbit_tests {
    use super::*;
    #[test]
    fn duplicated_groups_get_new_ids() {
        let mut groups=vec![Group{id:1,members:vec![Member::CreatedObject(1),Member::CreatedObject(2)]}];
        assert_eq!(duplicate_complete_groups(&mut groups,&[
            (Member::CreatedObject(1),Member::CreatedObject(11)),
            (Member::CreatedObject(2),Member::CreatedObject(12)),
        ]).unwrap(),1);
        assert_eq!(groups.len(),2);
        assert_ne!(groups[0].id,groups[1].id);
        assert_eq!(groups[1].members[0],Member::CreatedObject(11));
    }
    #[test]
    fn mixed_group_copies_preserve_copied_subset() {
        let mut groups=vec![Group{id:1,members:vec![
            Member::CreatedObject(1),Member::CreatedObject(2),
            Member::NativeFloor([2,3])
        ]}];
        let copied=duplicate_complete_groups(&mut groups,&[
            (Member::CreatedObject(1),Member::CreatedObject(11)),
            (Member::CreatedObject(2),Member::CreatedObject(12)),
        ]).unwrap();
        assert_eq!(copied,1);
        assert_eq!(groups.len(),2);
        assert_eq!(groups[1].members.len(),2);
        assert_eq!(groups[1].members[0],Member::CreatedObject(11));
        assert_ne!(groups[0].id,groups[1].id);
    }
    #[test]
    fn duplicated_floor_group_has_new_identity() {
        let mut groups=vec![Group{id:7,members:vec![
            Member::NativeFloor([2,3]),Member::NativeFloor([3,3]),
        ]}];
        let copied=duplicate_complete_groups(&mut groups,&[
            (Member::NativeFloor([2,3]),Member::CopiedFloor(20)),
            (Member::NativeFloor([3,3]),Member::CopiedFloor(21)),
        ]).unwrap();
        assert_eq!(copied,1);
        assert_eq!(groups[1].id,8);
        assert_eq!(groups[1].members,vec![
            Member::CopiedFloor(20),Member::CopiedFloor(21)
        ]);
    }
    #[test]
    fn partially_duplicated_groups_are_unchanged() {
        let mut groups=vec![Group{id:1,members:vec![Member::CreatedObject(1),Member::NativeFloor([2,3])]}];
        assert_eq!(duplicate_complete_groups(&mut groups,&[(Member::CreatedObject(1),Member::CreatedObject(11))]).unwrap(),0);
        assert_eq!(groups.len(),1);
    }
}

#[cfg(test)]
mod lck_preserve_donor_groups_tests {
    use super::*;
    #[test]
    fn floor_copies_never_consume_donor_group() {
        let original=Group{id:9,members:vec![
            Member::NativeFloor([3,3]),Member::NativeFloor([4,3]),
        ]};
        let mut groups=vec![original.clone()];
        let count=duplicate_complete_groups(&mut groups,&[
            (Member::NativeFloor([3,3]),Member::CopiedFloor(71)),
            (Member::NativeFloor([4,3]),Member::CopiedFloor(72)),
        ]).unwrap();
        assert_eq!(count,1);
        assert_eq!(groups[0],original);
        assert_eq!(groups[1].id,10);
        assert_eq!(groups[1].members,vec![Member::CopiedFloor(71),Member::CopiedFloor(72)]);
        assert!(group_for(&groups,&Member::NativeFloor([3,3])).is_some());
        assert!(group_for(&groups,&Member::CopiedFloor(71)).is_some());
    }
    #[test]
    fn wall_copies_keep_original_wall_group() {
        let original=Group{id:4,members:vec![
            Member::Wall([10,10]),Member::Wall([11,10]),
        ]};
        let mut groups=vec![original.clone()];
        duplicate_complete_groups(&mut groups,&[
            (Member::Wall([10,10]),Member::Wall([20,20])),
            (Member::Wall([11,10]),Member::Wall([21,20])),
        ]).unwrap();
        assert_eq!(groups[0],original);
        assert_eq!(groups[1].members,vec![Member::Wall([20,20]),Member::Wall([21,20])]);
        assert!(validate(&groups).is_ok());
    }
}
