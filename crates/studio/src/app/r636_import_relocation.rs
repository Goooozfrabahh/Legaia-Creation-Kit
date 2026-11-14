// LCK_R636_IMPORT_INTEGRITY
//
// Protect cross-cell native MAP references after every staged export.
// Only independent unbound static decorations may be re-anchored.
// Never silently relocate MAN-bound actors or scripted identities.

use anyhow::{bail,ensure,Context,Result};

use legaia_scene_formats::field_objects::ObjectRecord;

const GRID:usize=0x8000;
const SIDE:usize=128;

fn at(x:usize,z:usize)->usize {
    GRID+(z*SIDE+x)*2
}

fn word(map:&[u8],x:usize,z:usize)->u16 {
    let i=at(x,z);
    u16::from_le_bytes([map[i],map[i+1]])
}

fn means(
    map:&[u8],
    x:usize,
    z:usize,
    lut:&[i16;16]
)->Result<i32>{
    ensure!(
        x<127 && z<127,
        "R636: native floor at border lacks four corners"
    );

    let p=0x4000+z*128+x;

    let sum=[0,1,128,129]
        .iter()
        .map(|&d|lut[(map[p+d]&15) as usize] as i32)
        .sum::<i32>();

    Ok(sum/4)
}

fn grid_refs(
    map:&[u8],
    id:usize
)->Vec<(usize,usize)>{
    let mut found=Vec::new();

    for z in 0..128 {
        for x in 0..128 {
            if (word(map,x,z)&511) as usize==id {
                found.push((x,z));
            }
        }
    }

    found
}

fn native_xyz(
    map:&[u8],
    id:usize,
    x:usize,
    z:usize,
    lut:&[i16;16]
)->Result<(i32,i32,i32)>{
    let rec=ObjectRecord::parse(map,id)
        .context("R636: descriptor missing")?;

    Ok((
        x as i32*128+64+rec.x_off as i32,
        means(map,x,z,lut)?+rec.y_off as i32,
        z as i32*128+64-rec.z_off as i32
    ))
}

fn has_trigger(
    map:&[u8],
    x:usize,
    z:usize
)->bool{
    let base=0x10000;

    for kind in [0usize,1,3] {
        let ptr=base+2+4*kind;

        let start=u16::from_le_bytes([
            map[ptr],map[ptr+1]
        ]) as usize;

        let count=u16::from_le_bytes([
            map[ptr+2],map[ptr+3]
        ]) as usize;

        let stride=if kind==3 {8}else{4};

        if start
            .checked_add(count.saturating_mul(stride))
            .is_none_or(|end|end>0x2000)
        {
            return true;
        }

        for i in 0..count {
            let p=base+start+i*stride;

            if map[p] as usize==x &&
               map[p+1] as usize==z
            {
                return true;
            }
        }
    }

    false
}

fn region_drawable(
    map:&[u8],
    x:usize,
    z:usize
)->bool{
    let base=0x10000;

    let start=u16::from_le_bytes([
        map[base+14],map[base+15]
    ]) as usize;

    let count=u16::from_le_bytes([
        map[base+16],map[base+17]
    ]) as usize;

    // Permit fixtures without region tables.
    if count==0 {
        return true;
    }

    if start<18 || start+count*8>0x2000 {
        return false;
    }

    (0..count).any(|i|{
        let p=base+start+i*8;
        let r=&map[p..p+8];

        let xmin=r[0].min(r[2]) as usize;
        let xmax=r[0].max(r[2]) as usize;
        let zmin=r[1].min(r[3]) as usize;
        let zmax=r[1].max(r[3]) as usize;

        r[4]<=1 &&
        xmin<=x && x<xmax &&
        zmin<=z && z<zmax
    })
}

