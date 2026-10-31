//! Legacy decoding at the migration boundary. Draw ordinals are decoder hints;
//! the persisted import registry assigns identities from original native records.
use super::{collision_recipes::digest, form_database::*, lck_project::*, world_view::WorldModel};
use anyhow::{Context, Result};
use legaia_engine_scene::{
    scene::{ProtIndex, Scene},
    scene_assembly::AssembledScene,
};
use std::collections::{BTreeMap, BTreeSet};

fn source(
    scene: &AssembledScene,
    index: &ProtIndex,
    kind: &str,
    ordinal: usize,
    world: Option<&WorldModel>,
) -> Result<(String, NativeOrigin, Option<String>)> {
    let native = Scene::load(index, &scene.name)?;
    let entry = native.field_map_index(index);
    let mut fingerprints = BTreeMap::new();
    let mut origin = NativeOrigin {
        source_identity: String::new(),
        scene: Some(scene.name.clone()),
        prot_entry: entry,
        descriptor: None,
        man_record: None,
        anchor: None,
        cell: None,
        fingerprints: Default::default(),
    };
    let identity = if kind == "npc" {
        origin.man_record = Some(ordinal as u32);
        if let Some(man) = native.field_man_payload(index)? {
            fingerprints.insert("MAN".into(), digest(&man));
        }
        format!("{}:MAN:{ordinal}", scene.name)
    } else if kind == "mesh" {
        format!("{}:TMD-resource:{ordinal}", scene.name)
    } else {
        let draw = if kind == "terrain" {
            scene.terrain.get(ordinal)
        } else {
            scene.placements.get(ordinal)
        }
        .context("Native source draw unavailable")?;
        origin.cell = Some([draw.cell.0, draw.cell.1]);
        if let Some(entry) = entry {
            let map = index.entry_bytes(entry)?;
            fingerprints.insert("MAP".into(), digest(&map));
            if map.len() >= 0x10000 && draw.cell.0 < 128 && draw.cell.1 < 128 {
                let at = 0x8000 + (draw.cell.1 as usize * 128 + draw.cell.0 as usize) * 2;
                let id = u16::from_le_bytes(map[at..at + 2].try_into()?) & 511;
                origin.descriptor = Some(id);
                let record =
                    legaia_scene_formats::field_objects::ObjectRecord::parse(&map, id as usize)
                        .context("Native descriptor")?;
                let a = [
                    draw.cell.0 as i32 + record.col_delta as i32,
                    draw.cell.1 as i32 + record.row_delta as i32,
                ];
                if a.iter().all(|v| (0..128).contains(v)) {
                    origin.anchor = Some([a[0] as u8, a[1] as u8]);
                }
                fingerprints.insert(
                    "descriptor".into(),
                    digest(&map[id as usize * 32..id as usize * 32 + 32]),
                );
            }
        }
        if kind == "terrain" {
            format!(
                "{}:terrain-cell:{}:{}",
                scene.name, draw.cell.0, draw.cell.1
            )
        } else {
            format!(
                "{}:MAP:{:?}:descriptor:{:?}",
                scene.name, entry, origin.descriptor
            )
        }
    };
    let mesh = if kind == "npc" {
        world
            .and_then(|w| w.npcs.iter().find(|n| n.record == ordinal))
            .and_then(|n| n.model_pool.get(n.original_model_index as usize))
            .and_then(|m| m.as_ref())
            .map(|(raw, _)| digest(raw))
    } else {
        let slot = if kind == "mesh" {
            scene.env_tmds.get(ordinal).copied()
        } else {
            if kind == "terrain" {
                scene.terrain.get(ordinal)
            } else {
                scene.placements.get(ordinal)
            }
            .map(|d| d.res_tmd)
        };
        slot.and_then(|s| scene.res.tmds.get(s))
            .map(|r| digest(&r.raw))
    };
    if let Some(hash) = &mesh {
        fingerprints.insert("TMD".into(), hash.clone());
    }
    origin.source_identity = identity.clone();
    origin.fingerprints = fingerprints;
    Ok((identity, origin, mesh))
}

