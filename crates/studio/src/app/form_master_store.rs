//! Shared, durable master import registry. Paths select a mount alias; the
//! randomly assigned FileId (not that path/hash) is its permanent identity.
use super::{
    collision_recipes::digest,
    form_database::*,
    lck_project::{new_project_id, MasterDependency},
};
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
#[derive(Clone, Serialize, Deserialize)]
struct Manifest {
    format: String,
    master: MasterDependency,
    snapshot: MasterSnapshot,
}
#[derive(Clone)]
pub(super) struct MasterStore {
    path: PathBuf,
    bytes: Vec<u8>,
    manifest: Manifest,
}
impl MasterStore {
    pub fn default_root() -> Result<PathBuf> {
        Ok(
            if let Some(root) = std::env::var_os("LCK_MASTER_REGISTRY_DIR") {
                root.into()
            } else {
                dirs::data_local_dir()
                    .context("Local metadata directory unavailable")?
                    .join("LegaiaStudio")
                    .join("FormMasters")
            },
        )
    }
    pub fn open(root: &Path, source: &Path, actual_hash: &str) -> Result<Self> {
        let canonical = std::fs::canonicalize(source).unwrap_or_else(|_| source.into());
        let alias = root.join(format!(
            "alias-{}.json",
            digest(canonical.to_string_lossy().to_lowercase().as_bytes())
        ));
        if alias.exists() {
            let file: FileId = serde_json::from_slice(&std::fs::read(&alias)?)?;
            return Self::read(&root.join(format!("{}.lckmaster", file.as_str())));
        }
        let file = FileId::new(format!("master-{}", new_project_id()?.as_str()))?;
        let db = FormDatabase::new(file.clone(), new_project_id()?)?;
        let master = MasterDependency {
            file: file.clone(),
            disc_sha256: actual_hash.into(),
            path_hint: Some(source.into()),
        };
        let manifest = Manifest {
            format: "LCK_MASTER_V1".into(),
            master,
            snapshot: db.master_snapshot(),
        };
        let path = root.join(format!("{}.lckmaster", file.as_str()));
        let bytes = serde_json::to_vec_pretty(&manifest)?;
        crate::asset_catalog::atomic_write(&path, &bytes, None).map_err(anyhow::Error::msg)?;
        // On an alias race keep both complete manifests; never replace the
        // winner or recycle either namespace. Caller may retry the chosen alias.
        crate::asset_catalog::atomic_write(&alias, &serde_json::to_vec(&file)?, None)
            .map_err(anyhow::Error::msg)?;
        Ok(Self {
            path,
            bytes,
            manifest,
        })
    }
    pub fn mount_project(
        root: &Path,
        master: &MasterDependency,
        db: &FormDatabase,
    ) -> Result<Self> {
        let path = root.join(format!("{}.lckmaster", master.file.as_str()));
        if path.exists() {
            let store = Self::read(&path)?;
            ensure!(
                store.manifest.master.disc_sha256 == master.disc_sha256,
                "Selected logical master has different source evidence"
            );
            Ok(store)
        } else {
            let manifest = Manifest {
                format: "LCK_MASTER_V1".into(),
                master: master.clone(),
                snapshot: db.master_snapshot(),
            };
            let bytes = serde_json::to_vec_pretty(&manifest)?;
            crate::asset_catalog::atomic_write(&path, &bytes, None).map_err(anyhow::Error::msg)?;
            Ok(Self {
                path,
                bytes,
                manifest,
            })
        }
    }
    fn read(path: &Path) -> Result<Self> {
        let bytes = std::fs::read(path)?;
        let manifest: Manifest = serde_json::from_slice(&bytes)?;
        ensure!(
            manifest.format == "LCK_MASTER_V1" && manifest.master.file == manifest.snapshot.file,
            "Invalid master registry manifest"
        );
        let mut db = FormDatabase::new(manifest.master.file.clone(), new_project_id()?)?;
        db.mount_master_snapshot(&manifest.snapshot)?;
        Ok(Self {
            path: path.into(),
            bytes,
            manifest,
        })
    }
    pub fn master(&self) -> &MasterDependency {
        &self.manifest.master
    }
    pub fn refresh(&mut self, db: &mut FormDatabase) -> Result<()> {
        let latest = Self::read(&self.path)?;
        db.mount_master_snapshot(&latest.manifest.snapshot)?;
        *self = latest;
        Ok(())
    }
    pub fn commit(&mut self, db: &FormDatabase) -> Result<()> {
        let snapshot = db.master_snapshot();
        if snapshot == self.manifest.snapshot {
            return Ok(());
        }
        let mut manifest = self.manifest.clone();
        manifest.snapshot = snapshot;
        let bytes = serde_json::to_vec_pretty(&manifest)?;
        crate::asset_catalog::atomic_write(&self.path, &bytes, Some(&self.bytes))
            .map_err(anyhow::Error::msg)?;
        self.manifest = manifest;
        self.bytes = bytes;
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn projects_share_registry_ids_regardless_of_visit_order_and_concurrent_commit_is_rejected(
    ) -> Result<()> {
        let dir = tempfile::tempdir()?;
        let source = dir.path().join("master.img");
        std::fs::write(&source, b"fingerprint fixture")?;
        let root = dir.path().join("registries");
        let mut a = MasterStore::open(&root, &source, &"a".repeat(64))?;
        let mut db = FormDatabase::new(a.master().file.clone(), new_project_id()?)?;
        a.refresh(&mut db)?;
        let dummy = FormKey::new(db.master_file.clone(), 1)?;
        let house = db.transaction(|d| {
            d.import_master_base(
                "town01-house".into(),
                BaseForm::empty(dummy.clone(), "House".into(), FormKind::StaticEnvironment),
            )
        })?;
        a.commit(&db)?;
        let mut b = MasterStore::open(&root, &source, &"a".repeat(64))?;
        let mut db2 = FormDatabase::new(b.master().file.clone(), new_project_id()?)?;
        b.refresh(&mut db2)?;
        let conk = db2.transaction(|d| {
            d.import_master_base(
                "conc-architecture".into(),
                BaseForm::empty(dummy, "Conkram".into(), FormKind::StaticEnvironment),
            )
        })?;
        b.commit(&db2)?;
        a.refresh(&mut db)?;
        ensure!(
            db.imported_key(&db.master_file, "town01-house") == Some(&house)
                && db.imported_key(&db.master_file, "conc-architecture") == Some(&conk),
            "Master identities depend on visit order"
        );
        ensure!(
            a.master().file == b.master().file,
            "Source alias changed logical file identity"
        );
        let before = std::fs::read(&a.path)?;
        std::fs::write(&a.path, &before)?;
        let mut stale = a.clone();
        let mut fresh = db.clone();
        fresh.transaction(|d| {
            d.import_master_base(
                "later-source".into(),
                BaseForm::empty(
                    FormKey::new(d.master_file.clone(), 1)?,
                    "Later".into(),
                    FormKind::Texture,
                ),
            )?;
            Ok(())
        })?;
        a.commit(&fresh)?;
        db.transaction(|d| {
            d.import_master_base(
                "concurrent-source".into(),
                BaseForm::empty(
                    FormKey::new(d.master_file.clone(), 1)?,
                    "Other".into(),
                    FormKind::Texture,
                ),
            )?;
            Ok(())
        })?;
        ensure!(
            stale.commit(&db).is_err(),
            "Concurrent allocation overwrote canonical registry"
        );
        Ok(())
    }
}
