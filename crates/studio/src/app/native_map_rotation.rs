//! Exact quarter-yaw adapter for static MAP references. No wall ownership
//! follows from a descriptor, a mesh bound, or a successful encoding.
use super::*;

#[derive(Clone, Debug)]
pub(super) struct YawEdit {
    pub source: MoveEdit,
    pub center: [f32; 2],
    pub quarters: u8,
}

pub(super) fn quarters(rotation: [f32; 3]) -> Result<u8> {
    ensure!(rotation.iter().all(|v|v.is_finite()) && rotation[0].abs()<0.00001 && rotation[2].abs()<0.00001,
        "Native MAP rotation currently requires pure quarter-turn yaw");
    let q=rotation[1]/std::f32::consts::FRAC_PI_2;
    ensure!((q-q.round()).abs()<0.00001,"Native MAP yaw must be an exact quarter turn");
    Ok((q.round() as i32).rem_euclid(4) as u8)
}

fn turn(mut p:(i32,i32), q:u8)->(i32,i32) {
    for _ in 0..q {p=(p.1,-p.0);} p
}

/// Returns descriptor/grid writes only after independent source and collider
/// readback. All transforms are computed from the untouched source, so a group
/// move plus yaw is applied once, without cumulative rounding.
pub(super) fn apply(map:&mut [u8], original:&[u8], edits:&[YawEdit])->Result<Vec<MovedRecord>> {
    let mut touched=HashSet::new();let mut changed=Vec::new();
    for edit in edits {
        let source=&edit.source;
        ensure!(edit.quarters<4 && touched.insert(source.cell),"Invalid/repeated native yaw reference");
        let (id,r)=record_at(original,source.cell,source.source_position,source.env_slot,source.source_rotation)?;
        ensure!(r.rot_x&4095==0 && r.rot_z&4095==0,"Tilted native MAP rotations require full matrix composition");
        ensure!(!anchor_has_bind(original,source.cell,&r),"Script-bound MAP yaw requires native dependency closure");
        require_no_unowned_walls(original,source.collision_bounds)?;
        let p=source.source_position;
        ensure!(edit.center.iter().all(|v|v.is_finite()),"Invalid editor rotation center");
        let mut rotated=(p.0 as f32-edit.center[0],p.1 as f32-edit.center[1]);
        for _ in 0..edit.quarters {rotated=(rotated.1,-rotated.0);}
        let next_p=(quantize(edit.center[0]+rotated.0+source.delta[0],"yaw world X")?,
            quantize(edit.center[1]+rotated.1+source.delta[2],"yaw world Z")?);
        let dx=next_p.0-p.0;let dz=next_p.1-p.1;
        let mut next=[0u8;32];next.copy_from_slice(&original[id*32..id*32+32]);
        let x=i16::try_from(r.x_off as i32+dx).context("Yaw pivot compensation exceeds native X")?;
        let z=i16::try_from(r.z_off as i32-dz).context("Yaw pivot compensation exceeds native Z")?;
        let y=i16::try_from(r.y_off as i32-quantize(source.delta[1],"Y")?).context("Yaw exceeds native Y")?;
        ensure!(anchor_score(x as i32,z as i32).is_some_and(|s|s<LCK_EXTENDED_ANCHOR_PENALTY),"Yaw exceeds native spawn-anchor reach");
        next[..2].copy_from_slice(&x.to_le_bytes());next[2..4].copy_from_slice(&y.to_le_bytes());next[4..6].copy_from_slice(&z.to_le_bytes());
        let angle=(r.rot_y&0xf000)|((r.rot_y.wrapping_add(edit.quarters as u16*1024))&4095);
        next[10..12].copy_from_slice(&angle.to_le_bytes());
        let off=turn(field_objects::collision_footprint_offset(&r),edit.quarters);
        let corrected=r.flags&field_objects::FLAG_COLLIDER_CORRECTION!=0;
        let sx=off.0-r.col_delta as i32*128+if corrected {x as i32}else{0};
        let sz=off.1-r.row_delta as i32*128-if corrected {z as i32}else{0};
        ensure!(sx%16==0 && sz%16==0,"Rotated native collider cannot be represented exactly in 16-unit sub-anchors");
        next[14]=i8::try_from(sx/16).context("Rotated collider X exceeds signed sub-anchor")? as u8;
        next[15]=i8::try_from(sz/16).context("Rotated collider Z exceeds signed sub-anchor")? as u8;
        let read=ObjectRecord::parse(&next,0).context("Yaw readback failed")?;
        ensure!(read.rot_y==angle && source_collision(&read,source.cell.0,source.cell.1)==(next_p.0+off.0,next_p.1+off.1),"Native yaw/collider readback mismatch");
        let refs=map[GRID_OFF..0x10000].chunks_exact(2).filter(|w|u16::from_le_bytes([w[0],w[1]])&511==id as u16).count();
        let detached=refs>1;let target=if detached {blank_object_record(map)?}else{id};
        map[target*32..target*32+32].copy_from_slice(&next);
        if detached {
            let go=GRID_OFF+(source.cell.1 as usize*128+source.cell.0 as usize)*2;
            let word=u16::from_le_bytes(map[go..go+2].try_into()?);
            map[go..go+2].copy_from_slice(&((word&!511)|target as u16).to_le_bytes());
        }
        changed.push(MovedRecord{id:target,detached_cell:detached.then_some(source.cell)});
    }
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(corrected:bool)->(Vec<u8>,YawEdit) {
        let mut map=vec![0;MAP_LEN];let cell=(30,30);let id=100;
        let o=id*32;map[o..o+2].copy_from_slice(&32i16.to_le_bytes());
        map[o+4..o+6].copy_from_slice(&(-16i16).to_le_bytes());
        map[o+14]=2;map[o+15]=255;
        map[o+16..o+18].copy_from_slice(&50u16.to_le_bytes());
        let flags=field_objects::FLAG_PLACED|if corrected{field_objects::FLAG_COLLIDER_CORRECTION}else{0};
        map[o+18..o+20].copy_from_slice(&flags.to_le_bytes());
        let go=GRID_OFF+(cell.1 as usize*128+cell.0 as usize)*2;
        map[go..go+2].copy_from_slice(&(id as u16).to_le_bytes());
        let p=(field_objects::world_x(cell.0,32),field_objects::world_z(cell.1,-16));
        (map,YawEdit{source:MoveEdit{cell,source_position:p,env_slot:50,source_rotation:[0;3],delta:[64.,0.,-32.],collision_bounds:None},
            center:[(p.0+16) as f32,(p.1+32) as f32],quarters:1})
    }
    #[test]
    fn native_map_yaw_matches_editor_pivot_and_collider_for_all_quarters() {
        for corrected in [false,true] {for q in 0..4 {
            let (map,mut edit)=fixture(corrected);edit.quarters=q;
            let batch=ExportEdits{yaws:vec![edit.clone()],..Default::default()};
            let mut output=map.clone();for (o,b) in map_patches(&map,&batch).unwrap(){output[o..o+b.len()].copy_from_slice(&b);}
            let r=ObjectRecord::parse(&output,100).unwrap();let old=ObjectRecord::parse(&map,100).unwrap();
            assert_eq!(r.rot_y,q as u16*1024);
            let old_c=source_collision(&old,30,30);
            let expected=world_view::editor_rotate_vertex([old_c.0 as f32,0.,old_c.1 as f32],
                [edit.center[0],0.,edit.center[1]],edit.source.delta,[0.,q as f32*std::f32::consts::FRAC_PI_2,0.],[1.;3]);
            let actual=source_collision(&r,30,30);
            assert!((actual.0 as f32-expected[0]).abs()<0.002 && (actual.1 as f32-expected[2]).abs()<0.002);
            assert_eq!(&output[WALL_OFF..],&map[WALL_OFF..]);
            for i in 0..WALL_OFF {if !(3200..3206).contains(&i) && !(3210..3212).contains(&i) && !(3214..3216).contains(&i){assert_eq!(output[i],map[i]);}}
        }}
    }
    #[test]
    fn native_map_yaw_detaches_only_selected_shared_reference() {
        let (mut map,edit)=fixture(false);let unrelated=GRID_OFF+(31*128+30)*2;
        map[unrelated..unrelated+2].copy_from_slice(&100u16.to_le_bytes());
        let mut output=map.clone();let changes=apply(&mut output,&map,&[edit]).unwrap();
        assert!(changes[0].detached_cell.is_some());assert_ne!(changes[0].id,100);
        assert_eq!(&output[3200..3232],&map[3200..3232]);
        assert_eq!(&output[unrelated..unrelated+2],&map[unrelated..unrelated+2]);
    }
    #[test]
    fn native_map_yaw_group_allocates_independent_identities_without_moving_third_reference() {
        let (mut map,first)=fixture(false);
        for col in [31,32] {let go=GRID_OFF+(30*128+col)*2;map[go..go+2].copy_from_slice(&100u16.to_le_bytes());}
        let mut second=first.clone();second.source.cell.0=31;
        second.source.source_position.0+=128;second.center[0]+=128.;second.quarters=3;
        let edits=ExportEdits{yaws:vec![first,second],..Default::default()};
        let mut output=map.clone();for (o,b) in map_patches(&map,&edits).unwrap(){output[o..o+b.len()].copy_from_slice(&b);}
        let mut ids=HashSet::new();
        for (col,angle) in [(30,1024),(31,3072),(32,0)] {
            let go=GRID_OFF+(30*128+col)*2;let id=(u16::from_le_bytes(output[go..go+2].try_into().unwrap())&511) as usize;
            assert!(ids.insert(id));assert_eq!(ObjectRecord::parse(&output,id).unwrap().rot_y,angle);
        }
        assert_eq!(&output[3200..3232],&map[3200..3232]);
    }
    #[test]
    fn native_map_yaw_refuses_unowned_walls_tilt_bindings_and_fractional_collision() {
        let (map,edit)=fixture(true);
        let mut walls=map.clone();walls[WALL_OFF]=0x10;
        assert!(apply(&mut walls.clone(),&walls,&[edit.clone()]).is_err());
        let mut tilt=map.clone();tilt[3208]=1;
        let mut tilted=edit.clone();tilted.source.source_rotation[0]=1;
        assert!(apply(&mut tilt.clone(),&tilt,&[tilted]).is_err());
        let mut bound=map.clone();let go=GRID_OFF+(30*128+30)*2;
        bound[go+1]|=4;assert!(apply(&mut bound.clone(),&bound,&[edit.clone()]).is_err());
        let mut fraction=edit;fraction.source.delta[0]=1.;
        assert!(apply(&mut map.clone(),&map,&[fraction]).is_err());
        assert!(quarters([0.,0.1,0.]).is_err());assert!(quarters([0.1,0.,0.]).is_err());
        assert_eq!(quarters([0.,-std::f32::consts::FRAC_PI_2,0.]).unwrap(),3);
    }
    #[test]
    fn native_map_yaw_batch_exports_reparses_and_preserves_other_disc_records()->Result<()> {
        let (map,edit)=fixture(false);let image=super::super::tests::map_disc(&map);
        let plan=compile_plan(PathBuf::new(),DiscPatcher::open(image.clone())?,image.len(),0,
            &ExportEdits{yaws:vec![edit],..Default::default()})?;
        let parsed=DiscPatcher::open(plan.image.clone())?;
        let result=parsed.read_entry_footprint(0)?;
        assert_eq!(ObjectRecord::parse(&result,100).unwrap().rot_y,1024);
        let original=DiscPatcher::open(image)?;
        assert_eq!(parsed.read_entry_footprint(1)?,original.read_entry_footprint(1)?);
        assert_eq!(&result[WALL_OFF..],&map[WALL_OFF..]);Ok(())
    }

    #[test]
    #[ignore="explicit protected retail input and new output directory; native format/readback only"]
    fn retail_cells_native_map_yaw_editor_to_img_readback()->Result<()> {
        let path=PathBuf::from(std::env::var_os("LCK_RETAIL_SOURCE").context("LCK_RETAIL_SOURCE required")?);
        let folder=PathBuf::from(std::env::var_os("LCK_YAW_OUTPUT").context("LCK_YAW_OUTPUT required")?);
        ensure!(!folder.exists() && folder.is_absolute(),"Use a new absolute output directory");
        let source=std::fs::read(&path)?;
        let source_hash=super::super::super::collision_recipes::digest(&source);
        ensure!(source_hash=="750dbcbadb70362feb3468467bbe7efbf3d151a414caa428c66c368d4b0b73e6","Protected retail fixture differs");
        let library=crate::library::Library::open(&path)?;
        let disc=DiscPatcher::open(source.clone())?;
        std::fs::create_dir(&folder)?;
        let mut verified=0;
        // Representative fixture names only. Production eligibility uses record
        // evidence, never scene names or object indices.
        for name in ["retona","vell"] {
            let scene=legaia_engine_world_views::scene_staging::assemble_field_scene(&library.index,name).map_err(anyhow::Error::msg)?;
            let native=Scene::load(&library.index,name)?;
            let entry=native.field_map_index(&library.index).context("No native MAP")? as usize;
            let map=disc.read_entry_footprint(entry)?;
            let loaded=disc.read_disc_sectors(disc.entry_disc_lba(entry).context("No MAP address")?,0x28)?;
            let bindings=NativeBindings::parse(&loaded[0x10000..0x12000],&loaded[0x12000..])?;
            let mut world=world_view::build_world(&scene,&library.index);
            let mut accepted=None;
            for (i,p) in scene.placements.iter().enumerate() {
                if p.anim_id!=0 || p.rot_x&4095!=0 || p.rot_z&4095!=0 {continue;}
                if record_at(&map,p.cell,(p.world_x,p.world_z),p.env_slot,[p.rot_x,p.rot_y,p.rot_z]).is_err() {continue;}
                let Some(draw)=world.at(false,i) else {continue};
                let yaw=YawEdit{source:MoveEdit{cell:p.cell,source_position:(p.world_x,p.world_z),env_slot:p.env_slot,
                    source_rotation:[p.rot_x,p.rot_y,p.rot_z],delta:[0.;3],collision_bounds:original_collision_bounds(&world,i)},
                    center:[draw.original_center[0],draw.original_center[2]],quarters:1};
                match map_patches_with_bindings(&map,&ExportEdits{yaws:vec![yaw],..Default::default()},&bindings) {Ok(_)=>{accepted=Some(i);break;},Err(error)=>println!("{name} placement {i}: {error:#}")}
            }
            let Some(i)=accepted else {println!("{name}: no quarter-yaw candidate passed all dependency/precision guards");continue};
            ensure!(world.set_rotation(false,i,[0.,std::f32::consts::FRAC_PI_2,0.]),"Editor yaw rejected");
            let edits=prepare(&scene,&world)?;ensure!(edits.yaws.len()==1,"Editor yaw omitted");
            // Exercise the complete public planner, including dependency routes
            // and archive identity validation, not a hand-written MAP patch.
            let plan=plan_export(&path,&scene,&world,&library.index)?;
            let expected=DiscPatcher::open(plan.image.clone())?.read_entry_footprint(entry)?;
            let output=folder.join(format!("LCK_M2_{name}_quarter_yaw.img"));export(plan,&output)?;
            std::fs::write(output.with_extension("lckscene"),scene_instances::serialize(name,&world))?;
            let bytes=std::fs::read(&output)?;
            let read=DiscPatcher::open(bytes.clone())?;
            ensure!(read.read_entry_footprint(entry)?==expected,"Final native MAP changed");
            for other in 0..disc.entry_count() {if other!=entry {ensure!(read.read_entry(other)?==disc.read_entry(other)?,"Unrelated PROT entry {other} changed");}}
            let reopened=crate::library::Library::open(&output)?;
            let after=legaia_engine_world_views::scene_staging::assemble_field_scene(&reopened.index,name).map_err(anyhow::Error::msg)?;
            let before=&scene.placements[i];let found=after.placements.iter().find(|p|p.cell==before.cell).context("Native identity missing on reopen")?;
            ensure!(found.env_slot==before.env_slot && found.rot_y&4095==(before.rot_y+1024)&4095,"Reopened native orientation/mesh mismatch");
            let after_world=world_view::build_world(&after,&reopened.index);
            let before_draw=world.at(false,i).context("Missing edited draw")?;
            let after_i=after.placements.iter().position(|p|p.cell==before.cell).context("Missing reopened draw index")?;
            let after_draw=after_world.at(false,after_i).context("Missing reopened draw")?;
            let old_model=world.model.as_ref().context("No original render geometry")?;
            let new_model=after_world.model.as_ref().context("No reopened render geometry")?;
            let old_vertices=&old_model.rest[before_draw.vertex_start..before_draw.vertex_end];
            let new_vertices=&new_model.rest[after_draw.vertex_start..after_draw.vertex_end];
            ensure!(!old_vertices.is_empty() && old_vertices.len()==new_vertices.len(),"Reopened geometry count differs");
            for (&old,&new) in old_vertices.iter().zip(new_vertices) {
                let intended=world_view::editor_rotate_vertex(old,before_draw.original_center,before_draw.offset,before_draw.rotation,before_draw.scale);
                ensure!((0..3).all(|axis|(intended[axis]-new[axis]).abs()<=EPS),"Reopened geometry does not reproduce intended editor transform: {intended:?} vs {new:?}");
            }
            ensure!(expected[WALL_OFF..GRID_OFF]==map[WALL_OFF..GRID_OFF] && expected[0x10000..]==map[0x10000..],"Unrelated wall/floor/script bytes changed");
            let selected_grid=GRID_OFF+(before.cell.1 as usize*128+before.cell.0 as usize)*2;
            for go in (GRID_OFF..0x10000).step_by(2) {if go!=selected_grid {ensure!(expected[go..go+2]==map[go..go+2],"Unrelated native grid reference changed");}}
            println!("{name}: reference {:?}, selector {}, reopened quarter-yaw IMG {}; GUI/gameplay NOT verified",before.cell,before.env_slot,output.display());
            verified+=1;
            if verified==2 {break;}
        }
        ensure!(verified>=2,"Need native exports from at least two representative cells, got {verified}");
        ensure!(super::super::super::collision_recipes::digest(&std::fs::read(path)?)==source_hash,"Retail source changed");
        Ok(())
    }
}
