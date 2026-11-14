//! Record-level controls; existing camera/selection gestures stay in app.rs.
use super::{form_database::*, lck_project::ProjectFile, App, EditorSceneHistory, NativeSceneSlot};
use anyhow::{Context, Result};
use eframe::egui;
use std::sync::Arc;
#[derive(Clone)]
pub(super) enum Action {
    Transform(FormKey, Transform),
    DuplicateReference(FormKey),
    DeleteReference(FormKey),
    SwitchCell(FormKey),
    Save,
    SaveAs,
    Open,
    Recover,
    Place(FormKey, [f32; 3]),
    DuplicateBase(FormKey),
    BaseId(FormKey, String),
    ReferenceId(FormKey, Option<String>),
    AssignBase(FormKey, FormKey),
    Geometry(FormKey, FormKey),
    Tint(FormKey, [f32; 3]),
    Disable(FormKey, bool),
    Restore(FormKey),
    Cell(FormKey, FormKey),
}
impl Action {
    pub fn is_authoring(&self) -> bool {
        !matches!(
            self,
            Self::Save | Self::SaveAs | Self::Open | Self::Recover | Self::SwitchCell(_)
        )
    }
}
#[cfg(test)]
mod lck_collision_save_r1_tests {
    use super::*;

    #[test]
    fn collision_walls_and_heights_survive_serialization(){
        let source="COLLISION_EDIT\t{\"version\":1,\"scene\":\"town01\",\"walls\":[[10,10,true],[12,13,false]],\"heights\":[[10,10,64]]}\n";

        let before=
            super::super::collision_edit::State::from_embedded_record(
                source,"town01"
            ).unwrap();

        assert!(before.has_edits());

        let encoded=before.embedded_record("town01");

        let after=
            super::super::collision_edit::State::from_embedded_record(
                &encoded,"town01"
            ).unwrap();

        assert_eq!(
            before.native_edit_snapshot(),
            after.native_edit_snapshot()
        );

        assert_eq!(
            after.native_edit_snapshot().1,
            vec![(10,10,64)]
        );
    }

    #[test]
    fn missing_collision_record_resets_to_clean_layer(){
        let layer=
            super::super::collision_edit::State::from_embedded_record(
                "LCK_SCENE_V1\ttown01\n",
                "town01"
            ).unwrap();

        assert!(!layer.has_edits());
    }