// LCK_R636D_DUAL_STATIC_GROUND
//
// Experimental native dual-owner serializer.
// Supports only independently registered, unbound static 0x0012
// descriptors and ground-only 0x0001 floor descriptors.
//
// No MAN, actor, shared descriptor, script, or trigger mutation.
// Runtime scene initialization must retain BOTH visibility marks.
fn r636d_fuse_static_and_ground(
    objects:&[u8],
    out:&mut [u8],
    id:usize,
    x:usize,
    z:usize,
    lut:&[i16;16],
    source_world:(i32,i32,i32),
)->Result<()> {
    use legaia_scene_formats::field_objects::{
        CELL_VISIBLE,
        CELL_WALK_VISIBLE,
        CELL_BIND_OWNED,
    };

    let original=ObjectRecord::parse(objects,id)
        .context("R636D: original native descriptor missing")?;

    ensure!(
        original.flags==0x0012 && !original.is_placed(),
        "R636D: descriptor {id} requires actor/script handling"
    );

    ensure!(
        grid_refs(objects,id)==vec![(x,z)],
        "R636D: descriptor {id} is shared by multiple grid references"
    );

    let floor_word=word(out,x,z);

    ensure!(
        floor_word&CELL_WALK_VISIBLE!=0 &&
        floor_word&CELL_BIND_OWNED==0,
        "R636D: destination is not independent walk-ground"
    );

    let floor_id=(floor_word&0x01ff) as usize;

    ensure!(
        floor_id>=4 && floor_id!=id,
        "R636D: separate ground descriptor missing"
    );

    let floor=ObjectRecord::parse(out,floor_id)
        .context("R636D: ground-only descriptor missing")?;

    ensure!(
        floor.flags==1 && !floor.is_placed(),
        "R636D: destination descriptor {floor_id} is not ground-only; flags {:#x}",
        floor.flags
    );

    ensure!(
        grid_refs(out,id).is_empty(),
        "R636D: static descriptor {id} already has another reference"
    );

    let p=id*32;

    ensure!(
        objects[p+6..p+32]==out[p+6..p+32],
        "R636D: another stage changed imported static model identity"
    );

    // R60 stores native ground material information at +20..+24.
    // Keep all native model selector, transform, and cull fields.
    let ground_material:[u8;4]=
        out[floor_id*32+20..floor_id*32+24]
            .try_into()
            .context("R636D: ground material truncated")?;

    let y=i16::try_from(
        source_world.1-means(out,x,z,lut)?
    ).context(
        "R636D: floor-relative static Y exceeds signed 16-bit range"
    )?;

    // Add ground capability to the independent static record.
    out[p+2..p+4].copy_from_slice(&y.to_le_bytes());

    out[p+18..p+20].copy_from_slice(
        &(original.flags|1).to_le_bytes()
    );

    out[p+20..p+24].copy_from_slice(&ground_material);

    let g=at(x,z);

    let composite=
        (floor_word&!0x01ff) |
        (id as u16) |
        CELL_WALK_VISIBLE |
        CELL_VISIBLE;

    out[g..g+2].copy_from_slice(&composite.to_le_bytes());

    let fused=ObjectRecord::parse(out,id)
        .context("R636D: combined descriptor cannot be decoded")?;

    ensure!(
        fused.flags==0x0013 &&
        !fused.is_placed() &&
        fused.pack_index_field==original.pack_index_field &&
        fused.cull_radius==original.cull_radius &&
        [fused.rot_x,fused.rot_y,fused.rot_z] ==
        [original.rot_x,original.rot_y,original.rot_z],
        "R636D: combined descriptor changed model, rotation or culling"
    );

    ensure!(
        native_xyz(out,id,x,z,lut)?==source_world,
        "R636D: static model moved during ground combination"
    );

    // Simulate the scene initializer which recomputes native
    // walk-ground and object draw marks from descriptor flags.
    let mut initialized=out.to_vec();

    legaia_engine_field::field_regions::
        refresh_object_grid_marks(&mut initialized);

    let initialized_word=word(&initialized,x,z);

    ensure!(
        initialized_word&0x01ff==id as u16 &&
        initialized_word&CELL_WALK_VISIBLE!=0 &&
        initialized_word&CELL_VISIBLE!=0,
        "R636D: scene initialization does not recognize combined ground/static ownership"
    );

    Ok(())
}
// Run only after all normal native export stages are complete.
pub(super) fn reconcile(
    retail:&[u8],
    objects:&[u8],
    final_map:&[u8],
    lut:&[i16;16]
)->Result<(Vec<u8>,usize,usize)>{
    ensure!(
        retail.len()==objects.len() &&
        objects.len()==final_map.len() &&
        final_map.len()>=0x12000,
        "R636: inconsistent native MAP sizes"
    );

    let mut out=final_map.to_vec();

    let mut inspected=0usize;
    let mut moved=0usize;

    let mut seen=std::collections::BTreeSet::new();
    let mut fused=std::collections::BTreeSet::new();

    for z in 0..128usize {
        for x in 0..128usize {
            let bw=word(objects,x,z);

            let id=(bw&511) as usize;
            let old_id=(word(retail,x,z)&511) as usize;

            if id<4 || id==old_id {
                continue;
            }

            let p=id*32;

            if objects[p..p+32]==retail[p..p+32] {
                continue;
            }

            let rec=ObjectRecord::parse(objects,id)
                .with_context(||format!(
                    "R636: native descriptor {id} invalid"
                ))?;

            // Empty and dedicated ground-only descriptors do
            // not represent independently imported models.
            if rec.flags==0 || rec.flags==1 {
                continue;
            }

            if !seen.insert((id,x,z)) {
                continue;
            }

            inspected+=1;

            let refs=grid_refs(&out,id);

            if refs.len()==1 && refs[0]==(x,z) {
                // The original MAP reference survived.
                // A floor-height edit can still move an unbound
                // decoration vertically. Compensate its Y offset.
                if rec.flags==0x0012 && !rec.is_placed() {
                    let original_world=
                        native_xyz(objects,id,x,z,lut)?;

                    let new_y=original_world.1-
                        means(&out,x,z,lut)?;

                    let new_y=i16::try_from(new_y)
                        .context(
                            "R636: changed floor height exceeds static native Y offset"
                        )?;

                    out[p+2..p+4]
                        .copy_from_slice(&new_y.to_le_bytes());

                    ensure!(
                        native_xyz(&out,id,x,z,lut)?
                            ==original_world,
                        "R636: static reference moved vertically after floor composition"
                    );
                }

            } else if refs.len()==1 {
                // Another stage moved the reference. Confirm
                // world-space position has not been changed.
                let (ax,az)=refs[0];

                ensure!(
                    native_xyz(&out,id,ax,az,lut)?
                        ==native_xyz(objects,id,x,z,lut)?,
                    "R636: imported descriptor was re-anchored by a later stage but changed world position"
                );

            } else if refs.is_empty() {
                // One native grid word cannot store both a
                // ground descriptor and a separate object.
                // Re-anchor ONLY independently unbound static
                // decorations. Actors need MAN-safe handling.

                ensure!(
                    rec.flags==0x0012 && !rec.is_placed(),
                    "R636: imported descriptor {id} lost its MAP anchor at ({x},{z}); actor/script-bound or other native classes require dedicated relocation, NOT static anchor rewriting"
                );

                ensure!(
                    grid_refs(objects,id).len()==1,
                    "R636: imported descriptor {id} is shared; cannot independently move its anchor"
                );

                let original_world=
                    native_xyz(objects,id,x,z,lut)?;

                let mut choice=None;

                // Prefer adjacent native draw anchors.
                // The limited search preserves the original
                // renderer's near-camera selection behavior.
                for radius in 1i32..=2 {
                    for dz in -radius..=radius {
                        for dx in -radius..=radius {
                            if dx.abs().max(dz.abs())!=radius {
                                continue;
                            }

                            if dx.abs()>rec.cull_radius as i32 ||
                               dz.abs()>rec.cull_radius as i32
                            {
                                continue;
                            }

                            let ax=x as i32+dx;
                            let az=z as i32+dz;

                            if !(0..127).contains(&ax) ||
                               !(0..127).contains(&az)
                            {
                                continue;
                            }

                            let (ax,az)=(
                                ax as usize,
                                az as usize
                            );

                            // Do not displace ground, another
                            // imported object, original objects,
                            // native triggers, or region ownership.
                            let final_word=word(&out,ax,az);
                            let staged_word=word(objects,ax,az);
                            let retail_word=word(retail,ax,az);

                            // LCK_R636B_ANCHOR_OWNERSHIP
                            // Descriptor occupancy uses the actual 9-bit index.
                            // Other cell flags must be independently inspected.
                            let all_words=
                                final_word|staged_word|retail_word;

                            if (all_words&0x01ff)!=0 ||
                               (all_words&
                                legaia_scene_formats::field_objects::CELL_BIND_OWNED)!=0 ||
                               (all_words&
                                legaia_scene_formats::field_objects::CELL_WALK_VISIBLE)!=0 ||
                               (all_words&0x0800)!=0 ||
                               has_trigger(&out,ax,az) ||
                               !region_drawable(&out,ax,az)
                            {
                                continue;
                            }

                            let xx=rec.x_off as i32-dx*128;
                            let zz=rec.z_off as i32+dz*128;

                            // Native render-window safe reach.
                            if xx.abs()>384 || zz.abs()>384 {
                                continue;
                            }

                            let y=original_world.1-
                                means(&out,ax,az,lut)?;

                            let (
                                Ok(xx),
                                Ok(zz),
                                Ok(yy)
                            )=(
                                i16::try_from(xx),
                                i16::try_from(zz),
                                i16::try_from(y)
                            ) else {
                                continue;
                            };

                            choice=Some((
                                ax,az,xx,yy,zz
                            ));

                            break;
                        }

                        if choice.is_some() {
                            break;
                        }
                    }

                    if choice.is_some() {
                        break;
                    }
                }

                if let Some((ax,az,xx,yy,zz))=choice {

                out[p..p+2]
                    .copy_from_slice(&xx.to_le_bytes());

                out[p+2..p+4]
                    .copy_from_slice(&yy.to_le_bytes());

                out[p+4..p+6]
                    .copy_from_slice(&zz.to_le_bytes());

                let g=at(ax,az);

                // Carry the original native reference flags.
                let existing=word(&out,ax,az);

                // Preserve this anchor cell's existing native flags.
                // Add only the imported descriptor identity and
                // the static-object visibility bit.
                let combined=
                    (existing&!0x01ff) |
                    (bw&0x01ff) |
                    legaia_scene_formats::field_objects::CELL_VISIBLE;

                out[g..g+2]
                    .copy_from_slice(&combined.to_le_bytes());

                ensure!(
                    native_xyz(&out,id,ax,az,lut)?
                        ==original_world,
                    "R636: native draw-anchor relocation changed world position"
                );

                moved+=1;
                } else {
                    // R636D requires a separate enabled-file opt-in.
                    let enabled=std::env::current_exe()
                        .ok()
                        .and_then(|path|
                            path.parent().map(|parent|
                                parent.join(
                                    "LCK_R636D_DUAL_OWNER.enabled"
                                ).is_file()
                            )
                        )
                        .unwrap_or(false);

                    ensure!(
                        enabled,
                        "R636D: no free native anchor for descriptor {id} at ({x},{z}); experimental dual-owner mode disabled"
                    );

                    r636d_fuse_static_and_ground(
                        objects,
                        &mut out,
                        id,
                        x,
                        z,
                        lut,
                        original_world
                    ).with_context(||format!(
                        "R636D: cannot combine static descriptor {id} with native floor at ({x},{z})"
                    ))?;

                    fused.insert(id);
                    moved+=1;
                }

            } else {
                bail!(
                    "R636: descriptor {id} gained {} MAP references; shared identity is ambiguous",
                    refs.len()
                );
            }

            // Do not let later exporters alter model identity,
            // rotation, scale-related native data, flags,
            // culling, or resource selector.
            if fused.contains(&id) {
                // Only ground capability and ground material may
                // differ from the original static descriptor.
                ensure!(
                    objects[p+6..p+18]==out[p+6..p+18] &&
                    objects[p+24..p+32]==out[p+24..p+32] &&
                    out[p+18]==(objects[p+18]|1) &&
                    out[p+19]==objects[p+19],
                    "R636D: combined descriptor {id} changed unrelated model data"
                );
            } else {
                ensure!(
                    objects[p+6..p+32]==out[p+6..p+32],
                    "R636: later stage altered imported descriptor {id} model/flags/rotation/cull fields"
                );
            }

            let final_refs=grid_refs(&out,id);

            ensure!(
                final_refs.len()==1,
                "R636: imported descriptor {id} is not uniquely referenced in final MAP"
            );
        }
    }

    Ok((out,inspected,moved))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(
        id:usize,
        flags:u16,
        x:usize,
        z:usize
    )->Vec<u8>{
        let mut map=vec![0u8;0x12000];

        let p=id*32;

        map[p..p+2]
            .copy_from_slice(&15i16.to_le_bytes());

        map[p+2..p+4]
            .copy_from_slice(&(-348i16).to_le_bytes());

        map[p+4..p+6]
            .copy_from_slice(&57i16.to_le_bytes());

        map[p+18..p+20]
            .copy_from_slice(&flags.to_le_bytes());

        map[p+30]=2;

        let g=at(x,z);

        map[g..g+2]
            .copy_from_slice(&(0x2000u16|id as u16).to_le_bytes());

        map
    }

    #[test]
    fn imported_static_objects_survive_ground_overwrites()
        ->Result<()>
    {
        // Multiple arbitrary imported descriptor IDs.
        // No special handling for shrub UID 23 or slot 509.
        for id in [509usize,478,411] {
            let retail=vec![0u8;0x12000];

            let objects=fixture(
                id,0x0012,56,32
            );

            let mut final_map=objects.clone();

            let cell=at(56,32);

            // A later floor stage claims descriptor 174.
            final_map[cell..cell+2]
                .copy_from_slice(&0x34aeu16.to_le_bytes());

            let (result,inspected,moved)=reconcile(
                &retail,
                &objects,
                &final_map,
                &[0;16]
            )?;

            assert_eq!((inspected,moved),(1,1));

            assert_eq!(
                word(&result,56,32),
                0x34ae
            );

            let refs=grid_refs(&result,id);

            assert_eq!(refs.len(),1);

            assert_eq!(
                native_xyz(
                    &objects,id,56,32,&[0;16]
                )?,
                native_xyz(
                    &result,id,
                    refs[0].0,refs[0].1,
                    &[0;16]
                )?
            );
        }

        Ok(())
    }

    #[test]
    fn actor_refs_cannot_be_moved_as_static_objects()
        ->Result<()>
    {
        let retail=vec![0u8;0x12000];

        let objects=fixture(
            421,0x0016,56,32
        );

        let mut final_map=objects.clone();

        let cell=at(56,32);

        final_map[cell..cell+2]
            .copy_from_slice(&0x34aeu16.to_le_bytes());

        assert!(
            reconcile(
                &retail,
                &objects,
                &final_map,
                &[0;16]
            ).is_err()
        );

        Ok(())
    }

    #[test]
    fn intact_import_needs_no_relocation()
        ->Result<()>
    {
        let retail=vec![0u8;0x12000];

        let objects=fixture(
            509,0x0012,56,32
        );

        let (result,inspected,moved)=reconcile(
            &retail,
            &objects,
            &objects,
            &[0;16]
        )?;

        assert_eq!((inspected,moved),(1,0));
        assert_eq!(objects,result);

        Ok(())
    }
}

