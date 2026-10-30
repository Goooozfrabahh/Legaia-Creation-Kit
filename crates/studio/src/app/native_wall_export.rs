//! Hash-bound persistence/reconciliation of actual editor wall contributions.
//! No missing journal permits inferred removal of original native collision.
use super::native_asset_plan::scene_registration as reg;
use super::{
    collision_recipes::{digest, ObjectId},
    native_wall_journal::{self as walls, Journal, Reference},
    scene_instances::{NameTarget, SceneDocument},
    world_view::{self, WorldModel},
};
use anyhow::{ensure, Context, Result};
use legaia_disc_patch::disc::DiscPatcher;
use legaia_engine_scene::{scene::ProtIndex, scene_assembly::AssembledScene};
use legaia_scene_formats::field_objects::{self, ObjectRecord};
use std::{
    collections::{BTreeSet, HashMap},
    path::{Path, PathBuf},
};

pub(super) struct Snapshot {
    journal: Journal,
    source_map: Vec<u8>,
    source_hash: String,
    source_path: PathBuf,
    baseline_path: PathBuf,
    baseline_hash: String,
    baseline_map: Vec<u8>,
    resource_baseline: reg::ResourceBaseline,
    other_cells: Vec<serde_json::Value>,
}
fn hashes(disc: &DiscPatcher, entry: usize) -> Result<Vec<String>> {
    let b = disc.read_entry_footprint(entry + 3)?;
    let p = disc.read_entry_footprint(entry + 4)?;
    Ok(reg::native_model_pool(&b, Some(&p))
        .iter()
        .map(|m| digest(&m.bytes))
        .collect())
}
fn reference(map: &[u8], cell: (u8, u8), models: &[String]) -> Result<Reference> {
    ensure!(cell.0 < 128 && cell.1 < 128, "Collision reference bounds");
    let at = 0x8000 + (cell.1 as usize * 128 + cell.0 as usize) * 2;
    let id = u16::from_le_bytes(map.get(at..at + 2).context("Collision grid")?.try_into()?) & 511;
    ensure!(id > 3, "No native environment reference allocated");
    let rec = ObjectRecord::parse(map, id as usize).context("Collision descriptor")?;
    let index = field_objects::pack_mesh_index(id, &rec).context("Collision model selector")?;
    Ok(Reference {
        cell: [cell.0, cell.1],
        descriptor: id,
        model_selector: index,
        descriptor_sha256: digest(&map[id as usize * 32..id as usize * 32 + 32]),
        model_sha256: models
            .get(index as usize)
            .context("Collision native model missing")?
            .clone(),
    })
}