    #[test]
    fn collision_record_rejects_wrong_native_scene(){
        let source="COLLISION_EDIT\t{\"version\":1,\"scene\":\"town01\",\"walls\":[[10,10,true]],\"heights\":[]}\n";

        assert!(
            super::super::collision_edit::State::from_embedded_record(
                source,"town0b"
            ).is_err()
        );
    }
}
impl App {
    // LCK_COLLISION_SAVE_R1
    // A project save must never silently omit an
    // authored native collision wall or height.
    fn lck_collision_verify_project_snapshot(
        &self,
        project:&super::lck_project::LckProject
    )->Result<()> {
        if !self.collision_edit.has_edits() {
            return Ok(());
        }

        let source=match self.native_scene.as_ref(){
            Some((name,NativeSceneSlot::Ready(..)))=>name,
            _=>anyhow::bail!(
                "Collision edits cannot be saved without a loaded native cell"
            ),
        };

        let identity=self.editor_cell_active.as_ref()
            .map(|cell|cell.form_identity(source))
            .context(
                "Collision edits have no active cell identity"
            )?;

        let raw=project.cell_snapshots.get(&identity)
            .context(format!(
                "Collision edits were not captured for cell {identity}; project was not saved"
            ))?;

        let restored=
            super::collision_edit::State::from_embedded_record(
                raw,source
            ).map_err(anyhow::Error::msg)?;

        anyhow::ensure!(
            restored.native_edit_snapshot()
                ==self.collision_edit.native_edit_snapshot(),
            "Collision wall/height snapshot differs from the live editor; project was not saved"
        );

        Ok(())
    }
    // LCK_R625B_WARP_SAVE
    // Verify that project data matches the exact live warp
    // and trigger records and that native restoration succeeds.
    fn lck_r625b_verify_warp_snapshot(
        &self,
        project:&super::lck_project::LckProject
    )->Result<()> {
        let Some((scene,NativeSceneSlot::Ready(..)))=
            self.native_scene.as_ref()
        else{
            return Ok(());
        };

        let cell=self.editor_cell_active.as_ref()
            .context("Warp editor has no active project cell")?;

        let identity=cell.form_identity(scene);
        let expected=self.doors.embedded_record(scene);

        anyhow::ensure!(
            !expected.is_empty(),
            "Warp editor belongs to another cell"
        );

        let raw=project.cell_snapshots.get(&identity)
            .context(format!(
                "Warp/trigger snapshot missing for {identity}"
            ))?;

        let count=raw.lines()
            .filter(|line|line.starts_with("DOOR_EDIT\t"))
            .count();

        anyhow::ensure!(
            count==1,
            "Expected one DOOR_EDIT record, found {count}"
        );

        anyhow::ensure!(
            raw.contains(&expected),
            "Project warp records differ from live editor"
        );

        let lib=self.studio.library.as_ref()
            .context("Native IMG unavailable for verification")?;

        let mut restored=super::door_teleport::State::load(
            lib.index.as_ref(),scene
        ).map_err(anyhow::Error::msg)?;

        restored.restore(raw).map_err(anyhow::Error::msg)?;

        anyhow::ensure!(
            restored.embedded_record(scene)==expected,
            "Warp/trigger snapshot failed native round-trip"
        );

        Ok(())
    }
    fn save_unified_project(&mut self, save_as: bool) -> Result<()> {
        self.lck_v2082f_commit_pending_warps()?;
        self.save_active_editor_cell();

        let project=self.form_session.as_ref()
            .context("Open a source cell before saving")?
            .project.clone();

        self.lck_collision_verify_project_snapshot(
            &project
        )?;

        self.lck_r625b_verify_warp_snapshot(&project)?;

        let existing=if save_as {
            None
        } else {
            self.form_file.as_ref().map(|f|f.path.clone())
        };

        let path=if let Some(path)=existing {
            path
        } else {
            let Some(path)=rfd::FileDialog::new()
                .set_file_name("Legaia.lckproject")
                .add_filter("LCK project",&["lckproject"])
                .save_file()
            else {
                return Ok(());
            };
            path.with_extension("lckproject")
        };

        if self.form_file.as_ref().is_none_or(|f|f.path!=path) {
            self.form_file=Some(ProjectFile::new(
                project.clone(),path.clone()
            ));
        }

        // LCK_R625B_WARP_SAVE
        // End the mutable file borrow before calling
        // verification methods that borrow self immutably.
        let saved_path={
            let file=self.form_file.as_mut().unwrap();
            file.project=project;
            file.save()?;
            file.path.clone()
        };

        // LCK_R625B_WARP_SAVE
        // Verify bytes actually written to the project file.
        let warp_disk_bytes=std::fs::read(&saved_path)
            .context("Read saved project for warp verification")?;

        let warp_disk_project:super::lck_project::LckProject=
            serde_json::from_slice(&warp_disk_bytes)
                .context("Decode saved warp project")?;

        self.lck_r625b_verify_warp_snapshot(
            &warp_disk_project
        )?;
        // Validate the collision layer from bytes actually
        // written to the .lckproject file, not merely memory.
        if self.collision_edit.has_edits(){
            let written=std::fs::read(&saved_path)
                .context("Read saved project for collision verification")?;

            let disk_project:super::lck_project::LckProject=
                serde_json::from_slice(&written)
                    .context("Reparse saved collision project")?;

            self.lck_collision_verify_project_snapshot(
                &disk_project
            )?;
        }


        self.studio.status=Some(format!(
            "Saved LCK project: {}",path.display()
        ));
        Ok(())
    }

