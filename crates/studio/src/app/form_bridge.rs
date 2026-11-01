//! Form database -> resolved references -> the existing CPU preview renderer.
//! Only this compatibility boundary interprets legacy draw keys/editor UIDs.
use super::{
    collision_recipes::{self, digest},
    form_database::*,
    form_legacy,
    lck_project::*,
    scene_instances,
    world_view::{self, WorldModel},
};
use anyhow::{ensure, Context, Result};
use legaia_engine_scene::{scene::ProtIndex, scene_assembly::AssembledScene};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    path::{Path, PathBuf},
};
pub(super) type DrawKey = (bool, usize);
#[derive(Clone)]
pub(super) struct Session {
    pub project: LckProject,
    pub cell: Option<FormKey>,
    pub reference_draws: BTreeMap<FormKey, DrawKey>,
    pub projected_transforms: BTreeMap<FormKey, Transform>,
    pub source: PathBuf,
    pub source_fingerprint: String,
    master_store: Option<super::form_master_store::MasterStore>,
    native_worlds: BTreeMap<String, std::sync::Arc<WorldModel>>,
    donor_scenes: HashMap<String, std::sync::Arc<AssembledScene>>,
}
impl Session {
    pub fn new(source: &Path, index: &ProtIndex) -> Result<Self> {
        Self::create(source, index, true)
    }
    #[cfg(test)]
    pub fn temporary(source: &Path, index: &ProtIndex) -> Result<Self> {
        Self::create(source, index, false)
    }
    fn create(source: &Path, index: &ProtIndex, shared: bool) -> Result<Self> {
        let fingerprint = if source.is_file() {
            use sha2::{Digest, Sha256};
            use std::io::Read;
            let mut file = std::fs::File::open(source)?;
            let mut hash = Sha256::new();
            let mut bytes = [0u8; 65536];
            loop {
                let n = file.read(&mut bytes)?;
                if n == 0 {
                    break;
                }
                hash.update(&bytes[..n]);
            }
            format!("{:x}", hash.finalize())
        } else {
            digest(&index.entry_bytes(0)?)
        };
        let mut master_store = if shared {
            Some(super::form_master_store::MasterStore::open(
                &super::form_master_store::MasterStore::default_root()?,
                source,
                &fingerprint,
            )?)
        } else {
            None
        };
        let master = if let Some(store) = &master_store {
            store.master().clone()
        } else {
            MasterDependency {
                file: FileId::new(format!("master-{}", new_project_id()?.as_str()))?,
                disc_sha256: fingerprint.clone(),
                path_hint: Some(source.into()),
            }
        };
        let mut project = LckProject::new(master, new_project_id()?)?;
        if let Some(store) = &mut master_store {
            store.refresh(&mut project.database)?;
        }
        project.check_master(Some(&fingerprint));
        Ok(Self {
            project,
            cell: None,
            reference_draws: Default::default(),
            projected_transforms: Default::default(),
            source: source.into(),
            source_fingerprint: fingerprint,
            master_store,
            native_worlds: Default::default(),
            donor_scenes: Default::default(),
        })
    }
    pub fn mount_project(&mut self, mut project: LckProject) -> Result<()> {
        if self.master_store.is_some() {
            let mut store = super::form_master_store::MasterStore::mount_project(
                &super::form_master_store::MasterStore::default_root()?,
                &project.master,
                &project.database,
            )?;
            store.refresh(&mut project.database)?;
            self.master_store = Some(store);
        }
        project.check_master(Some(&self.source_fingerprint));
        project.validate()?;
        self.project = project;
        self.native_worlds.clear();
        self.donor_scenes.clear();
        self.reference_draws.clear();
        self.projected_transforms.clear();
        Ok(())
    }
    pub fn migrate_cell_document(
        &mut self,
        scene: &AssembledScene,
        index: &ProtIndex,
        text: &str,
        identity: String,
        label: String,
        copy: Option<u64>,
    ) -> Result<FormKey> {
        let mut draft = self.clone();
        if let Some(store) = &mut draft.master_store {
            store.refresh(&mut draft.project.database)?;
        }
        let original = world_view::build_world(scene, index);
        let cell = scene_instances::migrate_into_project(
            &mut draft.project,
            scene,
            index,
            &original,
            text,
            identity,
            label,
            copy,
        )?;
        draft.project.validate()?;
        if let Some(store) = &mut draft.master_store {
            store.commit(&draft.project.database)?;
        }
        *self = draft;
        Ok(cell)
    }
    pub fn enter(
        &mut self,
        scene: &AssembledScene,
        index: &ProtIndex,
        legacy: &WorldModel,
        identity: String,
        label: String,
        copy_id: Option<u64>,
    ) -> Result<WorldModel> {
        let mut draft = self.clone();
        if let Some(store) = &mut draft.master_store {
            store.refresh(&mut draft.project.database)?;
        }
        let cell = if let Some(c) = draft
            .project
            .cells
            .iter()
            .find(|c| c.identity == identity && c.resolved)
        {
            c.key.clone()
        } else {
            let text = scene_instances::serialize(&scene.name, legacy);
            let input = form_legacy::scene_input(
                scene,
                index,
                legacy,
                identity,
                label,
                copy_id,
                text,
                &draft.project.master.file,
            )?;
            draft.project.migrate_scene(&input)?
        };
        draft.cell = Some(cell);
        let sidecar = draft.source.with_extension("lck-collision.json");
        if sidecar.exists() {
            draft
                .project
                .retain_sidecar(&sidecar, std::fs::read(&sidecar)?)?;
        }
        let rendered = draft.render(scene, index)?;
        draft.project.validate()?;
        if let Some(store) = &mut draft.master_store {
            store.commit(&draft.project.database)?;
        }
        *self = draft;
        Ok(rendered)
    }
    pub fn static_base_evidence(&self,index:&ProtIndex)->BTreeMap<FormKey,serde_json::Value> {
        let mut contexts=BTreeMap::new();let mut result=BTreeMap::new();
        let Some(cell)=&self.cell else{return result;};
        let bases=self.project.database.references().values().filter(|r|&r.cell==cell&&!r.deleted&&!r.disabled).map(|r|r.base.clone()).collect::<BTreeSet<_>>();
        for key in bases {let evidence=(||->Result<_>{let mut base=self.project.database.resolve_base(&key).context("Base Form missing")?.clone();let adapter=self.project.base_adapters.get(&key).context("Native registration adapter unresolved")?;
            ensure!(matches!(adapter.kind.as_str(),"terrain"|"placement"|"mesh"),"Static Base Form selector adapter unresolved for {}",adapter.kind);
            let scene=legaia_engine_scene::scene::Scene::load(index,&adapter.source_scene)?;
            let origin=adapter.origin.as_ref().context("Original record provenance unresolved")?;let entry=origin.prot_entry.context("Native MAP identity unresolved")?;
            ensure!(scene.field_map_index(index)==Some(entry),"Base Form native MAP provenance changed");
            let selector=if adapter.kind=="mesh" {
                // Reusable TMD resources have no native MAP descriptor.
                u16::try_from(adapter.source_ordinal)?
            }else{
                let descriptor=origin.descriptor.context("Native descriptor identity unresolved")?;
                let bytes=index.entry_bytes(entry)?;
                legaia_scene_formats::field_objects::ObjectRecord::parse(&bytes,descriptor as usize)
                    .context("Native descriptor missing")?.pack_index_field
            };
            if !contexts.contains_key(&adapter.source_scene){contexts.insert(adapter.source_scene.clone(),super::native_asset_plan::static_resources::resolve_context(index,&scene)?);}
            base.resolve_static_resources(&contexts[&adapter.source_scene],&scene,selector)?;
            Ok(serde_json::json!({"base":key,"static_resources":base.static_resources,"behavior_verified":false}))})();
            result.insert(key.clone(),match evidence {Ok(v)=>v,Err(e)=>serde_json::json!({"base":key,"unresolved":format!("{e:#}"),"behavior_verified":false})});
        }result
    }
    pub fn reference_at(&self, world: &WorldModel, key: DrawKey) -> Option<FormKey> {
        let draw = world.at(key.0, key.1)?;
        if let Some(uid) = draw.editor_uid {
            let cell = self.cell.as_ref()?;
            self.project
                .render_uids
                .iter()
                .find(|b| &b.cell == cell && b.uid == uid)
                .map(|b| b.reference.clone())
                .or_else(|| {
                    self.project
                        .legacy_bindings
                        .iter()
                        .find(|b| &b.cell == cell && b.uid == Some(uid))
                        .map(|b| b.reference.clone())
                })
        } else {
            self.reference_draws
                .iter()
                .find(|(_, v)| **v == key)
                .map(|(k, _)| k.clone())
        }
    }
    pub fn transaction<T>(&mut self, edit: impl FnOnce(&mut LckProject) -> Result<T>) -> Result<T> {
        let mut draft = self.project.clone();
        let value = edit(&mut draft)?;
        draft.validate()?;
        self.project = draft;
        Ok(value)
    }
    pub fn duplicate(&mut self, keys: &[FormKey]) -> Result<Vec<FormKey>> {
        self.transaction(|p| {
            let mut created = vec![];
            for key in keys {
                let new = p.database.duplicate_reference(key)?;
                let associations = p
                    .collision_associations
                    .iter()
                    .filter(|a| &a.reference == key)
                    .cloned()
                    .collect::<Vec<_>>();
                for mut a in associations {
                    a.reference = new.clone();
                    p.collision_associations.push(a);
                }
                created.push(new);
            }
            Ok(created)
        })
    }
    pub fn place(&mut self, base: FormKey, transform: Transform) -> Result<FormKey> {
        let cell = self.cell.clone().context("Select a cell")?;
        self.transaction(|p| {
            let key = p.database.place_reference(base.clone(), cell, transform)?;
            if p.database
                .resolve_base(&base)
                .is_some_and(|b| b.kind == FormKind::StaticEnvironment)
            {
                p.database.edit_reference(&key, |r| {
                    r.flags.insert("static_architecture".into());
                    Ok(())
                })?;
            }
            Ok(key)
        })
    }
    pub fn delete(&mut self, keys: &[FormKey]) -> Result<()> {
        self.transaction(|p| {
            for key in keys {
                p.database.delete_reference(key)?;
            }
            Ok(())
        })
    }
    pub fn assign_base(&mut self, reference: &FormKey, base: FormKey) -> Result<()> {
        self.transaction(|p| p.database.change_reference_base(reference, base))
    }
    pub fn duplicate_base(&mut self, key: &FormKey) -> Result<FormKey> {
        self.transaction(|p| {
            let b = p.database.resolve_base(key).context("Base missing")?;
            let id = format!("{}_Copy_{:X}", b.editor_id, p.database.bases().len());
            let new = p.database.duplicate_base(key, id)?;
            if let Some(a) = p.base_adapters.get(key).cloned() {
                p.base_adapters.insert(new.clone(), a);
            }
            Ok(new)
        })
    }
    pub fn assign_geometry(&mut self, key: &FormKey, source: &FormKey) -> Result<()> {
        self.transaction(|p| {
            let definition = p
                .database
                .resolve_base(source)
                .context("Geometry definition absent")?
                .clone();
            let adapter = p
                .base_adapters
                .get(source)
                .context("Geometry preview adapter unresolved")?
                .clone();
            p.database.edit_base(key, |b| {
                b.resources = definition.resources;
                b.materials = definition.materials;
                Ok(())
            })?;
            p.base_adapters.insert(key.clone(), adapter);
            Ok(())
        })
    }
    pub fn tint_base(&mut self, key: &FormKey, tint: [f32; 3]) -> Result<()> {
        self.transaction(|p| {
            p.database.edit_base(key, |b| {
                b.materials
                    .insert("preview_tint".into(), Property::Vector(tint));
                Ok(())
            })
        })
    }
    pub fn disable(&mut self, key: &FormKey, disabled: bool) -> Result<()> {
        self.transaction(|p| {
            p.database.edit_reference(key, |r| {
                r.disabled = disabled;
                Ok(())
            })
        })
    }
    pub fn restore_project(&mut self, mut previous: LckProject) -> Result<()> {
        previous
            .database
            .retain_identity_inventory(&self.project.database)?;
        // Cell/resource discovery is immutable inventory, not an authoring edit.
        for cell in &self.project.cells {
            if !previous.cells.iter().any(|c| c.key == cell.key) {
                previous
                    .database
                    .retain_context_cell(&cell.key, &self.project.database)?;
                previous.cells.push(cell.clone());
            }
        }
        for binding in &self.project.legacy_bindings {
            if (binding.reference.file == previous.master.file || binding.uid.is_none())
                && !previous
                    .legacy_bindings
                    .iter()
                    .any(|b| b.cell == binding.cell && b.source_identity == binding.source_identity)
            {
                if binding.reference.file != previous.master.file {
                    previous
                        .database
                        .retain_context_reference(&binding.reference, &self.project.database)?;
                }
                previous.legacy_bindings.push(binding.clone());
            }
        }
        for (key, adapter) in &self.project.base_adapters {
            if previous.database.resolve_base(key).is_some() {
                previous
                    .base_adapters
                    .entry(key.clone())
                    .or_insert_with(|| adapter.clone());
            }
        }
        for (cell, next) in &self.project.next_render_uid {
            let value = previous
                .next_render_uid
                .entry(cell.clone())
                .or_insert(*next);
            *value = (*value).max(*next);
        }
        previous.validate()?;
        self.project = previous;
        Ok(())
    }
    /// Accept a gesture's preview values as a validated record transaction.
    /// Existing keys come ONLY from the reference/draw mapping, never new hashes.
    pub fn synchronize_gesture(
        &mut self,
        scene: &AssembledScene,
        index: &ProtIndex,
        world: &WorldModel,
    ) -> Result<()> {
        let cell = self.cell.clone().context("Active Cell missing")?;
        let mut draft = self.clone();
        let mut observed = BTreeSet::new();
        let mut map = BTreeMap::new();
        let new_inputs = if world.draws.iter().any(|d| {
            d.editor_uid.is_some() && self.reference_at(world, (d.terrain, d.index)).is_none()
        }) {
            Some(form_legacy::scene_input(
                scene,
                index,
                world,
                "temporary-input".into(),
                scene.name.clone(),
                None,
                String::new(),
                &draft.project.master.file,
            )?)
        } else {
            None
        };
        if new_inputs.is_some() {
            if let Some(store) = &mut draft.master_store {
                store.refresh(&mut draft.project.database)?;
            }
        }
        for draw in &world.draws {
            let key = (draw.terrain, draw.index);
            let reference = if let Some(r) = self.reference_at(world, key) {
                r
            } else if let Some(uid) = draw.editor_uid {
                let input = new_inputs
                    .as_ref()
                    .and_then(|i| i.objects.iter().find(|o| o.uid == Some(uid)))
                    .context("New reference source not resolved")?;
                let base = draft.project.database.import_master_base(
                    format!("base:{}", input.base_identity),
                    input.base.clone(),
                )?;
                draft
                    .project
                    .base_adapters
                    .entry(base.clone())
                    .or_insert_with(|| input.adapter.clone());
                let r =
                    draft
                        .project
                        .database
                        .place_reference(base, cell.clone(), input.transform)?;
                draft.project.database.edit_reference(&r, |r| {
                    r.properties = input.properties.clone();
                    r.flags = input.flags.clone();
                    Ok(())
                })?;
                draft.project.legacy_bindings.push(LegacyBinding {
                    cell: cell.clone(),
                    source_identity: input.source_identity.clone(),
                    uid: Some(uid),
                    reference: r.clone(),
                });
                r
            } else {
                continue;
            };
            let old = draft
                .project
                .database
                .reference(&reference)
                .context("Mapped Reference missing")?
                .clone();
            if old.cell != cell {
                continue;
            }
            draft.project.database.edit_reference(&reference, |r| {
                let candidate = Transform {
                    position: draw.center,
                    rotation: draw.rotation,
                    scale: draw.scale,
                };
                if self.projected_transforms.get(&reference) != Some(&candidate) {
                    r.transform = candidate;
                }
                if !r.disabled {
                    r.deleted = draw.hidden;
                }
                if let Some(name) = &draw.display_name {
                    r.properties
                        .insert("display_name".into(), Property::Text(name.clone()));
                } else {
                    r.properties.remove("display_name");
                }
                if let Some(f) = draw
                    .editor_uid
                    .and_then(|uid| world.foreign.iter().find(|f| f.uid == uid))
                {
                    if f.static_architecture {
                        r.flags.insert("static_architecture".into());
                    } else {
                        r.flags.remove("static_architecture");
                    }
                }
                if let Some(n) = world.npc(key) {
                    r.properties.insert(
                        "npc_model_selection".into(),
                        Property::Integer(n.model_index as i64),
                    );
                    r.properties.insert(
                        "npc_animation_clip".into(),
                        Property::Integer(n.clip as i64),
                    );
                    r.properties.insert(
                        "npc_animation_frame".into(),
                        Property::Integer(n.frame as i64),
                    );
                }
                Ok(())
            })?;
            let preserve_old = draft.project.diagnostics.iter().any(|d| {
                d.form.as_ref() == Some(&reference) && d.message.starts_with("DEPENDENCY:")
            });
            if !preserve_old {
                draft
                    .project
                    .collision_associations
                    .retain(|a| a.reference != reference);
                if let Some(owner) = world.collision.owner(world, key) {
                    for assignment in world.collision.recipes.iter().filter(|a| a.owner == owner) {
                        draft
                            .project
                            .collision_associations
                            .push(CollisionAssociation {
                                reference: reference.clone(),
                                family: "legacy-authored-collision".into(),
                                payload: serde_json::to_value(assignment)?,
                            });
                    }
                }
                if let Some(recipe) = draw
                    .editor_uid
                    .and_then(|uid| world.collision.mesh_recipes.get(&uid))
                {
                    draft
                        .project
                        .collision_associations
                        .push(CollisionAssociation {
                            reference: reference.clone(),
                            family: "legacy-generated-mesh-collision".into(),
                            payload: serde_json::to_value(recipe)?,
                        });
                }
            }
            observed.insert(reference.clone());
            map.insert(reference, key);
        }
        for key in self
            .reference_draws
            .keys()
            .filter(|k| !observed.contains(*k))
        {
            if draft
                .project
                .database
                .reference(key)
                .is_some_and(|r| r.cell == cell && !r.disabled)
            {
                draft.project.database.delete_reference(key)?;
            }
        }
        draft.project.validate()?;
        if new_inputs.is_some() {
            if let Some(store) = &mut draft.master_store {
                store.commit(&draft.project.database)?;
            }
        }
        draft.projected_transforms = map
            .iter()
            .filter_map(|(r, k)| {
                world.at(k.0, k.1).map(|d| {
                    (
                        r.clone(),
                        Transform {
                            position: d.center,
                            rotation: d.rotation,
                            scale: d.scale,
                        },
                    )
                })
            })
            .collect();
        draft.reference_draws = map;
        *self = draft;
        Ok(())
    }
    pub fn render(&mut self, scene: &AssembledScene, index: &ProtIndex) -> Result<WorldModel> {
        let cell = self.cell.clone().context("Active Cell missing")?;
        let cache_key =
            digest(format!("{}:{:?}:{:?}", scene.name, scene.placements, scene.terrain).as_bytes());
        let mut world = if let Some(native) = self.native_worlds.get(&cache_key) {
            (**native).clone()
        } else {
            let mut native = world_view::build_world(scene, index);
            let metadata = self.preserved_collision_metadata()?;
            super::native_wall_export::attach_preview_with_metadata(
                &self.source,
                scene,
                &mut native,
                index,
                metadata.as_deref(),
            );
            if self.native_worlds.len() >= 2 {
                self.native_worlds.clear();
            }
            self.native_worlds
                .insert(cache_key, std::sync::Arc::new(native.clone()));
            native
        };
        let references = self
            .project
            .database
            .references()
            .into_values()
            .cloned()
            .collect::<Vec<_>>();
        let mut mappings = BTreeMap::new();
        let mut donors = self.donor_scenes.clone();
        let mut diagnostics = vec![];
        // A native record is a rendering allocation only in its registered
        // cell context. Moving a Reference to another cell retains provenance,
        // but never aliases that destination's original native draw.
        let context = self
            .project
            .legacy_bindings
            .iter()
            .filter(|b| b.cell == cell && b.uid.is_none())
            .map(|b| b.reference.clone())
            .collect::<BTreeSet<_>>();
        let mut claimed = BTreeSet::new();
        for r in references.iter().filter(|r| context.contains(&r.key)) {
            if let Some(origin) = &r.origin {
                if origin.scene.as_deref() == Some(&scene.name) {
                    if let Some(key) = native_key(scene, &world, origin) {
                        claimed.insert(key);
                        let same = self
                            .project
                            .base_adapters
                            .get(&r.base)
                            .and_then(|a| a.origin.as_ref())
                            .is_some_and(|o| o.source_identity == origin.source_identity);
                        if r.cell != cell || r.deleted || r.disabled || !same {
                            world.set_hidden(key, true);
                        }
                    }
                }
            }
        }
        let unclaimed = world
            .draws
            .iter()
            .filter(|d| d.editor_uid.is_none() && !claimed.contains(&(d.terrain, d.index)))
            .map(|d| (d.terrain, d.index))
            .collect::<Vec<_>>();
        for key in unclaimed {
            world.set_hidden(key, true);
        }
        for r in references.into_iter().filter(|r| r.cell == cell) {
            let resolved = self.project.database.resolve_reference(&r.key)?;
            let adapter = self.project.base_adapters.get(&r.base).cloned();
            let native = r
                .origin
                .as_ref()
                .filter(|o| o.scene.as_deref() == Some(&scene.name))
                .and_then(|o| native_key(scene, &world, o));
            let reusable = native.filter(|_| context.contains(&r.key)).filter(|_| {
                adapter
                    .as_ref()
                    .and_then(|a| a.origin.as_ref())
                    .zip(r.origin.as_ref())
                    .is_some_and(|(a, b)| a.source_identity == b.source_identity)
            });
            let attempt = (|| -> Result<Option<DrawKey>> {
                let key = if let Some(key) = reusable {
                    if let Some(a) = adapter.as_ref() {
                        validate_geometry(scene, a, resolve_ordinal(scene, a)?)?;
                    }
                    key
                } else {
                    if r.deleted || r.disabled {
                        return Ok(None);
                    }
                    let adapter = adapter.as_ref().context(
                        "No renderer adapter for this Base Form category yet; record retained",
                    )?;
                    if adapter.source_scene != scene.name
                        && !donors.contains_key(&adapter.source_scene)
                    {
                        let donor = legaia_engine_world_views::scene_staging::assemble_field_scene(
                            index,
                            &adapter.source_scene,
                        )
                        .map_err(anyhow::Error::msg)?;
                        donors.insert(adapter.source_scene.clone(), std::sync::Arc::new(donor));
                    }
                    let donor = if adapter.source_scene == scene.name {
                        scene
                    } else {
                        donors.get(&adapter.source_scene).unwrap().as_ref()
                    };
                    let ordinal = resolve_ordinal(donor, adapter)?;
                    validate_geometry(donor, adapter, ordinal)?;
                    let uid = self.project.render_uid(&cell, &r.key)?;
                    let source_key = if adapter.source_scene == scene.name {
                        match adapter.kind.as_str() {
                            "placement" => Some((false, ordinal)),
                            "terrain" => Some((true, ordinal)),
                            "npc" => Some((false, world_view::NPC_KEY_BASE + ordinal)),
                            _ => None,
                        }
                    } else {
                        None
                    };
                    let force_foreign = r.properties.get("legacy_instance_kind")
                        == Some(&Property::Text("foreign".into()))
                        || (r.origin.is_none() && r.flags.contains("static_architecture"));
                    let key = if let Some(source_key) = source_key.filter(|_| !force_foreign) {
                        let src = world
                            .at(source_key.0, source_key.1)
                            .context("Native preview source unavailable")?;
                        let offset =
                            [0, 1, 2].map(|a| r.transform.position[a] - src.original_center[a]);
                        let copied = world
                            .duplicate_native(source_key, scene.placements.len(), uid, offset)
                            .map_err(anyhow::Error::msg)?;
                        // Native terrain can own a serialized editor mesh layer
                        // without pretending to be a placed MAP collider record.
                        if source_key.0 {
                            let policy = world
                                .collision
                                .owned_draws
                                .iter()
                                .find(|(_, key)| **key == source_key)
                                .and_then(|(id, _)| {
                                    world
                                        .collision
                                        .owned_journal
                                        .as_ref()
                                        .and_then(|j| j.owners.iter().find(|o| &o.id == id))
                                })
                                .and_then(|o| o.mesh_policy);
                            if let Some(policy) = policy {
                                let mut generated=world.mesh_wall_geometry(copied,"Independent copy of serialized editor-owned mesh wall policy").map_err(anyhow::Error::msg)?;
                                generated.band_height = policy[0];
                                generated.min_face_area = policy[1];
                                let source =
                                    donor.terrain.get(ordinal).context("Terrain copy source")?;
                                let raw = &donor
                                    .res
                                    .tmds
                                    .get(source.res_tmd)
                                    .context("Terrain TMD")?
                                    .raw;
                                world.collision.mesh_recipes.insert(
                                    uid,
                                    super::collision_generation::MeshOnlyRecipe {
                                        uid,
                                        scene: donor.name.clone(),
                                        kind: "terrain".into(),
                                        record: ordinal,
                                        tmd_sha256: digest(raw),
                                        generated,
                                    },
                                );
                            }
                        }
                        copied
                    } else {
                        world
                            .insert_foreign(
                                donor,
                                &adapter.kind,
                                ordinal,
                                uid,
                                r.transform.position,
                                index,
                            )
                            .map_err(anyhow::Error::msg)?
                    };
                    if let Some(f) = world.foreign.iter_mut().find(|f| f.uid == uid) {
                        f.static_architecture = r.flags.contains("static_architecture");
                    }
                    key
                };
                let original = world
                    .at(key.0, key.1)
                    .context("Preview draw absent")?
                    .original_center;
                if reusable.is_some()
                    && r.transform.position == original
                    && r.transform.rotation == [0.; 3]
                    && r.transform.scale == [1.; 3]
                    && world
                        .at(key.0, key.1)
                        .is_some_and(|d| d.vertex_start == d.vertex_end)
                {
                    if r.deleted || r.disabled {
                        world.set_hidden(key, true);
                    }
                    if let Some(d) = world.draws.iter_mut().find(|d| (d.terrain, d.index) == key) {
                        d.display_name = match r.properties.get("display_name") {
                            Some(Property::Text(name)) => Some(name.clone()),
                            _ => None,
                        };
                    }
                    return Ok(Some(key));
                }

                // Tombstones retain their authored pose in the legacy manifest.
                // A hidden native allocation must be restored temporarily because
                // the existing rotation/scale controls deliberately reject it.
                if world.at(key.0, key.1).is_some_and(|d| d.hidden) {
                    ensure!(
                        world.set_hidden(key, false),
                        "Cannot restore reference preview"
                    );
                }
                ensure!(
                    world.set_offset(
                        key.0,
                        key.1,
                        [0, 1, 2].map(|a| r.transform.position[a] - original[a])
                    ),
                    "Reference position has no preview adapter"
                );
                ensure!(
                    world.set_rotation(key.0, key.1, r.transform.rotation),
                    "Reference rotation has no preview adapter"
                );
                ensure!(
                    world.set_scale(key.0, key.1, r.transform.scale),
                    "Reference scale has no preview adapter"
                );
                if r.deleted || r.disabled {
                    world.set_hidden(key, true);
                }
                if let Some(d) = world.draws.iter_mut().find(|d| (d.terrain, d.index) == key) {
                    d.display_name = match r.properties.get("display_name") {
                        Some(Property::Text(name)) => Some(name.clone()),
                        _ => None,
                    };
                }
                if let Some(npc) = world.npc(key).cloned() {
                    let model = integer_property(&r, "npc_model_selection")
                        .unwrap_or(npc.original_model_index as usize);
                    if model != npc.model_index as usize {
                        world
                            .set_npc_model(key, model as u8)
                            .map_err(anyhow::Error::msg)?;
                    }
                    let clip =
                        integer_property(&r, "npc_animation_clip").unwrap_or(npc.anim_id as usize);
                    let frame = integer_property(&r, "npc_animation_frame").unwrap_or(0);
                    if clip != npc.clip as usize || frame != npc.frame {
                        world
                            .set_npc_animation(key, clip as u8, frame)
                            .map_err(anyhow::Error::msg)?;
                    }
                }
                for a in self
                    .project
                    .collision_associations
                    .iter()
                    .filter(|a| a.reference == r.key)
                {
                    if a.family == "legacy-authored-collision" {
                        let mut assignment: collision_recipes::Assignment =
                            serde_json::from_value(a.payload.clone())?;
                        assignment.owner = world
                            .collision
                            .owner(&world, key)
                            .context("Collision owner source unavailable")?;
                        if let Err(error) = world.set_collision_recipe(key, assignment.recipe) {
                            diagnostics.push(Diagnostic{form:Some(r.key.clone()),message:format!("DEPENDENCY: preserved collision recipe belongs to another Base geometry source: {error}; shared geometry preview retained, collision payload not rewritten")});
                        }
                    } else if a.family == "legacy-generated-mesh-collision" {
                        let mut recipe: super::collision_generation::MeshOnlyRecipe =
                            serde_json::from_value(a.payload.clone())?;
                        recipe.uid = world
                            .at(key.0, key.1)
                            .and_then(|d| d.editor_uid)
                            .context("Mesh recipe reference UID missing")?;
                        world.collision.mesh_recipes.insert(recipe.uid, recipe);
                    }
                }
                if let Some(Property::Vector(tint)) = resolved.base.materials.get("preview_tint") {
                    let draw = world.at(key.0, key.1).unwrap();
                    let range = draw.vertex_start..draw.vertex_end;
                    if let Some(model) = &mut world.model {
                        for color in &mut model.modulation[range] {
                            for a in 0..3 {
                                color[a] =
                                    (color[a] as f32 * tint[a]).round().clamp(0., 255.) as u8;
                            }
                        }
                    }
                }
                Ok(Some(key))
            })();
            match attempt {
                Ok(Some(key)) => {
                    mappings.insert(r.key, key);
                }
                Ok(None) => {}
                Err(e) => {
                    if let Some(key) = native {
                        world.set_hidden(key, true);
                    }
                    diagnostics.push(Diagnostic {
                        form: Some(r.key),
                        message: format!("PREVIEW: {e:#}; record and dependencies retained"),
                    });
                }
            }
        }
        if donors.len() > 8 {
            let remove = donors
                .keys()
                .take(donors.len() - 8)
                .cloned()
                .collect::<Vec<_>>();
            for key in remove {
                donors.remove(&key);
            }
        }
        self.donor_scenes = donors;
        self.project.diagnostics.retain(|d| {
            !d.message.starts_with("PREVIEW:") && !d.message.starts_with("DEPENDENCY:")
        });
        self.project.diagnostics.extend(diagnostics);
        self.projected_transforms = mappings
            .iter()
            .filter_map(|(r, k)| {
                world.at(k.0, k.1).map(|d| {
                    (
                        r.clone(),
                        Transform {
                            position: d.center,
                            rotation: d.rotation,
                            scale: d.scale,
                        },
                    )
                })
            })
            .collect();
        self.reference_draws = mappings;
        self.project.validate()?;
        Ok(world)
    }
    /// Exact saved journal is a portable fallback only. A physical sidecar is
    /// authoritative, including its validation failure; never replace it here.
    pub fn preserved_collision_metadata(&self) -> Result<Option<Vec<u8>>> {
        if self.source.with_extension("lck-collision.json").exists() {
            return Ok(None);
        }
        let mut found: Option<Vec<u8>> = None;
        for snapshot in &self.project.sidecars {
            ensure!(
                digest(&snapshot.exact_bytes) == snapshot.sha256,
                "Saved collision evidence changed"
            );
            let Ok(root) = serde_json::from_slice::<serde_json::Value>(&snapshot.exact_bytes)
            else {
                continue;
            };
            if root["image_sha256"].as_str() != Some(&self.source_fingerprint) {
                continue;
            }
            if let Some(old) = &found {
                ensure!(
                    old == &snapshot.exact_bytes,
                    "Conflicting saved collision journals for source revision"
                );
            }
            found = Some(snapshot.exact_bytes.clone());
        }
        Ok(found)
    }
    pub fn export_gaps(&self) -> Vec<String> {
        let mut gaps = vec![];
        let relevant = self
            .project
            .database
            .references()
            .into_values()
            .filter(|r| Some(&r.cell) == self.cell.as_ref() && !r.deleted && !r.disabled)
            .map(|r| r.base.clone())
            .collect::<BTreeSet<_>>();
        for (key, b) in self.project.database.bases() {
            if !relevant.contains(&key) {
                continue;
            }
            if !b.materials.is_empty() {
                gaps.push(format!("Base {key:?}: shared material edit is persisted and previewed; native shared-material override serialization is not verified"));
            }
            if b.revision > 0
                && self
                    .project
                    .database
                    .master_layer()
                    .bases
                    .get(&key)
                    .is_some_and(|old| {
                        old.resources != b.resources
                            || old.collision != b.collision
                            || old.behavior != b.behavior
                    })
            {
                gaps.push(format!("Base {key:?}: changed shared definition needs native dependency/ownership verification"));
            }
        }
        for d in &self.project.diagnostics {
            if d.message.starts_with("PREVIEW:")
                || d.message.starts_with("DEPENDENCY:")
                || d.message.starts_with("MASTER:")
            {
                gaps.push(d.message.clone());
            }
        }
        gaps
    }
}
fn integer_property(r: &PlacedReference, name: &str) -> Option<usize> {
    if let Some(Property::Integer(v)) = r.properties.get(name) {
        usize::try_from(*v).ok()
    } else {
        None
    }
}
fn native_key(
    scene: &AssembledScene,
    world: &WorldModel,
    origin: &NativeOrigin,
) -> Option<DrawKey> {
    if let Some(record) = origin.man_record {
        return world
            .npcs
            .iter()
            .find(|n| n.record == record as usize && !n.is_copy)
            .map(|n| (false, n.key));
    }
    let cell = origin.cell?;
    if origin.source_identity.contains(":terrain-cell:") {
        scene
            .terrain
            .iter()
            .position(|d| [d.cell.0, d.cell.1] == cell)
            .map(|i| (true, i))
    } else {
        world
            .collision
            .sources
            .iter()
            .find(|(_, p)| {
                p.object.cell == cell
                    && Some(p.object.descriptor_id) == origin.descriptor
                    && Some(p.object.map_entry) == origin.prot_entry
            })
            .map(|(k, _)| *k)
    }
}
fn resolve_ordinal(scene: &AssembledScene, a: &LegacyBaseAdapter) -> Result<usize> {
    if let Some(origin) = &a.origin {
        if let Some(cell) = origin.cell {
            return if a.kind == "terrain" {
                scene
                    .terrain
                    .iter()
                    .position(|d| [d.cell.0, d.cell.1] == cell)
            } else {
                scene
                    .placements
                    .iter()
                    .position(|d| [d.cell.0, d.cell.1] == cell)
            }
            .context("Original native source cell no longer resolves");
        }
        if let Some(record) = origin.man_record {
            return Ok(record as usize);
        }
    }
    Ok(a.source_ordinal)
}
fn validate_geometry(scene: &AssembledScene, a: &LegacyBaseAdapter, ordinal: usize) -> Result<()> {
    if let Some(expected) = a.origin.as_ref().and_then(|o| o.fingerprints.get("TMD")) {
        if a.kind != "npc" {
            let slot = if a.kind == "mesh" {
                scene.env_tmds.get(ordinal).copied()
            } else {
                if a.kind == "terrain" {
                    scene.terrain.get(ordinal)
                } else {
                    scene.placements.get(ordinal)
                }
                .map(|d| d.res_tmd)
            }
            .context("Native model reference missing")?;
            ensure!(
                &digest(&scene.res.tmds.get(slot).context("Native TMD missing")?.raw) == expected,
                "Donor model fingerprint changed"
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "real native Rim Elm/Conkram record, renderer and project acceptance; no GUI or emulator"]
    fn native_base_reference_acceptance_and_reordered_draws() -> Result<()> {
        let source = Path::new(r"D:\Emulators\Games\Legend of Legaia\LegendOfLegaia.img");
        let lib = crate::library::Library::open(source)?;
        let mut rim =
            legaia_engine_world_views::scene_staging::assemble_field_scene(&lib.index, "town01")
                .map_err(anyhow::Error::msg)?;
        let mut native = world_view::build_world(&rim, &lib.index);
        super::super::native_wall_export::attach_preview(source, &rim, &mut native, &lib.index);
        let original_text = scene_instances::serialize("town01", &native);
        let mut session = Session::temporary(source, &lib.index)?;
        let rendered = session.enter(
            &rim,
            &lib.index,
            &native,
            "original:town01".into(),
            "Rim Elm".into(),
            None,
        )?;
        ensure!(
            scene_instances::serialize("town01", &rendered) == original_text,
            "Unchanged native projection changed legacy export operations"
        );
        let original = session
            .reference_at(&rendered, (false, 14))
            .context("Rim Elm source reference")?;
        let base = session
            .project
            .database
            .reference(&original)
            .unwrap()
            .base
            .clone();
        let original_trs = session
            .project
            .database
            .reference(&original)
            .unwrap()
            .transform;
        let rim_cell = session.cell.clone().unwrap();
        let copy_world = session.enter(
            &rim,
            &lib.index,
            &native,
            "copy:9:town01".into(),
            "Rim Elm Copy".into(),
            Some(9),
        )?;
        ensure!(
            scene_instances::serialize("town01", &copy_world) == original_text,
            "Cell copy fabricated extra placements instead of mapping native references"
        );
        let copy_ref = session
            .reference_at(&copy_world, (false, 14))
            .context("Copy reference")?;
        ensure!(
            copy_ref != original
                && session.project.database.reference(&copy_ref).unwrap().base == base,
            "Cell copy lost independent reference/shared definition semantics"
        );
        session.transaction(|p| {
            p.database.move_reference(
                &copy_ref,
                Transform {
                    position: [2400., 100., 2400.],
                    ..Default::default()
                },
            )
        })?;
        ensure!(
            session
                .project
                .database
                .reference(&original)
                .unwrap()
                .transform
                == original_trs,
            "Copied cell changed original"
        );
        let conk =
            legaia_engine_world_views::scene_staging::assemble_field_scene(&lib.index, "conc")
                .map_err(anyhow::Error::msg)?;
        let conk_native = world_view::build_world(&conk, &lib.index);
        session.enter(
            &conk,
            &lib.index,
            &conk_native,
            "original:conc".into(),
            "Conkram".into(),
            None,
        )?;
        let second = session.place(
            base.clone(),
            Transform {
                position: [2000., 64., 2000.],
                ..Default::default()
            },
        )?;
        let render = session.render(&conk, &lib.index)?;
        session.synchronize_gesture(&conk, &lib.index, &render)?;
        let third = session.duplicate(&[second.clone()])?[0].clone();
        ensure!(
            third != second && second != original,
            "Reference key reused"
        );
        ensure!(
            session.project.database.reference(&third).unwrap().base == base,
            "Duplicate copied a definition instead of a reference"
        );
        session.transaction(|p| {
            p.database.move_reference(
                &third,
                Transform {
                    position: [2200., 64., 1800.],
                    rotation: [0., 0.5, 0.],
                    scale: [1., 2., 3.],
                },
            )
        })?;
        ensure!(
            session
                .project
                .database
                .reference(&original)
                .unwrap()
                .transform
                == original_trs
                && session
                    .project
                    .database
                    .reference(&second)
                    .unwrap()
                    .transform
                    .position
                    == [2000., 64., 2000.],
            "One reference changed another"
        );
        let conk_base = session
            .project
            .database
            .references()
            .values()
            .find(|r| {
                r.cell == *session.cell.as_ref().unwrap()
                    && r.origin.is_some()
                    && session
                        .project
                        .database
                        .resolve_base(&r.base)
                        .is_some_and(|b| b.kind == FormKind::StaticEnvironment)
            })
            .context("Conkram geometry definition")?
            .base
            .clone();
        session.assign_geometry(&base, &conk_base)?;
        session.tint_base(&base, [0.5, 1., 0.5])?;
        for r in [&original, &second, &third] {
            ensure!(
                session
                    .project
                    .database
                    .resolve_reference(r)?
                    .base
                    .materials
                    .contains_key("preview_tint"),
                "Shared material did not resolve on every reference"
            );
        }
        let render = session.render(&conk, &lib.index)?;
        ensure!(
            session.reference_draws.contains_key(&second)
                && session.reference_draws.contains_key(&third),
            "Shared changed geometry failed to project: {:?}",
            session.project.diagnostics
        );
        let before = session
            .project
            .database
            .reference(&third)
            .unwrap()
            .transform;
        session.synchronize_gesture(&conk, &lib.index, &render)?;
        ensure!(
            session
                .project
                .database
                .reference(&third)
                .unwrap()
                .transform
                == before,
            "Renderer rebuild rewrote exact editor transform"
        );
        let new_base = session.duplicate_base(&base)?;
        session.assign_base(&third, new_base.clone())?;
        session.tint_base(&base, [1., 1., 1.])?;
        ensure!(
            session
                .project
                .database
                .resolve_base(&new_base)
                .unwrap()
                .materials
                != session
                    .project
                    .database
                    .resolve_base(&base)
                    .unwrap()
                    .materials,
            "Duplicate Base shared mutable definition"
        );
        let before_delete = session.project.clone();
        session.disable(&second, true)?;
        ensure!(
            !session.project.database.reference(&second).unwrap().deleted,
            "Disable became Delete"
        );
        session.delete(&[second.clone()])?;
        ensure!(
            session.project.database.resolve_base(&base).is_some()
                && session.project.database.reference(&third).is_some(),
            "Deletion removed shared definition/other reference"
        );
        session.restore_project(before_delete.clone())?;
        ensure!(
            session.project.database.reference(&second).unwrap().key == second
                && !session.project.database.reference(&second).unwrap().deleted,
            "Undo allocated new identity"
        );
        let temp = tempfile::tempdir()?;
        let path = temp.path().join("stage-a.lckproject");
        let mut file = ProjectFile::new(session.project.clone(), path.clone());
        file.save()?;
        session.project = ProjectFile::open(&path, Some(&file.project.master.disc_sha256))?.project;
        ensure!(
            session.project.database.reference(&third).unwrap().base == new_base,
            "Reopen lost independent definition"
        );
        session.enter(
            &rim,
            &lib.index,
            &native,
            "original:town01".into(),
            "Rim Elm".into(),
            None,
        )?;
        ensure!(session.cell == Some(rim_cell), "Cell identity changed");
        let key = session
            .reference_draws
            .get(&original)
            .copied()
            .context("Rebuilt original Reference missing")?;
        let stable = session
            .project
            .database
            .reference(&original)
            .unwrap()
            .key
            .clone();
        rim.placements.reverse();
        let reordered_native = world_view::build_world(&rim, &lib.index);
        session.enter(
            &rim,
            &lib.index,
            &reordered_native,
            "original:town01".into(),
            "Rim Elm".into(),
            None,
        )?;
        ensure!(
            session.project.database.reference(&original).unwrap().key == stable,
            "Draw reordering changed FormKey"
        );
        ensure!(
            session.reference_draws.contains_key(&original),
            "Draw mapping lost Reference after reorder (previous key {:?})",
            key
        );
        ensure!(
            !session.export_gaps().is_empty(),
            "Unverified shared-material compiler support silently claimed"
        );
        // An Object Window static reference needs the existing explicit
        // FOREIGN_STATIC writer even when its donor is the destination cell.
        // Legacy native Ctrl+D copies retain their existing INSTANCE routing.
        let local_base = session
            .project
            .base_adapters
            .iter()
            .find(|(key, a)| a.source_scene == rim.name && a.kind == "placement" && *key != &base)
            .map(|(key, _)| key.clone())
            .context("Local static Base")?;
        let placed = session.place(
            local_base,
            Transform {
                position: [6000., 0., 6000.],
                ..Default::default()
            },
        )?;
        let local_world = session.render(&rim, &lib.index)?;
        let draw = session.reference_draws[&placed];
        let uid = local_world
            .at(draw.0, draw.1)
            .and_then(|d| d.editor_uid)
            .context("Static UID")?;
        ensure!(
            local_world
                .foreign
                .iter()
                .any(|f| f.uid == uid && f.static_architecture && f.source == rim.name),
            "Same-cell Object Window placement silently lost static architecture state"
        );
        let text = scene_instances::serialize(&rim.name, &local_world);
        ensure!(
            text.contains(&format!("FOREIGN_STATIC\t{uid}\n")),
            "Static flag did not reach native writer manifest"
        );
        Ok(())
    }
}