pub(super) fn scene_input(
    scene: &AssembledScene,
    index: &ProtIndex,
    world: &WorldModel,
    identity: String,
    label: String,
    copy_id: Option<u64>,
    exact_document: String,
    master_file: &FileId,
) -> Result<LegacySceneInput> {
    let mut objects = vec![];
    let dummy = FormKey::new(master_file.clone(), 1)?;
    for draw in &world.draws {
        let key = (draw.terrain, draw.index);
        let foreign = draw
            .editor_uid
            .and_then(|uid| world.foreign.iter().find(|f| f.uid == uid));
        let local_key = draw.source_ref.unwrap_or(key);
        let npc = world.npc(local_key);
        let (kind, ordinal) = if let Some(f) = foreign {
            (f.kind.as_str(), f.record)
        } else if let Some(n) = npc {
            ("npc", n.record)
        } else {
            (
                if local_key.0 { "terrain" } else { "placement" },
                local_key.1,
            )
        };
        let donor_scene = foreign.map(|f| f.source.as_str()).unwrap_or(&scene.name);
        let donor = if donor_scene != scene.name {
            legaia_engine_world_views::scene_staging::assemble_field_scene(index, donor_scene).ok()
        } else {
            None
        };
        let native = if donor_scene == scene.name {
            Some(scene)
        } else {
            donor.as_ref()
        };
        let evidence = native.and_then(|s| {
            source(
                s,
                index,
                kind,
                ordinal,
                if foreign.is_none() { Some(world) } else { None },
            )
            .ok()
        });
        // An unresolved donor is still a record. A later validation diagnostic
        // prevents the fallback identity from being presented as native evidence.
        let fallback = format!("unresolved-source:{donor_scene}:{kind}:{ordinal}");
        let (base_identity, origin, hash) = if let Some((id, origin, hash)) = evidence {
            (id, Some(origin), hash)
        } else {
            (fallback, None, foreign.map(|f| digest(&f.raw_tmd)))
        };
        let location = origin
            .as_ref()
            .map(|o| {
                if let Some(id) = o.man_record {
                    format!("MAN{id}")
                } else if kind == "terrain" {
                    o.cell
                        .map(|c| format!("x{}_z{}", c[0], c[1]))
                        .unwrap_or_else(|| format!("source{ordinal}"))
                } else {
                    o.descriptor
                        .map(|n| format!("MAP{n}"))
                        .unwrap_or_else(|| format!("source{ordinal}"))
                }
            })
            .unwrap_or_else(|| format!("source{ordinal}"));
        let editor_id = format!(
            "Native_{}_{}_{}_{}",
            donor_scene,
            kind,
            location,
            &digest(base_identity.as_bytes())[..8]
        );
        let form_kind = match kind {
            "npc" => FormKind::Actor,
            "terrain" => FormKind::Terrain,
            _ => FormKind::StaticEnvironment,
        };
        let mut base = BaseForm::empty(dummy.clone(), editor_id, form_kind);
        base.origin = origin.clone();
        let mut resource_origin = origin.clone();
        if let (Some(hash), Some(o)) = (&hash, &mut resource_origin) {
            o.fingerprints.insert("TMD".into(), hash.clone());
        }
        base.resources.push(ResourceReference {
            identity: format!("native-resource:{base_identity}"),
            family: "native-TMD-with-material-dependencies".into(),
            form: None,
            origin: resource_origin,
        });
        let adapter = LegacyBaseAdapter {
            source_scene: donor_scene.into(),
            kind: kind.into(),
            source_ordinal: ordinal,
            origin: origin.clone(),
        };
        let source_identity = if let Some(uid) = draw.editor_uid {
            format!("created:{uid}")
        } else if let Some(o) = &origin {
            format!(
                "reference:{base_identity}:cell:{:?}:man:{:?}",
                o.cell, o.man_record
            )
        } else {
            format!("unresolved-reference:{}:{kind}:{ordinal}", scene.name)
        };
        let original = world.at(local_key.0, local_key.1).unwrap_or(draw);
        let original_transform = Transform {
            position: original.original_center,
            rotation: [0.; 3],
            scale: [1.; 3],
        };
        let transform = Transform {
            position: draw.center,
            rotation: draw.rotation,
            scale: draw.scale,
        };
        let mut flags = BTreeSet::new();
        if foreign.is_some_and(|f| f.static_architecture) {
            flags.insert("static_architecture".into());
        }
        let mut properties = BTreeMap::new();
        if draw.editor_uid.is_some() {
            properties.insert(
                "legacy_instance_kind".into(),
                Property::Text(
                    if foreign.is_some() {
                        "foreign"
                    } else {
                        "native-copy"
                    }
                    .into(),
                ),
            );
        }

        if let Some(n) = world.npc(key) {
            properties.insert(
                "npc_model_selection".into(),
                Property::Integer(n.model_index as i64),
            );
            properties.insert(
                "npc_animation_clip".into(),
                Property::Integer(n.clip as i64),
            );
            properties.insert(
                "npc_animation_frame".into(),
                Property::Integer(n.frame as i64),
            );
            if n.conditional {
                flags.insert("native_story_conditional".into());
            }
        }
        let mut collision_owners = BTreeSet::new();
        let mut collision_payloads = vec![];
        if let Some(owner) = world.collision.owner(world, key) {
            for assignment in world.collision.recipes.iter().filter(|a| a.owner == owner) {
                collision_payloads.push((
                    "legacy-authored-collision".into(),
                    serde_json::to_value(assignment)?,
                ));
            }
        }
        if let Some(uid) = draw.editor_uid {
            if let Some(recipe) = world.collision.mesh_recipes.get(&uid) {
                collision_payloads.push((
                    "legacy-generated-mesh-collision".into(),
                    serde_json::to_value(recipe)?,
                ));
            }
        }
        if draw.editor_uid.is_none() {
            for (owner, native_key) in &world.collision.owned_draws {
                if *native_key == key {
                    collision_owners.insert(owner.clone());
                }
            }
            if let (Some(j), Some(p)) = (
                &world.collision.owned_journal,
                world.collision.sources.get(&key),
            ) {
                for owned in &j.owners {
                    if owned.native.as_ref().is_some_and(|r| {
                        r.cell == p.object.cell && r.descriptor == p.object.descriptor_id
                    }) {
                        collision_owners.insert(owned.id.clone());
                    }
                }
            }
        }
        let mut original_properties = BTreeMap::new();
        let mut original_flags = BTreeSet::new();
        if let Some(n) = world.npc(key) {
            original_properties.insert(
                "npc_model_selection".into(),
                Property::Integer(n.original_model_index as i64),
            );
            original_properties.insert(
                "npc_animation_clip".into(),
                Property::Integer(n.anim_id as i64),
            );
            original_properties.insert("npc_animation_frame".into(), Property::Integer(0));
            if n.conditional {
                original_flags.insert("native_story_conditional".into());
            }
        }
        objects.push(LegacyObjectInput {
            source_identity,
            base_identity,
            base,
            adapter,
            origin,
            uid: draw.editor_uid,
            original_transform,
            original_properties,
            original_flags,
            transform,
            display_name: draw.display_name.clone(),
            hidden: draw.hidden,
            disabled: false,
            flags,
            properties,
            collision_owners,
            collision_payloads,
        });
    }
    Ok(LegacySceneInput {
        identity,
        scene: scene.name.clone(),
        label,
        copy_id,
        exact_document,
        objects,
    })
}