    fn restore_unified_project(&mut self) -> Result<()> {
        let Some((name,NativeSceneSlot::Ready(scene,world)))=
            self.native_scene.as_ref()
        else {
            return Ok(());
        };

        let name=name.clone();
        let scene=Arc::clone(scene);
        let world=Arc::clone(world);

        let identity=self.editor_cell_active.as_ref()
            .map(|cell|cell.form_identity(&name))
            .unwrap_or_else(||format!("original:{name}"));

        let snapshot=self.form_session.as_ref()
            .and_then(|session|
                session.project.cell_snapshots.get(&identity)
            ).cloned();

        // LCK_R625B_WARP_SAVE
        // Open Project must reconstruct the native warp state
        // and apply this cell's saved DOOR_EDIT overlay.
        let restored_doors={
            let lib=self.studio.library.as_ref()
                .context("Native disc unavailable for warp restore")?;

            let mut doors=super::door_teleport::State::load(
                lib.index.as_ref(),&name
            ).map_err(anyhow::Error::msg)?;

            if let Some(raw)=snapshot.as_deref(){
                doors.restore(raw).map_err(anyhow::Error::msg)?;
            }

            doors
        };
        let Some(raw)=snapshot else {
            self.ground_edits=super::ground_edit_ops::State::default();
            self.ground_edits.scene=name.clone();

            self.collision_edit=
                super::collision_edit::State::from_embedded_record(
                    "",&name
                ).map_err(anyhow::Error::msg)?;
            self.doors=restored_doors;
            return Ok(());
        };

        let lib=self.studio.library.as_ref()
            .context("Source IMG unavailable")?;

        let mut restored=(*world).clone();
        let overlay=super::ground_edit_ops::strip(&raw);
        let mut document=super::scene_instances::parse(
            &overlay,&name,&restored
        ).map_err(anyhow::Error::msg)?;
        if let Some(session)=self.form_session.as_ref(){
            session.r641_repair_projection_snapshot(
                &scene,&lib.index,&restored,&mut document
            )?;
        }

        super::scene_instances::apply(
            &document,&mut restored,
            scene.placements.len(),&lib.index
        ).map_err(anyhow::Error::msg)?;

        let ground=super::ground_edit_ops::parse(
            &raw,scene.ground.as_ref(),&name
        ).map_err(anyhow::Error::msg)?;

        // LCK_COLLISION_SAVE_R1
        // Restore the exact native wall and height
        // overrides serialized by Save Project.
        let collision=
            super::collision_edit::State::from_embedded_record(
                &raw,&name
            ).map_err(anyhow::Error::msg)?;
        if let Some((_,NativeSceneSlot::Ready(_,current)))=
            self.native_scene.as_mut()
        {
            *current=Arc::new(restored);
        }

        self.ground_edits=ground;
        self.collision_edit=collision;
        self.doors=restored_doors;
        self.native_view.version=
            self.native_view.version.wrapping_add(1);
        Ok(())
    }