pub(super) fn capture(
    path: &Path,
    source_hash: &str,
    disc: &DiscPatcher,
    entry: usize,
    scene: &str,
) -> Result<Snapshot> {
    let map = disc.read_entry_footprint(entry)?;
    let file = path.with_extension("lck-collision.json");
    let limits = reg::Limits {
        max_models: 1024,
        max_decoded: 16 * 1024 * 1024,
        max_encoded: 16 * 1024 * 1024,
    };
    let mut other_cells = Vec::new();
    let mut cell = None;
    if file.exists() {
        ensure!(
            std::fs::metadata(&file)?.len() <= 64 * 1024 * 1024,
            "Collision journal metadata too large"
        );
        let mut root: serde_json::Value = serde_json::from_slice(&std::fs::read(&file)?)?;
        ensure!(
            root["image_sha256"].as_str() == Some(source_hash),
            "Collision sidecar belongs to another IMG revision"
        );
        let cells = if root["schema"] == "lck-native-wall-journal-v2" {
            root["cells"].as_array().context("Collision cells")?.clone()
        } else {
            ensure!(
                root["schema"] == "lck-native-wall-journal-v1",
                "Unknown collision metadata schema"
            );
            root.as_object_mut()
                .context("Collision metadata")?
                .remove("image_sha256");
            vec![root]
        };
        for value in cells {
            if value["journal"]["map_entry"].as_u64() == Some(entry as u64) {
                ensure!(cell.is_none(), "Duplicate collision cell journal");
                cell = Some(value);
            } else {
                other_cells.push(value);
            }
        }
    }
    let (journal, baseline_path, baseline_hash, baseline_map, resource_baseline) =
        if let Some(value) = cell {
            let mut j: Journal = serde_json::from_value(value["journal"].clone())?;
            j.scene = scene.into();
            j.validate(&map)?;
            let models = hashes(disc, entry)?;
            for owner in &j.owners {
                if let Some(r) = &owner.native {
                    ensure!(
                        models.get(r.model_selector as usize) == Some(&r.model_sha256),
                        "Collision model witness changed"
                    );
                }
            }
            let baseline = PathBuf::from(
                value["baseline_image"]
                    .as_str()
                    .context("Collision baseline path")?,
            );
            let hash = value["baseline_image_sha256"]
                .as_str()
                .context("Collision baseline hash")?
                .to_string();
            let fp = if !value["resource_baseline"].is_null() {
                serde_json::from_value(value["resource_baseline"].clone())?
            } else {
                // Migrate old verified journals once. New journals are self-contained.
                let bytes = std::fs::read(&baseline)?;
                ensure!(digest(&bytes) == hash, "Legacy baseline IMG changed");
                let base = DiscPatcher::open(bytes)?;
                let bm = base.read_entry_footprint(entry)?;
                ensure!(
                    j.baseline_high
                        == bm[0x4000..0x8000]
                            .iter()
                            .map(|v| v & 0xf0)
                            .collect::<Vec<_>>(),
                    "Legacy baseline wall mismatch"
                );
                reg::resource_baseline(
                    &base.read_entry_footprint(entry + 3)?,
                    Some(&base.read_entry_footprint(entry + 4)?),
                    limits,
                )?
            };
            let mut bm = map.clone();
            bm[0x4000..0x8000].copy_from_slice(&j.baseline_high);
            (j, baseline, hash, bm, fp)
        } else {
            let j = Journal::reconstruct(scene.into(), entry as u32, &map, &map, vec![])?;
            let fp = reg::resource_baseline(
                &disc.read_entry_footprint(entry + 3)?,
                Some(&disc.read_entry_footprint(entry + 4)?),
                limits,
            )?;
            (j, path.to_path_buf(), source_hash.into(), map.clone(), fp)
        };
    Ok(Snapshot {
        journal,
        source_map: map,
        source_hash: source_hash.into(),
        source_path: path.to_path_buf(),
        baseline_path,
        baseline_hash,
        baseline_map,
        resource_baseline,
        other_cells,
    })
}

