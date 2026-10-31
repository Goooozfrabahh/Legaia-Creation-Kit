//! Database-to-existing-writer regressions. No application or emulator launch.
use super::*;
use crate::app::{
    collision_recipes::digest, form_bridge::Session, form_database::*, lck_project::ProjectFile,
};
#[test]
fn failed_record_adapter_is_prominent_even_without_foreign_import_failures() {
    let result = ExportResult {
        path: "test.img".into(),
        patched: vec![],
        skipped: vec!["FAILED PROJECT NATIVE ADAPTER: shared material".into()],
        imports: Some(ImportCoverage {
            requested: 0,
            written: 0,
        }),
    };
    assert!(result
        .verification_warning()
        .unwrap()
        .contains("INCOMPLETE NATIVE EXPORT"));
    assert!(result.experimental_summary().contains("INCOMPLETE SCENE"));
}
#[test]
#[ignore = "real R8 project/export/portable journal regression; no GUI or emulator"]
fn project_bridge_preserves_existing_native_bytes_and_portable_owned_collision() -> Result<()> {
    let source = Path::new(
        r"D:\AstraRimElmProject\ManualTests\RuntimeRecovery_20261030_01\R8_FULL_SCENE_OWNED_COLLISION_RECOVERY.img",
    );
    let library = crate::library::Library::open(source)?;
    let scene =
        legaia_engine_world_views::scene_staging::assemble_field_scene(&library.index, "town01")
            .map_err(anyhow::Error::msg)?;
    let mut world = world_view::build_world(&scene, &library.index);
    crate::app::native_wall_export::attach_preview(source, &scene, &mut world, &library.index);
    ensure!(
        world.collision.ownership_warning.is_none(),
        "R8 ownership unavailable"
    );
    let mut session = Session::temporary(source, &library.index)?;
    let projected = session.enter(
        &scene,
        &library.index,
        &world,
        "original:town01".into(),
        "Rim Elm".into(),
        None,
    )?;
    ensure!(
        scene_instances::serialize("town01", &world)
            == scene_instances::serialize("town01", &projected),
        "Project changed original native operations"
    );
    let legacy = unrestricted::prepare(source, &scene, &world, &library.index)?;
    let expected = digest(&legacy.image);
    drop(legacy);
    let plan = prepare_project(source, &scene, &mut session, &library.index, true)?;
    ensure!(
        digest(&plan.image) == expected,
        "Unchanged project native output is not byte equivalent to existing writer"
    );
    ensure!(
        plan.skipped.iter().all(|s| !s.starts_with("FAILED ")),
        "Project writer failures: {:?}",
        plan.skipped
    );
    ensure!(
        plan.object_verification
            .iter()
            .any(|o| !o["stage_a_reference"].is_null()),
        "Permanent record report missing"
    );
    drop(plan);
    println!("Existing writer/project writer IMG byte equivalence passed");
    let root = Path::new(r"D:\AstraRimElmProject\ManualTests").join(format!(
        "lck-stage-a4-{}",
        crate::app::lck_project::new_project_id()?.as_str()
    ));
    std::fs::create_dir(&root)?;
    let relocated = root.join("relocated.img");
    std::fs::hard_link(source, &relocated)?;
    // Use the same source bytes without any adjacent journal. The saved exact
    // hash-bound journal must supply identical native witnesses in memory.
    let mut portable = Session::temporary(&relocated, &library.index)?;
    portable.mount_project(session.project.clone())?;
    portable.cell = session.cell.clone();
    let metadata = portable
        .preserved_collision_metadata()?
        .context("Exact journal not retained in project")?;
    let portable_world = portable.render(&scene, &library.index)?;
    ensure!(
        portable_world.collision.ownership_warning.is_none(),
        "Portable evidence failed: {:?}",
        portable_world.collision.ownership_warning
    );
    ensure!(
        serde_json::to_vec(&portable_world.collision.owned_journal.as_deref())?
            == serde_json::to_vec(&world.collision.owned_journal.as_deref())?,
        "Portable journal changed owned contributions"
    );
    let mut file = ProjectFile::new(portable.project.clone(), root.join("acceptance.lckproject"));
    file.save()?;
    let reopened = ProjectFile::open(&file.path, Some(&portable.source_fingerprint))?;
    ensure!(
        reopened.project.collision_associations == portable.project.collision_associations
            && reopened.project.sidecars == portable.project.sidecars,
        "Project lost exact collision state"
    );
    println!("Project save/reopen and portable collision witness equality passed");
    let plan = prepare_project(&relocated, &scene, &mut portable, &library.index, true)?;
    ensure!(
        digest(&plan.image) == expected,
        "Saved journal changed native output bytes"
    );
    drop(plan);
    let disc = DiscPatcher::open(std::fs::read(source)?)?;
    let mut stale: serde_json::Value = serde_json::from_slice(&metadata)?;
    stale["image_sha256"] = serde_json::json!("0".repeat(64));
    ensure!(
        crate::app::native_wall_export::capture_with_metadata(
            &relocated,
            &portable.source_fingerprint,
            &disc,
            1,
            "town01",
            Some(&serde_json::to_vec(&stale)?)
        )
        .is_err(),
        "Stale collision evidence accepted"
    );
    drop(disc);
    let reference = portable
        .reference_draws
        .keys()
        .next()
        .context("No reference")?
        .clone();
    let base = portable
        .project
        .database
        .reference(&reference)
        .unwrap()
        .base
        .clone();
    portable.transaction(|p| {
        p.database.edit_base(&base, |b| {
            b.materials
                .insert("preview_tint".into(), Property::Vector([0.4, 0.7, 1.]));
            Ok(())
        })
    })?;
    let partial = prepare_project(&relocated, &scene, &mut portable, &library.index, true)?;
    ensure!(
        partial
            .skipped
            .iter()
            .any(|s| s.starts_with("FAILED PROJECT NATIVE ADAPTER")),
        "Saved unsupported edit silently omitted"
    );
    drop(partial);
    // Only new disposable links/project files are removed; the retail/R8 paths
    // and source sidecars were never opened for writing.
    std::fs::remove_file(relocated)?;
    println!("A4 evidence/project retained at {}; native writer byte equivalence and portable journal verified; GUI/gameplay unverified",root.display());
    Ok(())
}
#[test]
#[ignore = "real R8 persistent-reference move/delete and owned collision bridge; no GUI or emulator"]
fn reference_move_delete_projects_exact_existing_owned_collision_operations() -> Result<()> {
    let source = Path::new(
        r"D:\AstraRimElmProject\ManualTests\RuntimeRecovery_20261030_01\R8_FULL_SCENE_OWNED_COLLISION_RECOVERY.img",
    );
    let library = crate::library::Library::open(source)?;
    let scene =
        legaia_engine_world_views::scene_staging::assemble_field_scene(&library.index, "town01")
            .map_err(anyhow::Error::msg)?;
    let mut world = world_view::build_world(&scene, &library.index);
    crate::app::native_wall_export::attach_preview(source, &scene, &mut world, &library.index);
    let journal = world
        .collision
        .owned_journal
        .as_ref()
        .context("R8 journal")?
        .clone();
    let native = journal
        .owners
        .iter()
        .find(|o| o.id == "legacy-import-3")
        .and_then(|o| o.native.as_ref())
        .context("Verified owner")?;
    let key = world
        .collision
        .sources
        .iter()
        .find(|(_, p)| p.object.cell == native.cell && p.object.descriptor_id == native.descriptor)
        .map(|(k, _)| *k)
        .context("Verified draw")?;
    let mut session = Session::temporary(source, &library.index)?;
    let projected = session.enter(
        &scene,
        &library.index,
        &world,
        "original:town01".into(),
        "Rim Elm".into(),
        None,
    )?;
    let reference = session
        .reference_at(&projected, key)
        .context("Persistent owner reference")?;
    let mut transform = session
        .project
        .database
        .reference(&reference)
        .unwrap()
        .transform;
    transform.position[0] += 128.;
    transform.position[2] += 64.;
    transform.rotation[1] = 0.35;
    session.transaction(|p| p.database.move_reference(&reference, transform))?;
    let moved = session.render(&scene, &library.index)?;
    ensure!(
        world.set_offset(key.0, key.1, [128., 0., 64.])
            && world.set_rotation(key.0, key.1, [0., 0.35, 0.]),
        "Legacy reference input"
    );
    ensure!(
        scene_instances::serialize("town01", &moved)
            == scene_instances::serialize("town01", &world),
        "Database move/rotation altered existing writer operations"
    );
    ensure!(
        crate::app::native_wall_export::preview(&moved)?
            == crate::app::native_wall_export::preview(&world)?,
        "Owned collision did not follow persistent reference transform"
    );
    session.delete(&[reference.clone()])?;
    let deleted = session.render(&scene, &library.index)?;
    world.set_hidden(key, true);
    ensure!(
        scene_instances::serialize("town01", &deleted)
            == scene_instances::serialize("town01", &world),
        "Reference tombstone altered native deletion operations"
    );
    ensure!(
        crate::app::native_wall_export::preview(&deleted)?
            == crate::app::native_wall_export::preview(&world)?,
        "Deleted reference retained owned collision in preview"
    );
    let plan = prepare_project(source, &scene, &mut session, &library.index, true)?;
    ensure!(
        plan.skipped.iter().all(|s| !s.starts_with("FAILED ")),
        "Native deletion failures: {:?}",
        plan.skipped
    );
    let log = plan.unrestricted_log.as_deref().context("Native report")?;
    let metadata: serde_json::Value = serde_json::from_str(
        log.lines()
            .find_map(|l| l.strip_prefix("WALL_OWNERSHIP_JOURNAL\t"))
            .context("Output ownership journal")?,
    )?;
    let cells = metadata["cells"].as_array().context("Cells")?;
    let cell = cells
        .iter()
        .find(|c| c["journal"]["map_entry"] == journal.map_entry)
        .context("Native cell")?;
    ensure!(
        cell["journal"]["owners"]
            .as_array()
            .context("Owners")?
            .iter()
            .all(|o| o["id"] != "legacy-import-3"),
        "Native export retained deleted owner"
    );
    let disc = DiscPatcher::open(plan.image)?;
    let map = disc.read_entry_footprint(journal.map_entry as usize)?;
    let parsed: crate::app::native_wall_journal::Journal =
        serde_json::from_value(cell["journal"].clone())?;
    parsed.validate(&map)?;
    ensure!(
        session
            .project
            .database
            .reference(&reference)
            .unwrap()
            .deleted,
        "Native writer changed tombstone"
    );
    println!("Persistent move/rotate/delete projects identical legacy operations and collision masks; native deletion journal/MAP validated; gameplay unverified");
    Ok(())
}