    pub(super) fn capture_form_history(&self, history: &mut EditorSceneHistory) {
        if let Some(session) = &self.form_session {
            history.project = Some(Arc::new(session.project.clone()));
            history.form_selection = self
                .native_selection
                .iter()
                .filter_map(|k| {
                    self.native_scene.as_ref().and_then(|(_, s)| {
                        if let NativeSceneSlot::Ready(_, w) = s {
                            session.reference_at(w, *k)
                        } else {
                            None
                        }
                    })
                })
                .collect();
            history.form_primary = self.native_selected.and_then(|k| {
                self.native_scene.as_ref().and_then(|(_, s)| {
                    if let NativeSceneSlot::Ready(_, w) = s {
                        session.reference_at(w, k)
                    } else {
                        None
                    }
                })
            });
        }
    }
    pub(super) fn form_record_panel(&mut self, ctx: &egui::Context) -> Option<Action> {
        if !self.form_panel {
            return None;
        }
        let Some(session) = &self.form_session else {
            return None;
        };
        let mut action = None;
        let mut open = true;
        let selected = self
            .native_selected
            .and_then(|k| {
                self.native_scene.as_ref().and_then(|(_, s)| {
                    if let NativeSceneSlot::Ready(_, w) = s {
                        session.reference_at(w, k)
                    } else {
                        None
                    }
                })
            })
            .or_else(|| self.form_selected.clone());
        egui::Window::new("Base Forms and References").open(&mut open).default_width(420.).show(ctx,|ui|{
            ui.label(format!("Active file: {}",session.project.file_id.as_str()));
            egui::ComboBox::from_id_salt("active-project-cell").selected_text("Open project cell...").show_ui(ui,|ui|{for cell in &session.project.cells{if ui.selectable_label(session.cell.as_ref()==Some(&cell.key),&cell.label).clicked(){action=Some(Action::SwitchCell(cell.key.clone()));}}});
ui.small("Save Project preserves FormKeys, overrides, deleted records and shared definitions. Legacy scene files are compatibility exports.");
            ui.horizontal(|ui|{if !self.form_recovery.is_empty()&&ui.button("Previous project").clicked(){action=Some(Action::Recover);}});
            if let Some(reference)=selected.as_ref().and_then(|k|session.project.database.reference(k)){
                let base=session.project.database.resolve_base(&reference.base).unwrap();let order=[session.project.master.file.clone(),session.project.file_id.clone()];
                ui.separator();ui.heading("Placed Reference");ui.monospace(format!("{:08X}  {:?}",reference.key.display_id(&order).unwrap_or(0),reference.key));
                let mut ref_id=reference.editor_id.clone().unwrap_or_default();ui.horizontal(|ui|{ui.label("Reference EditorID");if ui.text_edit_singleline(&mut ref_id).lost_focus(){let value=if ref_id.trim().is_empty(){None}else{Some(ref_id.trim().into())};if value!=reference.editor_id{action=Some(Action::ReferenceId(reference.key.clone(),value));}}});
                let mut transform=reference.transform;let mut changed=false;
                for (label,values) in [("Position",&mut transform.position),("Rotation (radians)",&mut transform.rotation),("Scale XYZ",&mut transform.scale)]{ui.horizontal(|ui|{ui.label(label);for value in values{changed|=ui.add(egui::DragValue::new(value).speed(0.1)).changed();}});}
                if changed{action=Some(Action::Transform(reference.key.clone(),transform));}
                ui.horizontal(|ui|{if ui.button("Duplicate Reference").clicked(){action=Some(Action::DuplicateReference(reference.key.clone()));}if ui.button("Delete Reference").clicked(){action=Some(Action::DeleteReference(reference.key.clone()));}});
                let mut disabled=reference.disabled;if ui.checkbox(&mut disabled,"Disabled (separate from deletion)").changed(){action=Some(Action::Disable(reference.key.clone(),disabled));}if reference.deleted{ui.colored_label(egui::Color32::YELLOW,"Deleted in active project");if ui.button("Restore Reference").clicked(){action=Some(Action::Restore(reference.key.clone()));}}
                egui::ComboBox::from_id_salt("reference-base-form").selected_text(&base.editor_id).show_ui(ui,|ui|{for(key,b)in session.project.database.bases().into_iter().filter(|(_,b)|b.kind.placeable()){if ui.selectable_label(key==reference.base,&b.editor_id).clicked(){action=Some(Action::AssignBase(reference.key.clone(),key));}}});
                let current=session.project.cells.iter().find(|c|c.key==reference.cell).map(|c|c.label.as_str()).unwrap_or("Unknown cell");egui::ComboBox::from_id_salt("reference-cell-form").selected_text(format!("Cell: {current}")).show_ui(ui,|ui|{for cell in &session.project.cells{if ui.selectable_label(cell.key==reference.cell,&cell.label).clicked(){action=Some(Action::Cell(reference.key.clone(),cell.key.clone()));}}});
                if reference.flags.contains("native_animated_reference") {ui.small("Animated Reference: native field-actor route; donor dialogue, story and motion scripts detached. Export reports unsupported dependencies.");}
                ui.label(format!("Dependencies: {}",base.dependency_classification().label()));
                egui::CollapsingHeader::new("Copy capabilities and native dependencies").show(ui,|ui|{
                    let report=base.copy_assessment();
                    ui.label(format!("Static visual copy: {}",report.static_visual));
                    ui.label(format!("Animated copy: {}",report.animated));
                    ui.label(format!("Behavior-preserving copy: {}",report.behavior_preserving));
                    for gap in &report.unresolved {ui.small(gap);}
                    if let Some(deps)=&base.native_behavior {for edge in &deps.edges {ui.small(format!("{:?}: {} ({} +{:#x})",edge.role,edge.target,edge.evidence.source,edge.evidence.offset));}}
                });
                ui.heading("Shared Base Form");ui.monospace(format!("{:08X}",base.key.display_id(&order).unwrap_or(0)));let info=session.project.database.use_info(&base.key);ui.label(format!("Count: {}   Known Users: {}{}",info.count,info.users.len(),if info.dependency_coverage_complete{""}else{" (dependency coverage incomplete)"}));
                if self.form_editor_target.as_ref()!=Some(&base.key){self.form_editor_target=Some(base.key.clone());self.form_editor_id=base.editor_id.clone();}
                ui.horizontal(|ui|{ui.label("Base EditorID");ui.text_edit_singleline(&mut self.form_editor_id);if ui.button("Apply shared ID").clicked(){action=Some(Action::BaseId(base.key.clone(),self.form_editor_id.clone()));}});
                if ui.button("Duplicate Base Form (definition only)").clicked(){action=Some(Action::DuplicateBase(base.key.clone()));}
                egui::ComboBox::from_id_salt("shared-base-geometry").selected_text("Assign shared geometry/material source...").show_ui(ui,|ui|{for(key,b)in session.project.database.bases().into_iter().filter(|(k,_)|session.project.base_adapters.contains_key(k)){if ui.selectable_label(false,&b.editor_id).clicked(){action=Some(Action::Geometry(base.key.clone(),key));}}});
                let mut tint=match base.materials.get("preview_tint"){Some(Property::Vector(v))=>*v,_=>[1.;3]};let mut changed=false;ui.horizontal(|ui|{ui.label("Shared preview tint");for value in &mut tint{changed|=ui.add(egui::DragValue::new(value).speed(0.01)).changed();}});if changed{action=Some(Action::Tint(base.key.clone(),tint));}ui.small("Shared material edits refresh all References. Native material override compilation is reported separately.");
            }
            ui.separator();ui.heading("Object Window — Base Forms");ui.small("Drag a placeable definition into the viewport to create a Reference. Duplicate Base creates no placement.");
            egui::ScrollArea::vertical().id_salt("project-base-forms").max_height(180.).show(ui,|ui|{for(key,base)in session.project.database.bases(){let text=format!("{}  [{:?}] - {}",base.editor_id,base.kind,base.dependency_classification().label());let response=ui.add(egui::Label::new(text).sense(if base.kind.placeable(){egui::Sense::click_and_drag()}else{egui::Sense::click()}));if response.drag_started()&&base.kind.placeable(){self.form_drag=Some(key);} }});
            egui::CollapsingHeader::new("Reference records in this cell (including deleted)").show(ui,|ui|{for(key,r)in session.project.database.references().into_iter().filter(|(_,r)|Some(&r.cell)==session.cell.as_ref()){let name=match r.properties.get("display_name"){Some(Property::Text(name))=>name.as_str(),_=>r.editor_id.as_deref().unwrap_or("Reference")};if ui.selectable_label(selected.as_ref()==Some(&key),format!("{} {:06X}{}{}",name,key.local_id,if r.deleted{" [deleted]"}else{""},if r.disabled{" [disabled]"}else{""})).clicked(){self.form_selected=Some(key.clone());self.native_selected=session.reference_draws.get(&key).copied();self.native_selection=self.native_selected.into_iter().collect();}}});
            if !session.project.diagnostics.is_empty(){egui::CollapsingHeader::new(format!("Dependency diagnostics ({})",session.project.diagnostics.len())).show(ui,|ui|{for d in &session.project.diagnostics{ui.colored_label(egui::Color32::YELLOW,&d.message);}});}
        });
        self.form_panel = open;
        action
    }
    pub(super) fn rebuild_form_preview(&mut self) {
        let result = (|| -> Result<_> {
            let session = self
                .form_session
                .as_mut()
                .context("Active project missing")?;
            let (_, NativeSceneSlot::Ready(scene, _)) =
                self.native_scene.as_ref().context("Active scene missing")?
            else {
                anyhow::bail!("Scene loading")
            };
            let index = &self
                .studio
                .library
                .as_ref()
                .context("Master library missing")?
                .index;
            session.render(scene, index)
        })();
        match result {
            Ok(world) => {
                if let Some((_, NativeSceneSlot::Ready(_, w))) = &mut self.native_scene {
                    *w = Arc::new(world);
                }
                self.native_view.version = self.native_view.version.wrapping_add(1);
            }
            Err(e) => self.studio.error = Some(format!("Project preview incomplete: {e:#}")),
        }
    }
    pub(super) fn apply_form_action(&mut self, action: Action) -> Result<()> {
        match action {
            Action::SwitchCell(key) => {
                let cell = self
                    .form_session
                    .as_ref()
                    .context("Project missing")?
                    .project
                    .cells
                    .iter()
                    .find(|c| c.key == key)
                    .context("Cell missing")?
                    .clone();
                let selection = if let Some(id) = cell.legacy_copy_id {
                    super::cell_catalog::CellSelection::Copy(id)
                } else {
                    super::cell_catalog::CellSelection::Original(cell.source_scene.clone())
                };
                self.r642_pair=Default::default();
                self.r642_toggle_requested=false;
                self.select_editor_cell(selection);
                return Ok(());
            }
            Action::Save => return self.save_unified_project(false),
            Action::SaveAs => return self.save_unified_project(true),
            Action::Open => {
                let Some(path) = rfd::FileDialog::new()
                    .add_filter("LCK project", &["lckproject"])
                    .pick_file()
                else {
                    return Ok(());
                };
                let hash = self
                    .form_session
                    .as_ref()
                    .map(|s| s.source_fingerprint.as_str());
                let file = ProjectFile::open(&path, hash)?;
                let lib = self
                    .studio
                    .library
                    .as_ref()
                    .context("Open the source disc for preview; project file remains intact")?;
                let mut session = super::form_bridge::Session::new(lib.source.path(), &lib.index)?;
                session.mount_project(file.project.clone())?;
                if let Some((_, NativeSceneSlot::Ready(scene, _))) = &self.native_scene {
                    let native = super::world_view::build_world(scene, &lib.index);
                    let binding = session
                        .project
                        .cells
                        .iter()
                        .find(|c| c.source_scene == scene.name && c.legacy_copy_id.is_none())
                        .cloned();
                    let (identity, label, copy) = binding
                        .map(|c| (c.identity, c.label, c.legacy_copy_id))
                        .unwrap_or((format!("original:{}", scene.name), scene.name.clone(), None));
                    session.enter(scene, &lib.index, &native, identity, label, copy)?;
                }
                if let Some(old) = self.form_session.take() {
                    self.form_recovery.push((old, self.form_file.take()));
                }
                self.form_session = Some(session);
                self.form_file = Some(file);
                self.editor_undo.clear();
                self.editor_redo.clear();
                self.editor_gesture_start = None;
                self.form_selected = None;
                self.native_selection.clear();
                self.native_selected = None;
                self.rebuild_form_preview();
                self.restore_unified_project()?;
                self.r642_pair=Default::default();
                self.r642_toggle_requested=false;
                self.studio.status = Some(format!("Opened LCK project {}", path.display()));
                return Ok(());
            }
            Action::Recover => {
                if let Some((previous, _)) = self.form_recovery.last() {
                    let current = self
                        .form_session
                        .as_ref()
                        .context("Open the previous project's source disc first")?;
                    anyhow::ensure!(current.source_fingerprint==previous.source_fingerprint,
                        "Previous project retained: open its matching source IMG before restoring it");
                }
                if let Some((session, file)) = self.form_recovery.pop() {
                    if let Some(current) = self.form_session.replace(session) {
                        self.form_recovery.push((current, self.form_file.take()));
                    }
                    self.form_file = file;
                    self.r642_pair=Default::default();
                    self.r642_toggle_requested=false;
                    self.editor_undo.clear();
                    self.editor_redo.clear();
                    self.rebuild_form_preview();
                }
                return Ok(());
            }
            _ => {}
        }
        let current = self
            .form_session
            .as_ref()
            .context("Active project missing")?;
        let mut draft = current.clone();
        let mut selected = self
            .native_selected
            .and_then(|key| {
                self.native_scene.as_ref().and_then(|(_, slot)| {
                    if let NativeSceneSlot::Ready(_, world) = slot {
                        current.reference_at(world, key)
                    } else {
                        None
                    }
                })
            })
            .or_else(|| self.form_selected.clone());
        match action {
            Action::Transform(key, transform) => {
                draft.transaction(|p| p.database.move_reference(&key, transform))?
            }
            Action::DuplicateReference(key) => selected = Some(draft.duplicate(&[key])?[0].clone()),
            Action::DeleteReference(key) => draft.delete(&[key])?,
            Action::Place(base, center) => {
                selected = Some(super::placement_drop::place_form(&mut draft, base, center)?)
            }
            Action::DuplicateBase(base) => {
                let key = draft.duplicate_base(&base)?;
                self.studio.status = Some(format!(
                    "Created independent Base definition {:?}; drag it from Object Window to place",
                    key
                ));
            }
            Action::BaseId(base, id) => draft.transaction(|p| {
                p.database.edit_base(&base, |b| {
                    b.editor_id = id;
                    Ok(())
                })
            })?,
            Action::ReferenceId(key, id) => draft.transaction(|p| {
                p.database.edit_reference(&key, |r| {
                    r.editor_id = id;
                    Ok(())
                })
            })?,
            Action::AssignBase(r, b) => draft.assign_base(&r, b)?,
            Action::Geometry(base, source) => draft.assign_geometry(&base, &source)?,
            Action::Tint(base, tint) => draft.tint_base(&base, tint)?,
            Action::Disable(key, value) => draft.disable(&key, value)?,
            Action::Restore(key) => draft.transaction(|p| p.database.restore_reference(&key))?,
            Action::Cell(key, cell) => {
                draft.transaction(|p| p.database.change_reference_cell(&key, cell))?
            }
            _ => {}
        }
        let (_, NativeSceneSlot::Ready(scene, _)) =
            self.native_scene.as_ref().context("Scene missing")?
        else {
            anyhow::bail!("Scene not ready")
        };
        let index = &self
            .studio
            .library
            .as_ref()
            .context("Library missing")?
            .index;
        let world = draft.render(scene, index)?;
        draft.synchronize_gesture(scene, index, &world)?;
        draft.project.validate()?;
        self.form_session = Some(draft);
        if let Some((_, NativeSceneSlot::Ready(_, w))) = &mut self.native_scene {
            *w = Arc::new(world);
        }
        self.form_selected = selected;
        if let Some(r) = &self.form_selected {
            self.native_selected = self
                .form_session
                .as_ref()
                .unwrap()
                .reference_draws
                .get(r)
                .copied();
            self.native_selection = self.native_selected.into_iter().collect();
        }
        self.native_view.version = self.native_view.version.wrapping_add(1);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "real native editor workspace command routing, entirely in-memory egui; no window or emulator"]
    fn record_workspace_duplicate_undo_redo_restores_formkeys_and_collision_associations(
    ) -> Result<()> {
        let source = std::path::Path::new(
            r"D:\AstraRimElmProject\ManualTests\RuntimeRecovery_20261030_01\R8_FULL_SCENE_OWNED_COLLISION_RECOVERY.img",
        );
        let lib = Arc::new(crate::library::Library::open(source)?);
        let scene =
            legaia_engine_world_views::scene_staging::assemble_field_scene(&lib.index, "town01")
                .map_err(anyhow::Error::msg)?;
        let mut native = super::super::world_view::build_world(&scene, &lib.index);
        super::super::native_wall_export::attach_preview(source, &scene, &mut native, &lib.index);
        let mut session = super::super::form_bridge::Session::temporary(source, &lib.index)?;
        let preview = session.enter(
            &scene,
            &lib.index,
            &native,
            "original:town01".into(),
            "Rim Elm".into(),
            None,
        )?;
        let originals =
            [(false, 13), (false, 14)].map(|key| session.reference_at(&preview, key).unwrap());
        let mut app = App::new(
            crate::Studio::new(None),
            None,
            super::super::Automation::default(),
        );
        app.studio.library = Some(lib);
        app.global_repository_started = true;
        app.native_scene = Some((
            "town01".into(),
            NativeSceneSlot::Ready(Arc::new(scene), Arc::new(preview)),
        ));
        app.form_session = Some(session);
        app.native_selected = Some((false, 13));
        app.native_selection.extend([(false, 13), (false, 14)]);
        let ctx = egui::Context::default();
        let run = |app: &mut App, key: Option<egui::Key>| {
            let events = key
                .map(|key| {
                    vec![egui::Event::Key {
                        key,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: egui::Modifiers {
                            ctrl: true,
                            command: true,
                            ..Default::default()
                        },
                    }]
                })
                .unwrap_or_default();
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1280., 820.),
                    )),
                    events,
                    ..Default::default()
                },
                |ctx| app.editor_workspace(ctx),
            );
        };
        run(&mut app, None);
        let camera = app.native_view.cam;
        run(&mut app, Some(egui::Key::D));
        let session = app.form_session.as_ref().unwrap();
        let created = session
            .project
            .database
            .references()
            .keys()
            .filter(|key| key.file == session.project.file_id)
            .cloned()
            .collect::<Vec<_>>();
        anyhow::ensure!(
            created.len() == 2,
            "Ctrl+D did not create exactly two Reference records: {:?}",
            app.studio.error
        );
        anyhow::ensure!(
            app.editor_undo.len() == 1,
            "Group duplicate not one complete undo transaction"
        );
        let associations = session.project.collision_associations.clone();
        anyhow::ensure!(
            associations.len() == 2,
            "Generated independent collision assignments not captured in project"
        );
        for key in &created {
            anyhow::ensure!(
                originals
                    .iter()
                    .any(|old| session.project.database.reference(old).unwrap().base
                        == session.project.database.reference(key).unwrap().base),
                "Duplicate copied a Base definition"
            );
        }
        run(&mut app, Some(egui::Key::Z));
        anyhow::ensure!(
            created.iter().all(|k| app
                .form_session
                .as_ref()
                .unwrap()
                .project
                .database
                .reference(k)
                .is_none()),
            "Undo left new Reference records"
        );
        run(&mut app, Some(egui::Key::Y));
        let session = app.form_session.as_ref().unwrap();
        anyhow::ensure!(
            created
                .iter()
                .all(|k| session.project.database.reference(k).is_some()),
            "Redo reallocated/lost FormKeys"
        );
        anyhow::ensure!(
            session.project.collision_associations == associations,
            "Redo did not restore collision owner mappings"
        );
        anyhow::ensure!(
            app.native_view.cam == camera,
            "Record commands changed camera"
        );
        Ok(())
    }
}