/// Transactional: caller commits returned disc only if every ownership/replay
/// check passes. Existing unrestricted geometry/other writer attempts survive
/// an error; it must be reported as incomplete collision, never hidden success.
pub(super) fn reconcile(
    snapshot: Snapshot,
    disc: &DiscPatcher,
    scene: &AssembledScene,
    world: &WorldModel,
    doc: &SceneDocument,
    targets: &HashMap<NameTarget, (u8, u8)>,
    log: &mut Vec<String>,
) -> Result<(DiscPatcher, serde_json::Value)> {
    let entry = snapshot.journal.map_entry as usize;
    let current = disc.read_entry_footprint(entry)?;
    let model_hashes = hashes(disc, entry)?;
    snapshot.journal.validate(&snapshot.source_map)?;
    let source_key = |r: &Reference| {
        world
            .collision
            .sources
            .iter()
            .find(|(_, p)| p.object.cell == r.cell && p.object.descriptor_id == r.descriptor)
            .map(|(k, _)| *k)
    };
    let owner_id = |target: &ObjectId| -> String {
        match target {
            ObjectId::Created(uid) => format!("export-{}-created-{uid}", snapshot.source_hash),
            ObjectId::Native(id) => snapshot
                .journal
                .owners
                .iter()
                .find(|o| {
                    o.native
                        .as_ref()
                        .is_some_and(|r| r.cell == id.cell && r.descriptor == id.descriptor_id)
                })
                .map(|o| o.id.clone())
                .unwrap_or_else(|| {
                    format!(
                        "export-{}-native-{}-{}-{}",
                        snapshot.source_hash, id.cell[0], id.cell[1], id.descriptor_id
                    )
                }),
        }
    };
    let changed_keys = doc
        .moves
        .iter()
        .map(|(k, _)| *k)
        .chain(doc.hidden.iter().copied())
        .chain(
            doc.rotations
                .iter()
                .chain(&doc.scales)
                .filter_map(|(t, _)| {
                    if let NameTarget::Native(t, i) = t {
                        Some((*t, *i))
                    } else {
                        None
                    }
                }),
        )
        .collect::<BTreeSet<_>>();
    for key in changed_keys {
        if let Some(provenance) = world.collision.sources.get(&key) {
            if !snapshot.journal.owners.iter().any(|o| {
                o.native.as_ref().is_some_and(|r| {
                    r.cell == provenance.object.cell
                        && r.descriptor == provenance.object.descriptor_id
                })
            }) {
                log.push(format!("WARNING UNRESOLVED ORIGINAL COLLISION OWNERSHIP {key:?}: native geometry attempted; no recorded exclusive wall layer exists for this source. Original/shared wall data retained; no proximity-based removal."));
            }
        }
    }
    let mut written = vec![];
    for assignment in &doc.collision_recipes {
        let Some(draw) = world.draws.iter().find(|d| {
            world.collision.owner(world, (d.terrain, d.index)).as_ref() == Some(&assignment.owner)
        }) else {
            continue;
        };
        if draw.hidden {
            continue;
        }
        let mut owner =
            walls::serialized_owner(owner_id(&assignment.owner), &assignment.recipe, draw)?;
        let cell = match &assignment.owner {
            ObjectId::Created(uid) => targets.get(&NameTarget::Created(*uid)).copied(),
            ObjectId::Native(id) => Some((id.cell[0], id.cell[1])),
        };
        if let Some(cell) = cell {
            owner.native = reference(&current, cell, &model_hashes).ok();
        }
        written.push(owner);
    }
    // Replay what the existing additive writer actually did, BEFORE replacing
    // any source layer. Rename appended observations to keep identities unique.
    let mut observed = snapshot.journal.owners.clone();
    for o in &mut observed {
        o.native = None;
    }
    for (i, o) in written.iter().enumerate() {
        let mut o = o.clone();
        o.id = format!("current-write-{i}");
        o.native = None;
        observed.push(o);
    }
    Journal::reconstruct(
        scene.name.clone(),
        entry as u32,
        &snapshot.baseline_map,
        &current,
        observed,
    )?;
    let mut desired = snapshot.journal.clone();
    let mut retired = BTreeSet::new();
    let mut replacements = 0usize;
    // Reparse final native bytes so moved/rotated mesh collision uses encoded
    // PS1 positions/angles rather than unsaved editor floats or nearby meshes.
    let need_mesh = desired.owners.iter().any(|o| {
        o.native.as_ref().and_then(source_key).is_some_and(|k| {
            doc.moves.iter().any(|(a, _)| *a == k)
                || doc
                    .rotations
                    .iter()
                    .any(|(a, _)| *a == NameTarget::Native(k.0, k.1))
                || doc
                    .scales
                    .iter()
                    .any(|(a, _)| *a == NameTarget::Native(k.0, k.1))
        })
    });
    let native = if need_mesh {
        let prot = disc
            .read_named_file("PROT.DAT")
            .context("Final native PROT")?;
        let cd = disc
            .read_named_file("CDNAME.TXT")
            .map(|b| String::from_utf8_lossy(&b).into_owned());
        let index = ProtIndex::from_bytes(prot, cd.as_deref())?;
        let assembled =
            legaia_engine_world_views::scene_staging::assemble_field_scene(&index, &scene.name)
                .map_err(anyhow::Error::msg)?;
        let view = world_view::build_world(&assembled, &index);
        Some((assembled, view))
    } else {
        None
    };
    let mut next_owners = vec![];
    for mut owner in desired.owners {
        let Some(r) = owner.native.clone() else {
            next_owners.push(owner);
            continue;
        };
        let Some(key) = source_key(&r) else {
            next_owners.push(owner);
            continue;
        };
        if doc.hidden.contains(&key) {
            ensure!(
                reference(&current, (r.cell[0], r.cell[1]), &model_hashes).is_err(),
                "Owned collision deletion has a surviving native instance reference"
            );
            retired.insert(r.model_selector as usize);
            replacements += 1;
            continue;
        }
        let changed = doc.moves.iter().any(|(k, _)| *k == key)
            || doc
                .rotations
                .iter()
                .any(|(k, _)| *k == NameTarget::Native(key.0, key.1))
            || doc
                .scales
                .iter()
                .any(|(k, _)| *k == NameTarget::Native(key.0, key.1));
        if changed && !written.iter().any(|o| o.id == owner.id) {
            let failed = log.iter().any(|l| {
                l.starts_with(&format!("FAILED MOVE {key:?}"))
                    || l.starts_with(&format!(
                        "FAILED ROTATE {:?}",
                        NameTarget::Native(key.0, key.1)
                    ))
                    || l.starts_with(&format!(
                        "FAILED SCALE {:?}",
                        NameTarget::Native(key.0, key.1)
                    ))
            });
            ensure!(
                !failed,
                "Native geometry operation failed; collision must not move independently"
            );
            if let Some(policy) = owner.mesh_policy {
                let (assembled, view) = native
                    .as_ref()
                    .context("Final native geometry unavailable")?;
                let i = assembled
                    .placements
                    .iter()
                    .position(|p| p.cell == (r.cell[0], r.cell[1]))
                    .context("Owned final native instance missing")?;
                let mut recipe = view
                    .automatic_collision_recipe((false, i))
                    .map_err(anyhow::Error::msg)?;
                let generated = recipe
                    .generated
                    .as_mut()
                    .context("Owned mesh recipe missing")?;
                generated.band_height = policy[0];
                generated.min_face_area = policy[1];
                owner = walls::serialized_owner(
                    owner.id,
                    &recipe,
                    view.at(false, i).context("Final native draw")?,
                )?;
            } else {
                ensure!(
                    !doc.rotations
                        .iter()
                        .any(|(k, _)| *k == NameTarget::Native(key.0, key.1))
                        && !doc
                            .scales
                            .iter()
                            .any(|(k, _)| *k == NameTarget::Native(key.0, key.1)),
                    "Manual owned sub-cells have no rotation/scale serializer"
                );
                let delta = doc
                    .moves
                    .iter()
                    .find(|(k, _)| *k == key)
                    .context("Manual wall movement unavailable")?
                    .1;
                ensure!(
                    delta[1] == 0. && delta[0].round() % 64. == 0. && delta[2].round() % 64. == 0.,
                    "Manual owned sub-cells require exact native grid translation"
                );
                for c in owner.blocked.iter_mut().chain(&mut owner.open) {
                    c[0] += delta[0].round() as i32 / 64;
                    c[1] += delta[2].round() as i32 / 64;
                }
            }
            replacements += 1;
        }
        owner.native = Some(reference(&current, (r.cell[0], r.cell[1]), &model_hashes)?);
        next_owners.push(owner);
    }
    for owner in written {
        if let Some(i) = next_owners.iter().position(|o| o.id == owner.id) {
            next_owners[i] = owner;
        } else {
            next_owners.push(owner);
        }
    }
    desired.owners = next_owners;
    let (map, desired) = desired.compose_over(&current)?;
    let mut output = DiscPatcher::open(disc.image().to_vec())?;
    if map != current {
        output.patch_prot_entry(entry, 0, &map)?;
    }
    // Reclaim only editor-introduced, now-unreferenced resources. A failed
    // optional storage proof does not block unrestricted geometry export.
    let original_count = snapshot.resource_baseline.models.len();
    retired.retain(|i| *i >= original_count);
    if !retired.is_empty() {
        let limits = reg::Limits {
            max_models: 1024,
            max_decoded: 16 * 1024 * 1024,
            max_encoded: 16 * 1024 * 1024,
        };
        let b = output.read_entry_footprint(entry + 3)?;
        let p = output.read_entry_footprint(entry + 4)?;
        match reg::prove_retirable_baseline(&snapshot.resource_baseline,&b,Some(&p),&map,&retired,limits).and_then(|eligible|reg::retire_model_storage(&b,Some(&p),&eligible,limits)){
            Ok((b,p,saved))=>{let patched=(||->Result<DiscPatcher>{let mut candidate=DiscPatcher::open(output.image().to_vec())?;candidate.patch_prot_entry(entry+3,0,&b)?;if let Some(p)=p{candidate.patch_prot_entry(entry+4,0,&p)?;}Ok(candidate)})();match patched{Ok(candidate)=>{output=candidate;log.push(format!("WRITTEN OWNED MODEL RETIREMENT: {saved} persistent bytes reclaimed; selector numbering preserved"));},Err(e)=>log.push(format!("WARNING Model retirement write failed; collision retained: {e:#}"))}},
            Err(e)=>log.push(format!("WARNING Owned model storage retained: {e:#}; no original resource ownership inferred")),
        }
    }
    if !doc.hidden.is_empty() {
        log.push("WARNING TEXTURE LIFECYCLE: original/shared TIM and palette uploads retained. Unused texture release requires proven surviving model/animation/runtime upload closure; model retirement does not claim texture retirement.".into());
    }
    log.push(format!("WRITTEN OWNED COLLISION: {replacements} inherited owner transformations/deletions reconciled; {} serialized owners persisted; baseline/shared bits and floor nibbles protected",desired.owners.len()));
    let mut cells = snapshot.other_cells;
    cells.push(serde_json::json!({"source_image":snapshot.source_path,"source_image_sha256":snapshot.source_hash,"baseline_image":snapshot.baseline_path,"baseline_image_sha256":snapshot.baseline_hash,"resource_baseline":snapshot.resource_baseline,"journal":desired}));
    let metadata = serde_json::json!({"schema":"lck-native-wall-journal-v2","image_sha256":digest(output.image()),"cells":cells,"native_gameplay_verified":false,"original_native_wall_ownership_inferred":false});
    Ok((output, metadata))
}