pub(super) fn migrate_document(
    project: &mut LckProject,
    scene: &AssembledScene,
    index: &ProtIndex,
    original: &WorldModel,
    text: &str,
    cell_identity: String,
    label: String,
    copy_id: Option<u64>,
) -> Result<FormKey> {
    let doc =
        super::scene_instances::parse(text, &scene.name, original).map_err(anyhow::Error::msg)?;
    let mut preview = original.clone();
    super::scene_instances::apply(&doc, &mut preview, scene.placements.len(), index)
        .map_err(anyhow::Error::msg)?;
    let input = scene_input(
        scene,
        index,
        &preview,
        cell_identity,
        label,
        copy_id,
        text.into(),
        &project.master.file,
    )?;
    project.migrate_scene(&input)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "reads preserved native disc and exact full user legacy scene; no GUI, no native writes"]
    fn exact_user_legacy_scene_migrates_idempotently_and_roundtrips_project() -> Result<()> {
        let source = std::path::Path::new(
            r"D:\AstraRimElmProject\ManualTests\Native_Format_Adapters_20261028_01\LCK_EXPERIMENTAL_MIXED_FORMATS_FIXED.img",
        );
        let lib = crate::library::Library::open(source)?;
        let scene =
            legaia_engine_world_views::scene_staging::assemble_field_scene(&lib.index, "town01")
                .map_err(anyhow::Error::msg)?;
        let world = super::super::world_view::build_world(&scene, &lib.index);
        let text = std::fs::read_to_string(
            r"D:\AstraRimElmProject\ManualTests\Crash222_20261029_01\222-full-scene-preserved.lckscene",
        )?;
        let mut p = LckProject::new(
            MasterDependency {
                file: FileId::new("retail-SCUS94254")?,
                disc_sha256: digest(&std::fs::read(source)?),
                path_hint: Some(source.into()),
            },
            FileId::new("legacy-exact-regression")?,
        )?;
        let cell = migrate_document(
            &mut p,
            &scene,
            &lib.index,
            &world,
            &text,
            "original:town01".into(),
            "Rim Elm".into(),
            None,
        )?;
        anyhow::ensure!(
            p.legacy_bindings.iter().filter(|b| b.uid.is_some()).count() == 7,
            "Created identities lost"
        );
        anyhow::ensure!(
            p.collision_associations.len() == 7,
            "Collision assignments lost"
        );
        anyhow::ensure!(
            p.database
                .references()
                .values()
                .filter(|r| r.deleted)
                .count()
                == 2,
            "Native hidden/deletion state lost"
        );
        let records = serde_json::to_vec(&p.database)?;
        let bindings = p.legacy_bindings.clone();
        migrate_document(
            &mut p,
            &scene,
            &lib.index,
            &world,
            &text,
            "original:town01".into(),
            "Rim Elm".into(),
            None,
        )?;
        anyhow::ensure!(
            serde_json::to_vec(&p.database)? == records && p.legacy_bindings == bindings,
            "Migration allocated/replaced identities on repetition"
        );
        anyhow::ensure!(
            p.legacy_documents[&cell] == text,
            "Legacy source not retained exactly"
        );
        let temp = tempfile::tempdir()?;
        let path = temp.path().join("migrated.lckproject");
        let mut file = ProjectFile::new(p, path.clone());
        file.save()?;
        let reopened = ProjectFile::open(&path, Some(&file.project.master.disc_sha256))?;
        anyhow::ensure!(
            serde_json::to_vec(&reopened.project)? == serde_json::to_vec(&file.project)?,
            "Save/reopen changed relationships or provenance"
        );
        Ok(())
    }
}
