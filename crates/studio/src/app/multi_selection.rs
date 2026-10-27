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
pub(super) fn click(primary:&mut Option<Key>, selected:&mut BTreeSet<Key>, key:Key, shift:bool) {
    *selected=keys(*primary,selected);
    if shift && selected.remove(&key) {
        if *primary==Some(key) {*primary=selected.iter().next_back().copied();}
    } else {
        if !shift && !selected.contains(&key) {selected.clear();}
        selected.insert(key);*primary=Some(key);
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
        world.foreign.push(super::super::world_view::ForeignRef{uid:90,source:"synthetic-donor".into(),kind:"placement".into(),record:0,
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
        let mut rects=Vec::new();
        let mut frame=|events:Vec<egui::Event>,shift:bool,primary:&mut Option<Key>,selected:&mut BTreeSet<Key>| {
            rects.clear();
            let input=egui::RawInput{screen_rect:Some(egui::Rect::from_min_size(egui::Pos2::ZERO,egui::vec2(320.0,240.0))),
                events,modifiers:egui::Modifiers{shift,..Default::default()},..Default::default()};
            let _=ctx.run(input,|ctx|{egui::CentralPanel::default().show(ctx,|ui|{
                for i in 0..3 {rects.push(super::super::editor_hierarchy_row(ui,format!("Object {i}"),(false,i),primary,selected,&mut None,&mut None));}
            });});rects.clone()
        };
        let rows=frame(vec![],false,&mut primary,&mut selected);
        for (i,shift) in [(0,false),(1,true),(2,true),(0,false),(1,true)] {
            let pos=rows[i].center();let modifiers=egui::Modifiers{shift,..Default::default()};
            frame(vec![egui::Event::PointerMoved(pos),egui::Event::PointerButton{pos,button:egui::PointerButton::Primary,pressed:true,modifiers}],shift,&mut primary,&mut selected);
            frame(vec![egui::Event::PointerButton{pos,button:egui::PointerButton::Primary,pressed:false,modifiers}],shift,&mut primary,&mut selected);
        }
        assert_eq!(selected,BTreeSet::from([(false,0),(false,2)]));
        // A viewport toggle operates on exactly the same set and primary.
        click(&mut primary,&mut selected,(false,2),true);
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
    fn both_surfaces_share_toggle_preserve_and_primary_rules() {
        let mut primary=None;let mut selected=BTreeSet::new();
        click(&mut primary,&mut selected,(false,1),false); // hierarchy
        click(&mut primary,&mut selected,(true,2),true); // viewport
        click(&mut primary,&mut selected,(false,3),true);
        click(&mut primary,&mut selected,(false,1),false);
        assert_eq!(selected.len(),3);assert_eq!(primary,Some((false,1)));
        click(&mut primary,&mut selected,(true,2),true);
        assert_eq!(selected.len(),2);
        click(&mut primary,&mut selected,(false,1),true);
        assert_eq!(primary,Some((false,3)));
        click(&mut primary,&mut selected,(false,3),true);
        assert!(selected.is_empty());assert_eq!(primary,None);
        click(&mut primary,&mut selected,(false,4),false);
        click(&mut primary,&mut selected,(false,5),true);
        click(&mut primary,&mut selected,(false,6),false);
        assert_eq!(selected,BTreeSet::from([(false,6)]));
        assert!(keys(None,&selected).is_empty());
    }
}
