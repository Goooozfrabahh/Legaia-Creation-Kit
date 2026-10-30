//! Persistent authoring records. Native offsets, hashes and render indices are
//! provenance/adapter data, never the identity of an editable form.
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub(super) struct FileId(String);
impl FileId {
    pub fn new(value: impl Into<String>) -> Result<Self> {
        let value = value.into();
        ensure!(
            !value.is_empty()
                && value.len() <= 128
                && value
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b)),
            "Invalid logical file identity"
        );
        Ok(Self(value))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(super) struct FormKey {
    pub file: FileId,
    pub local_id: u32,
}
impl FormKey {
    pub fn new(file: FileId, local_id: u32) -> Result<Self> {
        ensure!(
            (1..=0x00ff_ffff).contains(&local_id),
            "Local FormID outside 24-bit namespace"
        );
        Ok(Self { file, local_id })
    }
    pub fn display_id(&self, load_order: &[FileId]) -> Result<u32> {
        let index = load_order
            .iter()
            .position(|f| f == &self.file)
            .context("File absent from load order")?;
        ensure!(
            load_order.len() <= 256
                && load_order.iter().collect::<BTreeSet<_>>().len() == load_order.len(),
            "Invalid file load order"
        );
        Ok(((index as u32) << 24) | self.local_id)
    }
}
impl Serialize for FormKey {
    fn serialize<S: serde::Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.serialize_str(&format!("{}:{:06X}", self.file.0, self.local_id))
    }
}
impl<'de> Deserialize<'de> for FormKey {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        let (f, id) = s
            .rsplit_once(':')
            .ok_or_else(|| serde::de::Error::custom("Invalid FormKey"))?;
        let file = FileId::new(f).map_err(serde::de::Error::custom)?;
        let id = u32::from_str_radix(id, 16).map_err(serde::de::Error::custom)?;
        Self::new(file, id).map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) enum FormKind {
    StaticEnvironment,
    Actor,
    Item,
    Terrain,
    Texture,
    Material,
    Script,
    Animation,
    Cell,
    Other { family: String, placeable: bool },
}
impl FormKind {
    pub fn placeable(&self) -> bool {
        matches!(
            self,
            Self::StaticEnvironment
                | Self::Actor
                | Self::Item
                | Self::Terrain
                | Self::Other {
                    placeable: true,
                    ..
                }
        )
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(super) enum Property {
    Bool(bool),
    Integer(i64),
    Number(f64),
    Text(String),
    Vector([f32; 3]),
    Form(FormKey),
    Forms(Vec<FormKey>),
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct NativeOrigin {
    pub source_identity: String,
    pub scene: Option<String>,
    pub prot_entry: Option<u32>,
    pub descriptor: Option<u16>,
    pub man_record: Option<u32>,
    pub anchor: Option<[u8; 2]>,
    pub cell: Option<[u8; 2]>,
    pub fingerprints: BTreeMap<String, String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct ResourceReference {
    pub identity: String,
    pub family: String,
    pub form: Option<FormKey>,
    pub origin: Option<NativeOrigin>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Dependency {
    pub target: FormKey,
    pub role: String,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(super) struct BaseForm {
    pub key: FormKey,
    pub editor_id: String,
    pub kind: FormKind,
    pub resources: Vec<ResourceReference>,
    pub materials: BTreeMap<String, Property>,
    pub collision: BTreeMap<String, Property>,
    pub behavior: Vec<Dependency>,
    pub origin: Option<NativeOrigin>,
    pub properties: BTreeMap<String, Property>,
    pub dependency_coverage_complete: bool,
    pub revision: u64,
}
impl BaseForm {
    pub fn empty(key: FormKey, editor_id: String, kind: FormKind) -> Self {
        Self {
            key,
            editor_id,
            kind,
            resources: vec![],
            materials: Default::default(),
            collision: Default::default(),
            behavior: vec![],
            origin: None,
            properties: Default::default(),
            dependency_coverage_complete: false,
            revision: 0,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub(super) struct Transform {
    pub position: [f32; 3],
    pub rotation: [f32; 3],
    pub scale: [f32; 3],
}
impl Default for Transform {
    fn default() -> Self {
        Self {
            position: [0.; 3],
            rotation: [0.; 3],
            scale: [1.; 3],
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(super) struct PlacedReference {
    pub key: FormKey,
    pub base: FormKey,
    pub cell: FormKey,
    pub transform: Transform,
    pub editor_id: Option<String>,
    pub disabled: bool,
    pub deleted: bool,
    pub flags: BTreeSet<String>,
    pub properties: BTreeMap<String, Property>,
    pub origin: Option<NativeOrigin>,
    /// Stable ownership handles. Existing hash-bound IMG sidecars remain authoritative.
    pub collision_owners: BTreeSet<String>,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub(super) struct Layer {
    pub bases: BTreeMap<FormKey, BaseForm>,
    pub references: BTreeMap<FormKey, PlacedReference>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct ImportRegistration {
    pub source_file: FileId,
    pub source_identity: String,
    pub kind: FormKind,
    pub key: FormKey,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub(super) struct FormIdAllocator {
    next: BTreeMap<FileId, u32>,
}
impl FormIdAllocator {
    pub fn allocate(&mut self, file: &FileId) -> Result<FormKey> {
        let next = self.next.entry(file.clone()).or_insert(0x800);
        let key = FormKey::new(file.clone(), *next)?;
        *next = next.checked_add(1).context("FormID allocation overflow")?;
        Ok(key)
    }
    pub fn reserve(&mut self, key: &FormKey) {
        let n = self.next.entry(key.file.clone()).or_insert(0x800);
        *n = (*n).max(key.local_id + 1);
    }
    fn retain_high_water(&mut self, other: &Self) {
        for (f, n) in &other.next {
            let v = self.next.entry(f.clone()).or_insert(*n);
            *v = (*v).max(*n);
        }
    }
    pub fn validate(&self) -> Result<()> {
        for (file, n) in &self.next {
            FileId::new(file.0.clone())?;
            ensure!((1..=0x1000000).contains(n), "Invalid allocator state");
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Default)]
pub(super) struct Indexes {
    pub base_references: BTreeMap<FormKey, BTreeSet<FormKey>>,
    pub cell_references: BTreeMap<FormKey, BTreeSet<FormKey>>,
    pub users: BTreeMap<FormKey, BTreeSet<FormKey>>,
    pub editor_ids: BTreeMap<String, FormKey>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct FormDatabase {
    pub(super) master_file: FileId,
    pub(super) active_file: FileId,
    master: Layer,
    active: Layer,
    pub(super) allocator: FormIdAllocator,
    registry: Vec<ImportRegistration>,
    #[serde(skip)]
    indexes: Indexes,
}
#[derive(Clone, Debug)]
pub(super) struct ResolvedReference {
    pub reference: PlacedReference,
    pub base: BaseForm,
    pub appearance_revision: u64,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct UseInfo {
    pub count: usize,
    pub users: BTreeSet<FormKey>,
    pub dependency_coverage_complete: bool,
}
impl FormDatabase {
    pub fn new(master_file: FileId, active_file: FileId) -> Result<Self> {
        ensure!(
            master_file != active_file,
            "Master and active file must differ"
        );
        Ok(Self {
            master_file,
            active_file,
            master: Default::default(),
            active: Default::default(),
            allocator: Default::default(),
            registry: vec![],
            indexes: Default::default(),
        })
    }
    pub fn master_layer(&self) -> &Layer {
        &self.master
    }
    pub fn active_layer(&self) -> &Layer {
        &self.active
    }
    pub fn registry(&self) -> &[ImportRegistration] {
        &self.registry
    }
    pub fn indexes(&self) -> &Indexes {
        &self.indexes
    }
    pub fn resolve_base(&self, key: &FormKey) -> Option<&BaseForm> {
        self.active
            .bases
            .get(key)
            .or_else(|| self.master.bases.get(key))
    }
    pub fn reference(&self, key: &FormKey) -> Option<&PlacedReference> {
        self.active
            .references
            .get(key)
            .or_else(|| self.master.references.get(key))
    }
    pub fn bases(&self) -> BTreeMap<FormKey, &BaseForm> {
        let mut all = self
            .master
            .bases
            .iter()
            .map(|(k, v)| (k.clone(), v))
            .collect::<BTreeMap<_, _>>();
        all.extend(self.active.bases.iter().map(|(k, v)| (k.clone(), v)));
        all
    }
    pub fn references(&self) -> BTreeMap<FormKey, &PlacedReference> {
        let mut all = self
            .master
            .references
            .iter()
            .map(|(k, v)| (k.clone(), v))
            .collect::<BTreeMap<_, _>>();
        all.extend(self.active.references.iter().map(|(k, v)| (k.clone(), v)));
        all
    }
    pub fn resolve_reference(&self, key: &FormKey) -> Result<ResolvedReference> {
        let r = self.reference(key).context("Reference missing")?;
        let b = self.resolve_base(&r.base).context("Base missing")?;
        Ok(ResolvedReference {
            reference: r.clone(),
            base: b.clone(),
            appearance_revision: b.revision,
        })
    }
    pub fn use_info(&self, key: &FormKey) -> UseInfo {
        UseInfo {
            count: self
                .indexes
                .base_references
                .get(key)
                .map_or(0, BTreeSet::len),
            users: self.indexes.users.get(key).cloned().unwrap_or_default(),
            dependency_coverage_complete: self
                .resolve_base(key)
                .is_some_and(|b| b.dependency_coverage_complete),
        }
    }
    /// Clone/validate/commit is the atomic mutation boundary. A caller can group
    /// many low-level operations in one transaction (including multi-selection).
    pub fn transaction<T>(&mut self, edit: impl FnOnce(&mut Self) -> Result<T>) -> Result<T> {
        let mut next = self.clone();
        let value = edit(&mut next)?;
        next.validate()?;
        *self = next;
        Ok(value)
    }
    pub fn create_base(&mut self, editor_id: String, kind: FormKind) -> Result<FormKey> {
        let key = self.allocator.allocate(&self.active_file)?;
        self.active
            .bases
            .insert(key.clone(), BaseForm::empty(key.clone(), editor_id, kind));
        Ok(key)
    }
    pub fn duplicate_base(&mut self, key: &FormKey, editor_id: String) -> Result<FormKey> {
        let mut b = self.resolve_base(key).context("Base missing")?.clone();
        let key = self.allocator.allocate(&self.active_file)?;
        b.key = key.clone();
        b.editor_id = editor_id;
        b.origin = None;
        b.revision = 0;
        self.active.bases.insert(key.clone(), b);
        Ok(key)
    }
    pub fn edit_base(
        &mut self,
        key: &FormKey,
        edit: impl FnOnce(&mut BaseForm) -> Result<()>,
    ) -> Result<()> {
        let mut b = self.resolve_base(key).context("Base missing")?.clone();
        let kind = b.kind.clone();
        edit(&mut b)?;
        ensure!(
            &b.key == key && b.kind == kind,
            "Base identity/category cannot be changed by property editing"
        );
        b.revision = b
            .revision
            .checked_add(1)
            .context("Base revision exhausted")?;
        self.active.bases.insert(key.clone(), b);
        Ok(())
    }
    pub fn place_reference(
        &mut self,
        base: FormKey,
        cell: FormKey,
        transform: Transform,
    ) -> Result<FormKey> {
        ensure!(
            self.resolve_base(&base)
                .context("Base missing")?
                .kind
                .placeable(),
            "Asset record is not a placeable category"
        );
        ensure!(
            self.resolve_base(&cell)
                .is_some_and(|b| b.kind == FormKind::Cell),
            "Destination is not a Cell form"
        );
        let key = self.allocator.allocate(&self.active_file)?;
        self.active.references.insert(
            key.clone(),
            PlacedReference {
                key: key.clone(),
                base,
                cell,
                transform,
                editor_id: None,
                disabled: false,
                deleted: false,
                flags: Default::default(),
                properties: Default::default(),
                origin: None,
                collision_owners: Default::default(),
            },
        );
        Ok(key)
    }
    pub fn duplicate_reference(&mut self, key: &FormKey) -> Result<FormKey> {
        let mut r = self.reference(key).context("Reference missing")?.clone();
        ensure!(!r.deleted, "Cannot duplicate deleted Reference");
        let key = self.allocator.allocate(&self.active_file)?;
        r.key = key.clone();
        r.origin = None;
        r.deleted = false;
        if let Some(id) = &r.editor_id {
            r.editor_id = Some(format!("{id}_Copy_{:06X}", key.local_id));
        }
        r.collision_owners.clear();
        self.active.references.insert(key.clone(), r);
        Ok(key)
    }
    pub fn edit_reference(
        &mut self,
        key: &FormKey,
        edit: impl FnOnce(&mut PlacedReference) -> Result<()>,
    ) -> Result<()> {
        let mut r = self.reference(key).context("Reference missing")?.clone();
        edit(&mut r)?;
        ensure!(&r.key == key, "Reference identity cannot change");
        self.active.references.insert(key.clone(), r);
        Ok(())
    }
    pub fn move_reference(&mut self, key: &FormKey, transform: Transform) -> Result<()> {
        self.edit_reference(key, |r| {
            r.transform = transform;
            Ok(())
        })
    }
    pub fn delete_reference(&mut self, key: &FormKey) -> Result<()> {
        self.edit_reference(key, |r| {
            r.deleted = true;
            Ok(())
        })
    }
    pub fn restore_reference(&mut self, key: &FormKey) -> Result<()> {
        self.edit_reference(key, |r| {
            r.deleted = false;
            Ok(())
        })
    }
    pub fn change_reference_cell(&mut self, key: &FormKey, cell: FormKey) -> Result<()> {
        self.edit_reference(key, |r| {
            r.cell = cell;
            Ok(())
        })
    }
    pub fn change_reference_base(&mut self, key: &FormKey, base: FormKey) -> Result<()> {
        self.edit_reference(key, |r| {
            r.base = base;
            Ok(())
        })
    }
    pub fn imported_key(&self, source_file: &FileId, identity: &str) -> Option<&FormKey> {
        self.registry
            .iter()
            .find(|r| &r.source_file == source_file && r.source_identity == identity)
            .map(|r| &r.key)
    }
    pub fn import_master_base(&mut self, identity: String, mut base: BaseForm) -> Result<FormKey> {
        if let Some(key) = self.imported_key(&self.master_file, &identity) {
            ensure!(
                self.master.bases.contains_key(key),
                "Registry category conflict"
            );
            return Ok(key.clone());
        }
        let key = self.allocator.allocate(&self.master_file)?;
        base.key = key.clone();
        self.registry.push(ImportRegistration {
            source_file: self.master_file.clone(),
            source_identity: identity,
            kind: base.kind.clone(),
            key: key.clone(),
        });
        self.master.bases.insert(key.clone(), base);
        Ok(key)
    }
    pub fn import_master_reference(
        &mut self,
        identity: String,
        mut r: PlacedReference,
    ) -> Result<FormKey> {
        if let Some(key) = self.imported_key(&self.master_file, &identity) {
            ensure!(
                self.master.references.contains_key(key),
                "Registry category conflict"
            );
            return Ok(key.clone());
        }
        let key = self.allocator.allocate(&self.master_file)?;
        r.key = key.clone();
        self.registry.push(ImportRegistration {
            source_file: self.master_file.clone(),
            source_identity: identity,
            kind: self
                .resolve_base(&r.base)
                .context("Import base missing")?
                .kind
                .clone(),
            key: key.clone(),
        });
        self.master.references.insert(key.clone(), r);
        Ok(key)
    }
    pub fn validate(&mut self) -> Result<()> {
        FileId::new(self.master_file.0.clone())?;
        FileId::new(self.active_file.0.clone())?;
        ensure!(
            self.master_file != self.active_file,
            "Master/active identity collision"
        );
        self.allocator.validate()?;
        for layer in [&self.master, &self.active] {
            for (k, b) in &layer.bases {
                ensure!(k == &b.key, "Base map/key mismatch");
            }
            for (k, r) in &layer.references {
                ensure!(k == &r.key, "Reference map/key mismatch");
            }
        }
        ensure!(
            self.master
                .bases
                .keys()
                .chain(self.master.references.keys())
                .all(|k| k.file == self.master_file),
            "Master record in another namespace"
        );
        ensure!(
            self.active
                .bases
                .keys()
                .chain(self.active.references.keys())
                .all(|k| k.file == self.active_file
                    || self.master.bases.contains_key(k)
                    || self.master.references.contains_key(k)),
            "Override without master or active ownership"
        );
        for r in self.master.references.values() {
            ensure!(
                self.master.bases.contains_key(&r.base)
                    && self
                        .master
                        .bases
                        .get(&r.cell)
                        .is_some_and(|b| b.kind == FormKind::Cell),
                "Read-only master cannot depend on active project forms"
            );
        }
        let bases = self.bases();
        let refs = self.references();
        ensure!(
            bases.keys().all(|k| !refs.contains_key(k)),
            "Base and Reference share FormKey"
        );
        let mut idx = Indexes::default();
        fn editor(idx: &mut Indexes, id: &str, key: &FormKey) -> Result<()> {
            ensure!(
                !id.trim().is_empty() && id.len() <= 256 && !id.chars().any(char::is_control),
                "Invalid EditorID"
            );
            ensure!(
                idx.editor_ids
                    .insert(id.to_lowercase(), key.clone())
                    .is_none(),
                "EditorID is not globally unique"
            );
            Ok(())
        }
        let mut dependencies = Vec::new();
        for (k, b) in &bases {
            editor(&mut idx, &b.editor_id, k)?;
            for p in b
                .properties
                .values()
                .chain(b.materials.values())
                .chain(b.collision.values())
            {
                property_dependencies(k, p, &mut dependencies)?;
            }
            for r in &b.resources {
                if let Some(target) = &r.form {
                    dependencies.push((k.clone(), target.clone()));
                }
            }
            for d in &b.behavior {
                dependencies.push((k.clone(), d.target.clone()));
            }
        }
        for (k, r) in &refs {
            ensure!(
                bases.get(&r.base).is_some_and(|b| b.kind.placeable()),
                "Reference base absent or non-placeable"
            );
            ensure!(
                bases.get(&r.cell).is_some_and(|b| b.kind == FormKind::Cell),
                "Reference Cell absent"
            );
            ensure!(
                r.transform
                    .position
                    .iter()
                    .chain(&r.transform.rotation)
                    .chain(&r.transform.scale)
                    .all(|v| v.is_finite())
                    && r.transform.scale.iter().all(|v| *v != 0.),
                "Invalid Reference transform"
            );
            if let Some(id) = &r.editor_id {
                editor(&mut idx, id, k)?;
            }
            for p in r.properties.values() {
                property_dependencies(k, p, &mut dependencies)?;
            }
            if !r.deleted {
                idx.base_references
                    .entry(r.base.clone())
                    .or_default()
                    .insert((*k).clone());
                idx.cell_references
                    .entry(r.cell.clone())
                    .or_default()
                    .insert((*k).clone());
                dependencies.extend([(k.clone(), r.base.clone()), (k.clone(), r.cell.clone())]);
            }
        }
        for (from, to) in dependencies {
            ensure!(
                bases.contains_key(&to) || refs.contains_key(&to),
                "Unresolved form dependency {to:?}"
            );
            idx.users.entry(to).or_default().insert(from);
        }
        let mut identities = BTreeSet::new();
        let mut targets = BTreeSet::new();
        for entry in &self.registry {
            ensure!(
                identities.insert((&entry.source_file, &entry.source_identity))
                    && targets.insert(&entry.key),
                "Duplicate import registry mapping"
            );
            ensure!(
                self.master.bases.contains_key(&entry.key)
                    || self.master.references.contains_key(&entry.key),
                "Registry target absent"
            );
        }
        for key in bases.keys().chain(refs.keys()) {
            ensure!(
                self.allocator
                    .next
                    .get(&key.file)
                    .is_some_and(|n| *n > key.local_id),
                "Allocator would reuse an existing form identity"
            );
        }
        self.indexes = idx;
        Ok(())
    }
}
fn property_dependencies(
    from: &FormKey,
    p: &Property,
    out: &mut Vec<(FormKey, FormKey)>,
) -> Result<()> {
    match p {
        Property::Form(k) => out.push((from.clone(), k.clone())),
        Property::Forms(keys) => out.extend(keys.iter().map(|k| (from.clone(), k.clone()))),
        Property::Number(v) => ensure!(v.is_finite(), "Non-finite property"),
        Property::Vector(v) => ensure!(v.iter().all(|f| f.is_finite()), "Non-finite vector"),
        _ => {}
    }
    Ok(())
}

#[derive(Clone, Default)]
pub(super) struct CommandHistory {
    undo: Vec<FormDatabase>,
    redo: Vec<FormDatabase>,
}
impl CommandHistory {
    pub fn execute<T>(
        &mut self,
        db: &mut FormDatabase,
        edit: impl FnOnce(&mut FormDatabase) -> Result<T>,
    ) -> Result<T> {
        let before = db.clone();
        let value = db.transaction(edit)?;
        self.undo.push(before);
        self.redo.clear();
        Ok(value)
    }
    pub fn undo(&mut self, db: &mut FormDatabase) -> Result<bool> {
        let Some(mut before) = self.undo.last().cloned() else {
            return Ok(false);
        };
        before.allocator.retain_high_water(&db.allocator);
        before.validate()?;
        self.undo.pop();
        self.redo.push(db.clone());
        *db = before;
        Ok(true)
    }
    pub fn redo(&mut self, db: &mut FormDatabase) -> Result<bool> {
        let Some(mut next) = self.redo.last().cloned() else {
            return Ok(false);
        };
        next.allocator.retain_high_water(&db.allocator);
        next.validate()?;
        self.redo.pop();
        self.undo.push(db.clone());
        *db = next;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn db() -> FormDatabase {
        FormDatabase::new(
            FileId::new("retail-SCUS94254").unwrap(),
            FileId::new("project-test").unwrap(),
        )
        .unwrap()
    }
    fn setup() -> (FormDatabase, FormKey, FormKey, FormKey) {
        let mut d = db();
        let keys = d
            .transaction(|d| {
                let house = d.create_base("House".into(), FormKind::StaticEnvironment)?;
                let rim = d.create_base("RimElm".into(), FormKind::Cell)?;
                let conk = d.create_base("Conkram".into(), FormKind::Cell)?;
                Ok((house, rim, conk))
            })
            .unwrap();
        (d, keys.0, keys.1, keys.2)
    }
    #[test]
    fn stable_namespaces_are_independent_of_load_order_and_roundtrip_as_map_keys() {
        let k = FormKey::new(FileId::new("plugin-test").unwrap(), 0x123).unwrap();
        let order = vec![FileId::new("retail").unwrap(), k.file.clone()];
        assert_eq!(k.display_id(&order).unwrap(), 0x01000123);
        assert_eq!(k.display_id(&[k.file.clone()]).unwrap(), 0x123);
        let map = BTreeMap::from([(k.clone(), 7)]);
        let out: BTreeMap<FormKey, i32> =
            serde_json::from_slice(&serde_json::to_vec(&map).unwrap()).unwrap();
        assert_eq!(out[&k], 7);
        assert!(FormKey::new(k.file.clone(), 0x1000000).is_err());
    }
    #[test]
    fn base_edit_refreshes_shared_appearance_without_touching_independent_trs() {
        let (mut d, b, rim, conk) = setup();
        let (a, b2) = d
            .transaction(|d| {
                let a = d.place_reference(b.clone(), rim, Transform::default())?;
                let b = d.duplicate_reference(&a)?;
                d.change_reference_cell(&b, conk)?;
                d.move_reference(
                    &b,
                    Transform {
                        position: [10., 20., 30.],
                        scale: [1., 2., 3.],
                        ..Default::default()
                    },
                )?;
                Ok((a, b))
            })
            .unwrap();
        d.transaction(|d| {
            d.edit_base(&b, |b| {
                b.materials
                    .insert("material".into(), Property::Text("new".into()));
                Ok(())
            })
        })
        .unwrap();
        assert_eq!(d.resolve_reference(&a).unwrap().appearance_revision, 1);
        assert_eq!(
            d.resolve_reference(&b2).unwrap().base.materials,
            d.resolve_reference(&a).unwrap().base.materials
        );
        assert_eq!(d.reference(&a).unwrap().transform.position, [0.; 3]);
        assert_eq!(d.reference(&b2).unwrap().transform.scale, [1., 2., 3.]);
        assert_eq!(d.use_info(&b).count, 2);
    }
    #[test]
    fn duplicate_definition_is_independent_and_equal_resources_do_not_merge_bases() {
        let (mut d, b, _, _) = setup();
        let new = d
            .transaction(|d| d.duplicate_base(&b, "HouseCopy".into()))
            .unwrap();
        assert_ne!(new, b);
        d.transaction(|d| {
            d.edit_base(&new, |b| {
                b.properties.insert("value".into(), Property::Integer(2));
                Ok(())
            })
        })
        .unwrap();
        assert!(d.resolve_base(&b).unwrap().properties.is_empty());
    }
    #[test]
    fn grouped_undo_redo_restores_keys_and_collision_owners_without_reusing_allocations() {
        let (mut d, b, rim, _) = setup();
        let mut h = CommandHistory::default();
        let a = h
            .execute(&mut d, |d| {
                d.place_reference(b.clone(), rim, Transform::default())
            })
            .unwrap();
        h.execute(&mut d, |d| {
            d.edit_reference(&a, |r| {
                r.collision_owners.insert("owned-wall-1".into());
                Ok(())
            })
        })
        .unwrap();
        let dup = h.execute(&mut d, |d| d.duplicate_reference(&a)).unwrap();
        assert!(d.reference(&dup).unwrap().collision_owners.is_empty());
        h.undo(&mut d).unwrap();
        assert!(d.reference(&dup).is_none());
        h.redo(&mut d).unwrap();
        assert_eq!(d.reference(&dup).unwrap().key, dup);
        h.execute(&mut d, |d| d.delete_reference(&a)).unwrap();
        assert_eq!(d.use_info(&b).count, 1);
        h.undo(&mut d).unwrap();
        assert!(d
            .reference(&a)
            .unwrap()
            .collision_owners
            .contains("owned-wall-1"));
        h.redo(&mut d).unwrap();
        assert!(d.reference(&a).unwrap().deleted);
        h.undo(&mut d).unwrap();
        h.undo(&mut d).unwrap();
        let later = h.execute(&mut d, |d| d.duplicate_reference(&a)).unwrap();
        assert!(later.local_id > dup.local_id);
    }
    #[test]
    fn deletion_disable_and_master_override_are_distinct() {
        let mut d = db();
        let dummy = FormKey::new(d.master_file.clone(), 1).unwrap();
        let (b, rim) = d
            .transaction(|d| {
                let b = d.import_master_base(
                    "town01:house-base".into(),
                    BaseForm::empty(
                        dummy.clone(),
                        "RetailHouse".into(),
                        FormKind::StaticEnvironment,
                    ),
                )?;
                let cell = d.import_master_base(
                    "town01:cell".into(),
                    BaseForm::empty(dummy.clone(), "RetailRimElm".into(), FormKind::Cell),
                )?;
                Ok((b, cell))
            })
            .unwrap();
        let r = PlacedReference {
            key: dummy,
            base: b.clone(),
            cell: rim,
            transform: Default::default(),
            editor_id: None,
            disabled: false,
            deleted: false,
            flags: Default::default(),
            properties: Default::default(),
            origin: None,
            collision_owners: Default::default(),
        };
        let a = d
            .transaction(|d| d.import_master_reference("town01:original-house-reference".into(), r))
            .unwrap();
        let master = d.master_layer().clone();
        d.transaction(|d| {
            d.edit_reference(&a, |r| {
                r.disabled = true;
                Ok(())
            })
        })
        .unwrap();
        assert_eq!(d.use_info(&b).count, 1);
        d.transaction(|d| d.delete_reference(&a)).unwrap();
        assert_eq!(d.use_info(&b).count, 0);
        assert!(d.reference(&a).unwrap().disabled);
        assert_eq!(d.master_layer(), &master);
        assert!(d.active_layer().references[&a].deleted);
        d.transaction(|d| d.restore_reference(&a)).unwrap();
        assert_eq!(d.reference(&a).unwrap().key, a);
    }
    #[test]
    fn transaction_failure_keeps_database_allocator_and_indexes_unchanged() {
        let (mut d, b, rim, _) = setup();
        let before = serde_json::to_vec(&d).unwrap();
        assert!(d
            .transaction(|d| {
                d.place_reference(b, rim, Transform::default())?;
                d.create_base("House".into(), FormKind::Texture)?;
                Ok(())
            })
            .is_err());
        assert_eq!(serde_json::to_vec(&d).unwrap(), before);
    }
    #[test]
    fn nonplaceable_assets_remain_valid_records_and_use_counts_are_explicitly_incomplete() {
        let (mut d, _, rim, _) = setup();
        let tex = d
            .transaction(|d| d.create_base("TextureAsset".into(), FormKind::Texture))
            .unwrap();
        assert!(d
            .transaction(|d| d.place_reference(tex.clone(), rim, Transform::default()))
            .is_err());
        assert!(!d.use_info(&tex).dependency_coverage_complete);
        assert_eq!(d.use_info(&tex).count, 0);
    }
}
