//! Shared viewport/hierarchy selection and transactional batch editor operations.
use std::collections::BTreeSet;
use super::world_view::WorldModel;
pub(super) type Key=(bool,usize);

/// Legacy single-object commands may replace the primary directly. Never retain
/// an unrelated stale group when that happens, or when a scene is closed.
pub(super) fn keys(primary:Option<Key>, selected:&BTreeSet<Key>)->BTreeSet<Key> {
    match primary {Some(key) if selected.contains(&key)=>selected.clone(),
        Some(key)=>BTreeSet::from([key]),None=>BTreeSet::new()}
}
/// Keep an already-selected member at mouse-down so a drag can move the group.
/// A plain click collapses on release; Ctrl toggling a selected member is also
/// deferred so the existing Ctrl+drag vertical gesture remains available.
pub(super) fn viewport_press(primary:&mut Option<Key>, selected:&mut BTreeSet<Key>, key:Key, shift:bool, ctrl:bool) {
    *selected=keys(*primary,selected);
    if !shift && !ctrl && !selected.contains(&key) {selected.clear();}
    selected.insert(key);*primary=Some(key);
}
pub(super) fn toggle(primary:&mut Option<Key>, selected:&mut BTreeSet<Key>, key:Key) {
    *selected=keys(*primary,selected);
    if selected.remove(&key) {
        if *primary==Some(key) {*primary=selected.iter().next_back().copied();}
    } else {selected.insert(key);*primary=Some(key);}
}
pub(super) fn single(primary:&mut Option<Key>, selected:&mut BTreeSet<Key>, key:Key) {
    *primary=Some(key);*selected=BTreeSet::from([key]);
}

#[derive(Default)]
pub(super) struct HierarchyInput {
    pub rows: Vec<Key>,
    pub click: Option<(Key, eframe::egui::Modifiers, bool)>,
}
impl HierarchyInput {
    /// Apply after collecting all expanded, filtered rows, so reverse ranges
    /// and ranges across visible hierarchy groups use exactly the display order.
    pub fn apply(&self, primary:&mut Option<Key>, selected:&mut BTreeSet<Key>, anchor:&mut Option<Key>) -> bool {
        let Some((key,modifiers,secondary))=self.click else {return false;};
        *selected=keys(*primary,selected);
        if secondary {
            viewport_press(primary,selected,key,false,false);
        } else if modifiers.shift {
            let start=anchor.or(*primary).and_then(|k|self.rows.iter().position(|r|*r==k));
            let end=self.rows.iter().position(|r|*r==key);
            if !modifiers.ctrl {selected.clear();}
            if let (Some(a),Some(b))=(start,end) {selected.extend(&self.rows[a.min(b)..=a.max(b)]);}
            else {selected.insert(key);*anchor=Some(key);}
            *primary=Some(key);
        } else if modifiers.ctrl {
            toggle(primary,selected,key);*anchor=Some(key);
        } else {single(primary,selected,key);*anchor=Some(key);}
        true
    }
}

/// Prevalidate every participant before changing any vertex. One rejected member
/// leaves the entire group intact. Collision definitions remain independent;
/// existing compilation applies each draw's final transform.
pub(super) fn move_offsets(world:&mut WorldModel, edits:&[(Key,[f32;3])])->Result<(),String> {
    let model=world.model.as_ref().ok_or("World geometry unavailable")?;
    let mut seen=BTreeSet::new();
    for &(key,offset) in edits {
        let draw=world.at(key.0,key.1).ok_or("Group member no longer exists")?;
        if !seen.insert(key)||draw.hidden||draw.vertex_start>=draw.vertex_end||draw.vertex_end>model.positions.len()||draw.vertex_end>model.rest.len()
            ||offset.iter().any(|v|!v.is_finite()||v.abs()>=1_000_000.0) {
            return Err("Invalid group movement; no objects moved".into());
        }
    }
    for &(key,offset) in edits {assert!(world.set_offset(key.0,key.1,offset));}
    Ok(())
}

