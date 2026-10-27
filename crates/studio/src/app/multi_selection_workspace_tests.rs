//! Automated egui event-loop tests. These are not manual GUI verification.
use super::*;
use std::collections::BTreeSet;

fn fixture() -> (App, egui::Context) {
    let mut app=App::new(Studio::new(None),None,Automation::default());
    app.editor_names=Catalog::default();app.editor_catalog_error=None;
    let (scene,_)=ps1_export::tests::actor_fixture();
    let mut world=collision_recipes::properties_fixture();
    world.duplicate_native((false,0),1,91,[256.0,0.0,0.0]).unwrap();
    let index=legaia_engine_scene::scene::ProtIndex::from_bytes(legaia_disc_patch::disc::synth::synth_prot(&[]),None).unwrap();
    app.studio.library=Some(Arc::new(crate::Library{source:Source::Disc("unused-synthetic.img".into()),entries:vec![],cdname:None,iso_files:vec![],region:None,exe:None,index:Arc::new(index)}));
    app.native_scene=Some(("synthetic".into(),NativeSceneSlot::Ready(Arc::new(scene),Arc::new(world))));
    (app,egui::Context::default())
}
fn frame(app:&mut App,ctx:&egui::Context,events:Vec<egui::Event>,modifiers:egui::Modifiers)->egui::FullOutput {
    ctx.run(egui::RawInput{screen_rect:Some(egui::Rect::from_min_size(egui::Pos2::ZERO,egui::vec2(1280.0,820.0))),events,modifiers,..Default::default()},|ctx|app.editor_workspace(ctx))
}
fn world(app:&App)->&world_view::WorldModel {
    match &app.native_scene {Some((_,NativeSceneSlot::Ready(_,w)))=>w,_=>panic!("Missing test scene")}
}
fn viewport_hit(app:&App,output:&egui::FullOutput,id:u32)->egui::Pos2 {
    let rect=output.shapes.iter().filter_map(|s|match &s.shape {
        egui::Shape::Mesh(m) if m.texture_id!=egui::TextureId::Managed(0)=>Some(m.calc_bounds()),_=>None,
    }).max_by(|a,b|a.area().total_cmp(&b.area())).expect("Missing viewport");
    let px=[(rect.width() as usize).clamp(16,900),(rect.height() as usize).clamp(16,680)];
    let model=world(app).model.as_ref().unwrap();
    let point=(2..px[1]-2).step_by(2).find_map(|y|(2..px[0]-2).step_by(2).find_map(|x|
        ([-2.0,0.0,2.0].iter().all(|dx|[-2.0,0.0,2.0].iter().all(|dy|
            crate::raster::pick_instance(model,app.native_view.cam,px[0],px[1],[x as f32+dx,y as f32+dy])==Some(id)))).then_some([x,y]))).expect("Fixture member not visible");
    let x=if app.native_view.world_mirrored {px[0]-1-point[0]}else{point[0]};
    rect.min+egui::vec2(x as f32/px[0] as f32*rect.width(),point[1] as f32/px[1] as f32*rect.height())
}
fn click(app:&mut App,ctx:&egui::Context,pos:egui::Pos2,shift:bool,ctrl:bool) {
    let modifiers=egui::Modifiers{shift,ctrl,..Default::default()};
    frame(app,ctx,vec![egui::Event::PointerMoved(pos),egui::Event::PointerButton{pos,button:egui::PointerButton::Primary,pressed:true,modifiers}],modifiers);
    frame(app,ctx,vec![egui::Event::PointerButton{pos,button:egui::PointerButton::Primary,pressed:false,modifiers}],modifiers);
}
fn key(app:&mut App,ctx:&egui::Context,key:egui::Key) {
    let modifiers=egui::Modifiers{ctrl:true,command:true,..Default::default()};
    frame(app,ctx,vec![egui::Event::Key{key,physical_key:None,pressed:true,repeat:false,modifiers}],modifiers);
    frame(app,ctx,vec![egui::Event::Key{key,physical_key:None,pressed:false,repeat:false,modifiers:Default::default()}],Default::default());
}