// LCK_R636B_ANCHOR_OWNERSHIP
#[cfg(test)]
mod lck_r636b_anchor_tests {
    use super::*;

    fn fixture()->(Vec<u8>,Vec<u8>,Vec<u8>){
        const ID:usize=329;

        let mut retail=vec![0u8;0x12000];
        let mut objects=vec![0u8;0x12000];

        let p=ID*32;

        objects[p..p+2]
            .copy_from_slice(&15i16.to_le_bytes());

        objects[p+2..p+4]
            .copy_from_slice(&(-348i16).to_le_bytes());

        objects[p+4..p+6]
            .copy_from_slice(&57i16.to_le_bytes());

        objects[p+18..p+20]
            .copy_from_slice(&0x0012u16.to_le_bytes());

        objects[p+30]=2;

        let original=at(41,14);

        objects[original..original+2]
            .copy_from_slice(&(0x2000u16|ID as u16).to_le_bytes());

        // Occupy the local neighborhood with unrelated
        // model references. Leave one nearby candidate whose
        // word is nonzero but descriptor index is zero.
        for dz in -2i32..=2 {
            for dx in -2i32..=2 {
                if dx==0 && dz==0 {
                    continue;
                }

                let x=(41+dx) as usize;
                let z=(14+dz) as usize;
                let a=at(x,z);

                let value=if dx==1 && dz==0 {
                    legaia_scene_formats::field_objects::CELL_VISIBLE
                }else{
                    17u16
                };

                retail[a..a+2]
                    .copy_from_slice(&value.to_le_bytes());

                objects[a..a+2]
                    .copy_from_slice(&value.to_le_bytes());
            }
        }

        let mut final_map=objects.clone();

        // Native floor compositor replaces the imported
        // descriptor at the original anchor.
        final_map[original..original+2]
            .copy_from_slice(&0x34aeu16.to_le_bytes());

        (retail,objects,final_map)
    }