fn validate_draws(world:&WorldModel, edits:&[(Key,[f32;3])], scale:bool)->Result<(),String> {
    let model=world.model.as_ref().ok_or("World geometry unavailable")?;let mut seen=BTreeSet::new();
    for &(key,value) in edits {
        let d=world.at(key.0,key.1).ok_or("Selected member no longer exists")?;
        if !seen.insert(key)||d.hidden||d.vertex_start>=d.vertex_end||d.vertex_end>model.positions.len()||d.vertex_end>model.rest.len()
            ||value.iter().any(|v|!v.is_finite()||if scale {!(0.05..=20.0).contains(v)}else{v.abs()>=1000.0}) {
            return Err("Invalid group transform; no objects changed".into());
        }
    }
    Ok(())
}
/// Inspector values apply a common angular delta about each object's own pivot.
pub(super) fn rotate(world:&mut WorldModel, primary:Key, selected:&BTreeSet<Key>, desired:[f32;3])->Result<(),String> {
    let before=world.at(primary.0,primary.1).ok_or("Primary object unavailable")?.rotation;
    let edits=keys(Some(primary),selected).into_iter().map(|key| {
        let d=world.at(key.0,key.1).ok_or("Selected object unavailable")?;
        Ok((key,[0,1,2].map(|a|d.rotation[a]+desired[a]-before[a])))
    }).collect::<Result<Vec<_>,String>>()?;
    validate_draws(world,&edits,false)?;
    for (key,value) in edits {assert!(world.set_rotation(key.0,key.1,value));} Ok(())
}
/// Preserve each member's individual scale using the primary's per-axis ratio.
pub(super) fn scale(world:&mut WorldModel, primary:Key, selected:&BTreeSet<Key>, desired:[f32;3])->Result<(),String> {
    let before=world.at(primary.0,primary.1).ok_or("Primary object unavailable")?.scale;
    let edits=keys(Some(primary),selected).into_iter().map(|key| {
        let d=world.at(key.0,key.1).ok_or("Selected object unavailable")?;
        Ok((key,[0,1,2].map(|a|d.scale[a]*desired[a]/before[a])))
    }).collect::<Result<Vec<_>,String>>()?;
    validate_draws(world,&edits,true)?;
    for (key,value) in edits {assert!(world.set_scale(key.0,key.1,value));} Ok(())
}

/// Stage infrequent duplication as one transaction, preserving every selected
/// instance's name, recipe, transform and NPC appearance. One bad member cannot
/// leave a partially duplicated group or alter an unrelated source.
pub(super) fn duplicate(world:&mut WorldModel, selected:&BTreeSet<Key>, first_created:usize)->Result<Vec<Key>,String> {
    if selected.is_empty() {return Err("No objects selected".into());}
    let mut staged=world.clone();let mut result=Vec::new();
    let mut uid=world.draws.iter().filter_map(|d|d.editor_uid).max().unwrap_or(0);
    for &key in selected {
        let d=world.at(key.0,key.1).ok_or("Selected object unavailable")?;
        if d.hidden||d.vertex_start>=d.vertex_end {return Err("Selected object has no visible geometry to duplicate".into());}
        uid=uid.checked_add(1).ok_or("No remaining instance UIDs")?;
        let foreign=d.editor_uid.is_some_and(|uid|world.foreign.iter().any(|f|f.uid==uid));
        let source=if foreign {key}else{d.source_ref.unwrap_or(key)};
        let mut offset=d.offset;offset[0]+=128.0;
        let next=staged.duplicate_native(source,first_created,uid,offset)?;
        if world.collision.source(world,key).is_some() {staged.inherit_collision_recipe(key,next)?;}
        staged.set_display_name(next,d.display_name.clone());
        if !staged.set_rotation(next.0,next.1,d.rotation)||!staged.set_scale(next.0,next.1,d.scale) {return Err("Cannot inherit duplicate transform".into());}
        if let Some(npc)=world.npc(key) {staged.set_npc_model(next,npc.model_index)?;staged.set_npc_animation(next,npc.clip,npc.frame)?;}
        result.push(next);
    }
    *world=staged;Ok(result)
}