#[test]
fn real_viewport_events_add_toggle_plain_click_and_highlight_without_editing_geometry() {
    let (mut app,ctx)=fixture();let initial=scene_instances::serialize("synthetic",world(&app));
    let output=frame(&mut app,&ctx,vec![],Default::default());let camera=app.native_view.cam;
    let points=[0,1,2].map(|id|viewport_hit(&app,&output,id));
    click(&mut app,&ctx,points[0],false,false);
    assert_eq!(app.native_selection,BTreeSet::from([(false,0)]));
    click(&mut app,&ctx,points[1],true,false);
    assert_eq!(app.native_selection,BTreeSet::from([(false,0),(false,1)]));
    click(&mut app,&ctx,points[1],true,false);assert_eq!(app.native_selection.len(),2);
    click(&mut app,&ctx,points[2],false,true);assert_eq!(app.native_selection.len(),3);
    click(&mut app,&ctx,points[1],false,true);assert_eq!(app.native_selection,BTreeSet::from([(false,0),(false,2)]));
    frame(&mut app,&ctx,vec![],Default::default());assert_eq!(app.native_view.world_selected.len(),2);
    click(&mut app,&ctx,points[0],false,false);assert_eq!(app.native_selection,BTreeSet::from([(false,0)]));
    assert_eq!(app.native_view.cam,camera);assert_eq!(scene_instances::serialize("synthetic",world(&app)),initial);
    assert!(app.editor_undo.is_empty());
}

#[test]
fn ctrl_drag_keeps_group_and_moves_only_y_in_one_undo_redo() {
    let (mut app,ctx)=fixture();app.native_selected=Some((false,0));app.native_selection=BTreeSet::from([(false,0),(false,1)]);
    let output=frame(&mut app,&ctx,vec![],Default::default());let pos=viewport_hit(&app,&output,0);
    let before=scene_instances::serialize("synthetic",world(&app));let camera=app.native_view.cam;
    let starts=world(&app).draws.iter().map(|d|d.offset).collect::<Vec<_>>();
    let modifiers=egui::Modifiers{ctrl:true,..Default::default()};
    frame(&mut app,&ctx,vec![egui::Event::PointerMoved(pos),egui::Event::PointerButton{pos,button:egui::PointerButton::Primary,pressed:true,modifiers}],modifiers);
    let end=pos+egui::vec2(0.0,30.0);
    frame(&mut app,&ctx,vec![egui::Event::PointerMoved(end)],modifiers);
    frame(&mut app,&ctx,vec![egui::Event::PointerButton{pos:end,button:egui::PointerButton::Primary,pressed:false,modifiers}],modifiers);
    assert_eq!(app.native_selection.len(),2);assert_eq!(app.editor_undo.len(),1);
    for i in 0..2 {let d=world(&app).at(false,i).unwrap();assert_eq!(d.offset[0],starts[i][0]);assert_eq!(d.offset[2],starts[i][2]);assert_ne!(d.offset[1],starts[i][1]);}
    assert_eq!(world(&app).at(false,2).unwrap().offset,starts[2]);assert_eq!(app.native_view.cam,camera);
    let after=scene_instances::serialize("synthetic",world(&app));
    key(&mut app,&ctx,egui::Key::Z);assert_eq!(scene_instances::serialize("synthetic",world(&app)),before);
    key(&mut app,&ctx,egui::Key::Y);assert_eq!(scene_instances::serialize("synthetic",world(&app)),after);
}

