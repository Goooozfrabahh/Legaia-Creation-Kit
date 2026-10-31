use super::*;
use crate::app::{
    collision_recipes::digest, native_asset_plan::scene_registration as reg,
    native_wall_journal::Journal,
};
const SOURCE: &str = r"D:\Emulators\Games\Legend of Legaia\LegendOfLegaia.img";
const LOG: &str = r"D:\AstraRimElmProject\test12345_export_1793408211327185600\debug.txt";
fn fixture() -> Result<(crate::library::Library, AssembledScene, WorldModel)> {
    let lib = crate::library::Library::open(Path::new(SOURCE))?;
    let scene =
        legaia_engine_world_views::scene_staging::assemble_field_scene(&lib.index, "town01")
            .map_err(anyhow::Error::msg)?;
    let mut world = world_view::build_world(&scene, &lib.index);
    let text = std::fs::read_to_string(LOG)?;
    let start = text.find("LCK_SCENE_V1\t").context("manifest")?;
    let manifest = text[start..]
        .lines()
        .take_while(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    let doc = scene_instances::parse(&manifest, "town01", &world).map_err(anyhow::Error::msg)?;
    ensure!(
        doc.foreign.len() == 5 && doc.mesh_collision.len() == 5,
        "Exact fixture changed"
    );
    let expected = [
        ("jiji", 1),
        ("jiji", 1),
        ("jiji", 1),
        ("bubu1", 22),
        ("conc2", 107),
    ];
    for (f, (name, record)) in doc.foreign.iter().zip(expected) {
        ensure!(
            f.source == name && f.kind == "terrain" && f.record == record,
            "Fixture donor changed"
        );
    }
    scene_instances::apply(&doc, &mut world, scene.placements.len(), &lib.index)
        .map_err(anyhow::Error::msg)?;
    Ok((lib, scene, world))
}
fn limits() -> reg::Limits {
    reg::Limits {
        max_models: 1024,
        max_decoded: 16 * 1024 * 1024,
        max_encoded: 16 * 1024 * 1024,
    }
}
fn journal(path: &Path) -> Result<Journal> {
    let v: serde_json::Value =
        serde_json::from_slice(&std::fs::read(path.with_extension("lck-collision.json"))?)?;
    Ok(serde_json::from_value(
        v["cells"]
            .as_array()
            .context("cells")?
            .iter()
            .find(|c| c["journal"]["map_entry"] == 1)
            .context("town journal")?["journal"]
            .clone(),
    )?)
}
fn assert_floors(original: &[u8], edited: &[u8]) -> Result<()> {
    ensure!(
        (0x4000..0x8000).all(|i| original[i] & 15 == edited[i] & 15),
        "Retail floor nibbles changed"
    );
    ensure!(
        original[0x10000..] == edited[0x10000..],
        "Story bindings changed"
    );
    Ok(())
}
#[test]
#[ignore = "exact five user terrain imports and repeat native ownership; requires new LCK_REPAIR_DIR; no GUI/emulator"]
fn exact_five_terrain_export_readback_and_owned_lifecycle() -> Result<()> {
    let root = PathBuf::from(
        std::env::var_os("LCK_REPAIR_DIR").context("Choose a NEW test output directory")?,
    );
    std::fs::create_dir(&root)?;
    let (lib, scene, world) = fixture()?;
    let doc = scene_instances::parse(
        &scene_instances::serialize("town01", &world),
        "town01",
        &world,
    )
    .map_err(anyhow::Error::msg)?;
    for foreign in &doc.foreign {
        let donor = legaia_engine_world_views::scene_staging::assemble_field_scene(
            &lib.index,
            &foreign.source,
        )
        .map_err(anyhow::Error::msg)?;
        let n = Scene::load(&lib.index, &foreign.source)?;
        let e = n.field_map_index(&lib.index).context("donor MAP")?;
        let b = legaia_engine_scene::scene_bundle::find_bundle(&n)
            .map(|b| b.bytes().to_vec())
            .unwrap_or(lib.index.entry_bytes(e + 3)?.as_ref().clone());
        let p = reg::native_stream_mode(&b)
            .map(|_| lib.index.entry_bytes(e + 4))
            .transpose()?;
        let (_, vram) = reg::native_donor_texture_sequence(
            &lib.index,
            &b,
            p.as_deref().map(|v| v.as_slice()),
            limits(),
        )?;
        let raw = &donor.res.tmds[donor.terrain[foreign.record].res_tmd].raw;
        let (samples, changed) =
            crate::app::native_asset_plan::texture_relocation::texture_sample_changes(
                raw,
                &vram,
                raw,
                &donor.res.vram,
            )?;
        ensure!(
            changed == 0,
            "Native ordered uploads disagree with donor material: UID {}, {changed}/{samples}",
            foreign.uid
        );
        println!(
            "UID {} donor texture samples {samples}, changed {changed}, TMD {}",
            foreign.uid,
            digest(raw)
        );
    }
    let plan = unrestricted::prepare(Path::new(SOURCE), &scene, &world, &lib.index)?;
    ensure!(
        plan.imports
            == Some(ImportCoverage {
                requested: 5,
                written: 5
            }),
        "Not all five resources registered"
    );
    ensure!(
        plan.skipped.iter().all(|s| !s.starts_with("FAILED ")),
        "Writer failures: {:?}",
        plan.skipped
    );
    ensure!(
        plan.object_verification.len() == 5
            && plan
                .object_verification
                .iter()
                .all(|o| o["encoded_successfully"] == true
                    && o["native_collision"]["owned_wall_encoding_verified"] == true),
        "Per-object resource/collision report incomplete"
    );
    let reports = plan.object_verification.clone();
    let output = root.join("TEST12345_FIVE_TERRAIN_REPAIRED.img");
    let result = export(plan, &output)?;
    ensure!(
        result.verification_warning().is_none(),
        "Incomplete native writes: {:?}",
        result.skipped
    );
    let disc = DiscPatcher::open(std::fs::read(&output)?)?;
    let map = disc.read_entry_footprint(1)?;
    let baseline = DiscPatcher::open(std::fs::read(SOURCE)?)?;
    let original = baseline.read_entry_footprint(1)?;
    assert_floors(&original, &map)?;
    for e in 0..baseline.entry_count() {
        if ![1, 4, 5].contains(&e) {
            ensure!(
                baseline.read_entry_footprint(e)? == disc.read_entry_footprint(e)?,
                "Unrelated PROT {e} changed"
            );
        }
    }
    let j = journal(&output)?;
    j.validate(&map)?;
    ensure!(j.owners.len() == 5, "Five collision owners not retained");
    let mut owned_baseline = map.clone();
    owned_baseline[0x4000..0x8000].copy_from_slice(&j.baseline_high);
    ensure!(
        (0x4000..0x8000).all(|i| owned_baseline[i] & 0xf0 == original[i] & 0xf0),
        "Retail wall baseline altered"
    );
    let trace = crate::app::native_loader_trace::trace(
        &disc.read_entry_footprint(4)?,
        0,
        Some(&disc.read_entry_footprint(5)?),
    )?;
    let mut cells = std::collections::BTreeSet::new();
    for report in &reports {
        let cell = report["native_reference"]["cell"]
            .as_array()
            .context("native cell")?;
        ensure!(
            cells.insert((cell[0].as_u64(), cell[1].as_u64())),
            "Shared native identity"
        );
        let selector = report["native_loader_identity"]["scene_selector"]
            .as_u64()
            .unwrap() as usize;
        ensure!(
            trace.models[selector].sha256
                == report["native_loader_identity"]["relocated_tmd_sha256"],
            "Native loader identity mismatch"
        );
    }
    let (_, before) = reg::native_texture_sequence(
        &baseline.read_entry_footprint(4)?,
        Some(&baseline.read_entry_footprint(5)?),
        limits(),
    )?;
    let (_, after) = reg::native_texture_sequence(
        &disc.read_entry_footprint(4)?,
        Some(&disc.read_entry_footprint(5)?),
        limits(),
    )?;
    let mut resident = legaia_tim::Vram::new();
    lib.index
        .system_ui_bundle()?
        .upload_boot_resident_to_vram(&mut resident);
    let party = legaia_actor_formats::field_char_textures::parse(
        &lib.index
            .entry_bytes(legaia_actor_formats::field_char_textures::PROT_ENTRY_INDEX)?,
    )?;
    party.upload_to_vram(&mut resident, false);
    for i in 0..1024 * 512 {
        if before.is_written(i % 1024, i / 1024) {
            ensure!(
                before.as_u16()[i] == after.as_u16()[i],
                "Original scene VRAM changed"
            );
        } else if resident.is_written(i % 1024, i / 1024) {
            ensure!(
                !after.is_written(i % 1024, i / 1024),
                "New import overwrites resident VRAM at {},{}",
                i % 1024,
                i / 1024
            );
        }
    }
    drop(disc);
    drop(baseline);
    let reopened = crate::library::Library::open(&output)?;
    let assembled =
        legaia_engine_world_views::scene_staging::assemble_field_scene(&reopened.index, "town01")
            .map_err(anyhow::Error::msg)?;
    let mut native = world_view::build_world(&assembled, &reopened.index);
    crate::app::native_wall_export::attach_preview(
        &output,
        &assembled,
        &mut native,
        &reopened.index,
    );
    ensure!(
        native.collision.ownership_warning.is_none(),
        "Reopen ownership {:?}",
        native.collision.ownership_warning
    );
    let mut session = crate::app::form_bridge::Session::temporary(&output, &reopened.index)?;
    let preview = session.enter(
        &assembled,
        &reopened.index,
        &native,
        "original:town01".into(),
        "Rim Elm".into(),
        None,
    )?;
    let key_for_uid = |uid: u64| -> Result<(bool, usize)> {
        let r = reports.iter().find(|r| r["uid"] == uid).context("report")?;
        let c = &r["native_reference"]["cell"];
        let cell = (c[0].as_u64().unwrap() as u8, c[1].as_u64().unwrap() as u8);
        if let Some(i) = assembled.placements.iter().position(|p| p.cell == cell) {
            Ok((false, i))
        } else {
            Ok((
                true,
                assembled
                    .terrain
                    .iter()
                    .position(|p| p.cell == cell)
                    .context("reopened terrain")?,
            ))
        }
    };
    let moved = session
        .reference_at(&preview, key_for_uid(5)?)
        .context("persistent moved reference")?;
    let removed = session
        .reference_at(&preview, key_for_uid(4)?)
        .context("persistent deleted reference")?;
    ensure!(
        !session
            .project
            .database
            .reference(&moved)
            .unwrap()
            .collision_owners
            .is_empty(),
        "Terrain lost persistent ownership handles"
    );
    let copy = session.duplicate(&[removed.clone()])?[0].clone();
    session.transaction(|p| {
        let mut t = p.database.reference(&moved).unwrap().transform;
        t.position[0] += 128.;
        t.position[2] += 64.;
        t.rotation[1] = 0.25;
        p.database.move_reference(&moved, t)?;
        let mut t = p.database.reference(&copy).unwrap().transform;
        t.position[0] += 512.;
        p.database.move_reference(&copy, t)?;
        p.database.delete_reference(&removed)
    })?;
    let edited = session.render(&assembled, &reopened.index)?;
    session.synchronize_gesture(&assembled, &reopened.index, &edited)?;
    ensure!(
        session
            .project
            .diagnostics
            .iter()
            .all(|d| !d.message.starts_with("PREVIEW:")),
        "Reference projection failed: {:?}",
        session.project.diagnostics
    );
    crate::app::native_wall_export::preview(&edited)?.context("live owned walls")?;
    let plan = prepare_project(&output, &assembled, &mut session, &reopened.index, true)?;
    ensure!(
        plan.skipped.iter().all(|s| !s.starts_with("FAILED ")),
        "Repeat writer failures: {:?}",
        plan.skipped
    );
    let second = root.join("TEST12345_REFERENCE_MOVE_COPY_DELETE.img");
    export(plan, &second)?;
    let second_disc = DiscPatcher::open(std::fs::read(&second)?)?;
    let second_map = second_disc.read_entry_footprint(1)?;
    assert_floors(&original, &second_map)?;
    let second_j = journal(&second)?;
    second_j.validate(&second_map)?;
    ensure!(
        second_j.owners.len() == 5,
        "Move/copy/delete lost independent owners"
    );
    for uid in 1..=3 {
        let cell = &reports[(uid - 1) as usize]["native_reference"]["cell"];
        let x = cell[0].as_u64().unwrap() as u8;
        let z = cell[1].as_u64().unwrap() as u8;
        let prior = j
            .owners
            .iter()
            .find(|o| o.native.as_ref().is_some_and(|r| r.cell == [x, z]))
            .unwrap();
        ensure!(
            second_j.owners.iter().any(|o| o == prior),
            "Unrelated surviving owner {uid} changed"
        );
    }
    let again = crate::library::Library::open(&second)?;
    let scene2 =
        legaia_engine_world_views::scene_staging::assemble_field_scene(&again.index, "town01")
            .map_err(anyhow::Error::msg)?;
    let mut world2 = world_view::build_world(&scene2, &again.index);
    crate::app::native_wall_export::attach_preview(&second, &scene2, &mut world2, &again.index);
    ensure!(
        world2.collision.ownership_warning.is_none(),
        "Second ownership readback failed"
    );
    let third = root.join("TEST12345_REOPEN_UNCHANGED.img");
    export(
        unrestricted::prepare(&second, &scene2, &world2, &again.index)?,
        &third,
    )?;
    let third_disc = DiscPatcher::open(std::fs::read(&third)?)?;
    ensure!(
        third_disc.read_entry_footprint(1)? == second_map,
        "Unchanged reopen changed MAP"
    );
    journal(&third)?.validate(&second_map)?;
    println!("EXACT FIVE TERRAIN ACCEPTANCE PASSED: 5/5 native resources, samples/resident protection, 5 independent owned wall policies, persistent move/duplicate/delete and repeat readback. Outputs {}. Gameplay and new walkable surfaces unverified",root.display());
    Ok(())
}
#[test]
#[ignore = "read-only audit of the original user partial IMG and its report"]
fn audit_original_test12345_partial_native_bytes() -> Result<()> {
    let root = Path::new(r"D:\AstraRimElmProject\test12345_export_1793408211327185600");
    let report: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("object-verification.json"))?)?;
    let profile: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("scene-verification.json"))?)?;
    let bytes = std::fs::read(r"D:\AstraRimElmProject\test12345.img")?;
    ensure!(
        digest(&bytes) == report["img_sha256"] && report["img_sha256"] == profile["img_sha256"],
        "Original partial IMG no longer matches supplied reports"
    );
    let partial = DiscPatcher::open(bytes)?;
    let b = partial.read_entry_footprint(4)?;
    let p = partial.read_entry_footprint(5)?;
    let (base, _) = reg::native_descriptor_table(&b)?;
    let trace = crate::app::native_loader_trace::trace(&b, base, Some(&p))?;
    ensure!(
        trace.models.len() == 116,
        "Unexpected original partial registration count"
    );
    let objects = report["objects"].as_array().context("objects")?;
    for uid in 1..=5 {
        let o = objects
            .iter()
            .find(|o| o["uid"] == uid)
            .context("requested object")?;
        if uid == 5 {
            ensure!(
                o["encoded_successfully"] == false,
                "Original UID 5 unexpectedly encoded"
            );
        } else {
            let selector = o["native_loader_identity"]["scene_selector"]
                .as_u64()
                .context("selector")? as usize;
            ensure!(
                trace.models[selector].sha256
                    == o["native_loader_identity"]["relocated_tmd_sha256"],
                "Original partial loader identity disagrees with report"
            );
        }
    }
    let original = DiscPatcher::open(std::fs::read(SOURCE)?)?;
    assert_floors(
        &original.read_entry_footprint(1)?,
        &partial.read_entry_footprint(1)?,
    )?;
    ensure!(
        profile["static_native_request_profile"]["persistent_destination_requests"] == 460260,
        "Original accounting changed"
    );
    println!("Original test12345 IMG matches supplied SHA and four successful object identities; UID 5 absent; native pool 116; retail floor/binding records intact. No gameplay inference");
    Ok(())
}
