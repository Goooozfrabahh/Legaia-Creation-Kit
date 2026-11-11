//! LCK_PROJECT_V1: read-only master plus an active authoring layer. Native IMG
//! serialization and hash-bound collision sidecars remain separate adapters.
use super::form_database::*;
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(super) struct MasterDependency {
    pub file: FileId,
    pub disc_sha256: String,
    pub path_hint: Option<PathBuf>,
}
fn resolved_default() -> bool {
    true
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct CellBinding {
    #[serde(default = "resolved_default")]
    pub resolved: bool,
    pub identity: String,
    pub key: FormKey,
    pub source_scene: String,
    pub label: String,
    pub legacy_copy_id: Option<u64>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct LegacyBinding {
    pub cell: FormKey,
    pub source_identity: String,
    pub uid: Option<u64>,
    pub reference: FormKey,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(super) struct LegacyBaseAdapter {
    pub source_scene: String,
    pub kind: String,
    /// Legacy decoder hint, not permanent identity. NativeOrigin validates it.
    pub source_ordinal: usize,
    pub origin: Option<NativeOrigin>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(super) struct SidecarSnapshot {
    pub source_hint: PathBuf,
    pub sha256: String,
    pub exact_bytes: Vec<u8>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(super) struct CollisionAssociation {
    pub reference: FormKey,
    pub family: String,
    pub payload: serde_json::Value,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct RenderUid {
    pub cell: FormKey,
    pub reference: FormKey,
    pub uid: u64,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(super) struct LckProject {
    // LCK_R57_NATIVE_WARPS
    #[serde(default)]
    pub native_warps: Vec<super::native_warps::SceneWarps>,
    #[serde(default)]
    pub render_uids: Vec<RenderUid>,
    #[serde(default)]
    pub next_render_uid: BTreeMap<FormKey, u64>,
    pub format: String,
    pub schema_version: u32,
    pub file_id: FileId,
    pub master: MasterDependency,
    pub database: FormDatabase,
    pub cells: Vec<CellBinding>,
    pub legacy_bindings: Vec<LegacyBinding>,
    pub base_adapters: BTreeMap<FormKey, LegacyBaseAdapter>,
    pub collision_associations: Vec<CollisionAssociation>,
    pub sidecars: Vec<SidecarSnapshot>,
    /// Original text is retained losslessly for rollback/compatibility, not the
    /// authority for new authoring. The database is saved in the same transaction.
    #[serde(default)]
    pub cell_snapshots: BTreeMap<String, String>,
    pub legacy_documents: BTreeMap<FormKey, String>,
    pub diagnostics: Vec<Diagnostic>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Diagnostic {
    pub form: Option<FormKey>,
    pub message: String,
}
#[derive(Clone, Debug)]
pub(super) struct LegacyObjectInput {
    pub source_identity: String,
    pub base_identity: String,
    pub base: BaseForm,
    pub adapter: LegacyBaseAdapter,
    pub origin: Option<NativeOrigin>,
    pub uid: Option<u64>,
    pub original_transform: Transform,
    pub original_properties: BTreeMap<String, Property>,
    pub original_flags: BTreeSet<String>,
    pub transform: Transform,
    pub display_name: Option<String>,
    pub hidden: bool,
    pub disabled: bool,
    pub flags: BTreeSet<String>,
    pub properties: BTreeMap<String, Property>,
    pub collision_owners: BTreeSet<String>,
    pub collision_payloads: Vec<(String, serde_json::Value)>,
}
#[derive(Clone, Debug)]
pub(super) struct LegacySceneInput {
    pub identity: String,
    pub scene: String,
    pub label: String,
    pub copy_id: Option<u64>,
    pub exact_document: String,
    pub objects: Vec<LegacyObjectInput>,
}

pub(super) fn new_project_id() -> Result<FileId> {
    static SERIAL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let seed = format!(
        "{}-{}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos(),
        std::process::id(),
        SERIAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    );
    FileId::new(format!(
        "lck-{}",
        super::collision_recipes::digest(seed.as_bytes())
    ))
}
impl LckProject {
    pub fn new(master: MasterDependency, file_id: FileId) -> Result<Self> {
        let database = FormDatabase::new(master.file.clone(), file_id.clone())?;
        let p = Self {
            format: "LCK_PROJECT_V1".into(),
            render_uids: vec![],
            next_render_uid: Default::default(),
            native_warps: Vec::new(),
            schema_version: 1,
            file_id,
            master,
            database,
            cells: vec![],
            legacy_bindings: vec![],
            base_adapters: Default::default(),
            collision_associations: vec![],
            sidecars: vec![],
            cell_snapshots: Default::default(),
            legacy_documents: Default::default(),
            diagnostics: vec![],
        };
        Ok(p)
    }
    pub fn validate(&mut self) -> Result<()> {
        ensure!(
            self.format == "LCK_PROJECT_V1" && self.schema_version == 1,
            "Unsupported project schema; original file preserved"
        );
        ensure!(
            self.file_id == self.database.active_file
                && self.master.file == self.database.master_file,
            "Project file identities disagree"
        );
        ensure!(
            self.master.disc_sha256.len() == 64
                && self
                    .master
                    .disc_sha256
                    .bytes()
                    .all(|b| b.is_ascii_hexdigit()),
            "Invalid master-disc fingerprint"
        );
        self.database.validate()?;
        let mut cell_ids = BTreeSet::new();
        for c in &self.cells {
            ensure!(
                cell_ids.insert(&c.identity)
                    && self
                        .database
                        .resolve_base(&c.key)
                        .is_some_and(|b| b.kind == FormKind::Cell),
                "Invalid/duplicate cell binding"
            );
        }
        for (identity, raw) in &self.cell_snapshots {
            let binding = self.cells.iter()
                .find(|cell| &cell.identity == identity)
                .context("Scene snapshot has no matching cell")?;
            ensure!(
                raw.len() <= 2_200_000 &&
                raw.starts_with(&format!(
                    "LCK_SCENE_V1\t{}\n", binding.source_scene
                )),
                "Invalid project scene snapshot"
            );
        }
        let mut identities = BTreeSet::new();
        let mut uids = BTreeSet::new();

        for b in &self.legacy_bindings {
            ensure!(
                identities.insert((&b.cell, &b.source_identity))
                    && self.database.reference(&b.reference).is_some(),
                "Invalid/duplicate legacy binding"
            );
            ensure!(
                self.cells.iter().any(|c| c.key == b.cell),
                "Legacy cell absent"
            );
            if let Some(uid) = b.uid {
                ensure!(
                    uid > 0 && uids.insert((&b.cell, uid)),
                    "Duplicate legacy UID mapping"
                );
            }
        }
        let mut render_refs = BTreeSet::new();
        let mut render_ids = BTreeSet::new();
        for binding in &self.render_uids {
            ensure!(
                self.database.reference(&binding.reference).is_some()
                    && self.cells.iter().any(|c| c.key == binding.cell)
                    && binding.uid > 0
                    && render_refs.insert((&binding.cell, &binding.reference))
                    && render_ids.insert((&binding.cell, binding.uid)),
                "Invalid/duplicate render UID mapping"
            );
            ensure!(
                self.next_render_uid
                    .get(&binding.cell)
                    .is_some_and(|next| *next > binding.uid),
                "Render UID allocator would reuse identity"
            );
        }
        for (key, _) in &self.base_adapters {
            ensure!(
                self.database.resolve_base(key).is_some(),
                "Adapter base absent"
            );
        }
        for c in &self.collision_associations {
            ensure!(
                self.database.reference(&c.reference).is_some(),
                "Orphaned collision association"
            );
        }
        for sidecar in &self.sidecars {
            ensure!(
                super::collision_recipes::digest(&sidecar.exact_bytes) == sidecar.sha256,
                "Preserved collision sidecar changed"
            );
        }
        Ok(())
    }
    pub fn ensure_cell(
        &mut self,
        identity: &str,
        scene: &str,
        label: &str,
        copy_id: Option<u64>,
    ) -> Result<FormKey> {
        if let Some(c) = self.cells.iter().find(|c| c.identity == identity) {
            ensure!(
                c.source_scene == scene && c.legacy_copy_id == copy_id,
                "Cell import identity changed source"
            );
            return Ok(c.key.clone());
        }
        let editor = format!(
            "CELL_{}",
            super::collision_recipes::digest(identity.as_bytes())
        );
        let key = if copy_id.is_none() {
            let dummy = FormKey::new(self.master.file.clone(), 1)?;
            self.database.import_master_base(
                format!("cell:{identity}"),
                BaseForm::empty(dummy, editor, FormKind::Cell),
            )?
        } else {
            self.database.create_base(editor, FormKind::Cell)?
        };
        self.cells.push(CellBinding {
            resolved: false,
            identity: identity.into(),
            key: key.clone(),
            source_scene: scene.into(),
            label: label.into(),
            legacy_copy_id: copy_id,
        });
        Ok(key)
    }
    /// Deterministic, idempotent migration. The outer clone includes registry,
    /// allocation state, memberships, UID mappings and collision associations.
    pub fn migrate_scene(&mut self, input: &LegacySceneInput) -> Result<FormKey> {
        let mut next = self.clone();
        let cell = next.ensure_cell(&input.identity, &input.scene, &input.label, input.copy_id)?;
        let mut seen = BTreeSet::new();
        for obj in &input.objects {
            ensure!(
                seen.insert(obj.source_identity.clone()),
                "Duplicate legacy source identity"
            );
            let prior = next
                .legacy_bindings
                .iter()
                .find(|b| {
                    b.cell == cell
                        && match obj.uid {
                            Some(uid) => b.uid == Some(uid),
                            None => b.source_identity == obj.source_identity,
                        }
                })
                .cloned();
            let retained = prior
                .as_ref()
                .and_then(|b| next.database.reference(&b.reference))
                .map(|r| r.base.clone());
            let base = if obj.origin.is_none() && retained.is_some() {
                retained.unwrap()
            } else {
                next.database
                    .import_master_base(format!("base:{}", obj.base_identity), obj.base.clone())?
            };
            if let Some(o) = &obj.origin {
                next.check_origin(&base, &o.fingerprints);
            }
            next.base_adapters
                .entry(base.clone())
                .or_insert_with(|| obj.adapter.clone());
            let key = if let Some(b) = prior {
                ensure!(
                    b.source_identity == obj.source_identity,
                    "Legacy UID was reassigned to another source"
                );
                b.reference
            } else {
                let key = if obj.uid.is_none() && input.copy_id.is_none() {
                    let dummy = FormKey::new(next.master.file.clone(), 1)?;
                    let r = PlacedReference {
                        key: dummy,
                        base: base.clone(),
                        cell: cell.clone(),
                        transform: obj.original_transform,
                        editor_id: None,
                        disabled: false,
                        deleted: false,
                        flags: obj.original_flags.clone(),
                        properties: obj.original_properties.clone(),
                        origin: obj.origin.clone(),
                        collision_owners: obj.collision_owners.clone(),
                    };
                    next.database.import_master_reference(
                        format!("ref:{}:{}", input.identity, obj.source_identity),
                        r,
                    )?
                } else {
                    next.database.place_reference(
                        base.clone(),
                        cell.clone(),
                        obj.original_transform,
                    )?
                };
                next.legacy_bindings.push(LegacyBinding {
                    cell: cell.clone(),
                    source_identity: obj.source_identity.clone(),
                    uid: obj.uid,
                    reference: key.clone(),
                });
                key
            };
            let mut reference = next
                .database
                .reference(&key)
                .context("Imported reference")?
                .clone();
            reference.base = base;
            if obj.uid.is_none() {
                reference.origin = obj.origin.clone();
            }
            reference.cell = cell.clone();
            reference.transform = obj.transform;
            reference.deleted = obj.hidden;
            reference.disabled = obj.disabled;
            reference.flags = obj.flags.clone();
            reference.properties = obj.properties.clone();
            if let Some(name) = &obj.display_name {
                reference
                    .properties
                    .insert("display_name".into(), Property::Text(name.clone()));
            }
            reference
                .collision_owners
                .extend(obj.collision_owners.iter().cloned());
            if next.database.reference(&key) != Some(&reference) {
                next.database.edit_reference(&key, |r| {
                    *r = reference;
                    Ok(())
                })?;
            }
            next.collision_associations.retain(|a| a.reference != key);
            for (family, payload) in &obj.collision_payloads {
                next.collision_associations.push(CollisionAssociation {
                    reference: key.clone(),
                    family: family.clone(),
                    payload: payload.clone(),
                });
            }
            if obj.origin.is_none() {
                next.diagnostics.push(Diagnostic{form:Some(key),message:"Native provenance unresolved; record preserved and editable; no native ownership manufactured".into()});
            }
        }
        // Legacy deletion of a created draw used to remove the draw completely.
        // Keep its identity and relationships as an active tombstone instead.
        for b in next
            .legacy_bindings
            .clone()
            .into_iter()
            .filter(|b| b.cell == cell && b.uid.is_some() && !seen.contains(&b.source_identity))
        {
            next.database.delete_reference(&b.reference)?;
        }
        next.legacy_documents
            .insert(cell.clone(), input.exact_document.clone());
        next.cells
            .iter_mut()
            .find(|c| c.key == cell)
            .unwrap()
            .resolved = true;
        next.deduplicate_diagnostics();
        next.validate()?;
        *self = next;
        Ok(cell)
    }
    pub fn render_uid(&mut self, cell: &FormKey, reference: &FormKey) -> Result<u64> {
        if let Some(b) = self
            .legacy_bindings
            .iter()
            .find(|b| &b.cell == cell && &b.reference == reference && b.uid.is_some())
        {
            return Ok(b.uid.unwrap());
        }
        if let Some(b) = self
            .render_uids
            .iter()
            .find(|b| &b.cell == cell && &b.reference == reference)
        {
            return Ok(b.uid);
        }
        let used = self
            .legacy_bindings
            .iter()
            .filter(|b| &b.cell == cell)
            .filter_map(|b| b.uid)
            .chain(
                self.render_uids
                    .iter()
                    .filter(|b| &b.cell == cell)
                    .map(|b| b.uid),
            )
            .max()
            .unwrap_or(0);
        let next = self.next_render_uid.entry(cell.clone()).or_insert(1);
        let uid = (*next).max(used.checked_add(1).context("Editor UID exhausted")?);
        *next = uid.checked_add(1).context("Editor UID exhausted")?;
        self.render_uids.push(RenderUid {
            cell: cell.clone(),
            reference: reference.clone(),
            uid,
        });
        Ok(uid)
    }
    pub fn retain_sidecar(&mut self, path: &Path, bytes: Vec<u8>) -> Result<()> {
        let hash = super::collision_recipes::digest(&bytes);
        if self
            .sidecars
            .iter()
            .any(|s| s.sha256 == hash && s.source_hint == path)
        {
            return Ok(());
        }
        self.sidecars.push(SidecarSnapshot {
            source_hint: path.into(),
            sha256: hash,
            exact_bytes: bytes,
        });
        Ok(())
    }
    pub fn check_master(&mut self, actual: Option<&str>) {
        self.diagnostics
            .retain(|d| !d.message.starts_with("MASTER:"));
        match actual{Some(hash)if hash.eq_ignore_ascii_case(&self.master.disc_sha256)=>{},Some(_)=>self.diagnostics.push(Diagnostic{form:None,message:"MASTER: selected disc fingerprint differs; all records retained, native provenance must be checked before preview/export".into()}),None=>self.diagnostics.push(Diagnostic{form:None,message:"MASTER: source disc unavailable; all records retained".into()})}
    }
    pub fn check_origin(&mut self, key: &FormKey, actual: &BTreeMap<String, String>) {
        if let Some(origin) = self
            .database
            .resolve_base(key)
            .and_then(|b| b.origin.as_ref())
        {
            for (name, expected) in &origin.fingerprints {
                if actual.get(name) != Some(expected) {
                    self.diagnostics.push(Diagnostic {
                        form: Some(key.clone()),
                        message: format!(
                            "Native donor evidence missing/changed: {name}; definition retained"
                        ),
                    });
                }
            }
        }
        self.deduplicate_diagnostics();
    }
    fn deduplicate_diagnostics(&mut self) {
        let mut seen = BTreeSet::new();
        self.diagnostics
            .retain(|d| seen.insert((d.form.clone(), d.message.clone())));
    }
}

pub(super) struct ProjectFile {
    pub project: LckProject,
    pub path: PathBuf,
    snapshot: Option<Vec<u8>>,
}
impl ProjectFile {
    pub fn new(project: LckProject, path: PathBuf) -> Self {
        Self {
            project,
            path,
            snapshot: None,
        }
    }
    pub fn open(path: &Path, actual_master: Option<&str>) -> Result<Self> {
        let bytes = std::fs::read(path)?;
        let mut project: LckProject =
            serde_json::from_slice(&bytes).context("Read LCK_PROJECT_V1")?;
        project.validate()?;
        project.check_master(actual_master);
        Ok(Self {
            project,
            path: path.into(),
            snapshot: Some(bytes),
        })
    }
    pub fn save(&mut self) -> Result<()> {
        ensure!(
            self.path
                .extension()
                .is_some_and(|e| e.to_string_lossy().eq_ignore_ascii_case("lckproject")),
            "Save Project requires .lckproject; never a retail IMG path"
        );
        let mut checked = self.project.clone();
        checked.validate()?;
        let bytes = serde_json::to_vec_pretty(&checked)?;
        let mut readback: LckProject = serde_json::from_slice(&bytes)?;
        readback.validate()?;
        crate::asset_catalog::atomic_write(&self.path, &bytes, self.snapshot.as_deref())
            .map_err(anyhow::Error::msg)?;
        self.project = checked;
        self.snapshot = Some(bytes);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn project() -> LckProject {
        LckProject::new(
            MasterDependency {
                file: FileId::new("retail-test").unwrap(),
                disc_sha256: "a".repeat(64),
                path_hint: None,
            },
            FileId::new("project-test").unwrap(),
        )
        .unwrap()
    }
    fn input(p: &LckProject) -> LegacySceneInput {
        let key = FormKey::new(p.master.file.clone(), 1).unwrap();
        let base = BaseForm::empty(key, "HOUSE_NATIVE_A".into(), FormKind::StaticEnvironment);
        let object = LegacyObjectInput {
            source_identity: "map-house-cell-20-23".into(),
            base_identity: "town01-descriptor-house".into(),
            base,
            adapter: LegacyBaseAdapter {
                source_scene: "town01".into(),
                kind: "placement".into(),
                source_ordinal: 13,
                origin: None,
            },
            origin: None,
            uid: None,
            original_transform: Default::default(),
            original_properties: Default::default(),
            original_flags: Default::default(),
            transform: Transform {
                position: [128., 96., -256.],
                rotation: [0., 0.4, 0.],
                scale: [1., 2., 3.],
            },
            display_name: Some("My house".into()),
            hidden: false,
            disabled: false,
            flags: BTreeSet::from(["static_architecture".into()]),
            properties: BTreeMap::from([("npc_animation".into(), Property::Integer(3))]),
            collision_owners: BTreeSet::from(["existing-owned-layer".into()]),
            collision_payloads: vec![(
                "legacy-collision".into(),
                serde_json::json!({"preserved":true}),
            )],
        };
        LegacySceneInput {
            identity: "original:town01".into(),
            scene: "town01".into(),
            label: "Rim Elm".into(),
            copy_id: None,
            exact_document: "LCK_SCENE_V1\ttown01\n".into(),
            objects: vec![object],
        }
    }
    #[test]
    fn migration_is_idempotent_preserves_uids_flags_names_trs_and_collision() {
        let mut p = project();
        let mut i = input(&p);
        let mut copy = i.objects[0].clone();
        copy.uid = Some(42);
        copy.source_identity = "legacy-created-42".into();
        i.objects.push(copy);
        p.migrate_scene(&i).unwrap();
        let ids = p.legacy_bindings.clone();
        let alloc = serde_json::to_vec(&p.database.allocator).unwrap();
        p.migrate_scene(&i).unwrap();
        assert_eq!(p.legacy_bindings, ids);
        assert_eq!(serde_json::to_vec(&p.database.allocator).unwrap(), alloc);
        let r = p.database.reference(&ids[1].reference).unwrap();
        assert_eq!(r.transform.scale, [1., 2., 3.]);
        assert!(r.flags.contains("static_architecture"));
        assert_eq!(
            r.properties["display_name"],
            Property::Text("My house".into())
        );
        assert!(r.collision_owners.contains("existing-owned-layer"));
        assert_eq!(p.collision_associations.len(), 2);
        assert_eq!(p.database.use_info(&r.base).count, 2);
    }
    #[test]
    fn project_atomic_roundtrip_keeps_file_identity_records_registry_sidecars_and_allocator() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("example.lckproject");
        let mut p = project();
        let i = input(&p);
        p.migrate_scene(&i).unwrap();
        p.retain_sidecar(
            Path::new("original.lck-collision.json"),
            b"exact original bytes".to_vec(),
        )
        .unwrap();
        let mut file = ProjectFile::new(p, path.clone());
        file.save().unwrap();
        let bytes = std::fs::read(&path).unwrap();
        let mut reopened = ProjectFile::open(&path, Some(&"a".repeat(64))).unwrap();
        assert_eq!(
            serde_json::to_vec(&file.project).unwrap(),
            serde_json::to_vec(&reopened.project).unwrap()
        );
        reopened.project.format = "bad".into();
        assert!(reopened.save().is_err());
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        reopened.project = file.project.clone();
        std::fs::write(&path, b"external writer").unwrap();
        assert!(reopened.save().is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"external writer");
    }
    #[test]
    fn stale_missing_master_and_donor_preserve_project_and_emit_diagnostics() {
        let mut p = project();
        let mut i = input(&p);
        i.objects[0].base.origin = Some(NativeOrigin {
            source_identity: "original-house".into(),
            scene: Some("town01".into()),
            prot_entry: Some(1),
            descriptor: Some(462),
            man_record: None,
            anchor: Some([20, 25]),
            cell: Some([20, 23]),
            fingerprints: BTreeMap::from([("TMD".into(), "b".repeat(64))]),
        });
        p.migrate_scene(&i).unwrap();
        let key = p.legacy_bindings[0].reference.clone();
        let base = p.database.reference(&key).unwrap().base.clone();
        p.check_master(None);
        p.check_master(Some(&"c".repeat(64)));
        p.check_origin(&base, &BTreeMap::new());
        assert!(p.database.reference(&key).is_some());
        assert!(p.diagnostics.iter().any(|d| d.message.contains("differs")));
        assert!(p.diagnostics.iter().any(|d| d.message.contains("donor")));
    }
    #[test]
    fn cell_copies_and_deleted_created_reference_identity_survive_remigration() {
        let mut p = project();
        let mut i = input(&p);
        i.copy_id = Some(9);
        i.identity = "copy:9:town01".into();
        i.objects[0].uid = Some(12);
        p.migrate_scene(&i).unwrap();
        let key = p.legacy_bindings[0].reference.clone();
        i.objects.clear();
        p.migrate_scene(&i).unwrap();
        assert!(p.database.reference(&key).unwrap().deleted);
        i = input(&p);
        i.copy_id = Some(9);
        i.identity = "copy:9:town01".into();
        i.objects[0].uid = Some(12);
        p.migrate_scene(&i).unwrap();
        assert!(!p.database.reference(&key).unwrap().deleted);
        assert_eq!(p.cells[0].legacy_copy_id, Some(9));
    }
}