    #[test]
    fn nonzero_word_without_descriptor_is_usable()
        ->Result<()>
    {
        let (retail,objects,final_map)=fixture();

        let original_world=
            native_xyz(&objects,329,41,14,&[0;16])?;

        let (repaired,inspected,moved)=reconcile(
            &retail,
            &objects,
            &final_map,
            &[0;16]
        )?;

        assert_eq!((inspected,moved),(1,1));

        // The new native floor descriptor remains intact.
        assert_eq!(word(&repaired,41,14),0x34ae);

        // A separate reference now points to static 329.
        assert_eq!(word(&repaired,42,14)&511,329);

        assert_ne!(
            word(&repaired,42,14)&
                legaia_scene_formats::field_objects::CELL_VISIBLE,
            0
        );

        // Exact static world-space position is preserved.
        assert_eq!(
            native_xyz(&repaired,329,42,14,&[0;16])?,
            original_world
        );

        Ok(())
    }

    #[test]
    fn walkable_ground_is_not_taken_as_a_free_anchor()
        ->Result<()>
    {
        let (mut retail,mut objects,mut final_map)=fixture();

        let candidate=at(42,14);

        let walk=
            legaia_scene_formats::field_objects::CELL_WALK_VISIBLE;

        for map in [
            &mut retail,
            &mut objects,
            &mut final_map
        ] {
            map[candidate..candidate+2]
                .copy_from_slice(&walk.to_le_bytes());
        }

        assert!(
            reconcile(
                &retail,
                &objects,
                &final_map,
                &[0;16]
            ).is_err()
        );

        Ok(())
    }