#[test]
fn keyboard_duplicate_creates_whole_group_and_restores_selection_with_undo_redo() {
    let (mut app,ctx)=fixture();app.native_selected=Some((false,1));app.native_selection=BTreeSet::from([(false,0),(false,1)]);
    let before=scene_instances::serialize("synthetic",world(&app));frame(&mut app,&ctx,vec![],Default::default());
    key(&mut app,&ctx,egui::Key::D);
    assert!(app.studio.error.is_none(),"{:?}",app.studio.error);assert_eq!(world(&app).draws.len(),5);
    assert_eq!(app.native_selection.len(),2);assert!(app.native_selection.iter().all(|k|![(false,0),(false,1),(false,2)].contains(k)));
    assert_eq!(app.editor_undo.len(),1);let after=scene_instances::serialize("synthetic",world(&app));
    key(&mut app,&ctx,egui::Key::Z);assert_eq!(scene_instances::serialize("synthetic",world(&app)),before);assert_eq!(app.native_selection.len(),2);
    key(&mut app,&ctx,egui::Key::Y);assert_eq!(scene_instances::serialize("synthetic",world(&app)),after);assert_eq!(app.native_selection.len(),2);
}

#[test]
fn hierarchy_ctrl_range_and_viewport_selection_share_inspector_and_highlights() {
    fn texts(shape:&egui::Shape,output:&mut Vec<(String,egui::Pos2)>) {
        match shape {egui::Shape::Text(t)=>output.push((t.galley.job.text.clone(),t.pos)),
            egui::Shape::Vec(shapes)=>for s in shapes {texts(s,output)},_=>{}}
    }
    let (mut app,ctx)=fixture();let output=frame(&mut app,&ctx,vec![],Default::default());
    let mut labels=Vec::new();for s in &output.shapes {texts(&s.shape,&mut labels);}
    let first=labels.iter().find(|(s,_)|s.contains("[copy #90]")).expect("First copy row missing").1+egui::vec2(8.0,5.0);
    let second=labels.iter().find(|(s,_)|s.contains("[copy #91]")).expect("Second copy row missing").1+egui::vec2(8.0,5.0);
    click(&mut app,&ctx,first,false,false);click(&mut app,&ctx,second,false,true);
    assert_eq!(app.native_selection,BTreeSet::from([(false,1),(false,2)]));
    click(&mut app,&ctx,first,true,false);assert_eq!(app.native_selection.len(),2);
    frame(&mut app,&ctx,vec![],Default::default());assert_eq!(app.native_view.world_selected.len(),2);
    // An open inspector follows deliberate scene selection on either surface.
    app.editor_properties_target=Some((false,2));
    click(&mut app,&ctx,first,false,false);
    assert_eq!(app.editor_properties_target,Some((false,1)));assert_eq!(app.native_selection.len(),1);
}

#[test]
fn right_drag_rotates_every_member_with_one_undo_redo_and_no_camera_change() {
    let (mut app,ctx)=fixture();app.native_selected=Some((false,0));app.native_selection=BTreeSet::from([(false,0),(false,1)]);
    let output=frame(&mut app,&ctx,vec![],Default::default());let pos=viewport_hit(&app,&output,0);
    let camera=app.native_view.cam;let before=scene_instances::serialize("synthetic",world(&app));
    frame(&mut app,&ctx,vec![egui::Event::PointerMoved(pos),egui::Event::PointerButton{pos,button:egui::PointerButton::Secondary,pressed:true,modifiers:Default::default()}],Default::default());
    let end=pos+egui::vec2(28.0,0.0);frame(&mut app,&ctx,vec![egui::Event::PointerMoved(end)],Default::default());
    frame(&mut app,&ctx,vec![egui::Event::PointerButton{pos:end,button:egui::PointerButton::Secondary,pressed:false,modifiers:Default::default()}],Default::default());
    assert_ne!(world(&app).at(false,0).unwrap().rotation,[0.0;3]);
    assert_eq!(world(&app).at(false,0).unwrap().rotation,world(&app).at(false,1).unwrap().rotation);
    assert_eq!(world(&app).at(false,2).unwrap().rotation,[0.0;3]);assert_eq!(app.native_view.cam,camera);
    assert_eq!(app.editor_undo.len(),1);let after=scene_instances::serialize("synthetic",world(&app));
    key(&mut app,&ctx,egui::Key::Z);assert_eq!(scene_instances::serialize("synthetic",world(&app)),before);
    key(&mut app,&ctx,egui::Key::Y);assert_eq!(scene_instances::serialize("synthetic",world(&app)),after);
}