/// Load provenance in the background scene worker; never on a drag/repaint.
pub(super) fn attach_preview(
    path: &Path,
    scene: &AssembledScene,
    world: &mut WorldModel,
    index: &ProtIndex,
) {
    if !path.with_extension("lck-collision.json").exists() {
        let result = (|| -> Result<_> {
            let entry = legaia_engine_scene::scene::Scene::load(index, &scene.name)?
                .field_map_index(index)
                .context("MAP")?;
            let map = index.entry_bytes(entry)?.as_ref().clone();
            let journal = Journal::reconstruct(scene.name.clone(), entry, &map, &map, vec![])?;
            Ok((journal, map))
        })();
        match result {
            Ok((journal, map)) => {
                world.collision.owned_journal = Some(std::sync::Arc::new(journal));
                world.collision.owned_source_map = Some(std::sync::Arc::new(map));
            }
            Err(e) => {
                world.collision.ownership_warning =
                    Some(format!("Collision preview unavailable: {e:#}"))
            }
        }
        return;
    }
    let result = (|| -> Result<Snapshot> {
        let bytes = std::fs::read(path)?;
        let hash = digest(&bytes);
        let disc = DiscPatcher::open(bytes)?;
        let entry = legaia_engine_scene::scene::Scene::load(index, &scene.name)?
            .field_map_index(index)
            .context("MAP")? as usize;
        capture(path, &hash, &disc, entry, &scene.name)
    })();
    match result {
        Ok(s) => {
            world.collision.owned_journal = Some(std::sync::Arc::new(s.journal));
            world.collision.owned_source_map = Some(std::sync::Arc::new(s.source_map));
        }
        Err(e) => {
            world.collision.ownership_warning =
                Some(format!("Serialized collision ownership unavailable: {e:#}"))
        }
    }
}