    #[test]
    fn actor_ownership_is_still_protected()
        ->Result<()>
    {
        let (retail,mut objects,final_map)=fixture();

        let p=329*32;

        objects[p+18..p+20]
            .copy_from_slice(&0x0016u16.to_le_bytes());

        assert!(
            reconcile(
                &retail,
                &objects,
                &final_map,
                &[0;16]
            ).is_err()
        );

        Ok(())
    }
}


// LCK_R636D_DUAL_STATIC_GROUND
#[cfg(test)]
mod lck_r636d_dual_owner_tests {
    use super::*;
    use legaia_scene_formats::field_objects::{
        CELL_VISIBLE,
        CELL_WALK_VISIBLE,
    };

    #[test]
    fn static_and_walk_ground_survive_one_native_cell()
        ->Result<()>
    {
        // Test arbitrary native descriptor identities.
        // No hardcoded exception for the shrub or one source town.
        for id in [329usize,509,473] {
            let mut objects=vec![0u8;0x12000];
            let p=id*32;

            objects[p..p+2]
                .copy_from_slice(&15i16.to_le_bytes());

            objects[p+2..p+4]
                .copy_from_slice(&(-348i16).to_le_bytes());

            objects[p+4..p+6]
                .copy_from_slice(&57i16.to_le_bytes());

            objects[p+16..p+18]
                .copy_from_slice(&114u16.to_le_bytes());

            objects[p+18..p+20]
                .copy_from_slice(&0x12u16.to_le_bytes());

            objects[p+30]=2;

            let original_cell=at(41,14);

            objects[original_cell..original_cell+2]
                .copy_from_slice(
                    &(0x2000u16|id as u16).to_le_bytes()
                );

            let original_world=
                native_xyz(&objects,id,41,14,&[0;16])?;

            let mut completed=objects.clone();

            let ground_id=174usize;

            completed[
                ground_id*32+18..ground_id*32+20
            ].copy_from_slice(&1u16.to_le_bytes());

            completed[
                ground_id*32+20..ground_id*32+24
            ].copy_from_slice(&[10,20,30,40]);

            completed[original_cell..original_cell+2]
                .copy_from_slice(
                    &(CELL_WALK_VISIBLE|ground_id as u16)
                        .to_le_bytes()
                );

            r636d_fuse_static_and_ground(
                &objects,
                &mut completed,
                id,
                41,
                14,
                &[0;16],
                original_world
            )?;

            assert_eq!(
                word(&completed,41,14)&0x01ff,
                id as u16
            );

            assert_ne!(
                word(&completed,41,14)&CELL_WALK_VISIBLE,
                0
            );

            assert_ne!(
                word(&completed,41,14)&CELL_VISIBLE,
                0
            );

            let record=ObjectRecord::parse(&completed,id)
                .context("Combined record not decoded")?;

            assert_eq!(record.flags,0x0013);
            assert!(!record.is_placed());
            assert_eq!(record.pack_index_field,114);
            assert_eq!(record.cull_radius,2);

            assert_eq!(
                native_xyz(
                    &completed,id,41,14,&[0;16]
                )?,
                original_world
            );

            assert_eq!(
                &completed[p+20..p+24],
                &[10,20,30,40]
            );
        }

        Ok(())
    }

    #[test]
    fn actor_and_script_records_do_not_use_dual_owner()
        ->Result<()>
    {
        let mut objects=vec![0u8;0x12000];
        let p=329*32;

        objects[p+18..p+20]
            .copy_from_slice(&0x0016u16.to_le_bytes());

        let cell=at(41,14);

        objects[cell..cell+2]
            .copy_from_slice(&329u16.to_le_bytes());

        let mut completed=objects.clone();

        completed[174*32+18..174*32+20]
            .copy_from_slice(&1u16.to_le_bytes());

        completed[cell..cell+2]
            .copy_from_slice(
                &(CELL_WALK_VISIBLE|174u16).to_le_bytes()
            );

        assert!(
            r636d_fuse_static_and_ground(
                &objects,
                &mut completed,
                329,
                41,
                14,
                &[0;16],
                (0,0,0)
            ).is_err()
        );

        Ok(())
    }
}