/// Deletion is infrequent; stage it on a clone so malformed/missing members do
/// not produce partial deletion. Native hiding is persisted and must pass the
/// native export deletion gate; created instances are removed by stable key.
pub(super) fn delete(world:&mut WorldModel, keys:&BTreeSet<Key>)->Result<(),String> {
    let mut staged=world.clone();
    for &key in keys {
        let draw=staged.at(key.0,key.1).ok_or("Group member no longer exists")?;
        if draw.editor_uid.is_some() {staged.delete_created(key)?;}
        else if !staged.set_hidden(key,true) {return Err("Cannot hide group member".into());}
    }
    *world=staged;Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::{collision_recipes,scene_instances,editor_history_capture_group,editor_history_selected};
    fn three()->WorldModel {
        let mut world=collision_recipes::properties_fixture();
        world.duplicate_native((false,0),1,91,[256.0,0.0,0.0]).unwrap();world
    }
    #[test]
    fn three_object_translation_vertical_spacing_materials_and_persistence() {
        let mut world=three();let original=world.clone();
        let keys=BTreeSet::from([(false,0),(false,1),(false,2)]);
        let before=editor_history_capture_group(&world,"test",Some((false,1)),&keys);
        let starts=keys.iter().map(|&key|(key,world.at(key.0,key.1).unwrap().offset)).collect::<Vec<_>>();
        for delta in [[64.0,0.0,-32.0],[64.0,128.0,-32.0]] {
            let edits=starts.iter().map(|(key,start)|(*key,[0,1,2].map(|a|start[a]+delta[a]))).collect::<Vec<_>>();
            move_offsets(&mut world,&edits).unwrap();
            for key in &keys {
                let a=world.at(key.0,key.1).unwrap();let b=original.at(key.0,key.1).unwrap();
                assert_eq!([0,1,2].map(|i|a.center[i]-b.center[i]),delta);
            }
        }
        assert_eq!(world.collision.recipes,original.collision.recipes);
        assert_eq!(world.model.as_ref().unwrap().uvs,original.model.as_ref().unwrap().uvs);
        assert_eq!(world.model.as_ref().unwrap().cba_tsb,original.model.as_ref().unwrap().cba_tsb);
        let after=editor_history_capture_group(&world,"test",Some((false,1)),&keys);
        assert_eq!(after.selection.len(),3);
        let index=legaia_engine_scene::scene::ProtIndex::from_bytes(legaia_disc_patch::disc::synth::synth_prot(&[]),None).unwrap();
        // The same complete snapshots used by one gesture undo/redo, including UIDs.
        for state in [&before,&after,&before,&after] {
            let doc=scene_instances::parse(&state.overlay,"test",&world).unwrap();
            scene_instances::apply(&doc,&mut world,1,&index).unwrap();
            assert_eq!(scene_instances::serialize("test",&world),state.overlay);
            assert_eq!(state.selection.iter().filter_map(|t|editor_history_selected(&world,t)).count(),3);
        }
    }
    #[test]
    fn batch_delete_and_complete_snapshot_restore_preserve_unrelated_geometry() {
        let mut world=three();let original=world.clone();
        let keys=BTreeSet::from([(false,0),(false,1)]);
        let before=editor_history_capture_group(&world,"test",Some((false,1)),&keys);
        delete(&mut world,&keys).unwrap();
        assert!(world.at(false,0).unwrap().hidden);assert!(world.at(false,1).is_none());
        assert_eq!(world.at(false,2).unwrap().offset,original.at(false,2).unwrap().offset);
        let after=super::super::editor_history_capture(&world,"test",None);
        let index=legaia_engine_scene::scene::ProtIndex::from_bytes(legaia_disc_patch::disc::synth::synth_prot(&[]),None).unwrap();
        for state in [&before,&after] {
            let doc=scene_instances::parse(&state.overlay,"test",&world).unwrap();
            scene_instances::apply(&doc,&mut world,1,&index).unwrap();
            assert_eq!(scene_instances::serialize("test",&world),state.overlay);
        }
    }
    #[test]
    fn invalid_member_cannot_partially_move_or_delete_a_batch() {
        let mut world=three();let before=scene_instances::serialize("test",&world);
        assert!(move_offsets(&mut world,&[((false,0),[64.0;3]),((false,99),[1.0;3])]).is_err());
        assert_eq!(scene_instances::serialize("test",&world),before);
        assert!(delete(&mut world,&BTreeSet::from([(false,0),(false,99)])).is_err());
        assert_eq!(scene_instances::serialize("test",&world),before);
    }
    #[test]
    fn deleting_an_import_releases_only_its_instance_metadata_and_material_view() {
        let mut world=three();let id=world.at(false,1).unwrap().id;
        world.foreign.push(super::super::world_view::ForeignRef{uid:90,static_architecture:true,source:"synthetic-donor".into(),kind:"placement".into(),record:0,
            raw_tmd:vec![],native_record:None,donor_wall_bits:vec![],donor_center:[0.0;3]});
        world.foreign_vram.insert(id,std::sync::Arc::new(legaia_tim::Vram::new()));
        let unrelated=world.at(false,2).unwrap().offset;
        delete(&mut world,&BTreeSet::from([(false,1)])).unwrap();
        assert!(world.foreign.is_empty());assert!(world.foreign_vram.is_empty());
        assert!(!world.collision.recipes.iter().any(|a|a.owner==collision_recipes::ObjectId::Created(90)));
        assert_eq!(world.at(false,2).unwrap().offset,unrelated);
        assert!(!world.at(false,0).unwrap().hidden);
    }
    #[test]
    fn hierarchy_shift_clicks_share_the_viewport_selection() {
        use eframe::egui;
        let ctx=egui::Context::default();let mut primary=None;let mut selected=BTreeSet::new();
        let mut rects=Vec::new();let mut anchor=None;
        let mut frame=|events:Vec<egui::Event>,modifiers:egui::Modifiers,primary:&mut Option<Key>,selected:&mut BTreeSet<Key>| {
            let mut input=HierarchyInput::default();
            rects.clear();
            let raw=egui::RawInput{screen_rect:Some(egui::Rect::from_min_size(egui::Pos2::ZERO,egui::vec2(320.0,240.0))),
                events,modifiers,..Default::default()};
            let _=ctx.run(raw,|ctx|{egui::CentralPanel::default().show(ctx,|ui|{
                for i in 0..3 {rects.push(super::super::editor_hierarchy_row(ui,format!("Object {i}"),(false,i),primary,selected,&mut None,&mut None,&mut input));}
            });});input.apply(primary,selected,&mut anchor);rects.clone()
        };
        let rows=frame(vec![],Default::default(),&mut primary,&mut selected);
        for (i,shift,ctrl) in [(0,false,false),(1,false,true),(2,true,false),(0,false,true),(1,false,true)] {
            let pos=rows[i].center();let modifiers=egui::Modifiers{shift,ctrl,..Default::default()};
            frame(vec![egui::Event::PointerMoved(pos),egui::Event::PointerButton{pos,button:egui::PointerButton::Primary,pressed:true,modifiers}],modifiers,&mut primary,&mut selected);
            frame(vec![egui::Event::PointerButton{pos,button:egui::PointerButton::Primary,pressed:false,modifiers}],modifiers,&mut primary,&mut selected);
        }
        assert_eq!(selected,BTreeSet::from([(false,0),(false,2)]));
        // A viewport toggle operates on exactly the same set and primary.
        toggle(&mut primary,&mut selected,(false,2));
        assert_eq!(selected,BTreeSet::from([(false,0)]));
    }
    #[test]
    fn pointer_translation_and_vertical_drag_apply_one_common_snapped_delta() {
        let mut world=three();let original=world.clone();let primary=(false,1);
        let initial=world.at(false,1).unwrap().offset;
        for vertical in [false,true] {
            let desired=super::super::editor_drag_offset(initial,eframe::egui::vec2(38.0,17.0),crate::raster::Camera::default(),512.0,240.0,true,vertical);
            let snapped=super::super::world_view::quantize_offset(desired,16.0);
            let delta=[0,1,2].map(|a|snapped[a]-initial[a]);
            let edits=original.draws.iter().map(|d|((d.terrain,d.index),[0,1,2].map(|a|d.offset[a]+delta[a]))).collect::<Vec<_>>();
            move_offsets(&mut world,&edits).unwrap();
            assert_eq!(world.at(primary.0,primary.1).unwrap().offset,snapped);
            for (draw,start) in world.draws.iter().zip(&original.draws) {
                assert_eq!([0,1,2].map(|a|draw.offset[a]-start.offset[a]),delta);
            }
            if vertical {assert_eq!(delta[0],0.0);assert_eq!(delta[2],0.0);assert_ne!(delta[1],0.0);}
        }
    }
    #[test]
    fn viewport_shift_adds_idempotently_and_ctrl_toggles_members() {
        let mut primary=None;let mut selected=BTreeSet::new();
        viewport_press(&mut primary,&mut selected,(false,1),false,false);
        viewport_press(&mut primary,&mut selected,(true,2),true,false);
        viewport_press(&mut primary,&mut selected,(true,2),true,false);
        assert_eq!(selected.len(),2);assert_eq!(primary,Some((true,2)));
        toggle(&mut primary,&mut selected,(true,2));
        assert_eq!(selected,BTreeSet::from([(false,1)]));
        toggle(&mut primary,&mut selected,(false,1));assert!(selected.is_empty());assert_eq!(primary,None);
        single(&mut primary,&mut selected,(false,4));
        viewport_press(&mut primary,&mut selected,(false,5),true,false);
        single(&mut primary,&mut selected,(false,6));
        assert_eq!(selected,BTreeSet::from([(false,6)]));assert!(keys(None,&selected).is_empty());
    }

    #[test]
    fn hierarchy_ranges_follow_visible_order_in_both_directions_and_ctrl_shift_unions() {
        let rows=vec![(false,4),(false,1),(true,8),(false,20)];
        let mut primary=Some(rows[2]);let mut selected=BTreeSet::from([rows[2]]);let mut anchor=primary;
        let apply=|key,shift,ctrl,primary:&mut Option<Key>,selected:&mut BTreeSet<Key>,anchor:&mut Option<Key>| {
            HierarchyInput{rows:rows.clone(),click:Some((key,eframe::egui::Modifiers{shift,ctrl,..Default::default()},false))}.apply(primary,selected,anchor)
        };
        apply(rows[0],true,false,&mut primary,&mut selected,&mut anchor);
        assert_eq!(selected,BTreeSet::from([rows[0],rows[1],rows[2]]));assert_eq!(anchor,Some(rows[2]));
        apply(rows[3],false,true,&mut primary,&mut selected,&mut anchor);
        apply(rows[2],true,true,&mut primary,&mut selected,&mut anchor);
        assert_eq!(selected.len(),4);
        apply(rows[1],false,false,&mut primary,&mut selected,&mut anchor);assert_eq!(selected,BTreeSet::from([rows[1]]));
    }

    #[test]
    fn group_rotation_scale_and_duplication_preserve_other_instances_and_metadata() {
        let mut world=three();let keys=BTreeSet::from([(false,0),(false,1)]);
        world.set_rotation(false,1,[0.1,0.2,0.3]);world.set_scale(false,1,[2.0,1.0,0.5]);
        let unrelated=world.at(false,2).unwrap().clone();
        rotate(&mut world,(false,0),&keys,[0.0,0.5,0.0]).unwrap();
        assert_eq!(world.at(false,0).unwrap().rotation,[0.0,0.5,0.0]);
        assert!((world.at(false,1).unwrap().rotation[1]-0.7).abs()<1e-6);
        scale(&mut world,(false,0),&keys,[2.0,2.0,2.0]).unwrap();
        assert_eq!(world.at(false,1).unwrap().scale,[4.0,2.0,1.0]);
        let before=scene_instances::serialize("test",&world);
        assert!(scale(&mut world,(false,0),&keys,[20.0;3]).is_err());assert_eq!(before,scene_instances::serialize("test",&world));
        let created=duplicate(&mut world,&keys,1).unwrap();assert_eq!(created.len(),2);
        for (old,new) in keys.iter().zip(&created) {
            let a=world.at(old.0,old.1).unwrap();let b=world.at(new.0,new.1).unwrap();
            assert_eq!(b.offset,[a.offset[0]+128.0,a.offset[1],a.offset[2]]);
            assert_eq!(b.rotation,a.rotation);assert_eq!(b.scale,a.scale);assert_ne!(b.editor_uid,a.editor_uid);
        }
        assert_eq!(world.at(false,2).unwrap().offset,unrelated.offset);
        assert_eq!(world.at(false,2).unwrap().rotation,unrelated.rotation);
        let before=scene_instances::serialize("test",&world);
        assert!(duplicate(&mut world,&BTreeSet::from([(false,0),(false,999)]),1).is_err());
        assert_eq!(before,scene_instances::serialize("test",&world));
    }

    #[test]
    fn foreign_duplication_uses_selected_instance_and_keeps_donor_materials_and_collision(){
        use std::sync::Arc;
        let mut w=super::super::collision_recipes::properties_fixture();
        let key=(false,1);let uid=w.at(false,1).unwrap().editor_uid.unwrap();
        w.draws.iter_mut().find(|d|(d.terrain,d.index)==key).unwrap().source_ref=Some((false,usize::MAX));
        let raw=vec![1,2,3,4];w.foreign.push(super::super::world_view::ForeignRef{uid,static_architecture:true,source:"conc".into(),kind:"placement".into(),record:19,raw_tmd:raw.clone(),native_record:Some([9;32]),donor_wall_bits:vec![(2,3)],donor_center:[5.,6.,7.]});
        let texture=Arc::new(legaia_tim::Vram::new());let id=w.at(false,1).unwrap().id;w.foreign_vram.insert(id,texture.clone());
        let provenance=w.collision.sources[&(false,0)].clone();w.collision.foreign_sources.insert(uid,provenance.clone());
        w.set_rotation(false,1,[0.1,0.2,0.3]);w.set_scale(false,1,[2.,1.,0.5]);
        let old=scene_instances::serialize("test",&w);
        let next=duplicate(&mut w,&BTreeSet::from([key]),2).unwrap()[0];
        let copy=w.at(next.0,next.1).unwrap();let copy_uid=copy.editor_uid.unwrap();
        assert_ne!(uid,copy_uid);assert!(Arc::ptr_eq(&w.foreign_vram[&copy.id],&texture));
        assert_eq!(copy.rotation,[0.1,0.2,0.3]);assert_eq!(copy.scale,[2.,1.,0.5]);
        let donor=w.foreign.iter().find(|f|f.uid==copy_uid).unwrap();assert!(donor.static_architecture);assert_eq!(donor.raw_tmd,raw);assert_eq!(donor.native_record,Some([9;32]));assert_eq!(donor.donor_wall_bits,vec![(2,3)]);
        assert_eq!(w.collision.foreign_sources[&copy_uid],provenance);
        let text=scene_instances::serialize("test",&w);assert!(text.contains(&format!("FOREIGN\t{copy_uid}\t")));assert!(!text.contains(&format!("INSTANCE\t{copy_uid}\t")));
        assert!(text.contains(&format!("FOREIGN_STATIC\t{copy_uid}\n")));
        assert!(scene_instances::parse(&text,"test",&w).unwrap().foreign.iter().any(|f|f.uid==copy_uid&&f.static_architecture));
        assert_ne!(old,text);let twice=duplicate(&mut w,&BTreeSet::from([next]),2).unwrap();assert_eq!(twice.len(),1);
        assert_eq!(w.foreign.len(),3);
    }
}