/// Recompose live authored layers in memory. No native resource reads or writes.
/// Undo restores the same immutable journal alongside the editor transforms.
pub(super) fn preview(world: &WorldModel) -> Result<Option<Vec<u8>>> {
    let (Some(journal), Some(source)) = (
        &world.collision.owned_journal,
        &world.collision.owned_source_map,
    ) else {
        return Ok(None);
    };
    let mut next = (**journal).clone();
    let mut owners = Vec::new();
    for original in &journal.owners {
        let mut owner = original.clone();
        if let Some(r) = &original.native {
            let key = world
                .collision
                .sources
                .iter()
                .find(|(_, p)| p.object.cell == r.cell && p.object.descriptor_id == r.descriptor)
                .map(|(k, _)| *k);
            if let Some(key) = key {
                if let Some(draw) = world.at(key.0, key.1) {
                    if draw.hidden {
                        continue;
                    }
                    if draw.offset != [0.; 3] || draw.rotation != [0.; 3] || draw.scale != [1.; 3] {
                        if let Some(policy) = original.mesh_policy {
                            let mut recipe = world
                                .automatic_collision_recipe(key)
                                .map_err(anyhow::Error::msg)?;
                            let g = recipe.generated.as_mut().context("Live owned mesh")?;
                            g.band_height = policy[0];
                            g.min_face_area = policy[1];
                            owner = walls::serialized_owner(original.id.clone(), &recipe, draw)?;
                            owner.native = original.native.clone();
                        } else {
                            ensure!(
                                draw.rotation == [0.; 3]
                                    && draw.scale == [1.; 3]
                                    && draw.offset[1] == 0.
                                    && draw.offset[0] % 64. == 0.
                                    && draw.offset[2] % 64. == 0.,
                                "Manual collision layer has no serializer for this transform"
                            );
                            for cell in owner.blocked.iter_mut().chain(&mut owner.open) {
                                cell[0] += draw.offset[0] as i32 / 64;
                                cell[1] += draw.offset[2] as i32 / 64;
                            }
                        }
                    }
                }
            }
        }
        owners.push(owner);
    }
    for assignment in &world.collision.recipes {
        let Some(draw) = world.draws.iter().find(|d| {
            world.collision.owner(world, (d.terrain, d.index)).as_ref() == Some(&assignment.owner)
        }) else {
            continue;
        };
        if draw.hidden {
            continue;
        }
        let old = match &assignment.owner {
            ObjectId::Native(id) => owners.iter().position(|o| {
                o.native
                    .as_ref()
                    .is_some_and(|r| r.cell == id.cell && r.descriptor == id.descriptor_id)
            }),
            ObjectId::Created(_) => None,
        };
        let id = old
            .map(|i| owners[i].id.clone())
            .unwrap_or_else(|| format!("live-{:?}", assignment.owner));
        let mut owner = walls::serialized_owner(id, &assignment.recipe, draw)?;
        if let Some(i) = old {
            owner.native = owners[i].native.clone();
            owners[i] = owner;
        } else {
            owners.push(owner);
        }
    }
    next.owners = owners;
    Ok(Some(next.compose_over(source)?.0))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "preserved user R8 scene, real normal exporter/reopen lifecycle; no GUI or emulator"]
    fn repeated_owned_move_rotate_delete_export_reopen() -> Result<()> {
        let folder = PathBuf::from(
            std::env::var_os("LCK_LIFECYCLE_DIR").context("new lifecycle directory")?,
        );
        ensure!(!folder.exists(), "Use a new output directory");
        std::fs::create_dir(&folder)?;
        let mut source = PathBuf::from(
            r"D:\AstraRimElmProject\ManualTests\RuntimeRecovery_20261030_01\R8_FULL_SCENE_OWNED_COLLISION_RECOVERY.img",
        );
        let mut prior_mask = None;
        let mut rows = vec![];
        for step in 0..3 {
            let lib = crate::library::Library::open(&source)?;
            let scene = legaia_engine_world_views::scene_staging::assemble_field_scene(
                &lib.index, "town01",
            )
            .map_err(anyhow::Error::msg)?;
            let mut world = world_view::build_world(&scene, &lib.index);
            attach_preview(&source, &scene, &mut world, &lib.index);
            ensure!(
                world.collision.ownership_warning.is_none(),
                "Ownership load: {:?}",
                world.collision.ownership_warning
            );
            let journal = world
                .collision
                .owned_journal
                .as_ref()
                .context("Ownership journal")?
                .clone();
            let original = journal
                .owners
                .iter()
                .find(|o| o.id == "legacy-import-3")
                .context("Recorded moved owner")?
                .clone();
            let r = original.native.as_ref().context("Native witness")?;
            let key = world
                .collision
                .sources
                .iter()
                .find(|(_, p)| p.object.cell == r.cell && p.object.descriptor_id == r.descriptor)
                .map(|(k, _)| *k)
                .context("Owned draw")?;
            let unchanged = preview(&world)?.context("Live preview")?;
            ensure!(
                digest(&unchanged) == journal.source_map_sha256,
                "Reopen changed collision"
            );
            let undo = world.clone();
            if step == 2 {
                ensure!(world.set_hidden(key, true), "Hide owned object");
            } else {
                ensure!(
                    world.set_offset(key.0, key.1, [128. * (step + 1) as f32, 0., 64.]),
                    "Move"
                );
                ensure!(
                    world.set_rotation(key.0, key.1, [0., 0.35 * (step + 1) as f32, 0.]),
                    "Rotate"
                );
            }
            let live = preview(&world)?.context("Live transform preview")?;
            ensure!(live != unchanged, "Live collision did not follow editing");
            ensure!(
                preview(&undo)?.unwrap() == unchanged,
                "Undo clone lost ownership baseline"
            );
            let editor = super::super::scene_instances::serialize("town01", &world);
            let plan = super::super::ps1_export::unrestricted::prepare(
                &source, &scene, &world, &lib.index,
            )?;
            ensure!(
                super::super::scene_instances::serialize("town01", &world) == editor,
                "Export changed editor transforms"
            );
            let output = folder.join(format!("repeat-{step}.img"));
            let result = super::super::ps1_export::export(plan, &output)?;
            ensure!(
                result.skipped.iter().all(|s| !s.starts_with("FAILED ")),
                "Export failures: {:?}",
                result.skipped
            );
            let disc = DiscPatcher::open(std::fs::read(&output)?)?;
            let map = disc.read_entry_footprint(journal.map_entry as usize)?;
            let snapshot = capture(
                &output,
                &digest(disc.image()),
                &disc,
                journal.map_entry as usize,
                "town01",
            )?;
            ensure!(
                snapshot
                    .journal
                    .owners
                    .iter()
                    .filter(|o| o.id != "legacy-import-3")
                    .eq(journal.owners.iter().filter(|o| o.id != "legacy-import-3")),
                "Unrelated/shared owner changed"
            );
            if step == 2 {
                ensure!(
                    snapshot
                        .journal
                        .owners
                        .iter()
                        .all(|o| o.id != "legacy-import-3"),
                    "Deleted layer persisted"
                );
            } else {
                let moved = snapshot
                    .journal
                    .owners
                    .iter()
                    .find(|o| o.id == "legacy-import-3")
                    .context("Moved owner lost")?;
                ensure!(
                    moved.blocked != original.blocked,
                    "Old collision mask retained"
                );
            }
            for i in 0x4000..0x8000 {
                ensure!(map[i] & 15 == unchanged[i] & 15, "Floor tier changed");
            }
            if let Some(mask) = prior_mask {
                ensure!(
                    mask != map[0x4000..0x8000],
                    "Repeated export kept prior collision grid"
                );
            }
            prior_mask = Some(map[0x4000..0x8000].to_vec());
            ensure!(snapshot.other_cells.is_empty(), "Unexpected journal cells");
            let metadata: serde_json::Value = serde_json::from_slice(&std::fs::read(
                output.with_extension("lck-collision.json"),
            )?)?;
            ensure!(
                metadata["schema"] == "lck-native-wall-journal-v2",
                "Self-contained journal missing"
            );
            rows.push(serde_json::json!({"step":step,"output":output,"owners":snapshot.journal.owners.len(),"map_sha256":digest(&map),"native_gameplay_verified":false}));
            source = output;
        }
        // A different cell adds its own journal without losing the first cell.
        let lib = crate::library::Library::open(&source)?;
        let scene =
            legaia_engine_world_views::scene_staging::assemble_field_scene(&lib.index, "bylon")
                .map_err(anyhow::Error::msg)?;
        let world = world_view::build_world(&scene, &lib.index);
        let plan =
            super::super::ps1_export::unrestricted::prepare(&source, &scene, &world, &lib.index)?;
        let output = folder.join("multi-cell.img");
        let result = super::super::ps1_export::export(plan, &output)?;
        ensure!(
            result.skipped.iter().all(|s| !s.starts_with("FAILED ")),
            "Second-cell export failed: {:?}",
            result.skipped
        );
        let file = output.with_extension("lck-collision.json");
        let mut metadata: serde_json::Value = serde_json::from_slice(&std::fs::read(&file)?)?;
        ensure!(
            metadata["cells"].as_array().context("Cell journals")?.len() == 2,
            "Another cell discarded first-cell ownership"
        );
        // The v2 fingerprint is sufficient even after intermediate files move.
        for cell in metadata["cells"].as_array_mut().unwrap() {
            cell["baseline_image"] = serde_json::json!("D:/not-an-existing-baseline.img");
        }
        std::fs::write(&file, serde_json::to_vec(&metadata)?)?;
        let disc = DiscPatcher::open(std::fs::read(&output)?)?;
        let entry = legaia_engine_scene::scene::Scene::load(&lib.index, "town01")?
            .field_map_index(&lib.index)
            .context("Town MAP")? as usize;
        let snapshot = capture(&output, &digest(disc.image()), &disc, entry, "town01")?;
        ensure!(
            snapshot.other_cells.len() == 1,
            "Multi-cell provenance not retained"
        );
        rows.push(serde_json::json!({"multi_cell_output":output,"self_contained_provenance_verified":true,"native_gameplay_verified":false}));
        std::fs::write(
            folder.join("lifecycle-verification.json"),
            serde_json::to_vec_pretty(&rows)?,
        )?;
        Ok(())
    }
}
